//! PDF stream filters (ISO 32000-2 §7.4).
//!
//! Layer L0, standalone: this crate depends on no other PdfCraft crate.
//!
//! Decoding covers the general-purpose filters (`FlateDecode`, `LZWDecode` with
//! TIFF/PNG predictors, `ASCIIHexDecode`, `ASCII85Decode`, `RunLengthDecode`).
//! Image codecs (`DCTDecode`, `JPXDecode`, `JBIG2Decode`, `CCITTFaxDecode`),
//! `Crypt` and unknown filters are reported as [`FilterError::Unsupported`] so
//! callers keep such data encoded. CCITT fax and JBIG2 images decode on their own
//! ([`decode_ccitt_within`], [`decode_jbig2_within`]), each spending from an allowance its caller
//! shares across a run of images, as `pdfcraft_cos::ImageBudget::decode` does after the filters
//! before them. Neither decodes without one:
//!
//! ```compile_fail
//! let _ = pdfcraft_filters::decode_jbig2(&[], None, 1, 1, 1 << 20);
//! ```
//!
//! ```compile_fail
//! let _ = pdfcraft_filters::decode_ccitt(&[], &pdfcraft_filters::CcittParams::default(), 1);
//! ```
//!
//! ```
//! let mut work = 1 << 20;
//! assert!(pdfcraft_filters::decode_jbig2_within(&[], None, 1, 1, 1 << 20, &mut work).is_err());
//! let p = pdfcraft_filters::CcittParams { columns: 8, ..Default::default() };
//! assert!(pdfcraft_filters::decode_ccitt_within(&[], &p, 1, &mut work).is_ok());
//! ```
//!
//! 1-bit samples can be encoded as CCITT Group 4 ([`encode_g4`]) or lossless JBIG2
//! ([`encode_jbig2`]), a group of images sharing their marks through `/JBIG2Globals`
//! ([`encode_jbig2_shared`]).
//!
//! Every decoder is bounded by a caller-supplied `max_output` (decompression-bomb
//! defence) and never panics on malformed input.
#![forbid(unsafe_code)]
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

mod ascii85;
mod asciihex;
mod ccitt;
mod flate;
mod jbig2;
mod lzw;
mod mq;
mod predictor;
mod runlength;

use std::borrow::Cow;

pub use ccitt::{CcittParams, MAX_CCITT_COLUMNS, MAX_G4_COLUMNS, decode_ccitt_within, encode_g4};
pub use jbig2::{MAX_JBIG2_COLUMNS, MAX_JBIG2_PIXELS, SharedJbig2, decode_jbig2_within, encode_jbig2, encode_jbig2_generic, encode_jbig2_shared};

/// A PDF stream filter, identified by its `/Filter` name.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Filter {
    Flate,
    Lzw,
    AsciiHex,
    Ascii85,
    RunLength,
    Dct,
    Jpx,
    Jbig2,
    CcittFax,
    Crypt,
    /// Any other name (kept verbatim, lossily converted to UTF-8).
    Unknown(String),
}

impl Filter {
    /// Parses a filter name. Accepts the full names (ISO 32000-2 Table 6) and the
    /// inline-image abbreviations (Table 92): `Fl`, `LZW`, `AHx`, `A85`, `RL`, `DCT`, `CCF`.
    /// A leading `/` is ignored.
    pub fn from_name(name: &[u8]) -> Filter {
        let n = name.strip_prefix(b"/").unwrap_or(name);
        match n {
            b"FlateDecode" | b"Fl" => Filter::Flate,
            b"LZWDecode" | b"LZW" => Filter::Lzw,
            b"ASCIIHexDecode" | b"AHx" => Filter::AsciiHex,
            b"ASCII85Decode" | b"A85" => Filter::Ascii85,
            b"RunLengthDecode" | b"RL" => Filter::RunLength,
            b"DCTDecode" | b"DCT" => Filter::Dct,
            b"JPXDecode" => Filter::Jpx,
            b"JBIG2Decode" => Filter::Jbig2,
            b"CCITTFaxDecode" | b"CCF" => Filter::CcittFax,
            b"Crypt" => Filter::Crypt,
            other => Filter::Unknown(String::from_utf8_lossy(other).into_owned()),
        }
    }

    /// The full PDF name (without the leading `/`).
    pub fn name(&self) -> &str {
        match self {
            Filter::Flate => "FlateDecode",
            Filter::Lzw => "LZWDecode",
            Filter::AsciiHex => "ASCIIHexDecode",
            Filter::Ascii85 => "ASCII85Decode",
            Filter::RunLength => "RunLengthDecode",
            Filter::Dct => "DCTDecode",
            Filter::Jpx => "JPXDecode",
            Filter::Jbig2 => "JBIG2Decode",
            Filter::CcittFax => "CCITTFaxDecode",
            Filter::Crypt => "Crypt",
            Filter::Unknown(s) => s,
        }
    }

    /// True for the image codecs (DCT, JPX, JBIG2, CCITT fax).
    pub fn is_image_codec(&self) -> bool {
        matches!(self, Filter::Dct | Filter::Jpx | Filter::Jbig2 | Filter::CcittFax)
    }

    /// True for the general-purpose filters, which a chain can hold ([`decode`]).
    pub fn is_decodable(&self) -> bool {
        matches!(self, Filter::Flate | Filter::Lzw | Filter::AsciiHex | Filter::Ascii85 | Filter::RunLength)
    }
}

/// `/DecodeParms` values relevant to the general-purpose filters (ISO 32000-2 Tables 8–9).
#[derive(Clone, Debug, PartialEq)]
pub struct Params {
    /// 1 = none, 2 = TIFF predictor 2, 10–15 = PNG predictors.
    pub predictor: i64,
    pub colors: i64,
    pub bits_per_component: i64,
    pub columns: i64,
    /// LZW only: 1 (default) switches code width one code early, 0 does not.
    pub early_change: i64,
}

impl Default for Params {
    fn default() -> Self {
        Params { predictor: 1, colors: 1, bits_per_component: 8, columns: 1, early_change: 1 }
    }
}

/// Errors from decoding or encoding.
#[derive(Debug, thiserror::Error)]
pub enum FilterError {
    #[error("{filter}: corrupt data: {detail}")]
    Corrupt { filter: &'static str, detail: String },
    #[error("decoded output exceeds the limit of {0} bytes")]
    LimitExceeded(usize),
    #[error("unsupported filter: {0}")]
    Unsupported(String),
    /// A run of images has nothing left to spend ([`decode_tolerant_within`]).
    #[error("more image decoding than one operation may do")]
    Spent,
}

/// A decoder failure carrying whatever was decoded before the error.
pub(crate) struct Failure {
    pub(crate) error: FilterError,
    pub(crate) partial: Vec<u8>,
    /// Bytes decoded on the way that `partial` doesn't hold: they took as long.
    pub(crate) discarded: usize,
}

impl Failure {
    pub(crate) fn corrupt(filter: &'static str, detail: impl Into<String>, partial: Vec<u8>) -> Self {
        Failure { error: FilterError::Corrupt { filter, detail: detail.into() }, partial, discarded: 0 }
    }

    pub(crate) fn limit(max: usize) -> Self {
        Failure { error: FilterError::LimitExceeded(max), partial: Vec::new(), discarded: 0 }
    }
}

pub(crate) type Step = Result<Vec<u8>, Failure>;

/// Takes `steps` from `work`, what a run of images may still cost. What it can't pay for spends
/// all of it.
pub(crate) fn pay(work: &mut u64, steps: u64) -> Result<(), FilterError> {
    match work.checked_sub(steps) {
        Some(left) => {
            *work = left;
            Ok(())
        }
        None => {
            *work = 0;
            Err(FilterError::Spent)
        }
    }
}

/// Decodes `data` through `chain` (first element applied first).
///
/// Image codecs, `Crypt` and unknown filters anywhere in the chain give
/// [`FilterError::Unsupported`] (callers keep such data encoded), as does a chain longer than
/// [`MAX_CHAIN`]. `max_output` bounds every intermediate buffer.
pub fn decode(chain: &[(Filter, Params)], data: &[u8], max_output: usize) -> Result<Vec<u8>, FilterError> {
    run(chain, data, max_output, None, true).map(|(v, _, _)| v).map_err(|(e, _)| e)
}

/// Like [`decode`] for a caller charging an allowance: `max_output` bounds the bytes all stages
/// decode together, each stage getting what the earlier ones left, and the result gives those
/// bytes, on success and on failure. A stage's output counts even when a later stage or its
/// predictor shrinks it.
pub fn decode_counted(chain: &[(Filter, Params)], data: &[u8], max_output: usize) -> Result<(Vec<u8>, usize), (FilterError, usize)> {
    run(chain, data, max_output, Some(max_output), true).map(|(v, _, spent)| (v, spent))
}

/// Like [`decode`], but on corrupt data returns what was decoded before the error and
/// reports `partial = true` (the behaviour viewers expect). The remaining filters of the
/// chain still run on the partial data. Limit and unsupported errors are still errors.
pub fn decode_tolerant(chain: &[(Filter, Params)], data: &[u8], max_output: usize) -> Result<(Vec<u8>, bool), FilterError> {
    run(chain, data, max_output, None, false).map(|(v, partial, _)| (v, partial)).map_err(|(e, _)| e)
}

/// [`decode_tolerant`] bounded and counted as [`decode_counted`] is.
pub fn decode_tolerant_counted(chain: &[(Filter, Params)], data: &[u8], max_output: usize) -> Result<(Vec<u8>, bool, usize), (FilterError, usize)> {
    run(chain, data, max_output, Some(max_output), false)
}

/// [`decode_tolerant`], spending from `work`, what decoding may still cost across a run of images:
/// a step for each byte of `data`, paid before anything is decoded, and one for each byte a stage
/// decodes, so a chain that fails, or a stage that goes past `max_output`, has paid for what it did.
/// No stage decodes past what is left: a chain that `work` can't pay for spends all of it and stops
/// there, so with nothing left nothing is read. A chain that can't be decoded at all costs nothing,
/// whatever is left, so what an image's filters are is known from the error.
///
/// Unlike [`decode_tolerant`], no filters at all gives `data` itself, within `max_output`.
pub fn decode_tolerant_within(chain: &[(Filter, Params)], data: &[u8], max_output: usize, work: &mut u64) -> Result<Vec<u8>, FilterError> {
    spending(chain, data, max_output, work, false)
}

/// [`decode`], spending from `work` as [`decode_tolerant_within`] does: corrupt data is an error,
/// having paid for what was decoded before it.
pub fn decode_within(chain: &[(Filter, Params)], data: &[u8], max_output: usize, work: &mut u64) -> Result<Vec<u8>, FilterError> {
    spending(chain, data, max_output, work, true)
}

fn spending(chain: &[(Filter, Params)], data: &[u8], max_output: usize, work: &mut u64, strict: bool) -> Result<Vec<u8>, FilterError> {
    decodable(chain)?;
    pay(work, data.len() as u64)?;
    if chain.is_empty() {
        return if data.len() > max_output { Err(FilterError::LimitExceeded(max_output)) } else { Ok(data.to_vec()) };
    }
    let left = usize::try_from(*work).unwrap_or(usize::MAX);
    match run(chain, data, max_output, Some(left), strict) {
        Ok((out, _, decoded)) => pay(work, decoded as u64).map(|()| out),
        // The stage stopped at the lower of the two, which took as much to find out.
        Err((FilterError::LimitExceeded(_), decoded)) if left.saturating_sub(decoded) > max_output => {
            pay(work, (decoded as u64).saturating_add(max_output as u64))?;
            Err(FilterError::LimitExceeded(max_output))
        }
        Err((FilterError::LimitExceeded(_), _)) => {
            *work = 0;
            Err(FilterError::Spent)
        }
        Err((e, decoded)) => pay(work, decoded as u64).and(Err(e)),
    }
}

/// Longest filter chain decoded. Real streams use one to three filters; every stage may produce
/// up to the output limit, so the chain length multiplies the work.
pub const MAX_CHAIN: usize = 8;

/// Whether a chain is one [`run`] takes, and if not, the error.
fn decodable(chain: &[(Filter, Params)]) -> Result<(), FilterError> {
    if chain.len() > MAX_CHAIN {
        return Err(FilterError::Unsupported(format!("a chain of {} filters", chain.len())));
    }
    match chain.iter().find(|(f, _)| !f.is_decodable()) {
        Some((f, _)) => Err(FilterError::Unsupported(f.name().to_owned())),
        None => Ok(()),
    }
}

/// The decoded data, whether it is partial and the bytes all stages decoded, or the error and
/// the bytes decoded before it. `max` bounds each stage, and `total` all of them together.
fn run(
    chain: &[(Filter, Params)],
    data: &[u8],
    max: usize,
    total: Option<usize>,
    strict: bool,
) -> Result<(Vec<u8>, bool, usize), (FilterError, usize)> {
    decodable(chain).map_err(|e| (e, 0))?;
    let mut cur: Cow<[u8]> = Cow::Borrowed(data);
    let mut partial = false;
    let mut spent = 0usize;
    for (filter, params) in chain {
        #[cfg(test)]
        STAGE_BYTES.with(|c| c.set(c.get() + cur.len()));
        let limit = total.map_or(max, |total| max.min(total.saturating_sub(spent)));
        let (step, work) = decode_one(filter, params, &cur, limit);
        spent = spent.saturating_add(work);
        cur = Cow::Owned(match step {
            Ok(v) => v,
            Err(Failure { error: FilterError::Corrupt { .. }, partial: out, .. }) if !strict => {
                partial = true;
                out
            }
            Err(Failure { error: FilterError::LimitExceeded(_), .. }) => return Err((FilterError::LimitExceeded(max), spent)),
            Err(f) => return Err((f.error, spent)),
        });
    }
    Ok((cur.into_owned(), partial, spent))
}

/// One stage's result and the bytes its decoder produced (before any predictor).
fn decode_one(filter: &Filter, params: &Params, data: &[u8], max: usize) -> (Step, usize) {
    match filter {
        Filter::Flate => with_predictor(flate::decode(data, max), params, "FlateDecode"),
        Filter::Lzw => with_predictor(lzw::decode(data, params.early_change != 0, max), params, "LZWDecode"),
        Filter::AsciiHex => counted(asciihex::decode(data, max)),
        Filter::Ascii85 => counted(ascii85::decode(data, max)),
        Filter::RunLength => counted(runlength::decode(data, max)),
        other => (Err(Failure { error: FilterError::Unsupported(other.name().to_owned()), partial: Vec::new(), discarded: 0 }), 0),
    }
}

/// A step with the bytes it produced: its output, or its partial output and what it threw away.
fn counted(step: Step) -> (Step, usize) {
    let n = match &step {
        Ok(v) => v.len(),
        Err(f) => f.partial.len().saturating_add(f.discarded),
    };
    (step, n)
}

/// Applies the predictor to a decoder's result, including to partial output on failure. The
/// count is the decoder's output: a PNG predictor drops a tag byte per row, but the decoder
/// produced it.
fn with_predictor(step: Step, params: &Params, name: &'static str) -> (Step, usize) {
    let (step, n) = counted(step);
    let step = match step {
        Ok(v) => predictor::decode(params, v, name),
        Err(Failure { error, partial, discarded }) => {
            let partial = match predictor::decode(params, partial, name) {
                Ok(v) => v,
                Err(f) => f.partial,
            };
            Err(Failure { error, partial, discarded })
        }
    };
    (step, n)
}

/// Encodes `data` with `filter`. Supported: Flate (zlib, level 6), LZW, ASCIIHex,
/// ASCII85, RunLength. For Flate and LZW, `params.predictor >= 2` applies the
/// predictor first (TIFF 2, PNG 10–15; 15 chooses the per-row optimum).
pub fn encode(filter: &Filter, params: &Params, data: &[u8]) -> Result<Vec<u8>, FilterError> {
    match filter {
        Filter::Flate => Ok(flate::encode(&predictor::encode(params, data)?)),
        Filter::Lzw => Ok(lzw::encode(&predictor::encode(params, data)?, params.early_change != 0)),
        Filter::AsciiHex => Ok(asciihex::encode(data)),
        Filter::Ascii85 => Ok(ascii85::encode(data)),
        Filter::RunLength => Ok(runlength::encode(data)),
        other => Err(FilterError::Unsupported(format!("encoding with {}", other.name()))),
    }
}

/// zlib (RFC 1950) at compression level 6.
pub fn encode_flate(data: &[u8]) -> Vec<u8> {
    flate::encode(data)
}

/// PDF white-space characters (ISO 32000-2 Table 1).
pub(crate) fn is_pdf_whitespace(b: u8) -> bool {
    matches!(b, 0 | 9 | 10 | 12 | 13 | 32)
}

#[cfg(test)]
thread_local! {
    /// Bytes handed to decoding stages on this thread.
    static STAGE_BYTES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_long_filter_chain_is_refused_before_decoding() {
        // `FF FF` run-length decodes to itself, so every stage of the chain repeats the work.
        let data = vec![0xFF; 4096];
        let chain = |n: usize| vec![(super::Filter::RunLength, super::Params::default()); n];
        super::STAGE_BYTES.with(|c| c.set(0));
        let long = super::decode(&chain(1000), &data, 1 << 20);
        let read = super::STAGE_BYTES.with(|c| c.get());
        assert!(read <= 16 * data.len(), "{read} bytes decoded through the chain");
        assert!(matches!(long, Err(super::FilterError::Unsupported(_))), "{long:?}");
        assert_eq!(super::decode(&chain(super::MAX_CHAIN), &data, 1 << 20).ok(), Some(data));
    }

    use super::*;

    #[test]
    fn names_and_abbreviations() {
        for (full, abbr, f) in [
            (&b"FlateDecode"[..], &b"Fl"[..], Filter::Flate),
            (b"LZWDecode", b"LZW", Filter::Lzw),
            (b"ASCIIHexDecode", b"AHx", Filter::AsciiHex),
            (b"ASCII85Decode", b"A85", Filter::Ascii85),
            (b"RunLengthDecode", b"RL", Filter::RunLength),
            (b"DCTDecode", b"DCT", Filter::Dct),
            (b"CCITTFaxDecode", b"CCF", Filter::CcittFax),
        ] {
            assert_eq!(Filter::from_name(full), f);
            assert_eq!(Filter::from_name(abbr), f);
            assert_eq!(f.name().as_bytes(), full);
        }
        assert_eq!(Filter::from_name(b"/JPXDecode"), Filter::Jpx);
        assert_eq!(Filter::from_name(b"JBIG2Decode"), Filter::Jbig2);
        assert_eq!(Filter::from_name(b"Crypt"), Filter::Crypt);
        assert_eq!(Filter::from_name(b"Foo"), Filter::Unknown("Foo".into()));
        assert_eq!(Filter::Unknown("Foo".into()).name(), "Foo");
        assert!(Filter::Dct.is_image_codec() && Filter::CcittFax.is_image_codec());
        assert!(!Filter::Flate.is_image_codec() && !Filter::Crypt.is_image_codec());
    }

    #[test]
    fn default_params() {
        let p = Params::default();
        assert_eq!((p.predictor, p.colors, p.bits_per_component, p.columns, p.early_change), (1, 1, 8, 1, 1));
    }

    #[test]
    fn unsupported_filters() {
        for f in [Filter::Dct, Filter::Jpx, Filter::Jbig2, Filter::CcittFax, Filter::Crypt, Filter::Unknown("X".into())] {
            let chain = [(Filter::AsciiHex, Params::default()), (f.clone(), Params::default())];
            assert!(matches!(decode(&chain, b"00>", 100), Err(FilterError::Unsupported(_))));
            assert!(matches!(decode_tolerant(&chain, b"00>", 100), Err(FilterError::Unsupported(_))));
            assert!(matches!(encode(&f, &Params::default(), b"x"), Err(FilterError::Unsupported(_))));
        }
    }

    #[test]
    fn empty_chain_is_identity() {
        assert_eq!(decode(&[], b"abc", 1).unwrap(), b"abc");
    }

    #[test]
    fn chain_ascii85_flate() {
        let data = b"Hello, chained filters! Hello, chained filters!".repeat(10);
        let z = encode_flate(&data);
        let a = encode(&Filter::Ascii85, &Params::default(), &z).unwrap();
        let chain = [(Filter::Ascii85, Params::default()), (Filter::Flate, Params::default())];
        assert_eq!(decode(&chain, &a, 1 << 20).unwrap(), data);
    }

    #[test]
    fn chain_three_filters_with_predictor() {
        let p = Params { predictor: 12, colors: 3, columns: 5, ..Params::default() };
        let data: Vec<u8> = (0..300u32).map(|i| (i * 7 % 251) as u8).collect();
        let lzw = encode(&Filter::Lzw, &p, &data).unwrap();
        let rl = encode(&Filter::RunLength, &Params::default(), &lzw).unwrap();
        let hex = encode(&Filter::AsciiHex, &Params::default(), &rl).unwrap();
        let chain = [(Filter::AsciiHex, Params::default()), (Filter::RunLength, Params::default()), (Filter::Lzw, p)];
        assert_eq!(decode(&chain, &hex, 1 << 20).unwrap(), data);
    }

    #[test]
    fn limit_applies_to_intermediate_buffers() {
        let hex = encode(&Filter::AsciiHex, &Params::default(), &[7u8; 100]).unwrap();
        let chain = [(Filter::AsciiHex, Params::default())];
        assert!(matches!(decode(&chain, &hex, 99), Err(FilterError::LimitExceeded(99))));
        assert_eq!(decode(&chain, &hex, 100).unwrap().len(), 100);
    }

    #[test]
    fn raw_flate_fallback_preserves_output_limits() {
        // RFC 1951 stored blocks: the first is nonfinal with LEN=257 and
        // NLEN=!257, followed by a final empty block. Its ignored alignment
        // bits make the first two bytes also look like a valid zlib header.
        let data = vec![b'A'; 257];
        let mut raw = vec![0x78, 0x01, 0x01, 0xfe, 0xfe];
        raw.extend_from_slice(&data);
        raw.extend_from_slice(&[0x01, 0x00, 0x00, 0xff, 0xff]);
        let chain = [(Filter::Flate, Params::default())];
        for max in [0, 1, 128, 256] {
            assert!(matches!(decode(&chain, &raw, max), Err(FilterError::LimitExceeded(m)) if m == max));
            assert!(matches!(decode_tolerant(&chain, &raw, max), Err(FilterError::LimitExceeded(m)) if m == max));
        }
        for max in [257, 258] {
            assert_eq!(decode(&chain, &raw, max).unwrap(), data);
            assert_eq!(decode_tolerant(&chain, &raw, max).unwrap(), (data.clone(), false));
        }
    }

    #[test]
    fn tolerant_flate_returns_partial() {
        let data: Vec<u8> = (0..20_000u32).map(|i| (i % 97) as u8 ^ (i / 300) as u8).collect();
        let mut z = encode_flate(&data);
        let cut = z.len() / 2;
        z.truncate(cut);
        let chain = [(Filter::Flate, Params::default())];
        assert!(matches!(decode(&chain, &z, 1 << 20), Err(FilterError::Corrupt { .. })));
        let (out, partial) = decode_tolerant(&chain, &z, 1 << 20).unwrap();
        assert!(partial);
        assert!(!out.is_empty() && out.len() < data.len());
        assert_eq!(&data[..out.len()], &out[..]);
        // Complete data is not partial.
        let (out, partial) = decode_tolerant(&chain, &encode_flate(&data), 1 << 20).unwrap();
        assert!(!partial);
        assert_eq!(out, data);
    }

    #[test]
    fn tolerant_partial_flows_through_predictor() {
        let p = Params { predictor: 12, columns: 4, ..Params::default() };
        let data: Vec<u8> = (0..4000u32).map(|i| (i * 31 % 256) as u8).collect();
        let mut z = encode(&Filter::Flate, &p, &data).unwrap();
        z.truncate(z.len() - 20);
        let (out, partial) = decode_tolerant(&[(Filter::Flate, p)], &z, 1 << 20).unwrap();
        assert!(partial);
        assert_eq!(&data[..out.len()], &out[..]);
    }

    /// A counted decode bounds what all stages decode together: Flate inflating 1 MiB of hex
    /// that ASCIIHex halves is 1.5 MiB of work, past a 1.25 MiB allowance. A plain decode keeps
    /// the limit per stage.
    #[test]
    fn a_counted_decode_bounds_all_stages_together() {
        let hex: Vec<u8> = std::iter::repeat_n(b"ab".as_slice(), 1 << 19).flatten().copied().collect();
        let data = encode_flate(&hex);
        let chain = [(Filter::Flate, Params::default()), (Filter::AsciiHex, Params::default())];
        let max = (1 << 20) + (1 << 18);
        assert_eq!(decode(&chain, &data, max).map(|d| d.len()).ok(), Some(1 << 19));
        assert!(matches!(decode_counted(&chain, &data, max), Err((FilterError::LimitExceeded(m), _)) if m == max));
        assert_eq!(decode_counted(&chain, &data, 2 << 20).map(|(_, spent)| spent).ok(), Some((1 << 20) + (1 << 19)));
        let tolerant = decode_tolerant_counted(&chain, &data, 2 << 20).map(|(out, partial, spent)| (out.len(), partial, spent));
        assert_eq!(tolerant.ok(), Some((1 << 19, false, (1 << 20) + (1 << 19))));
        assert!(matches!(decode_tolerant_counted(&chain, &data, max), Err((FilterError::LimitExceeded(m), _)) if m == max));
    }

    /// A PNG predictor drops a tag byte per row, but Flate produced it: the count includes it,
    /// also when the predictor fails on a bad tag.
    #[test]
    fn a_counted_decode_counts_output_before_the_predictor() {
        let params = Params { predictor: 12, ..Params::default() };
        let rows = vec![7u8; 1 << 16];
        let data = encode(&Filter::Flate, &params, &rows).unwrap();
        let chain = [(Filter::Flate, params.clone())];
        let (out, spent) = decode_counted(&chain, &data, usize::MAX).unwrap();
        assert!(out == rows);
        assert_eq!(spent, 2 << 16);
        let bad: Vec<u8> = std::iter::repeat_n([0u8, 7], 1 << 15).flatten().chain([9, 7]).collect();
        let (e, spent) = decode_counted(&chain, &encode_flate(&bad), usize::MAX).unwrap_err();
        assert!(matches!(e, FilterError::Corrupt { .. }), "{e}");
        assert_eq!(spent, bad.len());
    }

    fn refused(e: &FilterError) -> bool {
        matches!(e, FilterError::Spent)
    }

    /// A chain pays for what it reads and what each stage decodes, down to the byte: Flate inflates
    /// hex that ASCIIHex halves, so a stage counts when a later one shrinks it.
    #[test]
    fn a_chain_spends_down_to_the_byte() {
        let hex = b"ab".repeat(1 << 10);
        let data = encode_flate(&hex);
        let chain = [(Filter::Flate, Params::default()), (Filter::AsciiHex, Params::default())];
        let whole = decode(&chain, &data, 1 << 20).unwrap();
        let cost = (data.len() + hex.len() + whole.len()) as u64;
        let mut work = u64::MAX;
        assert_eq!(decode_tolerant_within(&chain, &data, 1 << 20, &mut work).unwrap(), whole);
        assert_eq!(u64::MAX - work, cost);
        let mut work = cost;
        assert_eq!(decode_tolerant_within(&chain, &data, 1 << 20, &mut work).unwrap(), whole, "exactly enough");
        assert_eq!(work, 0);
        let mut work = cost + 7;
        decode_tolerant_within(&chain, &data, 1 << 20, &mut work).unwrap();
        assert_eq!(work, 7, "only what it took");
        let mut work = cost - 1;
        assert!(refused(&decode_tolerant_within(&chain, &data, 1 << 20, &mut work).unwrap_err()), "a step short");
        assert_eq!(work, 0, "spends all that was left");
        // Each stage has `max_output` to itself, as in a plain decode: together they are past it.
        let mut work = u64::MAX;
        assert_eq!(decode_tolerant_within(&chain, &data, hex.len(), &mut work).unwrap(), whole);
    }

    /// No stage runs past what is left: the first stage of this bomb stops there, so the second
    /// isn't given anything to read, and with too little to read it nothing runs at all.
    #[test]
    fn a_chain_stops_at_what_is_left() {
        let inner = encode_flate(&vec![0; 1 << 20]);
        let bomb = encode_flate(&inner);
        let chain = [(Filter::Flate, Params::default()), (Filter::Flate, Params::default())];
        let read = |work: u64| {
            super::STAGE_BYTES.with(|c| c.set(0));
            let mut work = work;
            assert!(refused(&decode_tolerant_within(&chain, &bomb, usize::MAX, &mut work).unwrap_err()));
            assert_eq!(work, 0);
            super::STAGE_BYTES.with(|c| c.get())
        };
        assert_eq!(read((bomb.len() + inner.len()) as u64 + 10), bomb.len() + inner.len(), "the second stage stops");
        assert_eq!(read(bomb.len() as u64 + 10), bomb.len(), "the first stops, and the second never runs");
        assert_eq!(read(bomb.len() as u64 - 1), 0, "it can't pay to be read");
    }

    /// Past `max_output` the chain has failed, and hasn't run out of anything: it pays what it took
    /// to find out, and what is left stays. Whichever limit a stage stops at is the one reported.
    #[test]
    fn a_stage_past_its_limit_pays_for_finding_out() {
        let data = encode_flate(&vec![0; 1 << 16]);
        let chain = [(Filter::Flate, Params::default())];
        let mut work = u64::MAX;
        assert!(matches!(decode_tolerant_within(&chain, &data, 1000, &mut work), Err(FilterError::LimitExceeded(1000))));
        assert_eq!(u64::MAX - work, data.len() as u64 + 1000);
        // The later stage of two, which pays for the first as well.
        let bomb = encode_flate(&data);
        let two = [(Filter::Flate, Params::default()), (Filter::Flate, Params::default())];
        let mut work = u64::MAX;
        assert!(matches!(decode_tolerant_within(&two, &bomb, data.len(), &mut work), Err(FilterError::LimitExceeded(_))));
        assert_eq!(u64::MAX - work, (bomb.len() + 2 * data.len()) as u64);
        // Both limits at once: a run that can't go on is what a caller needs to hear.
        let mut work = data.len() as u64 + 1000;
        assert!(refused(&decode_tolerant_within(&chain, &data, 1000, &mut work).unwrap_err()));
        assert_eq!(work, 0);
        let mut work = data.len() as u64 + 1001;
        assert!(matches!(decode_tolerant_within(&chain, &data, 1000, &mut work), Err(FilterError::LimitExceeded(1000))));
        assert_eq!(work, 1, "the image's limit, with a step to spare");
    }

    /// What a damaged stream decoded is paid for, however the decode ends.
    #[test]
    fn a_chain_that_fails_has_paid_for_what_it_did() {
        let data: Vec<u8> = (0..20_000u32).map(|i| (i % 97) as u8 ^ (i / 300) as u8).collect();
        let mut cut = encode_flate(&data);
        cut.truncate(cut.len() / 2);
        let chain = [(Filter::Flate, Params::default())];
        let (partial, _) = decode_tolerant(&chain, &cut, 1 << 20).unwrap();
        assert!(!partial.is_empty());
        let mut work = u64::MAX;
        assert_eq!(decode_tolerant_within(&chain, &cut, 1 << 20, &mut work).unwrap(), partial);
        assert_eq!(u64::MAX - work, (cut.len() + partial.len()) as u64);
        // Read strictly it is an error, for the same work.
        let mut work = u64::MAX;
        assert!(matches!(decode_within(&chain, &cut, 1 << 20, &mut work), Err(FilterError::Corrupt { .. })));
        assert_eq!(u64::MAX - work, (cut.len() + partial.len()) as u64);
        let whole = encode_flate(&data);
        let mut work = u64::MAX;
        assert_eq!(decode_within(&chain, &whole, 1 << 20, &mut work).unwrap(), data);
        assert_eq!(u64::MAX - work, (whole.len() + data.len()) as u64);
        // A predictor that comes to nothing, after all of this was inflated for it.
        let p = Params { predictor: 99, ..Params::default() };
        let data = encode_flate(&vec![0; 1 << 16]);
        let mut work = u64::MAX;
        assert!(decode_tolerant_within(&[(Filter::Flate, p)], &data, 1 << 20, &mut work).unwrap().is_empty());
        assert_eq!(u64::MAX - work, data.len() as u64 + (1 << 16));
    }

    /// A stream that reads as zlib only until it fails, and as raw deflate until that fails too,
    /// gives back no data, and has still decoded everything the retry did.
    #[test]
    fn a_retry_that_fails_pays_for_what_it_decoded() {
        // Stored as raw deflate: this many bytes, then a block of a reserved type. As zlib: the
        // same reserved type at once, so the first reading fails with nothing.
        let mut data = vec![0x78, 0x01, 0x06, 0xFE, 0xF9];
        data.extend(vec![0u8; 0x0601]);
        data.push(0x07);
        let chain = [(Filter::Flate, Params::default())];
        let (out, partial) = decode_tolerant(&chain, &data, 1 << 20).unwrap();
        assert!(out.is_empty() && partial, "what is read stays as it was");
        let (_, _, spent) = decode_tolerant_counted(&chain, &data, 1 << 20).unwrap();
        assert_eq!(spent, 0x0601);
        let (e, spent) = decode_counted(&chain, &data, 1 << 20).unwrap_err();
        assert!(matches!(e, FilterError::Corrupt { .. }), "{e}");
        assert_eq!(spent, 0x0601);
        let mut work = u64::MAX;
        assert!(decode_tolerant_within(&chain, &data, 1 << 20, &mut work).unwrap().is_empty());
        assert_eq!(u64::MAX - work, (data.len() + 0x0601) as u64);
        // Through a predictor, which would otherwise lose the count.
        let p = Params { predictor: 12, columns: 4, ..Params::default() };
        let (_, _, spent) = decode_tolerant_counted(&[(Filter::Flate, p)], &data, 1 << 20).unwrap();
        assert_eq!(spent, 0x0601);
    }

    /// With nothing left nothing is read. What can't be decoded at all never costs anything, and
    /// says so whatever is left. Without filters, what is read is the data.
    #[test]
    fn what_a_chain_is_shows_whatever_is_left() {
        let flate = [(Filter::Flate, Params::default())];
        let mut work = 0;
        assert!(refused(&decode_tolerant_within(&flate, b"garbage", 1 << 20, &mut work).unwrap_err()));
        let long = vec![(Filter::Flate, Params::default()); MAX_CHAIN + 1];
        for chain in [vec![(Filter::Dct, Params::default())], vec![(Filter::Jbig2, Params::default())], long] {
            for left in [0, 100] {
                let mut work = left;
                let e = decode_tolerant_within(&chain, b"data", 1 << 20, &mut work).unwrap_err();
                assert!(matches!(e, FilterError::Unsupported(_)) && !refused(&e), "{e}");
                assert_eq!(work, left);
            }
        }
        let mut work = 100;
        assert_eq!(decode_tolerant_within(&[], b"data", 4, &mut work).unwrap(), b"data");
        assert_eq!(work, 96);
        assert!(matches!(decode_tolerant_within(&[], b"data", 3, &mut work), Err(FilterError::LimitExceeded(3))));
        assert_eq!(work, 92);
        let mut work = 3;
        assert!(refused(&decode_tolerant_within(&[], b"data", 4, &mut work).unwrap_err()));
        assert_eq!(work, 0);
    }
}

//! The PDF object model (ISO 32000-2 §7.3).
//!
//! Dictionaries keep their key order so rewritten objects stay recognisable in diffs and
//! byte-stable when re-serialized. Streams keep their *encoded* bytes; decoding is on demand.

use pdfcraft_filters::{Filter, Params};

use crate::{Bytes, CosError};

/// An indirect reference: object number and generation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ObjRef {
    pub num: u32,
    pub generation: u16,
}

impl ObjRef {
    pub const fn new(num: u32, generation: u16) -> Self {
        Self { num, generation }
    }
}

impl std::fmt::Display for ObjRef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} {} R", self.num, self.generation)
    }
}

/// A string object. `hex` records how it was written, so it is re-emitted the same way.
#[derive(Clone, Debug, PartialEq)]
pub struct PdfString {
    pub bytes: Vec<u8>,
    pub hex: bool,
}

impl PdfString {
    pub fn literal(bytes: impl Into<Vec<u8>>) -> Self {
        Self { bytes: bytes.into(), hex: false }
    }

    /// A text string (ISO 32000-2 §7.9.2.2): PDFDocEncoding when possible, else UTF-16BE with BOM.
    pub fn text(s: &str) -> Self {
        if s.chars().all(|c| (' '..='~').contains(&c) || c == '\n' || c == '\r' || c == '\t') {
            return Self::literal(s.as_bytes().to_vec());
        }
        let mut bytes = vec![0xFE, 0xFF];
        for u in s.encode_utf16() {
            bytes.extend_from_slice(&u.to_be_bytes());
        }
        Self { bytes, hex: false }
    }

    /// Decode a text string: UTF-16BE (BOM), UTF-8 (BOM, PDF 2.0) or PDFDocEncoding.
    ///
    /// Lenient beyond the spec, as readers are: invalid sequences after a BOM decode lossily
    /// instead of dropping the string, a BOM-less string whose bytes are valid UTF-8 with
    /// non-ASCII bytes (written by some producers) is read as UTF-8 rather than as
    /// PDFDocEncoding mojibake, and stray leading U+FEFF (doubled BOMs) are removed.
    pub fn to_text(&self) -> String {
        let s = match self.bytes.as_slice() {
            [0xFE, 0xFF, rest @ ..] => {
                let units: Vec<u16> = rest.as_chunks::<2>().0.iter().map(|c| u16::from_be_bytes(*c)).collect();
                String::from_utf16_lossy(&units)
            }
            [0xEF, 0xBB, 0xBF, rest @ ..] => String::from_utf8_lossy(rest).into_owned(),
            b => match std::str::from_utf8(b) {
                // Pure ASCII is identical either way; only non-ASCII UTF-8 needs the switch.
                Ok(utf8) if !b.is_ascii() => utf8.to_owned(),
                _ => b.iter().map(|&c| pdfcraft_crypt::pdfdoc_char(c)).collect(),
            },
        };
        if s.starts_with('\u{FEFF}') { s.trim_start_matches('\u{FEFF}').to_owned() } else { s }
    }
}

/// A name object (without the leading `/`, with `#xx` escapes already decoded).
pub type Name = Vec<u8>;

#[cfg(test)]
thread_local! {
    /// Key comparisons and index lookups made by [`Dict`] on this thread.
    static KEY_COMPARES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// Entries from which a [`Dict`] keeps a key index. Smaller dictionaries (nearly all of them)
/// search their entries; a parsed file controls the key count, and a linear search per
/// inserted key would make a dictionary of many distinct keys quadratic to build.
const INDEX_FROM: usize = 32;

/// An ordered dictionary.
#[derive(Clone, Default)]
pub struct Dict {
    entries: Vec<(Name, Object)>,
    /// Key to position in `entries`, kept once the dictionary reaches [`INDEX_FROM`] entries.
    /// Boxed so that every `Dict`, and so every `Object`, grows by one pointer, not by a map.
    #[allow(clippy::box_collection)]
    index: Option<Box<std::collections::HashMap<Name, usize>>>,
}

impl PartialEq for Dict {
    fn eq(&self, other: &Self) -> bool {
        self.entries == other.entries
    }
}

impl std::fmt::Debug for Dict {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Dict").field("entries", &self.entries).finish()
    }
}

impl Dict {
    pub fn new() -> Self {
        Self::default()
    }

    fn position(&self, key: &[u8]) -> Option<usize> {
        if let Some(index) = &self.index {
            #[cfg(test)]
            KEY_COMPARES.with(|c| c.set(c.get() + 1));
            return index.get(key).copied();
        }
        self.entries.iter().position(|(k, _)| {
            #[cfg(test)]
            KEY_COMPARES.with(|c| c.set(c.get() + 1));
            k == key
        })
    }

    pub fn get(&self, key: &[u8]) -> Option<&Object> {
        self.position(key).and_then(|i| self.entries.get(i)).map(|(_, v)| v)
    }

    pub fn get_mut(&mut self, key: &[u8]) -> Option<&mut Object> {
        let i = self.position(key)?;
        self.entries.get_mut(i).map(|(_, v)| v)
    }

    /// Insert or replace, keeping the position of an existing key.
    pub fn set(&mut self, key: impl Into<Name>, value: impl Into<Object>) {
        let key = key.into();
        let value = value.into();
        match self.position(&key).and_then(|i| self.entries.get_mut(i)) {
            Some((_, v)) => *v = value,
            None => {
                let at = self.entries.len();
                if let Some(index) = &mut self.index {
                    index.insert(key.clone(), at);
                } else if at + 1 >= INDEX_FROM {
                    let index = self.entries.iter().enumerate().map(|(i, (k, _))| (k.clone(), i)).chain([(key.clone(), at)]).collect();
                    self.index = Some(Box::new(index));
                }
                self.entries.push((key, value));
            }
        }
    }

    pub fn remove(&mut self, key: &[u8]) -> Option<Object> {
        let i = self.position(key)?;
        if let Some(index) = &mut self.index {
            index.remove(key);
            index.values_mut().filter(|at| **at > i).for_each(|at| *at -= 1);
        }
        Some(self.entries.remove(i).1)
    }

    pub fn contains(&self, key: &[u8]) -> bool {
        self.get(key).is_some()
    }

    pub fn iter(&self) -> impl Iterator<Item = (&Name, &Object)> {
        self.entries.iter().map(|(k, v)| (k, v))
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn name(&self, key: &[u8]) -> Option<&[u8]> {
        self.get(key).and_then(Object::as_name)
    }

    pub fn int(&self, key: &[u8]) -> Option<i64> {
        self.get(key).and_then(Object::as_int)
    }

    pub fn reference(&self, key: &[u8]) -> Option<ObjRef> {
        self.get(key).and_then(Object::as_ref)
    }
}

impl FromIterator<(Name, Object)> for Dict {
    fn from_iter<T: IntoIterator<Item = (Name, Object)>>(iter: T) -> Self {
        let mut d = Dict::new();
        for (k, v) in iter {
            d.set(k, v);
        }
        d
    }
}

/// A stream: dictionary plus its data *as stored* (still filtered).
#[derive(Clone, Debug, PartialEq)]
pub struct Stream {
    pub dict: Dict,
    pub raw: Bytes,
}

/// Upper bound for a single decoded stream (decompression-bomb defence).
pub const MAX_DECODED: usize = 1 << 30;

/// The most an image's `/JBIG2Globals` stream may decode to: the symbol and pattern dictionaries an
/// encoder shares between pages, themselves coded, so far less than any stream may decode to
/// ([`MAX_DECODED`]). They are decoded before the image's JBIG2 page is read, so the page's size
/// can't bound them: they have a limit of their own.
pub const MAX_JBIG2_GLOBALS: usize = 32 << 20;

impl Stream {
    /// A new stream from already-encoded bytes (`/Length` is set at write time).
    pub fn from_raw(dict: Dict, raw: Vec<u8>) -> Self {
        Self { dict, raw: raw.into() }
    }

    /// A new stream holding `data` compressed with Flate.
    pub fn flate(mut dict: Dict, data: &[u8]) -> Self {
        dict.set(b"Filter".to_vec(), Object::Name(b"FlateDecode".to_vec()));
        dict.remove(b"DecodeParms");
        Self { dict, raw: pdfcraft_filters::encode_flate(data).into() }
    }

    /// The filter chain declared in the dictionary.
    pub fn filters(&self) -> Vec<(Filter, Params)> {
        let names: Vec<&[u8]> = match self.dict.get(b"Filter") {
            Some(Object::Name(n)) => vec![n],
            Some(Object::Array(a)) => a.iter().filter_map(Object::as_name).collect(),
            _ => Vec::new(),
        };
        let parms: Vec<Option<&Dict>> = match self.dict.get(b"DecodeParms") {
            Some(Object::Dict(d)) => vec![Some(d)],
            Some(Object::Array(a)) => a.iter().map(Object::as_dict).collect(),
            _ => Vec::new(),
        };
        names
            .into_iter()
            .enumerate()
            // `/Crypt` is applied by the security handler when the object is loaded (§7.4.10).
            .filter(|(_, n)| *n != b"Crypt")
            .map(|(i, n)| {
                let p = parms.get(i).copied().flatten();
                let get = |k: &[u8], d: i64| p.and_then(|p| p.int(k)).unwrap_or(d);
                let params = Params {
                    predictor: get(b"Predictor", 1),
                    colors: get(b"Colors", 1),
                    bits_per_component: get(b"BitsPerComponent", 8),
                    columns: get(b"Columns", 1),
                    early_change: get(b"EarlyChange", 1),
                };
                (Filter::from_name(n), params)
            })
            .collect()
    }

    /// [`Stream::filters`], or an error when `/Filter` is present but is not a name or an
    /// array of names (an indirect reference, say): the data is not unfiltered, so it must not
    /// read as decoded.
    pub(crate) fn readable_filters(&self) -> Result<Vec<(Filter, Params)>, pdfcraft_filters::FilterError> {
        let readable = match self.dict.get(b"Filter") {
            None | Some(Object::Null | Object::Name(_)) => true,
            Some(Object::Array(a)) => a.iter().all(|f| f.as_name().is_some()),
            Some(_) => false,
        };
        if !readable {
            return Err(pdfcraft_filters::FilterError::Unsupported("/Filter is not a name or an array of names".into()));
        }
        Ok(self.filters())
    }

    /// Decoded data, tolerating truncated or corrupt encodings the way viewers do (the data
    /// decoded before the damage is returned). Streams with image codecs fail with
    /// `CosError::Filter` (keep them encoded). Use `decoded_strict` to detect damage.
    pub fn decoded(&self) -> Result<Vec<u8>, CosError> {
        self.decoded_within(MAX_DECODED)
    }

    /// Like [`Stream::decoded`], for a stream whose decoded size has a tighter bound than
    /// [`MAX_DECODED`]: decoding past `max` bytes is an error.
    pub fn decoded_within(&self, max: usize) -> Result<Vec<u8>, CosError> {
        let decode = |chain: &[(Filter, Params)], data: &[u8], max| {
            pdfcraft_filters::decode_tolerant(chain, data, max).map(|(v, _)| (v, 0)).map_err(|e| (e, 0))
        };
        self.decode_chain(max, decode).map(|(v, _)| v).map_err(|(e, _)| CosError::Filter(e.to_string()))
    }

    /// [`Stream::decoded_within`] for a caller charging an allowance: the filter error is kept
    /// typed, so a caller can tell a spent limit from corrupt data, `max` bounds all filter
    /// stages together, and the result gives the bytes they decoded (the work done, on success
    /// and on failure).
    pub(crate) fn decoded_within_counted(&self, max: usize) -> Result<(Vec<u8>, usize), (pdfcraft_filters::FilterError, usize)> {
        let decode = |chain: &[(Filter, Params)], data: &[u8], max| {
            pdfcraft_filters::decode_tolerant_counted(chain, data, max).map(|(v, _, spent)| (v, spent))
        };
        self.decode_chain(max, decode)
    }

    /// [`Stream::decoded_within`] spending from `work`, what an operation may still do across a
    /// document's images ([`pdfcraft_filters::decode_tolerant_within`]).
    pub(crate) fn decoded_spending(&self, max: usize, work: &mut u64) -> Result<Vec<u8>, CosError> {
        let filter = |e: pdfcraft_filters::FilterError| CosError::Filter(e.to_string());
        let chain = self.readable_filters().map_err(filter)?;
        pdfcraft_filters::decode_tolerant_within(&chain, &self.raw, max.min(MAX_DECODED), work).map_err(filter)
    }

    /// The stream decoded through its filters by `decode`, or copied when it has none, within
    /// `max` and [`MAX_DECODED`].
    fn decode_chain(
        &self,
        max: usize,
        decode: impl FnOnce(&[(Filter, Params)], &[u8], usize) -> Result<(Vec<u8>, usize), (pdfcraft_filters::FilterError, usize)>,
    ) -> Result<(Vec<u8>, usize), (pdfcraft_filters::FilterError, usize)> {
        let max = max.min(MAX_DECODED);
        let chain = self.readable_filters().map_err(|e| (e, 0))?;
        if chain.is_empty() {
            return self
                .raw_within_typed(max)
                .map(|v| {
                    let n = v.len();
                    (v, n)
                })
                .map_err(|e| (e, 0));
        }
        decode(&chain, &self.raw, max)
    }

    /// Decoded data, bounded by [`MAX_DECODED`]; any corruption is an error.
    pub fn decoded_strict(&self) -> Result<Vec<u8>, CosError> {
        let decode = |chain: &[(Filter, Params)], data: &[u8], max| pdfcraft_filters::decode(chain, data, max).map(|v| (v, 0)).map_err(|e| (e, 0));
        self.decode_chain(MAX_DECODED, decode).map(|(v, _)| v).map_err(|(e, _)| CosError::Filter(e.to_string()))
    }

    /// [`Stream::decoded_strict`] bounded by `max` and counted like
    /// [`Stream::decoded_within_counted`].
    pub(crate) fn decoded_strict_within_counted(&self, max: usize) -> Result<(Vec<u8>, usize), (pdfcraft_filters::FilterError, usize)> {
        self.decode_chain(max, pdfcraft_filters::decode_counted)
    }

    /// Data after the first filter alone, bounded by `max`: the rest of the chain stays encoded.
    /// Any corruption in that first filter is an error.
    pub fn decoded_first_filter_strict(&self, max: usize) -> Result<Vec<u8>, CosError> {
        let mut chain = self.readable_filters().map_err(|e| CosError::Filter(e.to_string()))?;
        chain.truncate(1);
        if chain.is_empty() {
            return self.raw_within(max);
        }
        pdfcraft_filters::decode(&chain, &self.raw, max.min(MAX_DECODED)).map_err(|e| CosError::Filter(e.to_string()))
    }

    fn raw_within(&self, max: usize) -> Result<Vec<u8>, CosError> {
        self.raw_within_typed(max).map_err(|e| CosError::Filter(e.to_string()))
    }

    fn raw_within_typed(&self, max: usize) -> Result<Vec<u8>, pdfcraft_filters::FilterError> {
        let max = max.min(MAX_DECODED);
        // An empty decoder chain still produces an owned output buffer: check its limit
        // before cloning, including streams whose /Crypt filter was applied on load.
        if self.raw.len() > max {
            return Err(pdfcraft_filters::FilterError::LimitExceeded(max));
        }
        Ok(self.raw.to_vec())
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Object {
    Null,
    Bool(bool),
    Int(i64),
    Real(f64),
    String(PdfString),
    Name(Name),
    Array(Vec<Object>),
    Dict(Dict),
    Stream(Stream),
    Ref(ObjRef),
}

impl Object {
    pub fn name(n: &str) -> Self {
        Object::Name(n.as_bytes().to_vec())
    }

    pub fn as_int(&self) -> Option<i64> {
        match self {
            Object::Int(i) => Some(*i),
            Object::Real(r) if r.fract() == 0.0 && r.is_finite() => Some(*r as i64),
            _ => None,
        }
    }

    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Object::Int(i) => Some(*i as f64),
            Object::Real(r) => Some(*r),
            _ => None,
        }
    }

    pub fn as_name(&self) -> Option<&[u8]> {
        match self {
            Object::Name(n) => Some(n),
            _ => None,
        }
    }

    pub fn as_ref(&self) -> Option<ObjRef> {
        match self {
            Object::Ref(r) => Some(*r),
            _ => None,
        }
    }

    pub fn as_dict(&self) -> Option<&Dict> {
        match self {
            Object::Dict(d) => Some(d),
            Object::Stream(s) => Some(&s.dict),
            _ => None,
        }
    }

    pub fn as_dict_mut(&mut self) -> Option<&mut Dict> {
        match self {
            Object::Dict(d) => Some(d),
            Object::Stream(s) => Some(&mut s.dict),
            _ => None,
        }
    }

    pub fn as_array(&self) -> Option<&Vec<Object>> {
        match self {
            Object::Array(a) => Some(a),
            _ => None,
        }
    }

    pub fn as_string(&self) -> Option<&PdfString> {
        match self {
            Object::String(s) => Some(s),
            _ => None,
        }
    }
}

impl From<i64> for Object {
    fn from(v: i64) -> Self {
        Object::Int(v)
    }
}
impl From<i32> for Object {
    fn from(v: i32) -> Self {
        Object::Int(v as i64)
    }
}
impl From<f64> for Object {
    fn from(v: f64) -> Self {
        Object::Real(v)
    }
}
impl From<bool> for Object {
    fn from(v: bool) -> Self {
        Object::Bool(v)
    }
}
impl From<ObjRef> for Object {
    fn from(v: ObjRef) -> Self {
        Object::Ref(v)
    }
}
impl From<Dict> for Object {
    fn from(v: Dict) -> Self {
        Object::Dict(v)
    }
}
impl From<Vec<Object>> for Object {
    fn from(v: Vec<Object>) -> Self {
        Object::Array(v)
    }
}
impl From<PdfString> for Object {
    fn from(v: PdfString) -> Self {
        Object::String(v)
    }
}
impl From<Stream> for Object {
    fn from(v: Stream) -> Self {
        Object::Stream(v)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_filter_that_is_not_a_direct_name_is_not_decoded_as_absent() {
        let stream = |filter: Object| {
            let mut d = Dict::new();
            d.set(b"Filter".to_vec(), filter);
            Stream::from_raw(d, b"48 69>".to_vec())
        };
        let hex = Object::Name(b"ASCIIHexDecode".to_vec());
        assert_eq!(stream(hex.clone()).decoded_strict().ok(), Some(b"Hi".to_vec()));
        assert_eq!(stream(Object::Array(vec![hex.clone()])).decoded().ok(), Some(b"Hi".to_vec()));
        assert_eq!(stream(Object::Null).decoded_strict().ok(), Some(b"48 69>".to_vec()));
        for filter in [
            Object::Ref(ObjRef::new(5, 0)),
            Object::Array(vec![Object::Ref(ObjRef::new(5, 0))]),
            Object::Int(3),
            Object::Array(vec![hex, Object::Int(1)]),
        ] {
            let s = stream(filter.clone());
            assert!(s.decoded_strict().is_err(), "{filter:?}");
            assert!(s.decoded().is_err(), "{filter:?}");
            assert!(s.decoded_first_filter_strict(1 << 20).is_err(), "{filter:?}");
        }
    }

    #[test]
    fn parsing_many_distinct_keys_is_not_quadratic() {
        let n = 20_000;
        let body: String = (0..n).map(|i| format!("/K{i} {i} ")).collect();
        let text = format!("<< {body}/K7 true >>");
        KEY_COMPARES.with(|c| c.set(0));
        let Ok(Object::Dict(d)) = crate::parser::Lexer::new(text.as_bytes(), 0).object() else { panic!("not a dict") };
        let compares = KEY_COMPARES.with(|c| c.get());
        assert!(compares < 8 * n, "{compares} key comparisons for {n} keys");
        // Order and replace-in-place semantics stay those of a plain ordered dictionary.
        assert_eq!(d.len(), n);
        assert_eq!(d.iter().next().map(|(k, _)| k.as_slice()), Some(&b"K0"[..]));
        assert_eq!(d.get(b"K7"), Some(&Object::Bool(true)));
        assert_eq!(d.iter().nth(7).map(|(_, v)| v), Some(&Object::Bool(true)));
        let mut d = d;
        assert_eq!(d.remove(b"K3"), Some(Object::Int(3)));
        assert_eq!(d.get(b"K4"), Some(&Object::Int(4)));
        assert_eq!(d.iter().nth(3).map(|(k, _)| k.as_slice()), Some(&b"K4"[..]));
        d.set(b"K3".to_vec(), Object::Null);
        assert_eq!(d.iter().last().map(|(k, _)| k.as_slice()), Some(&b"K3"[..]));
        assert!(d.get(b"K20000").is_none());
    }

    #[test]
    fn text_strings_round_trip() {
        for s in ["Plain ASCII", "Café — naïve", "日本語", "emoji 📄"] {
            assert_eq!(PdfString::text(s).to_text(), s);
        }
        assert!(!PdfString::text("abc").bytes.starts_with(&[0xFE, 0xFF]));
    }

    #[test]
    fn text_string_encodings() {
        // UTF-16BE with BOM.
        let mut utf16 = vec![0xFE, 0xFF];
        utf16.extend("第一章 概述".encode_utf16().flat_map(u16::to_be_bytes));
        assert_eq!(PdfString::literal(utf16).to_text(), "第一章 概述");
        // UTF-8 with BOM (PDF 2.0): the BOM is not part of the text.
        let mut utf8 = vec![0xEF, 0xBB, 0xBF];
        utf8.extend_from_slice("第一章".as_bytes());
        assert_eq!(PdfString::literal(utf8).to_text(), "第一章");
        // Invalid UTF-8 after the BOM decodes lossily instead of vanishing.
        assert_eq!(PdfString::literal(vec![0xEF, 0xBB, 0xBF, b'A', 0xFF, b'B']).to_text(), "A\u{FFFD}B");
        // BOM-less raw UTF-8 from producers that skip the BOM.
        assert_eq!(PdfString::literal("目录".as_bytes()).to_text(), "目录");
        // PDFDocEncoding Latin-1 (not valid UTF-8) stays PDFDocEncoding.
        assert_eq!(PdfString::literal(vec![b'C', b'a', b'f', 0xE9]).to_text(), "Café");
        // Doubled BOMs leave no U+FEFF behind.
        assert_eq!(PdfString::literal(vec![0xFE, 0xFF, 0xFE, 0xFF, 0x00, b'A']).to_text(), "A");
        assert_eq!(PdfString::literal(vec![0xEF, 0xBB, 0xBF, 0xEF, 0xBB, 0xBF, b'A']).to_text(), "A");
        // Odd trailing byte and empty strings never panic.
        assert_eq!(PdfString::literal(vec![0xFE, 0xFF, 0x00, b'A', 0x00]).to_text(), "A");
        assert_eq!(PdfString::literal(vec![0xFE, 0xFF]).to_text(), "");
        assert_eq!(PdfString::literal(Vec::new()).to_text(), "");
    }

    #[test]
    fn pdfdoc_encoding_high_range() {
        assert_eq!(PdfString::literal(vec![0x84, 0x92, 0xA0, b'A']).to_text(), "—™€A");
    }

    #[test]
    fn dict_preserves_order_and_replaces_in_place() {
        let mut d = Dict::new();
        d.set(b"B".to_vec(), 1);
        d.set(b"A".to_vec(), 2);
        d.set(b"B".to_vec(), 3);
        let keys: Vec<_> = d.iter().map(|(k, _)| k.clone()).collect();
        assert_eq!(keys, vec![b"B".to_vec(), b"A".to_vec()]);
        assert_eq!(d.int(b"B"), Some(3));
    }

    #[test]
    fn raw_streams_refuse_decoding_past_a_caller_limit() {
        let stream = Stream::from_raw(Dict::new(), b"abc".to_vec());
        for max in [0, 2] {
            assert!(matches!(stream.decoded_within(max), Err(CosError::Filter(_))));
        }
    }

    #[test]
    fn empty_filter_arrays_refuse_decoding_past_a_caller_limit() {
        let mut dict = Dict::new();
        dict.set(b"Filter".to_vec(), Object::Array(Vec::new()));
        let stream = Stream::from_raw(dict, b"abc".to_vec());
        assert!(matches!(stream.decoded_within(2), Err(CosError::Filter(_))));
    }

    #[test]
    fn crypt_only_streams_refuse_decoding_past_a_caller_limit() {
        let mut dict = Dict::new();
        dict.set(b"Filter".to_vec(), Object::name("Crypt"));
        let mut parms = Dict::new();
        parms.set(b"Name".to_vec(), Object::name("Identity"));
        dict.set(b"DecodeParms".to_vec(), Object::Dict(parms));
        let stream = Stream::from_raw(dict, b"abc".to_vec());
        assert!(matches!(stream.decoded_within(2), Err(CosError::Filter(_))));
    }

    #[test]
    fn identity_streams_preserve_bytes_within_a_caller_limit() {
        let mut array = Dict::new();
        array.set(b"Filter".to_vec(), Object::Array(Vec::new()));
        let mut crypt = Dict::new();
        crypt.set(b"Filter".to_vec(), Object::name("Crypt"));
        for dict in [Dict::new(), array, crypt] {
            let stream = Stream::from_raw(dict.clone(), b"abc".to_vec());
            assert_eq!(stream.decoded_within(3).unwrap(), b"abc");
            assert_eq!(stream.decoded_within(usize::MAX).unwrap(), b"abc");
            assert_eq!(stream.decoded().unwrap(), b"abc");
            assert_eq!(stream.decoded_strict().unwrap(), b"abc");
            assert!(Stream::from_raw(dict, Vec::new()).decoded_within(0).unwrap().is_empty());
        }
    }

    #[test]
    fn filtered_streams_preserve_the_caller_limit() {
        let stream = Stream::flate(Dict::new(), b"abc");
        assert!(matches!(stream.decoded_within(2), Err(CosError::Filter(_))));
        assert_eq!(stream.decoded_within(3).unwrap(), b"abc");
        assert_eq!(stream.decoded_strict().unwrap(), b"abc");
    }

    #[test]
    fn raw_flate_fallback_streams_preserve_the_caller_limit() {
        // Raw deflate with a stored byte and a final empty block. 78 01 also
        // passes the zlib header check, so decoding must retry the whole input.
        let mut dict = Dict::new();
        dict.set(b"Filter".to_vec(), Object::name("FlateDecode"));
        let stream = Stream::from_raw(dict, vec![0x78, 0x01, 0x00, 0xfe, 0xff, b'A', 0x01, 0x00, 0x00, 0xff, 0xff]);
        let result = stream.decoded_within(0);
        assert!(matches!(&result, Err(CosError::Filter(detail)) if detail.contains("exceeds the limit of 0 bytes")), "{result:?}");
        assert_eq!(stream.decoded_within(1).unwrap(), b"A");
        assert_eq!(stream.decoded_strict().unwrap(), b"A");
    }

    #[test]
    fn first_filter_decodes_only_the_outer_wrapper() {
        let data = "0 0 m 100 100 l S\n".repeat(64).into_bytes();
        let inner = Stream::flate(Dict::new(), &data).raw.to_vec();
        for (outer, name, corrupt) in [(Filter::Ascii85, "ASCII85Decode", &b"{~>"[..]), (Filter::AsciiHex, "ASCIIHexDecode", &b"G0>"[..])] {
            let encoded = pdfcraft_filters::encode(&outer, &Params::default(), &inner).unwrap();
            let mut dict = Dict::new();
            dict.set(b"Filter".to_vec(), Object::Array(vec![Object::name(name), Object::name("FlateDecode")]));
            let stream = Stream::from_raw(dict.clone(), encoded);
            assert_eq!(stream.decoded_first_filter_strict(usize::MAX).unwrap(), inner);
            assert_eq!(stream.decoded_strict().unwrap(), data);
            assert!(matches!(stream.decoded_first_filter_strict(inner.len() - 1), Err(CosError::Filter(_))));
            assert!(matches!(Stream::from_raw(dict, corrupt.to_vec()).decoded_first_filter_strict(usize::MAX), Err(CosError::Filter(_))));
        }
    }
}

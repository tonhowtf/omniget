//! Channel image data codecs: Raw, RLE (PackBits), ZIP, and ZIP with prediction.
//!
//! All decoded data is *planar* bytes in the file's native big-endian sample
//! order: `planes × height × row_bytes` where `row_bytes` depends on depth
//! (depth 1 packs 8 pixels per byte, MSB first).

use std::io::{Read, Write};

use crate::error::{PsdError, Result};
use crate::header::{Version, row_bytes};
use crate::io::Reader;

/// Maximum number of decoded bytes a single decode call may produce: 8 GiB on
/// 64-bit targets, 2 GiB elsewhere (the same budget as `photocraft-codecs`'
/// default `Limits::max_alloc`).
///
/// The decoders bound their output by their input on their own (RLE expands
/// at most 64x, Raw needs every byte present), so this is a backstop against
/// decompression bombs, not the main guard. It has to fit real PSBs: a
/// 30000² RGB merged image is 2.7 GB, and a 33000² 16-bit layer channel is
/// 2.2 GB, both of which a 2 GiB cap turned into empty layers (#375).
pub const MAX_DECODED_BYTES: u64 = if cfg!(target_pointer_width = "64") { 8 << 30 } else { 2 << 30 };

/// Compression method stored before channel / image data.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum Compression {
    /// 0: uncompressed.
    Raw,
    /// 1: PackBits RLE with a per-row byte-count table.
    Rle,
    /// 2: zlib without prediction.
    Zip,
    /// 3: zlib with horizontal delta prediction.
    ZipPrediction,
    /// Unknown method; data is preserved but cannot be decoded.
    Unknown(u16),
}

impl Compression {
    /// All four decodable methods.
    pub const ALL: [Compression; 4] = [Compression::Raw, Compression::Rle, Compression::Zip, Compression::ZipPrediction];

    /// From the stored value.
    pub fn from_u16(v: u16) -> Self {
        match v {
            0 => Compression::Raw,
            1 => Compression::Rle,
            2 => Compression::Zip,
            3 => Compression::ZipPrediction,
            v => Compression::Unknown(v),
        }
    }
    /// Stored value.
    pub fn as_u16(self) -> u16 {
        match self {
            Compression::Raw => 0,
            Compression::Rle => 1,
            Compression::Zip => 2,
            Compression::ZipPrediction => 3,
            Compression::Unknown(v) => v,
        }
    }
}

/// PackBits (Apple / TIFF) run-length coding as used by Photoshop RLE.
pub mod packbits {
    use crate::error::{PsdError, Result};

    /// Encodes one row, appending to `out`. Never emits the `-128` no-op.
    pub fn encode(src: &[u8], out: &mut Vec<u8>) {
        let n = src.len();
        let mut i = 0;
        while i < n {
            // Length of the run starting at i.
            let mut run = 1;
            while i + run < n && run < 128 && src[i + run] == src[i] {
                run += 1;
            }
            // A literal never stops before a 2-byte run, so any run of >= 2
            // seen here starts a packet boundary: a run packet is never larger.
            if run >= 2 {
                out.push((1i16 - run as i16) as i8 as u8);
                out.push(src[i]);
                i += run;
                continue;
            }
            // Literal: extend until a run of >= 3 begins or 128 bytes.
            let start = i;
            let mut len = 0;
            while i < n && len < 128 {
                if i + 2 < n && src[i] == src[i + 1] && src[i] == src[i + 2] {
                    break;
                }
                i += 1;
                len += 1;
            }
            out.push((len - 1) as u8);
            out.extend_from_slice(&src[start..start + len]);
        }
    }

    /// Returns the encoded form of `src`.
    pub fn encode_vec(src: &[u8]) -> Vec<u8> {
        let mut v = Vec::with_capacity(src.len() + src.len() / 128 + 1);
        encode(src, &mut v);
        v
    }

    /// Decodes `src` into exactly `expected` bytes appended to `out`.
    ///
    /// Errors if the input is exhausted early or a packet would overflow the
    /// row. Trailing input after the row is complete is ignored except for
    /// `-128` no-op bytes (which are always permitted). Errors, rather than
    /// overflowing, when `out.len() + expected` does not fit in `usize`.
    pub(crate) fn decode_into(src: &[u8], expected: usize, out: &mut Vec<u8>) -> Result<()> {
        let target = out.len().checked_add(expected).ok_or(PsdError::LimitExceeded("PackBits output length overflows usize"))?;
        let mut i = 0;
        while out.len() < target {
            let Some(&h) = src.get(i) else {
                return Err(PsdError::Decompress("PackBits row ended early".into()));
            };
            i += 1;
            let h = h as i8;
            if h >= 0 {
                let len = h as usize + 1;
                let Some(lit) = src.get(i..i + len) else {
                    return Err(PsdError::Decompress("PackBits literal truncated".into()));
                };
                if len > target - out.len() {
                    return Err(PsdError::Decompress("PackBits literal overflows row".into()));
                }
                out.extend_from_slice(lit);
                i += len;
            } else if h != -128 {
                let len = 1 - h as isize;
                let len = len as usize;
                let Some(&b) = src.get(i) else {
                    return Err(PsdError::Decompress("PackBits run truncated".into()));
                };
                i += 1;
                if len > target - out.len() {
                    return Err(PsdError::Decompress("PackBits run overflows row".into()));
                }
                out.extend(std::iter::repeat_n(b, len));
            }
        }
        Ok(())
    }

    /// Decodes into a new vector of exactly `expected` bytes.
    pub fn decode(src: &[u8], expected: usize) -> Result<Vec<u8>> {
        let mut v = Vec::with_capacity(expected.min(src.len().saturating_mul(64)));
        decode_into(src, expected, &mut v)?;
        Ok(v)
    }
}

/// Geometry of a set of planes to encode/decode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlaneLayout {
    /// Number of planes (1 for a layer channel, channel count for the merged image).
    pub planes: usize,
    /// Pixels per row.
    pub width: usize,
    /// Rows per plane.
    pub height: usize,
    /// Bits per sample (1, 8, 16, 32).
    pub depth: u16,
    /// PSD vs PSB (affects RLE count width).
    pub version: Version,
}

impl PlaneLayout {
    /// Bytes per row.
    pub fn row_bytes(&self) -> usize {
        row_bytes(self.width, self.depth)
    }
    /// Total rows over all planes.
    pub fn rows(&self) -> Result<usize> {
        self.planes.checked_mul(self.height).ok_or(PsdError::LimitExceeded("row count overflow"))
    }
    /// Total decoded bytes without the [`MAX_DECODED_BYTES`] cap.
    pub fn total_bytes(&self) -> Result<u64> {
        (self.planes as u64)
            .checked_mul(self.height as u64)
            .and_then(|v| v.checked_mul(self.row_bytes() as u64))
            .ok_or(PsdError::LimitExceeded("decoded size overflow"))
    }

    /// Total decoded bytes, checked against [`MAX_DECODED_BYTES`].
    pub fn decoded_len(&self) -> Result<usize> {
        let total = self.total_bytes()?;
        if total > MAX_DECODED_BYTES {
            return Err(PsdError::LimitExceeded("decoded channel data exceeds MAX_DECODED_BYTES"));
        }
        usize::try_from(total).map_err(|_| PsdError::LimitExceeded("decoded size exceeds address space"))
    }
    fn count_size(&self) -> usize {
        if self.version.is_psb() { 4 } else { 2 }
    }
}

/// Decodes planar data (excluding the 2-byte compression field).
pub fn decode_planes(compression: Compression, data: &[u8], layout: &PlaneLayout) -> Result<Vec<u8>> {
    let total = layout.decoded_len()?;
    match compression {
        Compression::Raw => {
            if data.len() < total {
                return Err(PsdError::UnexpectedEof { offset: data.len(), needed: total - data.len() });
            }
            let mut out = Vec::new();
            out.try_reserve_exact(total).map_err(|_| PsdError::OutOfMemory("not enough memory for the decoded channel data"))?;
            out.extend_from_slice(&data[..total]);
            Ok(out)
        }
        Compression::Rle => decode_rle(data, layout, total),
        Compression::Zip => inflate_checked(data, total as u64, true),
        Compression::ZipPrediction => {
            let mut v = inflate_checked(data, total as u64, true)?;
            unpredict(&mut v, layout)?;
            Ok(v)
        }
        Compression::Unknown(c) => Err(PsdError::Unsupported(format!("compression method {c}"))),
    }
}

/// Bytes a PSD RLE stream (two-byte row counts) of `rows` rows grows by when written as PSB
/// (four-byte row counts); an error when `data` is too short to hold the count table.
pub(crate) fn rle_widening(data: &[u8], rows: usize) -> Result<u64> {
    rle_widening_len(data.len() as u64, rows)
}

/// [`rle_widening`] from the stream's length alone, so data still in its source (or staged on
/// the scratch disk) is not read to measure it.
pub(crate) fn rle_widening_len(len: u64, rows: usize) -> Result<u64> {
    let table = rows.checked_mul(2).ok_or(PsdError::LimitExceeded("row count overflow"))? as u64;
    if len < table {
        let offset = usize::try_from(len).unwrap_or(usize::MAX);
        return Err(PsdError::UnexpectedEof { offset, needed: usize::try_from(table - len).unwrap_or(usize::MAX) });
    }
    Ok(table)
}

/// Encodes planar data (excluding the 2-byte compression field).
pub fn encode_planes(compression: Compression, decoded: &[u8], layout: &PlaneLayout) -> Result<Vec<u8>> {
    let total = layout.decoded_len()?;
    if decoded.len() != total {
        return Err(PsdError::invalid(format!("decoded buffer has {} bytes, layout requires {}", decoded.len(), total)));
    }
    match compression {
        Compression::Raw => Ok(decoded.to_vec()),
        Compression::Rle => encode_rle(decoded, layout),
        Compression::Zip => Ok(zip_compress(decoded)),
        Compression::ZipPrediction => {
            let mut v = decoded.to_vec();
            predict(&mut v, layout)?;
            Ok(zip_compress(&v))
        }
        Compression::Unknown(c) => Err(PsdError::Unsupported(format!("compression method {c}"))),
    }
}

/// Validates encoded sizes and, for ZIP, the complete stream and checksum.
///
/// A ZIP stream whose declared size is over [`MAX_DECODED_BYTES`] is not
/// decompressed at all (#2926): [`decode_planes`] refuses that layout with
/// `LimitExceeded` anyway, so inflating it here would only be work on an
/// untrusted file before the same error. The bytes are kept verbatim, like an
/// unknown compression method, so a layered file whose merged composite is
/// too large still opens.
pub(crate) fn validate_planes(compression: Compression, data: &[u8], layout: &PlaneLayout) -> Result<()> {
    let total = layout.total_bytes()?;
    match compression {
        Compression::Raw => {
            if (data.len() as u64) < total {
                return Err(PsdError::UnexpectedEof { offset: data.len(), needed: usize::try_from(total - data.len() as u64).unwrap_or(usize::MAX) });
            }
            Ok(())
        }
        Compression::Rle => {
            let rows = layout.rows()?;
            let mut r = Reader::new(data);
            r.check_count(rows as u64, layout.count_size() as u64)?;
            let mut sum: u64 = 0;
            for _ in 0..rows {
                sum += read_count(&mut r, layout.version)? as u64;
            }
            if sum > r.remaining() as u64 {
                return Err(PsdError::UnexpectedEof { offset: data.len(), needed: usize::try_from(sum - r.remaining() as u64).unwrap_or(usize::MAX) });
            }
            Ok(())
        }
        Compression::Zip | Compression::ZipPrediction if total > MAX_DECODED_BYTES => Ok(()),
        Compression::Zip | Compression::ZipPrediction => inflate_checked(data, total, false).map(|_| ()),
        Compression::Unknown(_) => Ok(()),
    }
}

#[cfg(test)]
thread_local! {
    /// ZIP streams inflated on this thread (tests count passes over the merged image).
    pub(crate) static INFLATES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

// Keep only the requested pixels, but consume the trailer to verify Adler-32.
// A bounded surplus tolerates padding without unbounded decompression work.
fn inflate_checked(data: &[u8], expected: u64, keep: bool) -> Result<Vec<u8>> {
    const OOM: PsdError = PsdError::OutOfMemory("not enough memory for the decoded channel data");
    #[cfg(test)]
    INFLATES.with(|n| n.set(n.get() + 1));
    let mut decoder = flate2::Decompress::new(true);
    let mut out = Vec::new();
    if keep {
        out.try_reserve_exact(expected.min(data.len().saturating_mul(1032) as u64) as usize).map_err(|_| OOM)?;
    }
    let mut buf = vec![0; expected.saturating_add(1).clamp(8192, 1 << 20) as usize];
    loop {
        let before = (decoder.total_in(), decoder.total_out());
        let input = data.get(before.0 as usize..).ok_or_else(|| PsdError::Decompress("zlib input overflow".into()))?;
        let status = decoder.decompress(input, &mut buf, flate2::FlushDecompress::None).map_err(|e| PsdError::Decompress(e.to_string()))?;
        let got = decoder.total_out();
        if got > expected.saturating_add(1 << 20) {
            return Err(PsdError::LimitExceeded("zlib output exceeds the declared size and padding allowance"));
        }
        if keep {
            let n = (got - before.1).min(expected.saturating_sub(out.len() as u64)) as usize;
            out.try_reserve(n).map_err(|_| OOM)?;
            out.extend_from_slice(buf.get(..n).ok_or_else(|| PsdError::Decompress("zlib output overflow".into()))?);
        }
        if status == flate2::Status::StreamEnd {
            if got < expected {
                return Err(PsdError::Decompress(format!("zlib stream produced {got} of {expected} bytes")));
            }
            return Ok(out);
        }
        if before == (decoder.total_in(), got) {
            return Err(PsdError::Decompress("incomplete zlib stream".into()));
        }
    }
}

fn read_count(r: &mut Reader<'_>, v: Version) -> Result<usize> {
    Ok(if v.is_psb() { r.u32()? as usize } else { r.u16()? as usize })
}

fn decode_rle(data: &[u8], layout: &PlaneLayout, total: usize) -> Result<Vec<u8>> {
    let rows = layout.rows()?;
    let rb = layout.row_bytes();
    let mut r = Reader::new(data);
    r.check_count(rows as u64, layout.count_size() as u64)?;
    let mut counts = Vec::with_capacity(rows);
    for _ in 0..rows {
        counts.push(read_count(&mut r, layout.version)?);
    }
    // PackBits expands at most 64x (2 input bytes -> 128 output bytes).
    if (total as u64) > (r.remaining() as u64).saturating_mul(64) {
        return Err(PsdError::Decompress("RLE data too short for declared size".into()));
    }
    // Up to MAX_DECODED_BYTES: report a machine without that much memory instead of aborting.
    let mut out = Vec::new();
    out.try_reserve_exact(total).map_err(|_| PsdError::OutOfMemory("not enough memory for the decoded channel data"))?;
    for c in counts {
        let row = r.bytes(c)?;
        packbits::decode_into(row, rb, &mut out)?;
    }
    Ok(out)
}

fn encode_rle(decoded: &[u8], layout: &PlaneLayout) -> Result<Vec<u8>> {
    let rows = layout.rows()?;
    let rb = layout.row_bytes();
    let cs = layout.count_size();
    if rows == 0 {
        return Ok(Vec::new());
    }
    // Rows are independent, so they pack in parallel chunks of rows. Two passes: the first only
    // measures each packed row, so the output is allocated once at its exact size and the second
    // packs every row straight into its place. Collecting packed chunks and then copying them into
    // the output held the encoded image twice, with growth slack, at the save's peak (#6607).
    let threads = if cfg!(target_arch = "wasm32") { 1 } else { std::thread::available_parallelism().map_or(1, |v| v.get()).clamp(1, 32) };
    let chunk_rows = rows.div_ceil(threads).max(1);
    let row = |r: usize| &decoded[r * rb..(r + 1) * rb];
    let mut lens = vec![0usize; rows];
    for_each_chunk(threads, lens.chunks_mut(chunk_rows).enumerate().collect(), |(c, lens): (usize, &mut [usize])| {
        let mut scratch = Vec::with_capacity(rb + rb.div_ceil(128));
        for (i, len) in lens.iter_mut().enumerate() {
            scratch.clear();
            packbits::encode(row(c * chunk_rows + i), &mut scratch);
            *len = scratch.len();
        }
    })?;
    let mut out = vec![0u8; rows * cs + lens.iter().sum::<usize>()];
    let (counts, mut packed) = out.split_at_mut(rows * cs);
    for (count, &n) in counts.chunks_exact_mut(cs).zip(&lens) {
        if cs == 2 {
            let n = u16::try_from(n).map_err(|_| PsdError::LimitExceeded("RLE row exceeds 65535 bytes (use PSB)"))?;
            count.copy_from_slice(&n.to_be_bytes());
        } else {
            let n = u32::try_from(n).map_err(|_| PsdError::LimitExceeded("RLE row too large"))?;
            count.copy_from_slice(&n.to_be_bytes());
        }
    }
    let mut parts = Vec::with_capacity(threads);
    for (c, lens) in lens.chunks(chunk_rows).enumerate() {
        let (part, rest) = packed.split_at_mut(lens.iter().sum());
        parts.push((c, lens, part));
        packed = rest;
    }
    for_each_chunk(threads, parts, |(c, lens, mut part): (usize, &[usize], &mut [u8])| {
        let mut scratch = Vec::with_capacity(rb + rb.div_ceil(128));
        for (i, &n) in lens.iter().enumerate() {
            scratch.clear();
            packbits::encode(row(c * chunk_rows + i), &mut scratch);
            let (dst, rest) = std::mem::take(&mut part).split_at_mut(n);
            dst.copy_from_slice(&scratch);
            part = rest;
        }
    })?;
    Ok(out)
}

/// Runs `f` on every item, each on its own thread unless `threads` is 1 (wasm32).
fn for_each_chunk<T: Send>(threads: usize, items: Vec<T>, f: impl Fn(T) + Sync) -> Result<()> {
    if threads == 1 {
        items.into_iter().for_each(f);
        return Ok(());
    }
    std::thread::scope(|scope| {
        let handles: Vec<_> = items.into_iter().map(|item| scope.spawn(|| f(item))).collect();
        // Join every worker instead of aborting the scope with a thread panic.
        let mut ok = true;
        for h in handles {
            ok &= h.join().is_ok();
        }
        if ok { Ok(()) } else { Err(PsdError::LimitExceeded("an RLE worker panicked")) }
    })
}

/// zlib-compresses `data` at the default level.
pub fn zip_compress(data: &[u8]) -> Vec<u8> {
    let mut e = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
    // Writing into a Vec cannot fail.
    let _ = e.write_all(data);
    e.finish().unwrap_or_default()
}

/// Decompresses a zlib stream, requiring at least `expected` output bytes and
/// returning exactly `expected`.
///
/// The output is reserved fallibly and read in chunks, never past `expected`
/// bytes, so a stream declaring a huge size reports an error on a machine
/// without that much memory instead of aborting.
pub fn zip_decompress(data: &[u8], expected: usize) -> Result<Vec<u8>> {
    const OOM: PsdError = PsdError::OutOfMemory("not enough memory for the decoded channel data");
    const CHUNK: usize = 1 << 20;
    let mut dec = flate2::read::ZlibDecoder::new(data).take(u64::try_from(expected).unwrap_or(u64::MAX));
    let mut out = Vec::new();
    // Deflate expands at most ~1032:1, so this is the most a valid stream can need.
    out.try_reserve_exact(expected.min(data.len().saturating_mul(1032))).map_err(|_| OOM)?;
    let mut buf = vec![0u8; CHUNK.min(expected).max(1)];
    loop {
        let n = match dec.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => n,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(PsdError::Decompress(e.to_string())),
        };
        let chunk = buf.get(..n).ok_or_else(|| PsdError::Decompress("zlib reader overran its buffer".into()))?;
        out.try_reserve(n).map_err(|_| OOM)?;
        out.extend_from_slice(chunk);
    }
    if out.len() < expected {
        return Err(PsdError::Decompress(format!("zlib stream produced {} of {} bytes", out.len(), expected)));
    }
    Ok(out)
}

/// Applies horizontal delta prediction in place (encoder side).
///
/// * 1/8-bit: byte-wise delta per row.
/// * 16-bit: big-endian u16 sample delta per row.
/// * 32-bit: each row's samples are split into four byte planes (MSB plane
///   first) and the resulting row is byte-wise delta coded, as Photoshop does.
pub fn predict(buf: &mut [u8], layout: &PlaneLayout) -> Result<()> {
    let rb = layout.row_bytes();
    if rb == 0 {
        return Ok(());
    }
    let w = layout.width;
    let mut scratch = Vec::new();
    for row in buf.chunks_exact_mut(rb) {
        match layout.depth {
            1 | 8 => {
                for i in (1..row.len()).rev() {
                    row[i] = row[i].wrapping_sub(row[i - 1]);
                }
            }
            16 => {
                for i in (1..w).rev() {
                    let a = u16::from_be_bytes([row[2 * i], row[2 * i + 1]]);
                    let b = u16::from_be_bytes([row[2 * i - 2], row[2 * i - 1]]);
                    row[2 * i..2 * i + 2].copy_from_slice(&a.wrapping_sub(b).to_be_bytes());
                }
            }
            32 => {
                scratch.clear();
                scratch.resize(rb, 0);
                for i in 0..w {
                    for k in 0..4 {
                        scratch[k * w + i] = row[4 * i + k];
                    }
                }
                for i in (1..rb).rev() {
                    scratch[i] = scratch[i].wrapping_sub(scratch[i - 1]);
                }
                row.copy_from_slice(&scratch);
            }
            d => return Err(PsdError::Unsupported(format!("prediction for depth {d}"))),
        }
    }
    Ok(())
}

/// Reverses [`predict`] in place (decoder side).
pub fn unpredict(buf: &mut [u8], layout: &PlaneLayout) -> Result<()> {
    let rb = layout.row_bytes();
    if rb == 0 {
        return Ok(());
    }
    let w = layout.width;
    let mut scratch = Vec::new();
    for row in buf.chunks_exact_mut(rb) {
        match layout.depth {
            1 | 8 => {
                for i in 1..row.len() {
                    row[i] = row[i].wrapping_add(row[i - 1]);
                }
            }
            16 => {
                for i in 1..w {
                    let a = u16::from_be_bytes([row[2 * i], row[2 * i + 1]]);
                    let b = u16::from_be_bytes([row[2 * i - 2], row[2 * i - 1]]);
                    row[2 * i..2 * i + 2].copy_from_slice(&a.wrapping_add(b).to_be_bytes());
                }
            }
            32 => {
                for i in 1..rb {
                    row[i] = row[i].wrapping_add(row[i - 1]);
                }
                scratch.clear();
                scratch.extend_from_slice(row);
                for i in 0..w {
                    for k in 0..4 {
                        row[4 * i + k] = scratch[k * w + i];
                    }
                }
            }
            d => return Err(PsdError::Unsupported(format!("prediction for depth {d}"))),
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod parallel_rle_equivalence {
    use super::*;

    /// The parallel row packer's output must be byte-equal to a straightforward sequential
    /// encoding of the same plane, across depths, row counts around the chunking and data
    /// shapes (runs, noise, all-default).
    #[test]
    fn parallel_rle_matches_sequential() {
        let mut data: u64 = 0x9e37_79b9_7f4a_7c15;
        let mut next = move || {
            data ^= data << 13;
            data ^= data >> 7;
            data ^= data << 17;
            data
        };
        let layout_for = |rows: usize, rb: usize, version: Version| PlaneLayout { planes: 1, width: rb, height: rows, depth: 8, version };
        for (rows, rb, version) in [
            (0usize, 4usize, Version::Psd),
            (1, 4, Version::Psd),
            (7, 4, Version::Psb),
            (64, 4, Version::Psd),
            (65, 4, Version::Psb),  // chunk boundary
            (513, 4, Version::Psd), // more rows than threads on most runners
            (2049, 4, Version::Psb),
            (2, 4, Version::Psd), // fewer rows than threads
            (33, 3000, Version::Psd),
            (31, 20000, Version::Psb),
        ] {
            let pattern: Vec<u8> = match rows % 3 {
                0 => vec![7u8; rows * rb],                                   // long runs
                1 => (0..rows * rb).map(|i| (i * 31 % 251) as u8).collect(), // noise-ish
                _ => vec![0u8; rows * rb],                                   // all default
            };
            let pattern = if pattern.len() != rows * rb { vec![7u8; rows * rb] } else { pattern };
            let mut data = pattern.clone();
            if rows % 4 == 0 {
                // sprinkle runs into the noise
                for chunk in data.chunks_mut(97) {
                    if let Some(f) = chunk.first_mut() {
                        *f = 200;
                    }
                }
            }
            let _ = next();
            let layout = layout_for(rows, rb, version);
            let parallel = encode_rle(&data, &layout).expect("parallel encode");
            let sequential = encode_rle_sequential(&data, &layout).expect("sequential encode");
            assert_eq!(parallel, sequential, "rows {rows} rb {rb}");
            // #6607: measured first, then packed in place: one allocation, exactly the file's size.
            assert_eq!(parallel.capacity(), parallel.len(), "rows {rows} rb {rb}: the output grew or was over-allocated");
        }
    }

    /// A row too long for a PSD's 16-bit RLE count is the same error from the two-pass encoder.
    #[test]
    fn rle_rows_past_sixteen_bits_are_refused_in_psd() {
        let layout_for = |rows: usize, rb: usize, version: Version| PlaneLayout { planes: 1, width: rb, height: rows, depth: 8, version };
        let rb = 70_000;
        let layout = layout_for(3, rb, Version::Psd);
        let noise: Vec<u8> = (0..3 * rb).map(|i| (i * 31 % 251) as u8).collect();
        assert_eq!(encode_rle(&noise, &layout), Err(PsdError::LimitExceeded("RLE row exceeds 65535 bytes (use PSB)")));
        assert_eq!(encode_rle(&noise, &layout_for(3, rb, Version::Psb)), encode_rle_sequential(&noise, &layout_for(3, rb, Version::Psb)));
    }

    /// The pre-parallel encoder, kept as the oracle.
    fn encode_rle_sequential(decoded: &[u8], layout: &PlaneLayout) -> Result<Vec<u8>> {
        let rows = layout.rows()?;
        let rb = layout.row_bytes();
        let cs = layout.count_size();
        let mut out = vec![0u8; rows * cs];
        let mut enc = Vec::new();
        for row in 0..rows {
            let start = enc.len();
            packbits::encode(&decoded[row * rb..(row + 1) * rb], &mut enc);
            let n = enc.len() - start;
            if cs == 2 {
                let n = u16::try_from(n).map_err(|_| PsdError::LimitExceeded("RLE row exceeds 65535 bytes (use PSB)"))?;
                out[row * 2..row * 2 + 2].copy_from_slice(&n.to_be_bytes());
            } else {
                let n = u32::try_from(n).map_err(|_| PsdError::LimitExceeded("RLE row too large"))?;
                out[row * 4..row * 4 + 4].copy_from_slice(&n.to_be_bytes());
            }
        }
        out.extend_from_slice(&enc);
        Ok(out)
    }
}

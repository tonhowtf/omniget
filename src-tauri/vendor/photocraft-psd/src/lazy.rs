//! Parsing from a [`Source`] without reading the whole file: [`PsdFile::from_source`].
//!
//! The structure (header, resources, layer records, global blocks) is read into memory as
//! [`PsdFile::from_bytes`] would hold it. The bulk of a layered file is left in the source: every
//! channel's encoded bytes and the merged image become [`Stored`] ranges, read when they are
//! decoded. Apart from those ranges, the model is the one `from_bytes` builds from the same bytes
//! (the parse tests hold the two to that), with one difference: a merged image the caller does
//! not ask for is not read, so it is not checked either.

use crate::compression::Compression;
use crate::error::{PsdError, Result};
use crate::file::{GlobalLayerMask, LayerInfoPlacement, PsdFile};
use crate::header::{Header, Version};
use crate::image_data::ImageData;
use crate::io::Reader;
use crate::layer::{LayerInfo, LayerRecord};
use crate::resources::ImageResource;
use crate::source::{Source, Stored};
use crate::tagged::{TaggedBlock, is_sig, uses_long_length};

/// Bytes read per attempt at the layer records; doubled until they all fit. (Small in tests, so
/// they grow it.)
const RECORDS_CHUNK: u64 = if cfg!(test) { 64 } else { 1 << 20 };

/// A bounds-checked cursor over `start..end` of a source.
struct Cursor<'a> {
    src: &'a Source,
    pos: u64,
    end: u64,
}

impl<'a> Cursor<'a> {
    fn new(src: &'a Source) -> Self {
        Cursor { src, pos: 0, end: src.get().len() }
    }

    fn remaining(&self) -> u64 {
        self.end.saturating_sub(self.pos)
    }

    fn is_empty(&self) -> bool {
        self.remaining() == 0
    }

    fn eof(&self, needed: u64) -> PsdError {
        PsdError::UnexpectedEof {
            offset: usize::try_from(self.pos).unwrap_or(usize::MAX),
            needed: usize::try_from(needed.saturating_sub(self.remaining())).unwrap_or(usize::MAX),
        }
    }

    fn need(&self, n: u64) -> Result<()> {
        if n > self.remaining() { Err(self.eof(n)) } else { Ok(()) }
    }

    /// Reads `n` bytes.
    fn bytes(&mut self, n: u64) -> Result<Vec<u8>> {
        self.need(n)?;
        let v = self.src.read_vec(self.pos, n)?;
        self.pos += n;
        Ok(v)
    }

    /// Up to `n` bytes from the current position, without consuming.
    fn peek(&self, n: u64) -> Result<Vec<u8>> {
        self.src.read_vec(self.pos, n.min(self.remaining()))
    }

    fn array<const N: usize>(&mut self) -> Result<[u8; N]> {
        self.need(N as u64)?;
        let mut b = [0u8; N];
        self.src.get().read_at(self.pos, &mut b)?;
        self.pos += N as u64;
        Ok(b)
    }

    fn u16(&mut self) -> Result<u16> {
        self.array().map(u16::from_be_bytes)
    }

    fn i16(&mut self) -> Result<i16> {
        self.array().map(i16::from_be_bytes)
    }

    fn u32(&mut self) -> Result<u32> {
        self.array().map(u32::from_be_bytes)
    }

    fn len_field(&mut self, long: bool) -> Result<u64> {
        if long { self.array().map(u64::from_be_bytes) } else { self.u32().map(u64::from) }
    }

    /// The next `n` bytes as a cursor of their own; this one moves past them.
    fn sub(&mut self, n: u64) -> Result<Cursor<'a>> {
        self.need(n)?;
        let sub = Cursor { src: self.src, pos: self.pos, end: self.pos + n };
        self.pos += n;
        Ok(sub)
    }

    fn skip(&mut self, n: u64) -> Result<()> {
        self.need(n)?;
        self.pos += n;
        Ok(())
    }

    fn rest(&mut self) -> Result<Vec<u8>> {
        self.bytes(self.remaining())
    }

    /// The rest as a stored range; the cursor moves to the end.
    fn stored_rest(&mut self) -> Stored {
        let s = Stored { source: self.src.clone(), offset: self.pos, len: self.remaining() };
        self.pos = self.end;
        s
    }
}

impl PsdFile {
    /// Parses a file from `source`, leaving channel data and the merged image in it: see
    /// [`crate::source`]. Decoding a channel then reads its bytes from the source, which must
    /// stay unchanged while the model is in use.
    pub fn from_source(source: Source) -> Result<Self> {
        Self::from_source_and_merged_recovering(source, |_| false, |_| false).map(|(file, _)| file)
    }

    /// [`Self::from_bytes_and_merged_recovering`] for a [`Source`]: the merged image is read only
    /// when `want` asks for it (and is then decoded as there, with the same recovery); otherwise
    /// it stays in the source, unread and unchecked.
    pub fn from_source_and_merged_recovering(
        source: Source,
        want: impl FnOnce(&PsdFile) -> bool,
        recover: impl FnOnce(&PsdFile) -> bool,
    ) -> Result<(Self, Option<Result<Vec<u8>>>)> {
        Self::from_source_and_merged_preflight(source, |file| Ok(want(file)), recover)
    }

    /// Parses with a fallible pixel preflight after layer records are available and before
    /// the merged image is read, validated or decoded. A preflight error is always fatal;
    /// `recover` only handles damage encountered after an accepted preflight.
    pub fn from_source_and_merged_preflight(
        source: Source,
        want: impl FnOnce(&PsdFile) -> Result<bool>,
        recover: impl FnOnce(&PsdFile) -> bool,
    ) -> Result<(Self, Option<Result<Vec<u8>>>)> {
        let mut c = Cursor::new(&source);
        let header = Header::read(&mut Reader::new(&c.bytes(26)?))?;
        let version = header.version;
        let psb = version.is_psb();
        let cmd_len = c.u32()?;
        let color_mode_data = c.bytes(u64::from(cmd_len))?;
        let res_len = c.u32()?;
        let res = c.bytes(u64::from(res_len))?;
        let mut rr = Reader::new(&res);
        let mut resources = Vec::new();
        while !rr.is_empty() {
            resources.push(ImageResource::read(&mut rr)?);
        }

        let lm_len = c.len_field(psb)?;
        let mut s = c.sub(lm_len)?;
        let mut layer_info = None;
        let mut global_layer_mask = None;
        let mut global_blocks = Vec::new();
        let mut layer_mask_trailing = Vec::new();
        let mut layer_info_placement = LayerInfoPlacement::Section;
        if !s.is_empty() {
            let li_len = s.len_field(psb)?;
            if li_len > 0 {
                layer_info = Some(read_layer_info(s.sub(li_len)?, version)?);
            }
            if s.remaining() >= 4 {
                let n = s.u32()?;
                global_layer_mask = Some(GlobalLayerMask { data: s.bytes(u64::from(n))? });
                let target_key = match header.depth {
                    16 => Some(b"Lr16"),
                    32 => Some(b"Lr32"),
                    _ => None,
                };
                let mut candidate_seen = match target_key {
                    Some(_) => false,
                    None => layer_info.as_ref().is_some_and(|i: &LayerInfo| !i.layers.is_empty()),
                };
                let trailing = read_blocks(
                    &mut s,
                    version,
                    |index, signature, key, data, padding| {
                        let is_candidate = match target_key {
                            Some(k) => &key == k,
                            None => matches!(&key, b"Layr" | b"Lr16" | b"Lr32"),
                        };
                        if candidate_seen || !is_candidate {
                            return Ok(None);
                        }
                        candidate_seen = true;
                        let info = match read_layer_info(data, version) {
                            Ok(info) => info,
                            // A failed read of the source is not damage: fail the parse, as
                            // reading the whole file would.
                            Err(e @ PsdError::Read(_)) => return Err(e),
                            Err(_) => {
                                if target_key.is_some() {
                                    layer_info = None;
                                }
                                return Ok(None);
                            }
                        };
                        layer_info = Some(info);
                        layer_info_placement = LayerInfoPlacement::GlobalBlock { index, signature, key, padding: padding.clone() };
                        Ok(Some(()))
                    },
                    &mut global_blocks,
                )?;
                layer_mask_trailing = trailing;
            } else {
                layer_mask_trailing = s.rest()?;
            }
        }
        let mut file = PsdFile {
            header,
            color_mode_data,
            resources,
            layer_info,
            layer_info_placement,
            global_layer_mask,
            global_blocks,
            layer_mask_trailing,
            image_data: ImageData::new(Compression::Raw, Vec::new()),
            write_as_psb: false,
        };
        if !want(&file)? {
            let compression = Compression::from_u16(c.u16()?);
            file.image_data = ImageData { compression, data: Vec::new(), stored: Some(c.stored_rest()) };
            return Ok((file, None));
        }
        // The merged image is wanted: read it and decode it as `from_bytes` does.
        let rest = c.rest()?;
        let merged = file.read_merged_recovering(&rest, recover)?;
        Ok((file, merged))
    }
}

/// Reads tagged blocks as [`crate::tagged::read_blocks_with`] does, from a cursor: `consume`
/// sees each block's data as a cursor and returns `Some` when it took the block; the others are
/// read into memory and pushed to `blocks`. Returns the trailing bytes.
fn read_blocks<'a>(
    r: &mut Cursor<'a>,
    version: Version,
    mut consume: impl FnMut(usize, [u8; 4], [u8; 4], Cursor<'a>, &Option<Vec<u8>>) -> Result<Option<()>>,
    blocks: &mut Vec<TaggedBlock>,
) -> Result<Vec<u8>> {
    const ALIGNMENT: usize = 4;
    loop {
        let head = r.peek(12)?;
        if head.len() < 12 || !is_sig(&head) {
            return r.rest();
        }
        let signature = r.array::<4>()?;
        let key = r.array::<4>()?;
        let len = r.len_field(uses_long_length(version, &key))?;
        let data = r.sub(len)?;
        // Padding as `read_blocks_with` detects it: the smallest k in 0..=3 such that the next
        // signature (or the end of the region) follows k bytes later.
        let rest = r.peek(7)?;
        let mut pad = None;
        for k in 0..=3usize {
            if k == rest.len() || rest.get(k..).is_some_and(is_sig) {
                pad = Some(k);
                break;
            }
        }
        let default = TaggedBlock::default_padding((len % ALIGNMENT as u64) as usize, ALIGNMENT);
        let pad = pad.unwrap_or_else(|| if rest.get(..default).is_some_and(|p| p.iter().all(|&b| b == 0)) { default } else { 0 });
        let pad_bytes = rest.get(..pad).map(<[u8]>::to_vec).unwrap_or_default();
        r.skip(pad as u64)?;
        let padding = if pad_bytes.len() == default && pad_bytes.iter().all(|&b| b == 0) { None } else { Some(pad_bytes) };
        let (start, end) = (data.pos, data.end);
        if consume(blocks.len(), signature, key, data, &padding)?.is_none() {
            let data = Cursor { src: r.src, pos: start, end }.rest()?;
            blocks.push(TaggedBlock { signature, key, data, padding });
        }
    }
}

/// [`LayerInfo::read_body`] from a cursor over the body: the records are read into memory, and
/// each channel's encoded bytes become a [`Stored`] range after its compression field.
fn read_layer_info(mut r: Cursor<'_>, version: Version) -> Result<LayerInfo> {
    let body_start = r.pos;
    let count = r.i16()?;
    let merged_alpha = count < 0;
    let n = count.unsigned_abs();
    // Minimum record size, as `read_body` checks it.
    if u64::from(n) * 34 > r.remaining() {
        return Err(r.eof(u64::from(n) * 34));
    }
    // The records' total size is only known once they are parsed: read a chunk and grow it until
    // they all fit (or the body is exhausted, when their error is the body's).
    let mut chunk = RECORDS_CHUNK.min(r.remaining());
    let (layers, all_lens, used) = loop {
        let buf = r.peek(chunk)?;
        let mut rr = Reader::new(&buf);
        let mut layers = Vec::with_capacity(usize::from(n));
        let mut all_lens = Vec::with_capacity(usize::from(n));
        let parsed = (0..n).try_for_each(|_| {
            let (rec, lens) = LayerRecord::read(&mut rr, version)?;
            layers.push(rec);
            all_lens.push(lens);
            Ok::<_, PsdError>(())
        });
        match parsed {
            Ok(()) => break (layers, all_lens, rr.pos() as u64),
            Err(PsdError::UnexpectedEof { .. }) if chunk < r.remaining() => chunk = chunk.saturating_mul(2).min(r.remaining()),
            Err(e) => return Err(e),
        }
    };
    r.skip(used)?;
    let mut layers = layers;
    for (rec, lens) in layers.iter_mut().zip(all_lens) {
        for (ch, len) in rec.channels.iter_mut().zip(lens) {
            if len == 0 {
                continue;
            }
            if len == 1 {
                return Err(PsdError::invalid("channel data length 1"));
            }
            ch.compression = Some(Compression::from_u16(r.u16()?));
            let data = r.sub(len - 2)?;
            ch.stored = Some(Stored { source: data.src.clone(), offset: data.pos, len: data.remaining() });
        }
    }
    let body_len = r.pos - body_start;
    let rest = r.rest()?;
    let padding = if rest.len() as u64 == body_len % 2 && rest.iter().all(|&b| b == 0) { None } else { Some(rest) };
    Ok(LayerInfo { merged_alpha, layers, padding })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testgen;

    /// Every generated file, plus the layered 16/32-bit ones with their layers in a global block.
    fn samples() -> Vec<Vec<u8>> {
        let mut v = Vec::new();
        for case in testgen::all_cases() {
            let mut file = case.file;
            v.push(file.to_bytes().unwrap());
            let key = match file.header.depth {
                16 => *b"Lr16",
                32 => *b"Lr32",
                _ => continue,
            };
            if file.layer_info.as_ref().is_some_and(|i| !i.layers.is_empty()) {
                file.global_layer_mask.get_or_insert_with(GlobalLayerMask::default);
                file.layer_info_placement = LayerInfoPlacement::GlobalBlock { index: 0, signature: *b"8BIM", key, padding: None };
                v.push(file.to_bytes().unwrap());
            }
        }
        v
    }

    fn same(bytes: Vec<u8>) {
        let eager = PsdFile::from_bytes(&bytes).unwrap();
        let lazy = PsdFile::from_source(Source::new(bytes.clone())).unwrap();
        assert_eq!(lazy, eager);
        assert_eq!(lazy.to_bytes().unwrap(), bytes);
        assert!(lazy.layers().iter().flat_map(|l| &l.channels).all(|c| c.data.is_empty()));
        assert!(lazy.image_data.data.is_empty() && lazy.image_data.stored.is_some());
    }

    #[test]
    fn lazy_parse_matches_the_eager_one() {
        for bytes in samples() {
            same(bytes);
        }
    }

    /// A PSD read lazily and written as PSB widens the RLE count tables of channels still in the
    /// source, byte for byte as an eager read does (main's PSB promotion, #5285, on #6583's reader).
    #[test]
    fn a_lazy_psd_promotes_to_the_same_psb_as_an_eager_one() {
        let bytes = testgen::layered(Version::Psd, crate::ColorMode::Rgb, 8, Compression::Rle).to_bytes().unwrap();
        let mut eager = PsdFile::from_bytes(&bytes).unwrap();
        let mut lazy = PsdFile::from_source(Source::new(bytes)).unwrap();
        eager.promote_to_psb().unwrap();
        lazy.promote_to_psb().unwrap();
        let written = eager.to_bytes().unwrap();
        assert_eq!(lazy.encoded_len().unwrap(), written.len() as u64);
        assert!(lazy.to_bytes().unwrap() == written, "the lazily read PSD writes another PSB");
    }

    #[test]
    fn lazy_parse_fails_where_the_eager_one_does() {
        for bytes in samples() {
            for cut in [1, 2, 7, 26, 40, bytes.len() / 3, bytes.len() / 2, bytes.len() - 3] {
                let Some(b) = bytes.get(..cut) else { continue };
                let eager = PsdFile::from_bytes(b);
                let lazy = PsdFile::from_source(Source::new(b.to_vec()));
                // A truncated merged image is only noticed when it is read.
                match eager {
                    Ok(eager) => assert_eq!(lazy.unwrap(), eager, "cut at {cut}"),
                    Err(e) => {
                        let records_ok = PsdFile::from_bytes_and_merged_recovering(b, |_| false, |_| true).is_ok();
                        assert!(lazy.is_err() || records_ok, "cut at {cut}: eager {e:?}");
                    }
                }
            }
        }
    }

    /// A source whose `fail_at`-th read fails once.
    struct FailOnce {
        bytes: Vec<u8>,
        fail_at: usize,
        reads: std::sync::atomic::AtomicUsize,
    }

    impl crate::source::ReadAt for FailOnce {
        fn len(&self) -> u64 {
            self.bytes.len() as u64
        }

        fn read_at(&self, offset: u64, buf: &mut [u8]) -> Result<()> {
            if self.reads.fetch_add(1, std::sync::atomic::Ordering::SeqCst) == self.fail_at {
                return Err(PsdError::Read("injected".into()));
            }
            self.bytes.read_at(offset, buf)
        }
    }

    #[test]
    fn a_failed_read_fails_the_parse_instead_of_dropping_the_layers() {
        for bytes in samples() {
            let eager = PsdFile::from_bytes(&bytes).unwrap();
            let clean = std::sync::Arc::new(FailOnce { bytes: bytes.clone(), fail_at: usize::MAX, reads: 0.into() });
            PsdFile::from_source(Source::from_arc(clean.clone())).unwrap();
            let reads = clean.reads.load(std::sync::atomic::Ordering::SeqCst);
            for fail_at in 0..reads {
                let src = FailOnce { bytes: bytes.clone(), fail_at, reads: 0.into() };
                match PsdFile::from_source(Source::new(src)) {
                    Err(PsdError::Read(_)) => {}
                    Ok(lazy) => assert_eq!(lazy, eager, "read {fail_at} failed silently"),
                    Err(e) => panic!("read {fail_at}: {e:?}"),
                }
            }
        }
    }
}

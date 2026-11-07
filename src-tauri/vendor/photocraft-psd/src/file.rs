//! The top-level [`PsdFile`] model, parser and writer.

use std::io::{Seek, Write};

use crate::compression::{Compression, MAX_DECODED_BYTES, decode_planes, rle_widening_len, validate_planes};
use crate::error::{PsdError, Result};
use crate::header::{ColorMode, Header, Version};
use crate::image_data::ImageData;
use crate::io::{Reader, Sink, WriteExt};
use crate::layer::{LayerInfo, LayerRecord};
use crate::resources::{self, ImageResource, ResourceData, ids};
use crate::tagged::{TaggedBlock, read_blocks_with, uses_long_length, write_blocks_to};

/// The name the reference editor gives the merged transparency among the alpha channel names.
const TRANSPARENCY_NAME: &str = "Transparency";

/// Where the layer info was stored in the file.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum LayerInfoPlacement {
    /// In the layer info field of the layer and mask section (8-bit files).
    #[default]
    Section,
    /// In a global `Lr16` / `Lr32` / `Layr` tagged block (16/32-bit files).
    /// The block is re-inserted at `index` among `global_blocks` on write.
    GlobalBlock {
        /// Position among the global blocks.
        index: usize,
        /// Block signature (`8BIM` / `8B64`).
        signature: [u8; 4],
        /// Block key.
        key: [u8; 4],
        /// Padding after the block (see [`TaggedBlock::padding`]).
        padding: Option<Vec<u8>>,
    },
}

/// Global layer mask info, stored raw with typed accessors.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct GlobalLayerMask {
    /// Raw bytes (without the length field).
    pub data: Vec<u8>,
}

impl GlobalLayerMask {
    /// The mask Photoshop writes for a layered file that has no global mask of its own:
    /// an RGB red overlay at 50% opacity, kind 128 (use the per-layer setting), plus the
    /// filler byte. A zero-length `LMsk` block in a TIFF makes Photoshop give up on the
    /// layer data (it reads this 14-byte record from the block).
    pub fn photoshop_default() -> Self {
        GlobalLayerMask { data: vec![0, 0, 0xff, 0xff, 0, 0, 0, 0, 0, 0, 0, 50, 128, 0] }
    }

    fn be16(&self, at: usize) -> Option<u16> {
        self.data.get(at..at + 2).map(|b| u16::from_be_bytes([b[0], b[1]]))
    }
    /// Overlay color space.
    pub fn overlay_color_space(&self) -> Option<u16> {
        self.be16(0)
    }
    /// Four color components.
    pub fn color_components(&self) -> Option<[u16; 4]> {
        Some([self.be16(2)?, self.be16(4)?, self.be16(6)?, self.be16(8)?])
    }
    /// Opacity 0..=100.
    pub fn opacity(&self) -> Option<u16> {
        self.be16(10)
    }
    /// Kind: 0 color selected, 1 color protected, 128 per layer.
    pub fn kind(&self) -> Option<u8> {
        self.data.get(12).copied()
    }
}

/// A complete PSD/PSB file.
///
/// `PsdFile::from_bytes(b)?.to_bytes()? == b` for unmodified well-formed
/// files, and `from_bytes(to_bytes(f)) == f` for any model `f` not marked
/// [`Self::write_as_psb`] (a marked one reads back as the PSB it was written as).
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct PsdFile {
    /// Header.
    pub header: Header,
    /// Color mode data (palette for indexed, duotone spec, …).
    pub color_mode_data: Vec<u8>,
    /// Image resources in stored order.
    pub resources: Vec<ImageResource>,
    /// Layer info (`None` when the layer info length is 0 and no
    /// `Lr16`/`Lr32` block holds layers).
    pub layer_info: Option<LayerInfo>,
    /// Where [`Self::layer_info`] is stored.
    pub layer_info_placement: LayerInfoPlacement,
    /// Global layer mask (`None` when the field is absent entirely).
    pub global_layer_mask: Option<GlobalLayerMask>,
    /// Global additional layer information blocks.
    pub global_blocks: Vec<TaggedBlock>,
    /// Bytes at the end of the layer and mask section that do not form a block.
    pub layer_mask_trailing: Vec<u8>,
    /// Merged image data.
    pub image_data: ImageData,
    /// Write this PSD model as PSB (set by [`Self::promote_to_psb`]). The model itself stays a
    /// consistent PSD: its channel data keeps two-byte RLE row counts, so everything that reads
    /// it with `header.version` decodes it as before. Only [`Self::to_bytes`] and
    /// [`Self::encoded_len`] write version 2, long length fields and four-byte row counts.
    #[cfg_attr(feature = "serde", serde(default))]
    pub write_as_psb: bool,
}

impl PsdFile {
    /// Parses a PSD or PSB file.
    pub fn from_bytes(data: &[u8]) -> Result<Self> {
        Self::from_bytes_and_merged(data, |_| false).map(|(file, _)| file)
    }

    /// Parses like [`Self::from_bytes`] and, when `want` asks for it, decodes the merged image
    /// (as [`Self::decode_merged`] would) in the same pass. `want` sees the file with everything
    /// but the merged image parsed (its `image_data` is still empty). A ZIP merged image is then
    /// inflated once for both the parse's check of the stream and the decode, not twice.
    ///
    /// Errors are those of [`Self::from_bytes`]; a merged image that passes the parse's check but
    /// cannot be decoded is the inner `Err`, as [`Self::decode_merged`] would return it.
    pub fn from_bytes_and_merged(data: &[u8], want: impl FnOnce(&PsdFile) -> bool) -> Result<(Self, Option<Result<Vec<u8>>>)> {
        Self::from_bytes_and_merged_recovering(data, want, |_| false)
    }

    /// Parses like [`Self::from_bytes_and_merged`], but lets `recover` accept damage confined to
    /// the merged image after all layer records and channel data have been read successfully.
    /// Callers should opt in only when they can reconstruct the document from its layers.
    ///
    /// The original encoded image bytes are preserved and its parse error is returned as
    /// `Some(Err(..))`, even when `want` did not request a decode. The caller must report that
    /// error rather than silently discarding the merged image (which may hold extra channels).
    /// Header/layer errors, missing compression fields and parse safety-limit errors stay fatal.
    pub fn from_bytes_and_merged_recovering(
        data: &[u8],
        want: impl FnOnce(&PsdFile) -> bool,
        recover: impl FnOnce(&PsdFile) -> bool,
    ) -> Result<(Self, Option<Result<Vec<u8>>>)> {
        Self::from_bytes_and_merged_preflight(data, |file| Ok(want(file)), recover)
    }

    /// Parses with a fallible pixel preflight after layer records are available and before
    /// the merged image is read, validated or decoded. A preflight error is always fatal;
    /// `recover` only handles damage encountered after an accepted preflight.
    pub fn from_bytes_and_merged_preflight(
        data: &[u8],
        want: impl FnOnce(&PsdFile) -> Result<bool>,
        recover: impl FnOnce(&PsdFile) -> bool,
    ) -> Result<(Self, Option<Result<Vec<u8>>>)> {
        let mut r = Reader::new(data);
        let header = Header::read(&mut r)?;
        let psb = header.version.is_psb();
        let cmd_len = r.u32()?;
        let color_mode_data = r.bytes_u64(u64::from(cmd_len))?.to_vec();
        let resources = resources::read_section(&mut r)?;

        let lm_len = r.len_field(psb)?;
        let mut s = r.sub(lm_len)?;
        let mut layer_info = None;
        let mut global_layer_mask = None;
        let mut global_blocks = Vec::new();
        let mut layer_mask_trailing = Vec::new();
        let mut layer_info_placement = LayerInfoPlacement::Section;
        if !s.is_empty() {
            let li_len = s.len_field(psb)?;
            if li_len > 0 {
                layer_info = Some(LayerInfo::read_body(s.bytes_u64(li_len)?, header.version)?);
            }
            if s.remaining() >= 4 {
                let n = s.u32()?;
                global_layer_mask = Some(GlobalLayerMask { data: s.bytes_u64(u64::from(n))?.to_vec() });
                let target_key = match header.depth {
                    16 => Some(b"Lr16"),
                    32 => Some(b"Lr32"),
                    _ => None,
                };
                let mut candidate_seen = match target_key {
                    Some(_) => false,
                    None => layer_info.as_ref().is_some_and(|i| !i.layers.is_empty()),
                };
                let (blocks, trailing) = read_blocks_with(&mut s, header.version, 4, |index, signature, key, data, padding| {
                    let is_candidate = match target_key {
                        Some(k) => &key == k,
                        None => matches!(&key, b"Layr" | b"Lr16" | b"Lr32"),
                    };
                    if candidate_seen || !is_candidate {
                        return false;
                    }
                    candidate_seen = true;
                    let Ok(info) = LayerInfo::read_body(data, header.version) else {
                        if target_key.is_some() {
                            layer_info = None;
                        }
                        return false;
                    };
                    layer_info = Some(info);
                    layer_info_placement = LayerInfoPlacement::GlobalBlock { index, signature, key, padding: padding.clone() };
                    true
                })?;
                global_blocks = blocks;
                layer_mask_trailing = trailing;
            } else {
                layer_mask_trailing = s.peek_rest().to_vec();
            }
        }
        let image_data = ImageData::new(Compression::Raw, Vec::new());
        let mut file = PsdFile {
            header,
            color_mode_data,
            resources,
            layer_info,
            layer_info_placement,
            global_layer_mask,
            global_blocks,
            layer_mask_trailing,
            image_data,
            write_as_psb: false,
        };
        let want = want(&file)?;
        let merged = file.read_merged_section(&mut r, want, recover)?;
        Ok((file, merged))
    }

    /// Reads the merged image section from `rest` (the bytes after the layer and mask section) as
    /// [`Self::from_bytes_and_merged_recovering`] does, decoding it.
    pub(crate) fn read_merged_recovering(&mut self, rest: &[u8], recover: impl FnOnce(&PsdFile) -> bool) -> Result<Option<Result<Vec<u8>>>> {
        self.read_merged_section(&mut Reader::new(rest), true, recover)
    }

    fn read_merged_section(&mut self, r: &mut Reader<'_>, want: bool, recover: impl FnOnce(&PsdFile) -> bool) -> Result<Option<Result<Vec<u8>>>> {
        let file = self;
        let mut encoded = r.clone();
        let merged = if want {
            file.read_merged_decoded(r).map(Some)
        } else {
            ImageData::read(r, &file.header).map(|image| {
                file.image_data = image;
                None
            })
        };
        let merged = match merged {
            Ok(merged) => merged,
            Err(error) => {
                if matches!(error, PsdError::LimitExceeded(_) | PsdError::OutOfMemory(_)) || !recover(file) {
                    return Err(error);
                }
                let compression = Compression::from_u16(encoded.u16()?);
                let source = encoded.peek_rest();
                let mut data = Vec::new();
                data.try_reserve_exact(source.len()).map_err(|_| PsdError::OutOfMemory("not enough memory to retain the damaged merged image"))?;
                data.extend_from_slice(source);
                file.image_data = ImageData::new(compression, data);
                Some(Err(error))
            }
        };
        Ok(merged)
    }

    /// Reads the merged image into `self.image_data` and decodes it. A ZIP stream within the
    /// decode limit is inflated once: by the decode, whose stream check is the one
    /// [`ImageData::read`] makes. Only when the decode fails is that check run on its own, to
    /// tell a file the parse rejects (the outer `Err`) from one it accepts (the inner `Err`).
    fn read_merged_decoded(&mut self, r: &mut Reader<'_>) -> Result<Result<Vec<u8>>> {
        let layout = ImageData::layout(&self.header);
        let compression = Compression::from_u16(r.clone().u16()?);
        let zip = matches!(compression, Compression::Zip | Compression::ZipPrediction);
        if !zip || layout.total_bytes()? > MAX_DECODED_BYTES {
            // Nothing is inflated twice here: the parse's check reads sizes only (or skips an
            // over-limit stream, which the decode then refuses).
            self.image_data = ImageData::read(r, &self.header)?;
            return Ok(self.decode_merged());
        }
        r.u16()?;
        let data = r.peek_rest();
        let decoded = decode_planes(compression, data, &layout);
        // An allocation failure is the open's, before the encoded image is copied under the same
        // memory pressure.
        if matches!(decoded, Err(PsdError::OutOfMemory(_))) {
            return decoded.map(Ok);
        }
        if decoded.is_err() {
            validate_planes(compression, data, &layout)?;
        }
        let mut copy = Vec::new();
        copy.try_reserve_exact(data.len()).map_err(|_| PsdError::OutOfMemory("not enough memory to retain the merged image"))?;
        copy.extend_from_slice(data);
        self.image_data = ImageData::new(compression, copy);
        r.skip(data.len())?;
        Ok(decoded)
    }

    /// Serializes the file.
    pub fn to_bytes(&self) -> Result<Vec<u8>> {
        // Reserve the whole file up front, at its exact length: appending to an empty Vec
        // reallocates and copies repeatedly, and a reserve that is a few bytes short makes the
        // last append double the buffer, copying the whole file once more (#6607).
        let len = usize::try_from(self.encoded_len()?).map_err(|_| PsdError::OutOfMemory("not enough memory to serialize the file"))?;
        let mut out = Vec::new();
        out.try_reserve_exact(len).map_err(|_| PsdError::OutOfMemory("not enough memory to serialize the file"))?;
        self.write_to(&mut std::io::Cursor::new(&mut out)).map_err(PsdError::from_io)?;
        Ok(out)
    }

    /// Serializes the file into `w` as it goes, starting at its current position: the bytes of
    /// [`Self::to_bytes`] without ever holding them all (#6607). Length fields are back-patched
    /// by seeking. A serializer error (a section too large for PSD, too many layers) comes back
    /// as `InvalidData` carrying the [`PsdError`]; `w` then holds a partial file.
    pub fn write_to<W: Write + Seek + ?Sized>(&self, w: &mut W) -> std::io::Result<()> {
        let mut out = Sink::new(w)?;
        self.write_staging(&mut out)?;
        out.finish()
    }

    /// [`Self::write_to`] writing out every `stage` staged bytes (tests: tiny stages make every
    /// length field a back-patch through the stream).
    #[cfg(test)]
    pub(crate) fn write_to_staging<W: Write + Seek + ?Sized>(&self, w: &mut W, stage: usize) -> std::io::Result<()> {
        let mut out = Sink::staging(w, stage)?;
        self.write_staging(&mut out)?;
        out.finish()
    }

    fn write_staging<W: Write + Seek + ?Sized>(&self, out: &mut Sink<'_, W>) -> std::io::Result<()> {
        let (v, widen) = self.write_form();
        let psb = v.is_psb();
        out.small(|o| {
            if widen {
                Header { version: v, ..self.header.clone() }.write(o);
            } else {
                self.header.write(o);
            }
            o.put_len(self.color_mode_data.len() as u64, false)
        })?;
        out.put(&self.color_mode_data)?;
        out.small(|o| resources::write_section(o, &self.resources))?;

        let in_block = matches!(self.layer_info_placement, LayerInfoPlacement::GlobalBlock { .. });
        // An entirely empty section is written as a zero length. (A section
        // containing only a zero layer-info length cannot be distinguished in
        // the model and is normalized to this form.)
        let section_empty =
            self.layer_info.is_none() && self.global_layer_mask.is_none() && self.global_blocks.is_empty() && self.layer_mask_trailing.is_empty();
        let at = out.begin_len(psb)?;
        if !section_empty {
            let li = out.begin_len(psb)?;
            if let (Some(info), false) = (&self.layer_info, in_block) {
                info.write_body_to(out, v, widen)?;
            }
            out.end_len(li, psb)?;
            let need_mask_field = self.global_layer_mask.is_some() || !self.global_blocks.is_empty() || (in_block && self.layer_info.is_some());
            if need_mask_field {
                let gm = self.global_layer_mask.as_ref().map_or(&[][..], |g| &g.data[..]);
                out.small(|o| o.put_len(gm.len() as u64, false))?;
                out.put(gm)?;
            }
            match (&self.layer_info, &self.layer_info_placement) {
                (Some(info), LayerInfoPlacement::GlobalBlock { index, signature, key, padding }) => {
                    let (before, after) = self.global_blocks.split_at((*index).min(self.global_blocks.len()));
                    write_blocks_to(out, before, v, 4)?;
                    // Written in place: a temporary LayerInfo payload and a clone of all
                    // neighboring opaque blocks otherwise double the save's memory.
                    out.put(signature)?;
                    out.put(key)?;
                    let long = uses_long_length(v, key);
                    let at = out.begin_len(long)?;
                    let start = out.pos();
                    info.write_body_to(out, v, widen)?;
                    let len = out.pos() - start;
                    out.end_len(at, long)?;
                    match padding {
                        Some(bytes) => out.put(bytes)?,
                        None => out.zeros(((4 - len % 4) % 4) as usize)?,
                    }
                    write_blocks_to(out, after, v, 4)?;
                }
                _ => write_blocks_to(out, &self.global_blocks, v, 4)?,
            }
            out.put(&self.layer_mask_trailing)?;
        }
        out.end_len(at, psb)?;

        out.small(|o| {
            o.put_u16(self.image_data.compression.as_u16());
            Ok(())
        })?;
        match self.merged_widened_rows(widen)? {
            0 => out.put_encoded(&self.image_data.data, self.image_data.stored.as_ref()),
            rows => out.put_widened_encoded(&self.image_data.data, self.image_data.stored.as_ref(), rows),
        }
    }

    /// The version the file is written as, and whether its RLE count tables widen on the way
    /// (a PSD model marked [`Self::write_as_psb`]).
    fn write_form(&self) -> (Version, bool) {
        let widen = self.write_as_psb && !self.header.version.is_psb();
        (if widen { Version::Psb } else { self.header.version }, widen)
    }

    /// The version [`Self::to_bytes`] writes: PSB for a PSD model marked [`Self::write_as_psb`],
    /// else `header.version`.
    pub fn written_version(&self) -> Version {
        self.write_form().0
    }

    /// Rows of the merged image's RLE count table that widen on write (0 when nothing widens).
    fn merged_widened_rows(&self, widen: bool) -> Result<usize> {
        if widen && self.image_data.compression == Compression::Rle { ImageData::layout(&self.header).rows() } else { Ok(0) }
    }

    /// Bytes every RLE count table of this PSD model grows by when written as PSB; an error
    /// when a table is too short for its rows.
    fn rle_widening(&self) -> Result<u64> {
        let mut n = 0u64;
        for layer in self.layers() {
            for c in &layer.channels {
                match layer.widened_rows(c, true)? {
                    0 => {}
                    rows => n = n.saturating_add(rle_widening_len(c.encoded_len(), rows)?),
                }
            }
        }
        match self.merged_widened_rows(true)? {
            0 => Ok(n),
            rows => Ok(n.saturating_add(rle_widening_len(self.image_data.encoded_len(), rows)?)),
        }
    }

    /// The number of bytes [`Self::to_bytes`] writes, computed without serializing the channel
    /// data (only the layer records, which are small, are serialized to measure them). Lets a
    /// writer check a file against the 2 GB PSD limit before choosing the version.
    pub fn encoded_len(&self) -> Result<u64> {
        let (v, widen) = self.write_form();
        let psb = v.is_psb();
        let len_field = |long: bool| if long { 8u64 } else { 4 };
        let blocks_len = |blocks: &[TaggedBlock]| blocks.iter().fold(0u64, |n, b| n.saturating_add(b.encoded_len_aligned(v, 4) as u64));
        let body_len = |info: &LayerInfo| -> Result<u64> {
            let unpadded = info.unpadded_len_as(v, widen)?;
            let padding = info.padding.as_ref().map_or(unpadded % 2, |p| p.len() as u64);
            Ok(unpadded.saturating_add(padding))
        };
        // Header, colour mode data, resources.
        let mut n = 26u64.saturating_add(4).saturating_add(self.color_mode_data.len() as u64).saturating_add(resources::section_len(&self.resources));

        let in_block = matches!(self.layer_info_placement, LayerInfoPlacement::GlobalBlock { .. });
        let section_empty =
            self.layer_info.is_none() && self.global_layer_mask.is_none() && self.global_blocks.is_empty() && self.layer_mask_trailing.is_empty();
        n = n.saturating_add(len_field(psb));
        if !section_empty {
            n = n.saturating_add(len_field(psb));
            if let (Some(info), false) = (&self.layer_info, in_block) {
                n = n.saturating_add(body_len(info)?);
            }
            let need_mask_field = self.global_layer_mask.is_some() || !self.global_blocks.is_empty() || (in_block && self.layer_info.is_some());
            if need_mask_field {
                n = n.saturating_add(4).saturating_add(self.global_layer_mask.as_ref().map_or(0, |g| g.data.len() as u64));
            }
            n = n.saturating_add(blocks_len(&self.global_blocks));
            if let (Some(info), LayerInfoPlacement::GlobalBlock { key, padding, .. }) = (&self.layer_info, &self.layer_info_placement) {
                let body = body_len(info)?;
                let pad = padding.as_ref().map_or((4 - body % 4) % 4, |p| p.len() as u64);
                n = n.saturating_add(8).saturating_add(len_field(uses_long_length(v, key))).saturating_add(body).saturating_add(pad);
            }
            n = n.saturating_add(self.layer_mask_trailing.len() as u64);
        }
        let merged = match self.merged_widened_rows(widen)? {
            0 => 0,
            rows => rle_widening_len(self.image_data.encoded_len(), rows)?,
        };
        Ok(n.saturating_add(2).saturating_add(self.image_data.encoded_len()).saturating_add(merged))
    }

    /// Marks a PSD model to be written as the equivalent PSB ([`Self::write_as_psb`]) without
    /// touching or re-encoding any pixels: the packed RLE rows are the same in both versions, so
    /// [`Self::to_bytes`] only widens the row-count tables of the layer channels and the merged
    /// image (two to four bytes per row) and the length fields while it copies them out. Raw and
    /// ZIP channel data is the same in both. A PSB model is left as is.
    ///
    /// Every RLE count table is checked first: an error leaves the model unmarked, and the write
    /// cannot fail on a table later. The layer info padding is kept; a writer that pads to an
    /// alignment re-pads with [`Self::pad_layer_info_to`].
    pub fn promote_to_psb(&mut self) -> Result<()> {
        if self.header.version.is_psb() || self.write_as_psb {
            return Ok(());
        }
        self.rle_widening()?;
        self.write_as_psb = true;
        Ok(())
    }

    /// Pads the layer info body to a multiple of `align` bytes as it will be written (see
    /// [`LayerInfo::pad_to`]), RLE count tables widened when [`Self::write_as_psb`] is set.
    pub fn pad_layer_info_to(&mut self, align: u64) -> Result<()> {
        let (v, widen) = self.write_form();
        if let Some(info) = &mut self.layer_info {
            info.padding = Some(vec![0; info.pad_len_as(v, widen, align)?]);
        }
        Ok(())
    }

    /// Layer records in file order (bottom-most first); empty if none.
    pub fn layers(&self) -> &[LayerRecord] {
        self.layer_info.as_ref().map_or(&[], |i| &i.layers)
    }

    /// Mutable layer records (creates an empty layer info if absent).
    pub fn layers_mut(&mut self) -> &mut Vec<LayerRecord> {
        &mut self.layer_info.get_or_insert_with(LayerInfo::default).layers
    }

    /// First resource with the given id.
    pub fn resource(&self, id: u16) -> Option<&ImageResource> {
        self.resources.iter().find(|r| r.id == id)
    }

    /// The global `Lr16` / `Lr32` / `Layr` block that should have held the layers but could not
    /// be parsed, with the parse error. [`Self::from_bytes`] keeps such a block verbatim in
    /// [`Self::global_blocks`] and leaves [`Self::layer_info`] `None`, so the file reads as
    /// flattened; this tells a reader that its layers were lost rather than absent.
    pub fn unreadable_layer_block(&self) -> Option<(&TaggedBlock, PsdError)> {
        if self.layer_info.is_some() {
            return None;
        }
        let block = self.global_blocks.iter().find(|b| matches!(&b.key, b"Lr16" | b"Lr32" | b"Layr"))?;
        LayerInfo::read_body(&block.data, self.header.version).err().map(|e| (block, e))
    }

    /// First global block with the given key.
    pub fn global_block(&self, key: &[u8; 4]) -> Option<&TaggedBlock> {
        self.global_blocks.iter().find(|b| &b.key == key)
    }

    /// ICC profile bytes (resource 1039).
    pub fn icc_profile(&self) -> Option<&[u8]> {
        self.resource(ids::ICC_PROFILE).map(|r| &r.data[..])
    }

    /// Resolution info (resource 1005).
    pub fn resolution(&self) -> Option<crate::resources::ResolutionInfo> {
        match self.resource(ids::RESOLUTION_INFO)?.parsed()? {
            Ok(ResourceData::ResolutionInfo(r)) => Some(r),
            _ => None,
        }
    }

    /// The "has real merged data" flag of resource 1057, if present.
    pub fn has_real_merged_data(&self) -> Option<bool> {
        match self.resource(ids::VERSION_INFO)?.parsed()? {
            Ok(ResourceData::VersionInfo(v)) => Some(v.has_real_merged_data),
            _ => None,
        }
    }

    /// Decodes the merged image into planar big-endian samples.
    pub fn decode_merged(&self) -> Result<Vec<u8>> {
        self.image_data.decode(&self.header)
    }

    /// `true` if the merged image carries a transparency channel: its first extra channel, when
    /// the layer count was stored negative or a "Saving Merged Transparency" global block (see
    /// [`TaggedBlock::is_merged_transparency_marker`]) is present. Photoshop marks 32-bit documents
    /// with `Mt32` and a positive layer count (psd-tools transparentbg.psd). Photoshop also names
    /// the merged transparency "Transparency" and lists it first among the alpha channel names,
    /// and some of its 16-bit PSBs carry no other mark (#4654): when the layers sit in a global
    /// `Lr16`/`Lr32` block, that name with one name per extra channel marks it too.
    ///
    /// A file without layer records has one when it is marked, or unless its alpha channel names
    /// account for every extra channel without "Transparency" among them: a flat file from the
    /// reference editor lists only its alpha and spot channels there (psd-tools
    /// cmyk-alpha-spot.psd), while writers of plain RGBA files name nothing.
    pub fn merged_has_alpha(&self) -> bool {
        let Some(color) = self.header.color_mode.color_channels() else {
            return false;
        };
        if self.header.channels <= color {
            return false;
        }
        let marked = self.global_blocks.iter().any(TaggedBlock::is_merged_transparency_marker);
        match &self.layer_info {
            Some(li) => {
                let in_block = matches!(self.layer_info_placement, LayerInfoPlacement::GlobalBlock { .. });
                li.merged_alpha || marked || (in_block && self.names_merged_transparency(self.header.channels - color))
            }
            None => {
                let names = self.alpha_channel_names();
                marked || names.len() != usize::from(self.header.channels - color) || names.first().is_some_and(|n| n == TRANSPARENCY_NAME)
            }
        }
    }

    /// `true` when the alpha channel names hold exactly `extra` names and the first is
    /// "Transparency", the reference editor's name for the merged transparency.
    fn names_merged_transparency(&self, extra: u16) -> bool {
        let names = self.alpha_channel_names();
        names.len() == usize::from(extra) && names.first().is_some_and(|n| n == TRANSPARENCY_NAME)
    }

    /// The alpha channel names: the Unicode names (resource 1045), else the Pascal names (1006)
    /// older writers and psd-tools store alone, decoded as MacRoman. Empty when the file names no channel.
    pub fn alpha_channel_names(&self) -> Vec<String> {
        let mut names = Vec::new();
        if let Some(r) = self.resource(1045) {
            let mut at = 0usize;
            while let Some(len) = r.data.get(at..at.saturating_add(4)).and_then(|b| b.try_into().ok()).map(u32::from_be_bytes) {
                at = at.saturating_add(4);
                let end = (len as usize).checked_mul(2).and_then(|n| at.checked_add(n));
                let Some(units) = end.and_then(|end| r.data.get(at..end)) else { break };
                let units: Vec<u16> = units.as_chunks::<2>().0.iter().map(|c| u16::from_be_bytes(*c)).collect();
                names.push(String::from_utf16_lossy(&units).trim_end_matches('\0').to_string());
                at = at.saturating_add(units.len() * 2);
            }
        } else if let Some(r) = self.resource(1006) {
            let mut at = 0usize;
            while let Some(&len) = r.data.get(at) {
                let Some(name) = r.data.get(at + 1..at + 1 + usize::from(len)) else { break };
                names.push(encoding_rs::MACINTOSH.decode_without_bom_handling(name).0.into_owned());
                at += 1 + usize::from(len);
            }
        }
        names
    }

    /// Checks structural invariants that the writer relies on.
    pub fn validate(&self) -> Result<()> {
        self.header.validate()?;
        if self.header.version == Version::Psd && (self.header.width > 30_000 || self.header.height > 30_000) {
            return Err(PsdError::LimitExceeded("PSD dimensions exceed 30000; use PSB"));
        }
        if self.header.color_mode == ColorMode::Indexed && self.color_mode_data.len() != 768 {
            return Err(PsdError::invalid("indexed color mode requires a 768-byte palette"));
        }
        Ok(())
    }

    /// Reads a file from disk (native targets only).
    #[cfg(not(target_arch = "wasm32"))]
    pub fn open(path: impl AsRef<std::path::Path>) -> std::io::Result<Result<Self>> {
        Ok(Self::from_bytes(&std::fs::read(path)?))
    }

    /// Writes the file to disk (native targets only).
    #[cfg(not(target_arch = "wasm32"))]
    pub fn save(&self, path: impl AsRef<std::path::Path>) -> std::io::Result<()> {
        let b = self.to_bytes().map_err(std::io::Error::other)?;
        std::fs::write(path, b)
    }
}

#[cfg(test)]
mod write_tests {
    use super::*;
    use crate::{Compression, testgen};
    use std::io::{Seek, Write};

    /// #6607: the output is allocated once, at its exact length. A reserve a few bytes short of
    /// the file made the last append double the buffer, copying the whole file once more.
    #[test]
    fn serializing_allocates_the_output_once() {
        for case in testgen::all_cases() {
            let bytes = case.file.to_bytes().unwrap();
            assert_eq!(bytes.capacity(), bytes.len(), "{}: the buffer grew past its first allocation", case.name);
        }
    }

    /// A stream that records the length of every write it gets.
    struct Recorder {
        inner: std::io::Cursor<Vec<u8>>,
        writes: Vec<usize>,
    }

    impl Write for Recorder {
        fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
            self.writes.push(b.len());
            self.inner.write(b)
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    impl Seek for Recorder {
        fn seek(&mut self, to: std::io::SeekFrom) -> std::io::Result<u64> {
            self.inner.seek(to)
        }
    }

    /// The bytes `file` streams with `stage`-byte staging, after a 3-byte prefix that must stay.
    fn streamed(file: &PsdFile, stage: usize) -> Vec<u8> {
        let mut w = std::io::Cursor::new(b"pre".to_vec());
        w.set_position(3);
        file.write_to_staging(&mut w, stage).unwrap();
        let end = w.position();
        let mut bytes = w.into_inner();
        assert_eq!(&bytes[..3], b"pre");
        assert_eq!(end, bytes.len() as u64, "the stream must end at the file's end");
        bytes.drain(..3);
        bytes
    }

    /// #6607: streaming into a file writes exactly the bytes of `to_bytes`, wherever the stream
    /// starts and however little is staged, so every length field is back-patched through the
    /// stream (both placements of the layer info, PSD and PSB, every mode and depth).
    #[test]
    fn streaming_writes_the_same_bytes_with_lengths_patched_by_seeking() {
        for case in testgen::all_cases() {
            let want = case.file.to_bytes().unwrap();
            for stage in [0, 1, 7, 4096] {
                assert!(streamed(&case.file, stage) == want, "{} stage {stage}: streamed bytes differ", case.name);
            }
            let mut w = std::io::Cursor::new(Vec::new());
            case.file.write_to(&mut w).unwrap();
            assert!(w.into_inner() == want, "{}: write_to differs from to_bytes", case.name);
        }
    }

    /// A PSD model written as PSB (its RLE count tables widened on the way out) and a model read
    /// lazily from a source (its pixel data not in memory) stream the bytes `to_bytes` writes.
    #[test]
    fn promoted_and_source_backed_files_stream_the_same_bytes() {
        for case in testgen::all_cases() {
            let mut promoted = case.file.clone();
            if promoted.header.version == Version::Psd {
                promoted.promote_to_psb().unwrap();
                let want = promoted.to_bytes().unwrap();
                for stage in [0, 1, 7, 4096] {
                    assert!(streamed(&promoted, stage) == want, "{} promoted, stage {stage}", case.name);
                }
            }
            let bytes = case.file.to_bytes().unwrap();
            let lazy = PsdFile::from_source(crate::Source::new(bytes.clone())).unwrap();
            for stage in [0, 1, 4096] {
                assert!(streamed(&lazy, stage) == bytes, "{} source-backed, stage {stage}", case.name);
            }
        }
    }

    /// Channel data and a merged image still in their source (an imported file's, or what an
    /// exporter spilled to scratch) are copied through in short reads, never read whole.
    #[test]
    fn source_backed_data_is_copied_in_short_reads() {
        struct Reads(Vec<u8>, std::sync::Mutex<usize>);
        impl crate::source::ReadAt for Reads {
            fn len(&self) -> u64 {
                self.0.len() as u64
            }
            fn read_at(&self, offset: u64, buf: &mut [u8]) -> Result<()> {
                let mut most = self.1.lock().unwrap();
                *most = (*most).max(buf.len());
                self.0.as_slice().read_at(offset, buf)
            }
        }
        let mut file = testgen::small(Version::Psd, Compression::Raw);
        file.image_data = ImageData::new(Compression::Raw, vec![5; 300]);
        let info = file.layer_info.as_mut().unwrap();
        info.layers[0].channels[0] = crate::layer::ChannelData::new(0, Some(Compression::Raw), vec![6; 301]);
        let bytes = file.to_bytes().unwrap();
        let source = std::sync::Arc::new(Reads(bytes.clone(), std::sync::Mutex::new(0)));
        let lazy = PsdFile::from_source(crate::Source::from_arc(source.clone())).unwrap();
        *source.1.lock().unwrap() = 0;
        let mut w = std::io::Cursor::new(Vec::new());
        lazy.write_to(&mut w).unwrap();
        assert!(w.into_inner() == bytes);
        assert!(*source.1.lock().unwrap() <= 7, "read {} bytes at once", source.1.lock().unwrap());
    }

    /// #5333: RLE data still in its source widens to PSB in short reads too, the count table and
    /// then the packed rows, one consecutive pass, never the whole merged image or channel at once.
    #[test]
    fn source_backed_rle_widens_in_short_reads() {
        struct Reads(Vec<u8>, std::sync::Mutex<(usize, Vec<(u64, usize)>)>);
        impl crate::source::ReadAt for Reads {
            fn len(&self) -> u64 {
                self.0.len() as u64
            }
            fn read_at(&self, offset: u64, buf: &mut [u8]) -> Result<()> {
                let mut seen = self.1.lock().unwrap();
                seen.0 = seen.0.max(buf.len());
                seen.1.push((offset, buf.len()));
                self.0.as_slice().read_at(offset, buf)
            }
        }
        let file = testgen::small(Version::Psd, Compression::Rle);
        assert_eq!(file.image_data.compression, Compression::Rle);
        let bytes = file.to_bytes().unwrap();
        let mut promoted = file.clone();
        promoted.promote_to_psb().unwrap();
        let want = promoted.to_bytes().unwrap();
        let source = std::sync::Arc::new(Reads(bytes, std::sync::Mutex::new((0, Vec::new()))));
        let mut lazy = PsdFile::from_source(crate::Source::from_arc(source.clone())).unwrap();
        assert!(lazy.image_data.stored.is_some(), "the merged image stays in the source");
        lazy.promote_to_psb().unwrap();
        *source.1.lock().unwrap() = (0, Vec::new());
        let mut w = std::io::Cursor::new(Vec::new());
        lazy.write_to(&mut w).unwrap();
        assert!(w.into_inner() == want);
        let seen = source.1.lock().unwrap();
        assert!(seen.0 <= 7, "read {} bytes at once", seen.0);
        let merged = lazy.image_data.stored.as_ref().unwrap();
        let reads: Vec<_> = seen.1.iter().filter(|(at, _)| (merged.offset..merged.offset + merged.len).contains(at)).collect();
        let mut next = merged.offset;
        for &&(at, n) in &reads {
            assert_eq!(at, next, "the merged image is read in one consecutive pass");
            next += n as u64;
        }
        assert_eq!(next, merged.offset + merged.len);
    }

    /// #6607: pixel data goes to the stream as it is, never through a staging copy, so a save
    /// holds the model and a short buffer, not a second copy of the file.
    #[test]
    fn streaming_passes_channel_and_merged_data_straight_through() {
        let mut file = testgen::small(Version::Psd, Compression::Raw);
        let big = 3 * crate::io::STAGE_FOR_TESTS;
        file.image_data = ImageData::new(Compression::Raw, vec![5; big]);
        let info = file.layer_info.as_mut().unwrap();
        info.layers[0].channels[0] = crate::layer::ChannelData::new(0, Some(Compression::Raw), vec![6; big + 1]);
        let mut w = Recorder { inner: std::io::Cursor::new(Vec::new()), writes: Vec::new() };
        file.write_to(&mut w).unwrap();
        assert!(w.writes.contains(&big), "merged image data was staged: {:?}", w.writes);
        assert!(w.writes.contains(&(big + 1)), "channel data was staged: {:?}", w.writes);
        assert!(w.writes.iter().filter(|&&n| n != big && n != big + 1).all(|&n| n < crate::io::STAGE_FOR_TESTS * 2), "{:?}", w.writes);
        let bytes = w.inner.into_inner();
        assert_eq!(PsdFile::from_bytes(&bytes).unwrap(), file);
        assert!(bytes == file.to_bytes().unwrap());
    }

    /// A serializer error in the middle of a stream comes back as the same `PsdError` `to_bytes`
    /// reports, inside an `InvalidData` I/O error.
    #[test]
    fn streaming_reports_serializer_errors() {
        let mut file = testgen::small(Version::Psd, Compression::Raw);
        let info = file.layer_info.as_mut().unwrap();
        let layer = info.layers[0].clone();
        info.layers = vec![layer; 32768];
        let want = file.to_bytes().unwrap_err();
        let e = file.write_to(&mut std::io::Cursor::new(Vec::new())).unwrap_err();
        assert_eq!(e.kind(), std::io::ErrorKind::InvalidData);
        assert_eq!(PsdError::from_io(e), want);
        // A damaged count table of a promoted file fails the stream as it fails `to_bytes`.
        let mut file = testgen::small(Version::Psd, Compression::Rle);
        file.promote_to_psb().unwrap();
        file.image_data.data.truncate(3);
        let want = file.to_bytes().unwrap_err();
        let e = file.write_to(&mut std::io::Cursor::new(Vec::new())).unwrap_err();
        assert_eq!(PsdError::from_io(e), want);
    }

    /// `encoded_len` is exactly what `to_bytes` writes, for every generated file and both layer
    /// info placements, with and without padding (#5285).
    #[test]
    fn encoded_len_matches_the_written_length() {
        for case in testgen::all_cases() {
            assert_eq!(case.file.encoded_len().unwrap(), case.file.to_bytes().unwrap().len() as u64, "{}", case.name);
        }
        for version in [Version::Psd, Version::Psb] {
            for key in [*b"Lr16", *b"Layr"] {
                for padding in [None, Some(vec![]), Some(vec![1, 2, 3])] {
                    for body_padding in [None, Some(vec![]), Some(vec![9])] {
                        let mut file = testgen::small(version, Compression::Rle);
                        file.global_blocks = vec![TaggedBlock::new(*b"zz01", vec![11]), TaggedBlock::new(*b"lnk2", vec![1; 5])];
                        file.layer_info_placement = LayerInfoPlacement::GlobalBlock { index: 1, signature: *b"8BIM", key, padding: padding.clone() };
                        file.layer_info.as_mut().unwrap().padding = body_padding;
                        file.layer_mask_trailing = vec![0; 3];
                        file.resources.push(ImageResource { name: b"odd".to_vec(), ..ImageResource::new(4000, vec![1, 2, 3]) });
                        assert_eq!(file.encoded_len().unwrap(), file.to_bytes().unwrap().len() as u64, "{version:?} {key:?}");
                    }
                }
            }
        }
        let mut empty = testgen::small(Version::Psd, Compression::Raw);
        empty.layer_info = None;
        empty.global_layer_mask = None;
        empty.global_blocks.clear();
        assert_eq!(empty.encoded_len().unwrap(), empty.to_bytes().unwrap().len() as u64);
    }

    /// A promoted PSD writes exactly the PSB written directly from the same pixels, and reads
    /// back as that PSB; the model itself stays a PSD that every reader decodes as before (#5285).
    #[test]
    fn promote_to_psb_writes_the_psb_written_directly() {
        let cases = testgen::all_cases();
        let (psd, psb) = cases.split_at(cases.len() / 2);
        for (psd, psb) in psd.iter().zip(psb) {
            assert_eq!(psd.file.header.version, Version::Psd, "{}", psd.name);
            let mut promoted = psd.file.clone();
            promoted.promote_to_psb().unwrap();
            assert!(promoted.write_as_psb);
            assert_eq!(promoted.written_version(), Version::Psb);
            let bytes = promoted.to_bytes().unwrap();
            assert!(bytes == psb.file.to_bytes().unwrap(), "{}", psd.name);
            assert_eq!(promoted.encoded_len().unwrap(), bytes.len() as u64, "{}", psd.name);
            assert_eq!(PsdFile::from_bytes(&bytes).unwrap(), psb.file, "{}", psd.name);
            // Unchanged as a model: same header, same (two-byte) channel data.
            assert_eq!(PsdFile { write_as_psb: false, ..promoted.clone() }, psd.file, "{}", psd.name);
            assert_eq!(promoted.decode_merged().ok(), psd.file.decode_merged().ok(), "{}", psd.name);
        }
    }

    /// Re-padding a promoted model pads what is written, as `pad_to` does for a PSB.
    #[test]
    fn pad_layer_info_to_pads_the_written_body() {
        let mut promoted = testgen::layered(Version::Psd, ColorMode::Rgb, 8, Compression::Rle);
        promoted.promote_to_psb().unwrap();
        promoted.pad_layer_info_to(4).unwrap();
        let mut direct = testgen::layered(Version::Psb, ColorMode::Rgb, 8, Compression::Rle);
        direct.layer_info.as_mut().unwrap().pad_to(Version::Psb, 4).unwrap();
        assert!(promoted.to_bytes().unwrap() == direct.to_bytes().unwrap());
    }

    /// A damaged RLE table is refused before anything changes: the file stays a valid PSD.
    #[test]
    fn promote_to_psb_is_atomic() {
        let mut file = testgen::small(Version::Psd, Compression::Rle);
        // The last layer with pixels ("b", two rows): its red channel loses its row-count table.
        let layers = &mut file.layer_info.as_mut().unwrap().layers;
        let red = layers[2].channels.iter_mut().find(|c| c.id == 0).unwrap();
        assert_eq!(red.compression, Some(Compression::Rle));
        red.data.truncate(1);
        let before = file.clone();
        assert!(file.promote_to_psb().is_err());
        assert_eq!(file, before);
        assert!(!file.write_as_psb);
        // A table damaged after promotion fails the write instead of panicking.
        let mut file = testgen::small(Version::Psd, Compression::Rle);
        file.promote_to_psb().unwrap();
        file.image_data.data.truncate(3);
        assert!(file.to_bytes().is_err());
        assert!(file.encoded_len().is_err());
    }

    #[test]
    fn global_blocks_default_to_four_byte_padding() {
        for version in [Version::Psd, Version::Psb] {
            for len in 1usize..8 {
                let mut file = testgen::small(version, Compression::Raw);
                file.global_blocks = vec![TaggedBlock::new(*b"TEST", vec![7; len]), TaggedBlock::new(*b"NEXT", vec![8; 4])];
                let bytes = file.to_bytes().unwrap();
                let start = bytes.windows(8).position(|b| b == b"8BIMTEST").unwrap() + 12;
                let next = bytes.windows(8).position(|b| b == b"8BIMNEXT").unwrap();
                assert_eq!(next - start, len.next_multiple_of(4));
                let mut expected = vec![7; len];
                expected.resize(len.next_multiple_of(4), 0);
                assert_eq!(&bytes[start..start + expected.len()], expected);
                assert_eq!(PsdFile::from_bytes(&bytes).unwrap(), file);
            }
        }
    }

    #[test]
    fn streamed_layer_info_equals_a_materialized_tagged_block() {
        for version in [Version::Psd, Version::Psb] {
            for key in [*b"Lr16", *b"Lr32", *b"Layr"] {
                for index in [0, 1, 2, usize::MAX] {
                    for padding in [None, Some(vec![]), Some(vec![42]), Some(vec![1, 2, 3])] {
                        let mut file = testgen::small(version, Compression::Raw);
                        file.global_blocks = vec![TaggedBlock::new(*b"zz01", vec![11]), TaggedBlock::new(*b"zz02", vec![12, 13])];
                        file.layer_info_placement = LayerInfoPlacement::GlobalBlock { index, signature: *b"8B64", key, padding: padding.clone() };
                        // An odd body catches padding calculated against the final file offset.
                        file.layer_info.as_mut().unwrap().padding = Some(vec![77]);
                        let actual = file.to_bytes().unwrap();
                        let mut materialized = file.clone();
                        let mut data = Vec::new();
                        materialized.layer_info.take().unwrap().write_body(&mut data, version).unwrap();
                        materialized.global_blocks.insert(index.min(materialized.global_blocks.len()), TaggedBlock { signature: *b"8B64", key, data, padding });
                        materialized.layer_info_placement = LayerInfoPlacement::Section;
                        assert_eq!(actual, materialized.to_bytes().unwrap(), "{version:?} {key:?} {index}");
                    }
                }
            }
        }
    }

    /// #2926: a PSB whose ZIP merged image declares more than `MAX_DECODED_BYTES` opens without
    /// inflating that stream (its layers are still usable); decoding the composite reports the limit.
    #[test]
    fn over_budget_zip_merged_image_opens_without_inflating() {
        let mut bytes = testgen::small(Version::Psb, Compression::Zip).to_bytes().unwrap();
        // Header: height at 14, width at 18 (big-endian u32).
        bytes[14..18].copy_from_slice(&300_000u32.to_be_bytes());
        bytes[18..22].copy_from_slice(&300_000u32.to_be_bytes());
        let file = PsdFile::from_bytes(&bytes).unwrap();
        assert!(!file.layers().is_empty());
        assert!(matches!(file.decode_merged(), Err(PsdError::LimitExceeded(_))));
        let (same, merged) = PsdFile::from_bytes_and_merged(&bytes, |_| true).unwrap();
        assert_eq!(same, file);
        assert!(matches!(merged, Some(Err(PsdError::LimitExceeded(_)))));
    }

    /// Parsing and decoding a ZIP merged image together inflates it once (the parse's stream
    /// check is the decode's), with the same model, pixels and errors as the two steps apart.
    #[test]
    fn merged_zip_image_is_inflated_once_when_decoded_with_the_parse() {
        use crate::compression::INFLATES;
        let inflates = |f: &dyn Fn()| {
            let before = INFLATES.with(std::cell::Cell::get);
            f();
            INFLATES.with(std::cell::Cell::get) - before
        };
        for compression in [Compression::Zip, Compression::ZipPrediction] {
            let bytes = testgen::small(Version::Psd, compression).to_bytes().unwrap();
            let file = PsdFile::from_bytes(&bytes).unwrap();
            let pixels = file.decode_merged().unwrap();
            let apart = inflates(&|| drop(PsdFile::from_bytes(&bytes).unwrap().decode_merged().unwrap()));
            let together = inflates(&|| {
                let (parsed, merged) = PsdFile::from_bytes_and_merged(&bytes, |f| f.image_data.data.is_empty()).unwrap();
                assert_eq!(parsed, file);
                assert_eq!(merged.unwrap().unwrap(), pixels);
            });
            assert_eq!(together + 1, apart, "{compression:?}");
            let (parsed, merged) = PsdFile::from_bytes_and_merged(&bytes, |_| false).unwrap();
            assert_eq!((parsed, merged), (file, None));
            // A broken stream (its checksum) is still the parse's error.
            let mut broken = bytes.clone();
            *broken.last_mut().unwrap() ^= 0xff;
            let alone = PsdFile::from_bytes(&broken).unwrap_err().to_string();
            assert_eq!(PsdFile::from_bytes_and_merged(&broken, |_| true).unwrap_err().to_string(), alone);
        }
    }

    #[test]
    fn global_layer_info_serialization_propagates_layer_limit_errors() {
        let mut file = testgen::small(Version::Psd, Compression::Raw);
        file.layer_info = Some(LayerInfo { layers: vec![LayerRecord::default(); 32768], ..Default::default() });
        file.layer_info_placement = LayerInfoPlacement::GlobalBlock { index: usize::MAX, signature: *b"8BIM", key: *b"Lr16", padding: None };
        assert!(matches!(file.to_bytes(), Err(PsdError::LimitExceeded(_))));
    }

    #[test]
    fn global_layers_preserve_order_padding_and_first_candidate() {
        for version in [Version::Psd, Version::Psb] {
            for depth in [16, 32] {
                let mut file = testgen::layered(version, ColorMode::Rgb, depth, Compression::Raw);
                let key = if depth == 16 { *b"Lr16" } else { *b"Lr32" };
                file.global_blocks = vec![TaggedBlock::new(*b"BEFR", vec![1]), TaggedBlock::new(*b"AFTR", vec![2, 3])];
                file.layer_info_placement = LayerInfoPlacement::GlobalBlock { index: 1, signature: *b"8B64", key, padding: Some(vec![7, 8]) };
                file.layer_mask_trailing = vec![9; 4];
                let bytes = file.to_bytes().unwrap();
                let parsed = PsdFile::from_bytes(&bytes).unwrap();
                assert_eq!(parsed, file);
                assert_eq!(parsed.to_bytes().unwrap(), bytes);
                let mut duplicate = Vec::new();
                file.layer_info.as_ref().unwrap().write_body(&mut duplicate, version).unwrap();
                file.global_blocks.push(TaggedBlock::new(key, duplicate));
                assert_eq!(PsdFile::from_bytes(&file.to_bytes().unwrap()).unwrap(), file);
                file.global_blocks.insert(0, TaggedBlock::new(key, vec![255]));
                let bytes = file.to_bytes().unwrap();
                let parsed = PsdFile::from_bytes(&bytes).unwrap();
                assert!(parsed.layer_info.is_none());
                assert_eq!(parsed.global_blocks.len(), 5);
                assert_eq!(parsed.to_bytes().unwrap(), bytes);
            }
        }
    }

    #[test]
    fn global_layers_prioritized_over_compatibility_section() {
        for version in [Version::Psd, Version::Psb] {
            for depth in [16, 32] {
                let file = testgen::layered(version, ColorMode::Rgb, depth, Compression::Raw);
                let key = if depth == 16 { *b"Lr16" } else { *b"Lr32" };
                let mut lr_data = Vec::new();
                file.layer_info.as_ref().unwrap().write_body(&mut lr_data, version).unwrap();

                // Photoshop writes an empty (or 8-bit compatibility) Layer Info section
                // alongside the global Lr16/Lr32 block (#4928).
                let mut compat_file = file.clone();
                compat_file.layer_info =
                    Some(LayerInfo { layers: vec![LayerRecord { name: b"CompatibilityLayer".to_vec(), ..Default::default() }], ..Default::default() });
                compat_file.layer_info_placement = LayerInfoPlacement::Section;
                compat_file.global_blocks.insert(0, TaggedBlock::new(key, lr_data));

                let bytes = compat_file.to_bytes().unwrap();
                let parsed = PsdFile::from_bytes(&bytes).unwrap();

                assert_eq!(parsed.layers().len(), file.layers().len());
                assert_ne!(parsed.layers()[0].name(), "CompatibilityLayer");
                assert_eq!(parsed.layers()[0].name(), file.layers()[0].name());
                assert!(matches!(parsed.layer_info_placement, LayerInfoPlacement::GlobalBlock { key: k, .. } if k == key));
            }
        }
    }

    /// Without layer records, the extra channels hold a merged transparency unless the alpha
    /// channel names (1045, else 1006) name each of them and none first as "Transparency".
    #[test]
    fn a_flat_file_names_its_extra_channels() {
        let mut file = testgen::small(Version::Psd, Compression::Raw);
        file.layer_info = None;
        file.header.channels = 5;
        file.resources.retain(|r| r.id != 1045 && r.id != 1006);
        assert!(file.merged_has_alpha(), "nothing named: a plain RGBA file");
        let pascal = |names: &[&str]| names.iter().flat_map(|n| std::iter::once(n.len() as u8).chain(n.bytes())).collect::<Vec<u8>>();
        let unicode = |names: &[&str]| {
            names
                .iter()
                .flat_map(|n| (n.encode_utf16().count() as u32).to_be_bytes().into_iter().chain(n.encode_utf16().flat_map(u16::to_be_bytes)))
                .collect::<Vec<u8>>()
        };
        for (names, transparency) in
            [(&["Spot", "Alpha 1"][..], false), (&["Transparency", "Alpha 1"][..], true), (&["Alpha 1"][..], true), (&["A", "B", "C"][..], true)]
        {
            let mut p = file.clone();
            p.resources.push(ImageResource::new(1006, pascal(names)));
            assert_eq!(p.merged_has_alpha(), transparency, "1006 {names:?}");
            assert_eq!(p.alpha_channel_names(), names, "1006");
            let mut u = file.clone();
            u.resources.push(ImageResource::new(1045, unicode(names)));
            assert_eq!(u.merged_has_alpha(), transparency, "1045 {names:?}");
            assert_eq!(u.alpha_channel_names(), names, "1045");
        }
        // A merged-transparency marker wins over a complete list of names.
        let mut marked = file.clone();
        marked.resources.push(ImageResource::new(1006, pascal(&["Spot", "Alpha 1"])));
        marked.global_blocks.push(TaggedBlock::new(*b"Mt32", Vec::new()));
        assert!(marked.merged_has_alpha());
        // Truncated names stop at the damage.
        let mut cut = file.clone();
        cut.resources.push(ImageResource::new(1006, vec![9, b'a']));
        assert!(cut.alpha_channel_names().is_empty());
        assert!(cut.merged_has_alpha());
    }

    #[test]
    fn pascal_channel_names_use_mac_roman_and_unicode_takes_precedence() {
        let mut file = testgen::small(Version::Psd, Compression::Raw);
        file.resources.retain(|r| !matches!(r.id, 1006 | 1045));
        file.resources.push(ImageResource::new(1006, b"\x04caf\x8e\x03x".to_vec()));
        assert_eq!(file.alpha_channel_names(), ["café"]);
        file.resources.push(ImageResource::new(1045, vec![0, 0, 0, 1, 0, b'Z']));
        assert_eq!(file.alpha_channel_names(), ["Z"]);
    }

    /// A positive layer count plus a merged-transparency marker still makes the first extra
    /// channel the transparency, for the specification's keys and for the `MTrn` Photoshop
    /// writes in a layered TIFF.
    #[test]
    fn merged_transparency_markers_mark_the_first_extra_channel() {
        let mut file = testgen::small(Version::Psd, Compression::Raw);
        if let Some(li) = &mut file.layer_info {
            li.merged_alpha = false;
        }
        assert!(!file.merged_has_alpha());
        for key in [*b"Mtrn", *b"MTrn", *b"Mt16", *b"Mt32"] {
            let mut marked = file.clone();
            marked.global_blocks.push(TaggedBlock::new(key, Vec::new()));
            assert!(marked.merged_has_alpha(), "{}", String::from_utf8_lossy(&key));
        }
        // Other keys, and the marker's letters in another case, do not.
        for key in [*b"MTRN", *b"mtrn", *b"Lr16"] {
            let mut other = file.clone();
            other.global_blocks.push(TaggedBlock::new(key, Vec::new()));
            assert!(!other.merged_has_alpha(), "{}", String::from_utf8_lossy(&key));
        }
    }
    #[test]
    fn preflight_refusal_precedes_merged_validation_and_cannot_be_recovered() {
        let file = testgen::small(Version::Psd, Compression::Zip);
        let mut bytes = file.to_bytes().unwrap();
        bytes.truncate(bytes.len() - file.image_data.data.len() - 2);
        let recover_called = std::cell::Cell::new(false);
        let preflight = |_: &PsdFile| Err(PsdError::LimitExceeded("test import budget"));
        let recover = |_: &PsdFile| {
            recover_called.set(true);
            true
        };
        assert!(matches!(PsdFile::from_bytes_and_merged_preflight(&bytes, preflight, recover), Err(PsdError::LimitExceeded("test import budget"))));
        assert!(matches!(
            PsdFile::from_source_and_merged_preflight(crate::Source::new(bytes), preflight, recover),
            Err(PsdError::LimitExceeded("test import budget"))
        ));
        assert!(!recover_called.get());
    }
}

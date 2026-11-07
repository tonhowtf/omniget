//! Layer records, layer masks, blending ranges and per-layer channel data.

use std::borrow::Cow;

use crate::blend::BlendMode;
use crate::compression::{Compression, PlaneLayout, decode_planes, encode_planes, rle_widening_len};
use crate::error::{PsdError, Result};
use crate::header::{MAX_DIMENSION, Version};
use crate::io::{Reader, Sink, WriteExt, decode_legacy_name, read_pascal, write_pascal};
use crate::source::Stored;
use crate::tagged::{BlockData, SectionDivider, SectionType, TaggedBlock, read_blocks, uses_long_length, write_blocks};

/// Channel id of the transparency mask.
pub const CHANNEL_TRANSPARENCY: i16 = -1;
/// Channel id of the user-supplied layer mask.
pub const CHANNEL_USER_MASK: i16 = -2;
/// Channel id of the real user-supplied layer mask (when both a user mask and
/// a vector mask are present).
pub const CHANNEL_REAL_USER_MASK: i16 = -3;

/// Maximum number of channels per layer accepted by the parser.
pub const MAX_LAYER_CHANNELS: u16 = 64;

/// A rectangle as stored in PSD files (top, left, bottom, right).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Rect {
    /// Top edge (inclusive).
    pub top: i32,
    /// Left edge (inclusive).
    pub left: i32,
    /// Bottom edge (exclusive).
    pub bottom: i32,
    /// Right edge (exclusive).
    pub right: i32,
}

impl Rect {
    /// Rectangle from origin and size.
    pub fn from_xywh(left: i32, top: i32, width: u32, height: u32) -> Self {
        Rect { top, left, bottom: top.saturating_add(height.min(i32::MAX as u32) as i32), right: left.saturating_add(width.min(i32::MAX as u32) as i32) }
    }
    /// Width (may be negative for malformed data).
    pub fn width(&self) -> i64 {
        i64::from(self.right) - i64::from(self.left)
    }
    /// Height (may be negative for malformed data).
    pub fn height(&self) -> i64 {
        i64::from(self.bottom) - i64::from(self.top)
    }
    /// `true` when width or height is zero or negative.
    pub fn is_empty(&self) -> bool {
        self.width() <= 0 || self.height() <= 0
    }
    /// Validated `(width, height)` as `usize`.
    pub fn size(&self) -> Result<(usize, usize)> {
        let (w, h) = (self.width(), self.height());
        if w < 0 || h < 0 {
            return Err(PsdError::invalid("rectangle has negative size"));
        }
        if w > i64::from(MAX_DIMENSION) || h > i64::from(MAX_DIMENSION) {
            return Err(PsdError::LimitExceeded("layer dimensions exceed 300000"));
        }
        Ok((w as usize, h as usize))
    }
    fn read(r: &mut Reader<'_>) -> Result<Self> {
        Ok(Rect { top: r.i32()?, left: r.i32()?, bottom: r.i32()?, right: r.i32()? })
    }
    fn write(&self, out: &mut Vec<u8>) {
        out.put_i32(self.top);
        out.put_i32(self.left);
        out.put_i32(self.bottom);
        out.put_i32(self.right);
    }
}

/// Layer record flag byte.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct LayerFlags(pub u8);

impl LayerFlags {
    /// Bit 0.
    pub const TRANSPARENCY_PROTECTED: u8 = 1 << 0;
    /// Bit 1 (set = hidden).
    pub const HIDDEN: u8 = 1 << 1;
    /// Bit 2 (obsolete).
    pub const OBSOLETE: u8 = 1 << 2;
    /// Bit 3: bit 4 carries useful information.
    pub const BIT4_USEFUL: u8 = 1 << 3;
    /// Bit 4: pixel data irrelevant to the appearance of the document.
    pub const PIXEL_DATA_IRRELEVANT: u8 = 1 << 4;

    fn bit(self, b: u8) -> bool {
        self.0 & b != 0
    }
    fn set(&mut self, b: u8, on: bool) {
        if on {
            self.0 |= b;
        } else {
            self.0 &= !b;
        }
    }
    /// Transparency protected.
    pub fn transparency_protected(self) -> bool {
        self.bit(Self::TRANSPARENCY_PROTECTED)
    }
    /// Hidden.
    pub fn hidden(self) -> bool {
        self.bit(Self::HIDDEN)
    }
    /// Bit 4 is meaningful.
    pub fn bit4_useful(self) -> bool {
        self.bit(Self::BIT4_USEFUL)
    }
    /// Pixel data irrelevant (only meaningful when `bit4_useful`).
    pub fn pixel_data_irrelevant(self) -> bool {
        self.bit(Self::PIXEL_DATA_IRRELEVANT)
    }
    /// Sets hidden.
    pub fn set_hidden(&mut self, v: bool) {
        self.set(Self::HIDDEN, v);
    }
    /// Sets transparency protection.
    pub fn set_transparency_protected(&mut self, v: bool) {
        self.set(Self::TRANSPARENCY_PROTECTED, v);
    }
    /// Sets pixel-data-irrelevant (also sets bit 3).
    pub fn set_pixel_data_irrelevant(&mut self, v: bool) {
        self.set(Self::BIT4_USEFUL, true);
        self.set(Self::PIXEL_DATA_IRRELEVANT, v);
    }
}

/// Optional mask parameters (present when mask flag bit 4 is set).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct MaskParameters {
    /// Parameter flags byte (bit 0 user density, 1 user feather, 2 vector
    /// density, 3 vector feather).
    pub flags: u8,
    /// User mask density.
    pub user_density: Option<u8>,
    /// User mask feather.
    pub user_feather: Option<f64>,
    /// Vector mask density.
    pub vector_density: Option<u8>,
    /// Vector mask feather.
    pub vector_feather: Option<f64>,
}

/// "Real" user mask fields (present in 36+-byte mask records).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct RealMask {
    /// Real flags.
    pub flags: u8,
    /// Real user mask background (0 or 255).
    pub background: u8,
    /// Real mask rectangle.
    pub rect: Rect,
}

/// Layer mask / adjustment layer data.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct LayerMask {
    /// Mask rectangle.
    pub rect: Rect,
    /// Default color outside the rectangle (0 or 255).
    pub default_color: u8,
    /// Flags: bit 0 position relative to layer, 1 disabled, 2 invert
    /// (obsolete), 3 user mask came from rendering other data, 4 parameters
    /// applied.
    pub flags: u8,
    /// Mask parameters (when flag bit 4 is set).
    pub parameters: Option<MaskParameters>,
    /// Real user mask fields.
    pub real: Option<RealMask>,
    /// Remaining bytes (the 2 padding bytes of the 20-byte form, etc.).
    pub trailing: Vec<u8>,
    /// When both are present, the real-mask fields come before the mask parameters. Photoshop
    /// writes this order (the published spec lists the parameters first); `false` keeps a file
    /// that uses the spec's order byte-stable.
    #[cfg_attr(feature = "serde", serde(default = "default_true"))]
    pub real_first: bool,
}

#[cfg(feature = "serde")]
fn default_true() -> bool {
    true
}

/// Byte length of the mask parameter values announced by the parameter flags byte `pf`.
pub(crate) fn mask_parameter_bytes(pf: u8) -> usize {
    usize::from(pf & 1 != 0) + 8 * usize::from(pf & 2 != 0) + usize::from(pf & 4 != 0) + 8 * usize::from(pf & 8 != 0)
}

impl LayerMask {
    /// Mask flag: position relative to layer.
    pub const FLAG_RELATIVE: u8 = 1 << 0;
    /// Mask flag: disabled.
    pub const FLAG_DISABLED: u8 = 1 << 1;
    /// Mask flag: invert when blending (obsolete).
    pub const FLAG_INVERT: u8 = 1 << 2;
    /// Mask flag: parameters present.
    pub const FLAG_PARAMETERS: u8 = 1 << 4;

    /// The simple 20-byte form.
    pub fn new(rect: Rect, default_color: u8, flags: u8) -> Self {
        LayerMask { rect, default_color, flags: flags & !Self::FLAG_PARAMETERS, parameters: None, real: None, trailing: vec![0, 0], real_first: true }
    }
    /// `true` if the disabled flag is set.
    pub fn disabled(&self) -> bool {
        self.flags & Self::FLAG_DISABLED != 0
    }

    /// Whether `d` (a whole mask record with mask parameters) stores the real-mask fields before
    /// the parameters, as Photoshop does. The spec's order wins when it reads as parameters
    /// followed by padding alone (no real mask), the one layout the two orders can share. When
    /// both orders fit the record's length, Photoshop's is taken unless its reading is
    /// implausible (a real-mask background that is neither 0 nor 255, or parameter flag bits
    /// the spec does not define) while the spec's reading with a real mask is sound.
    pub(crate) fn real_before_parameters(d: &[u8]) -> bool {
        // Parameters at `pf_at`, then 0..=3 bytes of padding, and nothing else.
        let fits =
            |pf_at: usize| d.get(pf_at).is_some_and(|&pf| (pf_at + 1 + mask_parameter_bytes(pf)..=pf_at + 4 + mask_parameter_bytes(pf)).contains(&d.len()));
        if fits(18) {
            return false;
        }
        let photoshop = fits(36);
        if !photoshop {
            return false;
        }
        let known_flags = |pf: u8| pf & 0xF0 == 0;
        let real_background_ok = |at: usize| d.get(at + 1).is_some_and(|&bg| bg == 0 || bg == 255);
        // Photoshop's reading: real-mask fields at 18, parameters at 36.
        let photoshop_plausible = real_background_ok(18) && d.get(36).is_some_and(|&pf| known_flags(pf));
        // The spec's reading: parameters at 18, then the 18 real-mask bytes, then padding.
        let spec_plausible = d.get(18).is_some_and(|&pf| {
            let real_at = 19 + mask_parameter_bytes(pf);
            known_flags(pf) && (real_at + 18..=real_at + 21).contains(&d.len()) && real_background_ok(real_at)
        });
        photoshop_plausible || !spec_plausible
    }

    fn parse(d: &[u8]) -> Result<Self> {
        let mut r = Reader::new(d);
        let rect = Rect::read(&mut r)?;
        let default_color = r.u8()?;
        let flags = r.u8()?;
        let read_real = |r: &mut Reader<'_>| -> Result<Option<RealMask>> {
            Ok(if r.remaining() >= 18 { Some(RealMask { flags: r.u8()?, background: r.u8()?, rect: Rect::read(r)? }) } else { None })
        };
        let has_parameters = flags & Self::FLAG_PARAMETERS != 0;
        let real_first = has_parameters && Self::real_before_parameters(d);
        let mut real = if real_first { read_real(&mut r)? } else { None };
        let parameters = if has_parameters {
            let pf = r.u8()?;
            Some(MaskParameters {
                flags: pf,
                user_density: if pf & 1 != 0 { Some(r.u8()?) } else { None },
                user_feather: if pf & 2 != 0 { Some(r.f64()?) } else { None },
                vector_density: if pf & 4 != 0 { Some(r.u8()?) } else { None },
                vector_feather: if pf & 8 != 0 { Some(r.f64()?) } else { None },
            })
        } else {
            None
        };
        if !real_first {
            real = read_real(&mut r)?;
        }
        // The order only matters when both are present; otherwise default to Photoshop's.
        let real_first = real_first || real.is_none() || parameters.is_none();
        Ok(LayerMask { rect, default_color, flags, parameters, real, trailing: r.peek_rest().to_vec(), real_first })
    }

    /// Bytes [`Self::write`] produces.
    fn encoded_len(&self) -> u64 {
        let real = if self.real.is_some() { 18 } else { 0 };
        let parameters = self.parameters.as_ref().map_or(0, |p| {
            1 + u64::from(p.user_density.is_some())
                + 8 * u64::from(p.user_feather.is_some())
                + u64::from(p.vector_density.is_some())
                + 8 * u64::from(p.vector_feather.is_some())
        });
        18 + real + parameters + self.trailing.len() as u64
    }

    fn write(&self, out: &mut Vec<u8>) {
        self.rect.write(out);
        out.put_u8(self.default_color);
        out.put_u8(self.flags);
        let write_real = |out: &mut Vec<u8>| {
            if let Some(real) = &self.real {
                out.put_u8(real.flags);
                out.put_u8(real.background);
                real.rect.write(out);
            }
        };
        if self.real_first {
            write_real(out);
        }
        if let Some(p) = &self.parameters {
            out.put_u8(p.flags);
            if let Some(v) = p.user_density {
                out.put_u8(v);
            }
            if let Some(v) = p.user_feather {
                out.put_f64(v);
            }
            if let Some(v) = p.vector_density {
                out.put_u8(v);
            }
            if let Some(v) = p.vector_feather {
                out.put_f64(v);
            }
        }
        if !self.real_first {
            write_real(out);
        }
        out.put(&self.trailing);
    }
}

/// The layer mask data field of a layer record.
#[derive(Debug, Clone, PartialEq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum MaskData {
    /// Length 0.
    #[default]
    None,
    /// Parsed mask.
    Mask(LayerMask),
    /// Unparseable mask data, preserved.
    Raw(Vec<u8>),
}

/// Layer blending ranges ("Blend If"), stored raw. The data is a sequence of
/// 8-byte entries: composite gray first, then one per channel; each entry is
/// source (black lo, black hi, white lo, white hi) then destination.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct BlendingRanges {
    /// Raw bytes.
    pub data: Vec<u8>,
}

/// One blending range entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BlendRange {
    /// Source range: black low, black high, white low, white high.
    pub source: [u8; 4],
    /// Destination range.
    pub dest: [u8; 4],
}

impl BlendingRanges {
    /// Default "full" ranges for composite + `channels` channels.
    pub fn full(channels: usize) -> Self {
        let mut data = Vec::with_capacity((channels + 1) * 8);
        for _ in 0..=channels {
            data.extend_from_slice(&[0, 0, 255, 255, 0, 0, 255, 255]);
        }
        BlendingRanges { data }
    }
    /// Parsed entries (incomplete trailing bytes are ignored).
    pub fn ranges(&self) -> Vec<BlendRange> {
        self.data.as_chunks::<8>().0.iter().map(|c| BlendRange { source: [c[0], c[1], c[2], c[3]], dest: [c[4], c[5], c[6], c[7]] }).collect()
    }
}

/// Encoded image data for one layer channel.
///
/// The encoded bytes are in `data`, or, for a file parsed by
/// [`PsdFile::from_source`](crate::PsdFile::from_source), still in the source at `stored` (and
/// `data` is empty). Every method here reads them from wherever they are; two channels are equal
/// when their bytes are, wherever each one keeps them.
#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ChannelData {
    /// Channel id: 0.. color channels, -1 transparency, -2 user mask,
    /// -3 real user mask.
    pub id: i16,
    /// Compression; `None` when the stored length was 0 (no compression
    /// field at all).
    pub compression: Option<Compression>,
    /// Encoded bytes following the compression field, kept verbatim (empty while `stored`).
    pub data: Vec<u8>,
    /// Where the encoded bytes are when they were left in the source: see [`Self::load`].
    /// (Not serialized: load the channel first.)
    #[cfg_attr(feature = "serde", serde(skip))]
    pub stored: Option<Stored>,
}

impl PartialEq for ChannelData {
    fn eq(&self, other: &Self) -> bool {
        if self.id != other.id || self.compression != other.compression || self.encoded_len() != other.encoded_len() {
            return false;
        }
        match (self.encoded(), other.encoded()) {
            (Ok(a), Ok(b)) => a == b,
            _ => false,
        }
    }
}

impl Eq for ChannelData {}

impl ChannelData {
    /// Encodes planar samples (`height × row_bytes`).
    pub fn encode(id: i16, compression: Compression, decoded: &[u8], width: usize, height: usize, depth: u16, version: Version) -> Result<Self> {
        let layout = PlaneLayout { planes: 1, width, height, depth, version };
        Ok(ChannelData::new(id, Some(compression), encode_planes(compression, decoded, &layout)?))
    }

    /// A channel whose encoded bytes are in memory.
    pub fn new(id: i16, compression: Option<Compression>, data: Vec<u8>) -> Self {
        ChannelData { id, compression, data, stored: None }
    }

    /// The encoded bytes (after the compression field): borrowed from `data`, or read from the
    /// source when they were left there.
    pub fn encoded(&self) -> Result<Cow<'_, [u8]>> {
        crate::source::encoded(&self.data, self.stored.as_ref())
    }

    /// Number of encoded bytes, wherever they are.
    pub fn encoded_len(&self) -> u64 {
        self.stored.as_ref().map_or(self.data.len() as u64, |s| s.len)
    }

    /// Reads bytes left in the source into `data`, so the channel no longer needs its source.
    pub fn load(&mut self) -> Result<()> {
        if let Some(s) = &self.stored {
            self.data = s.read()?;
            self.stored = None;
        }
        Ok(())
    }

    /// Decodes into planar big-endian samples.
    pub fn decode(&self, width: usize, height: usize, depth: u16, version: Version) -> Result<Vec<u8>> {
        let layout = PlaneLayout { planes: 1, width, height, depth, version };
        match self.compression {
            Some(c) => decode_planes(c, &self.encoded()?, &layout),
            None => {
                if layout.decoded_len()? == 0 {
                    Ok(Vec::new())
                } else {
                    Err(PsdError::invalid("channel has no data"))
                }
            }
        }
    }

    /// Replaces the channel contents, re-encoding with `compression`.
    #[allow(clippy::too_many_arguments)]
    pub fn set_decoded(&mut self, compression: Compression, decoded: &[u8], width: usize, height: usize, depth: u16, version: Version) -> Result<()> {
        *self = Self::encode(self.id, compression, decoded, width, height, depth, version)?;
        Ok(())
    }

    fn stored_len(&self) -> u64 {
        match self.compression {
            Some(_) => 2 + self.encoded_len(),
            None => 0,
        }
    }
}

/// A layer record together with its channel image data.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct LayerRecord {
    /// Layer bounds.
    pub rect: Rect,
    /// Channels in stored order, including encoded image data.
    pub channels: Vec<ChannelData>,
    /// Blend mode.
    pub blend_mode: BlendMode,
    /// Opacity 0..=255.
    pub opacity: u8,
    /// Clipping: 0 base, 1 non-base (clipped to layer below).
    pub clipping: u8,
    /// Flags.
    pub flags: LayerFlags,
    /// Filler byte (zero), preserved.
    pub filler: u8,
    /// Layer mask data.
    pub mask: MaskData,
    /// Blending ranges.
    pub blending_ranges: BlendingRanges,
    /// Legacy Pascal name bytes (padded to 4 in the file).
    pub name: Vec<u8>,
    /// Additional layer information blocks.
    pub blocks: Vec<TaggedBlock>,
    /// Bytes at the end of the extra-data field that do not form a block.
    pub extra_trailing: Vec<u8>,
}

impl Default for LayerRecord {
    fn default() -> Self {
        LayerRecord {
            rect: Rect::default(),
            channels: Vec::new(),
            blend_mode: BlendMode::Normal,
            opacity: 255,
            clipping: 0,
            flags: LayerFlags(0),
            filler: 0,
            mask: MaskData::None,
            blending_ranges: BlendingRanges::default(),
            name: Vec::new(),
            blocks: Vec::new(),
            extra_trailing: Vec::new(),
        }
    }
}

impl LayerRecord {
    /// Layer name, preferring the Unicode `luni` block over the legacy name.
    pub fn name(&self) -> String {
        for b in &self.blocks {
            if &b.key == b"luni"
                && let Some(Ok(BlockData::UnicodeName(n))) = b.parsed()
            {
                return n;
            }
        }
        decode_legacy_name(&self.name)
    }

    /// First block with the given key.
    pub fn block(&self, key: &[u8; 4]) -> Option<&TaggedBlock> {
        self.blocks.iter().find(|b| &b.key == key)
    }

    /// Mutable access to the first block with the given key.
    pub fn block_mut(&mut self, key: &[u8; 4]) -> Option<&mut TaggedBlock> {
        self.blocks.iter_mut().find(|b| &b.key == key)
    }

    /// Section divider (`lsct`, falling back to `lsdk`), if present and valid.
    pub fn section_divider(&self) -> Option<SectionDivider> {
        for key in [b"lsct", b"lsdk"] {
            if let Some(Ok(BlockData::SectionDivider(s))) = self.block(key).and_then(TaggedBlock::parsed) {
                return Some(s);
            }
        }
        None
    }

    /// Section type (Other if no divider).
    pub fn section_type(&self) -> SectionType {
        self.section_divider().map_or(SectionType::Other, |s| s.kind)
    }

    /// `true` unless the hidden flag is set.
    pub fn is_visible(&self) -> bool {
        !self.flags.hidden()
    }

    /// Layer id from `lyid`.
    pub fn layer_id(&self) -> Option<u32> {
        match self.block(b"lyid")?.parsed()? {
            Ok(BlockData::LayerId(v)) => Some(v),
            _ => None,
        }
    }

    /// Fill opacity from `iOpa` (255 if absent).
    pub fn fill_opacity(&self) -> u8 {
        match self.block(b"iOpa").and_then(TaggedBlock::parsed) {
            Some(Ok(BlockData::FillOpacity(v))) => v,
            _ => 255,
        }
    }

    /// Parsed layer mask, if any.
    pub fn layer_mask(&self) -> Option<&LayerMask> {
        match &self.mask {
            MaskData::Mask(m) => Some(m),
            _ => None,
        }
    }

    /// Channel with the given id.
    pub fn channel(&self, id: i16) -> Option<&ChannelData> {
        self.channels.iter().find(|c| c.id == id)
    }

    /// Rectangle that a channel's pixels cover: the layer rect for color and
    /// transparency channels, the mask rect for -2, the real mask rect for -3.
    pub fn channel_rect(&self, id: i16) -> Rect {
        match (id, &self.mask) {
            (CHANNEL_USER_MASK, MaskData::Mask(m)) => m.rect,
            (CHANNEL_REAL_USER_MASK, MaskData::Mask(m)) => m.real.map_or(m.rect, |r| r.rect),
            _ => self.rect,
        }
    }

    /// Decodes a channel's samples (planar, big-endian).
    pub fn decode_channel(&self, id: i16, depth: u16, version: Version) -> Result<Vec<u8>> {
        let ch = self.channel(id).ok_or_else(|| PsdError::invalid(format!("layer has no channel {id}")))?;
        let (w, h) = self.channel_rect(id).size()?;
        ch.decode(w, h, depth, version)
    }

    pub(crate) fn read(r: &mut Reader<'_>, version: Version) -> Result<(Self, Vec<u64>)> {
        let rect = Rect::read(r)?;
        let nch = r.u16()?;
        if nch > MAX_LAYER_CHANNELS {
            return Err(PsdError::LimitExceeded("too many channels in layer"));
        }
        let mut channels = Vec::with_capacity(usize::from(nch));
        let mut lens = Vec::with_capacity(usize::from(nch));
        for _ in 0..nch {
            let id = r.i16()?;
            let len = r.len_field(version.is_psb())?;
            channels.push(ChannelData::new(id, None, Vec::new()));
            lens.push(len);
        }
        let sig = r.array::<4>()?;
        if &sig != b"8BIM" {
            return Err(PsdError::InvalidSignature { expected: "8BIM (layer blend mode)", found: sig });
        }
        let blend_mode = BlendMode::from_key(r.array()?);
        let opacity = r.u8()?;
        let clipping = r.u8()?;
        let flags = LayerFlags(r.u8()?);
        let filler = r.u8()?;
        let extra_len = r.u32()?;
        let mut x = r.sub(u64::from(extra_len))?;
        let mask_len = x.u32()?;
        let mask_bytes = x.bytes_u64(u64::from(mask_len))?;
        let mask = if mask_bytes.is_empty() {
            MaskData::None
        } else {
            match LayerMask::parse(mask_bytes) {
                Ok(m) => MaskData::Mask(m),
                Err(_) => MaskData::Raw(mask_bytes.to_vec()),
            }
        };
        let br_len = x.u32()?;
        let blending_ranges = BlendingRanges { data: x.bytes_u64(u64::from(br_len))?.to_vec() };
        let name = read_pascal(&mut x, 4)?;
        let (blocks, extra_trailing) = read_blocks(&mut x, version)?;
        Ok((LayerRecord { rect, channels, blend_mode, opacity, clipping, flags, filler, mask, blending_ranges, name, blocks, extra_trailing }, lens))
    }

    /// Rows of a channel's RLE count table that grow when a PSD model is written as PSB
    /// (`widen`, see [`crate::PsdFile::promote_to_psb`]); 0 for other channels.
    pub(crate) fn widened_rows(&self, c: &ChannelData, widen: bool) -> Result<usize> {
        if widen && c.compression == Some(Compression::Rle) { Ok(self.channel_rect(c.id).size()?.1) } else { Ok(0) }
    }

    /// Bytes [`Self::write`] produces, computed without serializing. Errors where `write` would.
    fn encoded_len(&self, version: Version) -> Result<u64> {
        let too_big = || PsdError::LimitExceeded("section too large for 32-bit length (use PSB)");
        u16::try_from(self.channels.len()).map_err(|_| PsdError::LimitExceeded("too many channels"))?;
        let channel_len: u64 = if version.is_psb() { 8 } else { 4 };
        if !version.is_psb() && self.channels.iter().any(|c| u32::try_from(c.stored_len()).is_err()) {
            return Err(too_big());
        }
        let mask = match &self.mask {
            MaskData::None => 0,
            MaskData::Mask(mask) => mask.encoded_len(),
            MaskData::Raw(b) => b.len() as u64,
        };
        u32::try_from(mask).map_err(|_| too_big())?;
        let ranges = self.blending_ranges.data.len() as u64;
        u32::try_from(ranges).map_err(|_| too_big())?;
        let name = 1 + self.name.len().min(255) as u64;
        let name = name.next_multiple_of(4);
        let mut extra = (4 + mask).saturating_add(4 + ranges).saturating_add(name).saturating_add(self.extra_trailing.len() as u64);
        for b in &self.blocks {
            if !uses_long_length(version, &b.key) && u32::try_from(b.data.len()).is_err() {
                return Err(too_big());
            }
            extra = extra.saturating_add(b.encoded_len(version) as u64);
        }
        u32::try_from(extra).map_err(|_| too_big())?;
        // rect, channel count, channel entries, signature + blend key, four flag bytes, extra length.
        Ok((16 + 2 + self.channels.len() as u64 * (2 + channel_len) + 8 + 4 + 4).saturating_add(extra))
    }

    fn write(&self, out: &mut Vec<u8>, version: Version, widen: bool) -> Result<()> {
        self.rect.write(out);
        let nch = u16::try_from(self.channels.len()).map_err(|_| PsdError::LimitExceeded("too many channels"))?;
        out.put_u16(nch);
        for c in &self.channels {
            out.put_i16(c.id);
            let rows = self.widened_rows(c, widen)?;
            let grown = if rows == 0 { 0 } else { rle_widening_len(c.encoded_len(), rows)? };
            out.put_len(c.stored_len().saturating_add(grown), version.is_psb())?;
        }
        out.put(b"8BIM");
        out.put(&self.blend_mode.key());
        out.put_u8(self.opacity);
        out.put_u8(self.clipping);
        out.put_u8(self.flags.0);
        out.put_u8(self.filler);
        let at = out.begin_len(false);
        let m = out.begin_len(false);
        match &self.mask {
            MaskData::None => {}
            MaskData::Mask(mask) => mask.write(out),
            MaskData::Raw(b) => out.put(b),
        }
        out.end_len(m, false)?;
        out.put_len(self.blending_ranges.data.len() as u64, false)?;
        out.put(&self.blending_ranges.data);
        write_pascal(out, &self.name, 4);
        write_blocks(out, &self.blocks, version, 2)?;
        out.put(&self.extra_trailing);
        out.end_len(at, false)
    }
}

/// Layer info: layer records plus their channel image data.
#[derive(Debug, Clone, PartialEq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct LayerInfo {
    /// The layer count was stored negative: the first alpha channel of the
    /// merged image contains the transparency of the merged result.
    pub merged_alpha: bool,
    /// Layer records, bottom-most first (file order).
    pub layers: Vec<LayerRecord>,
    /// Bytes after the channel data. `None` = default (zero pad to even).
    pub padding: Option<Vec<u8>>,
}

impl LayerInfo {
    /// Parses a layer info body (the bytes covered by its length field).
    pub(crate) fn read_body(data: &[u8], version: Version) -> Result<Self> {
        let mut r = Reader::new(data);
        let count = r.i16()?;
        let merged_alpha = count < 0;
        let n = count.unsigned_abs();
        // Minimum record size: 16 rect + 2 + 4 sig + 4 key + 4 + 4 extra len.
        r.check_count(u64::from(n), 34)?;
        let mut layers = Vec::with_capacity(usize::from(n));
        let mut all_lens = Vec::with_capacity(usize::from(n));
        for _ in 0..n {
            let (rec, lens) = LayerRecord::read(&mut r, version)?;
            layers.push(rec);
            all_lens.push(lens);
        }
        for (rec, lens) in layers.iter_mut().zip(all_lens) {
            for (ch, len) in rec.channels.iter_mut().zip(lens) {
                if len == 0 {
                    continue;
                }
                if len == 1 {
                    return Err(PsdError::invalid("channel data length 1"));
                }
                ch.compression = Some(Compression::from_u16(r.u16()?));
                ch.data = r.bytes_u64(len - 2)?.to_vec();
            }
        }
        let body_len = r.pos();
        let rest = r.peek_rest();
        let padding = if rest.len() == body_len % 2 && rest.iter().all(|&b| b == 0) { None } else { Some(rest.to_vec()) };
        Ok(LayerInfo { merged_alpha, layers, padding })
    }

    /// Length of the body (layer count, records and channel data) without
    /// its trailing padding.
    pub fn unpadded_len(&self, version: Version) -> Result<u64> {
        self.unpadded_len_as(version, false)
    }

    /// [`Self::unpadded_len`] of the body written with its RLE count tables widened (`widen`).
    pub(crate) fn unpadded_len_as(&self, version: Version, widen: bool) -> Result<u64> {
        // Computed from the field sizes: serializing the records just to measure them would copy
        // every tagged block. Widening changes channel data only: the record's length fields keep
        // their width.
        let mut n = 2u64;
        for l in &self.layers {
            n = n.saturating_add(l.encoded_len(version)?);
        }
        for l in &self.layers {
            for c in l.channels.iter().filter(|c| c.compression.is_some()) {
                let rows = l.widened_rows(c, widen)?;
                let grown = if rows == 0 { 0 } else { rle_widening_len(c.encoded_len(), rows)? };
                n = n.saturating_add(2).saturating_add(c.encoded_len()).saturating_add(grown);
            }
        }
        Ok(n)
    }

    /// Sets the padding so the body length is a multiple of `align`, as
    /// Photoshop writes it (4). Readers that step over a global `Lr16`/`Lr32`
    /// block in 4-byte units (psd-tools) misread the blocks after a body that
    /// is only padded to even.
    pub fn pad_to(&mut self, version: Version, align: u64) -> Result<()> {
        self.padding = Some(vec![0; self.pad_len(version, align)?]);
        Ok(())
    }

    /// Number of zero bytes [`Self::pad_to`] would set, without touching `self`.
    pub(crate) fn pad_len(&self, version: Version, align: u64) -> Result<usize> {
        self.pad_len_as(version, false, align)
    }

    /// [`Self::pad_len`] for the body written with its RLE count tables widened (`widen`).
    pub(crate) fn pad_len_as(&self, version: Version, widen: bool, align: u64) -> Result<usize> {
        let align = align.max(1);
        let len = self.unpadded_len_as(version, widen)?;
        Ok(((align - len % align) % align) as usize)
    }

    /// Bytes of channel data and their compression fields: the part of the body worth reserving up
    /// front (the records are small next to the pixels). Saturates instead of overflowing.
    /// (Saves reserve their exact length now, #6607; the tests still check it as a lower bound.)
    #[cfg(test)]
    pub(crate) fn channel_data_len(&self) -> usize {
        self.layers
            .iter()
            .flat_map(|l| &l.channels)
            .filter(|c| c.compression.is_some())
            .fold(0usize, |n, c| n.saturating_add(2).saturating_add(usize::try_from(c.encoded_len()).unwrap_or(usize::MAX)))
    }

    /// Serializes the body without any trailing padding (the padding field is ignored).
    #[cfg(test)]
    pub(crate) fn write_unpadded(&self, out: &mut Vec<u8>, version: Version) -> Result<()> {
        crate::io::append_with(out, |s| self.write_unpadded_to(s, version, false))
    }

    /// Serializes the body (without a length field).
    #[cfg(test)]
    pub(crate) fn write_body(&self, out: &mut Vec<u8>, version: Version) -> Result<()> {
        crate::io::append_with(out, |s| self.write_body_to(s, version, false))
    }

    /// [`Self::write_unpadded`] to a streaming [`Sink`], widening the RLE count tables of a PSD
    /// model written as PSB (`widen`) on the way out: the records are staged, the channel data
    /// goes straight through (#6607).
    pub(crate) fn write_unpadded_to<W: std::io::Write + std::io::Seek + ?Sized>(
        &self,
        out: &mut Sink<'_, W>,
        version: Version,
        widen: bool,
    ) -> std::io::Result<()> {
        let n = i16::try_from(self.layers.len()).map_err(|_| PsdError::LimitExceeded("more than 32767 layers"))?;
        out.small(|o| {
            o.put_i16(if self.merged_alpha { -n } else { n });
            Ok(())
        })?;
        for l in &self.layers {
            out.small(|o| l.write(o, version, widen))?;
        }
        for l in &self.layers {
            for c in &l.channels {
                if let Some(comp) = c.compression {
                    out.small(|o| {
                        o.put_u16(comp.as_u16());
                        Ok(())
                    })?;
                    match l.widened_rows(c, widen)? {
                        0 => out.put_encoded(&c.data, c.stored.as_ref())?,
                        rows => out.put_widened_encoded(&c.data, c.stored.as_ref(), rows)?,
                    }
                }
            }
        }
        Ok(())
    }

    /// The body (without a length field) to a streaming [`Sink`], widening RLE count tables when
    /// `widen` (see [`Self::write_unpadded_to`]).
    pub(crate) fn write_body_to<W: std::io::Write + std::io::Seek + ?Sized>(
        &self,
        out: &mut Sink<'_, W>,
        version: Version,
        widen: bool,
    ) -> std::io::Result<()> {
        let start = out.pos();
        self.write_unpadded_to(out, version, widen)?;
        match &self.padding {
            Some(p) => out.put(p),
            None => out.zeros(((out.pos() - start) % 2) as usize),
        }
    }
}

#[cfg(test)]
mod tests;

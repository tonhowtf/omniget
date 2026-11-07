//! [`PsdBuilder`]: construct PSD/PSB files from pixel buffers.
//!
//! The builder never composites. Supply the merged image with
//! [`PsdBuilder::composite`]; otherwise a white placeholder is written and
//! resource 1057 records `has_real_merged_data = false`.

use crate::blend::BlendMode;
use crate::compression::Compression;
use crate::error::{PsdError, Result};
use crate::file::{GlobalLayerMask, LayerInfoPlacement, PsdFile};
use crate::header::{ColorMode, Header, Version};
use crate::image_data::ImageData;
use crate::io::encode_legacy_name;
use crate::layer::{BlendingRanges, ChannelData, LayerFlags, LayerInfo, LayerMask, LayerRecord, MaskData, Rect};
use crate::resources::{ImageResource, ResolutionInfo, ids, version_info_resource};
use crate::tagged::{SectionType, TaggedBlock};

/// Interleaved pixel data with alpha. CMYK values are *ink amounts*
/// (0 = no ink); the builder stores them inverted as Photoshop does.
#[derive(Debug, Clone, PartialEq)]
pub enum PixelData {
    /// RGBA, 8-bit.
    Rgba8(Vec<u8>),
    /// RGBA, 16-bit.
    Rgba16(Vec<u16>),
    /// Gray + alpha, 8-bit.
    GrayA8(Vec<u8>),
    /// Gray + alpha, 16-bit.
    GrayA16(Vec<u16>),
    /// CMYK ink + alpha, 8-bit.
    Cmyka8(Vec<u8>),
    /// CMYK ink + alpha, 16-bit.
    Cmyka16(Vec<u16>),
}

impl PixelData {
    fn mode_depth(&self) -> (ColorMode, u16) {
        match self {
            PixelData::Rgba8(_) => (ColorMode::Rgb, 8),
            PixelData::Rgba16(_) => (ColorMode::Rgb, 16),
            PixelData::GrayA8(_) => (ColorMode::Grayscale, 8),
            PixelData::GrayA16(_) => (ColorMode::Grayscale, 16),
            PixelData::Cmyka8(_) => (ColorMode::Cmyk, 8),
            PixelData::Cmyka16(_) => (ColorMode::Cmyk, 16),
        }
    }
    /// Samples per pixel including alpha.
    fn spp(&self) -> usize {
        match self.mode_depth().0 {
            ColorMode::Rgb => 4,
            ColorMode::Grayscale => 2,
            _ => 5,
        }
    }
    fn len(&self) -> usize {
        match self {
            PixelData::Rgba8(v) | PixelData::GrayA8(v) | PixelData::Cmyka8(v) => v.len(),
            PixelData::Rgba16(v) | PixelData::GrayA16(v) | PixelData::Cmyka16(v) => v.len(),
        }
    }
    /// Planar big-endian plane for interleaved component `c`, as stored
    /// (CMYK inverted).
    fn plane(&self, c: usize, n: usize) -> Vec<u8> {
        let spp = self.spp();
        let invert = self.mode_depth().0 == ColorMode::Cmyk && c < 4;
        match self {
            PixelData::Rgba8(v) | PixelData::GrayA8(v) | PixelData::Cmyka8(v) => {
                (0..n).map(|i| if invert { 255 - v[i * spp + c] } else { v[i * spp + c] }).collect()
            }
            PixelData::Rgba16(v) | PixelData::GrayA16(v) | PixelData::Cmyka16(v) => (0..n)
                .flat_map(|i| {
                    let s = v[i * spp + c];
                    (if invert { 65535 - s } else { s }).to_be_bytes()
                })
                .collect(),
        }
    }
    fn alpha_is_opaque(&self, n: usize) -> bool {
        let spp = self.spp();
        match self {
            PixelData::Rgba8(v) | PixelData::GrayA8(v) | PixelData::Cmyka8(v) => (0..n).all(|i| v[i * spp + spp - 1] == 255),
            PixelData::Rgba16(v) | PixelData::GrayA16(v) | PixelData::Cmyka16(v) => (0..n).all(|i| v[i * spp + spp - 1] == 65535),
        }
    }

    fn composite_plane(&self, c: usize, n: usize) -> Vec<u8> {
        // Photoshop stores transparent RGB composites against white; layer channels stay straight,
        // and so do Gray and CMYK composites here (see `PsdBuilder::composite`).
        match self {
            Self::Rgba8(v) if c < 3 => v
                .as_chunks::<4>()
                .0
                .iter()
                .take(n)
                .map(|p| {
                    let (color, alpha) = (u32::from(p[c]), u32::from(p[3]));
                    if alpha == 0 { p[c] } else { ((color * alpha + 127) / 255 + 255 - alpha) as u8 }
                })
                .collect(),
            Self::Rgba16(v) if c < 3 => v
                .as_chunks::<4>()
                .0
                .iter()
                .take(n)
                .flat_map(|p| {
                    let (color, alpha) = (u32::from(p[c]), u32::from(p[3]));
                    let value = if alpha == 0 { p[c] } else { ((color * alpha + 32767) / 65535 + 65535 - alpha) as u16 };
                    value.to_be_bytes()
                })
                .collect(),
            _ => self.plane(c, n),
        }
    }
}

/// A layer mask given as 8-bit samples.
#[derive(Debug, Clone, PartialEq)]
pub struct MaskSpec {
    /// Mask rectangle in document coordinates.
    pub rect: Rect,
    /// `rect.width × rect.height` samples (0 = hidden, 255 = shown).
    pub data: Vec<u8>,
    /// Value outside the rectangle.
    pub default_color: u8,
    /// Mask disabled.
    pub disabled: bool,
}

/// A raster layer.
#[derive(Debug, Clone, PartialEq)]
pub struct LayerSpec {
    /// Name (written both as legacy Pascal name and `luni`).
    pub name: String,
    /// Left edge.
    pub left: i32,
    /// Top edge.
    pub top: i32,
    /// Width.
    pub width: u32,
    /// Height.
    pub height: u32,
    /// Pixels (must match the builder's color mode and depth).
    pub pixels: PixelData,
    /// Blend mode.
    pub blend_mode: BlendMode,
    /// Opacity 0..=255.
    pub opacity: u8,
    /// Fill opacity (`iOpa`), if not 255.
    pub fill_opacity: Option<u8>,
    /// Visible.
    pub visible: bool,
    /// Clipped to the layer below.
    pub clipping: bool,
    /// Optional user mask.
    pub mask: Option<MaskSpec>,
    /// Additional blocks appended verbatim.
    pub extra_blocks: Vec<TaggedBlock>,
}

impl LayerSpec {
    /// A visible normal layer at full opacity.
    pub fn new(name: impl Into<String>, left: i32, top: i32, width: u32, height: u32, pixels: PixelData) -> Self {
        LayerSpec {
            name: name.into(),
            left,
            top,
            width,
            height,
            pixels,
            blend_mode: BlendMode::Normal,
            opacity: 255,
            fill_opacity: None,
            visible: true,
            clipping: false,
            mask: None,
            extra_blocks: Vec::new(),
        }
    }
}

/// A group (folder).
#[derive(Debug, Clone, PartialEq)]
pub struct GroupSpec {
    /// Name.
    pub name: String,
    /// Blend mode (default pass-through).
    pub blend_mode: BlendMode,
    /// Opacity.
    pub opacity: u8,
    /// Visible.
    pub visible: bool,
    /// Open (expanded) in the layers panel.
    pub open: bool,
}

impl GroupSpec {
    /// A visible, open, pass-through group.
    pub fn new(name: impl Into<String>) -> Self {
        GroupSpec { name: name.into(), blend_mode: BlendMode::PassThrough, opacity: 255, visible: true, open: true }
    }
}

#[derive(Debug, Clone)]
enum Entry {
    Layer(LayerSpec),
    GroupStart,
    GroupEnd(GroupSpec),
}

/// Builds [`PsdFile`]s. Layers are pushed bottom-to-top.
#[derive(Debug, Clone)]
pub struct PsdBuilder {
    width: u32,
    height: u32,
    color_mode: ColorMode,
    depth: u16,
    version: Version,
    compression: Compression,
    entries: Vec<Entry>,
    open_groups: Vec<GroupSpec>,
    composite: Option<PixelData>,
    resources: Vec<ImageResource>,
}

impl PsdBuilder {
    /// An 8-bit RGB PSD using RLE compression.
    pub fn new(width: u32, height: u32) -> Self {
        PsdBuilder {
            width,
            height,
            color_mode: ColorMode::Rgb,
            depth: 8,
            version: Version::Psd,
            compression: Compression::Rle,
            entries: Vec::new(),
            open_groups: Vec::new(),
            composite: None,
            resources: Vec::new(),
        }
    }
    /// Sets the color mode (RGB, Grayscale or CMYK).
    pub fn color_mode(mut self, mode: ColorMode) -> Self {
        self.color_mode = mode;
        self
    }
    /// Sets the depth (8 or 16).
    pub fn depth(mut self, depth: u16) -> Self {
        self.depth = depth;
        self
    }
    /// Sets PSD or PSB.
    pub fn version(mut self, v: Version) -> Self {
        self.version = v;
        self
    }
    /// Sets the compression used for all channels and the merged image.
    pub fn compression(mut self, c: Compression) -> Self {
        self.compression = c;
        self
    }
    /// Adds a resolution resource.
    pub fn resolution(mut self, dpi: f64) -> Self {
        self.resources.push(ImageResource::new(ids::RESOLUTION_INFO, ResolutionInfo::from_dpi(dpi).to_bytes()));
        self
    }
    /// Adds an ICC profile resource.
    pub fn icc_profile(mut self, icc: Vec<u8>) -> Self {
        self.resources.push(ImageResource::new(ids::ICC_PROFILE, icc));
        self
    }
    /// Adds an arbitrary image resource.
    pub fn resource(mut self, r: ImageResource) -> Self {
        self.resources.push(r);
        self
    }
    /// Pushes a layer above the previous one.
    pub fn push_layer(&mut self, spec: LayerSpec) -> &mut Self {
        self.entries.push(Entry::Layer(spec));
        self
    }
    /// Opens a group; subsequent layers go inside until [`Self::end_group`].
    pub fn begin_group(&mut self, spec: GroupSpec) -> &mut Self {
        self.entries.push(Entry::GroupStart);
        self.open_groups.push(spec);
        self
    }
    /// Closes the innermost open group.
    pub fn end_group(&mut self) -> Result<&mut Self> {
        let g = self.open_groups.pop().ok_or_else(|| PsdError::invalid("end_group without begin_group"))?;
        self.entries.push(Entry::GroupEnd(g));
        Ok(self)
    }
    /// Supplies the merged composite (straight samples, same format as layers, full canvas).
    /// Transparent RGB is stored against white, quantized at the document depth. Gray and CMYK
    /// composites with alpha are stored straight (not matted against white as Photoshop would):
    /// a test that reads them as a Photoshop merged image has to allow for that.
    pub fn composite(&mut self, pixels: PixelData) -> &mut Self {
        self.composite = Some(pixels);
        self
    }

    fn color_channels(&self) -> Result<usize> {
        match self.color_mode {
            ColorMode::Rgb => Ok(3),
            ColorMode::Grayscale => Ok(1),
            ColorMode::Cmyk => Ok(4),
            m => Err(PsdError::Unsupported(format!("builder color mode {m:?}"))),
        }
    }

    fn check_pixels(&self, p: &PixelData, n: usize) -> Result<()> {
        if p.mode_depth() != (self.color_mode, self.depth) {
            return Err(PsdError::invalid(format!("pixel format {:?} does not match document {:?}/{}", p.mode_depth(), self.color_mode, self.depth)));
        }
        if p.len() != n * p.spp() {
            return Err(PsdError::invalid(format!("pixel buffer has {} samples, expected {}", p.len(), n * p.spp())));
        }
        Ok(())
    }

    fn channel(&self, id: i16, plane: &[u8], w: usize, h: usize) -> Result<ChannelData> {
        ChannelData::encode(id, self.compression, plane, w, h, self.depth, self.version)
    }

    fn empty_channels(&self, cc: usize) -> Result<Vec<ChannelData>> {
        let mut v = vec![self.channel(-1, &[], 0, 0)?];
        for c in 0..cc {
            v.push(self.channel(c as i16, &[], 0, 0)?);
        }
        Ok(v)
    }

    fn layer_record(&self, spec: &LayerSpec, cc: usize, id: u32) -> Result<LayerRecord> {
        let (w, h) = (spec.width as usize, spec.height as usize);
        let n = w * h;
        self.check_pixels(&spec.pixels, n)?;
        let mut channels = vec![self.channel(-1, &spec.pixels.plane(cc, n), w, h)?];
        for c in 0..cc {
            channels.push(self.channel(c as i16, &spec.pixels.plane(c, n), w, h)?);
        }
        let mut mask = MaskData::None;
        if let Some(m) = &spec.mask {
            let (mw, mh) = m.rect.size()?;
            if m.data.len() != mw * mh {
                return Err(PsdError::invalid("mask buffer size mismatch"));
            }
            let plane: Vec<u8> = if self.depth == 16 { m.data.iter().flat_map(|&v| (u16::from(v) * 257).to_be_bytes()).collect() } else { m.data.clone() };
            channels.push(self.channel(-2, &plane, mw, mh)?);
            let flags = if m.disabled { LayerMask::FLAG_DISABLED } else { 0 };
            mask = MaskData::Mask(LayerMask::new(m.rect, m.default_color, flags));
        }
        let mut flags = LayerFlags(0);
        flags.set_hidden(!spec.visible);
        let mut blocks = vec![TaggedBlock::unicode_name(&spec.name), TaggedBlock::layer_id(id)];
        if let Some(f) = spec.fill_opacity {
            blocks.push(TaggedBlock::fill_opacity(f));
        }
        blocks.extend(spec.extra_blocks.iter().cloned());
        Ok(LayerRecord {
            rect: Rect::from_xywh(spec.left, spec.top, spec.width, spec.height),
            channels,
            blend_mode: spec.blend_mode,
            opacity: spec.opacity,
            clipping: u8::from(spec.clipping),
            flags,
            filler: 0,
            mask,
            blending_ranges: BlendingRanges::full(cc),
            name: encode_legacy_name(&spec.name),
            blocks,
            extra_trailing: Vec::new(),
        })
    }

    /// Builds the file model.
    pub fn build(&self) -> Result<PsdFile> {
        if !self.open_groups.is_empty() {
            return Err(PsdError::invalid("unclosed group"));
        }
        if !matches!(self.depth, 8 | 16) {
            return Err(PsdError::Unsupported(format!("builder depth {}", self.depth)));
        }
        let cc = self.color_channels()?;
        let mut layers = Vec::new();
        for (id, e) in (1u32..).zip(self.entries.iter()) {
            match e {
                Entry::Layer(spec) => layers.push(self.layer_record(spec, cc, id)?),
                Entry::GroupStart => layers.push(LayerRecord {
                    channels: self.empty_channels(cc)?,
                    blending_ranges: BlendingRanges::full(cc),
                    name: b"</Layer group>".to_vec(),
                    blocks: vec![
                        TaggedBlock::unicode_name("</Layer group>"),
                        TaggedBlock::layer_id(id),
                        TaggedBlock::section_divider(SectionType::BoundingDivider, None, None),
                    ],
                    ..Default::default()
                }),
                Entry::GroupEnd(g) => {
                    let kind = if g.open { SectionType::OpenFolder } else { SectionType::ClosedFolder };
                    let mut flags = LayerFlags(0);
                    flags.set_hidden(!g.visible);
                    layers.push(LayerRecord {
                        channels: self.empty_channels(cc)?,
                        blend_mode: g.blend_mode,
                        opacity: g.opacity,
                        flags,
                        blending_ranges: BlendingRanges::full(cc),
                        name: encode_legacy_name(&g.name),
                        blocks: vec![
                            TaggedBlock::unicode_name(&g.name),
                            TaggedBlock::layer_id(id),
                            TaggedBlock::section_divider(kind, Some(g.blend_mode), None),
                        ],
                        ..Default::default()
                    });
                }
            }
        }

        let (w, h) = (self.width as usize, self.height as usize);
        let n = w * h;
        let (real, planes, with_alpha) = match &self.composite {
            Some(p) => {
                self.check_pixels(p, n)?;
                let with_alpha = !p.alpha_is_opaque(n);
                let mut planes = Vec::new();
                for c in 0..cc + usize::from(with_alpha) {
                    planes.extend(if with_alpha { p.composite_plane(c, n) } else { p.plane(c, n) });
                }
                (true, planes, with_alpha)
            }
            None => {
                let bps = usize::from(self.depth / 8);
                // White: 0xff in RGB/gray; no ink (stored inverted as 0xff) in CMYK.
                (false, vec![0xff; n * bps * cc], false)
            }
        };
        let channels = (cc + usize::from(with_alpha)) as u16;
        let header = Header::new(self.version, self.width, self.height, channels, self.depth, self.color_mode);
        header.validate()?;
        let image_data = ImageData::encode(self.compression, &planes, &header)?;
        let mut resources = self.resources.clone();
        resources.push(version_info_resource(real));

        let has_layers = !layers.is_empty();
        let layer_info = has_layers.then_some(LayerInfo { merged_alpha: with_alpha, layers, padding: None });
        let layer_info_placement = if has_layers && self.depth == 16 {
            LayerInfoPlacement::GlobalBlock { index: 0, signature: *b"8BIM", key: *b"Lr16", padding: None }
        } else {
            LayerInfoPlacement::Section
        };
        Ok(PsdFile {
            header,
            color_mode_data: Vec::new(),
            resources,
            layer_info,
            layer_info_placement,
            global_layer_mask: has_layers.then(GlobalLayerMask::default),
            global_blocks: Vec::new(),
            layer_mask_trailing: Vec::new(),
            image_data,
            write_as_psb: false,
        })
    }

    /// Builds and serializes.
    pub fn to_bytes(&self) -> Result<Vec<u8>> {
        self.build()?.to_bytes()
    }
}

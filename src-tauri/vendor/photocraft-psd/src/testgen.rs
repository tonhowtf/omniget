//! Deterministic synthetic PSD/PSB generator for tests and fuzz seeds.
//!
//! Covers every depth (1, 8, 16, 32), color mode (Bitmap, Grayscale,
//! Indexed, RGB, CMYK, Lab, Multichannel, Duotone), every compression,
//! layers with masks (incl. real masks and parameters), nested groups, all
//! blend modes, Unicode names, empty (0×0) layers, large-ish bounds,
//! negative offsets, many tagged blocks, descriptors and image resources.

use crate::blend::BlendMode;
use crate::compression::Compression;
use crate::descriptor::{Descriptor, Id, UnicodeString, Value, VersionedDescriptor};
use crate::file::{GlobalLayerMask, LayerInfoPlacement, PsdFile};
use crate::header::{ColorMode, Header, Version, row_bytes};
use crate::image_data::ImageData;
use crate::layer::{BlendingRanges, ChannelData, LayerFlags, LayerInfo, LayerMask, LayerRecord, MaskData, MaskParameters, RealMask, Rect};
use crate::resources::{ImageResource, ResolutionInfo, ids, version_info_resource};
use crate::tagged::{SectionType, TaggedBlock};

/// A named generated file.
#[derive(Debug, Clone)]
pub struct Case {
    /// Descriptive name.
    pub name: String,
    /// The model.
    pub file: PsdFile,
}

/// Deterministic pattern of planar samples for a `w × h` plane.
pub fn pattern_plane(w: usize, h: usize, depth: u16, seed: u32) -> Vec<u8> {
    let mut out = Vec::with_capacity(row_bytes(w, depth) * h);
    let s = seed as usize;
    for y in 0..h {
        match depth {
            1 => {
                for bx in 0..row_bytes(w, 1) {
                    out.push(((bx * 37 + y * 11 + s) as u8) ^ 0x5a);
                }
            }
            8 => {
                for x in 0..w {
                    // Mix smooth gradients and runs so RLE sees both.
                    let v = if (x / 4 + y) % 3 == 0 { (s * 17) as u8 } else { (x * 3 + y * 7 + s) as u8 };
                    out.push(v);
                }
            }
            16 => {
                for x in 0..w {
                    let v = ((x * 997 + y * 131 + s * 7919) % 65536) as u16;
                    out.extend_from_slice(&v.to_be_bytes());
                }
            }
            _ => {
                for x in 0..w {
                    let v = ((x + y + s) % 17) as f32 / 16.0;
                    out.extend_from_slice(&v.to_be_bytes());
                }
            }
        }
    }
    out
}

/// Number of channels used for a color mode in generated files.
pub fn mode_channels(mode: ColorMode) -> u16 {
    match mode {
        ColorMode::Multichannel => 2,
        m => m.color_channels().unwrap_or(1),
    }
}

/// Valid depths for a color mode.
pub fn mode_depths(mode: ColorMode) -> &'static [u16] {
    match mode {
        ColorMode::Bitmap => &[1],
        ColorMode::Indexed | ColorMode::Duotone => &[8],
        ColorMode::Multichannel | ColorMode::Cmyk | ColorMode::Lab => &[8, 16],
        _ => &[8, 16, 32],
    }
}

/// All color modes covered.
pub const MODES: [ColorMode; 8] =
    [ColorMode::Bitmap, ColorMode::Grayscale, ColorMode::Indexed, ColorMode::Rgb, ColorMode::Cmyk, ColorMode::Lab, ColorMode::Multichannel, ColorMode::Duotone];

fn color_mode_data(mode: ColorMode) -> Vec<u8> {
    match mode {
        ColorMode::Indexed => (0..768u32).map(|i| (i * 7 % 256) as u8).collect(),
        // Duotone data is undocumented; any bytes are preserved.
        ColorMode::Duotone => (0..24u8).collect(),
        _ => Vec::new(),
    }
}

fn base_resources() -> Vec<ImageResource> {
    let mut named = ImageResource::new(2000, vec![1, 2, 3]);
    named.name = b"Path 1".to_vec();
    vec![
        ImageResource::new(ids::RESOLUTION_INFO, ResolutionInfo::from_dpi(72.0).to_bytes()),
        ImageResource::new(ids::GLOBAL_ANGLE, 30i32.to_be_bytes().to_vec()),
        ImageResource::new(ids::GLOBAL_ALTITUDE, 30i32.to_be_bytes().to_vec()),
        ImageResource::new(ids::ICC_PROFILE, vec![0x42; 13]),
        ImageResource::new(ids::XMP, b"<x:xmpmeta xmlns:x='adobe:ns:meta/'/>".to_vec()),
        named,
        version_info_resource(true),
    ]
}

/// A flattened file (no layer section) with a patterned merged image.
pub fn merged_only(version: Version, mode: ColorMode, depth: u16, compression: Compression, w: u32, h: u32) -> PsdFile {
    let channels = mode_channels(mode);
    let header = Header::new(version, w, h, channels, depth, mode);
    let mut planes = Vec::new();
    for c in 0..channels {
        planes.extend(pattern_plane(w as usize, h as usize, depth, u32::from(c)));
    }
    let image_data = ImageData::encode(compression, &planes, &header).unwrap_or(ImageData::new(compression, Vec::new()));
    PsdFile {
        header,
        color_mode_data: color_mode_data(mode),
        resources: base_resources(),
        layer_info: None,
        layer_info_placement: LayerInfoPlacement::Section,
        global_layer_mask: None,
        global_blocks: Vec::new(),
        layer_mask_trailing: Vec::new(),
        image_data,
        write_as_psb: false,
    }
}

/// Parameters for a generated raster layer.
struct LayerGen<'a> {
    header: &'a Header,
    compression: Compression,
    rect: Rect,
    name: &'a str,
    blend: BlendMode,
    mask: Option<LayerMask>,
    seed: u32,
}

fn raster_layer(g: LayerGen<'_>) -> LayerRecord {
    let (w, h) = g.rect.size().unwrap_or((0, 0));
    let depth = g.header.depth;
    let v = g.header.version;
    let cc = mode_channels(g.header.color_mode) as i16;
    let mut channels = Vec::new();
    for id in -1..cc {
        let plane = pattern_plane(w, h, depth, g.seed + (id + 1) as u32);
        let comp = if id == -1 && g.compression == Compression::Rle { Compression::Zip } else { g.compression };
        channels.push(ChannelData::encode(id, comp, &plane, w, h, depth, v).expect("encode"));
    }
    let mut mask_data = MaskData::None;
    if let Some(m) = g.mask {
        let (mw, mh) = m.rect.size().unwrap_or((0, 0));
        channels.push(ChannelData::encode(-2, g.compression, &pattern_plane(mw, mh, depth, g.seed + 99), mw, mh, depth, v).expect("encode"));
        if let Some(real) = m.real {
            let (rw, rh) = real.rect.size().unwrap_or((0, 0));
            channels.push(ChannelData::encode(-3, g.compression, &pattern_plane(rw, rh, depth, g.seed + 7), rw, rh, depth, v).expect("encode"));
        }
        mask_data = MaskData::Mask(m);
    }
    let mut flags = LayerFlags(0);
    flags.set_hidden(g.seed % 5 == 4);
    LayerRecord {
        rect: g.rect,
        channels,
        blend_mode: g.blend,
        opacity: (255 - (g.seed * 13) % 128) as u8,
        clipping: u8::from(g.seed % 7 == 3),
        flags,
        filler: 0,
        mask: mask_data,
        blending_ranges: BlendingRanges::full(cc as usize),
        name: crate::io::encode_legacy_name(g.name),
        blocks: vec![TaggedBlock::unicode_name(g.name), TaggedBlock::layer_id(g.seed + 1)],
        extra_trailing: Vec::new(),
    }
}

fn group_record(header: &Header, name: &str, kind: SectionType, blend: BlendMode, id: u32) -> LayerRecord {
    let cc = mode_channels(header.color_mode) as i16;
    let channels = (-1..cc).map(|c| ChannelData::new(c, Some(Compression::Raw), Vec::new())).collect();
    let lsct = if kind == SectionType::BoundingDivider {
        TaggedBlock::section_divider(kind, None, None)
    } else {
        TaggedBlock::section_divider(kind, Some(blend), Some(0))
    };
    LayerRecord {
        channels,
        blend_mode: blend,
        name: crate::io::encode_legacy_name(name),
        blending_ranges: BlendingRanges::full(cc as usize),
        blocks: vec![TaggedBlock::unicode_name(name), TaggedBlock::layer_id(id), lsct],
        ..Default::default()
    }
}

/// A sample versioned descriptor as used by `lfx2`/`SoCo`-style blocks.
pub fn sample_descriptor() -> VersionedDescriptor {
    VersionedDescriptor::new(
        Descriptor::new("null")
            .with("Scl ", Value::UnitFloat { unit: *b"#Prc", value: 100.0 })
            .with("masterFXSwitch", Value::Boolean(true))
            .with(
                "DrSh",
                Value::Descriptor(
                    Descriptor::new("DrSh")
                        .with("enab", Value::Boolean(true))
                        .with("Md  ", Value::Enumerated { type_id: Id::new("BlnM"), value: Id::new("Mltp") })
                        .with("Clr ", Value::Descriptor(Descriptor::new("RGBC").with("Rd  ", Value::Double(0.0))))
                        .with("lagl", Value::UnitFloat { unit: *b"#Ang", value: 120.0 })
                        .with("Nm  ", Value::Text(UnicodeString::new_nul("Shadow"))),
                ),
            )
            .with("list", Value::List(vec![Value::Integer(1), Value::LargeInteger(-2)])),
    )
}

fn extra_blocks(seed: u32) -> Vec<TaggedBlock> {
    let mut lfx2 = vec![0, 0, 0, 0];
    lfx2.extend(sample_descriptor().to_bytes());
    let mut odd = TaggedBlock::new(*b"zOdd", vec![1, 2, 3]);
    odd.padding = Some(vec![0, 0, 0]);
    let mut unpadded = TaggedBlock::new(*b"zNop", vec![9]);
    unpadded.padding = Some(vec![]);
    vec![
        TaggedBlock::name_source(*b"cont"),
        TaggedBlock::blend_clipped_as_group(true),
        TaggedBlock::blend_interior_elements(false),
        TaggedBlock::knockout(0),
        TaggedBlock::protection(seed % 8),
        TaggedBlock::sheet_color((seed % 8) as u16),
        TaggedBlock::fill_opacity(200),
        TaggedBlock::new(*b"shmd", vec![0, 0, 0, 0]),
        TaggedBlock::new(*b"lfx2", lfx2),
        TaggedBlock::new(*b"fxrp", vec![0; 16]),
        odd,
        unpadded,
    ]
}

/// A layered file exercising masks, nested groups, blend modes, names,
/// empty layers, negative offsets and many tagged blocks.
pub fn layered(version: Version, mode: ColorMode, depth: u16, compression: Compression) -> PsdFile {
    let (w, h) = (24u32, 16u32);
    let channels = mode_channels(mode) + 1;
    let header = Header::new(version, w, h, channels, depth, mode);
    let mut layers = Vec::new();
    let mut seed = 0u32;
    let mut next = |layers: &mut Vec<LayerRecord>, rect: Rect, name: &str, blend: BlendMode, mask: Option<LayerMask>| {
        seed += 1;
        let mut rec = raster_layer(LayerGen { header: &header, compression, rect, name, blend, mask, seed });
        if seed.is_multiple_of(3) {
            rec.blocks.extend(extra_blocks(seed));
        }
        layers.push(rec);
    };
    // Background.
    next(&mut layers, Rect::from_xywh(0, 0, w, h), "Background", BlendMode::Normal, None);
    // Negative offsets + simple mask.
    next(
        &mut layers,
        Rect::from_xywh(-5, -3, 10, 7),
        "Neg offset \u{1F600}",
        BlendMode::Multiply,
        Some(LayerMask::new(Rect::from_xywh(-2, -2, 6, 5), 0, LayerMask::FLAG_RELATIVE)),
    );
    // Empty layer.
    next(&mut layers, Rect::default(), "Empty", BlendMode::Screen, None);
    // Large-ish bounds beyond the canvas.
    next(&mut layers, Rect::from_xywh(-100, 5, 400, 2), "Wide", BlendMode::Overlay, None);
    // Nested groups: outer [ layer, inner [ masked layer (real mask + params) ] ].
    let id0 = 1000;
    layers.push(group_record(&header, "</Layer group>", SectionType::BoundingDivider, BlendMode::Normal, id0));
    next(&mut layers, Rect::from_xywh(2, 2, 5, 5), "In outer", BlendMode::Darken, None);
    layers.push(group_record(&header, "</Layer group>", SectionType::BoundingDivider, BlendMode::Normal, id0 + 1));
    let real_mask = LayerMask {
        rect: Rect::from_xywh(1, 1, 4, 3),
        default_color: 255,
        flags: LayerMask::FLAG_PARAMETERS,
        parameters: Some(MaskParameters { flags: 0b0011, user_density: Some(200), user_feather: Some(1.5), vector_density: None, vector_feather: None }),
        real: Some(RealMask { flags: 0, background: 255, rect: Rect::from_xywh(0, 0, 3, 3) }),
        trailing: Vec::new(),
        real_first: true,
    };
    next(&mut layers, Rect::from_xywh(3, 3, 6, 4), "Grüße", BlendMode::ColorDodge, Some(real_mask));
    layers.push(group_record(&header, "Inner", SectionType::ClosedFolder, BlendMode::Normal, id0 + 2));
    layers.push(group_record(&header, "Outer", SectionType::OpenFolder, BlendMode::PassThrough, id0 + 3));
    // One small layer per blend mode.
    for (i, m) in BlendMode::ALL.iter().enumerate() {
        let name = format!("blend {:?}", m);
        next(&mut layers, Rect::from_xywh(i as i32 % 20, i as i32 % 12, 3, 2), &name, *m, None);
    }
    next(&mut layers, Rect::from_xywh(0, 0, 2, 2), "unknown blend", BlendMode::Unknown(*b"zzzz"), None);

    let mut planes = Vec::new();
    for c in 0..channels {
        planes.extend(pattern_plane(w as usize, h as usize, depth, 50 + u32::from(c)));
    }
    let image_data = ImageData::encode(compression, &planes, &header).expect("encode merged");
    let layer_info = LayerInfo { merged_alpha: true, layers, padding: None };
    let (placement, blocks) = match depth {
        16 | 32 => (
            LayerInfoPlacement::GlobalBlock { index: 1, signature: *b"8BIM", key: if depth == 16 { *b"Lr16" } else { *b"Lr32" }, padding: None },
            vec![TaggedBlock::new(*b"Patt", vec![]), TaggedBlock::new(*b"Txt2", vec![0, 1, 2, 3])],
        ),
        _ => (LayerInfoPlacement::Section, vec![TaggedBlock::new(*b"Patt", vec![]), TaggedBlock::new(*b"FMsk", vec![0; 10])]),
    };
    PsdFile {
        header,
        color_mode_data: color_mode_data(mode),
        resources: base_resources(),
        layer_info: Some(layer_info),
        layer_info_placement: placement,
        global_layer_mask: Some(GlobalLayerMask { data: vec![0, 0, 0xff, 0xff, 0, 0, 0, 0, 0, 0, 0, 50, 128, 0] }),
        global_blocks: blocks,
        layer_mask_trailing: Vec::new(),
        image_data,
        write_as_psb: false,
    }
}

/// A tiny layered RGB file (useful for truncation sweeps).
pub fn small(version: Version, compression: Compression) -> PsdFile {
    let header = Header::new(version, 4, 3, 4, 8, ColorMode::Rgb);
    let mask = LayerMask::new(Rect::from_xywh(0, 0, 2, 2), 0, 0);
    let layers = vec![
        raster_layer(LayerGen {
            header: &header,
            compression,
            rect: Rect::from_xywh(0, 0, 4, 3),
            name: "a",
            blend: BlendMode::Normal,
            mask: Some(mask),
            seed: 1,
        }),
        group_record(&header, "</Layer group>", SectionType::BoundingDivider, BlendMode::Normal, 9),
        raster_layer(LayerGen { header: &header, compression, rect: Rect::from_xywh(-1, 1, 2, 2), name: "b", blend: BlendMode::Screen, mask: None, seed: 2 }),
        group_record(&header, "g", SectionType::OpenFolder, BlendMode::PassThrough, 10),
    ];
    let mut planes = Vec::new();
    for c in 0..4 {
        planes.extend(pattern_plane(4, 3, 8, c));
    }
    PsdFile {
        header: header.clone(),
        color_mode_data: Vec::new(),
        resources: vec![ImageResource::new(ids::RESOLUTION_INFO, ResolutionInfo::from_dpi(72.0).to_bytes())],
        layer_info: Some(LayerInfo { merged_alpha: true, layers, padding: None }),
        layer_info_placement: LayerInfoPlacement::Section,
        global_layer_mask: Some(GlobalLayerMask::default()),
        global_blocks: vec![TaggedBlock::new(*b"Patt", vec![1, 2])],
        layer_mask_trailing: Vec::new(),
        image_data: ImageData::encode(compression, &planes, &header).expect("encode"),
        write_as_psb: false,
    }
}

/// Every generated case.
pub fn all_cases() -> Vec<Case> {
    let mut v = Vec::new();
    for version in [Version::Psd, Version::Psb] {
        for mode in MODES {
            for &depth in mode_depths(mode) {
                for c in Compression::ALL {
                    v.push(Case { name: format!("merged {version:?} {mode:?} {depth}bit {c:?}"), file: merged_only(version, mode, depth, c, 13, 7) });
                }
            }
        }
        for mode in [ColorMode::Grayscale, ColorMode::Rgb, ColorMode::Cmyk, ColorMode::Lab] {
            for &depth in mode_depths(mode).iter().filter(|&&d| d >= 8) {
                for c in Compression::ALL {
                    v.push(Case { name: format!("layered {version:?} {mode:?} {depth}bit {c:?}"), file: layered(version, mode, depth, c) });
                }
            }
        }
        for c in Compression::ALL {
            v.push(Case { name: format!("small {version:?} {c:?}"), file: small(version, c) });
        }
        v.push(Case { name: format!("1x1 canvas {version:?}"), file: merged_only(version, ColorMode::Rgb, 8, Compression::Rle, 1, 1) });
    }
    v
}

use std::path::Path;

use ::image::{DynamicImage, RgbaImage};
use photocraft_psd::{BlendMode, ColorMode, Layer, LayerNode, LayerRecord, PsdFile, SectionType};

const UNSUPPORTED_BLOCKS: &[&[u8; 4]] = &[
    b"lfx2", b"lfxs", b"lmfx", b"levl", b"curv", b"brit", b"blnc", b"hue2", b"hue ", b"selc",
    b"mixr", b"grdm", b"phfl", b"expA", b"vibA", b"thrs", b"nvrt", b"post", b"blwh", b"clrL",
    b"SoCo", b"GdFl", b"PtFl", b"vmsk", b"vsms",
];

const MAX_NAME_CHARS: usize = 120;

#[derive(Debug, Clone)]
pub struct PsdLayer {
    pub name: String,
    pub left: i32,
    pub top: i32,
    pub visible: bool,
    pub image: RgbaImage,
}

pub fn open(bytes: &[u8]) -> Result<PsdFile, String> {
    let file = PsdFile::from_bytes(bytes)
        .map_err(|e| format!("This Photoshop file could not be read: {e}."))?;
    super::image::check_pixels(file.header.width, file.header.height)?;
    if file.header.color_mode == ColorMode::Lab {
        return Err("This Photoshop file uses Lab color, which OmniGet can't read. In Photoshop, choose Image > Mode > RGB Color and save it again.".into());
    }
    Ok(file)
}

pub fn composite(bytes: &[u8]) -> Result<DynamicImage, String> {
    let file = open(bytes)?;
    if file.has_real_merged_data() == Some(false) && !file.layers().is_empty() {
        return flatten(&file).map(DynamicImage::ImageRgba8);
    }
    let merged = file
        .composite_rgba8()
        .map_err(|e| format!("The image inside this Photoshop file could not be read: {e}."))?;
    to_image(merged.width, merged.height, merged.data).map(DynamicImage::ImageRgba8)
}

pub fn layers(bytes: &[u8]) -> Result<Vec<PsdLayer>, String> {
    let file = open(bytes)?;
    let mut out = Vec::new();
    each_layer(&file, |layer| {
        out.push(layer);
        Ok(())
    })?;
    Ok(out)
}

fn each_layer(
    file: &PsdFile,
    mut visit: impl FnMut(PsdLayer) -> Result<(), String>,
) -> Result<(), String> {
    for layer in (0..file.layers().len()).rev().filter_map(|i| file.layer(i)) {
        let rec = layer.record;
        if rec.section_type() != SectionType::Other || rec.rect.is_empty() {
            continue;
        }
        visit(PsdLayer {
            name: layer.name(),
            left: rec.rect.left,
            top: rec.rect.top,
            visible: rec.is_visible(),
            image: pixels(&layer)?,
        })?;
    }
    Ok(())
}

fn layer_file_name(index: usize, digits: usize, name: &str) -> String {
    let short: String = name.trim().chars().take(MAX_NAME_CHARS).collect();
    let name = sanitize_filename::sanitize(short.trim());
    let name = if name.is_empty() { "Layer" } else { &name };
    format!("{index:0digits$} {name}.png")
}

pub fn export_layers(bytes: &[u8], dir: &Path) -> Result<usize, String> {
    let file = open(bytes)?;
    let total = (0..file.layers().len())
        .filter_map(|i| file.layer(i))
        .filter(|l| l.record.section_type() == SectionType::Other && !l.record.rect.is_empty())
        .count();
    if total == 0 {
        return Err("This Photoshop file has no layers to export.".into());
    }
    std::fs::create_dir_all(dir)
        .map_err(|e| format!("Could not create the layers folder: {e}."))?;
    let digits = total.to_string().len().max(2);
    let mut index = 0;
    each_layer(&file, |layer| {
        index += 1;
        let path = dir.join(layer_file_name(index, digits, &layer.name));
        layer
            .image
            .save_with_format(&path, ::image::ImageFormat::Png)
            .map_err(|e| format!("Could not save the layer “{}”: {e}.", layer.name))
    })?;
    Ok(total)
}

fn to_image(width: u32, height: u32, data: Vec<u8>) -> Result<RgbaImage, String> {
    RgbaImage::from_raw(width, height, data)
        .ok_or_else(|| "The pixels in this Photoshop file are damaged.".to_string())
}

fn pixels(layer: &Layer<'_>) -> Result<RgbaImage, String> {
    let name = layer.name();
    let rect = layer.record.rect;
    super::image::check_pixels(
        u32::try_from(rect.width()).unwrap_or(u32::MAX),
        u32::try_from(rect.height()).unwrap_or(u32::MAX),
    )?;
    let decoded = layer
        .rgba8()
        .map_err(|e| format!("The layer “{name}” could not be read: {e}."))?;
    let mut img = to_image(decoded.width, decoded.height, decoded.data)?;
    if layer.record.layer_mask().is_some_and(|m| m.disabled()) {
        return Ok(img);
    }
    if let Some(Ok(mask)) = layer.user_mask() {
        for (x, y, px) in img.enumerate_pixels_mut() {
            let mx = i64::from(decoded.left) + i64::from(x) - i64::from(mask.left);
            let my = i64::from(decoded.top) + i64::from(y) - i64::from(mask.top);
            let inside = (0..i64::from(mask.width)).contains(&mx)
                && (0..i64::from(mask.height)).contains(&my);
            let m = if inside {
                mask.data[(my as usize) * mask.width as usize + mx as usize]
            } else {
                mask.default_value
            };
            px[3] = ((u16::from(px[3]) * u16::from(m) + 127) / 255) as u8;
        }
    }
    Ok(img)
}

fn flatten(file: &PsdFile) -> Result<RgbaImage, String> {
    let mut canvas = RgbaImage::new(file.header.width, file.header.height);
    draw(file, &file.layer_tree(), &mut canvas)?;
    Ok(canvas)
}

fn unsupported(rec: &LayerRecord) -> bool {
    rec.clipping != 0
        || UNSUPPORTED_BLOCKS
            .iter()
            .any(|key| rec.block(key).is_some())
}

fn needs_compatibility() -> String {
    "This Photoshop file was saved without “Maximize Compatibility” and uses blend modes, effects or adjustment layers that OmniGet can't combine. In Photoshop, turn on Maximize Compatibility (Settings > File Handling) and save again, or convert it to PNG layers.".into()
}

fn draw(file: &PsdFile, nodes: &[LayerNode], canvas: &mut RgbaImage) -> Result<(), String> {
    for node in nodes {
        let Some(layer) = file.layer(node.index()) else {
            continue;
        };
        let rec = layer.record;
        if !rec.is_visible() {
            continue;
        }
        match node {
            LayerNode::Group { children, .. } => {
                if !matches!(rec.blend_mode, BlendMode::PassThrough | BlendMode::Normal)
                    || unsupported(rec)
                {
                    return Err(needs_compatibility());
                }
                let mut group = RgbaImage::new(canvas.width(), canvas.height());
                draw(file, children, &mut group)?;
                over(canvas, &group, 0, 0, rec.opacity);
            }
            LayerNode::Layer { .. } => {
                if rec.blend_mode != BlendMode::Normal || unsupported(rec) {
                    return Err(needs_compatibility());
                }
                if rec.rect.is_empty() {
                    continue;
                }
                let opacity = (u16::from(rec.opacity) * u16::from(rec.fill_opacity()) + 127) / 255;
                over(
                    canvas,
                    &pixels(&layer)?,
                    rec.rect.left,
                    rec.rect.top,
                    opacity as u8,
                );
            }
        }
    }
    Ok(())
}

fn over(dst: &mut RgbaImage, src: &RgbaImage, left: i32, top: i32, opacity: u8) {
    for (x, y, s) in src.enumerate_pixels() {
        let (dx, dy) = (
            i64::from(left) + i64::from(x),
            i64::from(top) + i64::from(y),
        );
        if dx < 0 || dy < 0 || dx >= i64::from(dst.width()) || dy >= i64::from(dst.height()) {
            continue;
        }
        let d = dst.get_pixel_mut(dx as u32, dy as u32);
        let sa = f32::from(s[3]) / 255.0 * f32::from(opacity) / 255.0;
        if sa <= 0.0 {
            continue;
        }
        let da = f32::from(d[3]) / 255.0;
        let oa = sa + da * (1.0 - sa);
        for c in 0..3 {
            let v = (f32::from(s[c]) * sa + f32::from(d[c]) * da * (1.0 - sa)) / oa;
            d[c] = v.round().clamp(0.0, 255.0) as u8;
        }
        d[3] = (oa * 255.0).round() as u8;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_psd::{GroupSpec, LayerSpec, PixelData, PsdBuilder};

    fn solid(w: u32, h: u32, rgba: [u8; 4]) -> PixelData {
        PixelData::Rgba8(rgba.repeat((w * h) as usize))
    }

    fn two_layers(with_composite: bool, top_mode: BlendMode) -> Vec<u8> {
        let mut b = PsdBuilder::new(8, 4);
        b.push_layer(LayerSpec::new(
            "Background",
            0,
            0,
            8,
            4,
            solid(8, 4, [0, 0, 255, 255]),
        ));
        let mut top = LayerSpec::new("Red/Top", 2, 1, 4, 2, solid(4, 2, [255, 0, 0, 255]));
        top.opacity = 128;
        top.blend_mode = top_mode;
        b.push_layer(top);
        if with_composite {
            b.composite(solid(8, 4, [10, 20, 30, 255]));
        }
        b.to_bytes().unwrap()
    }

    #[test]
    fn reads_the_saved_composite() {
        let img = composite(&two_layers(true, BlendMode::Normal))
            .unwrap()
            .to_rgba8();
        assert_eq!(img.dimensions(), (8, 4));
        assert_eq!(img.get_pixel(0, 0).0, [10, 20, 30, 255]);
    }

    #[test]
    fn builds_the_composite_from_layers_when_missing() {
        let img = composite(&two_layers(false, BlendMode::Normal))
            .unwrap()
            .to_rgba8();
        assert_eq!(img.get_pixel(0, 0).0, [0, 0, 255, 255]);
        let mixed = img.get_pixel(3, 2).0;
        assert!(
            (126..=130).contains(&mixed[0]) && (125..=129).contains(&mixed[2]),
            "{mixed:?}"
        );
        assert_eq!(mixed[3], 255);
    }

    #[test]
    fn explains_files_it_cannot_combine() {
        let err = composite(&two_layers(false, BlendMode::Multiply)).unwrap_err();
        assert!(err.contains("Maximize Compatibility"), "{err}");
    }

    #[test]
    fn hidden_layers_and_groups_follow_visibility() {
        let mut b = PsdBuilder::new(4, 4);
        b.push_layer(LayerSpec::new(
            "Base",
            0,
            0,
            4,
            4,
            solid(4, 4, [0, 255, 0, 255]),
        ));
        let mut group = GroupSpec::new("Hidden group");
        group.visible = false;
        b.begin_group(group);
        b.push_layer(LayerSpec::new(
            "Inside",
            0,
            0,
            4,
            4,
            solid(4, 4, [255, 0, 0, 255]),
        ));
        b.end_group().unwrap();
        let mut hidden = LayerSpec::new("Hidden", 0, 0, 4, 4, solid(4, 4, [0, 0, 255, 255]));
        hidden.visible = false;
        b.push_layer(hidden);
        let bytes = b.to_bytes().unwrap();
        let img = composite(&bytes).unwrap().to_rgba8();
        assert_eq!(img.get_pixel(1, 1).0, [0, 255, 0, 255]);
        let all = layers(&bytes).unwrap();
        let names: Vec<&str> = all.iter().map(|l| l.name.as_str()).collect();
        assert_eq!(names, ["Hidden", "Inside", "Base"]);
        assert!(!all[0].visible && all[1].visible);
    }

    #[test]
    fn exports_each_layer_as_png() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("art layers");
        let count = export_layers(&two_layers(true, BlendMode::Normal), &out).unwrap();
        assert_eq!(count, 2);
        let mut names: Vec<String> = std::fs::read_dir(&out)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        assert_eq!(names, ["01 RedTop.png", "02 Background.png"]);
        let top = ::image::open(out.join("01 RedTop.png")).unwrap().to_rgba8();
        assert_eq!(top.dimensions(), (4, 2));
        assert_eq!(top.get_pixel(0, 0).0, [255, 0, 0, 255]);
    }

    #[test]
    fn layer_file_names_stay_short_and_safe() {
        assert_eq!(layer_file_name(3, 2, "  "), "03 Layer.png");
        assert_eq!(layer_file_name(7, 3, "a/b:c"), "007 abc.png");
        let long = layer_file_name(1, 2, &"é".repeat(400));
        assert_eq!(long, format!("01 {}.png", "é".repeat(MAX_NAME_CHARS)));
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(&long), b"x").unwrap();
    }

    #[test]
    fn rejects_damaged_files() {
        let err = composite(b"8BPS this is not really a psd").unwrap_err();
        assert!(err.contains("Photoshop"), "{err}");
    }
}

// Ported from VTracer 0.6.5 (https://github.com/visioncortex/vtracer), MIT OR Apache-2.0,
// Copyright (c) 2023 Tsang Hao Fung.

use std::fmt::Write;

use ::image::imageops::FilterType;
use ::image::{DynamicImage, GenericImageView, RgbaImage};
use visioncortex::color_clusters::{KeyingAction, Runner, RunnerConfig, HIERARCHICAL_MAX};
use visioncortex::{BinaryImage, Color, ColorImage, CompoundPath, PathSimplifyMode, PointF64};

const MAX_TRACE_PIXELS: u32 = 4_000_000;
const KEY_CANDIDATES: [(u8, u8, u8); 8] = [
    (255, 0, 255),
    (0, 255, 0),
    (1, 254, 253),
    (254, 1, 2),
    (3, 2, 254),
    (253, 254, 1),
    (17, 239, 131),
    (131, 17, 239),
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Preset {
    BlackAndWhite,
    FewColors,
    LineArt,
}

impl Preset {
    pub fn parse(name: Option<&str>) -> Result<Self, String> {
        match name.unwrap_or("colors") {
            "bw" => Ok(Self::BlackAndWhite),
            "colors" => Ok(Self::FewColors),
            "lineart" => Ok(Self::LineArt),
            other => Err(format!(
                "Unknown vector style \"{other}\". Use bw, colors or lineart."
            )),
        }
    }
}

struct Settings {
    speckle: usize,
    corner: f64,
    length: f64,
    splice: f64,
    precision_loss: i32,
    layer_difference: i32,
    threshold: u8,
}

fn settings(preset: Preset) -> Settings {
    let base = Settings {
        speckle: 4,
        corner: 60f64.to_radians(),
        length: 4.0,
        splice: 45f64.to_radians(),
        precision_loss: 2,
        layer_difference: 16,
        threshold: 128,
    };
    match preset {
        Preset::BlackAndWhite => base,
        Preset::FewColors => Settings {
            speckle: 6,
            precision_loss: 3,
            layer_difference: 32,
            ..base
        },
        Preset::LineArt => Settings {
            speckle: 2,
            corner: 45f64.to_radians(),
            length: 3.0,
            threshold: 170,
            ..base
        },
    }
}

pub fn trace(img: &DynamicImage, preset: Preset) -> String {
    let (width, height) = img.dimensions();
    let img = shrink(img);
    let (traced_w, traced_h) = img.dimensions();
    let s = settings(preset);
    let mut svg = String::new();
    let _ = write!(
        svg,
        r#"<?xml version="1.0" encoding="UTF-8"?>
<svg xmlns="http://www.w3.org/2000/svg" version="1.1" width="{width}" height="{height}" viewBox="0 0 {traced_w} {traced_h}">
"#
    );
    match preset {
        Preset::BlackAndWhite | Preset::LineArt => binary(&img, &s, &mut svg),
        Preset::FewColors => color(img, &s, &mut svg),
    }
    svg.push_str("</svg>\n");
    svg
}

fn shrink(img: &DynamicImage) -> RgbaImage {
    let (w, h) = img.dimensions();
    let pixels = u64::from(w) * u64::from(h);
    if pixels <= u64::from(MAX_TRACE_PIXELS) {
        return img.to_rgba8();
    }
    let scale = (f64::from(MAX_TRACE_PIXELS) / pixels as f64).sqrt();
    let nw = ((f64::from(w) * scale) as u32).max(1);
    let nh = ((f64::from(h) * scale) as u32).max(1);
    img.resize_exact(nw, nh, FilterType::Triangle).to_rgba8()
}

fn push_path(svg: &mut String, path: &CompoundPath, color: Color) {
    let (d, offset) = path.to_svg_string(true, PointF64::default(), Some(2));
    if d.is_empty() {
        return;
    }
    let _ = writeln!(
        svg,
        r#"<path d="{d}" fill="{}" transform="translate({},{})"/>"#,
        color.to_hex_string(),
        offset.x,
        offset.y
    );
}

fn binary(img: &RgbaImage, s: &Settings, svg: &mut String) {
    let (width, height) = (img.width() as usize, img.height() as usize);
    let mut ink = BinaryImage::new_w_h(width, height);
    for (x, y, p) in img.enumerate_pixels() {
        let a = u32::from(p[3]);
        let luma =
            (2126 * u32::from(p[0]) + 7152 * u32::from(p[1]) + 722 * u32::from(p[2])) / 10000;
        let on_white = (luma * a + 255 * (255 - a)) / 255;
        ink.set_pixel(x as usize, y as usize, on_white < u32::from(s.threshold));
    }
    let clusters = ink.to_clusters(false);
    for i in 0..clusters.len() {
        let cluster = clusters.get_cluster(i);
        if cluster.size() < s.speckle * s.speckle {
            continue;
        }
        let path =
            cluster.to_compound_path(PathSimplifyMode::Spline, s.corner, s.length, 10, s.splice);
        push_path(svg, &path, Color::new(0, 0, 0));
    }
}

fn should_key(img: &RgbaImage) -> bool {
    let (w, h) = img.dimensions();
    let threshold = (w * 2) as f32 * 0.2;
    let mut transparent = 0f32;
    for y in [0, h / 4, h / 2, 3 * h / 4, h - 1] {
        for x in 0..w {
            if img.get_pixel(x, y)[3] == 0 {
                transparent += 1.0;
                if transparent >= threshold {
                    return true;
                }
            }
        }
    }
    false
}

fn unused_color(img: &RgbaImage) -> Option<Color> {
    KEY_CANDIDATES
        .iter()
        .find(|(r, g, b)| !img.pixels().any(|p| p[0] == *r && p[1] == *g && p[2] == *b))
        .map(|(r, g, b)| Color::new(*r, *g, *b))
}

fn color(mut img: RgbaImage, s: &Settings, svg: &mut String) {
    let key = if should_key(&img) {
        unused_color(&img)
    } else {
        None
    };
    if let Some(key) = key {
        for p in img.pixels_mut() {
            if p[3] == 0 {
                *p = ::image::Rgba([key.r, key.g, key.b, 255]);
            }
        }
    }
    let (width, height) = (img.width() as usize, img.height() as usize);
    let runner = Runner::new(
        RunnerConfig {
            diagonal: s.layer_difference == 0,
            hierarchical: HIERARCHICAL_MAX,
            batch_size: 25600,
            good_min_area: s.speckle * s.speckle,
            good_max_area: width * height,
            is_same_color_a: s.precision_loss,
            is_same_color_b: 1,
            deepen_diff: s.layer_difference,
            hollow_neighbours: 1,
            key_color: key.unwrap_or_default(),
            keying_action: KeyingAction::Discard,
        },
        ColorImage {
            pixels: img.into_raw(),
            width,
            height,
        },
    );
    let clusters = runner.run();
    let view = clusters.view();
    for &index in view.clusters_output.iter().rev() {
        let cluster = view.get_cluster(index);
        let path = cluster.to_compound_path(
            &view,
            false,
            PathSimplifyMode::Spline,
            s.corner,
            s.length,
            10,
            s.splice,
        );
        push_path(svg, &path, cluster.residue_color());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ::image::Rgba;

    fn logo() -> DynamicImage {
        let mut img = RgbaImage::from_pixel(120, 80, Rgba([255, 255, 255, 255]));
        for (x, y, p) in img.enumerate_pixels_mut() {
            let (dx, dy) = (x as i32 - 40, y as i32 - 40);
            if dx * dx + dy * dy < 25 * 25 {
                *p = Rgba([20, 60, 220, 255]);
            } else if (75..110).contains(&x) && (20..60).contains(&y) {
                *p = Rgba([240, 160, 10, 255]);
            }
        }
        DynamicImage::ImageRgba8(img)
    }

    fn rasterize(svg: &str) -> RgbaImage {
        super::super::svg::rasterize(svg.as_bytes())
            .unwrap()
            .to_rgba8()
    }

    #[test]
    fn colors_keeps_each_shape_color() {
        let svg = trace(&logo(), Preset::FewColors);
        assert!(svg.contains(r#"viewBox="0 0 120 80""#));
        let back = rasterize(&svg);
        let blue = back.get_pixel(40, 40);
        let orange = back.get_pixel(92, 40);
        assert!(blue[2] > 180 && blue[0] < 80, "{blue:?}");
        assert!(orange[0] > 200 && orange[2] < 80, "{orange:?}");
    }

    #[test]
    fn black_and_white_only_uses_black_on_transparent() {
        let svg = trace(&logo(), Preset::BlackAndWhite);
        assert!(svg.contains("#000000"));
        assert!(!svg.contains("#FFFFFF") && !svg.contains("#ffffff"));
        let back = rasterize(&svg);
        assert_eq!(back.get_pixel(40, 40)[3], 255);
        assert_eq!(back.get_pixel(5, 75)[3], 0);
    }

    #[test]
    fn transparent_background_stays_transparent() {
        let mut img = RgbaImage::new(60, 60);
        for (x, y, p) in img.enumerate_pixels_mut() {
            if (15..45).contains(&x) && (15..45).contains(&y) {
                *p = Rgba([200, 0, 0, 255]);
            }
        }
        let svg = trace(&DynamicImage::ImageRgba8(img), Preset::FewColors);
        let back = rasterize(&svg);
        assert_eq!(back.get_pixel(2, 2)[3], 0);
        assert!(back.get_pixel(30, 30)[0] > 150);
    }

    #[test]
    fn large_images_are_traced_at_a_reduced_size() {
        let big = DynamicImage::ImageRgba8(RgbaImage::from_pixel(4000, 2000, Rgba([0, 0, 0, 255])));
        let svg = trace(&big, Preset::LineArt);
        assert!(
            svg.contains(r#"width="4000" height="2000" viewBox="0 0 2828 1414""#),
            "{}",
            &svg[..200]
        );
    }

    #[test]
    fn unknown_preset_is_explained() {
        assert_eq!(Preset::parse(None).unwrap(), Preset::FewColors);
        assert!(Preset::parse(Some("sketch"))
            .unwrap_err()
            .contains("bw, colors or lineart"));
    }
}

use std::io::Cursor;
use std::path::Path;

use ::image::{imageops::FilterType, DynamicImage, ImageDecoder, ImageFormat, ImageReader, Limits};
use base64::Engine;
use serde::Serialize;

const MAX_FILE_BYTES: u64 = 128 * 1024 * 1024;
const SAMPLE_SIDE: u32 = 160;
const PREVIEW_SIDE: u32 = 360;
const MAX_ITERATIONS: usize = 40;

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Swatch {
    pub hex: String,
    pub l: f64,
    pub c: f64,
    pub h: f64,
    pub share: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct Extraction {
    pub colors: Vec<Swatch>,
    pub width: u32,
    pub height: u32,
    pub preview: String,
}

fn srgb_to_linear(v: f32) -> f32 {
    if v <= 0.04045 {
        v / 12.92
    } else {
        ((v + 0.055) / 1.055).powf(2.4)
    }
}

fn linear_to_srgb(v: f32) -> f32 {
    if v <= 0.003_130_8 {
        v * 12.92
    } else {
        1.055 * v.powf(1.0 / 2.4) - 0.055
    }
}

pub fn linear_to_oklab([r, g, b]: [f32; 3]) -> [f32; 3] {
    let l = (0.412_221_46 * r + 0.536_332_55 * g + 0.051_445_995 * b).cbrt();
    let m = (0.211_903_5 * r + 0.680_699_5 * g + 0.107_396_96 * b).cbrt();
    let s = (0.088_302_46 * r + 0.281_718_85 * g + 0.629_978_7 * b).cbrt();
    [
        0.210_454_26 * l + 0.793_617_8 * m - 0.004_072_047 * s,
        1.977_998_5 * l - 2.428_592_2 * m + 0.450_593_7 * s,
        0.025_904_037 * l + 0.782_771_77 * m - 0.808_675_77 * s,
    ]
}

pub fn oklab_to_linear([l0, a, b]: [f32; 3]) -> [f32; 3] {
    let l = (l0 + 0.396_337_78 * a + 0.215_803_76 * b).powi(3);
    let m = (l0 - 0.105_561_346 * a - 0.063_854_17 * b).powi(3);
    let s = (l0 - 0.089_484_18 * a - 1.291_485_5 * b).powi(3);
    [
        4.076_741_7 * l - 3.307_711_6 * m + 0.230_969_94 * s,
        -1.268_438 * l + 2.609_757_4 * m - 0.341_319_38 * s,
        -0.004_196_086_3 * l - 0.703_418_6 * m + 1.707_614_7 * s,
    ]
}

fn to_hex(lab: [f32; 3]) -> String {
    let rgb = oklab_to_linear(lab);
    let byte = |v: f32| (linear_to_srgb(v.clamp(0.0, 1.0)) * 255.0).round() as u8;
    format!(
        "#{:02x}{:02x}{:02x}",
        byte(rgb[0]),
        byte(rgb[1]),
        byte(rgb[2])
    )
}

fn dist2(a: &[f32; 3], b: &[f32; 3]) -> f32 {
    let d0 = a[0] - b[0];
    let d1 = a[1] - b[1];
    let d2 = a[2] - b[2];
    d0 * d0 + d1 * d1 + d2 * d2
}

struct Rng(u64);

impl Rng {
    fn next_f32(&mut self) -> f32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 >> 40) as f32 / (1u64 << 24) as f32
    }
}

fn nearest(point: &[f32; 3], centers: &[[f32; 3]]) -> (usize, f32) {
    let mut best = (0, f32::MAX);
    for (i, c) in centers.iter().enumerate() {
        let d = dist2(point, c);
        if d < best.1 {
            best = (i, d);
        }
    }
    best
}

fn seed_centers(points: &[[f32; 3]], k: usize, rng: &mut Rng) -> Vec<[f32; 3]> {
    let mut centers = vec![points[points.len() / 2]];
    let mut weights: Vec<f32> = points.iter().map(|p| dist2(p, &centers[0])).collect();
    while centers.len() < k {
        let total: f32 = weights.iter().sum();
        if total <= f32::EPSILON {
            break;
        }
        let mut target = rng.next_f32() * total;
        let mut pick = points.len() - 1;
        for (i, w) in weights.iter().enumerate() {
            target -= w;
            if target <= 0.0 {
                pick = i;
                break;
            }
        }
        let center = points[pick];
        centers.push(center);
        for (w, p) in weights.iter_mut().zip(points) {
            *w = w.min(dist2(p, &center));
        }
    }
    centers
}

pub fn kmeans(points: &[[f32; 3]], k: usize) -> Vec<([f32; 3], usize)> {
    if points.is_empty() || k == 0 {
        return Vec::new();
    }
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    let mut centers = seed_centers(points, k.min(points.len()), &mut rng);
    let mut labels = vec![usize::MAX; points.len()];
    for _ in 0..MAX_ITERATIONS {
        let mut changed = false;
        for (label, p) in labels.iter_mut().zip(points) {
            let (i, _) = nearest(p, &centers);
            if *label != i {
                *label = i;
                changed = true;
            }
        }
        let mut sums = vec![[0.0f64; 3]; centers.len()];
        let mut counts = vec![0usize; centers.len()];
        for (label, p) in labels.iter().zip(points) {
            counts[*label] += 1;
            for d in 0..3 {
                sums[*label][d] += p[d] as f64;
            }
        }
        for (i, center) in centers.iter_mut().enumerate() {
            if counts[i] > 0 {
                let n = counts[i] as f64;
                *center = [
                    (sums[i][0] / n) as f32,
                    (sums[i][1] / n) as f32,
                    (sums[i][2] / n) as f32,
                ];
            }
        }
        if !changed {
            break;
        }
    }
    let mut counts = vec![0usize; centers.len()];
    for label in &labels {
        counts[*label] += 1;
    }
    let mut out: Vec<([f32; 3], usize)> = centers
        .into_iter()
        .zip(counts)
        .filter(|(_, n)| *n > 0)
        .collect();
    out.sort_by_key(|b| std::cmp::Reverse(b.1));
    out
}

fn swatch(lab: [f32; 3], share: f64) -> Swatch {
    let c = (lab[1] * lab[1] + lab[2] * lab[2]).sqrt();
    let mut h = (lab[2].atan2(lab[1]) as f64).to_degrees();
    if h < 0.0 {
        h += 360.0;
    }
    let round = |v: f64, p: f64| (v * p).round() / p;
    Swatch {
        hex: to_hex(lab),
        l: round(lab[0] as f64, 10_000.0),
        c: round(c as f64, 10_000.0),
        h: if c < 1e-4 { 0.0 } else { round(h, 100.0) },
        share: round(share, 10_000.0),
    }
}

pub fn sample_points(img: &DynamicImage) -> Vec<[f32; 3]> {
    let small = img
        .resize(SAMPLE_SIDE, SAMPLE_SIDE, FilterType::Triangle)
        .to_rgba8();
    let mut lut = [0f32; 256];
    for (i, v) in lut.iter_mut().enumerate() {
        *v = srgb_to_linear(i as f32 / 255.0);
    }
    small
        .pixels()
        .filter(|p| p.0[3] >= 128)
        .map(|p| {
            linear_to_oklab([
                lut[p.0[0] as usize],
                lut[p.0[1] as usize],
                lut[p.0[2] as usize],
            ])
        })
        .collect()
}

pub fn extract_from_image(img: &DynamicImage, count: usize) -> Vec<Swatch> {
    let points = sample_points(img);
    let total = points.len().max(1) as f64;
    kmeans(&points, count.clamp(1, 12))
        .into_iter()
        .map(|(center, n)| swatch(center, n as f64 / total))
        .collect()
}

fn preview_of(img: &DynamicImage) -> String {
    let thumb = img.resize(PREVIEW_SIDE, PREVIEW_SIDE, FilterType::Triangle);
    let mut bytes = Vec::new();
    if thumb
        .to_rgba8()
        .write_to(&mut Cursor::new(&mut bytes), ImageFormat::Png)
        .is_err()
    {
        return String::new();
    }
    format!(
        "data:image/png;base64,{}",
        base64::engine::general_purpose::STANDARD.encode(bytes)
    )
}

const MAX_DECODE_BYTES: u64 = 1024 * 1024 * 1024;

pub fn decode(path: &Path) -> Result<DynamicImage, String> {
    let meta =
        std::fs::metadata(path).map_err(|e| format!("The image could not be opened: {e}."))?;
    if !meta.is_file() {
        return Err(
            "That is a folder, not an image. Choose a PNG, JPEG, WebP or GIF file.".to_string(),
        );
    }
    if meta.len() > MAX_FILE_BYTES {
        return Err("The image is larger than 128 MB. Choose a smaller file.".to_string());
    }
    let mut reader = ImageReader::open(path)
        .and_then(|r| r.with_guessed_format())
        .map_err(|e| format!("The image could not be opened: {e}."))?;
    let mut limits = Limits::default();
    limits.max_alloc = Some(MAX_DECODE_BYTES);
    reader.limits(limits);
    if reader.format().is_none() {
        return Err(
            "This file is not an image the app can read. Choose a PNG, JPEG, WebP or GIF file."
                .to_string(),
        );
    }
    let unreadable = |e: ::image::ImageError| format!("This image could not be read: {e}.");
    let mut decoder = reader.into_decoder().map_err(unreadable)?;
    if decoder.total_bytes() > MAX_DECODE_BYTES {
        return Err("The image is too large to open. Choose a smaller image.".to_string());
    }
    let orientation = decoder.orientation().ok();
    let mut img = DynamicImage::from_decoder(decoder).map_err(unreadable)?;
    if let Some(o) = orientation {
        img.apply_orientation(o);
    }
    Ok(img)
}

pub fn extract_from_path(path: &Path, count: usize) -> Result<Extraction, String> {
    let img = decode(path)?;
    let colors = extract_from_image(&img, count);
    if colors.is_empty() {
        return Err("The image is fully transparent, so it has no colors to pick.".to_string());
    }
    Ok(Extraction {
        colors,
        width: img.width(),
        height: img.height(),
        preview: preview_of(&img),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use ::image::{Rgba, RgbaImage};

    fn two_tone() -> DynamicImage {
        let mut img = RgbaImage::new(100, 100);
        for (x, _, p) in img.enumerate_pixels_mut() {
            *p = if x < 75 {
                Rgba([255, 0, 0, 255])
            } else {
                Rgba([0, 0, 255, 255])
            };
        }
        DynamicImage::ImageRgba8(img)
    }

    fn near_hex(hex: &str, rgb: [u8; 3]) -> bool {
        (0..3).all(|i| {
            let v = u8::from_str_radix(&hex[1 + i * 2..3 + i * 2], 16).unwrap();
            v.abs_diff(rgb[i]) <= 3
        })
    }

    #[test]
    fn oklab_round_trip() {
        for rgb in [[1.0, 0.0, 0.0], [0.2, 0.5, 0.9], [1.0, 1.0, 1.0]] {
            let back = oklab_to_linear(linear_to_oklab(rgb));
            for d in 0..3 {
                assert!((back[d] - rgb[d]).abs() < 1e-4);
            }
        }
        let red = linear_to_oklab([1.0, 0.0, 0.0]);
        assert!((red[0] - 0.62796).abs() < 1e-3);
    }

    #[test]
    fn finds_dominant_colors_with_shares() {
        let colors = extract_from_image(&two_tone(), 2);
        assert_eq!(colors.len(), 2);
        assert!(near_hex(&colors[0].hex, [255, 0, 0]));
        assert!(near_hex(&colors[1].hex, [0, 0, 255]));
        assert!((colors[0].share - 0.75).abs() < 0.03);
        let total: f64 = colors.iter().map(|c| c.share).sum();
        assert!((total - 1.0).abs() < 1e-3);
    }

    #[test]
    fn never_returns_more_clusters_than_colors() {
        let colors = extract_from_image(&two_tone(), 8);
        assert!(colors.len() <= 8);
        let shares: f64 = colors.iter().map(|c| c.share).sum();
        assert!((shares - 1.0).abs() < 1e-3);
    }

    #[test]
    fn skips_transparent_pixels() {
        let img = DynamicImage::ImageRgba8(RgbaImage::from_pixel(10, 10, Rgba([0, 0, 0, 0])));
        assert!(extract_from_image(&img, 4).is_empty());
    }

    #[test]
    fn applies_exif_orientation() {
        use ::image::codecs::png::PngEncoder;
        use ::image::{ExtendedColorType, ImageEncoder};
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("portrait.png");
        let img = RgbaImage::from_pixel(40, 20, Rgba([200, 30, 30, 255]));
        let exif = vec![
            b'M', b'M', 0, 42, 0, 0, 0, 8, 0, 1, 0x01, 0x12, 0, 3, 0, 0, 0, 1, 0, 6, 0, 0, 0, 0, 0,
            0,
        ];
        let file = std::fs::File::create(&path).unwrap();
        let mut enc = PngEncoder::new(file);
        enc.set_exif_metadata(exif).unwrap();
        enc.write_image(img.as_raw(), 40, 20, ExtendedColorType::Rgba8)
            .unwrap();
        let out = extract_from_path(&path, 4).unwrap();
        assert_eq!((out.width, out.height), (20, 40));
    }

    #[test]
    fn reads_a_file_and_rejects_non_images() {
        let dir = tempfile::tempdir().unwrap();
        let png = dir.path().join("a.png");
        two_tone().save(&png).unwrap();
        let out = extract_from_path(&png, 4).unwrap();
        assert_eq!(out.width, 100);
        assert!(out.preview.starts_with("data:image/png;base64,"));
        let txt = dir.path().join("b.png");
        std::fs::write(&txt, b"hello").unwrap();
        assert!(extract_from_path(&txt, 4).is_err());
        assert!(extract_from_path(dir.path(), 4).is_err());
    }

    #[test]
    fn is_deterministic() {
        let a = extract_from_image(&two_tone(), 5);
        let b = extract_from_image(&two_tone(), 5);
        assert_eq!(a, b);
    }
}

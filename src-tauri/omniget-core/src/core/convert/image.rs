use std::io::Cursor;

use ::image::codecs::jpeg::JpegEncoder;
use ::image::imageops::FilterType;
use ::image::{DynamicImage, ImageDecoder, ImageFormat, ImageReader, Rgba, RgbaImage};

use super::ConvertOptions;

pub const MAX_PIXELS: u64 = 200_000_000;

pub fn check_pixels(width: u32, height: u32) -> Result<(), String> {
    if width == 0 || height == 0 {
        return Err("This image has no pixels.".into());
    }
    let pixels = u64::from(width) * u64::from(height);
    if pixels > MAX_PIXELS {
        return Err(format!(
            "This image is too large to open ({width} × {height} pixels). OmniGet opens images up to {} megapixels.",
            MAX_PIXELS / 1_000_000
        ));
    }
    Ok(())
}

pub fn input_limit(id: &str) -> Option<u64> {
    const MB: u64 = 1024 * 1024;
    match id {
        "heic" | "jxl" | "svg" => Some(256 * MB),
        "raw" | "dxf" | "dwg" => Some(512 * MB),
        "psd" => Some(4096 * MB),
        _ => None,
    }
}

fn format_of(id: &str) -> Option<ImageFormat> {
    Some(match id {
        "png" => ImageFormat::Png,
        "jpeg" => ImageFormat::Jpeg,
        "webp" => ImageFormat::WebP,
        "gif" => ImageFormat::Gif,
        "bmp" => ImageFormat::Bmp,
        "ico" => ImageFormat::Ico,
        "tiff" => ImageFormat::Tiff,
        "tga" => ImageFormat::Tga,
        _ => return None,
    })
}

pub fn decode(bytes: &[u8], from: &str) -> Result<DynamicImage, String> {
    match from {
        "heic" => return super::heic::decode(bytes),
        "jxl" => return super::jxl::decode(bytes),
        "psd" => return super::psd::composite(bytes),
        "raw" => return super::raw::decode(bytes),
        "svg" => return super::svg::rasterize(bytes),
        _ => {}
    }
    let mut reader = ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|e| e.to_string())?;
    if reader.format().is_none() {
        if let Some(f) = format_of(from) {
            reader.set_format(f);
        }
    }
    let bad = |e: ::image::ImageError| format!("This image could not be read: {e}.");
    let mut decoder = reader.into_decoder().map_err(bad)?;
    let orientation = decoder
        .orientation()
        .unwrap_or(::image::metadata::Orientation::NoTransforms);
    let mut img = DynamicImage::from_decoder(decoder).map_err(bad)?;
    img.apply_orientation(orientation);
    Ok(img)
}

fn fit(img: DynamicImage, max_w: Option<u32>, max_h: Option<u32>) -> DynamicImage {
    let (w, h) = (img.width(), img.height());
    let mw = max_w.filter(|v| *v > 0).unwrap_or(u32::MAX);
    let mh = max_h.filter(|v| *v > 0).unwrap_or(u32::MAX);
    if w <= mw && h <= mh {
        return img;
    }
    img.resize(mw.min(w), mh.min(h), FilterType::Lanczos3)
}

fn flatten_on_white(img: &DynamicImage) -> ::image::RgbImage {
    if !img.color().has_alpha() {
        return img.to_rgb8();
    }
    let rgba = img.to_rgba8();
    let mut out = ::image::RgbImage::new(rgba.width(), rgba.height());
    for (x, y, Rgba([r, g, b, a])) in rgba.enumerate_pixels().map(|(x, y, p)| (x, y, *p)) {
        let a = a as u32;
        let mix = |c: u8| ((c as u32 * a + 255 * (255 - a)) / 255) as u8;
        out.put_pixel(x, y, ::image::Rgb([mix(r), mix(g), mix(b)]));
    }
    out
}

pub fn encode(img: DynamicImage, to: &str, opts: &ConvertOptions) -> Result<Vec<u8>, String> {
    let img = fit(img, opts.max_width, opts.max_height);
    let mut out = Cursor::new(Vec::new());
    let fail = |e: ::image::ImageError| format!("Could not write {}: {e}.", to.to_uppercase());
    match to {
        "jpeg" => {
            let q = opts.quality.unwrap_or(88).clamp(1, 100);
            let rgb = flatten_on_white(&img);
            JpegEncoder::new_with_quality(&mut out, q)
                .encode_image(&rgb)
                .map_err(fail)?;
        }
        "ico" => {
            let img = fit(img, Some(256), Some(256));
            img.to_rgba8()
                .write_to(&mut out, ImageFormat::Ico)
                .map_err(fail)?;
        }
        "svg" => {
            let preset = super::vector::Preset::parse(opts.vector_preset.as_deref())?;
            return Ok(super::vector::trace(&img, preset).into_bytes());
        }
        "bmp" | "tga" | "tiff" | "png" | "webp" | "gif" => {
            let format = format_of(to).ok_or("unsupported format")?;
            let rgba: RgbaImage = img.to_rgba8();
            rgba.write_to(&mut out, format).map_err(fail)?;
        }
        other => return Err(format!("OmniGet can't write {other} images by itself.")),
    }
    Ok(out.into_inner())
}

pub fn convert_bytes(
    bytes: &[u8],
    from: &str,
    to: &str,
    opts: &ConvertOptions,
) -> Result<Vec<u8>, String> {
    encode(decode(bytes, from)?, to, opts)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_png() -> Vec<u8> {
        let mut img = RgbaImage::new(300, 200);
        for (x, y, p) in img.enumerate_pixels_mut() {
            *p = Rgba([
                (x % 256) as u8,
                (y % 256) as u8,
                128,
                if x < 10 { 0 } else { 255 },
            ]);
        }
        let mut out = Cursor::new(Vec::new());
        img.write_to(&mut out, ImageFormat::Png).unwrap();
        out.into_inner()
    }

    #[test]
    fn png_to_many_formats() {
        let png = sample_png();
        let o = ConvertOptions::default();
        for (to, magic) in [
            ("jpeg", &[0xFFu8, 0xD8][..]),
            ("webp", b"RIFF"),
            ("bmp", b"BM"),
            ("gif", b"GIF8"),
            ("tiff", b"II*\0"),
        ] {
            let out = convert_bytes(&png, "png", to, &o).unwrap();
            assert!(out.starts_with(magic), "{to}");
            let back = decode(&out, to).unwrap();
            assert_eq!((back.width(), back.height()), (300, 200), "{to}");
        }
        let tga = convert_bytes(&png, "png", "tga", &o).unwrap();
        assert_eq!(decode(&tga, "tga").unwrap().width(), 300);
    }

    #[test]
    fn ico_fits_256_and_resize_keeps_ratio() {
        let png = sample_png();
        let ico = convert_bytes(&png, "png", "ico", &ConvertOptions::default()).unwrap();
        let back = decode(&ico, "ico").unwrap();
        assert!(back.width() <= 256 && back.height() <= 256);
        let o = ConvertOptions {
            max_width: Some(150),
            ..Default::default()
        };
        let small = decode(&convert_bytes(&png, "png", "png", &o).unwrap(), "png").unwrap();
        assert_eq!((small.width(), small.height()), (150, 100));
    }

    #[test]
    fn jpeg_exif_orientation_is_applied() {
        let img = RgbaImage::from_pixel(30, 20, Rgba([200, 10, 10, 255]));
        let jpg = encode(
            DynamicImage::ImageRgba8(img),
            "jpeg",
            &ConvertOptions::default(),
        )
        .unwrap();
        let mut exif: Vec<u8> = b"Exif\0\0II*\0".to_vec();
        exif.extend([
            8, 0, 0, 0, 1, 0, 0x12, 0x01, 3, 0, 1, 0, 0, 0, 6, 0, 0, 0, 0, 0, 0, 0,
        ]);
        let len = (exif.len() + 2) as u16;
        let mut tagged = jpg[..2].to_vec();
        tagged.extend([0xFF, 0xE1]);
        tagged.extend(len.to_be_bytes());
        tagged.extend(&exif);
        tagged.extend(&jpg[2..]);
        let out = convert_bytes(&tagged, "jpeg", "png", &ConvertOptions::default()).unwrap();
        let back = decode(&out, "png").unwrap();
        assert_eq!((back.width(), back.height()), (20, 30));
    }

    #[test]
    fn alpha_to_jpeg_is_white() {
        let png = sample_png();
        let jpg = convert_bytes(&png, "png", "jpeg", &ConvertOptions::default()).unwrap();
        let px = decode(&jpg, "jpeg").unwrap().to_rgb8().get_pixel(2, 100).0;
        assert!(px.iter().all(|c| *c > 230), "{px:?}");
    }
}

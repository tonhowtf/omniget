use std::io::Cursor;
use std::path::Path;

use ::image::codecs::jpeg::JpegEncoder;
use ::image::imageops::FilterType;
use ::image::metadata::Orientation;
use ::image::{DynamicImage, GrayImage, RgbImage};
use rawler::decoders::RawDecodeParams;
use rawler::rawsource::RawSource;

use crate::core::convert::image as convert_image;

pub const THUMB_SIZE: u32 = 360;
const THUMB_QUALITY: u8 = 80;
const RAW_PREVIEW_MIN: u32 = 640;

pub fn read_bytes(path: &Path, format: &str) -> Result<Vec<u8>, String> {
    let size = std::fs::metadata(path)
        .map_err(|e| format!("This photo could not be opened: {e}."))?
        .len();
    if let Some(limit) = convert_image::input_limit(format) {
        if size > limit {
            return Err("This photo is too large to open.".to_string());
        }
    }
    std::fs::read(path).map_err(|e| format!("This photo could not be opened: {e}."))
}

pub fn decode_full(path: &Path, format: &str) -> Result<DynamicImage, String> {
    let bytes = read_bytes(path, format)?;
    let img = convert_image::decode(&bytes, format)?;
    convert_image::check_pixels(img.width(), img.height())?;
    Ok(img)
}

pub fn decode_quick(
    path: &Path,
    format: &str,
    orientation: Option<u8>,
) -> Result<DynamicImage, String> {
    if format == "raw" {
        if let Some(img) = raw_preview(path, orientation) {
            return Ok(img);
        }
    }
    decode_full(path, format)
}

fn raw_preview(path: &Path, orientation: Option<u8>) -> Option<DynamicImage> {
    let source = RawSource::new(path).ok()?;
    let decoder = rawler::get_decoder(&source).ok()?;
    let params = RawDecodeParams::default();
    let img = decoder
        .preview_image(&source, &params)
        .ok()
        .flatten()
        .filter(|i| i.width().max(i.height()) >= RAW_PREVIEW_MIN)
        .or_else(|| decoder.full_image(&source, &params).ok().flatten())?;
    let mut img = img;
    if let Some(o) = orientation.and_then(Orientation::from_exif) {
        img.apply_orientation(o);
    }
    Some(img)
}

pub fn flatten(img: &DynamicImage) -> RgbImage {
    if !img.color().has_alpha() {
        return img.to_rgb8();
    }
    let rgba = img.to_rgba8();
    let mut out = RgbImage::new(rgba.width(), rgba.height());
    for (src, dst) in rgba.pixels().zip(out.pixels_mut()) {
        let a = u32::from(src[3]);
        let mix = |c: u8| ((u32::from(c) * a + 255 * (255 - a)) / 255) as u8;
        *dst = ::image::Rgb([mix(src[0]), mix(src[1]), mix(src[2])]);
    }
    out
}

pub fn jpeg(img: &RgbImage, quality: u8) -> Result<Vec<u8>, String> {
    let mut out = Cursor::new(Vec::new());
    JpegEncoder::new_with_quality(&mut out, quality)
        .encode_image(img)
        .map_err(|e| format!("Could not save the picture: {e}."))?;
    Ok(out.into_inner())
}

pub fn downscale(img: &DynamicImage, max: u32) -> DynamicImage {
    if img.width() <= max && img.height() <= max {
        return img.clone();
    }
    img.resize(max, max, FilterType::Triangle)
}

pub struct Thumb {
    pub jpeg: Vec<u8>,
    pub dhash: u64,
}

pub fn make(img: &DynamicImage) -> Result<Thumb, String> {
    let small = if img.width() > THUMB_SIZE * 4 || img.height() > THUMB_SIZE * 4 {
        img.thumbnail(THUMB_SIZE * 2, THUMB_SIZE * 2).resize(
            THUMB_SIZE,
            THUMB_SIZE,
            FilterType::Triangle,
        )
    } else {
        downscale(img, THUMB_SIZE)
    };
    let rgb = flatten(&small);
    Ok(Thumb {
        dhash: dhash(&DynamicImage::ImageRgb8(rgb.clone()).to_luma8()),
        jpeg: jpeg(&rgb, THUMB_QUALITY)?,
    })
}

pub fn dhash(gray: &GrayImage) -> u64 {
    let small = ::image::imageops::resize(gray, 9, 8, FilterType::Triangle);
    let mut hash = 0u64;
    for y in 0..8 {
        for x in 0..8 {
            hash <<= 1;
            if small.get_pixel(x, y)[0] < small.get_pixel(x + 1, y)[0] {
                hash |= 1;
            }
        }
    }
    hash
}

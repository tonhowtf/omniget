use std::io::Cursor;

use ::image::{DynamicImage, ImageBuffer, Luma, LumaA, Rgb, Rgba};
use jxl_oxide::{AllocTracker, EnumColourEncoding, JxlImage, RenderingIntent};

const MEMORY_LIMIT: usize = 4 << 30;

pub fn decode(bytes: &[u8]) -> Result<DynamicImage, String> {
    let bad = |e: Box<dyn std::error::Error + Send + Sync>| {
        format!("This JPEG XL image could not be read: {e}.")
    };
    let mut image = JxlImage::builder()
        .alloc_tracker(AllocTracker::with_limit(MEMORY_LIMIT))
        .read(Cursor::new(bytes))
        .map_err(bad)?;
    super::image::check_pixels(image.width(), image.height())?;
    if !image.pixel_format().is_grayscale() {
        image.request_color_encoding(EnumColourEncoding::srgb(RenderingIntent::Relative));
    }
    let render = image.render_frame(0).map_err(bad)?;
    let mut stream = render.stream();
    let (width, height, channels) = (stream.width(), stream.height(), stream.channels());
    let mut buf = vec![0u8; width as usize * height as usize * channels as usize];
    stream.write_to_buffer(&mut buf);
    let broken = || "This JPEG XL image could not be read.".to_string();
    Ok(match channels {
        1 => DynamicImage::ImageLuma8(
            ImageBuffer::<Luma<u8>, _>::from_raw(width, height, buf).ok_or_else(broken)?,
        ),
        2 => DynamicImage::ImageLumaA8(
            ImageBuffer::<LumaA<u8>, _>::from_raw(width, height, buf).ok_or_else(broken)?,
        ),
        3 => DynamicImage::ImageRgb8(
            ImageBuffer::<Rgb<u8>, _>::from_raw(width, height, buf).ok_or_else(broken)?,
        ),
        4 => DynamicImage::ImageRgba8(
            ImageBuffer::<Rgba<u8>, _>::from_raw(width, height, buf).ok_or_else(broken)?,
        ),
        _ => return Err(broken()),
    })
}

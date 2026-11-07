use ::image::DynamicImage;
#[cfg(feature = "heic")]
use ::image::RgbaImage;

#[cfg(feature = "heic")]
pub fn decode(bytes: &[u8]) -> Result<DynamicImage, String> {
    let probe = heic_rs::probe(bytes).map_err(unreadable)?;
    super::image::check_pixels(probe.width, probe.height)?;
    let options = heic_rs::DecodeOptions::default()
        .with_layout(heic_rs::PixelLayout::Rgba8)
        .with_max_pixels(Some(super::image::MAX_PIXELS));
    let img = heic_rs::decode(bytes, &options).map_err(unreadable)?;
    RgbaImage::from_raw(img.width, img.height, img.data)
        .map(DynamicImage::ImageRgba8)
        .ok_or_else(|| "This HEIC photo could not be read.".to_string())
}

#[cfg(feature = "heic")]
fn unreadable(e: heic_rs::Error) -> String {
    format!("This HEIC photo could not be read: {e}.")
}

#[cfg(not(feature = "heic"))]
pub fn decode(_bytes: &[u8]) -> Result<DynamicImage, String> {
    Err("This copy of OmniGet was built without HEIC support.".to_string())
}

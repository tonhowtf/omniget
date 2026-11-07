//! Integration with the [image] crate.

use std::{
    ffi::OsStr,
    io::{BufRead, Seek},
};

use crate::Image;
use ::image::error::{DecodingError, ImageFormatHint};
use ::image::{ColorType, ExtendedColorType, ImageDecoder, ImageError, ImageResult};
use image::hooks::{decoding_hook_registered, register_format_detection_hook};

impl ImageDecoder for Image<'_> {
    fn dimensions(&self) -> (u32, u32) {
        (self.width(), self.height())
    }

    fn color_type(&self) -> ColorType {
        ColorType::L8
    }

    fn original_color_type(&self) -> ExtendedColorType {
        ExtendedColorType::L1
    }

    fn read_image(self, buf: &mut [u8]) -> ImageResult<()>
    where
        Self: Sized,
    {
        decode_into_buf(&self, buf)
    }

    fn read_image_boxed(self: Box<Self>, buf: &mut [u8]) -> ImageResult<()> {
        decode_into_buf(&self, buf)
    }
}

fn decode_into_buf(image: &Image<'_>, buf: &mut [u8]) -> ImageResult<()> {
    struct LumaDecoder<'a> {
        buf: &'a mut [u8],
        pos: usize,
    }

    impl crate::Decoder for LumaDecoder<'_> {
        fn push_pixel(&mut self, black: bool) {
            self.buf[self.pos] = if black { 0 } else { 255 };
            self.pos += 1;
        }

        fn push_pixel_chunk(&mut self, black: bool, chunk_count: u32) {
            let luma = if black { 0 } else { 255 };
            let count = chunk_count as usize * 8;
            self.buf[self.pos..self.pos + count].fill(luma);
            self.pos += count;
        }

        fn next_line(&mut self) {}
    }

    let mut decoder = LumaDecoder { buf, pos: 0 };
    image.decode(&mut decoder)?;

    Ok(())
}

/// JBIG2 decoder compatible with `image` decoding hook APIs that pass an `impl BufRead + Seek`.
#[doc(hidden)]
pub struct Jbig2Decoder {
    input: Vec<u8>,
    width: u32,
    height: u32,
}

impl Jbig2Decoder {
    /// Create a new decoder that decodes from the stream `r`.
    pub fn new<R: BufRead + Seek>(r: R) -> ImageResult<Self> {
        let mut input = Vec::new();
        let mut r = r;
        r.read_to_end(&mut input)?;

        let image = Image::new(&input)?;

        Ok(Self {
            width: image.width(),
            height: image.height(),
            input,
        })
    }
}

impl ImageDecoder for Jbig2Decoder {
    fn dimensions(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    fn color_type(&self) -> ColorType {
        ColorType::L8
    }

    fn original_color_type(&self) -> ExtendedColorType {
        ExtendedColorType::L1
    }

    fn read_image(self, buf: &mut [u8]) -> ImageResult<()>
    where
        Self: Sized,
    {
        let image = Image::new(&self.input)?;
        decode_into_buf(&image, buf)
    }

    fn read_image_boxed(self: Box<Self>, buf: &mut [u8]) -> ImageResult<()> {
        let image = Image::new(&self.input)?;
        decode_into_buf(&image, buf)
    }
}

impl From<crate::DecodeError> for DecodingError {
    fn from(value: crate::DecodeError) -> Self {
        let format = ImageFormatHint::Name("JBIG2".to_owned());
        Self::new(format, value)
    }
}

impl From<crate::DecodeError> for ImageError {
    fn from(value: crate::DecodeError) -> Self {
        Self::Decoding(value.into())
    }
}

const JBIG2_MAGIC: &[u8] = &[0x97, 0x4A, 0x42, 0x32, 0x0D, 0x0A, 0x1A, 0x0A];

/// Registers the decoder with the `image` crate so that non-format-specific calls such as
/// `ImageReader::open("image.jbig2")?.decode()?;` work with JBIG2 files.
///
/// Returns `true` on success, or `false` if the hook for JBIG2 is already registered.
pub fn register_decoding_hook() -> bool {
    if decoding_hook_registered(OsStr::new("jbig2")) {
        return false;
    }

    for extension in ["jbig2", "jb2"] {
        image::hooks::register_decoding_hook(
            extension.into(),
            Box::new(|r| Ok(Box::new(Jbig2Decoder::new(r)?))),
        );
        register_format_detection_hook(extension.into(), JBIG2_MAGIC, None);
    }

    true
}

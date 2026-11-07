//! Streams.

use crate::crypto::DecryptionTarget;
use crate::filter::Filter;
use crate::object;
use crate::object::Dict;
use crate::object::Name;
use crate::object::dict::keys::{DECODE_PARMS, DP, F, FILTER, LENGTH, NAME, TYPE};
use crate::object::{Array, ObjectIdentifier};
use crate::object::{Object, ObjectLike, ObjectRefLike};
use crate::reader::Reader;
use crate::reader::{Readable, ReaderContext, ReaderExt, Skippable};
use crate::trivia::is_white_space_character;
use crate::util::{OptionLog, find_needle};
use alloc::borrow::Cow;
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use core::fmt::{Debug, Formatter};
use smallvec::SmallVec;

struct FiltersAndParams<'a> {
    filters: SmallVec<[Filter; 2]>,
    params: SmallVec<[Dict<'a>; 2]>,
}

/// PdfCraft patch: how the decoded data of a stream can be produced (see `Stream::incremental`).
pub(crate) enum Incremental {
    /// The stream has no filter, so its data is its own bytes.
    Plain,
    /// The stream is Flate-compressed without a predictor, so it decodes a piece at a time.
    Flate,
    /// Anything else decodes whole.
    Whole,
}

/// A stream of arbitrary data.
#[derive(Clone)]
pub struct Stream<'a> {
    dict: Dict<'a>,
    data: &'a [u8],
}

impl PartialEq for Stream<'_> {
    fn eq(&self, other: &Self) -> bool {
        self.dict == other.dict && self.data == other.data
    }
}

/// Additional parameters for decoding images.
#[derive(Clone, PartialEq, Default)]
pub struct ImageDecodeParams {
    /// Whether the color space of the image is an indexed color space.
    pub is_indexed: bool,
    /// The bits per component of the image, if that information is available.
    pub bpc: Option<u8>,
    /// The components per channel of the image, if that information is available.
    pub num_components: Option<u8>,
    /// A target resolution for the image. Note that this is only a hint so that
    /// in case it's possible, a version of the image will be extracted that
    /// is as close as possible to the hinted dimension.
    pub target_dimension: Option<(u32, u32)>,
    /// The width of the image as indicated by the image dictionary.
    pub width: u32,
    /// The height of the image as indicated by the image dictionary.
    pub height: u32,
}

impl<'a> Stream<'a> {
    pub(crate) fn new(data: &'a [u8], dict: Dict<'a>) -> Self {
        Self { dict, data }
    }

    fn filters_and_params(&self) -> FiltersAndParams<'a> {
        let mut collected_filters = SmallVec::new();
        let mut collected_params = SmallVec::new();

        // PdfCraft patch (30): `/DecodeParms` in either form, whichever form `/Filter` has: one filter
        // in an array with a lone dictionary is an everyday spelling, and a lone dictionary is the
        // first filter's. Read as the chain asks, so a long array costs no more than a short one.
        let parms = self
            .dict
            .get::<Object<'_>>(DP)
            .or_else(|| self.dict.get::<Object<'_>>(DECODE_PARMS));
        let mut in_array = match &parms {
            Some(Object::Array(a)) => Some(a.iter::<Object<'_>>()),
            _ => None,
        };
        let mut lone = match parms {
            Some(Object::Dict(d)) => Some(d),
            _ => None,
        };
        let mut next_params = || {
            lone.take()
                .or_else(|| in_array.as_mut()?.next()?.into_dict())
                .unwrap_or_default()
        };

        if let Some(filter) = self
            .dict
            .get::<Name<'_>>(F)
            .or_else(|| self.dict.get::<Name<'_>>(FILTER))
            .and_then(Filter::from_name)
        {
            collected_filters.push(filter);
            collected_params.push(next_params());
        } else if let Some(filters) = self
            .dict
            .get::<Array<'_>>(F)
            .or_else(|| self.dict.get::<Array<'_>>(FILTER))
        {
            let filters = filters.iter::<Name<'_>>().map(Filter::from_name);

            for filter in filters {
                let params = next_params();

                if let Some(filter) = filter {
                    collected_filters.push(filter);
                    collected_params.push(params);
                }
            }
        }

        FiltersAndParams {
            filters: collected_filters,
            params: collected_params,
        }
    }

    /// PdfCraft patch (30): the crypt filter the stream names for itself: `/Crypt` in its chain, its
    /// name in that filter's parameters, or `/Identity` without one (ISO 32000-2 §7.4.10).
    fn crypt_filter(&self) -> Option<String> {
        let FiltersAndParams { filters, params } = self.filters_and_params();
        // A chain longer than cos decodes names none there, so it names none here either:
        // `pdfcraft_filters::MAX_CHAIN`, and `/Crypt`.
        if filters.len() > 9 {
            return None;
        }
        let i = filters.iter().position(|f| matches!(f, Filter::Crypt))?;
        let name = params.get(i).and_then(|p| p.get::<Name<'_>>(NAME));
        Some(name.map_or_else(|| "Identity".to_string(), |n| n.as_str().to_string()))
    }

    /// Return the raw, decrypted data of the stream.
    ///
    /// Stream filters will not be applied.
    pub fn raw_data(&self) -> Cow<'a, [u8]> {
        let ctx = self.dict.ctx();

        if ctx.xref().needs_decryption(ctx)
            && self
                .dict
                .get::<object::String<'_>>(TYPE)
                .map(|t| t.as_ref() != b"XRef")
                .unwrap_or(true)
        {
            let own = self.crypt_filter();
            let target = own
                .as_deref()
                .map_or(DecryptionTarget::Stream, DecryptionTarget::NamedStream);
            Cow::Owned(
                ctx.xref()
                    .decrypt(self.obj_id(), self.data, target)
                    // TODO: MAybe an error would be better?
                    .unwrap_or_default(),
            )
        } else {
            Cow::Borrowed(self.data)
        }
    }

    /// Return the raw, underlying dictionary of the stream.
    pub fn dict(&self) -> &Dict<'a> {
        &self.dict
    }

    /// Return the object identifier of the stream.
    pub fn obj_id(&self) -> ObjectIdentifier {
        self.dict.obj_id().unwrap_or_else(|| {
            // In theory shouldn't ever happen, but could theoretically be
            // triggered in a crafted PDF.
            ObjectIdentifier::new(0, 0)
        })
    }

    /// Return the filters that are applied to the stream.
    pub fn filters(&self) -> SmallVec<[Filter; 2]> {
        self.filters_and_params().filters
    }

    /// PdfCraft patch: how the decoded data of the stream can be produced a piece at a time.
    pub(crate) fn incremental(&self) -> Incremental {
        let filters_and_params = self.filters_and_params();

        match (
            filters_and_params.filters.as_slice(),
            filters_and_params.params.first(),
        ) {
            ([], _) => Incremental::Plain,
            ([Filter::FlateDecode], Some(params)) if crate::filter::flate_is_plain(params) => {
                Incremental::Flate
            }
            _ => Incremental::Whole,
        }
    }

    /// Return the decoded data of the stream.
    ///
    /// Note that the result of this method will not be cached, so calling it multiple
    /// times is expensive.
    pub fn decoded(&self) -> Result<Cow<'a, [u8]>, DecodeFailure> {
        self.decoded_image(&ImageDecodeParams::default())
            .map(|r| r.data)
    }

    /// Return the decoded data of the stream, and return image metadata
    /// if available.
    pub fn decoded_image(
        &self,
        image_params: &ImageDecodeParams,
    ) -> Result<FilterResult<'a>, DecodeFailure> {
        self.decode_within(image_params, usize::MAX)
    }

    /// PdfCraft patch: the decoded data of the stream, for callers that budget what a page
    /// holds. Flate, LZW and RunLength stop at `max` bytes (and at
    /// [`crate::filter::MAX_DECODED_STREAM`] in any case). A stream without filters is returned
    /// as it is; ASCIIHex and ASCII85 barely expand, and the image codecs stop at their own pixel
    /// limits.
    pub fn decoded_within(&self, max: usize) -> Result<Cow<'a, [u8]>, DecodeFailure> {
        self.decode_within(&ImageDecodeParams::default(), max)
            .map(|r| r.data)
    }

    fn decode_within(
        &self,
        image_params: &ImageDecodeParams,
        max: usize,
    ) -> Result<FilterResult<'a>, DecodeFailure> {
        self.decode_raw_within(self.raw_data(), image_params, max)
    }

    /// PdfCraft patch: decode caller-supplied raw/decrypted bytes with the same filters and limits.
    pub(crate) fn decoded_raw_within(
        &self,
        data: Cow<'a, [u8]>,
        max: usize,
    ) -> Result<Cow<'a, [u8]>, DecodeFailure> {
        self.decode_raw_within(data, &ImageDecodeParams::default(), max)
            .map(|r| r.data)
    }

    fn decode_raw_within(
        &self,
        data: Cow<'a, [u8]>,
        image_params: &ImageDecodeParams,
        max: usize,
    ) -> Result<FilterResult<'a>, DecodeFailure> {
        let filters_and_params = self.filters_and_params();
        let count = filters_and_params.filters.len();

        let mut current: Option<FilterResult<'a>> = None;

        for (i, (filter, params)) in filters_and_params
            .filters
            .iter()
            .zip(filters_and_params.params.iter())
            .enumerate()
        {
            // PdfCraft patch (30): `raw_data` decrypted the data, so there's nothing to undo, and a
            // chain can name this thousands of times.
            if matches!(filter, Filter::Crypt) {
                continue;
            }
            // PdfCraft patch: see `filter::decode_limit`.
            let limit = crate::filter::decode_limit(image_params, i + 1 == count).min(max);
            let new = filter.apply(
                current.as_ref().map(|c| c.data.as_ref()).unwrap_or(&data),
                params,
                image_params,
                limit,
            )?;
            current = Some(new);
        }

        Ok(current.unwrap_or(FilterResult {
            data,
            image_data: None,
        }))
    }
}

impl Debug for Stream<'_> {
    fn fmt(&self, f: &mut Formatter<'_>) -> core::fmt::Result {
        write!(f, "Stream (len: {:?})", self.data.len())
    }
}

impl Skippable for Stream<'_> {
    fn skip(_: &mut Reader<'_>, _: bool) -> Option<()> {
        // A stream can never appear in a dict/array, so it should never be skipped.
        warn!("attempted to skip a stream object");

        None
    }
}

impl<'a> Readable<'a> for Stream<'a> {
    fn read(r: &mut Reader<'a>, ctx: &ReaderContext<'a>) -> Option<Self> {
        let dict = r.read_with_context::<Dict<'_>>(ctx)?;

        if dict.contains_key(F) {
            warn!("encountered stream referencing external file, which is unsupported");

            return None;
        }

        let offset = r.offset();
        parse_proper(r, &dict)
            .or_else(|| {
                warn!("failed to parse stream, trying to parse it manually");

                r.jump(offset);
                parse_fallback(r, &dict)
            })
            .error_none("was unable to manually parse the stream")
    }
}

#[derive(Debug, Copy, Clone)]
/// A failure that can occur during decoding a data stream.
pub enum DecodeFailure {
    /// An image stream failed to decode.
    ImageDecode,
    /// A data stream failed to decode.
    StreamDecode,
    /// A failure occurred while decrypting a file.
    Decryption,
    /// An unknown failure occurred.
    Unknown,
}

/// An image color space.
#[derive(Debug, Copy, Clone)]
pub enum ImageColorSpace {
    /// Grayscale color space.
    Gray,
    /// RGB color space.
    Rgb,
    /// CMYK color space.
    Cmyk,
    /// An unknown color space.
    Unknown(u8),
}

/// Additional data that is extracted from some image streams.
pub struct ImageData {
    /// An optional alpha channel of the image.
    pub alpha: Option<Vec<u8>>,
    /// The color space of the image.
    pub color_space: Option<ImageColorSpace>,
    /// The bits per component of the image.
    pub bits_per_component: u8,
    /// The width of the image.
    pub width: u32,
    /// The height of the image.
    pub height: u32,
}

/// The result of applying a filter.
pub struct FilterResult<'a> {
    /// The decoded data.
    pub data: Cow<'a, [u8]>,
    /// Additional data that is extracted from JPX image streams.
    pub image_data: Option<ImageData>,
}

impl FilterResult<'_> {
    pub(crate) fn from_data(data: Vec<u8>) -> Self {
        Self {
            data: Cow::Owned(data),
            image_data: None,
        }
    }
}

/// PdfCraft patch (20): the end-of-line after the `stream` keyword. Microsoft Reporting Services
/// writes `stream \r\n`, a space before the line break, which the specification does not allow.
/// Without this the content stream was taken to start at the space, failed to inflate, and the
/// page rendered blank (bank statements, invoices) while other viewers showed it. Only spaces and
/// tabs are skipped, and only when a line break follows, so stream data is never swallowed.
fn forward_stream_eol(r: &mut Reader<'_>) -> Option<()> {
    r.forward_while(|b| b == b' ' || b == b'\t');
    r.forward_tag(b"\n")
        .or_else(|| r.forward_tag(b"\r\n"))
        // Technically not allowed, but no reason to not try it.
        .or_else(|| r.forward_tag(b"\r"))
}

fn parse_proper<'a>(r: &mut Reader<'a>, dict: &Dict<'a>) -> Option<Stream<'a>> {
    let length = dict.get::<u32>(LENGTH)?;

    r.skip_white_spaces_and_comments();
    r.forward_tag(b"stream")?;
    forward_stream_eol(r)?;
    let data = r.read_bytes(length as usize)?;
    r.skip_white_spaces();
    r.forward_tag(b"endstream")?;

    Some(Stream::new(data, dict.clone()))
}

fn parse_fallback<'a>(r: &mut Reader<'a>, dict: &Dict<'a>) -> Option<Stream<'a>> {
    // PdfCraft patch: `stream` must be the next token. Scanning ahead to a later
    // `stream` made a dictionary look like a stream — in particular an indirect
    // `/AP /N` appearance-state dictionary (`<< /Yes 5 0 R /Off 6 0 R >>`, as Quartz
    // writes checkboxes). The widget then had no form to draw, so the box disappeared.
    // A wrong `/Length` still recovers, because that keyword sits directly after the dict.
    r.skip_white_spaces_and_comments();
    r.forward_tag(b"stream")?;
    forward_stream_eol(r)?;

    let tail = r.tail()?;
    let endstream_offset = find_needle(tail, b"endstream")?;
    let data_end = trim_trailing_ascii_whitespace(&tail[..endstream_offset]);
    let data = tail.get(..data_end)?;

    r.read_bytes(endstream_offset)?;
    r.skip_white_spaces();
    r.forward_tag(b"endstream")?;

    Some(Stream::new(data, dict.clone()))
}

fn trim_trailing_ascii_whitespace(data: &[u8]) -> usize {
    let mut end = data.len();

    while data
        .get(end.wrapping_sub(1))
        .copied()
        .is_some_and(is_white_space_character)
    {
        end -= 1;
    }

    end
}

impl<'a> TryFrom<Object<'a>> for Stream<'a> {
    type Error = ();

    fn try_from(value: Object<'a>) -> Result<Self, Self::Error> {
        match value {
            Object::Stream(s) => Ok(s),
            _ => Err(()),
        }
    }
}

impl<'a> ObjectLike<'a> for Stream<'a> {}
impl<'a> ObjectRefLike<'a> for Stream<'a> {
    fn cast_ref<'b>(obj: &'b Object<'a>) -> Option<&'b Self> {
        match obj {
            Object::Stream(stream) => Some(stream),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::object::Stream;
    use crate::reader::Reader;
    use crate::reader::{ReaderContext, ReaderExt};

    #[test]
    fn stream() {
        let data = b"<< /Length 10 >> stream\nabcdefghij\nendstream";
        let mut r = Reader::new(data);
        let stream = r
            .read_with_context::<Stream<'_>>(&ReaderContext::dummy())
            .unwrap();

        assert_eq!(stream.data, b"abcdefghij");
    }

    #[test]
    fn stream_fallback() {
        let data = b"<< /Length 999 >> stream\nabcdefghij\nendstream";
        let mut r = Reader::new(data);
        let stream = r
            .read_with_context::<Stream<'_>>(&ReaderContext::dummy())
            .unwrap();

        assert_eq!(stream.data, b"abcdefghij");
    }

    /// A dictionary followed by some other object's stream is not itself a stream.
    /// `parse_fallback` used to latch onto that later `stream` keyword.
    #[test]
    fn dictionary_is_not_a_later_objects_stream() {
        let data = b"<< /Yes 5 0 R /Off 6 0 R >>\nendobj\n11 0 obj << /Length 5 >> stream\nhello\nendstream";
        let mut r = Reader::new(data);
        assert!(r.read_with_context::<Stream<'_>>(&ReaderContext::dummy()).is_none());
    }
}

//! Merged (composite) image data section.

use std::borrow::Cow;

use crate::compression::{Compression, PlaneLayout, decode_planes, encode_planes, validate_planes};
use crate::error::Result;
use crate::header::Header;
use crate::io::Reader;
use crate::source::Stored;

/// The merged image: compression plus the encoded bytes of all channels
/// (to the end of the file), kept verbatim.
///
/// As with [`crate::ChannelData`], a file parsed by
/// [`PsdFile::from_source`](crate::PsdFile::from_source) may leave the bytes in the source
/// (`stored`, with `data` empty); equality compares the bytes wherever they are.
#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ImageData {
    /// Compression method for the whole image.
    pub compression: Compression,
    /// Encoded data. For RLE: the byte-count table for every row of every
    /// channel followed by the rows.
    pub data: Vec<u8>,
    /// Where the encoded bytes are when they were left in the source. (Not serialized.)
    #[cfg_attr(feature = "serde", serde(skip))]
    pub stored: Option<Stored>,
}

impl PartialEq for ImageData {
    fn eq(&self, other: &Self) -> bool {
        if self.compression != other.compression || self.encoded_len() != other.encoded_len() {
            return false;
        }
        match (self.encoded(), other.encoded()) {
            (Ok(a), Ok(b)) => a == b,
            _ => false,
        }
    }
}

impl Eq for ImageData {}

impl ImageData {
    /// A merged image whose encoded bytes are in memory.
    pub fn new(compression: Compression, data: Vec<u8>) -> Self {
        ImageData { compression, data, stored: None }
    }

    /// The encoded bytes: borrowed from `data`, or read from the source.
    pub fn encoded(&self) -> Result<Cow<'_, [u8]>> {
        crate::source::encoded(&self.data, self.stored.as_ref())
    }

    /// Number of encoded bytes, wherever they are.
    pub fn encoded_len(&self) -> u64 {
        self.stored.as_ref().map_or(self.data.len() as u64, |s| s.len)
    }

    /// Reads bytes left in the source into `data`.
    pub fn load(&mut self) -> Result<()> {
        if let Some(s) = &self.stored {
            self.data = s.read()?;
            self.stored = None;
        }
        Ok(())
    }

    /// Plane layout for the merged image described by `header`.
    pub fn layout(header: &Header) -> PlaneLayout {
        PlaneLayout {
            planes: usize::from(header.channels),
            width: header.width as usize,
            height: header.height as usize,
            depth: header.depth,
            version: header.version,
        }
    }

    /// Encodes planar data (`channels × height × row_bytes`).
    pub fn encode(compression: Compression, decoded: &[u8], header: &Header) -> Result<Self> {
        Ok(ImageData::new(compression, encode_planes(compression, decoded, &Self::layout(header))?))
    }

    /// Decodes all channels into planar big-endian samples.
    pub fn decode(&self, header: &Header) -> Result<Vec<u8>> {
        decode_planes(self.compression, &self.encoded()?, &Self::layout(header))
    }

    pub(crate) fn read(r: &mut Reader<'_>, header: &Header) -> Result<Self> {
        let compression = Compression::from_u16(r.u16()?);
        let data = r.peek_rest();
        validate_planes(compression, data, &Self::layout(header))?;
        let data = data.to_vec();
        r.skip(data.len())?;
        Ok(ImageData::new(compression, data))
    }
}

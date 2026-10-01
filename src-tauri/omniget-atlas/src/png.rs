//! Minimal PNG chunk walker.
//!
//! `atlas-check` has to answer two questions that a decoded image cannot answer:
//! how big the page is *without* decoding it, and whether the file carries an
//! embedded colour profile. Pages must be plain sRGB, so `iCCP`, `cHRM` and
//! `gAMA` are rejected on sight — a profile makes the browser and the packer
//! disagree about the pixels. Reading the chunk table directly is the only way
//! to see them; `image` drops them.

const SIGNATURE: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];

/// Colour chunks that are forbidden on an atlas page.
pub const COLOR_PROFILE_CHUNKS: [&str; 3] = ["iCCP", "cHRM", "gAMA"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PngInfo {
    pub width: u32,
    pub height: u32,
    /// Chunk types present in the file, in file order, deduplicated.
    pub chunks: Vec<String>,
}

impl PngInfo {
    /// Forbidden colour chunks found, in file order.
    pub fn color_profile_chunks(&self) -> Vec<&str> {
        self.chunks
            .iter()
            .map(String::as_str)
            .filter(|c| COLOR_PROFILE_CHUNKS.contains(c))
            .collect()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PngError {
    NotPng,
    Truncated,
    BadChunkLength,
    MissingIhdr,
}

impl std::fmt::Display for PngError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            PngError::NotPng => "not a PNG file (signature mismatch)",
            PngError::Truncated => "PNG truncated",
            PngError::BadChunkLength => "PNG chunk length out of range",
            PngError::MissingIhdr => "PNG has no IHDR chunk",
        };
        f.write_str(s)
    }
}

impl std::error::Error for PngError {}

/// Walks the chunk table of `bytes` without decoding any pixels.
pub fn inspect(bytes: &[u8]) -> Result<PngInfo, PngError> {
    if bytes.len() < 8 || bytes[..8] != SIGNATURE {
        return Err(PngError::NotPng);
    }
    let mut pos = 8usize;
    let mut chunks: Vec<String> = Vec::new();
    let mut size: Option<(u32, u32)> = None;

    while pos + 8 <= bytes.len() {
        let len = u32::from_be_bytes([bytes[pos], bytes[pos + 1], bytes[pos + 2], bytes[pos + 3]])
            as usize;
        if len > i32::MAX as usize {
            return Err(PngError::BadChunkLength);
        }
        let kind = String::from_utf8_lossy(&bytes[pos + 4..pos + 8]).into_owned();
        let data_start = pos + 8;
        let data_end = data_start
            .checked_add(len)
            .ok_or(PngError::BadChunkLength)?;
        // 4 more bytes of CRC follow the data.
        if data_end + 4 > bytes.len() {
            return Err(PngError::Truncated);
        }
        if kind == "IHDR" {
            if len < 8 {
                return Err(PngError::Truncated);
            }
            let d = &bytes[data_start..data_start + 8];
            size = Some((
                u32::from_be_bytes([d[0], d[1], d[2], d[3]]),
                u32::from_be_bytes([d[4], d[5], d[6], d[7]]),
            ));
        }
        if !chunks.contains(&kind) {
            chunks.push(kind.clone());
        }
        if kind == "IEND" {
            break;
        }
        pos = data_end + 4;
    }

    let (width, height) = size.ok_or(PngError::MissingIhdr)?;
    Ok(PngInfo {
        width,
        height,
        chunks,
    })
}

/// Rewrites `bytes` with an extra ancillary chunk inserted right after IHDR.
/// Only used by tests, to forge a page that carries a colour profile.
pub fn insert_chunk_after_ihdr(
    bytes: &[u8],
    kind: &[u8; 4],
    data: &[u8],
) -> Result<Vec<u8>, PngError> {
    if bytes.len() < 8 || bytes[..8] != SIGNATURE {
        return Err(PngError::NotPng);
    }
    let mut pos = 8usize;
    while pos + 8 <= bytes.len() {
        let len = u32::from_be_bytes([bytes[pos], bytes[pos + 1], bytes[pos + 2], bytes[pos + 3]])
            as usize;
        let end = pos + 8 + len + 4;
        if end > bytes.len() {
            return Err(PngError::Truncated);
        }
        if &bytes[pos + 4..pos + 8] == b"IHDR" {
            let mut out = Vec::with_capacity(bytes.len() + data.len() + 12);
            out.extend_from_slice(&bytes[..end]);
            out.extend_from_slice(&(data.len() as u32).to_be_bytes());
            out.extend_from_slice(kind);
            out.extend_from_slice(data);
            let mut crc_input = Vec::with_capacity(4 + data.len());
            crc_input.extend_from_slice(kind);
            crc_input.extend_from_slice(data);
            out.extend_from_slice(&crc32(&crc_input).to_be_bytes());
            out.extend_from_slice(&bytes[end..]);
            return Ok(out);
        }
        pos = end;
    }
    Err(PngError::MissingIhdr)
}

/// CRC-32 (IEEE), the one PNG uses. Small enough not to be worth a dependency.
fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xffff_ffffu32;
    for byte in data {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xedb8_8320 & mask);
        }
    }
    !crc
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tiny_png() -> Vec<u8> {
        let mut out = Vec::new();
        let img = image::RgbaImage::from_pixel(3, 5, image::Rgba([1, 2, 3, 255]));
        image::DynamicImage::ImageRgba8(img)
            .write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png)
            .expect("encode");
        out
    }

    #[test]
    fn crc32_matches_the_known_vector() {
        // "123456789" -> 0xCBF43926 is the standard CRC-32/ISO-HDLC check value.
        assert_eq!(crc32(b"123456789"), 0xcbf4_3926);
    }

    #[test]
    fn inspect_reads_size_and_rejects_non_png() {
        let info = inspect(&tiny_png()).expect("inspect");
        assert_eq!((info.width, info.height), (3, 5));
        assert!(info.chunks.contains(&"IHDR".to_string()));
        assert!(
            info.color_profile_chunks().is_empty(),
            "a freshly encoded page is plain sRGB"
        );
        assert_eq!(inspect(b"not a png at all").unwrap_err(), PngError::NotPng);
    }

    #[test]
    fn inspect_sees_an_injected_colour_profile() {
        let forged =
            insert_chunk_after_ihdr(&tiny_png(), b"gAMA", &45455u32.to_be_bytes()).expect("forge");
        let info = inspect(&forged).expect("inspect");
        assert_eq!(info.color_profile_chunks(), vec!["gAMA"]);
        assert_eq!(
            (info.width, info.height),
            (3, 5),
            "the forged file still decodes its header"
        );
        // The forged file must still be a valid PNG for the decoder.
        image::load_from_memory(&forged).expect("still decodable");
    }

    #[test]
    fn inspect_reports_truncation() {
        let bytes = tiny_png();
        assert_eq!(inspect(&bytes[..20]).unwrap_err(), PngError::Truncated);
    }
}

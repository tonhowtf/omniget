use crate::reader::Reader;
use alloc::vec;
use alloc::vec::Vec;

/// PdfCraft patch: `limit` is the most bytes produced (see `filter::decode_limit`); a run of 128
/// copies costs two input bytes, so a few megabytes could ask for gigabytes.
pub(crate) fn decode(data: &[u8], limit: usize) -> Option<Vec<u8>> {
    let mut reader = Reader::new(data);
    let mut decoded = vec![];

    loop {
        let left = limit.saturating_sub(decoded.len());
        if left == 0 {
            break;
        }
        let length = reader.read_byte()?;

        match length {
            128 => break,
            0..=127 => {
                // PDFBOX-3990, just abort early if stream is invalid.
                let Some(bytes) = reader.read_bytes(length as usize + 1) else {
                    break;
                };

                decoded.extend(bytes.iter().take(left));
            }
            _ => {
                let length = (257 - length as usize).min(left);
                let byte = reader.read_byte()?;
                decoded.extend(core::iter::repeat_n(byte, length));
            }
        }
    }

    // PdfCraft patch: output cut at the limit is reported, never dropped silently.
    if decoded.len() >= limit {
        warn!("run-length stream stopped at its decode limit of {limit} bytes");
    }
    Some(decoded)
}

#[cfg(test)]
mod tests {
    use crate::filter::run_length::decode;

    #[test]
    fn run_length() {
        let input = vec![4, 10, 11, 12, 13, 14, 253, 3, 128];
        assert_eq!(
            decode(&input, usize::MAX).unwrap(),
            vec![10, 11, 12, 13, 14, 3, 3, 3, 3]
        );
    }
}

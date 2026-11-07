// Keep in sync with `hayro-postscript/src/string/ascii_85.rs`.

use crate::reader::Reader;
use crate::trivia::is_white_space_character;
use alloc::vec::Vec;

/// PdfCraft patch: `limit` is the most bytes produced (see `filter::decode_limit`); each `z`
/// expands one input byte to four.
pub(crate) fn decode(data: &[u8], limit: usize) -> Option<Vec<u8>> {
    const POW_85: [u32; 5] = [52200625, 614125, 7225, 85, 1];

    let mut reader = Reader::new(data);

    let mut read_byte = || -> Option<u8> {
        loop {
            let b = reader.read_byte()?;

            // White space characters should be ignored.
            if !is_white_space_character(b) {
                return Some(b);
            }
        }
    };

    let flush_group = |group: &mut Vec<u8>, decoded: &mut Vec<u8>| -> Option<()> {
        let (digits, output_len): ([u32; 5], usize) = match group.len() {
            0 => return Some(()),
            1 => return None, // A single character is not valid.
            2 => (
                [group[0], group[1], b'u', b'u', b'u'].map(|b| (b - b'!') as u32),
                1,
            ),
            3 => (
                [group[0], group[1], group[2], b'u', b'u'].map(|b| (b - b'!') as u32),
                2,
            ),
            4 => (
                [group[0], group[1], group[2], group[3], b'u'].map(|b| (b - b'!') as u32),
                3,
            ),
            5 => (
                [group[0], group[1], group[2], group[3], group[4]].map(|b| (b - b'!') as u32),
                4,
            ),
            _ => unreachable!(),
        };

        let value = digits[0]
            .checked_mul(POW_85[0])?
            .checked_add(digits[1].checked_mul(POW_85[1])?)?
            .checked_add(digits[2].checked_mul(POW_85[2])?)?
            .checked_add(digits[3].checked_mul(POW_85[3])?)?
            .checked_add(digits[4])?;

        let left = limit.saturating_sub(decoded.len());
        decoded.extend_from_slice(&value.to_be_bytes()[..output_len.min(left)]);
        group.clear();
        Some(())
    };

    let mut decoded = Vec::with_capacity((data.len() / 5 * 4).min(limit));
    let mut group = Vec::with_capacity(5);

    loop {
        // PdfCraft patch: output cut at the limit is reported, never dropped silently.
        if decoded.len() >= limit {
            warn!("ASCII85 stream stopped at its decode limit of {limit} bytes");
            return Some(decoded);
        }
        let Some(b) = read_byte() else {
            // Be lenient and accept what we have (see PDFBOX-5910).
            flush_group(&mut group, &mut decoded)?;

            return Some(decoded);
        };

        match b {
            b'!'..=b'u' => {
                group.push(b);

                if group.len() == 5 {
                    flush_group(&mut group, &mut decoded)?;
                }
            }
            b'z' => {
                flush_group(&mut group, &mut decoded)?;
                let left = limit.saturating_sub(decoded.len());
                decoded.extend(core::iter::repeat_n(0, left.min(4)));
            }
            b'~' => {
                // Technically requires a '>', but there is a PDF where it isn't
                // appended and decodes fine in other viewers.
                flush_group(&mut group, &mut decoded)?;

                return Some(decoded);
            }
            _ => return None, // Invalid character.
        }
    }
}

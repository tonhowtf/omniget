//! ASCIIHexDecode (ISO 32000-2 §7.4.2): pairs of hex digits, white space ignored,
//! `>` is EOD, an odd final digit is padded with 0. A missing EOD is tolerated.

use crate::{Failure, Step, is_pdf_whitespace};

const NAME: &str = "ASCIIHexDecode";

fn nibble(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

pub(crate) fn decode(data: &[u8], max: usize) -> Step {
    let mut out = Vec::with_capacity((data.len() / 2).min(max));
    let mut hi: Option<u8> = None;
    for (i, &b) in data.iter().enumerate() {
        if is_pdf_whitespace(b) {
            continue;
        }
        if b == b'>' {
            break;
        }
        let Some(n) = nibble(b) else {
            return Err(Failure::corrupt(NAME, format!("invalid character 0x{b:02x} at offset {i}"), out));
        };
        match hi.take() {
            None => hi = Some(n),
            Some(h) => {
                if out.len() == max {
                    return Err(Failure::limit(max));
                }
                out.push(h << 4 | n);
            }
        }
    }
    if let Some(h) = hi {
        if out.len() == max {
            return Err(Failure::limit(max));
        }
        out.push(h << 4);
    }
    Ok(out)
}

/// Upper-case hex, a newline every 64 digits, terminated by `>`.
pub(crate) fn encode(data: &[u8]) -> Vec<u8> {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let mut out = Vec::with_capacity(data.len() * 2 + data.len() / 32 + 1);
    for (i, &b) in data.iter().enumerate() {
        if i > 0 && i % 32 == 0 {
            out.push(b'\n');
        }
        out.push(HEX[usize::from(b >> 4)]);
        out.push(HEX[usize::from(b & 15)]);
    }
    out.push(b'>');
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::FilterError;

    fn dec(d: &[u8]) -> Result<Vec<u8>, FilterError> {
        decode(d, 1000).map_err(|f| f.error)
    }

    #[test]
    fn vectors() {
        assert_eq!(dec(b"48656c6C6F>").unwrap(), b"Hello");
        assert_eq!(dec(b" 4 8\n6\r5\t6\x0c c\x006c 6f >").unwrap(), b"Hello");
        assert_eq!(dec(b"901FA>").unwrap(), [0x90, 0x1f, 0xa0]);
        assert_eq!(dec(b"901FA").unwrap(), [0x90, 0x1f, 0xa0]); // missing EOD
        assert_eq!(dec(b"41>zz garbage after EOD").unwrap(), b"A");
        assert_eq!(dec(b">").unwrap(), b"");
        assert_eq!(dec(b"").unwrap(), b"");
    }

    #[test]
    fn invalid_char() {
        let f = decode(b"4142G43>", 1000).err().unwrap();
        assert!(matches!(f.error, FilterError::Corrupt { .. }));
        assert_eq!(f.partial, b"AB");
    }

    #[test]
    fn limit() {
        assert!(matches!(decode(b"414243", 2).map_err(|f| f.error), Err(FilterError::LimitExceeded(2))));
        assert!(matches!(decode(b"41424", 2).map_err(|f| f.error), Err(FilterError::LimitExceeded(2))));
        assert_eq!(decode(b"4142", 2).ok().unwrap(), b"AB");
    }

    #[test]
    fn encoding() {
        assert_eq!(encode(b"Hello"), b"48656C6C6F>");
        assert_eq!(encode(b""), b">");
        let long = encode(&[0xab; 40]);
        assert_eq!(long[64], b'\n');
        assert_eq!(dec(&long).unwrap(), [0xab; 40]);
    }
}

//! ASCII85Decode (ISO 32000-2 §7.4.3): groups of five characters `!`..`u` encode four
//! bytes base-85, `z` stands for four zero bytes, `~>` is EOD, white space is ignored and
//! a final partial group of n characters (2–5) yields n − 1 bytes. A leading `<~` and a
//! missing EOD are tolerated.

use crate::{Failure, Step, is_pdf_whitespace};

const NAME: &str = "ASCII85Decode";

fn push(out: &mut Vec<u8>, bytes: &[u8], max: usize) -> Result<(), Failure> {
    if out.len() + bytes.len() > max {
        return Err(Failure::limit(max));
    }
    out.extend_from_slice(bytes);
    Ok(())
}

pub(crate) fn decode(data: &[u8], max: usize) -> Step {
    let start = data.iter().position(|&b| !is_pdf_whitespace(b)).unwrap_or(data.len());
    let body = data[start..].strip_prefix(b"<~").unwrap_or(&data[start..]);
    let mut out = Vec::with_capacity((body.len() / 5 * 4 + 4).min(max));
    let mut acc: u64 = 0;
    let mut n = 0usize;
    for (i, &b) in body.iter().enumerate() {
        match b {
            b'!'..=b'u' => {
                acc = acc * 85 + u64::from(b - b'!');
                n += 1;
                if n == 5 {
                    let Ok(v) = u32::try_from(acc) else {
                        return Err(Failure::corrupt(NAME, format!("group ending at {i} exceeds 2^32"), out));
                    };
                    push(&mut out, &v.to_be_bytes(), max)?;
                    acc = 0;
                    n = 0;
                }
            }
            b'z' if n == 0 => push(&mut out, &[0; 4], max)?,
            b'z' => return Err(Failure::corrupt(NAME, format!("'z' inside a group at {i}"), out)),
            // `~` starts the EOD marker; tolerate it without the following `>`.
            b'~' => break,
            b if is_pdf_whitespace(b) => {}
            b => return Err(Failure::corrupt(NAME, format!("invalid character 0x{b:02x} at {i}"), out)),
        }
    }
    match n {
        0 => {}
        1 => return Err(Failure::corrupt(NAME, "final group has a single character", out)),
        _ => {
            // Pad with `u` (84) to a full group and keep n − 1 bytes.
            for _ in n..5 {
                acc = acc * 85 + 84;
            }
            let Ok(v) = u32::try_from(acc) else {
                return Err(Failure::corrupt(NAME, "final group exceeds 2^32", out));
            };
            push(&mut out, &v.to_be_bytes()[..n - 1], max)?;
        }
    }
    Ok(out)
}

/// Encodes with `z` for zero groups, a newline every 75 characters, and a `~>` EOD.
pub(crate) fn encode(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len() * 5 / 4 + data.len() / 60 + 8);
    let mut col = 0;
    let mut put = |out: &mut Vec<u8>, s: &[u8]| {
        for &c in s {
            if col == 75 {
                out.push(b'\n');
                col = 0;
            }
            out.push(c);
            col += 1;
        }
    };
    for chunk in data.chunks(4) {
        let mut word = [0u8; 4];
        word[..chunk.len()].copy_from_slice(chunk);
        let mut v = u32::from_be_bytes(word);
        if v == 0 && chunk.len() == 4 {
            put(&mut out, b"z");
            continue;
        }
        let mut digits = [0u8; 5];
        for d in digits.iter_mut().rev() {
            *d = (v % 85) as u8 + b'!';
            v /= 85;
        }
        put(&mut out, &digits[..chunk.len() + 1]);
    }
    out.extend_from_slice(b"~>");
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
        // "Man " = 0x4D616E20 = 24·85⁴ + 73·85³ + 80·85² + 78·85 + 61 -> "9jqo^".
        assert_eq!(dec(b"9jqo^~>").unwrap(), b"Man ");
        assert_eq!(encode(b"Man "), b"9jqo^~>");
        // "M" padded to 0x4D000000 -> "9`" (24, 63); decoding pads with 'u'.
        assert_eq!(encode(b"M"), b"9`~>");
        assert_eq!(dec(b"9`~>").unwrap(), b"M");
        assert_eq!(dec(b"z~>").unwrap(), [0; 4]);
        assert_eq!(encode(&[0; 4]), b"z~>");
        // A partial zero group is not `z`.
        assert_eq!(encode(&[0; 3]), b"!!!!~>");
        assert_eq!(dec(b"!!!!~>").unwrap(), [0; 3]);
        assert_eq!(dec(b"s8W-!~>").unwrap(), [0xff; 4]);
    }

    #[test]
    fn syntax_tolerance() {
        assert_eq!(dec(b"<~9jqo^~>").unwrap(), b"Man ");
        assert_eq!(dec(b" \n<~9j\r\nq o\t^z~>").unwrap(), b"Man \0\0\0\0");
        assert_eq!(dec(b"9jqo^").unwrap(), b"Man "); // missing EOD
        assert_eq!(dec(b"9jqo^~").unwrap(), b"Man "); // bare ~
        assert_eq!(dec(b"9jqo^~>garbage{").unwrap(), b"Man ");
        assert_eq!(dec(b"~>").unwrap(), b"");
        assert_eq!(dec(b"").unwrap(), b"");
    }

    #[test]
    fn corrupt() {
        assert!(matches!(dec(b"9jz~>"), Err(FilterError::Corrupt { .. })));
        assert!(matches!(dec(b"9jqo^9~>"), Err(FilterError::Corrupt { .. })));
        assert!(matches!(dec(b"uuuuu~>"), Err(FilterError::Corrupt { .. }))); // > 2^32 - 1
        assert!(matches!(dec(b"9jqo^v~>"), Err(FilterError::Corrupt { .. })));
        let f = decode(b"9jqo^{", 100).err().unwrap();
        assert_eq!(f.partial, b"Man ");
    }

    #[test]
    fn limit() {
        assert!(matches!(decode(b"zz", 7).map_err(|f| f.error), Err(FilterError::LimitExceeded(7))));
        assert!(matches!(decode(b"z9`", 4).map_err(|f| f.error), Err(FilterError::LimitExceeded(4))));
        assert_eq!(decode(b"zz", 8).ok().unwrap().len(), 8);
    }

    #[test]
    fn line_wrapping() {
        let data: Vec<u8> = (0..=255u8).collect();
        let e = encode(&data);
        assert!(e.split(|&b| b == b'\n').all(|l| l.len() <= 77));
        assert_eq!(dec(&e).unwrap_or_default().len(), 256);
    }
}

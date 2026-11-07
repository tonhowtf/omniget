//! RunLengthDecode (ISO 32000-2 §7.4.5): a length byte L of 0–127 copies the next
//! L + 1 bytes, 129–255 repeats the next byte 257 − L times, 128 is EOD.
//! A missing EOD is tolerated; a truncated run is corrupt.

use crate::{Failure, Step};

const NAME: &str = "RunLengthDecode";

pub(crate) fn decode(data: &[u8], max: usize) -> Step {
    let mut out = Vec::with_capacity(data.len().saturating_mul(2).min(max));
    let mut i = 0;
    while let Some(&l) = data.get(i) {
        i += 1;
        match l {
            128 => break,
            0..=127 => {
                let n = usize::from(l) + 1;
                let avail = n.min(data.len() - i);
                if out.len() + avail > max {
                    return Err(Failure::limit(max));
                }
                out.extend_from_slice(&data[i..i + avail]);
                i += avail;
                if avail < n {
                    return Err(Failure::corrupt(NAME, "truncated literal run", out));
                }
            }
            _ => {
                let Some(&b) = data.get(i) else {
                    return Err(Failure::corrupt(NAME, "truncated repeat run", out));
                };
                i += 1;
                let n = 257 - usize::from(l);
                if out.len() + n > max {
                    return Err(Failure::limit(max));
                }
                out.resize(out.len() + n, b);
            }
        }
    }
    Ok(out)
}

pub(crate) fn encode(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len() + data.len() / 128 + 2);
    let n = data.len();
    let mut i = 0;
    while i < n {
        let mut run = 1;
        while i + run < n && run < 128 && data[i + run] == data[i] {
            run += 1;
        }
        if run >= 2 {
            out.push((257 - run) as u8);
            out.push(data[i]);
            i += run;
            continue;
        }
        // Literal: extend until a repeat begins or 128 bytes.
        let start = i;
        let mut j = i + 1;
        while j < n && j - start < 128 && !(j + 1 < n && data[j] == data[j + 1]) {
            j += 1;
        }
        out.push((j - start - 1) as u8);
        out.extend_from_slice(&data[start..j]);
        i = j;
    }
    out.push(128);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::FilterError;

    fn dec(d: &[u8]) -> Result<Vec<u8>, FilterError> {
        decode(d, 10_000).map_err(|f| f.error)
    }

    #[test]
    fn vectors() {
        assert_eq!(dec(&[2, b'a', b'b', b'c', 254, b'x', 128]).unwrap(), b"abcxxx");
        assert_eq!(dec(&[129, 7, 128]).unwrap(), [7; 128]);
        assert!(dec(&[127]).is_err());
        assert_eq!(dec(&[0, b'q']).unwrap(), b"q"); // missing EOD
        assert_eq!(dec(&[128, 0, b'q']).unwrap(), b""); // data after EOD ignored
        assert_eq!(dec(&[]).unwrap(), b"");
    }

    #[test]
    fn truncated() {
        let f = decode(&[4, b'a', b'b'], 100).err().unwrap();
        assert!(matches!(f.error, FilterError::Corrupt { .. }));
        assert_eq!(f.partial, b"ab");
        assert!(matches!(dec(&[0, b'a', 200]), Err(FilterError::Corrupt { .. })));
    }

    #[test]
    fn limit() {
        assert!(matches!(decode(&[129, 0, 129, 0], 200).map_err(|f| f.error), Err(FilterError::LimitExceeded(200))));
        assert!(matches!(decode(&[3, 1, 2, 3, 4], 3).map_err(|f| f.error), Err(FilterError::LimitExceeded(3))));
        assert_eq!(decode(&[129, 0, 129, 0], 256).ok().unwrap().len(), 256);
    }

    #[test]
    fn encoding() {
        assert_eq!(encode(b"abcxxx"), [2, b'a', b'b', b'c', 254, b'x', 128]);
        assert_eq!(encode(b""), [128]);
        assert_eq!(encode(&[5; 300]), [129, 5, 129, 5, 213, 5, 128]);
        let lit: Vec<u8> = (0..200u8).collect();
        let e = encode(&lit);
        assert_eq!((e[0], e[129]), (127, 71));
        assert_eq!(dec(&e).unwrap(), lit);
    }
}

use super::super::api::{BilibiliError, Result};

#[derive(Debug, Clone, Default)]
pub struct DanmakuElem {
    pub id: i64,
    pub progress_ms: i32,
    pub mode: i32,
    pub fontsize: i32,
    pub color: u32,
    pub mid_hash: String,
    pub content: String,
    pub ctime: i64,
    pub weight: i32,
    pub pool: i32,
    pub attr: i32,
}

pub fn decode_segment(bytes: &[u8]) -> Result<Vec<DanmakuElem>> {
    let mut elems: Vec<DanmakuElem> = Vec::new();
    let mut idx = 0usize;
    while idx < bytes.len() {
        let (key, k_len) = read_varint(bytes, idx).ok_or(BilibiliError::ContentUnavailable)?;
        idx += k_len;
        let field = (key >> 3) as u32;
        let wire = (key & 0x07) as u8;
        if field == 1 && wire == 2 {
            let (len, l_len) = read_varint(bytes, idx).ok_or(BilibiliError::ContentUnavailable)?;
            idx += l_len;
            let end = idx
                .checked_add(len as usize)
                .ok_or(BilibiliError::ContentUnavailable)?;
            if end > bytes.len() {
                return Err(BilibiliError::ContentUnavailable);
            }
            let elem = decode_elem(&bytes[idx..end])?;
            elems.push(elem);
            idx = end;
        } else {
            idx = skip_field(bytes, idx, wire).ok_or(BilibiliError::ContentUnavailable)?;
        }
    }
    Ok(elems)
}

fn decode_elem(bytes: &[u8]) -> Result<DanmakuElem> {
    let mut elem = DanmakuElem::default();
    let mut idx = 0usize;
    while idx < bytes.len() {
        let (key, k_len) = read_varint(bytes, idx).ok_or(BilibiliError::ContentUnavailable)?;
        idx += k_len;
        let field = (key >> 3) as u32;
        let wire = (key & 0x07) as u8;
        match (field, wire) {
            (1, 0) => {
                let (v, vl) = read_varint(bytes, idx).ok_or(BilibiliError::ContentUnavailable)?;
                idx += vl;
                elem.id = v as i64;
            }
            (2, 0) => {
                let (v, vl) = read_varint(bytes, idx).ok_or(BilibiliError::ContentUnavailable)?;
                idx += vl;
                elem.progress_ms = v as i32;
            }
            (3, 0) => {
                let (v, vl) = read_varint(bytes, idx).ok_or(BilibiliError::ContentUnavailable)?;
                idx += vl;
                elem.mode = v as i32;
            }
            (4, 0) => {
                let (v, vl) = read_varint(bytes, idx).ok_or(BilibiliError::ContentUnavailable)?;
                idx += vl;
                elem.fontsize = v as i32;
            }
            (5, 0) => {
                let (v, vl) = read_varint(bytes, idx).ok_or(BilibiliError::ContentUnavailable)?;
                idx += vl;
                elem.color = v as u32;
            }
            (6, 2) => {
                let (v, vl) =
                    read_length_delimited(bytes, idx).ok_or(BilibiliError::ContentUnavailable)?;
                idx += vl;
                elem.mid_hash = String::from_utf8(v.to_vec()).unwrap_or_default();
            }
            (7, 2) => {
                let (v, vl) =
                    read_length_delimited(bytes, idx).ok_or(BilibiliError::ContentUnavailable)?;
                idx += vl;
                elem.content = String::from_utf8(v.to_vec()).unwrap_or_default();
            }
            (8, 0) => {
                let (v, vl) = read_varint(bytes, idx).ok_or(BilibiliError::ContentUnavailable)?;
                idx += vl;
                elem.ctime = v as i64;
            }
            (9, 0) => {
                let (v, vl) = read_varint(bytes, idx).ok_or(BilibiliError::ContentUnavailable)?;
                idx += vl;
                elem.weight = v as i32;
            }
            (11, 0) => {
                let (v, vl) = read_varint(bytes, idx).ok_or(BilibiliError::ContentUnavailable)?;
                idx += vl;
                elem.pool = v as i32;
            }
            (13, 0) => {
                let (v, vl) = read_varint(bytes, idx).ok_or(BilibiliError::ContentUnavailable)?;
                idx += vl;
                elem.attr = v as i32;
            }
            _ => {
                idx = skip_field(bytes, idx, wire).ok_or(BilibiliError::ContentUnavailable)?;
            }
        }
    }
    Ok(elem)
}

fn read_varint(bytes: &[u8], start: usize) -> Option<(u64, usize)> {
    let mut result: u64 = 0;
    let mut shift = 0u32;
    let mut idx = start;
    while idx < bytes.len() {
        let b = bytes[idx];
        idx += 1;
        result |= ((b & 0x7F) as u64) << shift;
        if b & 0x80 == 0 {
            return Some((result, idx - start));
        }
        shift += 7;
        if shift >= 64 {
            return None;
        }
    }
    None
}

fn read_length_delimited(bytes: &[u8], start: usize) -> Option<(&[u8], usize)> {
    let (len, l_len) = read_varint(bytes, start)?;
    let body_start = start + l_len;
    let body_end = body_start.checked_add(len as usize)?;
    if body_end > bytes.len() {
        return None;
    }
    Some((&bytes[body_start..body_end], body_end - start))
}

fn skip_field(bytes: &[u8], start: usize, wire: u8) -> Option<usize> {
    match wire {
        0 => {
            let (_, len) = read_varint(bytes, start)?;
            Some(start + len)
        }
        1 => {
            if start + 8 > bytes.len() {
                None
            } else {
                Some(start + 8)
            }
        }
        2 => {
            let (_, len) = read_length_delimited(bytes, start)?;
            Some(start + len)
        }
        5 => {
            if start + 4 > bytes.len() {
                None
            } else {
                Some(start + 4)
            }
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn build_elem_bytes() -> Vec<u8> {
        let mut buf: Vec<u8> = Vec::new();
        write_tag_varint(&mut buf, 2, 1234);
        write_tag_varint(&mut buf, 3, 1);
        write_tag_varint(&mut buf, 4, 25);
        write_tag_varint(&mut buf, 5, 0xFFFFFF);
        write_tag_str(&mut buf, 7, "hello danmaku");
        write_tag_varint(&mut buf, 8, 1700000000);
        buf
    }

    fn write_varint(buf: &mut Vec<u8>, mut v: u64) {
        while v >= 0x80 {
            buf.push(((v & 0x7F) as u8) | 0x80);
            v >>= 7;
        }
        buf.push(v as u8);
    }

    fn write_tag_varint(buf: &mut Vec<u8>, field: u32, value: u64) {
        let key = (field << 3) | 0;
        write_varint(buf, key as u64);
        write_varint(buf, value);
    }

    fn write_tag_str(buf: &mut Vec<u8>, field: u32, s: &str) {
        let key = (field << 3) | 2;
        write_varint(buf, key as u64);
        write_varint(buf, s.len() as u64);
        buf.extend_from_slice(s.as_bytes());
    }

    fn build_segment_bytes(elems: usize) -> Vec<u8> {
        let elem_bytes = build_elem_bytes();
        let mut buf: Vec<u8> = Vec::new();
        for _ in 0..elems {
            let key = (1 << 3) | 2;
            write_varint(&mut buf, key as u64);
            write_varint(&mut buf, elem_bytes.len() as u64);
            buf.extend_from_slice(&elem_bytes);
        }
        buf
    }

    #[test]
    fn decode_single_elem() {
        let bytes = build_segment_bytes(1);
        let elems = decode_segment(&bytes).unwrap();
        assert_eq!(elems.len(), 1);
        let e = &elems[0];
        assert_eq!(e.progress_ms, 1234);
        assert_eq!(e.mode, 1);
        assert_eq!(e.fontsize, 25);
        assert_eq!(e.color, 0xFFFFFF);
        assert_eq!(e.content, "hello danmaku");
        assert_eq!(e.ctime, 1700000000);
    }

    #[test]
    fn decode_multiple_elems() {
        let bytes = build_segment_bytes(5);
        let elems = decode_segment(&bytes).unwrap();
        assert_eq!(elems.len(), 5);
    }
}

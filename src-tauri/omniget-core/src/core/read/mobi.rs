use super::epub::BookMeta;

fn be16(b: &[u8], o: usize) -> Option<u16> {
    Some(u16::from_be_bytes(b.get(o..o + 2)?.try_into().ok()?))
}

fn be32(b: &[u8], o: usize) -> Option<u32> {
    Some(u32::from_be_bytes(b.get(o..o + 4)?.try_into().ok()?))
}

pub struct Mobi<'a> {
    data: &'a [u8],
    offsets: Vec<usize>,
}

impl<'a> Mobi<'a> {
    pub fn parse(data: &'a [u8]) -> Option<Self> {
        if data.get(60..68)? != b"BOOKMOBI" && data.get(60..68)? != b"TEXtREAd" {
            return None;
        }
        let n = be16(data, 76)? as usize;
        let mut offsets = Vec::with_capacity(n);
        for i in 0..n {
            offsets.push(be32(data, 78 + i * 8)? as usize);
        }
        Some(Self { data, offsets })
    }

    pub fn record(&self, i: usize) -> Option<&'a [u8]> {
        let start = *self.offsets.get(i)?;
        let end = self.offsets.get(i + 1).copied().unwrap_or(self.data.len());
        self.data.get(start..end.max(start))
    }

    pub fn meta(&self) -> (BookMeta, Option<Vec<u8>>) {
        let mut m = BookMeta::default();
        let Some(r0) = self.record(0) else {
            return (m, None);
        };
        let mobi = &r0[16.min(r0.len())..];
        if mobi.get(0..4) != Some(b"MOBI") {
            let name = &self.data[..32.min(self.data.len())];
            m.title = String::from_utf8_lossy(name)
                .trim_end_matches('\0')
                .to_string();
            return (m, None);
        }
        let header_len = be32(mobi, 4).unwrap_or(0) as usize;
        let encoding = be32(mobi, 12).unwrap_or(65001);
        let decode = |b: &[u8]| -> String {
            if encoding == 1252 {
                b.iter().map(|&c| c as char).collect()
            } else {
                String::from_utf8_lossy(b).to_string()
            }
        };
        let title_off = be32(mobi, 68).unwrap_or(0) as usize;
        let title_len = be32(mobi, 72).unwrap_or(0) as usize;
        if let Some(t) = r0.get(title_off..title_off + title_len) {
            m.title = decode(t);
        }
        let lang_code = be32(mobi, 76).unwrap_or(0);
        m.language = locale_to_lang(lang_code & 0xff).to_string();
        let first_image = be32(mobi, 92).unwrap_or(u32::MAX) as usize;
        let exth_flag = be32(mobi, 112).unwrap_or(0);
        let mut cover_off: Option<usize> = None;
        let mut thumb_off: Option<usize> = None;
        if exth_flag & 0x40 != 0 {
            let exth = &mobi[header_len.min(mobi.len())..];
            if exth.get(0..4) == Some(b"EXTH") {
                let count = be32(exth, 8).unwrap_or(0) as usize;
                let mut p = 12;
                for _ in 0..count {
                    let (Some(kind), Some(len)) = (be32(exth, p), be32(exth, p + 4)) else {
                        break;
                    };
                    let len = len as usize;
                    if len < 8 {
                        break;
                    }
                    let Some(val) = exth.get(p + 8..p + len) else {
                        break;
                    };
                    let s = || decode(val).trim().to_string();
                    match kind {
                        100 => m.authors.push(s()),
                        101 => m.publisher = s(),
                        103 => m.description = s(),
                        104 => m.identifiers.push(("isbn".into(), s())),
                        105 => m.subjects.push(s()),
                        106 => m.published = s(),
                        109 => m.rights = s(),
                        113 => m.identifiers.push(("asin".into(), s())),
                        201 => cover_off = be32(val, 0).map(|v| v as usize),
                        202 => thumb_off = be32(val, 0).map(|v| v as usize),
                        503 => m.title = s(),
                        524 => m.language = s(),
                        527 => m.rtl = s() == "rtl",
                        _ => {}
                    }
                    p += len;
                }
            }
        }
        let cover = cover_off
            .or(thumb_off)
            .filter(|_| first_image != u32::MAX as usize)
            .and_then(|o| self.record(first_image + o))
            .filter(|b| {
                b.starts_with(&[0xFF, 0xD8]) || b.starts_with(b"\x89PNG") || b.starts_with(b"GIF8")
            })
            .map(|b| b.to_vec());
        (m, cover)
    }
}

fn locale_to_lang(code: u32) -> &'static str {
    match code {
        0x09 => "en",
        0x16 => "pt",
        0x0a => "es",
        0x0c => "fr",
        0x07 => "de",
        0x10 => "it",
        0x11 => "ja",
        0x12 => "ko",
        0x04 => "zh",
        0x19 => "ru",
        0x13 => "nl",
        0x15 => "pl",
        0x1f => "tr",
        _ => "",
    }
}

pub fn read(data: &[u8]) -> Option<(BookMeta, Option<Vec<u8>>)> {
    Mobi::parse(data).map(|m| m.meta())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn build() -> Vec<u8> {
        let mut r0 = vec![0u8; 16];
        let mut mobi = b"MOBI".to_vec();
        mobi.extend(232u32.to_be_bytes());
        mobi.extend([0u8; 4]);
        mobi.extend(65001u32.to_be_bytes());
        mobi.resize(232, 0);
        mobi[92..96].copy_from_slice(&1u32.to_be_bytes());
        mobi[112..116].copy_from_slice(&0x40u32.to_be_bytes());
        let mut exth = b"EXTH".to_vec();
        let recs: Vec<(u32, Vec<u8>)> = vec![
            (100, b"Ana".to_vec()),
            (503, b"Livro".to_vec()),
            (201, 0u32.to_be_bytes().to_vec()),
        ];
        let body: Vec<u8> = recs
            .iter()
            .flat_map(|(k, v)| {
                let mut x = k.to_be_bytes().to_vec();
                x.extend(((v.len() + 8) as u32).to_be_bytes());
                x.extend(v);
                x
            })
            .collect();
        exth.extend(((body.len() + 12) as u32).to_be_bytes());
        exth.extend((recs.len() as u32).to_be_bytes());
        exth.extend(body);
        r0.extend(mobi);
        r0.extend(exth);
        let img = vec![0xFF, 0xD8, 0xFF, 0xE0];
        let mut out = vec![0u8; 78];
        out[60..68].copy_from_slice(b"BOOKMOBI");
        out[76..78].copy_from_slice(&2u16.to_be_bytes());
        let base = 78 + 16;
        out.extend((base as u32).to_be_bytes());
        out.extend([0u8; 4]);
        out.extend(((base + r0.len()) as u32).to_be_bytes());
        out.extend([0u8; 4]);
        out.extend(r0);
        out.extend(img);
        out
    }

    #[test]
    fn reads_exth_and_cover() {
        let data = build();
        let (m, cover) = read(&data).unwrap();
        assert_eq!(m.title, "Livro");
        assert_eq!(m.authors, vec!["Ana"]);
        assert_eq!(cover.unwrap()[0..2], [0xFF, 0xD8]);
    }
}

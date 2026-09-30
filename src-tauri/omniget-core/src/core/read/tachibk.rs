//! Mihon / Tachiyomi backups (`.tachibk`, `.proto.gz`): gzip + protobuf.
//! Read so a library can move into OmniGet: followed series with their
//! source URL, categories and read marks per chapter.

use std::io::Read;

use serde::Serialize;

use super::protobuf::{fields, numbers, strings, Field, Msg};

#[derive(Debug, Clone, Serialize, Default, PartialEq)]
pub struct BackupChapter {
    pub url: String,
    pub name: String,
    pub read: bool,
    pub bookmark: bool,
    pub last_page: u64,
    pub number: Option<f32>,
}

#[derive(Debug, Clone, Serialize, Default, PartialEq)]
pub struct BackupManga {
    pub source: u64,
    pub source_name: String,
    pub url: String,
    pub title: String,
    pub author: String,
    pub description: String,
    pub genres: Vec<String>,
    pub cover: String,
    pub favorite: bool,
    pub categories: Vec<String>,
    pub chapters: Vec<BackupChapter>,
    pub notes: String,
}

impl BackupManga {
    /// The MangaDex id when this came from any MangaDex extension
    /// (their URLs are `/manga/<uuid>`).
    pub fn mangadex_id(&self) -> Option<String> {
        let id = self.url.strip_prefix("/manga/")?.trim_end_matches('/');
        (id.len() == 36 && id.chars().filter(|c| *c == '-').count() == 4).then(|| id.to_string())
    }
}

pub fn parse(data: &[u8]) -> Result<Vec<BackupManga>, String> {
    let raw = if data.starts_with(&[0x1f, 0x8b]) {
        let mut out = Vec::new();
        flate2::read::GzDecoder::new(data)
            .read_to_end(&mut out)
            .map_err(|e| format!("not a Mihon backup: {e}"))?;
        out
    } else {
        data.to_vec()
    };
    let root = fields(&raw);
    // Categories: 1 name, 2 order; mangas point at them by order.
    let cats: Vec<(u64, String)> = root.msgs(2).iter().map(|c| (c.num(2), c.str(1))).collect();
    let sources: Vec<(u64, String)> = root
        .msgs(101)
        .iter()
        .map(|s| (s.num(2), s.str(1)))
        .collect();
    let mut out = Vec::new();
    for m in root.msgs(1) {
        let source = m.num(1);
        let chapters = m
            .msgs(16)
            .iter()
            .map(|c| BackupChapter {
                url: c.str(1),
                name: c.str(2),
                read: c.num(4) != 0,
                bookmark: c.num(5) != 0,
                last_page: c.num(6),
                number: c
                    .get(&9)
                    .and_then(|v| v.first())
                    .and_then(Field::as_f32)
                    .filter(|n| *n >= 0.0),
            })
            .collect();
        let categories = numbers(&m, 17)
            .into_iter()
            .filter_map(|order| {
                cats.iter()
                    .find(|(o, _)| *o == order)
                    .map(|(_, n)| n.clone())
            })
            .collect();
        out.push(BackupManga {
            source,
            source_name: sources
                .iter()
                .find(|(id, _)| *id == source)
                .map(|(_, n)| n.clone())
                .unwrap_or_default(),
            url: m.str(2),
            title: m.str(3),
            author: m.str(5),
            description: m.str(6),
            genres: strings(&m, 7),
            cover: m.str(9),
            // Absent means true in the schema.
            favorite: !m.has(100) || m.num(100) != 0,
            categories,
            chapters,
            notes: m.str(110),
        });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn varint(mut v: u64, out: &mut Vec<u8>) {
        loop {
            let b = (v & 0x7f) as u8;
            v >>= 7;
            if v == 0 {
                out.push(b);
                break;
            }
            out.push(b | 0x80);
        }
    }
    fn bytes(tag: u32, d: &[u8], out: &mut Vec<u8>) {
        varint(((tag as u64) << 3) | 2, out);
        varint(d.len() as u64, out);
        out.extend_from_slice(d);
    }
    fn num(tag: u32, v: u64, out: &mut Vec<u8>) {
        varint((tag as u64) << 3, out);
        varint(v, out);
    }

    #[test]
    fn parses_gzipped_backup() {
        let mut ch = Vec::new();
        bytes(1, b"/chapter/abc", &mut ch);
        bytes(2, b"Ch. 1", &mut ch);
        num(4, 1, &mut ch);
        varint((9 << 3) | 5, &mut ch);
        ch.extend_from_slice(&1.5f32.to_le_bytes());
        let mut m = Vec::new();
        num(1, 42, &mut m);
        bytes(2, b"/manga/0a1b2c3d-1111-2222-3333-444455556666", &mut m);
        bytes(3, b"Title", &mut m);
        bytes(16, &ch, &mut m);
        num(17, 0, &mut m);
        let mut cat = Vec::new();
        bytes(1, b"Reading", &mut cat);
        num(2, 0, &mut cat);
        let mut root = Vec::new();
        bytes(1, &m, &mut root);
        bytes(2, &cat, &mut root);
        let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        gz.write_all(&root).unwrap();
        let list = parse(&gz.finish().unwrap()).unwrap();
        assert_eq!(list.len(), 1);
        let b = &list[0];
        assert_eq!(b.title, "Title");
        assert!(b.favorite);
        assert_eq!(b.categories, vec!["Reading"]);
        assert_eq!(b.chapters[0].number, Some(1.5));
        assert!(b.chapters[0].read);
        assert_eq!(
            b.mangadex_id().as_deref(),
            Some("0a1b2c3d-1111-2222-3333-444455556666")
        );
    }
}

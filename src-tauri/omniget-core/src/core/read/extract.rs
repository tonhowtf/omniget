use std::io::Read;
use std::path::Path;

use super::comic::{self, ComicInfo};
use super::epub::{self, BookMeta};
use super::format::BookFormat;
use super::{fb2, mobi};

pub struct Extracted {
    pub format: BookFormat,
    pub meta: BookMeta,
    pub cover: Option<Vec<u8>>,
}

pub fn title_from_name(p: &Path) -> String {
    let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
    let stem = name
        .trim_end_matches(".kepub.epub")
        .trim_end_matches(".fb2.zip");
    let stem = Path::new(stem)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or(stem);
    stem.replace('_', " ").trim().to_string()
}

pub fn comic_meta(ci: &ComicInfo, pages: usize) -> BookMeta {
    let mut m = BookMeta {
        title: ci.title.clone(),
        series: ci.series.clone(),
        series_index: ci.number.trim().parse().ok(),
        description: ci.summary.clone(),
        publisher: ci.publisher.clone(),
        language: ci.language_iso.clone(),
        rtl: ci.rtl(),
        page_count: Some(pages as u32),
        ..Default::default()
    };
    for w in [&ci.writer, &ci.penciller] {
        for a in w.split(',').map(str::trim).filter(|a| !a.is_empty()) {
            if !m.authors.iter().any(|x| x == a) {
                m.authors.push(a.to_string());
            }
        }
    }
    m.subjects = ci
        .genre
        .split(',')
        .chain(ci.tags.split(','))
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect();
    if let Some(y) = ci.year {
        m.published = match (ci.month, ci.day) {
            (Some(mo), Some(d)) => format!("{y:04}-{mo:02}-{d:02}"),
            (Some(mo), None) => format!("{y:04}-{mo:02}"),
            _ => y.to_string(),
        };
    }
    if !ci.gtin.is_empty() {
        m.identifiers.push(("isbn".into(), ci.gtin.clone()));
    }
    m
}

pub fn extract(path: &Path) -> Result<Extracted, String> {
    let format = if path.is_dir() {
        BookFormat::Folder
    } else {
        match BookFormat::from_path(path) {
            Some(f) => f,
            None => {
                let mut head = [0u8; 128];
                let n = std::fs::File::open(path)
                    .and_then(|mut f| f.read(&mut head))
                    .map_err(|e| e.to_string())?;
                BookFormat::sniff(&head[..n]).ok_or("unknown format")?
            }
        }
    };
    let (mut meta, cover) = match format {
        BookFormat::Epub | BookFormat::Kepub => {
            let f = std::fs::File::open(path).map_err(|e| e.to_string())?;
            let meta = epub::read(f)?;
            let cover = meta.cover.as_ref().and_then(|c| {
                let f = std::fs::File::open(path).ok()?;
                let mut z = zip::ZipArchive::new(f).ok()?;
                epub::read_entry_bytes(&mut z, c)
            });
            (meta, cover)
        }
        BookFormat::Cbz => {
            let f = std::fs::File::open(path).map_err(|e| e.to_string())?;
            let mut z = zip::ZipArchive::new(f).map_err(|e| e.to_string())?;
            let pages = comic::zip_pages(&mut z);
            let meta = comic::zip_comic_info(&mut z)
                .map(|ci| comic_meta(&ci, pages.len()))
                .unwrap_or(BookMeta {
                    page_count: Some(pages.len() as u32),
                    ..Default::default()
                });
            let cover = pages
                .first()
                .and_then(|p| epub::read_entry_bytes(&mut z, p));
            (meta, cover)
        }
        BookFormat::Folder => {
            let pages = comic::folder_pages(path);
            let ci = std::fs::read_to_string(path.join("ComicInfo.xml"))
                .ok()
                .map(|s| ComicInfo::parse(&s));
            let meta = ci.map(|c| comic_meta(&c, pages.len())).unwrap_or(BookMeta {
                page_count: Some(pages.len() as u32),
                ..Default::default()
            });
            let cover = pages.first().and_then(|p| std::fs::read(path.join(p)).ok());
            (meta, cover)
        }
        BookFormat::Mobi | BookFormat::Azw3 => {
            let data = std::fs::read(path).map_err(|e| e.to_string())?;
            mobi::read(&data).unwrap_or_default()
        }
        BookFormat::Fb2 => {
            let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
            fb2::read(&String::from_utf8_lossy(&bytes))
        }
        BookFormat::Fbz => {
            let f = std::fs::File::open(path).map_err(|e| e.to_string())?;
            let mut z = zip::ZipArchive::new(f).map_err(|e| e.to_string())?;
            let name = (0..z.len())
                .find_map(|i| {
                    let n = z.by_index(i).ok()?.name().to_string();
                    n.ends_with(".fb2").then_some(n)
                })
                .ok_or("no .fb2 inside")?;
            let bytes = epub::read_entry_bytes(&mut z, &name).ok_or("unreadable fb2")?;
            fb2::read(&String::from_utf8_lossy(&bytes))
        }
        _ => (BookMeta::default(), None),
    };
    if meta.title.trim().is_empty() {
        meta.title = title_from_name(path);
    }
    if meta.series.is_empty() && format.is_comic() {
        if let Some(parent) = path
            .parent()
            .and_then(|p| p.file_name())
            .and_then(|n| n.to_str())
        {
            meta.series = parent.to_string();
        }
        if meta.series_index.is_none() {
            let (ch, vol) = comic::parse_chapter_number(&title_from_name(path));
            meta.series_index = ch.or(vol);
        }
    }
    Ok(Extracted {
        format,
        meta,
        cover,
    })
}

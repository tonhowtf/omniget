use std::io::{Read, Seek, Write};
use std::path::Path;

use serde::{Deserialize, Serialize};

use super::format::{is_image_name, natural_cmp};
use super::xml;

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct ComicInfo {
    pub title: String,
    pub series: String,
    pub number: String,
    pub count: Option<i64>,
    pub volume: Option<i64>,
    pub summary: String,
    pub year: Option<i64>,
    pub month: Option<i64>,
    pub day: Option<i64>,
    pub writer: String,
    pub penciller: String,
    pub inker: String,
    pub colorist: String,
    pub letterer: String,
    pub cover_artist: String,
    pub editor: String,
    pub translator: String,
    pub publisher: String,
    pub genre: String,
    pub tags: String,
    pub web: String,
    pub page_count: Option<i64>,
    pub language_iso: String,
    pub format: String,
    pub black_and_white: String,
    pub manga: String,
    pub characters: String,
    pub teams: String,
    pub locations: String,
    pub story_arc: String,
    pub series_group: String,
    pub age_rating: String,
    pub community_rating: Option<f64>,
    pub gtin: String,
}

impl ComicInfo {
    pub fn rtl(&self) -> bool {
        self.manga.eq_ignore_ascii_case("YesAndRightToLeft")
    }

    pub fn parse(text: &str) -> Self {
        let root = xml::parse(text);
        let Some(ci) = root.find("ComicInfo") else {
            return Self::default();
        };
        let s = |k: &str| ci.child_text(k).unwrap_or_default();
        let i = |k: &str| ci.child_text(k).and_then(|v| v.trim().parse().ok());
        Self {
            title: s("Title"),
            series: s("Series"),
            number: s("Number"),
            count: i("Count"),
            volume: i("Volume"),
            summary: s("Summary"),
            year: i("Year"),
            month: i("Month"),
            day: i("Day"),
            writer: s("Writer"),
            penciller: s("Penciller"),
            inker: s("Inker"),
            colorist: s("Colorist"),
            letterer: s("Letterer"),
            cover_artist: s("CoverArtist"),
            editor: s("Editor"),
            translator: s("Translator"),
            publisher: s("Publisher"),
            genre: s("Genre"),
            tags: s("Tags"),
            web: s("Web"),
            page_count: i("PageCount"),
            language_iso: s("LanguageISO"),
            format: s("Format"),
            black_and_white: s("BlackAndWhite"),
            manga: s("Manga"),
            characters: s("Characters"),
            teams: s("Teams"),
            locations: s("Locations"),
            story_arc: s("StoryArc"),
            series_group: s("SeriesGroup"),
            age_rating: s("AgeRating"),
            community_rating: ci
                .child_text("CommunityRating")
                .and_then(|v| v.parse().ok()),
            gtin: s("GTIN"),
        }
    }

    pub fn to_xml(&self) -> String {
        let mut o = String::from(
            "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<ComicInfo xmlns:xsd=\"http://www.w3.org/2001/XMLSchema\" xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\">\n",
        );
        let mut s = |k: &str, v: &str| {
            if !v.is_empty() {
                o.push_str(&format!("  <{k}>{}</{k}>\n", xml::escape(v)));
            }
        };
        s("Title", &self.title);
        s("Series", &self.series);
        s("Number", &self.number);
        s("Count", &opt(self.count));
        s("Volume", &opt(self.volume));
        s("Summary", &self.summary);
        s("Year", &opt(self.year));
        s("Month", &opt(self.month));
        s("Day", &opt(self.day));
        s("Writer", &self.writer);
        s("Penciller", &self.penciller);
        s("Inker", &self.inker);
        s("Colorist", &self.colorist);
        s("Letterer", &self.letterer);
        s("CoverArtist", &self.cover_artist);
        s("Editor", &self.editor);
        s("Translator", &self.translator);
        s("Publisher", &self.publisher);
        s("Genre", &self.genre);
        s("Tags", &self.tags);
        s("Web", &self.web);
        s("PageCount", &opt(self.page_count));
        s("LanguageISO", &self.language_iso);
        s("Format", &self.format);
        s("BlackAndWhite", &self.black_and_white);
        s("Manga", &self.manga);
        s("Characters", &self.characters);
        s("Teams", &self.teams);
        s("Locations", &self.locations);
        s("StoryArc", &self.story_arc);
        s("SeriesGroup", &self.series_group);
        s("AgeRating", &self.age_rating);
        s(
            "CommunityRating",
            &self
                .community_rating
                .map(|v| v.to_string())
                .unwrap_or_default(),
        );
        s("GTIN", &self.gtin);
        o.push_str("</ComicInfo>\n");
        o
    }
}

fn opt(v: Option<i64>) -> String {
    v.map(|x| x.to_string()).unwrap_or_default()
}

pub fn zip_pages<R: Read + Seek>(zip: &mut zip::ZipArchive<R>) -> Vec<String> {
    let mut names: Vec<String> = (0..zip.len())
        .filter_map(|i| {
            let f = zip.by_index(i).ok()?;
            let n = f.name().to_string();
            let base = n.rsplit('/').next().unwrap_or(&n);
            (f.is_file()
                && is_image_name(&n)
                && !n.starts_with("__MACOSX")
                && !base.starts_with('.'))
            .then_some(n)
        })
        .collect();
    names.sort_by(|a, b| natural_cmp(a, b));
    names
}

pub fn zip_comic_info<R: Read + Seek>(zip: &mut zip::ZipArchive<R>) -> Option<ComicInfo> {
    let name = (0..zip.len()).find_map(|i| {
        let n = zip.by_index(i).ok()?.name().to_string();
        n.rsplit('/')
            .next()
            .is_some_and(|b| b.eq_ignore_ascii_case("ComicInfo.xml"))
            .then_some(n)
    })?;
    let mut s = String::new();
    zip.by_name(&name).ok()?.read_to_string(&mut s).ok()?;
    Some(ComicInfo::parse(&s))
}

pub fn folder_pages(dir: &Path) -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(dir)
        .map(|rd| {
            rd.filter_map(|e| e.ok())
                .filter(|e| e.path().is_file())
                .filter_map(|e| e.file_name().to_str().map(str::to_string))
                .filter(|n| is_image_name(n) && !n.starts_with('.'))
                .collect()
        })
        .unwrap_or_default();
    v.sort_by(|a, b| natural_cmp(a, b));
    v
}

pub fn write_cbz(
    out: &Path,
    pages: &[(String, Vec<u8>)],
    info: Option<&ComicInfo>,
) -> Result<(), String> {
    if let Some(dir) = out.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let tmp = out.with_extension("cbz.part");
    let f = std::fs::File::create(&tmp).map_err(|e| e.to_string())?;
    let mut w = zip::ZipWriter::new(f);
    let stored =
        zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    let width = pages.len().to_string().len().max(3);
    for (i, (name, bytes)) in pages.iter().enumerate() {
        let ext = name
            .rsplit('.')
            .next()
            .filter(|e| e.len() <= 4)
            .unwrap_or("jpg");
        w.start_file(format!("{:0width$}.{ext}", i + 1), stored)
            .map_err(|e| e.to_string())?;
        w.write_all(bytes).map_err(|e| e.to_string())?;
    }
    if let Some(ci) = info {
        let deflated = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);
        w.start_file("ComicInfo.xml", deflated)
            .map_err(|e| e.to_string())?;
        w.write_all(ci.to_xml().as_bytes())
            .map_err(|e| e.to_string())?;
    }
    w.finish().map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, out).map_err(|e| e.to_string())
}

pub fn parse_chapter_number(name: &str) -> (Option<f64>, Option<f64>) {
    static VOL: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    static CH: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    static NUM: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let lower = name.to_lowercase();
    let vol_re = VOL.get_or_init(|| {
        regex::Regex::new(r"\b(?:volume|vol|v)[.\s_#-]*(\d+(?:\.\d+)?)").expect("static regex")
    });
    let ch_re = CH.get_or_init(|| {
        regex::Regex::new(
            r"(?:\b(?:chapter|chap|ch|c|episode|ep|cap[ií]tulo|cap)|#)[.\s_#-]*(\d+(?:\.\d+)?)",
        )
        .expect("static regex")
    });
    let num_re = NUM.get_or_init(|| regex::Regex::new(r"(\d+(?:\.\d+)?)").expect("static regex"));
    let vol_m = vol_re.captures(&lower);
    let vol = vol_m.as_ref().and_then(|c| c[1].parse().ok());
    let vol_span = vol_m.and_then(|c| c.get(0)).map(|m| m.range());
    let ch = ch_re
        .captures(&lower)
        .and_then(|c| c[1].parse().ok())
        .or_else(|| {
            num_re
                .find_iter(&lower)
                .filter(|m| vol_span.as_ref().is_none_or(|r| !r.contains(&m.start())))
                .find_map(|m| m.as_str().parse().ok())
        });
    (ch, vol)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn comicinfo_roundtrip() {
        let ci = ComicInfo {
            title: "A <b>".into(),
            series: "One Piece".into(),
            number: "1000".into(),
            manga: "YesAndRightToLeft".into(),
            page_count: Some(20),
            ..Default::default()
        };
        let back = ComicInfo::parse(&ci.to_xml());
        assert_eq!(back, ci);
        assert!(back.rtl());
    }

    #[test]
    fn chapter_numbers() {
        assert_eq!(
            parse_chapter_number("Vol.02 Ch.013.5 - x"),
            (Some(13.5), Some(2.0))
        );
        assert_eq!(parse_chapter_number("Chapter 7"), (Some(7.0), None));
        assert_eq!(parse_chapter_number("Naruto 700"), (Some(700.0), None));
    }

    #[test]
    fn cbz_write_and_list() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("a.cbz");
        let pages: Vec<(String, Vec<u8>)> = (0..12)
            .map(|i| (format!("p{i}.png"), vec![i as u8; 4]))
            .collect();
        write_cbz(&out, &pages, Some(&ComicInfo::default())).unwrap();
        let mut z = zip::ZipArchive::new(std::fs::File::open(&out).unwrap()).unwrap();
        let p = zip_pages(&mut z);
        assert_eq!(p.len(), 12);
        assert_eq!(p[0], "001.png");
        assert!(zip_comic_info(&mut z).is_some());
    }
}

use std::io::Read;
use std::path::Path;

use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Hash, Debug, PartialOrd, Ord)]
#[serde(rename_all = "lowercase")]
pub enum Category {
    Data,
    Document,
    Image,
    Cad,
    Audio,
    Video,
}

#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
pub struct FormatInfo {
    pub id: &'static str,
    pub label: &'static str,
    pub category: Category,
    pub extensions: &'static [&'static str],
}

const fn f(
    id: &'static str,
    label: &'static str,
    category: Category,
    extensions: &'static [&'static str],
) -> FormatInfo {
    FormatInfo {
        id,
        label,
        category,
        extensions,
    }
}

pub static FORMATS: &[FormatInfo] = &[
    f("json", "JSON", Category::Data, &["json"]),
    f("jsonl", "JSON Lines", Category::Data, &["jsonl", "ndjson"]),
    f("yaml", "YAML", Category::Data, &["yaml", "yml"]),
    f("toml", "TOML", Category::Data, &["toml"]),
    f("csv", "CSV", Category::Data, &["csv"]),
    f("tsv", "TSV", Category::Data, &["tsv", "tab"]),
    f("xml", "XML", Category::Data, &["xml"]),
    f("plist", "Property List", Category::Data, &["plist"]),
    f("md", "Markdown", Category::Document, &["md", "markdown"]),
    f(
        "html",
        "HTML",
        Category::Document,
        &["html", "htm", "xhtml"],
    ),
    f("txt", "Plain text", Category::Document, &["txt", "text"]),
    f("docx", "Word", Category::Document, &["docx"]),
    f("odt", "OpenDocument Text", Category::Document, &["odt"]),
    f("rtf", "Rich Text", Category::Document, &["rtf"]),
    f("epub", "EPUB", Category::Document, &["epub"]),
    f("pdf", "PDF", Category::Document, &["pdf"]),
    f("tex", "LaTeX", Category::Document, &["tex", "latex"]),
    f("rst", "reStructuredText", Category::Document, &["rst"]),
    f("org", "Org", Category::Document, &["org"]),
    f("png", "PNG", Category::Image, &["png"]),
    f("jpeg", "JPG", Category::Image, &["jpg", "jpeg", "jfif"]),
    f("webp", "WebP", Category::Image, &["webp"]),
    f("gif", "GIF", Category::Image, &["gif"]),
    f("bmp", "BMP", Category::Image, &["bmp"]),
    f("ico", "ICO", Category::Image, &["ico"]),
    f("tiff", "TIFF", Category::Image, &["tiff", "tif"]),
    f("tga", "TGA", Category::Image, &["tga"]),
    f("svg", "SVG", Category::Image, &["svg"]),
    f("avif", "AVIF", Category::Image, &["avif"]),
    f("heic", "HEIC", Category::Image, &["heic", "heif", "hif"]),
    f("jxl", "JPEG XL", Category::Image, &["jxl"]),
    f("psd", "Photoshop", Category::Image, &["psd", "psb"]),
    f(
        "raw",
        "Camera RAW",
        Category::Image,
        &[
            "cr2", "cr3", "nef", "nrw", "arw", "dng", "raf", "orf", "rw2", "pef", "srw",
        ],
    ),
    f("layers", "PNG layers", Category::Image, &[]),
    f("dxf", "DXF", Category::Cad, &["dxf"]),
    f("dwg", "DWG", Category::Cad, &["dwg"]),
    f("mp3", "MP3", Category::Audio, &["mp3"]),
    f("m4a", "M4A", Category::Audio, &["m4a"]),
    f("aac", "AAC", Category::Audio, &["aac"]),
    f("wav", "WAV", Category::Audio, &["wav"]),
    f("flac", "FLAC", Category::Audio, &["flac"]),
    f("ogg", "Ogg Vorbis", Category::Audio, &["ogg", "oga"]),
    f("opus", "Opus", Category::Audio, &["opus"]),
    f("mp4", "MP4", Category::Video, &["mp4", "m4v"]),
    f("mkv", "MKV", Category::Video, &["mkv"]),
    f("webm", "WebM", Category::Video, &["webm"]),
    f("mov", "MOV", Category::Video, &["mov"]),
    f("avi", "AVI", Category::Video, &["avi"]),
];

pub fn by_id(id: &str) -> Option<&'static FormatInfo> {
    FORMATS.iter().find(|f| f.id == id)
}

pub fn by_extension(ext: &str) -> Option<&'static FormatInfo> {
    let ext = ext.to_ascii_lowercase();
    FORMATS
        .iter()
        .find(|f| f.extensions.iter().any(|e| *e == ext))
}

pub fn category_of(id: &str) -> Option<Category> {
    by_id(id).map(|f| f.category)
}

pub fn is_folder(id: &str) -> bool {
    id == "layers"
}

pub fn output_name(stem: &str, id: &str, n: u32) -> String {
    let base = if is_folder(id) {
        format!("{stem} layers")
    } else {
        stem.to_string()
    };
    let numbered = if n == 0 {
        base
    } else {
        format!("{base} ({n})")
    };
    if is_folder(id) {
        numbered
    } else {
        format!("{numbered}.{}", extension_of(id))
    }
}

pub fn extension_of(id: &str) -> &'static str {
    by_id(id)
        .and_then(|f| f.extensions.first().copied())
        .unwrap_or("bin")
}

pub fn detect(path: &Path) -> Option<&'static FormatInfo> {
    if let Some(found) = path
        .extension()
        .and_then(|e| e.to_str())
        .and_then(by_extension)
    {
        return Some(found);
    }
    let mut head = [0u8; 64];
    let n = std::fs::File::open(path)
        .and_then(|mut file| file.read(&mut head))
        .ok()?;
    let kind = sniff(&head[..n])?;
    if kind == "zip" {
        return sniff_zip(path);
    }
    by_id(kind)
}

pub fn sniff(head: &[u8]) -> Option<&'static str> {
    if head.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("png")
    } else if head.starts_with(&[0xFF, 0xD8, 0xFF]) {
        Some("jpeg")
    } else if head.starts_with(b"GIF87a") || head.starts_with(b"GIF89a") {
        Some("gif")
    } else if head.len() >= 12 && &head[..4] == b"RIFF" && &head[8..12] == b"WEBP" {
        Some("webp")
    } else if head.starts_with(b"BM") && head.len() >= 14 {
        Some("bmp")
    } else if let Some(kind) = sniff_ftyp(head) {
        Some(kind)
    } else if head.starts_with(&[0xFF, 0x0A]) || head.starts_with(b"\0\0\0\x0cJXL \r\n\x87\n") {
        Some("jxl")
    } else if head.starts_with(b"8BPS") {
        Some("psd")
    } else if head.starts_with(b"FUJIFILMCCD-RAW")
        || head.starts_with(b"IIRO")
        || head.starts_with(b"IIRS")
        || head.starts_with(b"MMOR")
        || head.starts_with(b"IIU\0")
    {
        Some("raw")
    } else if head.starts_with(b"II*\0") || head.starts_with(b"MM\0*") {
        Some("tiff")
    } else if head.starts_with(b"%PDF") {
        Some("pdf")
    } else if head.starts_with(b"PK\x03\x04") {
        Some("zip")
    } else if head.len() >= 6
        && head.starts_with(b"AC10")
        && head[4..6].iter().all(u8::is_ascii_digit)
    {
        Some("dwg")
    } else if is_dxf(head) {
        Some("dxf")
    } else {
        None
    }
}

fn sniff_ftyp(head: &[u8]) -> Option<&'static str> {
    if head.len() < 16 || &head[4..8] != b"ftyp" {
        return None;
    }
    let major = &head[8..12];
    match major {
        b"crx " => return Some("raw"),
        b"avif" | b"avis" => return Some("avif"),
        _ => {}
    }
    let size = u32::from_be_bytes([head[0], head[1], head[2], head[3]]) as usize;
    let compatible = head[16..size.clamp(16, head.len())].chunks_exact(4);
    std::iter::once(major)
        .chain(compatible)
        .any(|b| matches!(b, b"heic" | b"heix" | b"heim" | b"heis" | b"hevc" | b"hevx"))
        .then_some("heic")
}

fn is_dxf(head: &[u8]) -> bool {
    if head.starts_with(b"AutoCAD Binary DXF") {
        return true;
    }
    let text = String::from_utf8_lossy(head);
    let mut tokens = text.split_whitespace();
    tokens.next() == Some("0") && tokens.next() == Some("SECTION")
}

fn sniff_zip(path: &Path) -> Option<&'static FormatInfo> {
    let file = std::fs::File::open(path).ok()?;
    let mut zip = zip::ZipArchive::new(file).ok()?;
    if let Ok(mut entry) = zip.by_name("mimetype") {
        let mut mime = String::new();
        let _ = entry.read_to_string(&mut mime);
        let mime = mime.trim();
        if mime == "application/epub+zip" {
            return by_id("epub");
        }
        if mime == "application/vnd.oasis.opendocument.text" {
            return by_id("odt");
        }
    }
    if zip.by_name("word/document.xml").is_ok() {
        return by_id("docx");
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_by_extension_case_insensitive() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("Photo.JPG");
        std::fs::write(&p, b"x").unwrap();
        assert_eq!(detect(&p).unwrap().id, "jpeg");
        assert_eq!(by_extension("YML").unwrap().id, "yaml");
        assert_eq!(extension_of("jpeg"), "jpg");
    }

    #[test]
    fn detects_by_magic_bytes() {
        let dir = tempfile::tempdir().unwrap();
        let png = dir.path().join("noext");
        std::fs::write(&png, b"\x89PNG\r\n\x1a\nrest").unwrap();
        assert_eq!(detect(&png).unwrap().id, "png");
        let pdf = dir.path().join("file.unknown");
        std::fs::write(&pdf, b"%PDF-1.7").unwrap();
        assert_eq!(detect(&pdf).unwrap().id, "pdf");
        assert_eq!(sniff(b"RIFF\0\0\0\0WEBPVP8 "), Some("webp"));
        assert_eq!(sniff(b"hello"), None);
    }

    #[test]
    fn sniffs_photo_and_drawing_formats() {
        let ftyp = |major: &[u8; 4], compat: &[u8]| {
            let mut head = 0u32.to_be_bytes().to_vec();
            head.extend(b"ftyp");
            head.extend(major);
            head.extend([0, 0, 0, 0]);
            head.extend(compat);
            let len = head.len() as u32;
            head[..4].copy_from_slice(&len.to_be_bytes());
            head
        };
        assert_eq!(sniff(&ftyp(b"heic", b"mif1heic")), Some("heic"));
        assert_eq!(sniff(&ftyp(b"mif1", b"mif1heic")), Some("heic"));
        assert_eq!(sniff(&ftyp(b"mif1", b"mif1avif")), None);
        assert_eq!(sniff(&ftyp(b"avif", b"mif1")), Some("avif"));
        assert_eq!(sniff(&ftyp(b"crx ", b"isom")), Some("raw"));
        assert_eq!(sniff(&ftyp(b"isom", b"mp41")), None);
        assert_eq!(sniff(&[0xFF, 0x0A, 0, 0]), Some("jxl"));
        assert_eq!(sniff(b"\0\0\0\x0cJXL \r\n\x87\nrest"), Some("jxl"));
        assert_eq!(sniff(b"8BPS\0\x01"), Some("psd"));
        assert_eq!(sniff(b"FUJIFILMCCD-RAW 0201"), Some("raw"));
        assert_eq!(sniff(b"AC1032\0\0"), Some("dwg"));
        assert_eq!(sniff(b"AC10xx"), None);
        assert_eq!(sniff(b"  0\r\nSECTION\r\n  2\r\nHEADER"), Some("dxf"));
        assert_eq!(sniff(b"0\nSECTIONAL"), None);
        assert_eq!(by_extension("CR3").unwrap().id, "raw");
        assert_eq!(by_extension("psb").unwrap().id, "psd");
        assert_eq!(by_extension("HEIF").unwrap().id, "heic");
    }

    #[test]
    fn folder_outputs_are_named_without_extension() {
        assert!(is_folder("layers"));
        assert_eq!(output_name("art", "layers", 0), "art layers");
        assert_eq!(output_name("art", "layers", 2), "art layers (2)");
        assert_eq!(output_name("art", "jpeg", 1), "art (1).jpg");
    }

    #[test]
    fn detects_docx_inside_zip() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("doc");
        let file = std::fs::File::create(&p).unwrap();
        let mut zip = zip::ZipWriter::new(file);
        zip.start_file(
            "word/document.xml",
            zip::write::SimpleFileOptions::default(),
        )
        .unwrap();
        std::io::Write::write_all(&mut zip, b"<w:document/>").unwrap();
        zip.finish().unwrap();
        assert_eq!(detect(&p).unwrap().id, "docx");
    }
}

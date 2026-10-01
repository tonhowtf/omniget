//! Which kind of book a file is, from its extension and, when that is
//! ambiguous or missing, its first bytes.

use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Hash)]
#[serde(rename_all = "lowercase")]
pub enum BookFormat {
    Epub,
    Kepub,
    Mobi,
    Azw3,
    Fb2,
    Fbz,
    Pdf,
    Cbz,
    Cbr,
    Cb7,
    Cbt,
    Txt,
    Md,
    Html,
    Djvu,
    /// A folder of images treated as one comic chapter.
    Folder,
}

impl BookFormat {
    pub fn from_ext(ext: &str) -> Option<Self> {
        Some(match ext.to_ascii_lowercase().as_str() {
            "epub" | "epub3" => Self::Epub,
            "kepub" => Self::Kepub,
            "mobi" | "prc" => Self::Mobi,
            "azw" | "azw3" | "kf8" => Self::Azw3,
            "fb2" => Self::Fb2,
            "fbz" => Self::Fbz,
            "pdf" => Self::Pdf,
            "cbz" => Self::Cbz,
            "cbr" => Self::Cbr,
            "cb7" => Self::Cb7,
            "cbt" => Self::Cbt,
            "txt" | "text" => Self::Txt,
            "md" | "markdown" => Self::Md,
            "html" | "htm" | "xhtml" => Self::Html,
            "djvu" | "djv" => Self::Djvu,
            _ => return None,
        })
    }

    pub fn from_path(p: &Path) -> Option<Self> {
        let name = p.file_name()?.to_str()?.to_ascii_lowercase();
        if name.ends_with(".kepub.epub") {
            return Some(Self::Kepub);
        }
        if name.ends_with(".fb2.zip") {
            return Some(Self::Fbz);
        }
        Self::from_ext(p.extension()?.to_str()?)
    }

    /// Sniffs a format from the first bytes (a download with no extension).
    pub fn sniff(head: &[u8]) -> Option<Self> {
        if head.starts_with(b"%PDF") {
            return Some(Self::Pdf);
        }
        if head.starts_with(b"PK\x03\x04") {
            // EPUB puts an uncompressed `mimetype` first.
            if head.len() > 58 && &head[30..38] == b"mimetype" {
                let tail = &head[38..head.len().min(80)];
                if tail.windows(20).any(|w| w == b"application/epub+zip") {
                    return Some(Self::Epub);
                }
            }
            return Some(Self::Cbz);
        }
        if head.starts_with(b"Rar!") {
            return Some(Self::Cbr);
        }
        if head.starts_with(b"7z\xBC\xAF\x27\x1C") {
            return Some(Self::Cb7);
        }
        if head.len() > 68 && (&head[60..68] == b"BOOKMOBI" || &head[60..68] == b"TEXtREAd") {
            return Some(Self::Mobi);
        }
        if head.starts_with(b"AT&TFORM") {
            return Some(Self::Djvu);
        }
        let s = String::from_utf8_lossy(&head[..head.len().min(512)]);
        if s.contains("<FictionBook") {
            return Some(Self::Fb2);
        }
        None
    }

    pub fn ext(self) -> &'static str {
        match self {
            Self::Epub => "epub",
            Self::Kepub => "kepub.epub",
            Self::Mobi => "mobi",
            Self::Azw3 => "azw3",
            Self::Fb2 => "fb2",
            Self::Fbz => "fbz",
            Self::Pdf => "pdf",
            Self::Cbz => "cbz",
            Self::Cbr => "cbr",
            Self::Cb7 => "cb7",
            Self::Cbt => "cbt",
            Self::Txt => "txt",
            Self::Md => "md",
            Self::Html => "html",
            Self::Djvu => "djvu",
            Self::Folder => "",
        }
    }

    pub fn mime(self) -> &'static str {
        match self {
            Self::Epub | Self::Kepub => "application/epub+zip",
            Self::Mobi => "application/x-mobipocket-ebook",
            Self::Azw3 => "application/vnd.amazon.ebook",
            Self::Fb2 => "application/x-fictionbook+xml",
            Self::Fbz => "application/x-zip-compressed-fb2",
            Self::Pdf => "application/pdf",
            Self::Cbz => "application/vnd.comicbook+zip",
            Self::Cbr => "application/vnd.comicbook-rar",
            Self::Cb7 => "application/x-cb7",
            Self::Cbt => "application/x-cbt",
            Self::Txt => "text/plain; charset=utf-8",
            Self::Md => "text/markdown; charset=utf-8",
            Self::Html => "text/html; charset=utf-8",
            Self::Djvu => "image/vnd.djvu",
            Self::Folder => "inode/directory",
        }
    }

    /// Comics are read page by page as images, everything else reflows.
    pub fn is_comic(self) -> bool {
        matches!(
            self,
            Self::Cbz | Self::Cbr | Self::Cb7 | Self::Cbt | Self::Folder
        )
    }

    /// Formats the in-app reader opens directly (foliate-js or the comic
    /// reader). The rest need a conversion first.
    pub fn readable_in_app(self) -> bool {
        !matches!(self, Self::Djvu | Self::Cbr | Self::Cb7)
    }
}

pub fn is_image_name(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    [
        ".jpg", ".jpeg", ".png", ".webp", ".gif", ".avif", ".bmp", ".jxl",
    ]
    .iter()
    .any(|e| n.ends_with(e))
}

pub fn image_mime(name: &str) -> &'static str {
    let n = name.to_ascii_lowercase();
    if n.ends_with(".png") {
        "image/png"
    } else if n.ends_with(".webp") {
        "image/webp"
    } else if n.ends_with(".gif") {
        "image/gif"
    } else if n.ends_with(".avif") {
        "image/avif"
    } else if n.ends_with(".bmp") {
        "image/bmp"
    } else if n.ends_with(".jxl") {
        "image/jxl"
    } else if n.ends_with(".svg") {
        "image/svg+xml"
    } else {
        "image/jpeg"
    }
}

/// Natural order: `page2` before `page10`, case-insensitive.
pub fn natural_cmp(a: &str, b: &str) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    let (mut ai, mut bi) = (a.chars().peekable(), b.chars().peekable());
    loop {
        match (ai.peek().copied(), bi.peek().copied()) {
            (None, None) => return Ordering::Equal,
            (None, _) => return Ordering::Less,
            (_, None) => return Ordering::Greater,
            (Some(x), Some(y)) if x.is_ascii_digit() && y.is_ascii_digit() => {
                let mut na = String::new();
                while let Some(c) = ai.peek().copied().filter(|c| c.is_ascii_digit()) {
                    na.push(c);
                    ai.next();
                }
                let mut nb = String::new();
                while let Some(c) = bi.peek().copied().filter(|c| c.is_ascii_digit()) {
                    nb.push(c);
                    bi.next();
                }
                let ta = na.trim_start_matches('0');
                let tb = nb.trim_start_matches('0');
                let o = ta.len().cmp(&tb.len()).then_with(|| ta.cmp(tb));
                if o != Ordering::Equal {
                    return o;
                }
            }
            (Some(x), Some(y)) => {
                let o = x.to_lowercase().cmp(y.to_lowercase());
                if o != Ordering::Equal {
                    return o;
                }
                ai.next();
                bi.next();
            }
        }
    }
}

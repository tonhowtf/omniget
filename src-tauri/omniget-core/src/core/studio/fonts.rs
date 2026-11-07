use std::path::{Path, PathBuf};

use ttf_parser::Face;

const MAX_FONT_BYTES: u64 = 96 * 1024 * 1024;

pub fn needed_chars(text: &str) -> Vec<char> {
    let mut chars: Vec<char> = text
        .chars()
        .filter(|c| !c.is_whitespace() && !c.is_control())
        .collect();
    chars.sort_unstable();
    chars.dedup();
    chars
}

pub fn coverage(data: &[u8], chars: &[char]) -> usize {
    Face::parse(data, 0)
        .map(|face| {
            chars
                .iter()
                .filter(|c| face.glyph_index(**c).is_some())
                .count()
        })
        .unwrap_or(0)
}

fn known_candidates() -> Vec<PathBuf> {
    let names: &[&str] = if cfg!(target_os = "macos") {
        &[
            "/System/Library/Fonts/ヒラギノ角ゴシック W6.ttc",
            "/System/Library/Fonts/Hiragino Sans GB.ttc",
            "/System/Library/Fonts/STHeiti Medium.ttc",
            "/System/Library/Fonts/PingFang.ttc",
            "/System/Library/Fonts/AppleSDGothicNeo.ttc",
            "/System/Library/Fonts/Supplemental/Tahoma Bold.ttf",
            "/System/Library/Fonts/GeezaPro.ttc",
            "/System/Library/Fonts/Supplemental/Lao Sangam MN.ttf",
            "/System/Library/Fonts/Supplemental/Lao MN.ttc",
            "/System/Library/Fonts/Supplemental/Arial Unicode.ttf",
            "/Library/Fonts/Arial Unicode.ttf",
        ]
    } else if cfg!(target_os = "windows") {
        &[
            "msyhbd.ttc",
            "msyh.ttc",
            "msjhbd.ttc",
            "msjh.ttc",
            "YuGothB.ttc",
            "meiryob.ttc",
            "malgunbd.ttf",
            "tahomabd.ttf",
            "segoeuib.ttf",
            "LeelaUIb.ttf",
            "LeelawUI.ttf",
            "arialuni.ttf",
        ]
    } else {
        &[
            "/usr/share/fonts/opentype/noto/NotoSansCJK-Bold.ttc",
            "/usr/share/fonts/noto-cjk/NotoSansCJK-Bold.ttc",
            "/usr/share/fonts/google-noto-cjk/NotoSansCJK-Bold.ttc",
            "/usr/share/fonts/truetype/noto/NotoSansArabic-Bold.ttf",
            "/usr/share/fonts/truetype/noto/NotoSansLao-Bold.ttf",
            "/usr/share/fonts/truetype/dejavu/DejaVuSans-Bold.ttf",
        ]
    };
    if cfg!(target_os = "windows") {
        let dir = std::env::var_os("WINDIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("C:\\Windows"))
            .join("Fonts");
        names.iter().map(|n| dir.join(n)).collect()
    } else {
        names.iter().map(PathBuf::from).collect()
    }
}

fn fc_match(chars: &[char]) -> Option<PathBuf> {
    if cfg!(any(target_os = "macos", target_os = "windows")) {
        return None;
    }
    let charset = chars
        .iter()
        .map(|c| format!("{:x}", *c as u32))
        .collect::<Vec<_>>()
        .join(" ");
    let out = crate::core::process::std_command("fc-match")
        .args(["-f", "%{file}"])
        .arg(format!("sans:weight=bold:charset={charset}"))
        .output()
        .ok()?;
    let path = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (out.status.success() && !path.is_empty()).then(|| PathBuf::from(path))
}

fn read_font(path: &Path) -> Option<Vec<u8>> {
    let meta = std::fs::metadata(path).ok()?;
    if !meta.is_file() || meta.len() > MAX_FONT_BYTES {
        return None;
    }
    std::fs::read(path).ok()
}

pub fn pick_from(
    chars: &[char],
    candidates: impl IntoIterator<Item = PathBuf>,
) -> Option<(PathBuf, usize)> {
    let mut best: Option<(PathBuf, usize)> = None;
    for path in candidates {
        let Some(data) = read_font(&path) else {
            continue;
        };
        let n = coverage(&data, chars);
        if n == chars.len() {
            return Some((path, n));
        }
        if n > best.as_ref().map(|b| b.1).unwrap_or(0) {
            best = Some((path, n));
        }
    }
    best
}

pub fn fallback_for(text: &str, bundled: &[u8]) -> Option<PathBuf> {
    let chars = needed_chars(text);
    let own = coverage(bundled, &chars);
    if own == chars.len() {
        return None;
    }
    let mut candidates = known_candidates();
    if let Some(p) = fc_match(&chars) {
        candidates.insert(0, p);
    }
    let mut best = pick_from(&chars, candidates);
    if best.as_ref().map(|b| b.1) != Some(chars.len()) {
        let dirs = crate::core::fonts::scan::system_font_dirs();
        let files = crate::core::fonts::scan::font_files(&dirs);
        if let Some(found) = pick_from(&chars, files) {
            if found.1 > best.as_ref().map(|b| b.1).unwrap_or(0) {
                best = Some(found);
            }
        }
    }
    best.filter(|b| b.1 > own).map(|b| b.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collects_distinct_visible_chars() {
        assert_eq!(needed_chars("a b\nab\u{0}"), vec!['a', 'b']);
    }

    #[test]
    fn bundled_font_covers_latin_but_not_cjk() {
        let latin = needed_chars("Olá, ÇÃO — Ελλάδα Привет");
        assert_eq!(coverage(super::super::FONT_BOLD, &latin), latin.len());
        let cjk = needed_chars("日本語のタイトル");
        assert_eq!(coverage(super::super::FONT_BOLD, &cjk), 0);
        assert!(fallback_for("Hello there", super::super::FONT_BOLD).is_none());
    }

    #[test]
    fn picks_the_first_font_that_covers_everything() {
        let dir = tempfile::tempdir().unwrap();
        let bold = dir.path().join("bold.ttf");
        std::fs::write(&bold, super::super::FONT_BOLD).unwrap();
        let junk = dir.path().join("junk.ttf");
        std::fs::write(&junk, b"not a font").unwrap();
        let chars = needed_chars("Hi");
        let got = pick_from(&chars, [dir.path().join("missing.ttf"), junk, bold.clone()]);
        assert_eq!(got, Some((bold, 2)));
        assert_eq!(
            pick_from(&needed_chars("日本"), [dir.path().join("bold.ttf")]),
            None
        );
    }

    #[test]
    fn finds_a_system_font_for_cjk_when_one_exists() {
        let text = "日本語のタイトル";
        if let Some(path) = fallback_for(text, super::super::FONT_BOLD) {
            let data = std::fs::read(&path).unwrap();
            assert!(coverage(&data, &needed_chars(text)) > 0);
        }
        if cfg!(target_os = "macos")
            && Path::new("/System/Library/Fonts/Hiragino Sans GB.ttc").exists()
        {
            assert!(fallback_for(text, super::super::FONT_BOLD).is_some());
        }
    }
}

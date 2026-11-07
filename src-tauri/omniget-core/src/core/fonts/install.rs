use std::io::Write;
use std::path::{Path, PathBuf};

use serde::Serialize;
use tokio_util::sync::CancellationToken;

use super::catalog::{self, FontDetails, LicenseClass};

pub const GOOGLE_CSS: &str = "https://fonts.googleapis.com/css2";
const LEGACY_AGENT: &str = "Mozilla/4.0 (compatible; OmniGet)";
pub const MAX_FILE_BYTES: u64 = 40 * 1024 * 1024;

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct PlannedFile {
    pub url: String,
    pub file_name: String,
    pub weight: u16,
    pub style: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileState {
    Downloading,
    Saved,
    Skipped,
    Failed,
}

impl FileState {
    pub fn as_str(self) -> &'static str {
        match self {
            FileState::Downloading => "downloading",
            FileState::Saved => "saved",
            FileState::Skipped => "skipped",
            FileState::Failed => "failed",
        }
    }
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct FailedFile {
    pub file_name: String,
    pub error: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Default)]
pub struct InstallOutcome {
    pub dir: String,
    pub installed: bool,
    pub saved: Vec<String>,
    pub skipped: Vec<String>,
    pub failed: Vec<FailedFile>,
    pub cancelled: bool,
    pub cache_refreshed: bool,
    pub registered: usize,
    pub needs_restart: bool,
    pub coverage: String,
}

fn file_stem(family: &str) -> String {
    let stem: String = family
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .collect();
    if stem.is_empty() {
        "Font".to_string()
    } else {
        stem
    }
}

fn title_case(text: &str) -> String {
    text.split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(|w| {
            let mut chars = w.chars();
            match chars.next() {
                Some(first) => first.to_ascii_uppercase().to_string() + chars.as_str(),
                None => String::new(),
            }
        })
        .collect()
}

pub fn style_label(weight: u16, italic: bool) -> String {
    let base = match weight {
        0..=149 => "Thin",
        150..=249 => "ExtraLight",
        250..=349 => "Light",
        350..=449 => "Regular",
        450..=549 => "Medium",
        550..=649 => "SemiBold",
        650..=749 => "Bold",
        750..=849 => "ExtraBold",
        _ => "Black",
    };
    match (base, italic) {
        ("Regular", true) => "Italic".to_string(),
        (b, true) => format!("{b}Italic"),
        (b, false) => b.to_string(),
    }
}

pub fn file_label(weight: u16, italic: bool) -> String {
    if weight.is_multiple_of(100) {
        style_label(weight, italic)
    } else if italic {
        format!("W{weight}Italic")
    } else {
        format!("W{weight}")
    }
}

fn subset_suffix(details: &FontDetails, subset: &str) -> String {
    if subset == details.font.default_subset || subset == "latin" {
        String::new()
    } else {
        format!("-{}", title_case(subset))
    }
}

pub fn plan(details: &FontDetails, subset: &str) -> Vec<PlannedFile> {
    let stem = file_stem(&details.font.family);
    let suffix = subset_suffix(details, subset);
    details
        .files
        .iter()
        .filter(|f| f.subset == subset)
        .filter_map(|f| {
            let url = f.ttf.clone()?;
            let italic = f.style == "italic";
            Some(PlannedFile {
                url,
                file_name: format!("{stem}{suffix}-{}.ttf", file_label(f.weight, italic)),
                weight: f.weight,
                style: f.style.clone(),
            })
        })
        .collect()
}

pub fn subsets_with_files(details: &FontDetails) -> Vec<String> {
    details
        .font
        .subsets
        .iter()
        .filter(|s| {
            details
                .files
                .iter()
                .any(|f| &f.subset == *s && f.ttf.is_some())
        })
        .cloned()
        .collect()
}

pub fn variants(details: &FontDetails) -> Vec<(u16, bool)> {
    let mut out: Vec<(u16, bool)> = details
        .files
        .iter()
        .map(|f| (f.weight, f.style == "italic"))
        .collect();
    out.sort_by_key(|(w, i)| (*i, *w));
    out.dedup();
    out
}

fn encode_family(family: &str) -> String {
    let mut out = String::new();
    for b in family.trim().bytes() {
        match b {
            b' ' => out.push('+'),
            b if b.is_ascii_alphanumeric() || b == b'-' || b == b'_' || b == b'.' => {
                out.push(b as char)
            }
            b => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

pub fn google_css_url(details: &FontDetails) -> Option<String> {
    if details.font.source != "google" {
        return None;
    }
    let pairs = variants(details);
    if pairs.is_empty() {
        return None;
    }
    let family = encode_family(&details.font.family);
    let spec = if pairs.iter().any(|(_, i)| *i) {
        let list: Vec<String> = pairs
            .iter()
            .map(|(w, i)| format!("{},{w}", u8::from(*i)))
            .collect();
        format!("ital,wght@{}", list.join(";"))
    } else {
        let list: Vec<String> = pairs.iter().map(|(w, _)| w.to_string()).collect();
        format!("wght@{}", list.join(";"))
    };
    Some(format!("{GOOGLE_CSS}?family={family}:{spec}"))
}

fn css_value<'a>(block: &'a str, prop: &str) -> Option<&'a str> {
    let start = block.find(prop)? + prop.len();
    let rest = &block[start..];
    let rest = rest.trim_start().strip_prefix(':')?;
    let end = rest.find(';').unwrap_or(rest.len());
    Some(rest[..end].trim())
}

pub fn parse_google_css(css: &str) -> Vec<(u16, bool, String)> {
    let mut out = Vec::new();
    for block in css.split("@font-face").skip(1) {
        let block = block.split('}').next().unwrap_or("");
        let Some(weight) = css_value(block, "font-weight").and_then(|w| w.parse::<u16>().ok())
        else {
            continue;
        };
        let italic = css_value(block, "font-style") == Some("italic");
        let Some(src) = css_value(block, "src") else {
            continue;
        };
        if !src.contains("truetype") {
            continue;
        }
        let Some(url) = src
            .split_once("url(")
            .and_then(|(_, rest)| rest.split_once(')'))
            .map(|(u, _)| u.trim_matches(|c| c == '\'' || c == '"').to_string())
        else {
            continue;
        };
        if !url.starts_with("https://fonts.gstatic.com/") || !url.ends_with(".ttf") {
            continue;
        }
        if !out.iter().any(|(w, i, _)| *w == weight && *i == italic) {
            out.push((weight, italic, url));
        }
    }
    out
}

pub fn plan_full(details: &FontDetails, faces: &[(u16, bool, String)]) -> Vec<PlannedFile> {
    let stem = file_stem(&details.font.family);
    faces
        .iter()
        .map(|(weight, italic, url)| PlannedFile {
            url: url.clone(),
            file_name: format!("{stem}-{}.ttf", file_label(*weight, *italic)),
            weight: *weight,
            style: if *italic { "italic" } else { "normal" }.to_string(),
        })
        .collect()
}

pub fn other_subset_files(details: &FontDetails, subset: Option<&str>, dir: &Path) -> Vec<String> {
    let mut found = Vec::new();
    for other in subsets_with_files(details) {
        if Some(other.as_str()) == subset || subset_suffix(details, &other).is_empty() {
            continue;
        }
        for file in plan(details, &other) {
            if dir.join(&file.file_name).exists() && !found.contains(&file.file_name) {
                found.push(file.file_name);
            }
        }
    }
    found
}

pub fn check_license(details: &FontDetails) -> Result<(), String> {
    if details.font.license_class == LicenseClass::Open {
        Ok(())
    } else {
        Err(format!(
            "{} uses the license \"{}\", which is not on the open list (OFL, Apache, MIT). Read its terms on the author's site before using it.",
            details.font.family,
            if details.font.license.is_empty() { "unknown" } else { details.font.license.as_str() }
        ))
    }
}

pub fn write_new(path: &Path, bytes: &[u8]) -> std::io::Result<bool> {
    let mut file = match std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
    {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => return Ok(false),
        Err(e) => return Err(e),
    };
    if let Err(e) = file.write_all(bytes).and_then(|_| file.sync_all()) {
        drop(file);
        let _ = std::fs::remove_file(path);
        return Err(e);
    }
    Ok(true)
}

async fn download(client: &reqwest::Client, url: &str) -> Result<Vec<u8>, String> {
    let response = client
        .get(url)
        .send()
        .await
        .map_err(|e| catalog::network_error(&e))?;
    let status = response.status();
    if !status.is_success() {
        return Err(format!("The font CDN answered {status} for this file."));
    }
    if response.content_length().unwrap_or(0) > MAX_FILE_BYTES {
        return Err("This file is larger than 40 MB, which no single font should be.".to_string());
    }
    let bytes = response
        .bytes()
        .await
        .map_err(|e| catalog::network_error(&e))?;
    if bytes.len() as u64 > MAX_FILE_BYTES {
        return Err("This file is larger than 40 MB, which no single font should be.".to_string());
    }
    if ttf_parser::Face::parse(&bytes, 0).is_err() {
        return Err("The downloaded file is not a valid TrueType font.".to_string());
    }
    Ok(bytes.to_vec())
}

#[cfg(target_os = "windows")]
async fn register_windows(path: &Path, details: &FontDetails, file: &PlannedFile) -> bool {
    let key = r"HKCU\Software\Microsoft\Windows NT\CurrentVersion\Fonts";
    let value = format!(
        "{} {} (TrueType)",
        details.font.family,
        file_label(file.weight, file.style == "italic")
    );
    let exists = crate::core::process::command("reg")
        .args(["query", key, "/v", &value])
        .output()
        .await
        .map(|o| o.status.success())
        .unwrap_or(false);
    if exists {
        return false;
    }
    crate::core::process::command("reg")
        .args(["add", key, "/v", &value, "/t", "REG_SZ", "/d"])
        .arg(path)
        .arg("/f")
        .output()
        .await
        .map(|o| o.status.success())
        .unwrap_or(false)
}

#[cfg(not(target_os = "windows"))]
async fn register_windows(_path: &Path, _details: &FontDetails, _file: &PlannedFile) -> bool {
    false
}

async fn full_files(client: &reqwest::Client, details: &FontDetails) -> Option<Vec<PlannedFile>> {
    let url = google_css_url(details)?;
    let response = client
        .get(&url)
        .header(reqwest::header::USER_AGENT, LEGACY_AGENT)
        .send()
        .await
        .ok()?;
    if !response.status().is_success() {
        return None;
    }
    let css = response.text().await.ok()?;
    let faces = parse_google_css(&css);
    if faces.is_empty() {
        return None;
    }
    Some(plan_full(details, &faces))
}

async fn refresh_cache(dir: &Path) -> bool {
    if !cfg!(target_os = "linux") {
        return false;
    }
    crate::core::process::command("fc-cache")
        .arg("-f")
        .arg(dir)
        .output()
        .await
        .map(|o| o.status.success())
        .unwrap_or(false)
}

pub async fn run<F>(
    details: &FontDetails,
    subset: &str,
    target: Option<PathBuf>,
    token: CancellationToken,
    mut report: F,
) -> Result<InstallOutcome, String>
where
    F: FnMut(usize, usize, &str, FileState, Option<&str>),
{
    check_license(details)?;
    let installing = target.is_none();
    let dir = match target {
        Some(d) => d,
        None => super::scan::user_font_dir()
            .ok_or_else(|| "Could not find your user fonts folder.".to_string())?,
    };
    let client = catalog::client()?;
    let full = if token.is_cancelled() {
        None
    } else {
        tokio::select! {
            f = full_files(&client, details) => f,
            _ = token.cancelled() => None,
        }
    };
    let coverage = if full.is_some() { "full" } else { "subset" };
    let files = match full {
        Some(f) => f,
        None => plan(details, subset),
    };
    if files.is_empty() {
        return Err(format!(
            "Fontsource has no TrueType files of {} for the \"{subset}\" script.",
            details.font.family
        ));
    }
    if installing {
        let chosen = (coverage == "subset").then_some(subset);
        let older = other_subset_files(details, chosen, &dir);
        if !older.is_empty() {
            return Err(format!(
                "{} is already in your fonts folder with another script ({}). A second copy with the same name confuses apps, so remove those files first, then install again.",
                details.font.family,
                older.join(", ")
            ));
        }
    }
    std::fs::create_dir_all(&dir)
        .map_err(|e| format!("Could not create the folder {}: {e}", dir.display()))?;
    let total = files.len();
    let mut outcome = InstallOutcome {
        dir: dir.to_string_lossy().to_string(),
        installed: installing,
        coverage: coverage.to_string(),
        ..Default::default()
    };
    for (index, file) in files.iter().enumerate() {
        if token.is_cancelled() {
            outcome.cancelled = true;
            break;
        }
        let path = dir.join(&file.file_name);
        if path.exists() {
            outcome.skipped.push(file.file_name.clone());
            report(index, total, &file.file_name, FileState::Skipped, None);
            continue;
        }
        report(index, total, &file.file_name, FileState::Downloading, None);
        let result = tokio::select! {
            r = download(&client, &file.url) => r,
            _ = token.cancelled() => {
                outcome.cancelled = true;
                break;
            }
        };
        let result = result.and_then(|bytes| {
            write_new(&path, &bytes).map_err(|e| format!("Could not save the file: {e}"))
        });
        match result {
            Ok(true) => {
                if installing && register_windows(&path, details, file).await {
                    outcome.registered += 1;
                }
                outcome.saved.push(path.to_string_lossy().to_string());
                report(index, total, &file.file_name, FileState::Saved, None);
            }
            Ok(false) => {
                outcome.skipped.push(file.file_name.clone());
                report(index, total, &file.file_name, FileState::Skipped, None);
            }
            Err(e) => {
                report(index, total, &file.file_name, FileState::Failed, Some(&e));
                outcome.failed.push(FailedFile {
                    file_name: file.file_name.clone(),
                    error: e,
                });
            }
        }
    }
    if installing && !outcome.saved.is_empty() {
        outcome.cache_refreshed = refresh_cache(&dir).await;
        outcome.needs_restart = cfg!(target_os = "windows") || !outcome.cache_refreshed;
        super::scan::invalidate();
    }
    Ok(outcome)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::fonts::catalog::{CatalogFont, FontFile};

    fn details(license: &str) -> FontDetails {
        let file = |weight, style: &str, subset: &str, ttf: bool| FontFile {
            weight,
            style: style.to_string(),
            subset: subset.to_string(),
            ttf: ttf.then(|| format!("https://x/{subset}-{weight}-{style}.ttf")),
            woff2: None,
        };
        FontDetails {
            font: CatalogFont {
                id: "geist-mono".into(),
                family: "Geist Mono".into(),
                category: "monospace".into(),
                subsets: vec!["latin".into(), "cyrillic".into()],
                default_subset: "latin".into(),
                weights: vec![400, 700],
                styles: vec!["normal".into(), "italic".into()],
                variable: true,
                license: license.into(),
                license_class: catalog::classify_license(license),
                source: "other".into(),
            },
            version: Some("5.0.0".into()),
            files: vec![
                file(400, "normal", "latin", true),
                file(400, "italic", "latin", true),
                file(700, "normal", "latin", true),
                file(700, "normal", "cyrillic", true),
                file(800, "normal", "latin", false),
            ],
        }
    }

    #[test]
    fn plans_ttf_files_for_one_subset() {
        let d = details("OFL-1.1");
        let names: Vec<String> = plan(&d, "latin").into_iter().map(|p| p.file_name).collect();
        assert_eq!(
            names,
            vec![
                "GeistMono-Regular.ttf",
                "GeistMono-Italic.ttf",
                "GeistMono-Bold.ttf"
            ]
        );
        let cyr = plan(&d, "cyrillic");
        assert_eq!(cyr.len(), 1);
        assert_eq!(cyr[0].file_name, "GeistMono-Cyrillic-Bold.ttf");
        assert!(plan(&d, "greek").is_empty());
    }

    #[test]
    fn keeps_odd_weights_apart() {
        let mut d = details("OFL-1.1");
        d.files.push(FontFile {
            weight: 450,
            style: "normal".into(),
            subset: "latin".into(),
            ttf: Some("https://x/latin-450.ttf".into()),
            woff2: None,
        });
        d.files.push(FontFile {
            weight: 500,
            style: "normal".into(),
            subset: "latin".into(),
            ttf: Some("https://x/latin-500.ttf".into()),
            woff2: None,
        });
        let names: Vec<String> = plan(&d, "latin").into_iter().map(|p| p.file_name).collect();
        assert!(names.contains(&"GeistMono-W450.ttf".to_string()));
        assert!(names.contains(&"GeistMono-Medium.ttf".to_string()));
        let mut unique = names.clone();
        unique.sort();
        unique.dedup();
        assert_eq!(unique.len(), names.len());
        assert_eq!(file_label(450, true), "W450Italic");
    }

    #[test]
    fn lists_only_subsets_with_files() {
        let mut d = details("OFL-1.1");
        d.font.subsets.push("japanese".into());
        d.files.push(FontFile {
            weight: 400,
            style: "normal".into(),
            subset: "17".into(),
            ttf: Some("https://x/17.ttf".into()),
            woff2: None,
        });
        assert_eq!(subsets_with_files(&d), vec!["latin", "cyrillic"]);
    }

    #[test]
    fn builds_google_request_only_for_google_fonts() {
        let mut d = details("OFL-1.1");
        assert!(google_css_url(&d).is_none());
        d.font.source = "google".into();
        assert_eq!(
            google_css_url(&d).unwrap(),
            "https://fonts.googleapis.com/css2?family=Geist+Mono:ital,wght@0,400;0,700;0,800;1,400"
        );
        d.files.retain(|f| f.style == "normal");
        assert_eq!(
            google_css_url(&d).unwrap(),
            "https://fonts.googleapis.com/css2?family=Geist+Mono:wght@400;700;800"
        );
    }

    #[test]
    fn reads_full_files_from_google_css() {
        let css = "/* latin */\n@font-face {\n  font-family: 'Inter';\n  font-style: italic;\n  font-weight: 400;\n  src: url(https://fonts.gstatic.com/s/inter/a.ttf) format('truetype');\n}\n@font-face {\n  font-family: 'Inter';\n  font-style: normal;\n  font-weight: 700;\n  src: url(https://fonts.gstatic.com/s/inter/b.ttf) format('truetype');\n}\n@font-face {\n  font-style: normal;\n  font-weight: 800;\n  src: url(https://evil.example/c.ttf) format('truetype');\n}\n";
        let faces = parse_google_css(css);
        assert_eq!(faces.len(), 2);
        assert_eq!(
            faces[0],
            (
                400,
                true,
                "https://fonts.gstatic.com/s/inter/a.ttf".to_string()
            )
        );
        let mut d = details("OFL-1.1");
        d.font.family = "Inter".into();
        let names: Vec<String> = plan_full(&d, &faces)
            .into_iter()
            .map(|p| p.file_name)
            .collect();
        assert_eq!(names, vec!["Inter-Italic.ttf", "Inter-Bold.ttf"]);
        assert!(parse_google_css("<html>error</html>").is_empty());
    }

    #[test]
    fn finds_files_from_another_script() {
        let d = details("OFL-1.1");
        let dir = tempfile::tempdir().unwrap();
        assert!(other_subset_files(&d, Some("latin"), dir.path()).is_empty());
        std::fs::write(dir.path().join("GeistMono-Cyrillic-Bold.ttf"), b"x").unwrap();
        std::fs::write(dir.path().join("GeistMono-Regular.ttf"), b"x").unwrap();
        assert_eq!(
            other_subset_files(&d, Some("latin"), dir.path()),
            vec!["GeistMono-Cyrillic-Bold.ttf"]
        );
        assert!(other_subset_files(&d, Some("cyrillic"), dir.path()).is_empty());
        assert_eq!(other_subset_files(&d, None, dir.path()).len(), 1);
    }

    #[test]
    fn labels_weights() {
        assert_eq!(style_label(100, false), "Thin");
        assert_eq!(style_label(600, true), "SemiBoldItalic");
        assert_eq!(style_label(400, true), "Italic");
        assert_eq!(style_label(900, false), "Black");
    }

    #[test]
    fn refuses_other_licenses() {
        assert!(check_license(&details("OFL-1.1")).is_ok());
        assert!(check_license(&details("UFL-1.0")).is_err());
    }

    #[test]
    fn never_overwrites() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.ttf");
        assert!(write_new(&path, b"one").unwrap());
        assert!(!write_new(&path, b"two").unwrap());
        assert_eq!(std::fs::read(&path).unwrap(), b"one");
    }

    #[tokio::test]
    async fn refuses_before_touching_the_disk() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("out");
        let result = run(
            &details("UFL-1.0"),
            "latin",
            Some(target.clone()),
            CancellationToken::new(),
            |_, _, _, _, _| {},
        )
        .await;
        assert!(result.is_err());
        assert!(!target.exists());
    }
}

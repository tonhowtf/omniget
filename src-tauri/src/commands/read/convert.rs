//! Conversions. Native ones need nothing installed: text, Markdown and HTML
//! to EPUB; EPUB to Kobo kepub; comics prepared for an e-reader (border crop, double-page
//! split, grayscale, resize to the device) as CBZ or fixed-layout EPUB;
//! a series' chapters merged into one volume. Everything else goes to the
//! tools the user has: calibre (`ebook-convert`), KCC (`kcc-c2e`), pandoc,
//! kepubify and go-comic-converter, run as processes on the side.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter};

use omniget_core::core::read::epub_writer::{self, EpubMeta};
use omniget_core::core::read::format::{self, BookFormat};

use super::{blocking, db, scan, server};

// ---------- tools ----------

#[derive(Debug, Clone, Serialize, Default)]
pub struct Tool {
    pub id: String,
    pub name: String,
    pub path: Option<String>,
    pub version: Option<String>,
    pub url: String,
}

fn platform_paths(id: &str) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = Vec::new();
    let home = dirs::home_dir().unwrap_or_default();
    match id {
        "ebook-convert" | "ebook-meta" | "ebook-polish" => {
            let exe = if cfg!(windows) {
                format!("{id}.exe")
            } else {
                id.to_string()
            };
            v.push(PathBuf::from(format!(
                "/Applications/calibre.app/Contents/MacOS/{id}"
            )));
            v.push(home.join(format!("Applications/calibre.app/Contents/MacOS/{id}")));
            v.push(PathBuf::from("/opt/calibre").join(&exe));
            if let Ok(pf) = std::env::var("ProgramFiles") {
                v.push(PathBuf::from(pf).join("Calibre2").join(&exe));
            }
            if let Ok(pf) = std::env::var("ProgramFiles(x86)") {
                v.push(PathBuf::from(pf).join("Calibre2").join(&exe));
            }
        }
        "kcc-c2e" => {
            v.push(PathBuf::from(
                "/Applications/Kindle Comic Converter.app/Contents/MacOS/kcc-c2e",
            ));
            v.push(home.join(".local/bin/kcc-c2e"));
        }
        "pandoc" => {
            if let Ok(l) = std::env::var("LOCALAPPDATA") {
                v.push(PathBuf::from(l).join("Pandoc").join("pandoc.exe"));
            }
            if let Ok(pf) = std::env::var("ProgramFiles") {
                v.push(PathBuf::from(pf).join("Pandoc").join("pandoc.exe"));
            }
        }
        "go-comic-converter" => {
            v.push(home.join("go/bin/go-comic-converter"));
            v.push(home.join("go\\bin\\go-comic-converter.exe"));
        }
        _ => {}
    }
    v
}

pub fn find(id: &str) -> Option<PathBuf> {
    // A path the user picked wins.
    if let Some(p) = db::kv_get::<String>(&format!("tool:{id}")).filter(|p| Path::new(p).is_file())
    {
        return Some(PathBuf::from(p));
    }
    if let Ok(p) = which::which(id) {
        return Some(p);
    }
    if let Some(dir) = omniget_core::core::paths::app_data_dir() {
        let p = dir.join("bin").join(if cfg!(windows) {
            format!("{id}.exe")
        } else {
            id.to_string()
        });
        if p.is_file() {
            return Some(p);
        }
    }
    platform_paths(id).into_iter().find(|p| p.is_file())
}

async fn version_of(path: &Path, id: &str) -> Option<String> {
    // 7-Zip has no version flag; run bare, it prints its banner first.
    let args: &[&str] = match id {
        "go-comic-converter" => &["-version"],
        "kcc-c2e" => &["--help"],
        "7z" => &[],
        _ => &["--version"],
    };
    let out = omniget_core::core::process::command(path)
        .args(args)
        .kill_on_drop(true)
        .output()
        .await
        .ok()?;
    let text =
        String::from_utf8_lossy(&out.stdout).to_string() + &String::from_utf8_lossy(&out.stderr);
    text.lines()
        .find(|l| !l.trim().is_empty())
        .map(|l| l.trim().chars().take(80).collect())
}

pub const TOOLS: &[(&str, &str, &str)] = &[
    (
        "ebook-convert",
        "calibre",
        "https://calibre-ebook.com/download",
    ),
    (
        "kcc-c2e",
        "Kindle Comic Converter",
        "https://github.com/ciromattia/kcc/releases",
    ),
    ("pandoc", "pandoc", "https://pandoc.org/installing.html"),
    ("kepubify", "kepubify", "https://pgaskin.net/kepubify/"),
    (
        "go-comic-converter",
        "go-comic-converter",
        "https://github.com/celogeek/go-comic-converter",
    ),
    ("7z", "7-Zip", "https://www.7-zip.org/"),
    (
        "markitdown",
        "MarkItDown",
        "https://github.com/microsoft/markitdown",
    ),
    (
        "ebook2audiobook",
        "ebook2audiobook",
        "https://github.com/DrewThomasson/ebook2audiobook",
    ),
    (
        "uv",
        "uv (runs MarkItDown without installing it)",
        "https://docs.astral.sh/uv/",
    ),
    (
        "fanficfare",
        "FanFicFare",
        "https://github.com/JimmXinu/FanFicFare",
    ),
    (
        "lncrawl",
        "lightnovel-crawler",
        "https://github.com/lncrawl/lightnovel-crawler",
    ),
];

#[tauri::command]
pub async fn read_tools() -> Result<Vec<Tool>, String> {
    let mut out = Vec::new();
    for (id, name, url) in TOOLS {
        let path = find(id).or_else(|| {
            if *id == "7z" {
                find("7zz").or_else(|| find("7za"))
            } else {
                None
            }
        });
        let version = match &path {
            Some(p) => tokio::time::timeout(std::time::Duration::from_secs(8), version_of(p, id))
                .await
                .ok()
                .flatten(),
            None => None,
        };
        out.push(Tool {
            id: id.to_string(),
            name: name.to_string(),
            path: path.map(|p| p.to_string_lossy().to_string()),
            version,
            url: url.to_string(),
        });
    }
    Ok(out)
}

#[tauri::command]
pub async fn read_tool_set_path(id: String, path: String) -> Result<(), String> {
    if !path.is_empty() && !Path::new(&path).is_file() {
        return Err("file not found".into());
    }
    blocking(move || db::kv_set(&format!("tool:{id}"), &path)).await
}

// ---------- devices ----------

/// E-reader screens (from KCC's profile table), for native comic output.
pub const DEVICES: &[(&str, &str, u32, u32, bool)] = &[
    ("KPW5", "Kindle Paperwhite 5 / Signature", 1236, 1648, false),
    ("KPW6", "Kindle Paperwhite 6", 1272, 1696, false),
    ("KPW34", "Kindle Paperwhite 3/4, Oasis 1", 1072, 1448, false),
    ("KO", "Kindle Oasis 2/3", 1264, 1680, false),
    ("K11", "Kindle 11", 1072, 1448, false),
    ("K810", "Kindle 8/10", 600, 800, false),
    ("KS", "Kindle Scribe", 1860, 2480, false),
    ("KCS", "Kindle Colorsoft", 1272, 1696, true),
    ("KoC", "Kobo Clara HD / 2E", 1072, 1448, false),
    ("KoCC", "Kobo Clara Colour", 1072, 1448, true),
    ("KoL", "Kobo Libra H2O / 2", 1264, 1680, false),
    ("KoLC", "Kobo Libra Colour", 1264, 1680, true),
    ("KoS", "Kobo Sage", 1440, 1920, false),
    ("KoE", "Kobo Elipsa", 1404, 1872, false),
    ("KoF", "Kobo Forma", 1440, 1920, false),
    ("KoN", "Kobo Nia", 758, 1024, false),
    ("Rmk2", "reMarkable 2", 1404, 1872, false),
    ("RmkPP", "reMarkable Paper Pro", 1620, 2160, true),
    ("PB", "PocketBook (6\")", 1072, 1448, false),
    ("TAB", "Tablet / phone (color, 1600 px)", 1200, 1600, true),
];

#[tauri::command]
pub fn read_devices() -> Vec<serde_json::Value> {
    DEVICES
        .iter()
        .map(|(id, name, w, h, color)| serde_json::json!({ "id": id, "name": name, "width": w, "height": h, "color": color }))
        .collect()
}

// ---------- jobs ----------

#[derive(Debug, Clone, Serialize)]
pub struct Job {
    pub id: u64,
    pub label: String,
    pub state: String,
    pub progress: f64,
    pub message: String,
    pub output: Option<String>,
    pub book_id: Option<i64>,
}

static NEXT: AtomicU64 = AtomicU64::new(1);
static JOBS: Mutex<Option<HashMap<u64, Job>>> = Mutex::new(None);

fn put(app: &AppHandle, job: &Job) {
    if let Ok(mut j) = JOBS.lock() {
        j.get_or_insert_with(HashMap::new)
            .insert(job.id, job.clone());
    }
    let _ = app.emit("read-convert-progress", job);
}

#[tauri::command]
pub fn read_convert_jobs() -> Vec<Job> {
    let mut v: Vec<Job> = JOBS
        .lock()
        .ok()
        .and_then(|j| j.as_ref().map(|m| m.values().cloned().collect()))
        .unwrap_or_default();
    v.sort_by_key(|i| std::cmp::Reverse(i.id));
    v
}

#[tauri::command]
pub fn read_convert_clear() {
    if let Ok(mut j) = JOBS.lock() {
        if let Some(m) = j.as_mut() {
            m.retain(|_, x| x.state == "running");
        }
    }
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default)]
pub struct ComicOpts {
    pub device: String,
    pub crop: bool,
    pub split: bool,
    pub grayscale: bool,
    pub rtl: bool,
    pub quality: u8,
    pub upscale: bool,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default)]
pub struct ConvertReq {
    pub book_ids: Vec<i64>,
    pub paths: Vec<String>,
    /// epub, kepub, azw3, mobi, pdf, cbz, docx, txt, fb2...
    pub to: String,
    /// auto, native, calibre, kcc, pandoc, kepubify, gcc
    pub engine: String,
    pub comic: ComicOpts,
    /// Extra command line flags for the external tool.
    pub extra_args: Vec<String>,
    pub output_dir: Option<String>,
    pub add_to_library: bool,
}

fn job(label: &str) -> Job {
    Job {
        id: NEXT.fetch_add(1, Ordering::SeqCst),
        label: label.to_string(),
        state: "running".into(),
        progress: 0.0,
        message: String::new(),
        output: None,
        book_id: None,
    }
}

fn pick_engine(from: Option<BookFormat>, to: &str, want: &str) -> &'static str {
    let Some(from) = from else {
        // Office documents and other files the library doesn't index.
        return match want {
            "calibre" => "calibre",
            "pandoc" => "pandoc",
            _ if find("markitdown").is_some() || find("uv").is_some() => "markitdown",
            _ if find("pandoc").is_some() => "pandoc",
            _ if find("ebook-convert").is_some() => "calibre",
            _ => "missing",
        };
    };
    let native_text =
        matches!(from, BookFormat::Txt | BookFormat::Md | BookFormat::Html) && to == "epub";
    let native_comic = from.is_comic() && matches!(to, "cbz" | "epub");
    let native_kepub = to == "kepub" && matches!(from, BookFormat::Epub);
    match want {
        "calibre" => "calibre",
        "kcc" => "kcc",
        "pandoc" => "pandoc",
        "kepubify" => "kepubify",
        "gcc" => "gcc",
        "native" => "native",
        "markitdown" => "markitdown",
        "audiobook" => "audiobook",
        _ if matches!(to, "m4b" | "mp3") => "audiobook",
        _ if native_text || native_comic => "native",
        _ if native_kepub => "native",
        _ if from.is_comic() && find("kcc-c2e").is_some() => "kcc",
        _ if find("ebook-convert").is_some() => "calibre",
        _ if find("pandoc").is_some()
            && !from.is_comic()
            && !matches!(from, BookFormat::Pdf | BookFormat::Mobi | BookFormat::Azw3) =>
        {
            "pandoc"
        }
        _ => "missing",
    }
}

#[tauri::command]
pub async fn read_convert(app: AppHandle, req: ConvertReq) -> Result<Vec<u64>, String> {
    let mut inputs: Vec<(PathBuf, Option<db::BookRow>)> = Vec::new();
    for id in &req.book_ids {
        let b = db::book(*id)?;
        inputs.push((PathBuf::from(&b.path), Some(b)));
    }
    for p in &req.paths {
        inputs.push((PathBuf::from(p), None));
    }
    let mut ids = Vec::new();
    for (path, book) in inputs {
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        let mut j = job(&format!("{name} → {}", req.to.to_uppercase()));
        ids.push(j.id);
        put(&app, &j);
        let app2 = app.clone();
        let req2 = req.clone();
        tauri::async_runtime::spawn(async move {
            match convert_one(&app2, &mut j, &path, book.as_ref(), &req2).await {
                Ok(out) => {
                    j.output = Some(out.to_string_lossy().to_string());
                    if req2.add_to_library {
                        let outp = out.clone();
                        let src = book.as_ref().map(|b| b.source.clone()).unwrap_or_default();
                        if let Ok(Ok(id)) = tokio::task::spawn_blocking(move || {
                            scan::index_file(
                                &outp,
                                &scan::IndexOpts {
                                    source: if src.is_empty() {
                                        "converted".into()
                                    } else {
                                        src
                                    },
                                    ..Default::default()
                                },
                            )
                        })
                        .await
                        {
                            j.book_id = Some(id);
                            let _ = app2.emit("read-library-changed", ());
                        }
                    }
                    j.state = "done".into();
                    j.progress = 1.0;
                }
                Err(e) => {
                    j.state = "failed".into();
                    j.message = e;
                }
            }
            put(&app2, &j);
        });
    }
    Ok(ids)
}

fn out_path(input: &Path, dir: Option<&str>, to: &str) -> PathBuf {
    let stem = omniget_core::core::read::extract::title_from_name(input);
    let dir = dir
        .map(PathBuf::from)
        .or_else(|| input.parent().map(|p| p.to_path_buf()))
        .unwrap_or_default();
    let ext = match to {
        "kepub" => "kepub.epub",
        other => other,
    };
    let mut p = dir.join(format!("{stem}.{ext}"));
    let mut n = 2;
    while p.exists() || p == input {
        p = dir.join(format!("{stem} ({n}).{ext}"));
        n += 1;
    }
    p
}

async fn convert_one(
    app: &AppHandle,
    j: &mut Job,
    input: &Path,
    book: Option<&db::BookRow>,
    req: &ConvertReq,
) -> Result<PathBuf, String> {
    let known = if input.is_dir() {
        Some(BookFormat::Folder)
    } else {
        BookFormat::from_path(input)
    };
    let to = req.to.to_lowercase();
    let out = out_path(input, req.output_dir.as_deref(), &to);
    let engine = pick_engine(known, &to, &req.engine);
    let from = known.unwrap_or(BookFormat::Html);
    j.message = engine.to_string();
    put(app, j);
    match engine {
        "native" => {
            let input = input.to_path_buf();
            let out2 = out.clone();
            let book = book.cloned();
            let opts = req.comic.clone();
            let to2 = to.clone();
            blocking(move || native(&input, from, &to2, &out2, book.as_ref(), &opts)).await?;
            Ok(out)
        }
        "calibre" => {
            let exe = find("ebook-convert").ok_or("calibre is not installed")?;
            let mut args: Vec<String> =
                vec![input.to_string_lossy().into(), out.to_string_lossy().into()];
            if to == "epub" {
                args.extend(["--epub-version".into(), "3".into()]);
            }
            if to == "azw3" || to == "mobi" {
                args.push("--no-inline-toc".into());
            }
            if from.is_comic() && req.comic.rtl {
                args.push("--right2left".into());
            }
            if let Some(dev) = calibre_profile(&req.comic.device) {
                args.extend(["--output-profile".into(), dev.into()]);
            }
            args.extend(req.extra_args.iter().cloned());
            run_tool(app, j, &exe, &args, &HashMap::new()).await?;
            found(&out)
        }
        "kcc" => {
            let exe = find("kcc-c2e").ok_or("KCC is not installed")?;
            let outdir = out.parent().map(|p| p.to_path_buf()).unwrap_or_default();
            let before = snapshot(&outdir);
            let dev = if req.comic.device.is_empty() {
                "KPW5".to_string()
            } else {
                req.comic.device.clone()
            };
            let fmt = match to.as_str() {
                "kepub" | "epub" => "EPUB",
                "azw3" | "mobi" => "MOBI",
                "pdf" => "PDF",
                "cbz" => "CBZ",
                _ => "Auto",
            };
            let mut args: Vec<String> = vec![
                "-p".into(),
                dev,
                "-f".into(),
                fmt.into(),
                "-o".into(),
                outdir.to_string_lossy().into(),
            ];
            if req.comic.rtl {
                args.push("-m".into());
            }
            if req.comic.upscale {
                args.push("-u".into());
            }
            if !req.comic.split {
                args.extend(["-r".into(), "1".into()]);
            }
            if !req.comic.grayscale {
                args.push("--forcecolor".into());
            }
            if let Some(b) = book {
                args.extend(["-t".into(), b.title.clone()]);
                if let Some(a) = b.authors.first() {
                    args.extend(["-a".into(), a.clone()]);
                }
            }
            args.extend(req.extra_args.iter().cloned());
            args.push(input.to_string_lossy().into());
            let mut env = HashMap::new();
            env.insert("PYTHONUNBUFFERED".to_string(), "1".to_string());
            let log = run_tool(app, j, &exe, &args, &env).await?;
            // KCC exits 0 even when kindlegen fails.
            if let Some(err) = log.lines().find(|l| l.contains("Error:")) {
                return Err(err.trim().to_string());
            }
            newest_new(&outdir, &before).ok_or_else(|| "KCC produced no file".to_string())
        }
        "pandoc" => {
            let exe = find("pandoc").ok_or("pandoc is not installed")?;
            let mut args: Vec<String> = vec![
                input.to_string_lossy().into(),
                "-o".into(),
                out.to_string_lossy().into(),
                "--toc".into(),
            ];
            if let Some(b) = book {
                args.extend(["-M".into(), format!("title={}", b.title)]);
                for a in &b.authors {
                    args.extend(["-M".into(), format!("author={a}")]);
                }
                if !b.language.is_empty() {
                    args.extend(["-M".into(), format!("lang={}", b.language)]);
                }
            }
            args.extend(req.extra_args.iter().cloned());
            run_tool(app, j, &exe, &args, &HashMap::new()).await?;
            found(&out)
        }
        "kepubify" => {
            let exe = find("kepubify").ok_or("kepubify is not installed")?;
            let outdir = out.parent().map(|p| p.to_path_buf()).unwrap_or_default();
            let before = snapshot(&outdir);
            let args: Vec<String> = vec![
                "-o".into(),
                outdir.to_string_lossy().into(),
                input.to_string_lossy().into(),
            ];
            run_tool(app, j, &exe, &args, &HashMap::new()).await?;
            newest_new(&outdir, &before).ok_or_else(|| "kepubify produced no file".to_string())
        }
        "gcc" => {
            let exe = find("go-comic-converter").ok_or("go-comic-converter is not installed")?;
            let dev = if req.comic.device.is_empty() {
                "KPW5".to_string()
            } else {
                req.comic.device.clone()
            };
            let args: Vec<String> = vec![
                format!("-input={}", input.to_string_lossy()),
                format!("-output={}", out.to_string_lossy()),
                format!("-profile={dev}"),
                format!("-manga={}", req.comic.rtl as u8),
                format!("-crop={}", req.comic.crop as u8),
                format!("-grayscale={}", req.comic.grayscale as u8),
                format!("-autosplitdoublepage={}", req.comic.split as u8),
                "-json".into(),
            ];
            run_tool(app, j, &exe, &args, &HashMap::new()).await?;
            found(&out)
        }
        "markitdown" => {
            // File -> Markdown with MarkItDown, then our own Markdown -> EPUB.
            let md = markitdown(input).await?;
            j.progress = 0.7;
            put(app, j);
            if to == "md" {
                std::fs::write(&out, md).map_err(|e| e.to_string())?;
                return Ok(out);
            }
            let meta = meta_for(input, book);
            let out2 = out.clone();
            blocking(move || {
                let chapters = epub_writer::markdown_chapters(&md, &meta.title);
                epub_writer::write_reflowable(&out2, &meta, &chapters, None)
            })
            .await?;
            Ok(out)
        }
        "audiobook" => {
            let exe = find("ebook2audiobook")
                .ok_or("ebook2audiobook is not installed (set its path in Tools)")?;
            let outdir = out.parent().map(|p| p.to_path_buf()).unwrap_or_default();
            let before = snapshot(&outdir);
            let lang = book.map(|b| b.language.clone()).unwrap_or_default();
            let mut args: Vec<String> = vec![
                "--headless".into(),
                "--ebook".into(),
                input.to_string_lossy().into(),
                "--output_dir".into(),
                outdir.to_string_lossy().into(),
                "--output_format".into(),
                if to == "mp3" {
                    "mp3".into()
                } else {
                    "m4b".into()
                },
            ];
            if let Some(l) = iso3(&lang) {
                args.extend(["--language".into(), l.into()]);
            }
            args.extend(req.extra_args.iter().cloned());
            run_tool(app, j, &exe, &args, &HashMap::new()).await?;
            newest_new(&outdir, &before)
                .ok_or_else(|| "ebook2audiobook produced no file".to_string())
        }
        _ => Err(format!(
            "no converter for {} → {to}; install calibre (or KCC for comics)",
            format!("{from:?}").to_uppercase()
        )),
    }
}

/// Runs MarkItDown (installed, or through `uv tool run`) and returns the
/// Markdown it prints.
async fn markitdown(input: &Path) -> Result<String, String> {
    let (exe, pre): (PathBuf, Vec<String>) = match find("markitdown") {
        Some(p) => (p, vec![]),
        None => (
            find("uv").ok_or("install MarkItDown or uv to convert this file")?,
            vec![
                "tool".into(),
                "run".into(),
                "--from".into(),
                "markitdown[all]".into(),
                "markitdown".into(),
            ],
        ),
    };
    let out = omniget_core::core::process::command(&exe)
        .args(&pre)
        .arg(input)
        .stdin(std::process::Stdio::null())
        .kill_on_drop(true)
        .output()
        .await
        .map_err(|e| e.to_string())?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        return Err(err
            .lines()
            .last()
            .unwrap_or("markitdown failed")
            .to_string());
    }
    Ok(String::from_utf8_lossy(&out.stdout).to_string())
}

/// ISO 639-3 codes ebook2audiobook wants.
fn iso3(lang: &str) -> Option<&'static str> {
    Some(match lang.get(..2)?.to_ascii_lowercase().as_str() {
        "en" => "eng",
        "pt" => "por",
        "es" => "spa",
        "fr" => "fra",
        "de" => "deu",
        "it" => "ita",
        "ru" => "rus",
        "ja" => "jpn",
        "zh" => "zho",
        "ko" => "kor",
        "nl" => "nld",
        "pl" => "pol",
        "tr" => "tur",
        "ar" => "ara",
        "hi" => "hin",
        _ => return None,
    })
}

fn calibre_profile(device: &str) -> Option<&'static str> {
    Some(match device {
        "KPW5" | "KPW6" | "KO" => "kindle_oasis",
        "KPW34" | "K11" => "kindle_pw3",
        "K810" => "kindle",
        "KS" => "kindle_scribe",
        "KoC" | "KoCC" | "KoL" | "KoLC" | "KoS" | "KoE" | "KoF" | "KoN" => "kobo",
        "TAB" => "tablet",
        "" => return None,
        _ => "generic_eink_hd",
    })
}

fn found(p: &Path) -> Result<PathBuf, String> {
    if p.exists() {
        Ok(p.to_path_buf())
    } else {
        Err("the converter finished but wrote no file".into())
    }
}

fn snapshot(dir: &Path) -> std::collections::HashSet<PathBuf> {
    std::fs::read_dir(dir)
        .map(|r| r.flatten().map(|e| e.path()).collect())
        .unwrap_or_default()
}

fn newest_new(dir: &Path, before: &std::collections::HashSet<PathBuf>) -> Option<PathBuf> {
    std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .filter(|p| !before.contains(p) && p.is_file())
        .max_by_key(|p| std::fs::metadata(p).and_then(|m| m.modified()).ok())
}

/// Runs a converter, turning `NN%` lines (calibre), KCC stage lines and
/// go-comic-converter JSON into progress. Returns the combined output.
async fn run_tool(
    app: &AppHandle,
    j: &mut Job,
    exe: &Path,
    args: &[String],
    env: &HashMap<String, String>,
) -> Result<String, String> {
    use tokio::io::{AsyncBufReadExt, BufReader};
    let mut cmd = omniget_core::core::process::command(exe);
    cmd.args(args)
        .envs(env)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);
    let mut child = cmd.spawn().map_err(|e| format!("{}: {e}", exe.display()))?;
    let stdout = child.stdout.take().ok_or("no stdout")?;
    let stderr = child.stderr.take().ok_or("no stderr")?;
    let err_task = tokio::spawn(async move {
        let mut s = String::new();
        let mut lines = BufReader::new(stderr).lines();
        while let Ok(Some(l)) = lines.next_line().await {
            s.push_str(&l);
            s.push('\n');
        }
        s
    });
    let pct = regex::Regex::new(r"^\s*(\d{1,3})% (.*)$").expect("static regex");
    let mut log = String::new();
    let mut lines = BufReader::new(stdout).lines();
    let mut last = std::time::Instant::now();
    while let Ok(Some(l)) = lines.next_line().await {
        log.push_str(&l);
        log.push('\n');
        let mut changed = false;
        if let Some(c) = pct.captures(&l) {
            j.progress = c[1].parse::<f64>().unwrap_or(0.0) / 100.0;
            j.message = c[2].trim().chars().take(120).collect();
            changed = true;
        } else if let Ok(v) = serde_json::from_str::<serde_json::Value>(&l) {
            let p = &v["data"]["epubprogress"];
            if let (Some(cur), Some(tot)) = (p["current"].as_f64(), p["total"].as_f64()) {
                if tot > 0.0 {
                    j.progress = cur / tot;
                    changed = true;
                }
            }
        } else if l.ends_with("...") {
            j.message = l.trim().chars().take(120).collect();
            changed = true;
        }
        if changed && last.elapsed().as_millis() > 200 {
            last = std::time::Instant::now();
            put(app, j);
        }
    }
    let status = child.wait().await.map_err(|e| e.to_string())?;
    let err = err_task.await.unwrap_or_default();
    log.push_str(&err);
    if !status.success() {
        let tail: Vec<&str> = log
            .lines()
            .rev()
            .filter(|l| !l.trim().is_empty())
            .take(3)
            .collect();
        return Err(tail.into_iter().rev().collect::<Vec<_>>().join(" | "));
    }
    Ok(log)
}

// ---------- web novels ----------

/// Saves a web novel or fanfic as EPUB with FanFicFare (fanfiction sites,
/// AO3...) or lightnovel-crawler (light novel sites). With `update`, an
/// existing FanFicFare EPUB gets only the new chapters.
#[tauri::command]
pub async fn read_webnovel(
    app: AppHandle,
    url: String,
    tool: String,
    update_book: Option<i64>,
) -> Result<Vec<i64>, String> {
    let dir = PathBuf::from(super::default_download_dir()).join("Web novels");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let before = snapshot(&dir);
    let mut j = job(&format!("{url} → EPUB"));
    put(&app, &j);
    let (exe, args): (PathBuf, Vec<String>) = match tool.as_str() {
        "lncrawl" => (
            find("lncrawl").ok_or("lightnovel-crawler is not installed")?,
            vec![
                "-s".into(),
                url.clone(),
                "-o".into(),
                dir.to_string_lossy().into(),
                "-f".into(),
                "epub".into(),
                "--all".into(),
                "--suppress".into(),
            ],
        ),
        _ => {
            let exe = find("fanficfare").ok_or("FanFicFare is not installed")?;
            let mut a: Vec<String> = vec!["--format=epub".into(), "--non-interactive".into()];
            match update_book.and_then(|id| db::book(id).ok()) {
                Some(b) => a.extend(["--update-epub".into(), b.path.clone()]),
                None => a.push(url.clone()),
            }
            (exe, a)
        }
    };
    let mut cmd = omniget_core::core::process::command(&exe);
    cmd.args(&args)
        .current_dir(&dir)
        .stdin(std::process::Stdio::null())
        .kill_on_drop(true);
    let out = cmd.output().await.map_err(|e| e.to_string())?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr).to_string()
            + &String::from_utf8_lossy(&out.stdout);
        j.state = "failed".into();
        j.message = err
            .lines()
            .rev()
            .find(|l| !l.trim().is_empty())
            .unwrap_or("failed")
            .to_string();
        put(&app, &j);
        return Err(j.message.clone());
    }
    // lncrawl writes into a folder per novel; look one level down too.
    let mut files: Vec<PathBuf> = walkdir::WalkDir::new(&dir)
        .max_depth(4)
        .into_iter()
        .filter_map(|e| e.ok())
        .map(|e| e.into_path())
        .filter(|p| p.extension().is_some_and(|x| x == "epub") && !before.contains(p))
        .collect();
    if let Some(b) = update_book.and_then(|id| db::book(id).ok()) {
        files.push(PathBuf::from(b.path));
    }
    let src = url.clone();
    let ids = blocking(move || {
        let mut ids = Vec::new();
        for f in files {
            if let Ok(id) = scan::index_file(
                &f,
                &scan::IndexOpts {
                    source: "webnovel".into(),
                    source_url: src.clone(),
                    ..Default::default()
                },
            ) {
                ids.push(id);
            }
        }
        Ok(ids)
    })
    .await?;
    j.state = "done".into();
    j.progress = 1.0;
    j.book_id = ids.first().copied();
    put(&app, &j);
    let _ = app.emit("read-library-changed", ());
    Ok(ids)
}

// ---------- native ----------

fn meta_for(input: &Path, book: Option<&db::BookRow>) -> EpubMeta {
    match book {
        Some(b) => EpubMeta {
            title: b.title.clone(),
            authors: b.authors.clone(),
            language: b.language.clone(),
            description: b.description.clone(),
            publisher: b.publisher.clone(),
            subjects: b.subjects.clone(),
            series: b.series.clone(),
            series_index: b.series_index,
            source_url: b.source_url.clone(),
            rtl: b.rtl,
        },
        None => EpubMeta {
            title: omniget_core::core::read::extract::title_from_name(input),
            ..Default::default()
        },
    }
}

fn native(
    input: &Path,
    from: BookFormat,
    to: &str,
    out: &Path,
    book: Option<&db::BookRow>,
    opts: &ComicOpts,
) -> Result<(), String> {
    let meta = meta_for(input, book);
    match (from, to) {
        (BookFormat::Epub, "kepub") => omniget_core::core::read::kepub::convert(input, out),
        (BookFormat::Txt, "epub") => {
            let text = epub_writer::decode_text(&std::fs::read(input).map_err(|e| e.to_string())?);
            epub_writer::write_reflowable(
                out,
                &meta,
                &epub_writer::text_chapters(&text, &meta.title),
                None,
            )
        }
        (BookFormat::Md, "epub") => {
            let text = epub_writer::decode_text(&std::fs::read(input).map_err(|e| e.to_string())?);
            epub_writer::write_reflowable(
                out,
                &meta,
                &epub_writer::markdown_chapters(&text, &meta.title),
                None,
            )
        }
        (BookFormat::Html, "epub") => {
            let text = epub_writer::decode_text(&std::fs::read(input).map_err(|e| e.to_string())?);
            epub_writer::write_reflowable(
                out,
                &meta,
                &[epub_writer::html_chapter(&text, &meta.title)],
                None,
            )
        }
        (f, "cbz" | "epub") if f.is_comic() => {
            let path = input.to_string_lossy().to_string();
            let n = server::page_count(&path);
            if n == 0 {
                return Err("no pages in this comic".into());
            }
            let raw: Vec<Vec<u8>> = (0..n)
                .filter_map(|i| server::read_page(&path, i).map(|(b, _)| b))
                .collect();
            let rtl = opts.rtl || meta.rtl;
            let pages = prepare_pages(&raw, opts, rtl);
            if to == "cbz" {
                let named: Vec<(String, Vec<u8>)> = pages
                    .into_iter()
                    .enumerate()
                    .map(|(i, (b, _, _))| (format!("{i}.jpg"), b))
                    .collect();
                let ci = omniget_core::core::read::comic::ComicInfo {
                    title: meta.title.clone(),
                    series: meta.series.clone(),
                    number: meta.series_index.map(|x| x.to_string()).unwrap_or_default(),
                    writer: meta.authors.join(", "),
                    summary: meta.description.clone(),
                    manga: if rtl {
                        "YesAndRightToLeft".into()
                    } else {
                        String::new()
                    },
                    page_count: Some(named.len() as i64),
                    ..Default::default()
                };
                omniget_core::core::read::comic::write_cbz(out, &named, Some(&ci))
            } else {
                let meta = EpubMeta { rtl, ..meta };
                epub_writer::write_fixed_layout(out, &meta, &pages, &[])
            }
        }
        _ => Err(format!("no native conversion from {from:?} to {to}")),
    }
}

/// "KCC lite": per page, trim uniform borders, split landscape spreads in
/// reading order, grayscale for e-ink and fit the device screen.
fn prepare_pages(raw: &[Vec<u8>], opts: &ComicOpts, rtl: bool) -> Vec<(Vec<u8>, u32, u32)> {
    let (dw, dh, color) = DEVICES
        .iter()
        .find(|d| d.0 == opts.device)
        .map(|d| (d.2, d.3, d.4))
        .unwrap_or((0, 0, true));
    let quality = if opts.quality == 0 {
        85
    } else {
        opts.quality.min(100)
    };
    let mut out = Vec::new();
    for bytes in raw {
        let Ok(img) = image::load_from_memory(bytes) else {
            continue;
        };
        let img = if opts.crop { crop_borders(img) } else { img };
        let parts = if opts.split && img.width() as f64 > img.height() as f64 * 1.16 {
            let half = img.width() / 2;
            let left = img.crop_imm(0, 0, half, img.height());
            let right = img.crop_imm(half, 0, img.width() - half, img.height());
            if rtl {
                vec![right, left]
            } else {
                vec![left, right]
            }
        } else {
            vec![img]
        };
        for mut p in parts {
            if dw > 0 && dh > 0 {
                let fits_smaller = p.width() > dw || p.height() > dh;
                if fits_smaller || opts.upscale {
                    p = p.resize(dw, dh, image::imageops::FilterType::Lanczos3);
                }
            }
            let (w, h) = (p.width(), p.height());
            let mut buf = std::io::Cursor::new(Vec::new());
            let flat = if opts.grayscale || !color {
                image::DynamicImage::ImageLuma8(p.to_luma8())
            } else {
                image::DynamicImage::ImageRgb8(p.to_rgb8())
            };
            let ok = flat
                .write_with_encoder(image::codecs::jpeg::JpegEncoder::new_with_quality(
                    &mut buf, quality,
                ))
                .is_ok();
            if ok {
                out.push((buf.into_inner(), w, h));
            }
        }
    }
    out
}

/// Trims rows/columns that are all near the corner colour (white or black
/// margins), at most 10% per side like KCC.
pub fn crop_borders(img: image::DynamicImage) -> image::DynamicImage {
    let g = img.to_luma8();
    let (w, h) = g.dimensions();
    if w < 20 || h < 20 {
        return img;
    }
    let bg = g.get_pixel(0, 0)[0] as i32;
    let tol = 24;
    let row_blank = |y: u32| {
        (0..w)
            .step_by(2)
            .all(|x| (g.get_pixel(x, y)[0] as i32 - bg).abs() <= tol)
    };
    let col_blank = |x: u32| {
        (0..h)
            .step_by(2)
            .all(|y| (g.get_pixel(x, y)[0] as i32 - bg).abs() <= tol)
    };
    let (max_x, max_y) = (w / 10, h / 10);
    let mut top = 0;
    while top < max_y && row_blank(top) {
        top += 1;
    }
    let mut bottom = 0;
    while bottom < max_y && row_blank(h - 1 - bottom) {
        bottom += 1;
    }
    let mut left = 0;
    while left < max_x && col_blank(left) {
        left += 1;
    }
    let mut right = 0;
    while right < max_x && col_blank(w - 1 - right) {
        right += 1;
    }
    if top + bottom + left + right == 0 {
        return img;
    }
    img.crop_imm(left, top, w - left - right, h - top - bottom)
}

/// Joins a series' downloaded chapters (in order) into one CBZ or
/// fixed-layout EPUB volume, with a TOC entry per chapter.
#[tauri::command]
pub async fn read_merge_chapters(
    app: AppHandle,
    chapter_ids: Vec<i64>,
    to: String,
    title: String,
    opts: Option<ComicOpts>,
) -> Result<i64, String> {
    let mut j = job(&format!("{title} → {}", to.to_uppercase()));
    put(&app, &j);
    let opts = opts.unwrap_or_default();
    let res = blocking(move || {
        let mut chapters: Vec<db::ChapterRow> = chapter_ids
            .iter()
            .filter_map(|id| db::chapter(*id).ok())
            .collect();
        chapters.sort_by_key(|c| c.position);
        let series = chapters
            .first()
            .map(|c| db::series(c.series_id))
            .transpose()?
            .ok_or("no chapters")?;
        let mut raw = Vec::new();
        let mut marks = Vec::new();
        for ch in &chapters {
            let Some(b) = ch.book_id.and_then(|b| db::book(b).ok()) else {
                return Err(format!(
                    "{} is not downloaded",
                    super::manga::chapter_label(ch)
                ));
            };
            marks.push((raw.len(), super::manga::chapter_label(ch)));
            let n = server::page_count(&b.path);
            for i in 0..n {
                if let Some((bytes, _)) = server::read_page(&b.path, i) {
                    raw.push(bytes);
                }
            }
        }
        let rtl = series.reader["mode"].as_str() == Some("rtl") || opts.rtl;
        let dir = PathBuf::from(super::default_download_dir())
            .join("Manga")
            .join(sanitize_filename::sanitize(&series.title));
        let out = dir.join(sanitize_filename::sanitize(format!(
            "{title}.{}",
            if to == "cbz" { "cbz" } else { "epub" }
        )));
        let meta = EpubMeta {
            title: title.clone(),
            authors: series.authors.clone(),
            description: series.description.clone(),
            series: series.title.clone(),
            rtl,
            ..Default::default()
        };
        if to == "cbz" && opts.device.is_empty() && !opts.crop && !opts.grayscale {
            let named: Vec<(String, Vec<u8>)> = raw
                .into_iter()
                .enumerate()
                .map(|(i, b)| (format!("{i}.{}", sniff_ext(&b)), b))
                .collect();
            let ci = omniget_core::core::read::comic::ComicInfo {
                title: title.clone(),
                series: series.title.clone(),
                writer: series.authors.join(", "),
                summary: series.description.clone(),
                manga: if rtl {
                    "YesAndRightToLeft".into()
                } else {
                    String::new()
                },
                page_count: Some(named.len() as i64),
                ..Default::default()
            };
            omniget_core::core::read::comic::write_cbz(&out, &named, Some(&ci))?;
        } else {
            let pages = prepare_pages(&raw, &opts, rtl);
            if to == "cbz" {
                let named: Vec<(String, Vec<u8>)> = pages
                    .into_iter()
                    .enumerate()
                    .map(|(i, (b, _, _))| (format!("{i}.jpg"), b))
                    .collect();
                omniget_core::core::read::comic::write_cbz(&out, &named, None)?;
            } else {
                epub_writer::write_fixed_layout(&out, &meta, &pages, &marks)?;
            }
        }
        scan::index_file(
            &out,
            &scan::IndexOpts {
                source: "manga".into(),
                series_id: Some(series.id),
                ..Default::default()
            },
        )
    })
    .await;
    match res {
        Ok(id) => {
            j.state = "done".into();
            j.progress = 1.0;
            j.book_id = Some(id);
            put(&app, &j);
            let _ = app.emit("read-library-changed", ());
            Ok(id)
        }
        Err(e) => {
            j.state = "failed".into();
            j.message = e.clone();
            put(&app, &j);
            Err(e)
        }
    }
}

fn sniff_ext(b: &[u8]) -> &'static str {
    let name = if b.starts_with(b"\x89PNG") {
        "x.png"
    } else if b.get(8..12) == Some(b"WEBP") {
        "x.webp"
    } else {
        "x.jpg"
    };
    &format::image_mime(name)[6..]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crops_white_margins() {
        let mut img = image::RgbImage::from_pixel(100, 100, image::Rgb([255, 255, 255]));
        for x in 5..95 {
            for y in 3..97 {
                img.put_pixel(x, y, image::Rgb([0, 0, 0]));
            }
        }
        let out = crop_borders(image::DynamicImage::ImageRgb8(img));
        assert_eq!((out.width(), out.height()), (90, 94));
    }

    #[test]
    fn engine_choice() {
        assert_eq!(pick_engine(Some(BookFormat::Txt), "epub", "auto"), "native");
        assert_eq!(pick_engine(Some(BookFormat::Cbz), "epub", "auto"), "native");
        assert_eq!(
            pick_engine(Some(BookFormat::Epub), "mobi", "calibre"),
            "calibre"
        );
        assert_eq!(
            pick_engine(Some(BookFormat::Epub), "m4b", "auto"),
            "audiobook"
        );
        assert_eq!(iso3("pt-BR"), Some("por"));
    }

    #[test]
    fn splits_spreads_in_reading_order() {
        let mut spread = image::RgbImage::from_pixel(200, 100, image::Rgb([255, 255, 255]));
        for x in 0..100 {
            for y in 0..100 {
                spread.put_pixel(x, y, image::Rgb([10, 10, 10]));
            }
        }
        let mut buf = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgb8(spread)
            .write_to(&mut buf, image::ImageFormat::Png)
            .unwrap();
        let opts = ComicOpts {
            split: true,
            ..Default::default()
        };
        let pages = prepare_pages(&[buf.into_inner()], &opts, true);
        assert_eq!(pages.len(), 2);
        // RTL: the right (white) half comes first.
        let first = image::load_from_memory(&pages[0].0).unwrap().to_luma8();
        assert!(first.get_pixel(50, 50)[0] > 200);
        let second = image::load_from_memory(&pages[1].0).unwrap().to_luma8();
        assert!(second.get_pixel(50, 50)[0] < 40);
    }
}

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde::Serialize;
use tokio::io::AsyncReadExt;
use tokio_util::sync::CancellationToken;

use super::formats;
use super::graph::Engine;

#[derive(Serialize, Clone, Default, Debug, PartialEq, Eq)]
pub struct ToolStatus {
    pub ffmpeg: bool,
    pub pandoc: bool,
    pub imagemagick: bool,
    pub libreoffice: bool,
}

impl ToolStatus {
    pub fn all() -> Self {
        Self {
            ffmpeg: true,
            pandoc: true,
            imagemagick: true,
            libreoffice: true,
        }
    }

    pub fn has(&self, engine: Engine) -> bool {
        match engine {
            Engine::Native => true,
            Engine::Ffmpeg => self.ffmpeg,
            Engine::Pandoc => self.pandoc,
            Engine::ImageMagick => self.imagemagick,
            Engine::LibreOffice => self.libreoffice,
        }
    }
}

struct Cached {
    at: Instant,
    status: ToolStatus,
    soffice: Option<PathBuf>,
}

static CACHE: Mutex<Option<Cached>> = Mutex::new(None);
const TTL: Duration = Duration::from_secs(60);

pub fn reset_cache() {
    if let Ok(mut c) = CACHE.lock() {
        *c = None;
    }
    crate::core::ffmpeg::reset_ffmpeg_available_cache();
}

pub async fn detect_tools() -> ToolStatus {
    if let Ok(c) = CACHE.lock() {
        if let Some(cached) = c.as_ref().filter(|c| c.at.elapsed() < TTL) {
            return cached.status.clone();
        }
    }
    let (ffmpeg, pandoc, imagemagick, soffice) = tokio::join!(
        crate::core::ffmpeg::is_ffmpeg_available(),
        responds("pandoc"),
        responds("magick"),
        find_soffice(),
    );
    let status = ToolStatus {
        ffmpeg,
        pandoc,
        imagemagick,
        libreoffice: soffice.is_some(),
    };
    if let Ok(mut c) = CACHE.lock() {
        *c = Some(Cached {
            at: Instant::now(),
            status: status.clone(),
            soffice,
        });
    }
    status
}

async fn responds(program: impl AsRef<std::ffi::OsStr>) -> bool {
    let mut cmd = crate::core::process::command(program);
    cmd.arg("--version")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    matches!(
        tokio::time::timeout(Duration::from_secs(15), cmd.status()).await,
        Ok(Ok(s)) if s.success()
    )
}

async fn find_soffice() -> Option<PathBuf> {
    let mut candidates: Vec<PathBuf> = vec![PathBuf::from("soffice")];
    if cfg!(target_os = "macos") {
        candidates.push(PathBuf::from(
            "/Applications/LibreOffice.app/Contents/MacOS/soffice",
        ));
    }
    if cfg!(windows) {
        candidates.push(PathBuf::from(
            r"C:\Program Files\LibreOffice\program\soffice.exe",
        ));
        candidates.push(PathBuf::from(
            r"C:\Program Files (x86)\LibreOffice\program\soffice.exe",
        ));
    }
    for c in candidates {
        if c.is_absolute() && !c.is_file() {
            continue;
        }
        if responds(&c).await {
            return Some(c);
        }
    }
    None
}

async fn soffice_program() -> Option<PathBuf> {
    if let Ok(c) = CACHE.lock() {
        if let Some(cached) = c.as_ref() {
            if cached.soffice.is_some() {
                return cached.soffice.clone();
            }
        }
    }
    find_soffice().await
}

pub fn tool_name(engine: Engine) -> &'static str {
    match engine {
        Engine::Native => "OmniGet",
        Engine::Ffmpeg => "FFmpeg",
        Engine::Pandoc => "Pandoc",
        Engine::ImageMagick => "ImageMagick",
        Engine::LibreOffice => "LibreOffice",
    }
}

pub async fn run_tool(
    mut cmd: tokio::process::Command,
    name: &str,
    cancel: &CancellationToken,
) -> Result<(), String> {
    cmd.stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    let mut child = cmd
        .spawn()
        .map_err(|e| format!("Could not start {name}: {e}. Check that it is installed."))?;
    let mut stderr = child.stderr.take();
    let reader = tokio::spawn(async move {
        let mut buf = Vec::new();
        if let Some(s) = stderr.as_mut() {
            let _ = s.read_to_end(&mut buf).await;
        }
        buf
    });
    let status = tokio::select! {
        s = child.wait() => s.map_err(|e| format!("{name} failed: {e}."))?,
        _ = cancel.cancelled() => {
            let _ = child.kill().await;
            return Err("Cancelled.".to_string());
        }
    };
    let err = reader.await.unwrap_or_default();
    if status.success() {
        return Ok(());
    }
    Err(format!(
        "{name} could not convert this file: {}",
        tail(&String::from_utf8_lossy(&err))
    ))
}

pub fn tail(stderr: &str) -> String {
    let line = stderr
        .lines()
        .map(str::trim)
        .rfind(|l| !l.is_empty())
        .unwrap_or("unknown error");
    let short: String = line.chars().take(240).collect();
    if short.ends_with('.') {
        short
    } else {
        format!("{short}.")
    }
}

fn pandoc_reader(id: &str) -> &str {
    match id {
        "md" | "txt" => "markdown",
        "tex" => "latex",
        other => other,
    }
}

fn pandoc_writer(id: &str) -> &str {
    match id {
        "md" => "gfm",
        "txt" => "plain",
        "tex" => "latex",
        other => other,
    }
}

pub async fn pandoc(
    input: &Path,
    from: &str,
    output: &Path,
    to: &str,
    cancel: &CancellationToken,
) -> Result<(), String> {
    let mut cmd = crate::core::process::command("pandoc");
    cmd.arg("-f")
        .arg(pandoc_reader(from))
        .arg("-t")
        .arg(pandoc_writer(to))
        .arg("-s")
        .arg(input)
        .arg("-o")
        .arg(output);
    if let Some(stem) = input.file_stem().and_then(|s| s.to_str()) {
        cmd.arg("--metadata").arg(format!("pagetitle={stem}"));
    }
    run_tool(cmd, "Pandoc", cancel).await
}

fn svg_patterns() -> &'static [regex::Regex] {
    static RE: std::sync::OnceLock<Vec<regex::Regex>> = std::sync::OnceLock::new();
    RE.get_or_init(|| {
        [
            r#"(?i)(?:^|[\s<"'])(?:xlink:href|href|src)\s*=\s*"([^"]*)""#,
            r#"(?i)(?:^|[\s<"'])(?:xlink:href|href|src)\s*=\s*'([^']*)'"#,
            r#"(?i)url\(\s*["']?([^"')]*)"#,
            r#"(?i)@import\s+(?:url\()?\s*["']?([^"');\s]*)"#,
        ]
        .iter()
        .filter_map(|p| regex::Regex::new(p).ok())
        .collect()
    })
}

fn svg_declaration() -> &'static regex::Regex {
    static RE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    RE.get_or_init(|| {
        regex::Regex::new(
            r"(?is)<!(?:ENTITY[^>]*\b(?:SYSTEM|PUBLIC)\b|DOCTYPE[^>\[]*\bSYSTEM\b)[^>]*",
        )
        .expect("static regex")
    })
}

fn svg_refusal(what: &str) -> String {
    let what: String = what.trim().chars().take(80).collect();
    format!("This SVG links to files outside itself ({what}). For safety it is not converted.")
}

pub fn check_svg(text: &str) -> Result<(), String> {
    if let Some(m) = svg_declaration().find(text) {
        return Err(svg_refusal(m.as_str()));
    }
    for re in svg_patterns() {
        for cap in re.captures_iter(text) {
            let target = cap.get(1).map(|m| m.as_str().trim()).unwrap_or("");
            let lower = target.to_ascii_lowercase();
            if target.is_empty() || target.starts_with('#') || lower.starts_with("data:") {
                continue;
            }
            return Err(svg_refusal(target));
        }
    }
    Ok(())
}

pub async fn magick(
    input: &Path,
    from: &str,
    output: &Path,
    quality: Option<u8>,
    cancel: &CancellationToken,
) -> Result<(), String> {
    if from == "svg" {
        let bytes = std::fs::read(input).map_err(|e| format!("Could not read the file: {e}."))?;
        check_svg(&crate::core::read::epub_writer::decode_text(&bytes))?;
    }
    let mut cmd = crate::core::process::command("magick");
    if from == "svg" {
        cmd.arg("-background")
            .arg("none")
            .arg("-density")
            .arg("192");
    }
    cmd.arg(input);
    if let Some(q) = quality {
        cmd.arg("-quality").arg(q.to_string());
    }
    cmd.arg(output);
    run_tool(cmd, "ImageMagick", cancel).await
}

pub async fn soffice(
    input: &Path,
    output: &Path,
    to: &str,
    cancel: &CancellationToken,
) -> Result<(), String> {
    let program = soffice_program().await.ok_or_else(|| {
        "LibreOffice is not installed. Install it to convert office documents.".to_string()
    })?;
    let work = tempfile::tempdir().map_err(|e| format!("Could not create a temp folder: {e}."))?;
    let outdir = work.path().join("out");
    std::fs::create_dir_all(&outdir).map_err(|e| e.to_string())?;
    let profile = url::Url::from_directory_path(work.path().join("profile"))
        .map(|u| u.to_string())
        .unwrap_or_default();
    let mut cmd = crate::core::process::command(&program);
    cmd.arg("--headless")
        .arg("--norestore")
        .arg(format!("-env:UserInstallation={profile}"))
        .arg("--convert-to")
        .arg(formats::extension_of(to))
        .arg("--outdir")
        .arg(&outdir)
        .arg(input);
    run_tool(cmd, "LibreOffice", cancel).await?;
    let produced = std::fs::read_dir(&outdir)
        .map_err(|e| e.to_string())?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .find(|p| p.is_file())
        .ok_or_else(|| {
            "LibreOffice finished but wrote no file. Try opening and re-saving the document."
                .to_string()
        })?;
    super::move_file(&produced, output)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn has_maps_engines() {
        let t = ToolStatus {
            ffmpeg: true,
            ..Default::default()
        };
        assert!(t.has(Engine::Native));
        assert!(t.has(Engine::Ffmpeg));
        assert!(!t.has(Engine::Pandoc));
        assert!(ToolStatus::all().has(Engine::LibreOffice));
    }

    #[test]
    fn svg_with_outside_links_is_refused() {
        let ok = r##"<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink"><defs><linearGradient id="g"/></defs><rect fill="url(#g)"/><use xlink:href="#g"/><image href="data:image/png;base64,AAAA"/></svg>"##;
        assert!(check_svg(ok).is_ok());
        let public_doctype = r#"<!DOCTYPE svg PUBLIC "-//W3C//DTD SVG 1.1//EN" "http://www.w3.org/Graphics/SVG/1.1/DTD/svg11.dtd"><svg xmlns="http://www.w3.org/2000/svg"/>"#;
        assert!(check_svg(public_doctype).is_ok());
        for bad in [
            r#"<svg><image href="/abs/secret.png"/></svg>"#,
            r#"<svg><image xlink:href='file:///etc/passwd'/></svg>"#,
            r#"<svg><rect style="fill:url(/tmp/x.svg#a)"/></svg>"#,
            r#"<svg><style>@import "http://evil/x.css";</style></svg>"#,
            r#"<!DOCTYPE svg [<!ENTITY x SYSTEM "file:///etc/passwd">]><svg>&x;</svg>"#,
            r#"<!DOCTYPE svg SYSTEM "file:///etc/x.dtd"><svg/>"#,
            r#"<svg><image src="secret.png"/></svg>"#,
        ] {
            let err = check_svg(bad).unwrap_err();
            assert!(
                err.starts_with("This SVG links to files outside itself ("),
                "{bad}"
            );
            assert!(err.ends_with("For safety it is not converted."));
        }
    }

    #[test]
    fn tail_keeps_last_line() {
        assert_eq!(tail("a\nboom\n\n"), "boom.");
        assert_eq!(tail(""), "unknown error.");
    }
}

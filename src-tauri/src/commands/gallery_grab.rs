//! Downloads › Tools › Gallery: every image (or video) of a page, through
//! gallery-dl, for any link gallery-dl understands, not only the hosts the
//! main queue routes to it. Filters by kind of file and minimum size.
//!
//! Mechanism from gallery-dl itself (`--filter`, `--filesize-min`, `-d` with
//! one folder per site and gallery); the code here only builds its command
//! line and reads the paths it prints.

use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Mutex;

use serde::Serialize;
use tauri::Emitter;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio_util::sync::CancellationToken;

const IMAGE_EXTS: &[&str] = &[
    "jpg", "jpeg", "png", "gif", "webp", "avif", "bmp", "tif", "tiff", "heic", "jxl",
];
const VIDEO_EXTS: &[&str] = &["mp4", "webm", "mkv", "mov", "m4v", "avi"];

static CURRENT: Mutex<Option<CancellationToken>> = Mutex::new(None);

#[derive(Debug, Clone, Serialize)]
pub struct GalleryGrabResult {
    pub folder: String,
    pub files: usize,
    pub bytes: u64,
    /// Files gallery-dl skipped because they were already there.
    pub skipped: usize,
}

#[derive(Debug, Clone, Serialize)]
struct GalleryGrabProgress {
    files: usize,
    skipped: usize,
    last: String,
}

fn quoted(exts: &[&str]) -> String {
    exts.iter()
        .map(|e| format!("'{e}'"))
        .collect::<Vec<_>>()
        .join(", ")
}

/// The `--filter` expression for the kinds of file the person kept
/// (`images`, `videos`, `other`). `None` keeps everything.
pub fn kind_filter(kinds: &[String]) -> Option<String> {
    let has = |k: &str| kinds.iter().any(|x| x == k);
    let (img, vid, other) = (has("images"), has("videos"), has("other"));
    if (img && vid && other) || (!img && !vid && !other) {
        return None;
    }
    if other {
        let mut dropped: Vec<&str> = Vec::new();
        if !img {
            dropped.extend(IMAGE_EXTS);
        }
        if !vid {
            dropped.extend(VIDEO_EXTS);
        }
        return Some(format!("extension.lower() not in ({})", quoted(&dropped)));
    }
    let mut kept: Vec<&str> = Vec::new();
    if img {
        kept.extend(IMAGE_EXTS);
    }
    if vid {
        kept.extend(VIDEO_EXTS);
    }
    Some(format!("extension.lower() in ({})", quoted(&kept)))
}

/// gallery-dl arguments for one run.
pub fn grab_args(
    url: &str,
    output_dir: &str,
    kinds: &[String],
    min_kb: Option<u32>,
    cookies: Option<&std::path::Path>,
) -> Vec<String> {
    let mut a = vec![
        "--no-colors".to_string(),
        "-d".to_string(),
        output_dir.to_string(),
    ];
    if let Some(expr) = kind_filter(kinds) {
        a.push("--filter".to_string());
        a.push(expr);
    }
    if let Some(kb) = min_kb.filter(|kb| *kb > 0) {
        a.push("--filesize-min".to_string());
        a.push(format!("{kb}k"));
    }
    if let Some(c) = cookies {
        a.push("--cookies".to_string());
        a.push(c.to_string_lossy().into_owned());
    }
    a.push("--".to_string());
    a.push(url.to_string());
    a
}

#[tauri::command]
pub async fn gallery_grab(
    app: tauri::AppHandle,
    url: String,
    output_dir: String,
    kinds: Vec<String>,
    min_kb: Option<u32>,
) -> Result<GalleryGrabResult, String> {
    let url = url.trim().to_string();
    if !(url.starts_with("http://") || url.starts_with("https://")) {
        return Err("Paste a web address that starts with http:// or https://".to_string());
    }
    omniget_core::core::platform_optout::ensure_allowed(&url)?;
    if output_dir.trim().is_empty() {
        return Err("Choose a download folder first".to_string());
    }
    std::fs::create_dir_all(&output_dir).map_err(|e| e.to_string())?;
    let bin = omniget_core::core::dependencies::ensure_gallerydl()
        .await
        .ok_or_else(|| "gallery-dl is not available on this computer".to_string())?;

    let cookies = omniget_core::core::paths::app_data_dir()
        .map(|d| d.join("chrome-extension-cookies.txt"))
        .filter(|f| std::fs::metadata(f).map(|m| m.len() > 0).unwrap_or(false));

    let token = CancellationToken::new();
    {
        let mut cur = CURRENT.lock().map_err(|e| e.to_string())?;
        if cur.is_some() {
            return Err("A gallery is already downloading".to_string());
        }
        *cur = Some(token.clone());
    }
    let result = run(
        &app,
        &bin,
        &url,
        &output_dir,
        &kinds,
        min_kb,
        cookies,
        token,
    )
    .await;
    if let Ok(mut cur) = CURRENT.lock() {
        *cur = None;
    }
    result
}

#[allow(clippy::too_many_arguments)]
async fn run(
    app: &tauri::AppHandle,
    bin: &std::path::Path,
    url: &str,
    output_dir: &str,
    kinds: &[String],
    min_kb: Option<u32>,
    cookies: Option<PathBuf>,
    token: CancellationToken,
) -> Result<GalleryGrabResult, String> {
    let mut cmd = omniget_core::core::process::command(bin);
    cmd.args(grab_args(
        url,
        output_dir,
        kinds,
        min_kb,
        cookies.as_deref(),
    ))
    .stdout(Stdio::piped())
    .stderr(Stdio::piped())
    .kill_on_drop(true);
    let mut child = cmd
        .spawn()
        .map_err(|e| format!("Failed to start gallery-dl: {e}"))?;
    let stdout = child.stdout.take().ok_or("No output from gallery-dl")?;
    let stderr = child.stderr.take().ok_or("No output from gallery-dl")?;

    let app2 = app.clone();
    let reader = tokio::spawn(async move {
        let mut saved: Vec<PathBuf> = Vec::new();
        let mut skipped = 0usize;
        let mut lines = BufReader::new(stdout).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            let line = line.trim();
            if line.is_empty() || line.starts_with('[') {
                continue;
            }
            // gallery-dl prints each saved path, and `# <path>` for a skipped one.
            if line.starts_with('#') {
                skipped += 1;
            } else {
                saved.push(PathBuf::from(line));
            }
            let last = std::path::Path::new(line.trim_start_matches("# "))
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            let _ = app2.emit(
                "gallery-grab-progress",
                GalleryGrabProgress {
                    files: saved.len(),
                    skipped,
                    last,
                },
            );
        }
        (saved, skipped)
    });
    let err_reader = tokio::spawn(async move {
        let mut buf = String::new();
        let mut lines = BufReader::new(stderr).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            buf.push_str(&line);
            buf.push('\n');
        }
        buf
    });

    let status = tokio::select! {
        s = child.wait() => s.map_err(|e| e.to_string())?,
        _ = token.cancelled() => {
            let _ = child.kill().await;
            return Err("Cancelled".to_string());
        }
    };
    let (saved, skipped) = reader.await.unwrap_or_default();
    let stderr_text = err_reader.await.unwrap_or_default();
    let bytes: u64 = saved
        .iter()
        .filter_map(|p| std::fs::metadata(p).ok())
        .map(|m| m.len())
        .sum();
    // The folder of the first saved file: gallery-dl makes one per site and gallery.
    let folder = saved
        .first()
        .and_then(|p| p.parent())
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|| output_dir.to_string());

    if !status.success() && saved.is_empty() {
        let detail = stderr_text
            .lines()
            .rev()
            .find(|l| l.contains("[error]"))
            .map(|l| l.split("[error]").last().unwrap_or(l).trim().to_string())
            .unwrap_or_else(|| "gallery-dl could not read this page".to_string());
        return Err(detail);
    }
    Ok(GalleryGrabResult {
        folder,
        files: saved.len(),
        bytes,
        skipped,
    })
}

#[tauri::command]
pub fn gallery_grab_cancel() -> bool {
    match CURRENT.lock() {
        Ok(cur) => cur.as_ref().map(|t| t.cancel()).is_some(),
        Err(_) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn k(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn everything_or_nothing_means_no_filter() {
        assert_eq!(kind_filter(&k(&["images", "videos", "other"])), None);
        assert_eq!(kind_filter(&[]), None);
    }

    #[test]
    fn images_only_keeps_image_extensions() {
        let f = kind_filter(&k(&["images"])).unwrap();
        assert!(f.starts_with("extension.lower() in ("));
        assert!(f.contains("'png'") && !f.contains("'mp4'"));
    }

    #[test]
    fn other_drops_the_unticked_groups() {
        let f = kind_filter(&k(&["images", "other"])).unwrap();
        assert!(f.starts_with("extension.lower() not in ("));
        assert!(f.contains("'mp4'") && !f.contains("'png'"));
    }

    #[test]
    fn builds_the_command_line() {
        let a = grab_args("https://x/y", "/tmp/o", &k(&["images"]), Some(50), None);
        assert_eq!(&a[..3], ["--no-colors", "-d", "/tmp/o"]);
        assert!(a.windows(2).any(|w| w == ["--filesize-min", "50k"]));
        assert_eq!(&a[a.len() - 2..], ["--", "https://x/y"]);
        let a = grab_args("https://x/y", "/tmp/o", &[], Some(0), None);
        assert!(!a.iter().any(|s| s == "--filter" || s == "--filesize-min"));
    }
}

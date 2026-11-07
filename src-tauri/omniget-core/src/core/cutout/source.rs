use std::path::Path;
use std::process::Stdio;

use image::DynamicImage;
use serde::{Deserialize, Serialize};

use crate::core::convert::formats::{self, Category};

const MAX_FILE_BYTES: u64 = 256 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    Image,
    Video,
}

#[derive(Debug, Clone, Serialize)]
pub struct Info {
    pub path: String,
    pub name: String,
    pub kind: Kind,
    pub width: u32,
    pub height: u32,
    pub duration_ms: u64,
}

pub fn kind_of(path: &Path) -> Kind {
    match formats::detect(path).map(|f| f.category) {
        Some(Category::Video) => Kind::Video,
        _ => Kind::Image,
    }
}

pub fn key(path: &Path, frame_ms: Option<u64>) -> String {
    let stamp = std::fs::metadata(path)
        .ok()
        .map(|m| {
            let modified = m
                .modified()
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_millis())
                .unwrap_or(0);
            format!("{}:{modified}", m.len())
        })
        .unwrap_or_default();
    let frame = frame_ms.map(|ms| ms.to_string()).unwrap_or_default();
    format!("{}|{frame}|{stamp}", path.to_string_lossy())
}

fn name_of(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

fn checked(path: &Path) -> Result<(), String> {
    let meta = std::fs::metadata(path).map_err(|_| {
        "The file could not be opened. It may have been moved or deleted.".to_string()
    })?;
    if !meta.is_file() {
        return Err("This is a folder. Pick a picture or a video.".to_string());
    }
    if meta.len() == 0 {
        return Err("The file is empty (0 bytes).".to_string());
    }
    Ok(())
}

async fn ffmpeg_frame(path: &Path, frame_ms: Option<u64>) -> Result<DynamicImage, String> {
    let mut cmd = crate::core::process::command("ffmpeg");
    cmd.kill_on_drop(true);
    cmd.args(["-hide_banner", "-loglevel", "error", "-nostdin"]);
    if let Some(ms) = frame_ms {
        cmd.args(["-ss", &format!("{:.3}", ms as f64 / 1000.0)]);
    }
    let out = cmd
        .arg("-i")
        .arg(path)
        .args(["-frames:v", "1", "-f", "image2pipe", "-c:v", "png", "-"])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .await
        .map_err(|_| {
            "FFmpeg is not installed, so this file cannot be read. Install it in Settings > Dependencies."
                .to_string()
        })?;
    if !out.status.success() || out.stdout.is_empty() {
        return Err(match frame_ms {
            Some(_) => "There is no picture at this point of the video. Pick an earlier moment."
                .to_string(),
            None => "This file could not be read as a picture.".to_string(),
        });
    }
    image::load_from_memory(&out.stdout).map_err(|e| format!("This frame could not be read: {e}."))
}

pub async fn load(path: &Path, frame_ms: Option<u64>) -> Result<DynamicImage, String> {
    checked(path)?;
    if kind_of(path) == Kind::Video {
        return ffmpeg_frame(path, Some(frame_ms.unwrap_or(0))).await;
    }
    let size = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
    if size > MAX_FILE_BYTES {
        return Err("This picture is too large to open (over 256 MB).".to_string());
    }
    let bytes = tokio::fs::read(path)
        .await
        .map_err(|e| format!("The file could not be read: {e}."))?;
    let from = formats::detect(path).map(|f| f.id).unwrap_or("");
    let decoded =
        tokio::task::spawn_blocking(move || crate::core::convert::image::decode(&bytes, from))
            .await
            .map_err(|e| format!("The picture could not be read: {e}."))?;
    match decoded {
        Ok(img) => Ok(img),
        Err(native) => ffmpeg_frame(path, None).await.map_err(|_| native),
    }
}

pub async fn inspect(path: &Path) -> Result<Info, String> {
    checked(path)?;
    let kind = kind_of(path);
    let (width, height, duration_ms) = match kind {
        Kind::Video => {
            let probe = crate::core::studio::probe(path).await?;
            (probe.width, probe.height, probe.duration_ms)
        }
        Kind::Image => {
            let img = load(path, None).await?;
            (img.width(), img.height(), 0)
        }
    };
    Ok(Info {
        path: path.to_string_lossy().into_owned(),
        name: name_of(path),
        kind,
        width,
        height,
        duration_ms,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn videos_are_told_apart_by_extension() {
        assert_eq!(kind_of(Path::new("/x/clip.MP4")), Kind::Video);
        assert_eq!(kind_of(Path::new("/x/clip.mkv")), Kind::Video);
        assert_eq!(kind_of(Path::new("/x/photo.jpg")), Kind::Image);
        assert_eq!(kind_of(Path::new("/x/photo.heic")), Kind::Image);
    }

    #[test]
    fn the_key_changes_with_the_frame_and_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("a.png");
        std::fs::write(&p, b"one").unwrap();
        let first = key(&p, None);
        assert_ne!(first, key(&p, Some(1000)));
        std::fs::write(&p, b"three").unwrap();
        assert_ne!(first, key(&p, None));
    }

    #[tokio::test]
    async fn a_picture_loads_and_reports_its_size() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("dot.png");
        image::RgbImage::from_pixel(12, 7, image::Rgb([9, 9, 9]))
            .save(&p)
            .unwrap();
        let info = inspect(&p).await.unwrap();
        assert_eq!((info.width, info.height, info.kind), (12, 7, Kind::Image));
        assert!(inspect(dir.path()).await.is_err());
        let empty = dir.path().join("empty.png");
        std::fs::write(&empty, b"").unwrap();
        assert!(load(&empty, None).await.is_err());
    }
}

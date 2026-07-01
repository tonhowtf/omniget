use std::path::{Path, PathBuf};

use crate::core::process;

use super::super::api::{BilibiliError, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Container {
    Mp4,
    Mkv,
}

impl Container {
    pub fn extension(self) -> &'static str {
        match self {
            Container::Mp4 => "mp4",
            Container::Mkv => "mkv",
        }
    }
}

pub struct MuxInputs<'a> {
    pub video: &'a Path,
    pub audio: &'a Path,
    pub cover: Option<&'a Path>,
    pub output: &'a Path,
    pub container: Container,
}

pub async fn mux(inputs: MuxInputs<'_>) -> Result<PathBuf> {
    if let Some(parent) = inputs.output.parent() {
        std::fs::create_dir_all(parent).ok();
    }

    let mut args: Vec<String> = Vec::new();
    args.push("-y".into());
    args.push("-i".into());
    args.push(inputs.video.to_string_lossy().into_owned());
    args.push("-i".into());
    args.push(inputs.audio.to_string_lossy().into_owned());

    if let Some(cover_path) = inputs.cover {
        args.push("-i".into());
        args.push(cover_path.to_string_lossy().into_owned());
    }

    args.push("-map".into());
    args.push("0:v:0".into());
    args.push("-map".into());
    args.push("1:a:0".into());

    if inputs.cover.is_some() {
        args.push("-map".into());
        args.push("2:v:0".into());
    }

    args.push("-c:v".into());
    args.push("copy".into());
    args.push("-c:a".into());
    args.push("copy".into());

    if inputs.cover.is_some() && inputs.container == Container::Mp4 {
        args.push("-c:v:1".into());
        args.push("mjpeg".into());
        args.push("-disposition:v:1".into());
        args.push("attached_pic".into());
        args.push("-pix_fmt:v:1".into());
        args.push("yuvj420p".into());
    }

    if inputs.container == Container::Mp4 {
        args.push("-movflags".into());
        args.push("+faststart".into());
    }

    args.push(inputs.output.to_string_lossy().into_owned());

    let status = process::command("ffmpeg")
        .args(args.iter().map(|s| s.as_str()))
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .await
        .map_err(|_| BilibiliError::ContentUnavailable)?;

    if !status.success() {
        return Err(BilibiliError::ContentUnavailable);
    }
    Ok(inputs.output.to_path_buf())
}

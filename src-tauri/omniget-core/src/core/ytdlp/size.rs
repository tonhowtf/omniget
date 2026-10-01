use std::path::Path;
use std::process::Stdio;

use anyhow::anyhow;
use serde::Serialize;

use super::*;

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct SizeItem {
    pub id: String,
    pub title: String,
    pub bytes: Option<u64>,
    pub estimate: bool,
    pub duration: Option<f64>,
    pub height: Option<u32>,
}

pub fn size_format_selector(quality_height: Option<u32>, ffmpeg_available: bool) -> String {
    match quality_height {
        Some(0) => download_format_selector(None, None, "audio", ffmpeg_available),
        h => download_format_selector(None, h, "video", ffmpeg_available),
    }
}

pub fn parse_size_line(line: &str) -> Option<SizeItem> {
    let v: serde_json::Value = serde_json::from_str(line.trim()).ok()?;
    let as_u64 = |k: &str| {
        v.get(k)
            .and_then(|x| x.as_u64().or_else(|| x.as_f64().map(|f| f as u64)))
            .filter(|n| *n > 0)
    };
    let exact = as_u64("filesize");
    let approx = as_u64("filesize_approx");
    Some(SizeItem {
        id: v.get("id")?.as_str()?.to_string(),
        title: v
            .get("title")
            .and_then(|t| t.as_str())
            .unwrap_or_default()
            .to_string(),
        bytes: exact.or(approx),
        estimate: exact.is_none() && approx.is_some(),
        duration: v.get("duration").and_then(|d| d.as_f64()),
        height: v.get("height").and_then(|h| h.as_u64()).map(|h| h as u32),
    })
}

pub async fn measure_sizes(
    ytdlp: &Path,
    url: &str,
    quality_height: Option<u32>,
    ffmpeg_available: bool,
    limit: u32,
) -> anyhow::Result<Vec<SizeItem>> {
    let is_yt = is_youtube_url(url);
    if is_yt {
        yt_rate_limiter().acquire().await;
    }
    let mut args = vec![
        "--skip-download".to_string(),
        "--no-warnings".to_string(),
        "--ignore-errors".to_string(),
        "--encoding".to_string(),
        "utf-8".to_string(),
        "--socket-timeout".to_string(),
        "15".to_string(),
        "--playlist-items".to_string(),
        format!("1:{}", limit.max(1)),
        "-f".to_string(),
        size_format_selector(quality_height, ffmpeg_available),
        "-O".to_string(),
        "%(.{id,title,filesize,filesize_approx,duration,height})j".to_string(),
    ];
    args.extend(default_user_agent_args(ytdlp_can_impersonate(ytdlp)));
    args.extend(js_runtime_args());
    if is_yt {
        args.push("--extractor-args".to_string());
        args.push("youtube:player_client=default".to_string());
    }
    append_metadata_cookie_args(&mut args, url, &[], "size check");
    args.extend(proxy_args());
    args.push(url.to_string());

    note_operator_command(ytdlp, &args, "size");
    let _slot = acquire_ytdlp_slot("size check").await;
    let output = tokio::time::timeout(
        std::time::Duration::from_secs(300),
        ytdlp_command(ytdlp)
            .args(&args)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .output(),
    )
    .await
    .map_err(|_| anyhow!("Timeout checking the size (300s)"))?
    .map_err(|e| anyhow!("Failed to run yt-dlp: {}", e))?;

    let items: Vec<SizeItem> = String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(parse_size_line)
        .collect();
    if items.is_empty() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let last = stderr
            .lines()
            .rev()
            .find(|l| l.contains("ERROR"))
            .unwrap_or("yt-dlp found nothing to download at this address");
        return Err(anyhow!("{}", last.trim()));
    }
    Ok(items)
}

#[cfg(test)]
mod size_tests {
    use super::*;

    #[test]
    fn exact_size_wins_over_the_estimate() {
        let i = parse_size_line(
            r#"{"id": "a", "title": "Glass Half", "filesize": 26959910, "filesize_approx": 27000000, "duration": 193, "height": 720}"#,
        )
        .unwrap();
        assert_eq!(i.bytes, Some(26959910));
        assert!(!i.estimate);
        assert_eq!(i.height, Some(720));
        assert_eq!(i.duration, Some(193.0));
    }

    #[test]
    fn merged_formats_fall_back_to_the_estimate() {
        let i = parse_size_line(
            r#"{"id": "b", "title": "x", "filesize": null, "filesize_approx": 1234.7, "duration": null, "height": null}"#,
        )
        .unwrap();
        assert_eq!(i.bytes, Some(1234));
        assert!(i.estimate);
    }

    #[test]
    fn unknown_size_and_noise_lines() {
        let i = parse_size_line(
            r#"{"id": "c", "title": "y", "filesize": null, "filesize_approx": null}"#,
        )
        .unwrap();
        assert_eq!(i.bytes, None);
        assert!(!i.estimate);
        assert!(parse_size_line("[youtube] Extracting URL").is_none());
        assert!(parse_size_line(r#"{"title": "no id"}"#).is_none());
    }

    #[test]
    fn selector_follows_the_download() {
        assert_eq!(size_format_selector(Some(0), true), "ba/b");
        assert_eq!(
            size_format_selector(Some(720), true),
            "bv*[height<=720]+ba[ext=m4a]/bv*[height<=720]+ba/b[height<=720]/b"
        );
        assert_eq!(size_format_selector(None, true), "bv*+ba[ext=m4a]/bv*+ba/b");
    }
}

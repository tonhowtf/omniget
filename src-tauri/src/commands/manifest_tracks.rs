use std::process::Stdio;
use std::sync::Mutex;

use serde::Serialize;
use serde_json::Value;
use tauri::Emitter;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio_util::sync::CancellationToken;

#[cfg(not(target_os = "android"))]
use crate::core::ytdlp;

static CURRENT: Mutex<Option<CancellationToken>> = Mutex::new(None);

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Track {
    pub id: String,
    pub label: String,
    pub language: Option<String>,
    pub height: Option<u64>,
    pub bitrate_kbps: Option<u64>,
    pub codec: Option<String>,
    pub has_audio: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ManifestTracks {
    pub title: String,
    pub video: Vec<Track>,
    pub audio: Vec<Track>,
    pub subtitles: Vec<Track>,
    pub protected: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct ManifestDownloadResult {
    pub path: String,
    pub bytes: u64,
}

#[derive(Debug, Clone, Serialize)]
struct ManifestProgress {
    percent: f64,
    stage: String,
}

fn s(v: &Value, k: &str) -> Option<String> {
    v.get(k)
        .and_then(|x| x.as_str())
        .filter(|x| !x.is_empty() && *x != "none")
        .map(str::to_string)
}

fn codec_name(c: &str) -> String {
    let short = c.split('.').next().unwrap_or(c);
    match short {
        "avc1" | "avc3" | "h264" => "H.264".to_string(),
        "hvc1" | "hev1" | "h265" => "HEVC".to_string(),
        "av01" => "AV1".to_string(),
        "vp09" | "vp9" => "VP9".to_string(),
        "mp4a" | "aac" => "AAC".to_string(),
        "ec-3" => "E-AC-3".to_string(),
        "ac-3" => "AC-3".to_string(),
        "opus" => "Opus".to_string(),
        other => other.to_string(),
    }
}

fn kbps(f: &Value, keys: &[&str]) -> Option<u64> {
    keys.iter()
        .find_map(|k| f.get(*k).and_then(|x| x.as_f64()))
        .filter(|x| *x > 0.0)
        .map(|x| x.round() as u64)
}

pub fn parse_tracks(json: &Value) -> ManifestTracks {
    let mut video = Vec::new();
    let mut audio = Vec::new();
    let mut protected = 0;
    for f in json
        .get("formats")
        .and_then(|x| x.as_array())
        .into_iter()
        .flatten()
    {
        let Some(id) = s(f, "format_id") else {
            continue;
        };
        if f.get("has_drm").and_then(|x| x.as_bool()).unwrap_or(false) {
            protected += 1;
            continue;
        }
        let vcodec = s(f, "vcodec");
        let acodec = s(f, "acodec");
        let ext = s(f, "ext").unwrap_or_default();
        if ext == "mhtml" {
            continue;
        }
        let language = s(f, "language");
        if let Some(vc) = vcodec {
            let height = f.get("height").and_then(|x| x.as_u64());
            let rate = kbps(f, &["vbr", "tbr"]);
            let mut label = height
                .map(|h| format!("{h}p"))
                .unwrap_or_else(|| "video".to_string());
            if let Some(fps) = f.get("fps").and_then(|x| x.as_f64()).filter(|x| *x > 30.5) {
                label.push_str(&format!("{}", fps.round() as u64));
            }
            video.push(Track {
                id,
                label,
                language: None,
                height,
                bitrate_kbps: rate,
                codec: Some(codec_name(&vc)),
                has_audio: acodec.is_some(),
            });
        } else if let Some(ac) = acodec {
            let name = s(f, "format_note")
                .or_else(|| language.clone())
                .unwrap_or_else(|| "audio".to_string());
            audio.push(Track {
                id,
                label: name,
                language,
                height: None,
                bitrate_kbps: kbps(f, &["abr", "tbr"]),
                codec: Some(codec_name(&ac)),
                has_audio: true,
            });
        }
    }
    video.sort_by_key(|t| std::cmp::Reverse((t.height, t.bitrate_kbps)));
    audio.sort_by(|a, b| {
        (a.language.clone(), b.bitrate_kbps).cmp(&(b.language.clone(), a.bitrate_kbps))
    });
    let mut subtitles: Vec<Track> = json
        .get("subtitles")
        .and_then(|x| x.as_object())
        .map(|m| {
            m.iter()
                .filter(|(lang, _)| lang.as_str() != "live_chat")
                .map(|(lang, list)| {
                    let name = list
                        .as_array()
                        .and_then(|l| l.first())
                        .and_then(|x| s(x, "name"))
                        .unwrap_or_else(|| lang.clone());
                    Track {
                        id: lang.clone(),
                        label: name,
                        language: Some(lang.clone()),
                        height: None,
                        bitrate_kbps: None,
                        codec: None,
                        has_audio: false,
                    }
                })
                .collect()
        })
        .unwrap_or_default();
    subtitles.sort_by(|a, b| a.id.cmp(&b.id));
    ManifestTracks {
        title: s(json, "title").unwrap_or_else(|| "video".to_string()),
        video,
        audio,
        subtitles,
        protected,
    }
}

pub fn download_args(
    video: Option<&str>,
    audio: &[String],
    subs: &[String],
    output_dir: &str,
    container: &str,
    fragments: u32,
) -> Result<Vec<String>, String> {
    let mut ids: Vec<&str> = Vec::new();
    if let Some(v) = video.filter(|v| !v.is_empty()) {
        ids.push(v);
    }
    ids.extend(audio.iter().map(String::as_str));
    if ids.is_empty() {
        return Err("Pick at least one video or audio track".to_string());
    }
    let container = if container == "mp4" { "mp4" } else { "mkv" };
    let mut a = vec![
        "--no-colors".to_string(),
        "--newline".to_string(),
        "--no-playlist".to_string(),
        "-f".to_string(),
        ids.join("+"),
        "-N".to_string(),
        fragments.clamp(1, 32).to_string(),
        "-o".to_string(),
        format!("{output_dir}/%(title).150B [%(id)s].%(ext)s"),
        "--print".to_string(),
        "after_move:filepath".to_string(),
    ];
    if audio.len() > 1 {
        a.push("--audio-multistreams".to_string());
    }
    if ids.len() > 1 {
        a.push("--merge-output-format".to_string());
        a.push(container.to_string());
    }
    if !subs.is_empty() {
        a.push("--write-subs".to_string());
        a.push("--sub-langs".to_string());
        a.push(subs.join(","));
        a.push("--embed-subs".to_string());
    }
    Ok(a)
}

fn check_url(url: &str) -> Result<(), String> {
    if !(url.starts_with("http://") || url.starts_with("https://")) {
        return Err("Paste a web address that starts with http:// or https://".to_string());
    }
    omniget_core::core::platform_optout::ensure_allowed(url)
}

#[cfg(not(target_os = "android"))]
#[tauri::command]
pub async fn manifest_tracks_list(url: String) -> Result<ManifestTracks, String> {
    let url = url.trim().to_string();
    check_url(&url)?;
    let bin = ytdlp::ensure_ytdlp()
        .await
        .map_err(|e| format!("yt-dlp unavailable: {e}"))?;
    let json = ytdlp::get_video_info(&bin, &url, &[])
        .await
        .map_err(|e| format!("Could not read this manifest: {e}"))?;
    let tracks = parse_tracks(&json);
    if tracks.video.is_empty() && tracks.audio.is_empty() {
        if tracks.protected > 0 {
            return Err(
                "Every track of this stream is protected (DRM); OmniGet does not download it"
                    .to_string(),
            );
        }
        return Err("No track was found in this link".to_string());
    }
    Ok(tracks)
}

#[cfg(not(target_os = "android"))]
#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn manifest_tracks_download(
    app: tauri::AppHandle,
    url: String,
    video: Option<String>,
    audio: Vec<String>,
    subtitles: Vec<String>,
    output_dir: String,
    container: String,
) -> Result<ManifestDownloadResult, String> {
    let url = url.trim().to_string();
    check_url(&url)?;
    if output_dir.trim().is_empty() {
        return Err("Choose a download folder first".to_string());
    }
    std::fs::create_dir_all(&output_dir).map_err(|e| e.to_string())?;
    let mut args = download_args(
        video.as_deref(),
        &audio,
        &subtitles,
        &output_dir,
        &container,
        8,
    )?;
    let bin = ytdlp::ensure_ytdlp()
        .await
        .map_err(|e| format!("yt-dlp unavailable: {e}"))?;
    if let Ok(ffmpeg) = omniget_core::core::dependencies::ensure_ffmpeg().await {
        args.push("--ffmpeg-location".to_string());
        args.push(ffmpeg.to_string_lossy().into_owned());
    }
    args.extend(ytdlp::insecure_tls_args());
    args.push("--".to_string());
    args.push(url);

    let token = CancellationToken::new();
    {
        let mut cur = CURRENT.lock().map_err(|e| e.to_string())?;
        if cur.is_some() {
            return Err("A download from this tool is already running".to_string());
        }
        *cur = Some(token.clone());
    }
    let result = run(&app, &bin, args, token).await;
    if let Ok(mut cur) = CURRENT.lock() {
        *cur = None;
    }
    result
}

pub fn parse_percent(line: &str) -> Option<f64> {
    let rest = line.trim().strip_prefix("[download]")?.trim_start();
    let pct = rest.split('%').next()?;
    pct.trim().parse::<f64>().ok()
}

#[cfg(not(target_os = "android"))]
async fn run(
    app: &tauri::AppHandle,
    bin: &std::path::Path,
    args: Vec<String>,
    token: CancellationToken,
) -> Result<ManifestDownloadResult, String> {
    let mut child = omniget_core::core::process::command(bin)
        .args(&args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| format!("Failed to start yt-dlp: {e}"))?;
    let stdout = child.stdout.take().ok_or("No output from yt-dlp")?;
    let stderr = child.stderr.take().ok_or("No output from yt-dlp")?;
    let app2 = app.clone();
    let reader = tokio::spawn(async move {
        let mut path = None;
        let mut lines = BufReader::new(stdout).lines();
        let mut stage = 0u32;
        let mut last = 0f64;
        while let Ok(Some(line)) = lines.next_line().await {
            if let Some(p) = parse_percent(&line) {
                if p + 0.5 < last {
                    stage += 1;
                }
                last = p;
                let _ = app2.emit(
                    "manifest-tracks-progress",
                    ManifestProgress {
                        percent: p,
                        stage: format!("{}", stage + 1),
                    },
                );
            } else if line.starts_with("[Merger]") || line.starts_with("[EmbedSubtitle]") {
                let _ = app2.emit(
                    "manifest-tracks-progress",
                    ManifestProgress {
                        percent: 100.0,
                        stage: "merge".to_string(),
                    },
                );
            } else if !line.starts_with('[') && std::path::Path::new(line.trim()).is_absolute() {
                path = Some(line.trim().to_string());
            }
        }
        path
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
    let path = reader.await.unwrap_or_default();
    let err = err_reader.await.unwrap_or_default();
    match path {
        Some(p) if status.success() => Ok(ManifestDownloadResult {
            bytes: std::fs::metadata(&p).map(|m| m.len()).unwrap_or(0),
            path: p,
        }),
        _ => Err(err
            .lines()
            .rev()
            .find(|l| l.starts_with("ERROR:"))
            .map(|l| l.trim_start_matches("ERROR:").trim().to_string())
            .unwrap_or_else(|| "yt-dlp could not download these tracks".to_string())),
    }
}

#[tauri::command]
pub fn manifest_tracks_cancel() -> bool {
    match CURRENT.lock() {
        Ok(cur) => cur.as_ref().map(|t| t.cancel()).is_some(),
        Err(_) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn sample() -> Value {
        json!({
            "title": "Tears of Steel",
            "formats": [
                {"format_id": "audio_eng=128000", "ext": "m4a", "vcodec": "none", "acodec": "mp4a.40.2", "language": "en", "abr": 128.0},
                {"format_id": "audio_deu=64000", "ext": "m4a", "vcodec": "none", "acodec": "mp4a.40.2", "language": "de", "abr": 64.0},
                {"format_id": "video=400000", "ext": "mp4", "vcodec": "avc1.42C00D", "acodec": "none", "height": 224, "tbr": 400.0},
                {"format_id": "video=2200000", "ext": "mp4", "vcodec": "avc1.640028", "acodec": "none", "height": 1080, "tbr": 2200.0},
                {"format_id": "drm", "ext": "mp4", "vcodec": "avc1", "acodec": "none", "height": 2160, "has_drm": true},
                {"format_id": "sb0", "ext": "mhtml", "vcodec": "none", "acodec": "none"}
            ],
            "subtitles": {"en": [{"ext": "vtt", "name": "English"}], "nl": [{"ext": "vtt"}]}
        })
    }

    #[test]
    fn lists_tracks_best_first_and_skips_drm() {
        let t = parse_tracks(&sample());
        assert_eq!(t.video.len(), 2);
        assert_eq!(t.video[0].id, "video=2200000");
        assert_eq!(t.video[0].codec.as_deref(), Some("H.264"));
        assert_eq!(
            t.audio
                .iter()
                .map(|a| a.language.clone().unwrap())
                .collect::<Vec<_>>(),
            ["de", "en"]
        );
        assert_eq!(
            t.subtitles
                .iter()
                .map(|s| s.label.as_str())
                .collect::<Vec<_>>(),
            ["English", "nl"]
        );
        assert_eq!(t.protected, 1);
    }

    #[test]
    fn builds_a_multi_audio_download() {
        let a = download_args(
            Some("video=2200000"),
            &["audio_eng=128000".into(), "audio_deu=64000".into()],
            &["en".into()],
            "/tmp/o",
            "mkv",
            8,
        )
        .unwrap();
        assert!(a
            .windows(2)
            .any(|w| w == ["-f", "video=2200000+audio_eng=128000+audio_deu=64000"]));
        assert!(a.iter().any(|x| x == "--audio-multistreams"));
        assert!(a.windows(2).any(|w| w == ["--merge-output-format", "mkv"]));
        assert!(a.windows(2).any(|w| w == ["--sub-langs", "en"]));
        assert!(a.windows(2).any(|w| w == ["-N", "8"]));
    }

    #[test]
    fn a_single_track_needs_no_merge_and_nothing_is_an_error() {
        let a = download_args(None, &["a".into()], &[], "/tmp/o", "mp4", 4).unwrap();
        assert!(!a
            .iter()
            .any(|x| x == "--merge-output-format" || x == "--audio-multistreams"));
        assert!(download_args(None, &[], &[], "/tmp/o", "mkv", 4).is_err());
    }

    #[test]
    fn reads_yt_dlp_progress() {
        assert_eq!(
            parse_percent("[download]  42.5% of ~ 10.00MiB at 2.00MiB/s"),
            Some(42.5)
        );
        assert_eq!(parse_percent("[Merger] Merging formats"), None);
    }
}

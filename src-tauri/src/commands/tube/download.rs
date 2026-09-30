//! OmniTube downloads go through the app's download queue (same progress,
//! history and retries as everything else). What OmniTube adds is the
//! exact yt-dlp format from the quality menu and the SponsorBlock choice.

use serde::Deserialize;
use tauri::{AppHandle, Manager};

/// SponsorBlock categories yt-dlp accepts.
const YTDLP_SB_CATEGORIES: &[&str] = &[
    "sponsor",
    "intro",
    "outro",
    "selfpromo",
    "preview",
    "filler",
    "interaction",
    "music_offtopic",
    "hook",
    "poi_highlight",
    "chapter",
];
/// Categories that only make sense as chapter marks.
const MARK_ONLY: &[&str] = &["poi_highlight", "chapter"];

#[derive(Debug, Clone, Default, Deserialize)]
pub struct TubeDownloadReq {
    pub video_id: String,
    /// music.youtube.com URL (audio track of a song).
    #[serde(default)]
    pub music: bool,
    /// Exact formats from the `StreamSet` (quality menu).
    #[serde(default)]
    pub video_itag: Option<u32>,
    #[serde(default)]
    pub audio_itag: Option<u32>,
    #[serde(default)]
    pub audio_only: bool,
    /// Height cap when no itag is given; `None` = best.
    #[serde(default)]
    pub height: Option<u32>,
    /// "off" | "mark" | "remove"; `None` = `settings.tube.sb_download`.
    #[serde(default)]
    pub sponsorblock: Option<String>,
    #[serde(default)]
    pub folder: Option<String>,
    /// For the SponsorBlock channel whitelist.
    #[serde(default)]
    pub channel_id: Option<String>,
}

/// What the queue receives.
#[derive(Debug, Clone, PartialEq)]
pub struct Plan {
    pub url: String,
    pub output_dir: String,
    pub download_mode: Option<String>,
    pub quality: Option<String>,
    pub format_id: Option<String>,
    pub extra_args: Vec<String>,
}

pub fn watch_url(video_id: &str, music: bool) -> String {
    if music {
        format!("https://music.youtube.com/watch?v={video_id}")
    } else {
        format!("https://www.youtube.com/watch?v={video_id}")
    }
}

/// Full `-f` selector for explicit itags. It always carries a `/` fallback,
/// which makes the queue pass it to yt-dlp untouched; the fallback keeps the
/// height when the exact itag is gone by the time the download starts.
pub fn format_selector(
    video_itag: Option<u32>,
    audio_itag: Option<u32>,
    audio_only: bool,
    height: Option<u32>,
) -> Option<String> {
    let cap = height.map(|h| format!("[height<={h}]")).unwrap_or_default();
    if audio_only {
        return audio_itag.map(|a| format!("{a}/ba/b"));
    }
    match (video_itag, audio_itag) {
        (Some(v), Some(a)) => Some(format!("{v}+{a}/bv*{cap}+ba/b{cap}/b")),
        (Some(v), None) => Some(format!("{v}+ba/bv*{cap}+ba/b{cap}/b")),
        _ => None,
    }
}

/// yt-dlp flags for one download. `--no-sponsorblock` also keeps the app-wide
/// SponsorBlock setting from applying to this download.
pub fn sponsorblock_args(mode: &str, categories: &[String]) -> Vec<String> {
    let known = |c: &&String| YTDLP_SB_CATEGORIES.contains(&c.as_str());
    let cats: Vec<&String> = match mode {
        "remove" => categories
            .iter()
            .filter(known)
            .filter(|c| !MARK_ONLY.contains(&c.as_str()))
            .collect(),
        "mark" => categories.iter().filter(known).collect(),
        _ => Vec::new(),
    };
    if cats.is_empty() {
        return vec!["--no-sponsorblock".into()];
    }
    let flag = if mode == "mark" {
        "--sponsorblock-mark"
    } else {
        "--sponsorblock-remove"
    };
    let list = cats
        .iter()
        .map(|c| c.as_str())
        .collect::<Vec<_>>()
        .join(",");
    vec![flag.into(), list]
}

/// Categories for a download: "remove" cuts what the player skips; "mark"
/// turns every visible category into a chapter.
pub fn download_categories(
    mode: &str,
    map: &std::collections::BTreeMap<String, String>,
) -> Vec<String> {
    map.iter()
        .filter(|(_, m)| match mode {
            "remove" => m.as_str() == "skip",
            "mark" => m.as_str() != "off",
            _ => false,
        })
        .map(|(c, _)| c.clone())
        .collect()
}

pub fn default_folder(app: &AppHandle) -> String {
    let s = crate::storage::config::load_settings(app);
    s.tube
        .download_folder
        .filter(|f| !f.trim().is_empty())
        .unwrap_or_else(|| s.download.default_output_dir.to_string_lossy().to_string())
}

pub fn plan_for(app: &AppHandle, req: &TubeDownloadReq) -> Plan {
    let tube = crate::storage::config::load_settings(app).tube;
    let whitelisted = req
        .channel_id
        .as_deref()
        .is_some_and(|id| tube.sb_whitelist.iter().any(|w| w == id));
    let mode = if whitelisted {
        "off".to_string()
    } else {
        req.sponsorblock
            .clone()
            .unwrap_or_else(|| tube.sb_download.clone())
    };
    let cats = download_categories(&mode, &tube.sb_categories);
    Plan {
        url: watch_url(&req.video_id, req.music),
        output_dir: req
            .folder
            .clone()
            .filter(|f| !f.trim().is_empty())
            .unwrap_or_else(|| default_folder(app)),
        download_mode: req.audio_only.then(|| "audio".to_string()),
        quality: if req.audio_only {
            None
        } else {
            Some(
                req.height
                    .map(|h| h.to_string())
                    .unwrap_or_else(|| "best".into()),
            )
        },
        format_id: format_selector(req.video_itag, req.audio_itag, req.audio_only, req.height),
        extra_args: sponsorblock_args(&mode, &cats),
    }
}

/// Hands a plan to the download queue. Returns the queue id.
pub async fn enqueue(app: &AppHandle, plan: Plan) -> Result<u64, String> {
    let started = crate::commands::downloads::download_from_url_with_args(
        app.clone(),
        app.state::<crate::AppState>(),
        plan.url,
        plan.output_dir,
        plan.download_mode,
        plan.quality,
        plan.format_id,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        Some(plan.extra_args),
    )
    .await?;
    Ok(started.id)
}

/// Returns the queue id as a string.
#[tauri::command]
pub async fn tube_download(app: AppHandle, req: TubeDownloadReq) -> Result<String, String> {
    if req.video_id.len() != 11 {
        return Err("Invalid video id".into());
    }
    let plan = plan_for(&app, &req);
    enqueue(&app, plan).await.map(|id| id.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selectors() {
        assert_eq!(
            format_selector(Some(137), Some(140), false, Some(1080)).as_deref(),
            Some("137+140/bv*[height<=1080]+ba/b[height<=1080]/b")
        );
        assert_eq!(
            format_selector(Some(248), None, false, None).as_deref(),
            Some("248+ba/bv*+ba/b/b")
        );
        assert_eq!(
            format_selector(None, Some(251), true, None).as_deref(),
            Some("251/ba/b")
        );
        assert_eq!(format_selector(None, None, false, Some(720)), None);
        // Every explicit selector must look "full" to the queue (has a '/').
        assert!(format_selector(Some(1), Some(2), false, None)
            .unwrap()
            .contains('/'));
    }

    #[test]
    fn sponsorblock_flags() {
        let cats = vec![
            "sponsor".to_string(),
            "poi_highlight".to_string(),
            "exclusive_access".to_string(),
        ];
        assert_eq!(
            sponsorblock_args("remove", &cats),
            vec!["--sponsorblock-remove", "sponsor"]
        );
        assert_eq!(
            sponsorblock_args("mark", &cats),
            vec!["--sponsorblock-mark", "sponsor,poi_highlight"]
        );
        assert_eq!(sponsorblock_args("off", &cats), vec!["--no-sponsorblock"]);
        assert_eq!(sponsorblock_args("remove", &[]), vec!["--no-sponsorblock"]);
    }

    #[test]
    fn categories_by_mode() {
        let map = [("sponsor", "skip"), ("intro", "show"), ("filler", "off")]
            .into_iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        assert_eq!(download_categories("remove", &map), vec!["sponsor"]);
        assert_eq!(download_categories("mark", &map), vec!["intro", "sponsor"]);
        assert!(download_categories("off", &map).is_empty());
    }
}

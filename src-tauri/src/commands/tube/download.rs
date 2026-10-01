use serde::Deserialize;
use tauri::{AppHandle, Manager};

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
const MARK_ONLY: &[&str] = &["poi_highlight", "chapter"];

#[derive(Debug, Clone, Default, Deserialize)]
pub struct TubeDownloadReq {
    pub video_id: String,
    #[serde(default)]
    pub music: bool,
    #[serde(default)]
    pub video_itag: Option<u32>,
    #[serde(default)]
    pub audio_itag: Option<u32>,
    #[serde(default)]
    pub audio_only: bool,
    #[serde(default)]
    pub height: Option<u32>,
    #[serde(default)]
    pub sponsorblock: Option<String>,
    #[serde(default)]
    pub folder: Option<String>,
    #[serde(default)]
    pub channel_id: Option<String>,
}

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
        None,
        None,
        Some(plan.extra_args),
    )
    .await?;
    Ok(started.id)
}

#[tauri::command]
pub async fn tube_download(app: AppHandle, req: TubeDownloadReq) -> Result<String, String> {
    if req.video_id.len() != 11 {
        return Err("Invalid video id".into());
    }
    let plan = plan_for(&app, &req);
    enqueue(&app, plan).await.map(|id| id.to_string())
}

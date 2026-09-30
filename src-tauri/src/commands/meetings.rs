//! Meetings screen: record, import (file or Teams/SharePoint recording), send a
//! bot, and everything after (transcript, summary, markdown, post to Teams).
//! The work itself lives in `omniget_core::core::meetings`; this file wires it
//! to the window: events, the SharePoint sign-in window and the recorder.
//!
//! Events: `meetings://changed` `{id}` when a meeting's state moves, and
//! `meetings://progress` (a `ToolProgress` whose `id` is the meeting id).

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use omniget_core::core::meetings::{
    self, pipeline, publish, settings, teams, vexa, whisper, Meeting, MeetingSource, MeetingStatus,
    Segment,
};
use omniget_core::core::tools::{ProgressFn, ToolProgress};
use serde::Serialize;
use tauri::{AppHandle, Emitter};

use crate::meetings_capture;

const EV_CHANGED: &str = "meetings://changed";
const EV_PROGRESS: &str = "meetings://progress";

/// SharePoint session cookies per host, kept for this run of the app only.
static SHAREPOINT_SESSIONS: Mutex<Option<HashMap<String, String>>> = Mutex::new(None);

fn session_cookie(host: &str) -> Option<String> {
    SHAREPOINT_SESSIONS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .as_ref()
        .and_then(|m| m.get(host).cloned())
}

fn remember_session(host: &str, cookie: String) {
    SHAREPOINT_SESSIONS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get_or_insert_with(HashMap::new)
        .insert(host.to_string(), cookie);
}

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

fn changed(app: &AppHandle, id: &str) {
    let _ = app.emit(EV_CHANGED, serde_json::json!({ "id": id }));
}

fn progress_fn(app: &AppHandle) -> ProgressFn {
    let app = app.clone();
    Arc::new(move |p: ToolProgress| {
        if p.stage == "status" {
            changed(&app, &p.id);
        }
        let _ = app.emit(EV_PROGRESS, &p);
    })
}

/// Runs the pipeline in the background. A SharePoint recording that needs a
/// session opens the sign-in window once and runs again with its cookies.
fn spawn_process(app: &AppHandle, id: String) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let host = sharepoint_host(&id);
        let mut ctx = pipeline::Ctx::new(progress_fn(&app));
        ctx.cookie = host.as_deref().and_then(session_cookie);
        let first = pipeline::process(&id, ctx.clone()).await;
        let needs_login =
            matches!(&first, Err(e) if e.chain().any(|c| c.is::<teams::AuthRequired>()));
        if needs_login {
            if let Some(host) = host {
                match sharepoint_login(&app, &host).await {
                    Ok(cookie) => {
                        remember_session(&host, cookie.clone());
                        ctx.cookie = Some(cookie);
                        let _ = pipeline::process(&id, ctx).await;
                    }
                    Err(e) => {
                        let _ = meetings::update(&id, |m| {
                            m.status = MeetingStatus::Failed;
                            m.error = Some(format!("SharePoint sign-in did not finish: {e}"));
                        });
                    }
                }
            }
        }
        changed(&app, &id);
    });
}

fn sharepoint_host(id: &str) -> Option<String> {
    let m = meetings::load(id).ok()?;
    let MeetingSource::Teams { url } = &m.source else {
        return None;
    };
    match teams::classify(url)? {
        teams::TeamsInput::SharePoint { host, .. } => Some(host),
        _ => None,
    }
}

/// Opens the tenant's sign-in in an app window and returns the session
/// cookie header once `FedAuth` exists.
async fn sharepoint_login(app: &AppHandle, host: &str) -> Result<String, String> {
    use crate::commands::auth_webview::{open_auth_webview, AuthWebviewRequest};
    let request = AuthWebviewRequest {
        url: format!("https://{host}/_layouts/15/Authenticate.aspx?Source=%2F"),
        title: "Sign in to SharePoint".to_string(),
        cookie_domains: vec![host.to_string()],
        success_url_contains: None,
        wait_for_cookie: Some("FedAuth".to_string()),
        initialization_script: None,
        width: Some(520.0),
        height: Some(720.0),
    };
    let result = open_auth_webview(app.clone(), request).await?;
    let header = result
        .cookies
        .iter()
        .filter(|c| {
            let d = c.domain.trim_start_matches('.');
            host == d || host.ends_with(&format!(".{d}"))
        })
        .map(|c| format!("{}={}", c.name, c.value))
        .collect::<Vec<_>>()
        .join("; ");
    if header.is_empty() {
        return Err("no SharePoint session cookie".into());
    }
    Ok(header)
}

#[tauri::command]
pub fn meetings_list() -> Vec<Meeting> {
    meetings::list()
}

#[derive(Serialize)]
pub struct MeetingDetail {
    pub meeting: Meeting,
    pub segments: Vec<Segment>,
    pub folder: String,
    pub markdown_path: Option<String>,
    pub audio_path: Option<String>,
    pub running: bool,
}

#[tauri::command]
pub fn meeting_get(id: String) -> Result<MeetingDetail, String> {
    let meeting = meetings::load(&id).map_err(err)?;
    let dir = meetings::dir_of(&id).map_err(err)?;
    let md = dir.join(meetings::MARKDOWN_FILE);
    Ok(MeetingDetail {
        segments: meetings::load_transcript(&id),
        folder: dir.to_string_lossy().to_string(),
        markdown_path: md.exists().then(|| md.to_string_lossy().to_string()),
        audio_path: meeting
            .audio
            .as_ref()
            .map(|a| dir.join(a))
            .filter(|p| p.exists())
            .map(|p| p.to_string_lossy().to_string()),
        running: pipeline::is_running(&id),
        meeting,
    })
}

#[tauri::command]
pub fn meeting_markdown(id: String) -> Result<String, String> {
    meetings::read_markdown(&id).map_err(err)
}

#[tauri::command]
pub fn meeting_update(
    app: AppHandle,
    id: String,
    title: Option<String>,
    notes: Option<String>,
) -> Result<Meeting, String> {
    let m = meetings::update(&id, |m| {
        if let Some(t) = title
            .map(|t| t.trim().to_string())
            .filter(|t| !t.is_empty())
        {
            m.title = t.chars().take(200).collect();
        }
        if let Some(n) = notes {
            m.notes = n;
        }
    })
    .map_err(err)?;
    if m.status == MeetingStatus::Ready {
        meetings::write_markdown(&m).map_err(err)?;
    }
    changed(&app, &id);
    Ok(m)
}

#[tauri::command]
pub fn meeting_set_action_done(
    app: AppHandle,
    id: String,
    index: usize,
    done: bool,
) -> Result<Meeting, String> {
    let m = meetings::set_action_done(&id, index, done).map_err(err)?;
    changed(&app, &id);
    Ok(m)
}

#[tauri::command]
pub fn meeting_delete(app: AppHandle, id: String) -> Result<(), String> {
    if pipeline::is_running(&id) {
        return Err("Wait for this meeting to finish processing".into());
    }
    if meetings_capture::status().is_some_and(|s| s.meeting_id == id) {
        return Err("Stop the recording first".into());
    }
    meetings::delete(&id).map_err(err)?;
    changed(&app, &id);
    Ok(())
}

#[tauri::command]
pub fn meeting_import_file(
    app: AppHandle,
    path: String,
    title: Option<String>,
) -> Result<Meeting, String> {
    let p = std::path::Path::new(&path);
    if !p.is_file() {
        return Err(format!("File not found: {path}"));
    }
    let title = title.or_else(|| p.file_stem().map(|s| s.to_string_lossy().to_string()));
    let m = meetings::create(
        title.as_deref(),
        MeetingSource::File { path },
        MeetingStatus::Queued,
    )
    .map_err(err)?;
    spawn_process(&app, m.id.clone());
    changed(&app, &m.id);
    Ok(m)
}

/// A Teams/SharePoint recording link, a `videomanifest` URL or a transcript
/// URL. Any other URL is refused: generic media goes through Downloads.
#[tauri::command]
pub fn meeting_import_url(
    app: AppHandle,
    url: String,
    title: Option<String>,
) -> Result<Meeting, String> {
    let url = url.trim().to_string();
    if teams::classify(&url).is_none() {
        return Err("Paste a Teams or SharePoint recording link (stream.aspx, a sharing link, the .mp4 on SharePoint) or a videomanifest URL".into());
    }
    let m = meetings::create(
        title.as_deref(),
        MeetingSource::Teams { url },
        MeetingStatus::Queued,
    )
    .map_err(err)?;
    spawn_process(&app, m.id.clone());
    changed(&app, &m.id);
    Ok(m)
}

#[tauri::command]
pub fn meeting_retranscribe(app: AppHandle, id: String) -> Result<(), String> {
    if pipeline::is_running(&id) {
        return Err("This meeting is already being processed".into());
    }
    let app2 = app.clone();
    tauri::async_runtime::spawn(async move {
        let _ = pipeline::retranscribe(&id, pipeline::Ctx::new(progress_fn(&app2))).await;
        changed(&app2, &id);
    });
    Ok(())
}

#[tauri::command]
pub async fn meeting_resummarize(app: AppHandle, id: String) -> Result<Meeting, String> {
    let m = pipeline::resummarize(&id, pipeline::Ctx::new(progress_fn(&app)))
        .await
        .map_err(|e| format!("{e:#}"))?;
    changed(&app, &id);
    Ok(m)
}

#[tauri::command]
pub async fn meeting_post_teams(id: String) -> Result<(), String> {
    let m = meetings::load(&id).map_err(err)?;
    publish::post_to_teams(&m)
        .await
        .map_err(|e| format!("{e:#}"))
}

// ---------------------------------------------------------------- capture

#[tauri::command]
pub async fn meeting_capture_support() -> meetings_capture::CaptureSupport {
    tauri::async_runtime::spawn_blocking(meetings_capture::support)
        .await
        .unwrap_or(meetings_capture::CaptureSupport {
            inputs: vec![],
            system_audio: false,
            system_audio_note: String::new(),
        })
}

#[derive(Serialize)]
pub struct RecordStart {
    pub meeting: Meeting,
    pub warnings: Vec<String>,
}

#[tauri::command]
pub async fn meeting_record_start(
    app: AppHandle,
    title: Option<String>,
    mic: bool,
    system: bool,
    mic_device: Option<String>,
) -> Result<RecordStart, String> {
    if meetings_capture::is_recording() {
        return Err("A meeting is already being recorded".into());
    }
    let m = meetings::create(
        title.as_deref(),
        MeetingSource::Capture { mic, system },
        MeetingStatus::Recording,
    )
    .map_err(err)?;
    let dir = meetings::dir_of(&m.id).map_err(err)?;
    let id = m.id.clone();
    let started = tauri::async_runtime::spawn_blocking(move || {
        meetings_capture::start(&id, &dir, mic, system, mic_device)
    })
    .await
    .map_err(err)?;
    match started {
        Ok(warnings) => {
            changed(&app, &m.id);
            Ok(RecordStart {
                meeting: m,
                warnings,
            })
        }
        Err(e) => {
            let _ = meetings::delete(&m.id);
            Err(e)
        }
    }
}

#[tauri::command]
pub fn meeting_record_status() -> Option<meetings_capture::CaptureStatus> {
    meetings_capture::status()
}

/// Stops the recorder, mixes the tracks into `audio.m4a` for playback and
/// starts the transcript.
#[tauri::command]
pub async fn meeting_record_stop(app: AppHandle) -> Result<Meeting, String> {
    let stopped = tauri::async_runtime::spawn_blocking(meetings_capture::stop)
        .await
        .map_err(err)??;
    let id = stopped.meeting_id.clone();
    let dir = meetings::dir_of(&id).map_err(err)?;
    let tracks: Vec<_> = stopped
        .tracks
        .into_iter()
        .filter(|t| {
            std::fs::metadata(dir.join(&t.file))
                .map(|m| m.len() > 44)
                .unwrap_or(false)
        })
        .collect();
    if tracks.is_empty() {
        let m = meetings::update(&id, |m| {
            m.status = MeetingStatus::Failed;
            m.error = Some(format!(
                "Nothing was recorded. {}",
                stopped.warnings.join("; ")
            ));
        })
        .map_err(err)?;
        changed(&app, &id);
        return Ok(m);
    }
    let mixed = mix_tracks(&dir, &tracks).await;
    let m = meetings::update(&id, |m| {
        m.duration_ms = stopped.duration_ms;
        m.tracks = tracks.clone();
        m.audio = mixed
            .ok()
            .or_else(|| tracks.first().map(|t| t.file.clone()));
        m.status = MeetingStatus::Queued;
    })
    .map_err(err)?;
    spawn_process(&app, id.clone());
    changed(&app, &id);
    Ok(m)
}

async fn mix_tracks(dir: &std::path::Path, tracks: &[meetings::Track]) -> Result<String, String> {
    let ffmpeg = omniget_core::core::dependencies::ensure_ffmpeg()
        .await
        .map_err(err)?;
    let mut cmd = omniget_core::core::process::command(&ffmpeg);
    cmd.args(["-y", "-hide_banner", "-loglevel", "error"]);
    for t in tracks {
        cmd.arg("-i").arg(dir.join(&t.file));
    }
    if tracks.len() > 1 {
        cmd.args([
            "-filter_complex",
            &format!("amix=inputs={}:duration=longest:normalize=0", tracks.len()),
        ]);
    }
    cmd.args(["-ac", "1", "-c:a", "aac", "-b:a", "64k"])
        .arg(dir.join("audio.m4a"));
    let out = cmd.output().await.map_err(err)?;
    if out.status.success() {
        Ok("audio.m4a".into())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).to_string())
    }
}

// ---------------------------------------------------------------- bot

#[tauri::command]
pub async fn meeting_bot_send(
    app: AppHandle,
    url: String,
    title: Option<String>,
) -> Result<Meeting, String> {
    let url = url.trim().to_string();
    let link = vexa::parse_link(&url).ok_or("Paste a Teams, Google Meet or Zoom meeting link")?;
    let s = settings::load();
    vexa::send_bot(&link, &url, Some(s.language.as_str()))
        .await
        .map_err(|e| format!("{e:#}"))?;
    let m = meetings::create(
        title.as_deref(),
        MeetingSource::Bot {
            platform: link.platform.clone(),
            meeting_url: url,
            native_id: link.native_id.clone(),
        },
        MeetingStatus::BotLive,
    )
    .map_err(err)?;
    changed(&app, &m.id);
    Ok(m)
}

fn bot_link(m: &Meeting) -> Result<vexa::MeetingLink, String> {
    match &m.source {
        MeetingSource::Bot {
            platform,
            meeting_url,
            native_id,
        } => Ok(vexa::parse_link(meeting_url).unwrap_or(vexa::MeetingLink {
            platform: platform.clone(),
            native_id: native_id.clone(),
            passcode: None,
        })),
        _ => Err("Not a bot meeting".into()),
    }
}

#[derive(Serialize)]
pub struct BotRefresh {
    pub meeting: Meeting,
    pub bot_status: String,
    pub live_segments: usize,
}

/// Pulls the bot's transcript. While the call runs it only reports progress;
/// once the bot is done the transcript is stored and summarized.
#[tauri::command]
pub async fn meeting_bot_refresh(app: AppHandle, id: String) -> Result<BotRefresh, String> {
    let m = meetings::load(&id).map_err(err)?;
    let link = bot_link(&m)?;
    let t = vexa::transcript(&link)
        .await
        .map_err(|e| format!("{e:#}"))?;
    if t.finished && m.status == MeetingStatus::BotLive {
        let m = pipeline::ingest_segments(
            &id,
            t.segments.clone(),
            "vexa",
            pipeline::Ctx::new(progress_fn(&app)),
        )
        .await
        .map_err(|e| format!("{e:#}"))?;
        changed(&app, &id);
        return Ok(BotRefresh {
            meeting: m,
            bot_status: t.status,
            live_segments: t.segments.len(),
        });
    }
    Ok(BotRefresh {
        live_segments: t.segments.len(),
        bot_status: t.status,
        meeting: m,
    })
}

/// Makes the bot leave and keeps what it transcribed.
#[tauri::command]
pub async fn meeting_bot_stop(app: AppHandle, id: String) -> Result<Meeting, String> {
    let m = meetings::load(&id).map_err(err)?;
    let link = bot_link(&m)?;
    if let Err(e) = vexa::stop_bot(&link).await {
        tracing::warn!("[meetings] stopping the bot: {e:#}");
    }
    let t = vexa::transcript(&link)
        .await
        .map_err(|e| format!("{e:#}"))?;
    let m = pipeline::ingest_segments(
        &id,
        t.segments,
        "vexa",
        pipeline::Ctx::new(progress_fn(&app)),
    )
    .await
    .map_err(|e| format!("{e:#}"))?;
    changed(&app, &id);
    Ok(m)
}

// ---------------------------------------------------------------- settings

#[tauri::command]
pub fn meetings_settings_get() -> settings::MeetingSettingsView {
    settings::view()
}

#[tauri::command]
pub fn meetings_settings_set(
    value: settings::MeetingSettings,
    vexa_key: Option<String>,
    teams_webhook: Option<String>,
) -> Result<settings::MeetingSettingsView, String> {
    settings::save(&value).map_err(err)?;
    settings::set_secrets(vexa_key.as_deref(), teams_webhook.as_deref())?;
    Ok(settings::view())
}

#[tauri::command]
pub async fn meetings_whisper_status() -> whisper::WhisperStatus {
    whisper::status().await
}

#[tauri::command]
pub async fn meetings_whisper_install(app: AppHandle) -> Result<String, String> {
    whisper::install(progress_fn(&app))
        .await
        .map(|p| p.to_string_lossy().to_string())
        .map_err(|e| format!("{e:#}"))
}

#[tauri::command]
pub async fn meetings_whisper_model_download(app: AppHandle, model: String) -> Result<(), String> {
    whisper::download_model(&model, progress_fn(&app))
        .await
        .map(|_| ())
        .map_err(|e| format!("{e:#}"))
}

#[tauri::command]
pub fn meetings_whisper_model_remove(model: String) -> Result<(), String> {
    whisper::remove_model(&model).map_err(err)
}

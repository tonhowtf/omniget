//! Takes a meeting from its source to `meeting.md`:
//! acquire (download / convert) → transcribe → summarize → markdown.
//! Every status change is reported as a `status` progress event so the UI,
//! the CLI and the MCP tools can follow the same run.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use anyhow::{anyhow, Context};

use super::teams::{self, TeamsInput};
use super::transcript::{self, merge_tracks, Segment};
use super::{settings, whisper, Meeting, MeetingSource, MeetingStatus};
use crate::core::tools::{report, ProgressFn};

static RUNNING: Mutex<Option<HashSet<String>>> = Mutex::new(None);

struct RunGuard(String);

impl RunGuard {
    fn take(id: &str) -> anyhow::Result<Self> {
        let mut g = RUNNING.lock().unwrap_or_else(|e| e.into_inner());
        let set = g.get_or_insert_with(HashSet::new);
        if !set.insert(id.to_string()) {
            return Err(anyhow!("This meeting is already being processed"));
        }
        Ok(Self(id.to_string()))
    }
}

impl Drop for RunGuard {
    fn drop(&mut self) {
        if let Some(set) = RUNNING.lock().unwrap_or_else(|e| e.into_inner()).as_mut() {
            set.remove(&self.0);
        }
    }
}

pub fn is_running(id: &str) -> bool {
    RUNNING
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .as_ref()
        .is_some_and(|s| s.contains(id))
}

#[derive(Clone)]
pub struct Ctx {
    pub progress: ProgressFn,
    /// SharePoint session cookie header for Teams recordings.
    pub cookie: Option<String>,
}

impl Ctx {
    pub fn new(progress: ProgressFn) -> Self {
        Self {
            progress,
            cookie: None,
        }
    }
}

fn set_status(id: &str, ctx: &Ctx, status: MeetingStatus) -> anyhow::Result<Meeting> {
    let m = super::update(id, |m| {
        m.status = status;
        if status != MeetingStatus::Failed {
            m.error = None;
        }
    })?;
    report(
        &ctx.progress,
        id,
        "status",
        0,
        None,
        Some(status_name(status).to_string()),
    );
    Ok(m)
}

pub fn status_name(s: MeetingStatus) -> &'static str {
    match s {
        MeetingStatus::Recording => "recording",
        MeetingStatus::BotLive => "bot_live",
        MeetingStatus::Queued => "queued",
        MeetingStatus::Downloading => "downloading",
        MeetingStatus::Transcribing => "transcribing",
        MeetingStatus::Summarizing => "summarizing",
        MeetingStatus::Ready => "ready",
        MeetingStatus::Failed => "failed",
    }
}

fn fail(id: &str, ctx: &Ctx, err: &anyhow::Error) {
    let msg = format!("{err:#}");
    let _ = super::update(id, |m| {
        m.status = MeetingStatus::Failed;
        m.error = Some(msg.clone());
    });
    report(&ctx.progress, id, "status", 0, None, Some("failed".into()));
}

/// Full run. On error the meeting is left `Failed` with the message.
pub async fn process(id: &str, ctx: Ctx) -> anyhow::Result<Meeting> {
    let _guard = RunGuard::take(id)?;
    match run(id, &ctx).await {
        Ok(m) => Ok(m),
        Err(e) => {
            fail(id, &ctx, &e);
            Err(e)
        }
    }
}

async fn run(id: &str, ctx: &Ctx) -> anyhow::Result<Meeting> {
    let m = super::load(id)?;
    let mut segments = super::load_transcript(id);
    if segments.is_empty() {
        let (segs, source) = acquire_and_transcribe(&m, ctx).await?;
        segments = segs;
        store_transcript(id, &segments, source, None)?;
    }
    finish(id, &segments, ctx).await
}

/// Summary (when asked for and possible) and markdown. A summary failure
/// does not fail the meeting: the transcript is the part that matters.
async fn finish(id: &str, segments: &[Segment], ctx: &Ctx) -> anyhow::Result<Meeting> {
    let s = settings::load();
    let m = super::load(id)?;
    let mut summary_error = None;
    if s.auto_summarize && crate::core::ai::get().is_configured() && !segments.is_empty() {
        set_status(id, ctx, MeetingStatus::Summarizing)?;
        if let Err(e) = summarize_into(&m, segments, &s).await {
            summary_error = Some(format!("Summary failed: {e:#}"));
        }
    }
    let m = super::update(id, |m| {
        m.status = MeetingStatus::Ready;
        m.error = summary_error;
    })?;
    super::write_markdown(&m)?;
    report(&ctx.progress, id, "status", 0, None, Some("ready".into()));
    Ok(m)
}

async fn summarize_into(
    m: &Meeting,
    segments: &[Segment],
    s: &settings::MeetingSettings,
) -> anyhow::Result<Meeting> {
    let lang = Some(s.summary_language.as_str()).filter(|l| !l.trim().is_empty());
    let summary = super::summary::summarize(m, segments, lang).await?;
    let auto_title = m.title == super::default_title(m.created_at_ms);
    super::update(&m.id, |m| {
        if auto_title && !summary.title.trim().is_empty() {
            m.title = summary.title.trim().to_string();
        }
        m.summary = Some(summary);
    })
}

/// Summarize again (after editing notes, or with another model).
pub async fn resummarize(id: &str, ctx: Ctx) -> anyhow::Result<Meeting> {
    let _guard = RunGuard::take(id)?;
    let segments = super::load_transcript(id);
    if segments.is_empty() {
        return Err(anyhow!("This meeting has no transcript yet"));
    }
    let result: anyhow::Result<Meeting> = async {
        let m = set_status(id, &ctx, MeetingStatus::Summarizing)?;
        let s = settings::load();
        summarize_into(&m, &segments, &s).await?;
        let m = super::update(id, |m| {
            m.status = MeetingStatus::Ready;
            m.error = None;
        })?;
        super::write_markdown(&m)?;
        report(&ctx.progress, id, "status", 0, None, Some("ready".into()));
        Ok(m)
    }
    .await;
    if let Err(e) = &result {
        // Keep the meeting usable: back to Ready with the error shown.
        let msg = format!("Summary failed: {e:#}");
        let _ = super::update(id, |m| {
            m.status = MeetingStatus::Ready;
            m.error = Some(msg);
        });
        report(&ctx.progress, id, "status", 0, None, Some("ready".into()));
    }
    result
}

/// Throws the transcript away and runs everything again from the audio.
pub async fn retranscribe(id: &str, ctx: Ctx) -> anyhow::Result<Meeting> {
    let m = super::load(id)?;
    if m.audio.is_none()
        && m.tracks.is_empty()
        && !matches!(
            m.source,
            MeetingSource::Teams { .. } | MeetingSource::File { .. }
        )
    {
        return Err(anyhow!("This meeting has no audio to transcribe again"));
    }
    super::save_transcript(id, &[])?;
    super::update(id, |m| {
        m.segment_count = 0;
        m.transcript_source = None;
    })?;
    process(id, ctx).await
}

/// Stores segments that came from outside (a bot) and finishes the meeting.
pub async fn ingest_segments(
    id: &str,
    segments: Vec<Segment>,
    source: &'static str,
    ctx: Ctx,
) -> anyhow::Result<Meeting> {
    let _guard = RunGuard::take(id)?;
    let result: anyhow::Result<Meeting> = async {
        if segments.is_empty() {
            return Err(anyhow!("The transcript came back empty"));
        }
        store_transcript(id, &segments, source, None)?;
        finish(id, &segments, &ctx).await
    }
    .await;
    if let Err(e) = &result {
        fail(id, &ctx, e);
    }
    result
}

fn store_transcript(
    id: &str,
    segments: &[Segment],
    source: &str,
    language: Option<String>,
) -> anyhow::Result<()> {
    super::save_transcript(id, segments)?;
    let speakers = transcript::speakers(segments);
    let end = segments.iter().map(|s| s.end_ms).max().unwrap_or(0);
    super::update(id, |m| {
        m.segment_count = segments.len();
        m.speakers = speakers;
        m.transcript_source = Some(source.to_string());
        if language.is_some() {
            m.language = language;
        }
        if m.duration_ms == 0 {
            m.duration_ms = end;
        }
    })?;
    Ok(())
}

fn is_transcript_file(p: &Path) -> bool {
    p.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| matches!(e.to_ascii_lowercase().as_str(), "vtt" | "srt" | "json"))
}

async fn probe_ms(p: &Path) -> u64 {
    crate::core::ffmpeg::probe(p)
        .await
        .map(|i| (i.duration_seconds * 1000.0) as u64)
        .unwrap_or(0)
}

/// Converts any local media to `audio.m4a` in the meeting folder.
async fn extract_audio(input: &Path, dest: &Path) -> anyhow::Result<()> {
    let ffmpeg = crate::core::dependencies::ensure_ffmpeg().await?;
    let out = crate::core::process::command(&ffmpeg)
        .args(["-y", "-hide_banner", "-loglevel", "error", "-i"])
        .arg(input)
        .args(["-vn", "-ac", "1", "-c:a", "aac", "-b:a", "64k"])
        .arg(dest)
        .output()
        .await?;
    if !out.status.success() {
        return Err(anyhow!(
            "ffmpeg could not read {}: {}",
            input.display(),
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    Ok(())
}

async fn acquire_and_transcribe(
    m: &Meeting,
    ctx: &Ctx,
) -> anyhow::Result<(Vec<Segment>, &'static str)> {
    let id = m.id.as_str();
    let dir = super::dir_of(id)?;
    let s = settings::load();
    match &m.source {
        MeetingSource::File { path } => {
            let input = PathBuf::from(path);
            if !input.exists() {
                return Err(anyhow!("File not found: {path}"));
            }
            if is_transcript_file(&input) {
                let text = tokio::fs::read_to_string(&input).await?;
                let segs = transcript::parse_any(&text);
                if segs.is_empty() {
                    return Err(anyhow!("No transcript cues found in {path}"));
                }
                return Ok((segs, "file"));
            }
            if m.audio.is_none() {
                set_status(id, ctx, MeetingStatus::Downloading)?;
                let dest = super::audio_path(id, "m4a")?;
                extract_audio(&input, &dest).await?;
                let ms = probe_ms(&dest).await;
                super::update(id, |m| {
                    m.audio = Some("audio.m4a".into());
                    m.duration_ms = ms;
                })?;
            }
            transcribe_file(id, &dir.join("audio.m4a"), &s, ctx).await
        }
        MeetingSource::Teams { url } => {
            let input = teams::classify(url)
                .ok_or_else(|| anyhow!("Not a Teams/SharePoint recording link: {url}"))?;
            if m.audio.is_none() {
                set_status(id, ctx, MeetingStatus::Downloading)?;
            }
            match input {
                TeamsInput::TranscriptUrl(t) => Ok((
                    teams::fetch_transcript(&t, ctx.cookie.as_deref()).await?,
                    "teams",
                )),
                TeamsInput::Manifest(u) => {
                    if m.audio.is_none() {
                        let dest = super::audio_path(id, "m4a")?;
                        teams::download_audio(&u, &dest, ctx.progress.clone(), id).await?;
                        let ms = probe_ms(&dest).await;
                        super::update(id, |m| {
                            m.audio = Some("audio.m4a".into());
                            m.duration_ms = ms;
                        })?;
                    }
                    transcribe_file(id, &dir.join("audio.m4a"), &s, ctx).await
                }
                sp @ TeamsInput::SharePoint { .. } => {
                    if let Some(audio) = &m.audio {
                        return transcribe_file(id, &dir.join(audio), &s, ctx).await;
                    }
                    let cookie = ctx.cookie.clone().or_else(|| sharepoint_cookie(&sp));
                    let rec = teams::resolve_sharepoint(&sp, cookie.as_deref()).await?;
                    if m.title == super::default_title(m.created_at_ms) {
                        let stem = Path::new(&rec.name)
                            .file_stem()
                            .map(|s| s.to_string_lossy().to_string())
                            .unwrap_or_default();
                        if !stem.is_empty() {
                            let _ = super::update(id, |m| m.title = stem);
                        }
                    }
                    if let Some(t) = &rec.transcript_url {
                        if let Ok(segs) = teams::fetch_transcript(t, cookie.as_deref()).await {
                            return Ok((segs, "teams"));
                        }
                    }
                    let Some(dl) = rec.download_url else {
                        return Err(anyhow!(
                            "This recording has no Teams transcript and its owner blocked downloads. Open it in the browser, copy the videomanifest request from DevTools (Network tab, filter \"videomanifest\") and import that URL instead"
                        ));
                    };
                    let dest = super::audio_path(id, "m4a")?;
                    teams::download_audio(&dl, &dest, ctx.progress.clone(), id)
                        .await
                        .context("downloading the recording")?;
                    let ms = probe_ms(&dest).await;
                    super::update(id, |m| {
                        m.audio = Some("audio.m4a".into());
                        m.duration_ms = ms;
                    })?;
                    transcribe_file(id, &dir.join("audio.m4a"), &s, ctx).await
                }
            }
        }
        MeetingSource::Capture { .. } => {
            let mic = m
                .tracks
                .iter()
                .find(|t| t.role == "mic")
                .map(|t| dir.join(&t.file));
            let sys = m
                .tracks
                .iter()
                .find(|t| t.role == "system")
                .map(|t| dir.join(&t.file));
            match (mic, sys) {
                (Some(mic), Some(sys)) if mic.exists() && sys.exists() => {
                    set_status(id, ctx, MeetingStatus::Transcribing)?;
                    let a = whisper::transcribe(&mic, &s, ctx.progress.clone(), id).await?;
                    let b = whisper::transcribe(&sys, &s, ctx.progress.clone(), id).await?;
                    let lang = b.language.clone().or(a.language.clone());
                    super::update(id, |m| m.language = lang)?;
                    Ok((merge_tracks(a.segments, b.segments), a.source))
                }
                _ => {
                    let audio = m
                        .audio
                        .as_ref()
                        .map(|a| dir.join(a))
                        .filter(|p| p.exists())
                        .ok_or_else(|| anyhow!("The recording has no audio"))?;
                    transcribe_file(id, &audio, &s, ctx).await
                }
            }
        }
        MeetingSource::Bot { .. } => Err(anyhow!(
            "Bot meetings get their transcript from the bot: refresh it instead"
        )),
    }
}

async fn transcribe_file(
    id: &str,
    audio: &Path,
    s: &settings::MeetingSettings,
    ctx: &Ctx,
) -> anyhow::Result<(Vec<Segment>, &'static str)> {
    set_status(id, ctx, MeetingStatus::Transcribing)?;
    let t = whisper::transcribe(audio, s, ctx.progress.clone(), id).await?;
    if t.segments.is_empty() {
        return Err(anyhow!("Whisper heard no speech in this recording"));
    }
    super::update(id, |m| m.language = t.language.clone())?;
    Ok((t.segments, t.source))
}

/// SharePoint cookies the person already gave OmniGet (browser extension or
/// Settings > Cookies) for this tenant.
fn sharepoint_cookie(input: &TeamsInput) -> Option<String> {
    let TeamsInput::SharePoint { host, .. } = input else {
        return None;
    };
    crate::platforms::cookie_provider::cookie_header_for_domains(&[host.as_str(), "sharepoint.com"])
}

//! Official Teams meeting transcripts through Graph (needs
//! OnlineMeetingTranscript.Read.All, which an admin approves): find the
//! online meeting by its join link, list its transcripts, download one as
//! WebVTT with the speakers' names, and store it as an OmniGet meeting.

use anyhow::anyhow;
use serde::Serialize;

use crate::core::meetings::{self, pipeline, transcript, Meeting, MeetingSource, MeetingStatus};
use crate::core::tools::ProgressFn;

#[derive(Debug, Clone, Serialize)]
pub struct TranscriptInfo {
    pub meeting_id: String,
    pub transcript_id: String,
    pub created: String,
}

/// The online meeting behind a join link (the person must be organizer or
/// invited). Graph wants the link exactly as the invite has it.
pub async fn online_meeting_id(join_url: &str) -> anyhow::Result<String> {
    let filter = format!("JoinWebUrl eq '{}'", join_url.replace('\'', "''"));
    let v = super::get(&format!(
        "/me/onlineMeetings?$filter={}",
        urlencoding::encode(&filter)
    ))
    .await?;
    v["value"][0]["id"]
        .as_str()
        .map(str::to_string)
        .ok_or_else(|| anyhow!("No online meeting with that join link is visible to your account"))
}

pub async fn list(join_url: &str) -> anyhow::Result<Vec<TranscriptInfo>> {
    let mid = online_meeting_id(join_url).await?;
    let v = super::get(&format!(
        "/me/onlineMeetings/{}/transcripts",
        urlencoding::encode(&mid)
    ))
    .await?;
    let mut out: Vec<TranscriptInfo> = v["value"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|t| TranscriptInfo {
            meeting_id: mid.clone(),
            transcript_id: t["id"].as_str().unwrap_or("").to_string(),
            created: t["createdDateTime"].as_str().unwrap_or("").to_string(),
        })
        .collect();
    out.sort_by(|a, b| b.created.cmp(&a.created));
    Ok(out)
}

pub async fn vtt(info: &TranscriptInfo) -> anyhow::Result<String> {
    let path = format!(
        "/me/onlineMeetings/{}/transcripts/{}/content?$format=text/vtt",
        urlencoding::encode(&info.meeting_id),
        urlencoding::encode(&info.transcript_id)
    );
    Ok(super::get_raw(&path, Some("text/vtt"))
        .await?
        .text()
        .await?)
}

/// Downloads the newest transcript of the meeting at `join_url` and turns
/// it into an OmniGet meeting (summary included when an AI is configured).
pub async fn import(
    join_url: &str,
    title: Option<&str>,
    progress: ProgressFn,
) -> anyhow::Result<Meeting> {
    let list = list(join_url).await?;
    let newest = list
        .first()
        .ok_or_else(|| anyhow!("This meeting has no transcript (transcription was not turned on, or it is still processing)"))?;
    let text = vtt(newest).await?;
    let segments = transcript::parse_any(&text);
    if segments.is_empty() {
        return Err(anyhow!("The transcript came back empty"));
    }
    let m = meetings::create(
        title,
        MeetingSource::Teams {
            url: join_url.to_string(),
        },
        MeetingStatus::Queued,
    )?;
    pipeline::ingest_segments(&m.id, segments, "teams", pipeline::Ctx::new(progress)).await
}

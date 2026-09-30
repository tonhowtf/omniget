//! Vexa (Apache-2.0, self-hosted or `api.cloud.vexa.ai`): a bot joins the
//! Teams / Meet / Zoom call and transcribes it with speaker names. OmniGet
//! sends the bot, follows its status and pulls the transcript into a meeting.
//!
//! API: `POST /bots {platform, native_meeting_id, passcode?, bot_name}`,
//! `GET /bots/status`, `GET /transcripts/{platform}/{id}`,
//! `DELETE /bots/{platform}/{id}`, all with `X-API-Key`.

use anyhow::anyhow;
use serde::Serialize;

use super::settings;
use super::Segment;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MeetingLink {
    /// "teams", "google_meet" or "zoom" (Vexa's platform names).
    pub platform: String,
    pub native_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub passcode: Option<String>,
}

/// Reads a join link into what Vexa wants.
///
/// - Teams: `teams.microsoft.com/meet/<digits>?p=<passcode>` (also
///   `teams.live.com`), or the classic `/l/meetup-join/19:meeting_...@thread.v2/...`
///   whose thread id Vexa takes as the id.
/// - Meet: `meet.google.com/abc-defg-hij`.
/// - Zoom: `*.zoom.us/j/<digits>?pwd=<passcode>`.
pub fn parse_link(raw: &str) -> Option<MeetingLink> {
    let url = url::Url::parse(raw.trim()).ok()?;
    let host = url.host_str()?.to_ascii_lowercase();
    let segs: Vec<String> = url
        .path_segments()
        .map(|s| {
            s.filter(|p| !p.is_empty())
                .map(|p| {
                    urlencoding::decode(p)
                        .map(|c| c.into_owned())
                        .unwrap_or_else(|_| p.to_string())
                })
                .collect()
        })
        .unwrap_or_default();
    let query = |k: &str| {
        url.query_pairs()
            .find(|(key, _)| key == k)
            .map(|(_, v)| v.to_string())
            .filter(|v| !v.is_empty())
    };
    if host == "teams.microsoft.com"
        || host == "teams.live.com"
        || host.ends_with(".teams.microsoft.com")
    {
        if let Some(i) = segs.iter().position(|s| s == "meet") {
            let id = segs.get(i + 1)?.clone();
            return Some(MeetingLink {
                platform: "teams".into(),
                native_id: id,
                passcode: query("p"),
            });
        }
        if let Some(i) = segs.iter().position(|s| s == "meetup-join") {
            let thread = segs.get(i + 1)?.clone();
            if thread.contains("@thread") {
                return Some(MeetingLink {
                    platform: "teams".into(),
                    native_id: thread,
                    passcode: None,
                });
            }
        }
        return None;
    }
    if host == "meet.google.com" {
        let code = segs.first()?.clone();
        let valid = code.len() == 12
            && code.chars().enumerate().all(|(i, c)| {
                if i == 3 || i == 8 {
                    c == '-'
                } else {
                    c.is_ascii_lowercase()
                }
            });
        return valid.then(|| MeetingLink {
            platform: "google_meet".into(),
            native_id: code,
            passcode: None,
        });
    }
    if host == "zoom.us" || host.ends_with(".zoom.us") {
        let i = segs.iter().position(|s| s == "j" || s == "wc")?;
        let id = segs.get(i + 1)?.clone();
        if !id.chars().all(|c| c.is_ascii_digit()) {
            return None;
        }
        return Some(MeetingLink {
            platform: "zoom".into(),
            native_id: id,
            passcode: query("pwd"),
        });
    }
    None
}

struct Api {
    base: String,
    key: String,
    client: reqwest::Client,
}

fn api() -> anyhow::Result<Api> {
    let s = settings::load();
    let key = settings::vexa_key().ok_or_else(|| {
        anyhow!("Add your Vexa API key in Meetings settings to send a bot (vexa.ai, or your self-hosted Vexa)")
    })?;
    Ok(Api {
        base: s.vexa_url.trim_end_matches('/').to_string(),
        key,
        client: crate::core::tools::client()?,
    })
}

async fn check(resp: reqwest::Response) -> anyhow::Result<serde_json::Value> {
    let status = resp.status();
    let text = resp.text().await.unwrap_or_default();
    if !status.is_success() {
        let detail = serde_json::from_str::<serde_json::Value>(&text)
            .ok()
            .and_then(|v| {
                v["detail"]
                    .as_str()
                    .map(str::to_string)
                    .or_else(|| Some(v["detail"].to_string()))
            })
            .filter(|d| d != "null")
            .unwrap_or_else(|| text.chars().take(200).collect());
        return Err(anyhow!("Vexa answered HTTP {}: {detail}", status.as_u16()));
    }
    Ok(serde_json::from_str(&text).unwrap_or(serde_json::Value::Null))
}

/// Sends the bot. `meeting_url` goes along so Vexa can resolve Teams links
/// it understands better than OmniGet does.
pub async fn send_bot(
    link: &MeetingLink,
    meeting_url: &str,
    language: Option<&str>,
) -> anyhow::Result<()> {
    let api = api()?;
    let s = settings::load();
    let mut body = serde_json::json!({
        "platform": link.platform,
        "native_meeting_id": link.native_id,
        "bot_name": s.bot_name,
    });
    if let Some(p) = &link.passcode {
        body["passcode"] = p.clone().into();
    }
    if link.platform == "teams" {
        body["meeting_url"] = meeting_url.into();
    }
    if let Some(l) = language.filter(|l| !l.is_empty() && *l != "auto") {
        body["language"] = l.into();
    }
    let resp = api
        .client
        .post(format!("{}/bots", api.base))
        .header("X-API-Key", &api.key)
        .json(&body)
        .send()
        .await?;
    check(resp).await.map(|_| ())
}

pub async fn stop_bot(link: &MeetingLink) -> anyhow::Result<()> {
    let api = api()?;
    let resp = api
        .client
        .delete(format!(
            "{}/bots/{}/{}",
            api.base,
            link.platform,
            urlencoding::encode(&link.native_id)
        ))
        .header("X-API-Key", &api.key)
        .send()
        .await?;
    check(resp).await.map(|_| ())
}

#[derive(Debug, Clone, Serialize)]
pub struct BotTranscript {
    pub status: String,
    pub segments: Vec<Segment>,
    pub finished: bool,
}

pub async fn transcript(link: &MeetingLink) -> anyhow::Result<BotTranscript> {
    let api = api()?;
    let resp = api
        .client
        .get(format!(
            "{}/transcripts/{}/{}",
            api.base,
            link.platform,
            urlencoding::encode(&link.native_id)
        ))
        .header("X-API-Key", &api.key)
        .send()
        .await?;
    let v = check(resp).await?;
    Ok(parse_transcript(&v))
}

fn parse_transcript(v: &serde_json::Value) -> BotTranscript {
    let status = v["status"].as_str().unwrap_or("").to_string();
    let base = v["start_time"]
        .as_str()
        .and_then(|t| chrono::DateTime::parse_from_rfc3339(t).ok())
        .map(|t| t.timestamp_millis());
    let mut segments: Vec<Segment> = v["segments"]
        .as_array()
        .into_iter()
        .flatten()
        // Live drafts come as completed:false and are replaced later.
        .filter(|s| s["completed"].as_bool() != Some(false))
        .filter_map(|s| {
            let text = s["text"].as_str()?.trim();
            if text.is_empty() {
                return None;
            }
            // Relative seconds when given; else absolute times against start.
            let rel = |k: &str, abs: &str| -> u64 {
                if let Some(x) = s[k].as_f64() {
                    return (x * 1000.0).max(0.0) as u64;
                }
                match (
                    base,
                    s[abs]
                        .as_str()
                        .and_then(|t| chrono::DateTime::parse_from_rfc3339(t).ok()),
                ) {
                    (Some(b), Some(t)) => (t.timestamp_millis() - b).max(0) as u64,
                    _ => 0,
                }
            };
            let start = rel("start", "absolute_start_time");
            let end = rel("end", "absolute_end_time").max(start);
            Some(Segment {
                start_ms: start,
                end_ms: end,
                speaker: s["speaker"]
                    .as_str()
                    .map(str::trim)
                    .filter(|x| !x.is_empty())
                    .map(str::to_string),
                text: text.to_string(),
            })
        })
        .collect();
    segments.sort_by_key(|s| s.start_ms);
    segments.dedup_by(|a, b| a.start_ms == b.start_ms && a.text == b.text);
    let finished = matches!(
        status.as_str(),
        "completed" | "failed" | "stopped" | "ended"
    );
    BotTranscript {
        status,
        segments,
        finished,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_teams_links() {
        let l = parse_link("https://teams.microsoft.com/meet/3845612903427?p=AbCd12").unwrap();
        assert_eq!(l.platform, "teams");
        assert_eq!(l.native_id, "3845612903427");
        assert_eq!(l.passcode.as_deref(), Some("AbCd12"));

        let classic = "https://teams.microsoft.com/l/meetup-join/19%3ameeting_NjA0YjM1%40thread.v2/0?context=%7b%7d";
        let l = parse_link(classic).unwrap();
        assert_eq!(l.native_id, "19:meeting_NjA0YjM1@thread.v2");

        assert!(parse_link("https://teams.microsoft.com/v2/").is_none());
    }

    #[test]
    fn parses_meet_and_zoom() {
        let m = parse_link("https://meet.google.com/abc-defg-hij?authuser=0").unwrap();
        assert_eq!(
            (m.platform.as_str(), m.native_id.as_str()),
            ("google_meet", "abc-defg-hij")
        );
        assert!(parse_link("https://meet.google.com/landing").is_none());
        let z = parse_link("https://us02web.zoom.us/j/81234567890?pwd=xyz").unwrap();
        assert_eq!(z.platform, "zoom");
        assert_eq!(z.passcode.as_deref(), Some("xyz"));
    }

    #[test]
    fn parses_transcript_segments() {
        let v = serde_json::json!({
            "status": "completed",
            "start_time": "2026-09-29T10:00:00Z",
            "segments": [
                {"speaker":"Ana","text":"Oi","start":1.0,"end":2.0,"completed":true},
                {"speaker":"Bruno","text":"rascunho","start":3.0,"end":4.0,"completed":false},
                {"speaker":"Bruno","text":"Tudo certo","absolute_start_time":"2026-09-29T10:00:05Z","absolute_end_time":"2026-09-29T10:00:07Z"}
            ]
        });
        let t = parse_transcript(&v);
        assert!(t.finished);
        assert_eq!(t.segments.len(), 2);
        assert_eq!(t.segments[1].start_ms, 5000);
        assert_eq!(t.segments[1].end_ms, 7000);
        assert_eq!(t.segments[1].speaker.as_deref(), Some("Bruno"));
    }
}

pub mod markdown;
pub mod pipeline;
pub mod publish;
pub mod settings;
pub mod summary;
pub mod teams;
pub mod transcript;
pub mod vexa;
pub mod whisper;

use std::path::{Path, PathBuf};

use anyhow::{anyhow, Context};
use serde::{Deserialize, Serialize};

pub use transcript::Segment;

const MEETING_FILE: &str = "meeting.json";
const TRANSCRIPT_FILE: &str = "transcript.json";
pub const MARKDOWN_FILE: &str = "meeting.md";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum MeetingSource {
    Capture {
        mic: bool,
        system: bool,
    },
    File {
        path: String,
    },
    Teams {
        url: String,
    },
    Bot {
        platform: String,
        meeting_url: String,
        native_id: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MeetingStatus {
    Recording,
    BotLive,
    Queued,
    Downloading,
    Transcribing,
    Summarizing,
    Ready,
    Failed,
}

impl MeetingStatus {
    pub fn is_busy(self) -> bool {
        matches!(
            self,
            Self::Queued | Self::Downloading | Self::Transcribing | Self::Summarizing
        )
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ActionItem {
    pub task: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub due: Option<String>,
    #[serde(default)]
    pub done: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Summary {
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub overview: String,
    #[serde(default)]
    pub key_points: Vec<String>,
    #[serde(default)]
    pub decisions: Vec<String>,
    #[serde(default)]
    pub action_items: Vec<ActionItem>,
    #[serde(default)]
    pub open_questions: Vec<String>,
    #[serde(default)]
    pub model: String,
    #[serde(default)]
    pub created_at_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Meeting {
    pub id: String,
    pub title: String,
    pub created_at_ms: u64,
    #[serde(default)]
    pub duration_ms: u64,
    pub source: MeetingSource,
    pub status: MeetingStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub audio: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tracks: Vec<Track>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transcript_source: Option<String>,
    #[serde(default)]
    pub segment_count: usize,
    #[serde(default)]
    pub speakers: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<Summary>,
    #[serde(default)]
    pub notes: String,
    #[serde(default)]
    pub updated_at_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Track {
    pub role: String,
    pub file: String,
}

pub fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

pub fn root() -> Option<PathBuf> {
    if let Ok(dir) = std::env::var("OMNIGET_MEETINGS_DIR") {
        if !dir.trim().is_empty() {
            return Some(PathBuf::from(dir));
        }
    }
    crate::core::paths::app_data_dir().map(|d| d.join("meetings"))
}

fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

pub fn dir_of(id: &str) -> anyhow::Result<PathBuf> {
    if !valid_id(id) {
        return Err(anyhow!("invalid meeting id"));
    }
    Ok(root()
        .ok_or_else(|| anyhow!("Could not determine data directory"))?
        .join(id))
}

fn new_id(created_at_ms: u64) -> String {
    let stamp = chrono::DateTime::from_timestamp_millis(created_at_ms as i64)
        .map(|d| d.format("%Y%m%d-%H%M%S").to_string())
        .unwrap_or_else(|| created_at_ms.to_string());
    let rand = uuid::Uuid::new_v4().simple().to_string();
    format!("{stamp}-{}", &rand[..6])
}

pub fn default_title(created_at_ms: u64) -> String {
    chrono::DateTime::from_timestamp_millis(created_at_ms as i64)
        .map(|d| {
            d.with_timezone(&chrono::Local)
                .format("Meeting %Y-%m-%d %H:%M")
                .to_string()
        })
        .unwrap_or_else(|| "Meeting".to_string())
}

pub fn create(
    title: Option<&str>,
    source: MeetingSource,
    status: MeetingStatus,
) -> anyhow::Result<Meeting> {
    let created = now_ms();
    let id = new_id(created);
    std::fs::create_dir_all(dir_of(&id)?)?;
    let title = title
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| default_title(created));
    let m = Meeting {
        id,
        title,
        created_at_ms: created,
        duration_ms: 0,
        source,
        status,
        error: None,
        audio: None,
        tracks: Vec::new(),
        language: None,
        transcript_source: None,
        segment_count: 0,
        speakers: Vec::new(),
        summary: None,
        notes: String::new(),
        updated_at_ms: created,
    };
    save(&m)?;
    Ok(m)
}

fn write_atomic(path: &Path, bytes: &[u8]) -> anyhow::Result<()> {
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

pub fn save(m: &Meeting) -> anyhow::Result<()> {
    let dir = dir_of(&m.id)?;
    std::fs::create_dir_all(&dir)?;
    let mut m = m.clone();
    m.updated_at_ms = now_ms();
    write_atomic(&dir.join(MEETING_FILE), &serde_json::to_vec_pretty(&m)?)
}

pub fn load(id: &str) -> anyhow::Result<Meeting> {
    let path = dir_of(id)?.join(MEETING_FILE);
    let text = std::fs::read_to_string(&path).with_context(|| format!("meeting {id} not found"))?;
    Ok(serde_json::from_str(&text)?)
}

pub fn update(id: &str, f: impl FnOnce(&mut Meeting)) -> anyhow::Result<Meeting> {
    let mut m = load(id)?;
    f(&mut m);
    save(&m)?;
    Ok(m)
}

pub fn list() -> Vec<Meeting> {
    let Some(root) = root() else {
        return Vec::new();
    };
    let Ok(entries) = std::fs::read_dir(&root) else {
        return Vec::new();
    };
    let mut out: Vec<Meeting> = entries
        .flatten()
        .filter_map(|e| {
            let text = std::fs::read_to_string(e.path().join(MEETING_FILE)).ok()?;
            serde_json::from_str::<Meeting>(&text).ok()
        })
        .collect();
    out.sort_by_key(|m| std::cmp::Reverse(m.created_at_ms));
    out
}

pub fn delete(id: &str) -> anyhow::Result<()> {
    let dir = dir_of(id)?;
    if dir.exists() {
        std::fs::remove_dir_all(dir)?;
    }
    Ok(())
}

pub fn save_transcript(id: &str, segments: &[Segment]) -> anyhow::Result<()> {
    let dir = dir_of(id)?;
    write_atomic(&dir.join(TRANSCRIPT_FILE), &serde_json::to_vec(segments)?)
}

pub fn load_transcript(id: &str) -> Vec<Segment> {
    dir_of(id)
        .ok()
        .and_then(|d| std::fs::read_to_string(d.join(TRANSCRIPT_FILE)).ok())
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

pub fn write_markdown(m: &Meeting) -> anyhow::Result<PathBuf> {
    let segs = load_transcript(&m.id);
    let md = markdown::render(m, &segs);
    let path = dir_of(&m.id)?.join(MARKDOWN_FILE);
    write_atomic(&path, md.as_bytes())?;
    Ok(path)
}

pub fn read_markdown(id: &str) -> anyhow::Result<String> {
    let m = load(id)?;
    let path = dir_of(id)?.join(MARKDOWN_FILE);
    match std::fs::read_to_string(&path) {
        Ok(t) => Ok(t),
        Err(_) => Ok(markdown::render(&m, &load_transcript(id))),
    }
}

pub fn recover_interrupted() {
    for m in list() {
        if m.status.is_busy() || m.status == MeetingStatus::Recording {
            let _ = update(&m.id, |m| {
                m.status = MeetingStatus::Failed;
                m.error = Some(
                    "Interrupted: OmniGet closed while this meeting was being processed.".into(),
                );
            });
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct SearchHit {
    pub id: String,
    pub title: String,
    pub created_at_ms: u64,
    pub field: String,
    pub snippet: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub at_ms: Option<u64>,
}

pub fn search(query: &str, limit: usize) -> Vec<SearchHit> {
    let words: Vec<String> = query
        .split_whitespace()
        .map(|w| w.to_lowercase())
        .filter(|w| !w.is_empty())
        .collect();
    if words.is_empty() {
        return Vec::new();
    }
    let hit = |text: &str| {
        let low = text.to_lowercase();
        words.iter().all(|w| low.contains(w))
    };
    let mut out = Vec::new();
    for m in list() {
        let mut push = |field: &str, snippet: &str, at_ms: Option<u64>| {
            out.push(SearchHit {
                id: m.id.clone(),
                title: m.title.clone(),
                created_at_ms: m.created_at_ms,
                field: field.to_string(),
                snippet: snippet.chars().take(280).collect(),
                at_ms,
            });
        };
        if hit(&m.title) {
            push("title", &m.title, None);
        }
        if let Some(s) = &m.summary {
            if hit(&s.overview) {
                push("summary", &s.overview, None);
            }
            for p in s.key_points.iter().chain(&s.decisions) {
                if hit(p) {
                    push("summary", p, None);
                }
            }
            for a in &s.action_items {
                if hit(&a.task) {
                    push("action_item", &a.task, None);
                }
            }
        }
        for seg in load_transcript(&m.id) {
            if hit(&seg.text) {
                let line = match &seg.speaker {
                    Some(sp) => format!("{sp}: {}", seg.text),
                    None => seg.text.clone(),
                };
                push("transcript", &line, Some(seg.start_ms));
            }
        }
        if out.len() >= limit {
            break;
        }
    }
    out.truncate(limit);
    out
}

#[derive(Debug, Clone, Serialize)]
pub struct OpenItem {
    pub meeting_id: String,
    pub meeting_title: String,
    pub meeting_date_ms: u64,
    pub index: usize,
    #[serde(flatten)]
    pub item: ActionItem,
}

pub fn action_items(include_done: bool, owner: Option<&str>) -> Vec<OpenItem> {
    let owner = owner
        .map(|o| o.trim().to_lowercase())
        .filter(|o| !o.is_empty());
    let mut out = Vec::new();
    for m in list() {
        let Some(s) = &m.summary else { continue };
        for (index, a) in s.action_items.iter().enumerate() {
            if a.done && !include_done {
                continue;
            }
            if let Some(o) = &owner {
                if !a
                    .owner
                    .as_deref()
                    .is_some_and(|x| x.to_lowercase().contains(o))
                {
                    continue;
                }
            }
            out.push(OpenItem {
                meeting_id: m.id.clone(),
                meeting_title: m.title.clone(),
                meeting_date_ms: m.created_at_ms,
                index,
                item: a.clone(),
            });
        }
    }
    out
}

pub fn set_action_done(id: &str, index: usize, done: bool) -> anyhow::Result<Meeting> {
    let current = load(id)?;
    if current
        .summary
        .as_ref()
        .is_none_or(|s| index >= s.action_items.len())
    {
        return Err(anyhow!("meeting {id} has no action item {index}"));
    }
    let m = update(id, |m| {
        if let Some(item) = m
            .summary
            .as_mut()
            .and_then(|s| s.action_items.get_mut(index))
        {
            item.done = done;
        }
    })?;
    write_markdown(&m)?;
    Ok(m)
}

pub fn audio_path(id: &str, ext: &str) -> anyhow::Result<PathBuf> {
    let ext = ext.trim_start_matches('.');
    let ext = if ext.is_empty() { "m4a" } else { ext };
    Ok(dir_of(id)?.join(format!("audio.{ext}")))
}

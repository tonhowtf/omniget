pub mod addons;
pub mod anilist;
pub mod cast;
pub mod db;
pub mod iptv;
pub mod jellyfin;
pub mod link;
pub mod notes;
pub mod probe;
pub mod scan;
pub mod server;
pub mod sources;
pub mod study;

use serde::{Deserialize, Serialize};

pub use server::ServerInfo;

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum RootKind {
    Library,
    Course,
}

impl RootKind {
    pub fn as_str(self) -> &'static str {
        match self {
            RootKind::Library => "library",
            RootKind::Course => "course",
        }
    }
    pub fn parse(s: &str) -> Self {
        if s == "course" {
            RootKind::Course
        } else {
            RootKind::Library
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Root {
    pub path: String,
    pub kind: RootKind,
    pub added_at: i64,
    pub online: bool,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum MediaKind {
    Movie,
    Episode,
    Lesson,
    Video,
}

impl MediaKind {
    pub fn as_str(self) -> &'static str {
        match self {
            MediaKind::Movie => "movie",
            MediaKind::Episode => "episode",
            MediaKind::Lesson => "lesson",
            MediaKind::Video => "video",
        }
    }
    pub fn parse(s: &str) -> Self {
        match s {
            "movie" => MediaKind::Movie,
            "episode" => MediaKind::Episode,
            "lesson" => MediaKind::Lesson,
            _ => MediaKind::Video,
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum SkipKind {
    Intro,
    Outro,
    Recap,
    Sponsor,
    Preview,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Chapter {
    pub start_ms: i64,
    pub end_ms: i64,
    pub title: String,
    #[serde(default)]
    pub kind: Option<SkipKind>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct StreamInfo {
    pub index: i64,
    pub kind: String,
    pub codec: String,
    pub lang: String,
    pub title: String,
    pub default: bool,
    pub forced: bool,
    #[serde(default)]
    pub channels: Option<i64>,
    #[serde(default)]
    pub width: Option<i64>,
    #[serde(default)]
    pub height: Option<i64>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct Progress {
    pub key: String,
    pub position_ms: i64,
    pub duration_ms: i64,
    pub watched: bool,
    pub play_count: i64,
    pub last_played: Option<i64>,
    pub seen_ratio: f64,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct MediaItem {
    pub id: i64,
    pub key: String,
    pub path: String,
    pub root: String,
    pub folder: String,
    pub kind: MediaKind,
    pub title: String,
    pub show: Option<String>,
    pub season: Option<i64>,
    pub episode: Option<i64>,
    pub episode_end: Option<i64>,
    pub year: Option<i64>,
    pub lesson_no: Option<i64>,
    pub module: Option<String>,
    pub course_id: Option<i64>,
    pub duration_ms: i64,
    pub width: Option<i64>,
    pub height: Option<i64>,
    pub vcodec: String,
    pub acodec: String,
    pub container: String,
    pub size: i64,
    pub has_subs: bool,
    pub thumb: Option<String>,
    pub added_at: i64,
    pub offline: bool,
    pub progress: Option<Progress>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct ItemsQuery {
    #[serde(default)]
    pub kind: Option<MediaKind>,
    #[serde(default)]
    pub root: Option<String>,
    #[serde(default)]
    pub folder: Option<String>,
    #[serde(default)]
    pub search: Option<String>,
    #[serde(default)]
    pub limit: Option<i64>,
    #[serde(default)]
    pub offset: Option<i64>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum SourceRef {
    Local {
        id: i64,
    },
    Url {
        url: String,
    },
    Jellyfin {
        source: i64,
        item: String,
    },
    Iptv {
        channel: i64,
    },
    Addon {
        addon: i64,
        #[serde(rename = "type")]
        typ: String,
        id: String,
        #[serde(default)]
        stream_index: Option<i64>,
    },
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ContinueEntry {
    pub key: String,
    pub title: String,
    pub subtitle: String,
    pub thumb: Option<String>,
    pub progress: Progress,
    pub kind: String,
    pub source: SourceRef,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct CourseSummary {
    pub id: i64,
    pub title: String,
    pub root_path: String,
    pub lessons: i64,
    pub watched: i64,
    pub duration_ms: i64,
    pub watched_ms: i64,
    pub last_played: Option<i64>,
    pub next_lesson: Option<i64>,
    pub thumb: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Attachment {
    pub path: String,
    pub name: String,
    pub kind: String,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Lesson {
    #[serde(flatten)]
    pub item: MediaItem,
    pub number: String,
    pub attachments: Vec<Attachment>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct CourseModule {
    pub index: i64,
    pub title: String,
    pub lessons: Vec<Lesson>,
    pub duration_ms: i64,
    pub watched: i64,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct CourseDetail {
    pub course: CourseSummary,
    pub modules: Vec<CourseModule>,
    pub loose_attachments: Vec<Attachment>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ShowSummary {
    pub name: String,
    pub seasons: i64,
    pub episodes: i64,
    pub watched: i64,
    pub thumb: Option<String>,
    pub year: Option<i64>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ShowSeason {
    pub season: i64,
    pub episodes: Vec<MediaItem>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ShowDetail {
    pub name: String,
    pub seasons: Vec<ShowSeason>,
    pub next: Option<i64>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct HomeCounts {
    pub movies: i64,
    pub shows: i64,
    pub courses: i64,
    pub videos: i64,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Home {
    #[serde(rename = "continue")]
    pub continue_: Vec<ContinueEntry>,
    pub recent: Vec<MediaItem>,
    pub courses: Vec<CourseSummary>,
    pub counts: HomeCounts,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PlayMode {
    Direct,
    Remux,
    TranscodeAudio,
    Transcode,
    Hls,
    Remote,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct SubtitleTrack {
    pub id: String,
    pub label: String,
    pub lang: String,
    pub url: String,
    pub default: bool,
    pub forced: bool,
    pub source: String,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct AudioTrack {
    pub index: i64,
    pub label: String,
    pub lang: String,
    pub codec: String,
    pub default: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct PlaybackMediaInfo {
    pub container: String,
    pub vcodec: String,
    pub acodec: String,
    pub width: Option<i64>,
    pub height: Option<i64>,
    pub bitrate: Option<i64>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct PlayPrefs {
    pub speed: f64,
    pub subtitle_lang: Option<String>,
    pub subtitle_id: Option<String>,
    pub audio_index: Option<i64>,
    pub subtitle_delay_ms: i64,
    pub subtitle_size: i64,
    pub subtitle_bg: bool,
    pub volume: f64,
    pub skip_silence: String,
    pub auto_pause: bool,
}

impl Default for PlayPrefs {
    fn default() -> Self {
        Self {
            speed: 1.0,
            subtitle_lang: None,
            subtitle_id: None,
            audio_index: None,
            subtitle_delay_ms: 0,
            subtitle_size: 100,
            subtitle_bg: true,
            volume: 1.0,
            skip_silence: "off".into(),
            auto_pause: false,
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct PlaybackInfo {
    pub key: String,
    pub title: String,
    pub subtitle: String,
    pub source: SourceRef,
    pub url: String,
    pub mode: PlayMode,
    pub seek_reloads: bool,
    pub offset_ms: i64,
    pub reason: String,
    pub duration_ms: i64,
    pub start_ms: i64,
    pub is_live: bool,
    pub subtitles: Vec<SubtitleTrack>,
    pub audio_tracks: Vec<AudioTrack>,
    pub chapters: Vec<Chapter>,
    pub info: PlaybackMediaInfo,
    pub prefs: PlayPrefs,
    pub next: Option<SourceRef>,
    pub prev: Option<SourceRef>,
    pub course_id: Option<i64>,
}

impl PlaybackInfo {
    pub fn remote(key: String, source: SourceRef, url: String) -> Self {
        Self {
            key,
            title: String::new(),
            subtitle: String::new(),
            source,
            url,
            mode: PlayMode::Remote,
            seek_reloads: false,
            offset_ms: 0,
            reason: String::new(),
            duration_ms: 0,
            start_ms: 0,
            is_live: false,
            subtitles: Vec::new(),
            audio_tracks: Vec::new(),
            chapters: Vec::new(),
            info: PlaybackMediaInfo::default(),
            prefs: PlayPrefs::default(),
            next: None,
            prev: None,
            course_id: None,
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ProgressReport {
    pub key: String,
    pub position_ms: i64,
    pub duration_ms: i64,
    pub playing: bool,
    #[serde(default)]
    pub watched_from_ms: Option<i64>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Note {
    pub id: i64,
    pub key: String,
    pub t_ms: i64,
    pub t_end_ms: Option<i64>,
    pub text: String,
    pub image: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Card {
    pub id: i64,
    pub key: String,
    pub front: String,
    pub back: String,
    pub t_ms: Option<i64>,
    pub image: Option<String>,
    pub due: i64,
    pub interval_days: f64,
    pub ease: f64,
    pub reps: i64,
    pub created_at: i64,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Cue {
    pub start_ms: i64,
    pub end_ms: i64,
    pub text: String,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct TranscriptHit {
    pub key: String,
    pub title: String,
    pub media_id: Option<i64>,
    pub start_ms: i64,
    pub text: String,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct HistoryEntry {
    pub key: String,
    pub title: String,
    pub subtitle: String,
    pub at: i64,
    pub position_ms: i64,
    pub duration_ms: i64,
    pub source: SourceRef,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ScanProgress {
    pub done: usize,
    pub total: usize,
    pub current: String,
    pub finished: bool,
}

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use rusqlite::{params, OptionalExtension};
use tauri::{AppHandle, Emitter};

use omniget_core::core::video::{naming, progress as rules};

use db::MediaRow;

async fn blocking<T: Send + 'static>(
    f: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    tokio::task::spawn_blocking(f)
        .await
        .map_err(|e| e.to_string())?
}

fn natural(a: &str, b: &str) -> std::cmp::Ordering {
    naming::natural_cmp(a, b)
}

fn sort_natural(rows: &mut [MediaRow]) {
    rows.sort_by(|a, b| natural(&a.item.path, &b.item.path));
}

fn sort_episodes(rows: &mut [MediaRow]) {
    rows.sort_by(|a, b| {
        let ka = (
            a.item.season.unwrap_or(i64::MAX),
            a.item.episode.unwrap_or(i64::MAX),
        );
        let kb = (
            b.item.season.unwrap_or(i64::MAX),
            b.item.episode.unwrap_or(i64::MAX),
        );
        ka.cmp(&kb)
            .then_with(|| natural(&a.item.path, &b.item.path))
    });
}

fn sort_course(rows: &mut [MediaRow]) {
    rows.sort_by(|a, b| a.ord.cmp(&b.ord).then_with(|| natural(&a.rel, &b.rel)));
}

fn folder_name(p: &str) -> String {
    Path::new(p)
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default()
}

pub fn local_labels(row: &MediaRow) -> (String, String) {
    let it = &row.item;
    match it.kind {
        MediaKind::Episode => {
            let show = it.show.clone().unwrap_or_default();
            let code = match (it.season, it.episode) {
                (Some(s), Some(e)) => match it.episode_end {
                    Some(end) if end > e => format!("S{s:02}E{e:02}-E{end:02}"),
                    _ => format!("S{s:02}E{e:02}"),
                },
                (None, Some(e)) => format!("E{e:02}"),
                _ => String::new(),
            };
            let sub = [code, show.clone()]
                .into_iter()
                .filter(|s| !s.is_empty())
                .collect::<Vec<_>>()
                .join(" · ");
            let title = if it.title.trim().is_empty() {
                show
            } else {
                it.title.clone()
            };
            (title, sub)
        }
        MediaKind::Lesson => {
            let num = row
                .number
                .clone()
                .or_else(|| it.lesson_no.map(|n| n.to_string()));
            let mut parts = Vec::new();
            if let Some(n) = num.filter(|n| !n.is_empty()) {
                parts.push(format!("Lesson {n}"));
            }
            if let Some(m) = it.module.clone().filter(|m| !m.is_empty()) {
                parts.push(m);
            }
            (it.title.clone(), parts.join(" · "))
        }
        MediaKind::Movie => (
            it.title.clone(),
            it.year.map(|y| y.to_string()).unwrap_or_default(),
        ),
        MediaKind::Video => (it.title.clone(), folder_name(&it.folder)),
    }
}

fn rule_should_record(pos: i64, dur: i64) -> bool {
    rules::Rules::default().should_record(pos.max(0) as u64, dur.max(0) as u64)
}

fn rule_is_watched(pos: i64, dur: i64) -> bool {
    dur > 0 && rules::Rules::default().is_watched(pos.max(0) as u64, dur as u64)
}

fn rule_resume(pos: i64, dur: i64) -> i64 {
    rules::Rules::default()
        .resume_point(pos.max(0) as u64, dur.max(0) as u64)
        .map(|v| v as i64)
        .unwrap_or(0)
}

fn rule_lesson_complete(ratio: f64) -> bool {
    rules::Rules::default().lesson_complete(ratio)
}

struct Session {
    last_report: i64,
    last_pos: i64,
    history_id: Option<i64>,
    counted: bool,
}

fn sessions() -> &'static Mutex<HashMap<String, Session>> {
    static S: OnceLock<Mutex<HashMap<String, Session>>> = OnceLock::new();
    S.get_or_init(|| Mutex::new(HashMap::new()))
}

const SESSION_GAP_MS: i64 = 30 * 60 * 1000;

fn labels_for_key(key: &str) -> (String, String, Option<SourceRef>) {
    if let Some(hash) = key.strip_prefix("local:") {
        if let Ok(Some(row)) = db::media_by_hash(hash) {
            let (t, s) = local_labels(&row);
            return (t, s, Some(SourceRef::Local { id: row.item.id }));
        }
    }
    if let Ok(Some(m)) = db::item_meta(key) {
        return (m.title, m.subtitle, m.source);
    }
    if let Some(url) = key.strip_prefix("url:") {
        return (
            url.to_string(),
            String::new(),
            Some(SourceRef::Url { url: url.into() }),
        );
    }
    (key.to_string(), String::new(), None)
}

fn is_lesson_key(key: &str) -> bool {
    key.strip_prefix("local:")
        .and_then(|h| db::media_by_hash(h).ok().flatten())
        .map(|r| r.item.kind == MediaKind::Lesson)
        .unwrap_or(false)
}

fn progress_apply(r: &ProgressReport) -> Result<Progress, String> {
    if r.key.is_empty() {
        return Err("missing key".into());
    }
    let now = db::now_ms();
    let existing = db::progress_get(&r.key)?;
    let (old, old_seen) = match &existing {
        Some((p, seen, _)) => (Some(p.clone()), seen.clone()),
        None => (None, None),
    };
    let dur = if r.duration_ms > 0 {
        r.duration_ms
    } else {
        old.as_ref().map(|p| p.duration_ms).unwrap_or(0)
    };
    let pos = if dur > 0 {
        r.position_ms.clamp(0, dur)
    } else {
        r.position_ms.max(0)
    };
    let record = rule_should_record(pos, dur);

    let mut sess = sessions().lock().map_err(|_| "progress lock poisoned")?;
    let fresh = sess
        .get(&r.key)
        .map(|s| now - s.last_report > SESSION_GAP_MS)
        .unwrap_or(true);
    if fresh {
        sess.insert(
            r.key.clone(),
            Session {
                last_report: now,
                last_pos: pos,
                history_id: None,
                counted: false,
            },
        );
    }
    let s = sess.get_mut(&r.key).ok_or("session missing")?;

    let dur_s = (dur / 1000).max(1) as u64;
    let mut bits = match &old_seen {
        Some(b) => rules::SeenBits::from_bytes(b.clone()),
        None => rules::SeenBits::default(),
    };
    let from = match r.watched_from_ms {
        Some(f) => Some(f),
        None if !fresh && pos > s.last_pos => {
            let elapsed = now - s.last_report;
            (pos - s.last_pos <= elapsed * 4 + 5_000).then_some(s.last_pos)
        }
        None => None,
    };
    if let Some(f) = from {
        if f < pos && pos - f <= 10 * 60 * 1000 {
            bits.mark((f / 1000).max(0) as u64, (pos / 1000) as u64);
        }
    }
    let ratio = if dur > 0 { bits.ratio(dur_s) } else { 0.0 };
    let reached = if is_lesson_key(&r.key) {
        rule_lesson_complete(ratio)
    } else {
        rule_is_watched(pos, dur)
    };
    let was_watched = old.as_ref().map(|p| p.watched).unwrap_or(false);
    let watched = was_watched || reached;
    let mut play_count = old.as_ref().map(|p| p.play_count).unwrap_or(0);
    if reached && !s.counted {
        play_count += 1;
        s.counted = true;
    }
    let last_played = if record {
        Some(now)
    } else {
        old.as_ref().and_then(|p| p.last_played)
    };
    s.last_report = now;
    s.last_pos = pos;

    if existing.is_some() || record {
        let seen = bits.as_bytes().to_vec();
        db::with(|c| {
            c.execute(
                "INSERT INTO progress (key, position_ms, duration_ms, watched, play_count,
                    last_played, seen, hidden)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 0)
                 ON CONFLICT(key) DO UPDATE SET position_ms = excluded.position_ms,
                    duration_ms = excluded.duration_ms, watched = excluded.watched,
                    play_count = excluded.play_count, last_played = excluded.last_played,
                    seen = excluded.seen,
                    hidden = CASE WHEN ?8 THEN 0 ELSE progress.hidden END",
                params![
                    r.key,
                    pos,
                    dur,
                    watched as i64,
                    play_count,
                    last_played,
                    seen,
                    record
                ],
            )
        })?;
    }

    if record {
        match s.history_id {
            Some(id) => {
                db::with(|c| {
                    c.execute(
                        "UPDATE history SET at = ?2, position_ms = ?3, duration_ms = ?4 WHERE id = ?1",
                        params![id, now, pos, dur],
                    )
                })?;
            }
            None => {
                let (title, subtitle, source) = labels_for_key(&r.key);
                if let Some(src) = source {
                    let sj = serde_json::to_string(&src).map_err(|e| e.to_string())?;
                    let id: i64 = db::with(|c| {
                        c.query_row(
                            "INSERT INTO history (key, title, subtitle, source_json, at,
                                position_ms, duration_ms)
                             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7) RETURNING id",
                            params![r.key, title, subtitle, sj, now, pos, dur],
                            |row| row.get(0),
                        )
                    })?;
                    s.history_id = Some(id);
                }
            }
        }
    }

    Ok(Progress {
        key: r.key.clone(),
        position_ms: pos,
        duration_ms: dur,
        watched,
        play_count,
        last_played,
        seen_ratio: ratio,
    })
}

pub fn progress_record(
    key: &str,
    position_ms: i64,
    duration_ms: i64,
    playing: bool,
) -> Result<Progress, String> {
    progress_apply(&ProgressReport {
        key: key.to_string(),
        position_ms,
        duration_ms,
        playing,
        watched_from_ms: None,
    })
}

pub fn remember_item(
    key: &str,
    title: &str,
    subtitle: &str,
    thumb: Option<&str>,
    source: &SourceRef,
) -> Result<(), String> {
    db::set_item_meta(key, title, subtitle, thumb, source)
}

pub fn resume_for(key: &str) -> Result<(i64, Option<Progress>), String> {
    let Some((p, ..)) = db::progress_get(key)? else {
        return Ok((0, None));
    };
    let start = if p.watched {
        0
    } else {
        rule_resume(p.position_ms, p.duration_ms)
    };
    Ok((start, Some(p)))
}

pub async fn resolve_play(
    app: &AppHandle,
    source: SourceRef,
    force: Option<PlayMode>,
) -> Result<PlaybackInfo, String> {
    server::ensure(app).await?;
    match source {
        SourceRef::Local { id } => play_local(id, force, None).await,
        SourceRef::Url { url } => link::resolve(app, &url).await,
        _ => Err("This source plays through its own connection".into()),
    }
}

fn overlay(base: &mut serde_json::Value, top: &serde_json::Value) {
    if let (Some(b), Some(t)) = (base.as_object_mut(), top.as_object()) {
        for (k, v) in t {
            if !v.is_null() {
                b.insert(k.clone(), v.clone());
            }
        }
    }
}

pub fn merged_prefs(scopes: &[String]) -> PlayPrefs {
    let mut v = serde_json::to_value(PlayPrefs::default()).unwrap_or_default();
    for sc in std::iter::once("global".to_string()).chain(scopes.iter().cloned()) {
        if let Ok(Some(p)) = db::pref_json(&sc) {
            overlay(&mut v, &p);
        }
    }
    serde_json::from_value(v).unwrap_or_default()
}

fn scopes_for(row: &MediaRow) -> Vec<String> {
    let mut s = Vec::new();
    if let Some(c) = row.item.course_id {
        s.push(format!("course:{c}"));
    } else if let Some(show) = row
        .item
        .show
        .as_ref()
        .filter(|_| row.item.kind == MediaKind::Episode)
    {
        s.push(format!("show:{show}"));
    }
    s.push(format!("media:{}", row.item.key));
    s
}

#[tauri::command]
pub async fn video_prefs_get(
    scopes: Vec<String>,
) -> Result<HashMap<String, serde_json::Value>, String> {
    blocking(move || {
        let mut out = HashMap::new();
        for sc in scopes {
            let v = if sc == "global" {
                serde_json::to_value(merged_prefs(&[])).unwrap_or_default()
            } else {
                db::pref_json(&sc)?.unwrap_or_else(|| serde_json::json!({}))
            };
            out.insert(sc, v);
        }
        Ok(out)
    })
    .await
}

#[tauri::command]
pub async fn video_prefs_set(scope: String, prefs: serde_json::Value) -> Result<(), String> {
    blocking(move || {
        let mut cur = db::pref_json(&scope)?.unwrap_or_else(|| serde_json::json!({}));
        if let (Some(c), Some(p)) = (cur.as_object_mut(), prefs.as_object()) {
            for (k, v) in p {
                if v.is_null() {
                    c.remove(k);
                } else {
                    c.insert(k.clone(), v.clone());
                }
            }
        }
        let s = serde_json::to_string(&cur).map_err(|e| e.to_string())?;
        db::with(|c| {
            c.execute(
                "INSERT INTO prefs (scope, json) VALUES (?1, ?2)
                 ON CONFLICT(scope) DO UPDATE SET json = excluded.json",
                params![scope, s],
            )
        })?;
        Ok(())
    })
    .await
}

#[derive(Serialize)]
pub struct VideoStatus {
    pub server: ServerInfo,
    pub roots: Vec<Root>,
    pub counts: HomeCounts,
    pub scanning: bool,
    pub ffmpeg: bool,
}

fn counts() -> Result<HomeCounts, String> {
    db::with(|c| {
        let n = |sql: &str| c.query_row(sql, [], |r| r.get::<_, i64>(0));
        Ok(HomeCounts {
            movies: n("SELECT COUNT(*) FROM media WHERE kind = 'movie'")?,
            shows: n("SELECT COUNT(DISTINCT show) FROM media WHERE kind = 'episode' AND show IS NOT NULL")?,
            courses: n("SELECT COUNT(*) FROM courses")?,
            videos: n("SELECT COUNT(*) FROM media")?,
        })
    })
}

#[tauri::command]
pub async fn video_status(app: AppHandle) -> Result<VideoStatus, String> {
    let server = server::ensure(&app).await?;
    let (roots, counts) = blocking(|| Ok((db::roots()?, counts()?))).await?;
    Ok(VideoStatus {
        server,
        roots,
        counts,
        scanning: scan::is_scanning(),
        ffmpeg: omniget_core::core::ffmpeg::is_ffmpeg_available().await,
    })
}

fn norm_root(path: &str) -> Result<String, String> {
    let p = PathBuf::from(path.trim());
    if !p.is_dir() {
        return Err("not a folder".into());
    }
    let s = p.to_string_lossy().to_string();
    let t = s.trim_end_matches(['/', '\\']);
    Ok(if t.is_empty() { s } else { t.to_string() })
}

fn add_root(path: &str, kind: RootKind) -> Result<String, String> {
    let norm = norm_root(path)?;
    db::with(|c| {
        c.execute(
            "INSERT INTO roots (path, kind, added_at) VALUES (?1, ?2, ?3)
             ON CONFLICT(path) DO UPDATE SET kind = excluded.kind",
            params![norm, kind.as_str(), db::now_ms()],
        )
    })?;
    if kind == RootKind::Course {
        scan::ensure_course(&norm)?;
    }
    Ok(norm)
}

fn forget_root(path: &str) -> Result<(), String> {
    db::with_mut(|c| {
        let tx = c.transaction()?;
        tx.execute("DELETE FROM roots WHERE path = ?1", [path])?;
        tx.execute("DELETE FROM media WHERE root = ?1", [path])?;
        tx.execute("DELETE FROM courses WHERE root_path = ?1", [path])?;
        tx.commit()
    })
}

#[tauri::command]
pub async fn video_roots_add(
    app: AppHandle,
    path: String,
    kind: RootKind,
) -> Result<Vec<Root>, String> {
    let norm = add_root(&path, kind)?;
    tauri::async_runtime::spawn(async move {
        let _ = scan::scan(app, Some(norm)).await;
    });
    db::roots()
}

#[tauri::command]
pub async fn video_roots_remove(app: AppHandle, path: String) -> Result<Vec<Root>, String> {
    blocking(move || forget_root(&path)).await?;
    let _ = app.emit("video-library-changed", ());
    db::roots()
}

#[tauri::command]
pub async fn video_scan(app: AppHandle) -> Result<(), String> {
    tauri::async_runtime::spawn(async move {
        if let Err(e) = scan::scan(app, None).await {
            tracing::warn!("[video] scan failed: {e}");
        }
    });
    Ok(())
}

fn escape_like(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_")
}

#[tauri::command]
pub async fn video_items(query: ItemsQuery) -> Result<Vec<MediaItem>, String> {
    blocking(move || {
        let mut clauses = Vec::new();
        let mut args: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();
        if let Some(k) = query.kind {
            args.push(Box::new(k.as_str().to_string()));
            clauses.push(format!("m.kind = ?{}", args.len()));
        }
        if let Some(r) = query.root.filter(|s| !s.is_empty()) {
            args.push(Box::new(r));
            clauses.push(format!("m.root = ?{}", args.len()));
        }
        if let Some(f) = query.folder.filter(|s| !s.is_empty()) {
            args.push(Box::new(f));
            clauses.push(format!("m.folder = ?{}", args.len()));
        }
        if let Some(q) = query.search.map(|s| s.trim().to_string()).filter(|s| !s.is_empty()) {
            args.push(Box::new(format!("%{}%", escape_like(&q))));
            let n = args.len();
            clauses.push(format!(
                "(m.title LIKE ?{n} ESCAPE '\\' OR m.rel LIKE ?{n} ESCAPE '\\' OR m.show LIKE ?{n} ESCAPE '\\')"
            ));
        }
        let clause = if clauses.is_empty() {
            String::new()
        } else {
            format!("WHERE {}", clauses.join(" AND "))
        };
        let refs: Vec<&dyn rusqlite::ToSql> = args.iter().map(|b| b.as_ref()).collect();
        let mut rows = db::media_where(&clause, &refs)?;
        match query.kind {
            Some(MediaKind::Movie) => rows.sort_by(|a, b| {
                natural(&a.item.title, &b.item.title).then(a.item.year.cmp(&b.item.year))
            }),
            Some(MediaKind::Episode) => {
                rows.sort_by(|a, b| natural(a.item.show.as_deref().unwrap_or(""), b.item.show.as_deref().unwrap_or("")));
                let mut grouped: Vec<MediaRow> = Vec::with_capacity(rows.len());
                let mut i = 0;
                while i < rows.len() {
                    let show = rows[i].item.show.clone();
                    let mut j = i;
                    while j < rows.len() && rows[j].item.show == show {
                        j += 1;
                    }
                    let mut chunk = rows[i..j].to_vec();
                    sort_episodes(&mut chunk);
                    grouped.extend(chunk);
                    i = j;
                }
                rows = grouped;
            }
            _ => sort_natural(&mut rows),
        }
        let offset = query.offset.unwrap_or(0).max(0) as usize;
        let limit = query.limit.filter(|l| *l > 0).map(|l| l as usize).unwrap_or(usize::MAX);
        Ok(rows
            .into_iter()
            .skip(offset)
            .take(limit)
            .map(|r| r.item)
            .collect())
    })
    .await
}

#[tauri::command]
pub async fn video_item(id: i64) -> Result<MediaItem, String> {
    blocking(move || db::media_by_id(id).map(|r| r.item)).await
}

fn next_up(ordered: &[MediaRow]) -> Option<&MediaRow> {
    let last = ordered
        .iter()
        .enumerate()
        .filter_map(|(i, r)| {
            r.item
                .progress
                .as_ref()
                .and_then(|p| p.last_played)
                .map(|t| (i, t))
        })
        .max_by_key(|(_, t)| *t)
        .map(|(i, _)| i);
    let unwatched = |r: &&MediaRow| !r.item.progress.as_ref().map(|p| p.watched).unwrap_or(false);
    match last {
        Some(i) if !unwatched(&&ordered[i]) => ordered[i + 1..]
            .iter()
            .find(unwatched)
            .or_else(|| ordered[..i].iter().find(unwatched)),
        Some(i) => Some(&ordered[i]),
        None => ordered.iter().find(unwatched),
    }
}

#[tauri::command]
pub async fn video_shows() -> Result<Vec<ShowSummary>, String> {
    blocking(|| {
        let rows = db::media_where("WHERE m.kind = 'episode' AND m.show IS NOT NULL", &[])?;
        let mut by: HashMap<String, Vec<MediaRow>> = HashMap::new();
        for r in rows {
            by.entry(r.item.show.clone().unwrap_or_default())
                .or_default()
                .push(r);
        }
        let mut out: Vec<ShowSummary> = by
            .into_iter()
            .map(|(name, mut eps)| {
                sort_episodes(&mut eps);
                let seasons: std::collections::HashSet<i64> =
                    eps.iter().map(|e| e.item.season.unwrap_or(1)).collect();
                let watched = eps
                    .iter()
                    .filter(|e| e.item.progress.as_ref().map(|p| p.watched).unwrap_or(false))
                    .count() as i64;
                ShowSummary {
                    seasons: seasons.len() as i64,
                    episodes: eps.len() as i64,
                    watched,
                    thumb: eps.first().and_then(|e| e.item.thumb.clone()),
                    year: eps.iter().filter_map(|e| e.item.year).min(),
                    name,
                }
            })
            .collect();
        out.sort_by(|a, b| natural(&a.name, &b.name));
        Ok(out)
    })
    .await
}

#[tauri::command]
pub async fn video_show(name: String) -> Result<ShowDetail, String> {
    blocking(move || {
        let mut eps = db::media_where("WHERE m.kind = 'episode' AND m.show = ?1", &[&name])?;
        if eps.is_empty() {
            return Err("show not found".into());
        }
        sort_episodes(&mut eps);
        let next = next_up(&eps).map(|r| r.item.id);
        let mut seasons: Vec<ShowSeason> = Vec::new();
        for e in eps {
            let s = e.item.season.unwrap_or(1);
            match seasons.last_mut() {
                Some(last) if last.season == s => last.episodes.push(e.item),
                _ => seasons.push(ShowSeason {
                    season: s,
                    episodes: vec![e.item],
                }),
            }
        }
        Ok(ShowDetail {
            name,
            seasons,
            next,
        })
    })
    .await
}

fn course_rows(id: i64) -> Result<Vec<MediaRow>, String> {
    let mut rows = db::media_where("WHERE m.course_id = ?1", &[&id])?;
    sort_course(&mut rows);
    Ok(rows)
}

fn course_summary(id: i64, title: String, root: String) -> Result<CourseSummary, String> {
    let rows = course_rows(id)?;
    let mut s = CourseSummary {
        id,
        title,
        root_path: root,
        lessons: rows.len() as i64,
        watched: 0,
        duration_ms: 0,
        watched_ms: 0,
        last_played: None,
        next_lesson: None,
        thumb: None,
    };
    for r in &rows {
        s.duration_ms += r.item.duration_ms;
        if let Some(p) = &r.item.progress {
            if p.watched {
                s.watched += 1;
                s.watched_ms += r.item.duration_ms;
            } else {
                s.watched_ms += (p.seen_ratio * r.item.duration_ms as f64) as i64;
            }
            s.last_played = s.last_played.max(p.last_played);
        }
    }
    let next = next_up(&rows);
    s.next_lesson = next.map(|r| r.item.id);
    s.thumb = next.or(rows.first()).and_then(|r| r.item.thumb.clone());
    Ok(s)
}

fn all_courses() -> Result<Vec<CourseSummary>, String> {
    let list: Vec<(i64, String, String)> = db::with(|c| {
        let mut st = c.prepare("SELECT id, title, root_path FROM courses")?;
        let rows = st.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?;
        rows.collect()
    })?;
    let mut out = Vec::new();
    for (id, title, root) in list {
        out.push(course_summary(id, title, root)?);
    }
    out.sort_by(|a, b| {
        b.last_played
            .cmp(&a.last_played)
            .then_with(|| natural(&a.title, &b.title))
    });
    Ok(out)
}

#[tauri::command]
pub async fn video_courses() -> Result<Vec<CourseSummary>, String> {
    blocking(all_courses).await
}

fn course_head(id: i64) -> Result<(String, String, String), String> {
    db::with(|c| {
        c.query_row(
            "SELECT title, root_path, attachments_json FROM courses WHERE id = ?1",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()
    })?
    .ok_or_else(|| "course not found".into())
}

fn attachment(root: &str, rel: &str) -> Attachment {
    let path = Path::new(root).join(rel).to_string_lossy().to_string();
    Attachment {
        name: folder_name(&path),
        kind: scan::attachment_kind(&path),
        path,
    }
}

#[tauri::command]
pub async fn video_course(id: i64) -> Result<CourseDetail, String> {
    blocking(move || {
        let (title, root, atts_json) = course_head(id)?;
        let summary = course_summary(id, title, root.clone())?;
        let atts: Vec<(String, Option<String>)> =
            serde_json::from_str(&atts_json).unwrap_or_default();
        let mut by_lesson: HashMap<String, Vec<Attachment>> = HashMap::new();
        let mut loose = Vec::new();
        for (p, lesson) in atts {
            match lesson {
                Some(l) => by_lesson.entry(l).or_default().push(attachment(&root, &p)),
                None => loose.push(attachment(&root, &p)),
            }
        }
        let mut modules: Vec<CourseModule> = Vec::new();
        for (i, r) in course_rows(id)?.into_iter().enumerate() {
            let mi = r.module_index.unwrap_or(0);
            let number = r
                .number
                .clone()
                .filter(|n| !n.is_empty())
                .unwrap_or_else(|| (i + 1).to_string());
            let attachments = by_lesson.remove(&r.rel).unwrap_or_default();
            let watched = r.item.progress.as_ref().map(|p| p.watched).unwrap_or(false);
            let dur = r.item.duration_ms;
            let mtitle = r.item.module.clone().unwrap_or_default();
            let lesson = Lesson {
                item: r.item,
                number,
                attachments,
            };
            match modules.last_mut() {
                Some(m) if m.index == mi => {
                    m.duration_ms += dur;
                    m.watched += watched as i64;
                    m.lessons.push(lesson);
                }
                _ => modules.push(CourseModule {
                    index: mi,
                    title: mtitle,
                    lessons: vec![lesson],
                    duration_ms: dur,
                    watched: watched as i64,
                }),
            }
        }
        loose.extend(by_lesson.into_values().flatten());
        Ok(CourseDetail {
            course: summary,
            modules,
            loose_attachments: loose,
        })
    })
    .await
}

#[tauri::command]
pub async fn video_course_import(app: AppHandle, path: String) -> Result<CourseSummary, String> {
    let norm = add_root(&path, RootKind::Course)?;
    scan::scan(app, Some(norm.clone())).await?;
    blocking(move || {
        let id = scan::ensure_course(&norm)?;
        let (title, root, _) = course_head(id)?;
        course_summary(id, title, root)
    })
    .await
}

#[tauri::command]
pub async fn video_course_remove(app: AppHandle, id: i64) -> Result<(), String> {
    blocking(move || {
        let (_, root, _) = course_head(id)?;
        forget_root(&root)
    })
    .await?;
    let _ = app.emit("video-library-changed", ());
    Ok(())
}

fn continue_entries() -> Result<Vec<ContinueEntry>, String> {
    let mut out: Vec<(i64, ContinueEntry)> = Vec::new();
    let mut groups_with_resume = std::collections::HashSet::new();
    type ResumeRow = (String, i64, i64, i64, Option<i64>, Option<Vec<u8>>);
    let rows: Vec<ResumeRow> = db::with(|c| {
        let mut st = c.prepare(
            "SELECT key, position_ms, duration_ms, play_count, last_played, seen FROM progress
             WHERE watched = 0 AND hidden = 0 AND position_ms > 0 AND last_played IS NOT NULL
             ORDER BY last_played DESC LIMIT 200",
        )?;
        let rows = st.query_map([], |r| {
            Ok((
                r.get(0)?,
                r.get(1)?,
                r.get(2)?,
                r.get(3)?,
                r.get(4)?,
                r.get(5)?,
            ))
        })?;
        rows.collect()
    })?;
    for (key, pos, dur, plays, last, seen) in rows {
        if rule_resume(pos, dur) == 0 {
            continue;
        }
        let progress = Progress {
            key: key.clone(),
            position_ms: pos,
            duration_ms: dur,
            watched: false,
            play_count: plays,
            last_played: last,
            seen_ratio: db::seen_ratio(seen, dur),
        };
        let entry = if let Some(hash) = key.strip_prefix("local:") {
            let Some(row) = db::media_by_hash(hash)? else {
                continue;
            };
            if let Some(c) = row.item.course_id {
                groups_with_resume.insert(format!("course:{c}"));
            } else if let Some(s) = &row.item.show {
                groups_with_resume.insert(format!("show:{s}"));
            }
            let (title, subtitle) = local_labels(&row);
            ContinueEntry {
                key,
                title,
                subtitle,
                thumb: Some(server::url(&format!("/thumb/{}", row.item.id))),
                progress,
                kind: "resume".into(),
                source: SourceRef::Local { id: row.item.id },
            }
        } else {
            let Some(meta) = db::item_meta(&key)? else {
                continue;
            };
            let Some(source) = meta.source else {
                continue;
            };
            ContinueEntry {
                key,
                title: meta.title,
                subtitle: meta.subtitle,
                thumb: meta.thumb,
                progress,
                kind: "resume".into(),
                source,
            }
        };
        out.push((last.unwrap_or(0), entry));
    }
    let hidden: std::collections::HashSet<String> = db::with(|c| {
        let mut st = c.prepare("SELECT key FROM progress WHERE hidden = 1")?;
        let rows = st.query_map([], |r| r.get(0))?;
        rows.collect()
    })?;
    let mut groups: Vec<(String, Vec<MediaRow>)> = Vec::new();
    let course_ids: Vec<i64> = db::with(|c| {
        let mut st = c.prepare("SELECT id FROM courses")?;
        let rows = st.query_map([], |r| r.get(0))?;
        rows.collect()
    })?;
    for id in course_ids {
        groups.push((format!("course:{id}"), course_rows(id)?));
    }
    let shows: Vec<String> = db::with(|c| {
        let mut st = c.prepare(
            "SELECT DISTINCT m.show FROM media m JOIN progress p ON p.key = 'local:' || m.hash
             WHERE m.kind = 'episode' AND m.show IS NOT NULL AND p.last_played IS NOT NULL",
        )?;
        let rows = st.query_map([], |r| r.get(0))?;
        rows.collect()
    })?;
    for s in shows {
        let mut eps = db::media_where("WHERE m.kind = 'episode' AND m.show = ?1", &[&s])?;
        sort_episodes(&mut eps);
        groups.push((format!("show:{s}"), eps));
    }
    for (gkey, rows) in groups {
        if groups_with_resume.contains(&gkey) {
            continue;
        }
        let last = rows
            .iter()
            .filter_map(|r| r.item.progress.as_ref().and_then(|p| p.last_played))
            .max();
        let Some(last) = last else { continue };
        let Some(next) = next_up(&rows) else { continue };
        if hidden.contains(&next.item.key) || next.item.offline {
            continue;
        }
        if next
            .item
            .progress
            .as_ref()
            .map(|p| p.position_ms > 0 && !p.watched)
            .unwrap_or(false)
        {
            continue;
        }
        let (title, subtitle) = local_labels(next);
        out.push((
            last,
            ContinueEntry {
                key: next.item.key.clone(),
                title,
                subtitle,
                thumb: Some(server::url(&format!("/thumb/{}", next.item.id))),
                progress: next.item.progress.clone().unwrap_or(Progress {
                    key: next.item.key.clone(),
                    duration_ms: next.item.duration_ms,
                    ..Default::default()
                }),
                kind: "next".into(),
                source: SourceRef::Local { id: next.item.id },
            },
        ));
    }
    out.sort_by_key(|e| std::cmp::Reverse(e.0));
    Ok(out.into_iter().take(20).map(|(_, e)| e).collect())
}

#[tauri::command]
pub async fn video_home(app: AppHandle) -> Result<Home, String> {
    server::ensure(&app).await?;
    blocking(|| {
        let mut recent = db::media_where(
            "WHERE m.kind != 'lesson' AND m.offline = 0 ORDER BY m.added_at DESC LIMIT 24",
            &[],
        )?;
        recent.sort_by_key(|r| std::cmp::Reverse(r.item.added_at));
        let courses = all_courses()?
            .into_iter()
            .filter(|c| c.last_played.is_some() && c.watched < c.lessons)
            .take(12)
            .collect();
        Ok(Home {
            continue_: continue_entries()?,
            recent: recent.into_iter().map(|r| r.item).collect(),
            courses,
            counts: counts()?,
        })
    })
    .await
}

#[tauri::command]
pub async fn video_progress(report: ProgressReport) -> Result<Progress, String> {
    blocking(move || progress_apply(&report)).await
}

#[tauri::command]
pub async fn video_set_watched(keys: Vec<String>, watched: bool) -> Result<(), String> {
    blocking(move || {
        let now = db::now_ms();
        for key in keys {
            let dur = key
                .strip_prefix("local:")
                .and_then(|h| db::media_by_hash(h).ok().flatten())
                .map(|r| r.item.duration_ms)
                .unwrap_or(0);
            db::with(|c| {
                if watched {
                    c.execute(
                        "INSERT INTO progress (key, position_ms, duration_ms, watched, play_count,
                            last_played, hidden)
                         VALUES (?1, 0, ?2, 1, 1, ?3, 0)
                         ON CONFLICT(key) DO UPDATE SET watched = 1, position_ms = 0,
                            play_count = MAX(progress.play_count, 1),
                            last_played = COALESCE(progress.last_played, excluded.last_played)",
                        params![key, dur, now],
                    )
                } else {
                    c.execute(
                        "UPDATE progress SET watched = 0, position_ms = 0, seen = NULL WHERE key = ?1",
                        [&key],
                    )
                }
            })?;
            if let Ok(mut s) = sessions().lock() {
                s.remove(&key);
            }
        }
        Ok(())
    })
    .await
}

#[tauri::command]
pub async fn video_continue_remove(key: String) -> Result<(), String> {
    blocking(move || {
        db::with(|c| {
            c.execute(
                "INSERT INTO progress (key, hidden) VALUES (?1, 1)
                 ON CONFLICT(key) DO UPDATE SET hidden = 1",
                [&key],
            )
        })?;
        Ok(())
    })
    .await
}

#[tauri::command]
pub async fn video_history(limit: Option<i64>) -> Result<Vec<HistoryEntry>, String> {
    let limit = limit.unwrap_or(200).clamp(1, 5000);
    blocking(move || {
        db::with(|c| {
            let mut st = c.prepare(
                "SELECT key, title, subtitle, source_json, at, position_ms, duration_ms
                 FROM history ORDER BY at DESC LIMIT ?1",
            )?;
            let rows = st.query_map([limit], |r| {
                let sj: String = r.get(3)?;
                Ok((
                    serde_json::from_str::<SourceRef>(&sj).ok(),
                    HistoryEntry {
                        key: r.get(0)?,
                        title: r.get(1)?,
                        subtitle: r.get(2)?,
                        at: r.get(4)?,
                        position_ms: r.get(5)?,
                        duration_ms: r.get(6)?,
                        source: SourceRef::Url { url: String::new() },
                    },
                ))
            })?;
            Ok(rows
                .filter_map(|r| r.ok())
                .filter_map(|(src, mut e)| {
                    e.source = src?;
                    Some(e)
                })
                .collect())
        })
    })
    .await
}

#[tauri::command]
pub async fn video_history_clear() -> Result<(), String> {
    blocking(|| {
        db::with(|c| c.execute("DELETE FROM history", []))?;
        if let Ok(mut s) = sessions().lock() {
            for v in s.values_mut() {
                v.history_id = None;
            }
        }
        Ok(())
    })
    .await
}

fn probe_of(id: i64) -> Result<Option<probe::ProbeData>, String> {
    let raw: Option<String> = db::with(|c| {
        c.query_row(
            "SELECT probe_json FROM media WHERE id = ?1 AND probed = 1",
            [id],
            |r| r.get(0),
        )
        .optional()
    })?;
    Ok(raw.and_then(|s| serde_json::from_str(&s).ok()))
}

async fn ensure_probe(row: &MediaRow) -> Result<probe::ProbeData, String> {
    let id = row.item.id;
    if let Some(d) = blocking(move || probe_of(id)).await? {
        return Ok(d);
    }
    let d = probe::probe(Path::new(&row.item.path)).await?;
    let pj = serde_json::to_string(&d).unwrap_or_default();
    let streams = serde_json::to_string(&d.streams).unwrap_or_else(|_| "[]".into());
    let chapters = serde_json::to_string(&d.chapters).unwrap_or_else(|_| "[]".into());
    let dd = d.clone();
    blocking(move || {
        db::with(|c| {
            c.execute(
                "UPDATE media SET probe_json = ?2, probed = 1, streams_json = ?3,
                    chapters_json = ?4, duration_ms = ?5, width = ?6, height = ?7,
                    vcodec = ?8, acodec = ?9, container = ?10, bitrate = ?11 WHERE id = ?1",
                params![
                    id,
                    pj,
                    streams,
                    chapters,
                    dd.duration_ms,
                    dd.width,
                    dd.height,
                    dd.vcodec,
                    dd.acodec,
                    dd.format,
                    dd.bitrate
                ],
            )
        })
    })
    .await?;
    Ok(d)
}

fn siblings(row: &MediaRow) -> Result<Vec<i64>, String> {
    let rows = if let Some(c) = row.item.course_id {
        course_rows(c)?
    } else if let (MediaKind::Episode, Some(show)) = (row.item.kind, row.item.show.as_ref()) {
        let mut v = db::media_where("WHERE m.kind = 'episode' AND m.show = ?1", &[show])?;
        sort_episodes(&mut v);
        v
    } else {
        let mut v = db::media_where("WHERE m.folder = ?1", &[&row.item.folder])?;
        sort_natural(&mut v);
        v
    };
    Ok(rows.into_iter().map(|r| r.item.id).collect())
}

fn mode_path(mode: PlayMode, id: i64, ss_ms: i64, ai: usize) -> String {
    let route = match mode {
        PlayMode::Direct => return format!("/f/{id}"),
        PlayMode::Remux => "m",
        PlayMode::TranscodeAudio => "a",
        _ => "t",
    };
    format!(
        "/{route}/{id}/index.m3u8?ss={:.3}&ai={ai}",
        ss_ms as f64 / 1000.0
    )
}

pub async fn play_local(
    id: i64,
    force: Option<PlayMode>,
    reason: Option<String>,
) -> Result<PlaybackInfo, String> {
    let row = blocking(move || db::media_by_id(id)).await?;
    let path = PathBuf::from(&row.item.path);
    if row.item.offline || !path.exists() {
        return Err("This video is offline. Reconnect its drive or folder and scan again.".into());
    }
    let (d, probed) = match ensure_probe(&row).await {
        Ok(d) => (d, true),
        Err(e) => {
            tracing::warn!("[video] probe failed for {}: {e}", row.item.path);
            (probe::ProbeData::default(), false)
        }
    };
    let r2 = row.clone();
    let (prefs, (start_ms, _), sibs) = blocking(move || {
        Ok((
            merged_prefs(&scopes_for(&r2)),
            resume_for(&r2.item.key)?,
            siblings(&r2)?,
        ))
    })
    .await?;

    let audios: Vec<&StreamInfo> = d.audio_streams().collect();
    let audio_tracks: Vec<AudioTrack> = audios
        .iter()
        .enumerate()
        .map(|(i, a)| {
            let lang = probe::lang_label(&a.lang);
            let mut label = if !a.title.is_empty() {
                a.title.clone()
            } else if !lang.is_empty() {
                lang.clone()
            } else {
                format!("Track {}", i + 1)
            };
            if let Some(ch) = a.channels.filter(|c| *c > 2) {
                label = format!("{label} ({}.1)", ch - 1);
            }
            AudioTrack {
                index: i as i64,
                label,
                lang: a.lang.clone(),
                codec: a.codec.clone(),
                default: a.default,
            }
        })
        .collect();
    let default_ai = audios.iter().position(|a| a.default).unwrap_or(0);
    let ai = prefs
        .audio_index
        .filter(|i| *i >= 0 && (*i as usize) < audios.len())
        .map(|i| i as usize)
        .unwrap_or(default_ai);

    let (guess, why) = if probed {
        probe::decide(&path, &d, ai)
    } else {
        (
            PlayMode::Direct,
            "Couldn't inspect the file, trying to play it directly".to_string(),
        )
    };
    let mode = force.unwrap_or(guess);
    let reason = match (reason, force) {
        (Some(r), _) => r,
        (None, Some(f)) if f != guess => match f {
            PlayMode::Transcode => "Converting to H.264 for this device".to_string(),
            PlayMode::Remux => "Repackaged to MP4 without re-encoding".to_string(),
            PlayMode::TranscodeAudio => "Converting the audio to AAC".to_string(),
            _ => why,
        },
        _ => why,
    };
    let seek_reloads = mode != PlayMode::Direct;
    let offset_ms = if seek_reloads { start_ms } else { 0 };
    let url = server::url(&mode_path(mode, id, offset_ms, ai));

    let shift = if offset_ms > 0 {
        format!("&o={}", -offset_ms)
    } else {
        String::new()
    };
    let mut subtitles = Vec::new();
    for (i, (p, lang, forced)) in probe::external_subs(&path).into_iter().enumerate() {
        let name = folder_name(&p.to_string_lossy());
        let label = match probe::lang_label(&lang) {
            l if l.is_empty() => name.clone(),
            l => l,
        };
        subtitles.push(SubtitleTrack {
            id: format!("x{i}"),
            label,
            lang,
            url: server::url(&format!(
                "/x/{id}.vtt?p={}{shift}",
                urlencoding::encode(&p.to_string_lossy())
            )),
            default: false,
            forced,
            source: "external".into(),
        });
    }
    for s in d.text_subs() {
        let lang = probe::lang_label(&s.lang);
        let label = match (s.title.is_empty(), lang.is_empty()) {
            (false, true) => s.title.clone(),
            (false, false) => format!("{lang} · {}", s.title),
            (true, false) => lang,
            (true, true) => format!("Track {}", s.index),
        };
        subtitles.push(SubtitleTrack {
            id: format!("s{}", s.index),
            label,
            lang: s.lang.clone(),
            url: server::url(&format!(
                "/s/{id}/{}.vtt{}",
                s.index,
                shift.replacen('&', "?", 1)
            )),
            default: false,
            forced: s.forced,
            source: "embedded".into(),
        });
    }
    let off = prefs.subtitle_id.as_deref() == Some("off");
    let pick = prefs
        .subtitle_id
        .as_ref()
        .filter(|_| !off)
        .and_then(|sid| subtitles.iter().position(|t| &t.id == sid))
        .or_else(|| {
            if off {
                return None;
            }
            let want = prefs.subtitle_lang.as_ref()?.to_ascii_lowercase();
            subtitles.iter().position(|t| {
                let l = t.lang.to_ascii_lowercase();
                !l.is_empty() && (l == want || l.starts_with(&want) || want.starts_with(&l))
            })
        })
        .or_else(|| {
            (prefs.subtitle_id.is_none() && prefs.subtitle_lang.is_none())
                .then(|| subtitles.iter().position(|t| t.forced))
                .flatten()
        });
    if let Some(i) = pick {
        subtitles[i].default = true;
    }

    let pos = sibs.iter().position(|x| *x == id);
    let prev = pos
        .and_then(|i| i.checked_sub(1))
        .and_then(|i| sibs.get(i))
        .map(|id| SourceRef::Local { id: *id });
    let next = pos
        .and_then(|i| sibs.get(i + 1))
        .map(|id| SourceRef::Local { id: *id });
    let (title, subtitle) = local_labels(&row);
    Ok(PlaybackInfo {
        key: row.item.key.clone(),
        title,
        subtitle,
        source: SourceRef::Local { id },
        url,
        mode,
        seek_reloads,
        offset_ms,
        reason,
        duration_ms: d.duration_ms.max(row.item.duration_ms),
        start_ms,
        is_live: false,
        subtitles,
        audio_tracks,
        chapters: d.chapters.clone(),
        info: PlaybackMediaInfo {
            container: probe::ext_of(&path),
            vcodec: d.vcodec.clone(),
            acodec: audios
                .get(ai)
                .map(|a| a.codec.clone())
                .unwrap_or_else(|| d.acodec.clone()),
            width: d.width,
            height: d.height,
            bitrate: d.bitrate,
        },
        prefs,
        next,
        prev,
        course_id: row.item.course_id,
    })
}

#[tauri::command]
pub async fn video_play(app: AppHandle, source: SourceRef) -> Result<PlaybackInfo, String> {
    server::ensure(&app).await?;
    sources::resolve(&app, source, false).await
}

#[tauri::command]
pub async fn video_source_for_key(key: String) -> Result<SourceRef, String> {
    blocking(move || {
        if let Some(hash) = key.strip_prefix("local:") {
            if let Some(row) = db::media_by_hash(hash)? {
                return Ok(SourceRef::Local { id: row.item.id });
            }
        }
        if let Some(url) = key.strip_prefix("url:") {
            return Ok(SourceRef::Url {
                url: url.to_string(),
            });
        }
        db::item_meta(&key)?
            .and_then(|m| m.source)
            .ok_or_else(|| "This video is no longer in the library".to_string())
    })
    .await
}

#[tauri::command]
pub async fn video_open_path(app: AppHandle, path: String) -> Result<PlaybackInfo, String> {
    server::ensure(&app).await?;
    let p = PathBuf::from(&path);
    if !p.is_file() {
        return Err("File not found".into());
    }
    let id = scan::index_loose(&p).await?;
    play_local(id, None, None).await
}

#[tauri::command]
pub async fn video_play_fallback(
    app: AppHandle,
    source: SourceRef,
    failed_mode: PlayMode,
) -> Result<PlaybackInfo, String> {
    server::ensure(&app).await?;
    match source {
        SourceRef::Local { id } => {
            let next = probe::next_mode(failed_mode)
                .ok_or("This file can't play here. Try opening it in an external player.")?;
            play_local(
                id,
                Some(next),
                Some(probe::fallback_reason(failed_mode, next)),
            )
            .await
        }
        SourceRef::Url { url } => link::resolve_fallback(&app, &url, failed_mode).await,
        other => sources::resolve(&app, other, false).await,
    }
}

fn find_in_path(names: &[&str]) -> Option<PathBuf> {
    names.iter().find_map(|n| which::which(n).ok())
}

fn find_player(want: Option<&str>) -> Option<(String, PathBuf)> {
    let order: Vec<&str> = match want {
        Some(w) => vec![w],
        None => vec!["iina", "mpv", "vlc"],
    };
    for p in order {
        let found = match p {
            "iina" if cfg!(target_os = "macos") => {
                let cli = PathBuf::from("/Applications/IINA.app/Contents/MacOS/iina-cli");
                cli.exists()
                    .then_some(cli)
                    .or_else(|| find_in_path(&["iina-cli", "iina"]))
            }
            "mpv" => find_in_path(&["mpv"]).or_else(|| {
                let mac = PathBuf::from("/Applications/mpv.app/Contents/MacOS/mpv");
                (cfg!(target_os = "macos") && mac.exists()).then_some(mac)
            }),
            "vlc" => find_in_path(&["vlc"]).or_else(|| {
                [
                    "/Applications/VLC.app/Contents/MacOS/VLC",
                    r"C:\Program Files\VideoLAN\VLC\vlc.exe",
                    r"C:\Program Files (x86)\VideoLAN\VLC\vlc.exe",
                ]
                .iter()
                .map(PathBuf::from)
                .find(|p| p.exists())
            }),
            _ => None,
        };
        if let Some(bin) = found {
            return Some((p.to_string(), bin));
        }
    }
    None
}

#[tauri::command]
pub async fn video_open_external(source: SourceRef, player: Option<String>) -> Result<(), String> {
    let (target, key) = match &source {
        SourceRef::Local { id } => {
            let id = *id;
            let row = blocking(move || db::media_by_id(id)).await?;
            (row.item.path.clone(), row.item.key.clone())
        }
        SourceRef::Url { url } => (url.clone(), format!("url:{url}")),
        _ => return Err("Open this source from its own page".into()),
    };
    let start_s = blocking(move || resume_for(&key)).await?.0 / 1000;
    let (name, bin) = find_player(player.as_deref().filter(|p| !p.is_empty()))
        .ok_or("No external player found. Install mpv, IINA or VLC.")?;
    let mut cmd = std::process::Command::new(&bin);
    if start_s > 0 {
        match name.as_str() {
            "iina" => cmd.arg(format!("--mpv-start={start_s}")),
            "mpv" => cmd.arg(format!("--start={start_s}")),
            _ => cmd.arg(format!("--start-time={start_s}")),
        };
    }
    cmd.arg(&target)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|e| format!("Could not open {name}: {e}"))?;
    Ok(())
}

#[tauri::command]
pub async fn video_reveal(media_id: i64) -> Result<(), String> {
    let row = blocking(move || db::media_by_id(media_id)).await?;
    crate::commands::downloads::reveal_file(row.item.path).await
}

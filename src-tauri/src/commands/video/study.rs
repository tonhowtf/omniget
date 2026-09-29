//! Study tools: transcripts (from subtitles or the AI provider) with
//! full-text search, silence maps, questions to the AI, flashcards with a
//! small SM-2 and export to Anki through AnkiConnect.

use std::path::{Path, PathBuf};

use rusqlite::{params, OptionalExtension, Row};
use serde::Serialize;
use tauri::{AppHandle, Emitter};

use omniget_core::core::video::{notes_md, subs};

use super::{blocking, db, labels_for_key, link, notes, probe, server, Card, Cue, TranscriptHit};

// ---------- transcripts ----------

fn to_cues(v: Vec<subs::Cue>) -> Vec<Cue> {
    v.into_iter()
        .filter(|c| !c.text.trim().is_empty())
        .map(|c| Cue {
            start_ms: c.start_ms as i64,
            end_ms: c.end_ms as i64,
            text: c.text,
        })
        .collect()
}

/// The stored transcript of a key, if any.
pub fn cached_cues(key: &str) -> Result<Vec<Cue>, String> {
    let raw: Option<String> = db::with(|c| {
        c.query_row(
            "SELECT cues_json FROM transcripts WHERE key = ?1",
            [key],
            |r| r.get(0),
        )
        .optional()
    })?;
    Ok(raw
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default())
}

fn save_transcript(key: &str, lang: &str, cues: &[Cue], source: &str) -> Result<(), String> {
    let json = serde_json::to_string(cues).map_err(|e| e.to_string())?;
    db::with_mut(|c| {
        let tx = c.transaction()?;
        tx.execute(
            "INSERT INTO transcripts (key, lang, cues_json, source, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(key) DO UPDATE SET lang = excluded.lang, cues_json = excluded.cues_json,
                source = excluded.source, created_at = excluded.created_at",
            params![key, lang, json, source, db::now_ms()],
        )?;
        tx.execute("DELETE FROM transcripts_fts WHERE key = ?1", [key])?;
        {
            let mut st = tx
                .prepare("INSERT INTO transcripts_fts (key, start_ms, text) VALUES (?1, ?2, ?3)")?;
            for cue in cues {
                st.execute(params![key, cue.start_ms, cue.text])?;
            }
        }
        tx.commit()
    })
}

fn media_id_for(key: &str, media_id: Option<i64>) -> Option<i64> {
    media_id.or_else(|| {
        key.strip_prefix("local:")
            .and_then(|h| db::media_by_hash(h).ok().flatten())
            .map(|r| r.item.id)
    })
}

/// Cues from the subtitles the item already has: an external file first,
/// then an embedded text track, then the link's subtitles from yt-dlp.
async fn cues_from_subtitles(key: &str, media_id: Option<i64>) -> Option<(String, Vec<Cue>)> {
    if let Some(id) = media_id_for(key, media_id) {
        let (path, hash, ..) = db::media_file(id)?;
        let p = PathBuf::from(&path);
        if let Some((sub, lang, _)) = probe::external_subs(&p)
            .into_iter()
            .find(|(_, _, forced)| !forced)
        {
            if let Ok(bytes) = tokio::fs::read(&sub).await {
                let cues = to_cues(subs::parse_auto(&bytes));
                if !cues.is_empty() {
                    return Some((lang, cues));
                }
            }
        }
        let d = probe::probe(&p).await.ok()?;
        let s = d
            .text_subs()
            .find(|s| !s.forced && s.default)
            .or_else(|| d.text_subs().find(|s| !s.forced))?
            .clone();
        let vtt = server::embedded_vtt_file(&path, &hash, s.index).await?;
        let bytes = tokio::fs::read(&vtt).await.ok()?;
        let cues = to_cues(subs::parse_auto(&bytes));
        return (!cues.is_empty()).then_some((s.lang, cues));
    }
    for (lang, url) in link::subs_for_key(key) {
        let Ok(resp) = reqwest::get(&url).await else {
            continue;
        };
        let Ok(bytes) = resp.bytes().await else {
            continue;
        };
        let cues = to_cues(subs::parse_auto(&bytes));
        if !cues.is_empty() {
            return Some((lang, cues));
        }
    }
    None
}

async fn transcript_for(key: &str, media_id: Option<i64>) -> Result<Vec<Cue>, String> {
    let k = key.to_string();
    let stored = blocking(move || cached_cues(&k)).await?;
    if !stored.is_empty() {
        return Ok(stored);
    }
    let Some((lang, cues)) = cues_from_subtitles(key, media_id).await else {
        return Ok(Vec::new());
    };
    let k = key.to_string();
    let c2 = cues.clone();
    blocking(move || save_transcript(&k, &lang, &c2, "subtitle")).await?;
    Ok(cues)
}

#[tauri::command]
pub async fn video_transcript(key: String, media_id: Option<i64>) -> Result<Vec<Cue>, String> {
    transcript_for(&key, media_id).await
}

const CHUNK_S: u64 = 600;

#[derive(Serialize, Clone)]
struct TranscribeProgress {
    key: String,
    done: usize,
    total: usize,
}

/// Whisper-style request asking for segments. `None` when the provider
/// can't do `verbose_json`, so the caller falls back to plain text.
async fn transcribe_segments(chunk: &Path) -> Option<(String, Vec<Cue>)> {
    use omniget_core::core::ai::{self, AiProvider};
    let cfg = ai::get();
    let endpoint = match cfg.provider {
        AiProvider::Openai => "https://api.openai.com/v1/audio/transcriptions".to_string(),
        AiProvider::Local if !cfg.local_base_url.is_empty() => {
            format!("{}/audio/transcriptions", cfg.local_base_url)
        }
        _ => return None,
    };
    let bytes = tokio::fs::read(chunk).await.ok()?;
    let part = reqwest::multipart::Part::bytes(bytes)
        .file_name("audio.mp3")
        .mime_str("audio/mpeg")
        .ok()?;
    let form = reqwest::multipart::Form::new()
        .text("model", "whisper-1")
        .text("response_format", "verbose_json")
        .text("timestamp_granularities[]", "segment")
        .part("file", part);
    let client = reqwest::Client::builder()
        .use_rustls_tls()
        .timeout(std::time::Duration::from_secs(600))
        .build()
        .ok()?;
    let mut req = client.post(&endpoint).multipart(form);
    if !cfg.openai_key.is_empty() {
        req = req.bearer_auth(&cfg.openai_key);
    }
    let resp = req.send().await.ok()?;
    if !resp.status().is_success() {
        return None;
    }
    let json: serde_json::Value = resp.json().await.ok()?;
    let lang = json["language"].as_str().unwrap_or("").to_string();
    let segs = json["segments"].as_array()?;
    let cues: Vec<Cue> = segs
        .iter()
        .filter_map(|s| {
            let text = s["text"].as_str()?.trim().to_string();
            (!text.is_empty()).then(|| Cue {
                start_ms: (s["start"].as_f64().unwrap_or(0.0) * 1000.0) as i64,
                end_ms: (s["end"].as_f64().unwrap_or(0.0) * 1000.0) as i64,
                text,
            })
        })
        .collect();
    (!cues.is_empty()).then_some((lang, cues))
}

/// Extracts the audio (mono, 16 kHz MP3, 10-minute chunks), sends each chunk
/// to the configured AI provider and stores the result as the transcript.
#[tauri::command]
pub async fn video_transcribe(
    app: AppHandle,
    key: String,
    media_id: Option<i64>,
) -> Result<Vec<Cue>, String> {
    server::ensure(&app).await?;
    let k = key.clone();
    let input = blocking(move || Ok(notes::media_source(&k, media_id)))
        .await?
        .ok_or("Play the video first, then transcribe it")?;
    let work = db::cache_dir("transcribe")
        .ok_or("no cache folder")?
        .join(uuid::Uuid::new_v4().simple().to_string());
    std::fs::create_dir_all(&work).map_err(|e| e.to_string())?;
    let result = transcribe_in(&app, &key, &input, &work).await;
    let _ = std::fs::remove_dir_all(&work);
    result
}

async fn transcribe_in(
    app: &AppHandle,
    key: &str,
    input: &str,
    work: &Path,
) -> Result<Vec<Cue>, String> {
    let pattern = work.join("chunk%04d.mp3");
    let ok = omniget_core::core::process::command("ffmpeg")
        .args(["-hide_banner", "-nostdin", "-loglevel", "error", "-y", "-i"])
        .arg(input)
        .args([
            "-map",
            "0:a:0",
            "-vn",
            "-ac",
            "1",
            "-ar",
            "16000",
            "-c:a",
            "libmp3lame",
            "-b:a",
            "48k",
            "-f",
            "segment",
            "-segment_time",
            &CHUNK_S.to_string(),
            "-reset_timestamps",
            "1",
        ])
        .arg(&pattern)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true)
        .status()
        .await
        .map(|s| s.success())
        .unwrap_or(false);
    if !ok {
        return Err("Couldn't extract the audio".into());
    }
    let mut chunks: Vec<PathBuf> = std::fs::read_dir(work)
        .map_err(|e| e.to_string())?
        .flatten()
        .map(|e| e.path())
        .filter(|p| probe::ext_of(p) == "mp3")
        .collect();
    chunks.sort();
    if chunks.is_empty() {
        return Err("This video has no audio".into());
    }
    let total = chunks.len();
    let mut all = Vec::new();
    let mut lang = String::new();
    for (i, chunk) in chunks.iter().enumerate() {
        let offset = (i as u64 * CHUNK_S * 1000) as i64;
        match transcribe_segments(chunk).await {
            Some((l, cues)) => {
                if lang.is_empty() {
                    lang = l;
                }
                all.extend(cues.into_iter().map(|c| Cue {
                    start_ms: c.start_ms + offset,
                    end_ms: c.end_ms + offset,
                    text: c.text,
                }));
            }
            None => {
                // provider only returns text: one cue for the whole chunk
                let text = omniget_core::core::ai::transcribe(chunk).await?;
                let dur = omniget_core::core::ffmpeg::probe(chunk)
                    .await
                    .map(|p| (p.duration_seconds * 1000.0) as i64)
                    .unwrap_or((CHUNK_S * 1000) as i64);
                all.push(Cue {
                    start_ms: offset,
                    end_ms: offset + dur,
                    text,
                });
            }
        }
        let _ = app.emit(
            "video-transcribe-progress",
            TranscribeProgress {
                key: key.to_string(),
                done: i + 1,
                total,
            },
        );
    }
    let k = key.to_string();
    let c2 = all.clone();
    blocking(move || save_transcript(&k, &lang, &c2, "ai")).await?;
    Ok(all)
}

/// FTS5 query from free text: every word must appear, as a prefix.
fn fts_query(q: &str) -> String {
    q.split_whitespace()
        .map(|w| w.replace('"', ""))
        .filter(|w| !w.is_empty())
        .map(|w| format!("\"{w}\"*"))
        .collect::<Vec<_>>()
        .join(" ")
}

#[tauri::command]
pub async fn video_transcript_search(
    query: String,
    course_id: Option<i64>,
) -> Result<Vec<TranscriptHit>, String> {
    blocking(move || {
        let fq = fts_query(&query);
        if fq.is_empty() {
            return Ok(Vec::new());
        }
        let allowed: Option<std::collections::HashSet<String>> = match course_id {
            Some(c) => Some(db::with(|conn| {
                let mut st =
                    conn.prepare("SELECT 'local:' || hash FROM media WHERE course_id = ?1")?;
                let rows = st.query_map([c], |r| r.get(0))?;
                rows.collect()
            })?),
            None => None,
        };
        let hits: Vec<(String, i64, String)> = db::with(|c| {
            let mut st = c.prepare(
                "SELECT key, start_ms, text FROM transcripts_fts WHERE transcripts_fts MATCH ?1
                 ORDER BY rank LIMIT 500",
            )?;
            let rows = st.query_map([&fq], |r| Ok((r.get(0)?, r.get::<_, i64>(1)?, r.get(2)?)))?;
            rows.collect()
        })?;
        let mut titles: std::collections::HashMap<String, (String, Option<i64>)> =
            Default::default();
        let mut out = Vec::new();
        for (key, start_ms, text) in hits {
            if let Some(a) = &allowed {
                if !a.contains(&key) {
                    continue;
                }
            }
            let (title, media_id) = titles
                .entry(key.clone())
                .or_insert_with(|| {
                    let (t, _, src) = labels_for_key(&key);
                    let id = match src {
                        Some(super::SourceRef::Local { id }) => Some(id),
                        _ => None,
                    };
                    (t, id)
                })
                .clone();
            out.push(TranscriptHit {
                key,
                title,
                media_id,
                start_ms,
                text,
            });
            if out.len() >= 200 {
                break;
            }
        }
        Ok(out)
    })
    .await
}

// ---------- silences ----------

fn parse_silences(stderr: &str) -> Vec<(f64, f64)> {
    let mut out = Vec::new();
    let mut start: Option<f64> = None;
    for line in stderr.lines() {
        if let Some(i) = line.find("silence_start:") {
            start = line[i + 14..]
                .split_whitespace()
                .next()
                .and_then(|v| v.parse().ok());
        } else if let Some(i) = line.find("silence_end:") {
            let end: Option<f64> = line[i + 12..]
                .split_whitespace()
                .next()
                .and_then(|v| v.parse().ok());
            if let (Some(s), Some(e)) = (start.take(), end) {
                if e > s {
                    out.push((s.max(0.0), e));
                }
            }
        }
    }
    out
}

/// Silent stretches `[start_s, end_s]` in seconds of a local video (ffmpeg
/// `silencedetect`), cached per file and settings.
#[tauri::command]
pub async fn video_silences(
    media_id: i64,
    min_s: f64,
    noise_db: f64,
) -> Result<Vec<(f64, f64)>, String> {
    let min_s = if min_s.is_finite() {
        min_s.clamp(0.1, 10.0)
    } else {
        0.5
    };
    let noise = if noise_db.is_finite() {
        noise_db.clamp(-90.0, -5.0)
    } else {
        -35.0
    };
    let (path, hash, ..) = db::media_file(media_id).ok_or("video not found")?;
    let cache = db::cache_dir("silences").map(|d| {
        d.join(format!(
            "{hash}-{}-{}.json",
            (min_s * 1000.0) as i64,
            (noise * 10.0) as i64
        ))
    });
    if let Some(c) = &cache {
        if let Ok(b) = tokio::fs::read(c).await {
            if let Ok(v) = serde_json::from_slice(&b) {
                return Ok(v);
            }
        }
    }
    let out = omniget_core::core::process::command("ffmpeg")
        .args(["-hide_banner", "-nostdin", "-i"])
        .arg(&path)
        .args([
            "-map",
            "0:a:0",
            "-vn",
            "-sn",
            "-af",
            &format!("silencedetect=noise={noise}dB:d={min_s}"),
            "-f",
            "null",
            "-",
        ])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true)
        .output()
        .await
        .map_err(|e| format!("ffmpeg unavailable: {e}"))?;
    if !out.status.success() {
        return Err("Couldn't analyze the audio".into());
    }
    let list = parse_silences(&String::from_utf8_lossy(&out.stderr));
    if let Some(c) = cache {
        if let Ok(b) = serde_json::to_vec(&list) {
            let _ = tokio::fs::write(c, b).await;
        }
    }
    Ok(list)
}

// ---------- ask the AI ----------

const MAX_TRANSCRIPT_CHARS: usize = 90_000;

/// `[mm:ss] text` lines, merged into longer blocks until they fit.
fn timed_text(cues: &[Cue]) -> String {
    let mut block_ms = 0i64;
    loop {
        let mut out = String::new();
        let mut cur_start = -1i64;
        let mut cur = String::new();
        for c in cues {
            if cur_start < 0 || c.start_ms - cur_start >= block_ms {
                if !cur.is_empty() {
                    out.push_str(&format!(
                        "[{}] {}\n",
                        notes_md::fmt_ts(cur_start.max(0) as u64),
                        cur.trim()
                    ));
                }
                cur_start = c.start_ms;
                cur.clear();
            }
            cur.push(' ');
            cur.push_str(&c.text.replace('\n', " "));
        }
        if !cur.is_empty() {
            out.push_str(&format!(
                "[{}] {}\n",
                notes_md::fmt_ts(cur_start.max(0) as u64),
                cur.trim()
            ));
        }
        if out.len() <= MAX_TRANSCRIPT_CHARS || block_ms >= 600_000 {
            if out.len() > MAX_TRANSCRIPT_CHARS {
                let mut cut = MAX_TRANSCRIPT_CHARS;
                while !out.is_char_boundary(cut) {
                    cut -= 1;
                }
                out.truncate(cut);
            }
            return out;
        }
        block_ms = if block_ms == 0 { 15_000 } else { block_ms * 2 };
    }
}

#[tauri::command]
pub async fn video_ask(key: String, question: String, mode: String) -> Result<String, String> {
    let cues = transcript_for(&key, None).await?;
    if cues.is_empty() {
        return Err("No transcript yet. Transcribe the video first.".into());
    }
    let k = key.clone();
    let (title, ..) = blocking(move || Ok(labels_for_key(&k))).await?;
    let base = "You help someone study a video using its transcript. Every line starts \
        with its time as [mm:ss] or [h:mm:ss]. Cite the moments you rely on with the same \
        [mm:ss] format so they become clickable. Use only what the transcript says. Answer \
        in the language of the user's question when there is one, otherwise in the \
        language of the transcript. Be concise and use Markdown.";
    let task = match mode.as_str() {
        "summary" => "Write a structured summary: a two-sentence overview, then the key \
            points as a list, each with the time it starts."
            .to_string(),
        "quiz" => "Write a quiz of 5 to 8 questions that check understanding of the \
            important ideas. For each: the question, 3 or 4 options when it fits, the answer \
            and the [mm:ss] where it is explained."
            .to_string(),
        "chapters" => "Split the video into chapters. Output only lines in the form \
            `[mm:ss] Chapter title`, one per chapter, in order, starting at [00:00]. Titles \
            short (2 to 6 words)."
            .to_string(),
        _ => {
            if question.trim().is_empty() {
                return Err("Type a question".into());
            }
            format!("Question: {}", question.trim())
        }
    };
    let user = format!(
        "Video: {title}\n\nTranscript:\n{}\n\n{task}",
        timed_text(&cues)
    );
    omniget_core::core::ai::chat(base, &user).await
}

// ---------- flashcards ----------

const CARD_COLS: &str =
    "id, key, front, back, t_ms, image, due, interval_days, ease, reps, created_at";

fn card_from_row(r: &Row) -> rusqlite::Result<Card> {
    Ok(Card {
        id: r.get(0)?,
        key: r.get(1)?,
        front: r.get(2)?,
        back: r.get(3)?,
        t_ms: r.get(4)?,
        image: r.get(5)?,
        due: r.get(6)?,
        interval_days: r.get(7)?,
        ease: r.get(8)?,
        reps: r.get(9)?,
        created_at: r.get(10)?,
    })
}

fn card_by_id(id: i64) -> Result<Card, String> {
    db::with(|c| {
        c.query_row(
            &format!("SELECT {CARD_COLS} FROM cards WHERE id = ?1"),
            [id],
            card_from_row,
        )
        .optional()
    })?
    .ok_or_else(|| "card not found".into())
}

fn cards_where(clause: &str, args: &[&dyn rusqlite::ToSql]) -> Result<Vec<Card>, String> {
    db::with(|c| {
        let mut st = c.prepare(&format!("SELECT {CARD_COLS} FROM cards {clause}"))?;
        let rows = st.query_map(args, card_from_row)?;
        rows.collect()
    })
}

#[tauri::command]
pub async fn video_cards(key: Option<String>) -> Result<Vec<Card>, String> {
    blocking(move || match key.filter(|k| !k.is_empty()) {
        Some(k) => cards_where("WHERE key = ?1 ORDER BY t_ms, id", &[&k]),
        None => cards_where("ORDER BY created_at DESC", &[]),
    })
    .await
}

#[tauri::command]
pub async fn video_cards_due() -> Result<Vec<Card>, String> {
    blocking(|| {
        let now = db::now_ms();
        cards_where("WHERE due <= ?1 ORDER BY due LIMIT 500", &[&now])
    })
    .await
}

#[tauri::command]
pub async fn video_card_add(
    key: String,
    front: String,
    back: String,
    t_ms: Option<i64>,
    image: Option<String>,
) -> Result<Card, String> {
    if front.trim().is_empty() {
        return Err("The front can't be empty".into());
    }
    blocking(move || {
        let now = db::now_ms();
        let id: i64 = db::with(|c| {
            c.query_row(
                "INSERT INTO cards (key, front, back, t_ms, image, due, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6) RETURNING id",
                params![key, front.trim(), back.trim(), t_ms, image, now],
                |r| r.get(0),
            )
        })?;
        card_by_id(id)
    })
    .await
}

const DAY_MS: f64 = 86_400_000.0;

/// SM-2 with four buttons: 0 again, 1 hard, 2 good, 3 easy.
fn schedule(card: &Card, grade: i64) -> (f64, f64, i64, i64) {
    let mut ease = card.ease.max(1.3);
    let reps = card.reps;
    let (interval, new_reps) = match grade {
        0 => {
            ease = (ease - 0.2).max(1.3);
            // again: back in 10 minutes
            (10.0 / 1440.0, 0)
        }
        1 => {
            ease = (ease - 0.15).max(1.3);
            ((card.interval_days * 1.2).max(1.0), reps + 1)
        }
        2 => {
            let i = match reps {
                0 => 1.0,
                1 => 3.0,
                _ => (card.interval_days * ease).max(1.0),
            };
            (i, reps + 1)
        }
        _ => {
            ease += 0.15;
            let i = match reps {
                0 => 4.0,
                _ => (card.interval_days.max(1.0) * ease * 1.3).max(4.0),
            };
            (i, reps + 1)
        }
    };
    let interval = interval.min(3650.0);
    let due = db::now_ms() + (interval * DAY_MS) as i64;
    (interval, ease, new_reps, due)
}

#[tauri::command]
pub async fn video_card_review(id: i64, grade: i64) -> Result<Card, String> {
    blocking(move || {
        let card = card_by_id(id)?;
        let (interval, ease, reps, due) = schedule(&card, grade.clamp(0, 3));
        db::with(|c| {
            c.execute(
                "UPDATE cards SET interval_days = ?2, ease = ?3, reps = ?4, due = ?5,
                    lapses = lapses + ?6 WHERE id = ?1",
                params![id, interval, ease, reps, due, (grade <= 0) as i64],
            )
        })?;
        card_by_id(id)
    })
    .await
}

#[tauri::command]
pub async fn video_card_delete(id: i64) -> Result<(), String> {
    blocking(move || {
        db::with(|c| c.execute("DELETE FROM cards WHERE id = ?1", [id]))?;
        Ok(())
    })
    .await
}

// ---------- Anki ----------

const ANKI: &str = "http://127.0.0.1:8765";
const ANKI_DECK: &str = "OmniGet";

async fn anki(action: &str, params: serde_json::Value) -> Result<serde_json::Value, String> {
    let client = reqwest::Client::builder()
        .use_rustls_tls()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| e.to_string())?;
    let resp = client
        .post(ANKI)
        .json(&serde_json::json!({ "action": action, "version": 6, "params": params }))
        .send()
        .await
        .map_err(|_| "Anki isn't open. Start Anki with the AnkiConnect add-on.".to_string())?;
    let v: serde_json::Value = resp.json().await.map_err(|e| e.to_string())?;
    if let Some(err) = v["error"].as_str() {
        return Err(format!("Anki: {err}"));
    }
    Ok(v["result"].clone())
}

/// A two-field note type: "Basic" when it exists (English Anki), otherwise
/// the first model with exactly two fields.
async fn anki_model() -> Result<(String, String, String), String> {
    let names = anki("modelNames", serde_json::json!({})).await?;
    let names: Vec<String> = serde_json::from_value(names).unwrap_or_default();
    let mut ordered: Vec<&String> = names.iter().filter(|n| *n == "Basic").collect();
    ordered.extend(names.iter().filter(|n| *n != "Basic"));
    for name in ordered {
        let fields = anki("modelFieldNames", serde_json::json!({ "modelName": name })).await?;
        let fields: Vec<String> = serde_json::from_value(fields).unwrap_or_default();
        if fields.len() == 2 {
            return Ok((name.clone(), fields[0].clone(), fields[1].clone()));
        }
    }
    Err("Anki has no note type with two fields".into())
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('\n', "<br>")
}

#[derive(Serialize)]
pub struct AnkiResult {
    pub added: usize,
}

#[tauri::command]
pub async fn video_anki_export(ids: Vec<i64>) -> Result<AnkiResult, String> {
    let cards = blocking(move || {
        ids.into_iter()
            .map(card_by_id)
            .collect::<Result<Vec<_>, _>>()
    })
    .await?;
    if cards.is_empty() {
        return Ok(AnkiResult { added: 0 });
    }
    anki("createDeck", serde_json::json!({ "deck": ANKI_DECK })).await?;
    let (model, front_f, back_f) = anki_model().await?;
    let keys: Vec<String> = cards.iter().map(|c| c.key.clone()).collect();
    let titles = blocking(move || {
        Ok(keys
            .into_iter()
            .map(|k| labels_for_key(&k).0)
            .collect::<Vec<_>>())
    })
    .await?;
    let notes: Vec<serde_json::Value> = cards
        .iter()
        .zip(titles)
        .map(|(c, title)| {
            let mut back = html_escape(&c.back);
            // manual cards (key "manual") have no video to point back to
            let title = if c.key == "manual" { String::new() } else { title };
            let when = c
                .t_ms
                .filter(|_| c.key != "manual")
                .map(|t| format!(" · {}", notes_md::fmt_ts(t.max(0) as u64)))
                .unwrap_or_default();
            if !title.is_empty() || !when.is_empty() {
                back.push_str(&format!(
                    "<br><br><small>{}{}</small>",
                    html_escape(&title),
                    when
                ));
            }
            let mut fields = serde_json::Map::new();
            fields.insert(front_f.clone(), html_escape(&c.front).into());
            fields.insert(back_f.clone(), back.into());
            let mut note = serde_json::json!({
                "deckName": ANKI_DECK,
                "modelName": model,
                "fields": fields,
                "tags": ["omniget"],
                "options": { "allowDuplicate": false, "duplicateScope": "deck" },
            });
            if let Some(p) = c.image.as_deref().and_then(notes::img_path).filter(|p| p.exists()) {
                note["picture"] = serde_json::json!([{
                    "path": p.to_string_lossy(),
                    "filename": format!("omniget-{}", p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default()),
                    "fields": [back_f.clone()],
                }]);
            }
            note
        })
        .collect();
    // addNotes fails as a whole on one duplicate in recent AnkiConnect
    // versions, so notes go one by one
    let mut added = 0;
    for (card, note) in cards.iter().zip(notes) {
        if let Ok(v) = anki("addNote", serde_json::json!({ "note": note })).await {
            if let Some(nid) = v.as_i64() {
                added += 1;
                let id = card.id;
                let _ = blocking(move || {
                    db::with(|c| {
                        c.execute(
                            "UPDATE cards SET anki_id = ?2 WHERE id = ?1",
                            params![id, nid],
                        )
                    })
                })
                .await;
            }
        }
    }
    Ok(AnkiResult { added })
}

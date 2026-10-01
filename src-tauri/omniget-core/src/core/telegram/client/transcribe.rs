use std::path::{Path, PathBuf};

use grammers_client::grammers_tl_types as tl;
use serde::{Deserialize, Serialize};
use serde_json::json;

use super::auth::TelegramSessionHandle;
use super::events;
use crate::core::telegram::host;

const CHUNK_S: u64 = 600;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Segment {
    pub start_ms: i64,
    pub end_ms: i64,
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Transcript {
    pub text: String,
    pub segments: Vec<Segment>,
    pub lang: String,
    pub source: String,
    #[serde(default)]
    pub cached: bool,
}

fn cache_dir() -> PathBuf {
    let d = host::data_dir().join("transcripts");
    let _ = std::fs::create_dir_all(&d);
    d
}

fn cache_path(chat_id: i64, message_id: i32) -> PathBuf {
    cache_dir().join(format!("{}_{}.json", chat_id, message_id))
}

pub fn cached(chat_id: i64, message_id: i32) -> Option<Transcript> {
    let bytes = std::fs::read(cache_path(chat_id, message_id)).ok()?;
    let mut t: Transcript = serde_json::from_slice(&bytes).ok()?;
    t.cached = true;
    Some(t)
}

fn save(chat_id: i64, message_id: i32, t: &Transcript) {
    if let Ok(bytes) = serde_json::to_vec_pretty(t) {
        let _ = std::fs::write(cache_path(chat_id, message_id), bytes);
    }
}

fn progress(chat_id: i64, message_id: i32, stage: &str, done: usize, total: usize) {
    events::emit(
        "telegram-transcribe-progress",
        json!({
            "chat_id": chat_id,
            "message_id": message_id,
            "stage": stage,
            "done": done,
            "total": total,
        }),
    );
}

fn audio_endpoint() -> Option<(String, String)> {
    use crate::core::ai::{self, AiProvider};
    let cfg = ai::get();
    match cfg.provider {
        AiProvider::Openai => Some((
            "https://api.openai.com/v1/audio/transcriptions".to_string(),
            cfg.openai_key.clone(),
        )),
        AiProvider::Local if !cfg.local_base_url.is_empty() => Some((
            format!(
                "{}/audio/transcriptions",
                cfg.local_base_url.trim_end_matches('/')
            ),
            cfg.openai_key.clone(),
        )),
        _ => None,
    }
}

async fn chunk_segments(
    endpoint: &str,
    key: &str,
    chunk: &Path,
    lang: Option<&str>,
) -> Option<(String, Vec<Segment>)> {
    let bytes = tokio::fs::read(chunk).await.ok()?;
    let part = reqwest::multipart::Part::bytes(bytes)
        .file_name("audio.mp3")
        .mime_str("audio/mpeg")
        .ok()?;
    let mut form = reqwest::multipart::Form::new()
        .text("model", "whisper-1")
        .text("response_format", "verbose_json")
        .text("timestamp_granularities[]", "segment")
        .part("file", part);
    if let Some(l) = lang.filter(|l| !l.is_empty()) {
        form = form.text("language", l.to_string());
    }
    let client = reqwest::Client::builder()
        .use_rustls_tls()
        .timeout(std::time::Duration::from_secs(600))
        .build()
        .ok()?;
    let mut req = client.post(endpoint).multipart(form);
    if !key.is_empty() {
        req = req.bearer_auth(key);
    }
    let resp = req.send().await.ok()?;
    if !resp.status().is_success() {
        return None;
    }
    let v: serde_json::Value = resp.json().await.ok()?;
    let lang = v["language"].as_str().unwrap_or("").to_string();
    let segs: Vec<Segment> = v["segments"]
        .as_array()?
        .iter()
        .filter_map(|s| {
            let text = s["text"].as_str()?.trim().to_string();
            (!text.is_empty()).then(|| Segment {
                start_ms: (s["start"].as_f64().unwrap_or(0.0) * 1000.0) as i64,
                end_ms: (s["end"].as_f64().unwrap_or(0.0) * 1000.0) as i64,
                text,
            })
        })
        .collect();
    (!segs.is_empty()).then_some((lang, segs))
}

async fn input_for(
    handle: &TelegramSessionHandle,
    chat_id: i64,
    chat_type: &str,
    message_id: i32,
) -> Result<String, String> {
    let (_, media) = super::stream_server::media_for(handle, chat_id, chat_type, message_id)
        .await
        .map_err(|e| e.to_string())?;
    if !matches!(
        media.media_type.as_str(),
        "video" | "audio" | "voice" | "round_video" | "document"
    ) {
        return Err("This message has no audio".into());
    }
    let uid = super::index::file_uid(&media.media);
    if let Some(p) = super::dedupe::find(uid.as_deref(), media.file_size, &media.file_name) {
        return Ok(p.to_string_lossy().into_owned());
    }
    let (base, token) = super::stream_endpoint().await?;
    Ok(format!(
        "{}/stream/{}/{}/{}?token={}",
        base, chat_type, chat_id, message_id, token
    ))
}

async fn extract_chunks(input: &str, work: &Path) -> Result<Vec<PathBuf>, String> {
    let pattern = work.join("chunk%04d.mp3");
    let ok = crate::core::process::command("ffmpeg")
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
        .map_err(|e| format!("ffmpeg is not available: {}", e))?;
    if !ok {
        return Err("Couldn't extract the audio".into());
    }
    let mut chunks: Vec<PathBuf> = std::fs::read_dir(work)
        .map_err(|e| e.to_string())?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "mp3"))
        .collect();
    chunks.sort();
    if chunks.is_empty() {
        return Err("This file has no audio".into());
    }
    Ok(chunks)
}

async fn transcribe_ai(
    handle: &TelegramSessionHandle,
    chat_id: i64,
    chat_type: &str,
    message_id: i32,
    lang: Option<&str>,
    endpoint: &str,
    key: &str,
) -> Result<Transcript, String> {
    progress(chat_id, message_id, "audio", 0, 0);
    let input = input_for(handle, chat_id, chat_type, message_id).await?;
    let work = cache_dir().join(format!("tmp-{}", events::next_id()));
    std::fs::create_dir_all(&work).map_err(|e| e.to_string())?;
    let result = async {
        let chunks = extract_chunks(&input, &work).await?;
        let total = chunks.len();
        let mut segments = Vec::new();
        let mut detected = String::new();
        for (i, chunk) in chunks.iter().enumerate() {
            progress(chat_id, message_id, "chunk", i, total);
            let offset = (i as u64 * CHUNK_S * 1000) as i64;
            match chunk_segments(endpoint, key, chunk, lang).await {
                Some((l, segs)) => {
                    if detected.is_empty() {
                        detected = l;
                    }
                    segments.extend(segs.into_iter().map(|s| Segment {
                        start_ms: s.start_ms + offset,
                        end_ms: s.end_ms + offset,
                        text: s.text,
                    }));
                }
                None => {
                    let text = crate::core::ai::transcribe(chunk).await?;
                    let dur = crate::core::ffmpeg::probe(chunk)
                        .await
                        .map(|p| (p.duration_seconds * 1000.0) as i64)
                        .unwrap_or((CHUNK_S * 1000) as i64);
                    if !text.trim().is_empty() {
                        segments.push(Segment {
                            start_ms: offset,
                            end_ms: offset + dur,
                            text: text.trim().to_string(),
                        });
                    }
                }
            }
        }
        progress(chat_id, message_id, "done", total, total);
        let text = segments
            .iter()
            .map(|s| s.text.as_str())
            .collect::<Vec<_>>()
            .join(" ");
        Ok(Transcript {
            text,
            segments,
            lang: if detected.is_empty() {
                lang.unwrap_or("").to_string()
            } else {
                detected
            },
            source: "ai".into(),
            cached: false,
        })
    }
    .await;
    let _ = std::fs::remove_dir_all(&work);
    result
}

async fn transcribe_local(
    handle: &TelegramSessionHandle,
    chat_id: i64,
    chat_type: &str,
    message_id: i32,
    lang: Option<&str>,
    model: Option<&str>,
) -> Result<Transcript, String> {
    use crate::core::asr;
    if !asr::is_ready(model) {
        if !asr::status().runtime_installed && !asr::status().runtime_downloadable {
            return Err(
                "local transcription needs the ONNX Runtime, which has no build for this system"
                    .into(),
            );
        }
        progress(chat_id, message_id, "model", 0, 0);
        asr::install(model, &install_progress(model))
            .await
            .map_err(|e| format!("couldn't download the local Whisper model: {}", e))?;
    }
    progress(chat_id, message_id, "audio", 0, 0);
    let input = input_for(handle, chat_id, chat_type, message_id).await?;
    let duration = super::stream_server::media_for(handle, chat_id, chat_type, message_id)
        .await
        .ok()
        .and_then(|(_, m)| duration_ms(&m.media))
        .filter(|d| *d > 0)
        .map(|d| d as u64);
    let t = asr::transcribe_input(
        std::ffi::OsStr::new(&input),
        duration,
        lang,
        model,
        move |p| progress(chat_id, message_id, "local", (p * 100.0) as usize, 100),
    )
    .await
    .map_err(|e| e.to_string())?;
    progress(chat_id, message_id, "done", 100, 100);
    Ok(Transcript {
        text: t.text,
        segments: t
            .segments
            .into_iter()
            .map(|s| Segment {
                start_ms: s.start_ms,
                end_ms: s.end_ms,
                text: s.text,
            })
            .collect(),
        lang: t.lang,
        source: "local".into(),
        cached: false,
    })
}

pub fn install_progress(model: Option<&str>) -> crate::core::tools::ProgressFn {
    let model = crate::core::asr::models::pick(model)
        .map(|m| m.id.to_string())
        .unwrap_or_default();
    std::sync::Arc::new(move |p: crate::core::tools::ToolProgress| {
        events::emit(
            "telegram-asr-progress",
            json!({
                "model": model,
                "id": p.id,
                "stage": p.stage,
                "done": p.done,
                "total": p.total,
                "message": p.message,
            }),
        );
    })
}

pub async fn native(
    handle: &TelegramSessionHandle,
    chat_id: i64,
    chat_type: &str,
    message_id: i32,
) -> Result<serde_json::Value, String> {
    let (client, peer, _) = super::history::peer_for(handle, chat_id, chat_type)
        .await
        .map_err(|e| e.to_string())?;
    let req = tl::functions::messages::TranscribeAudio {
        peer,
        msg_id: message_id,
    };
    let res = client.invoke(&req).await.map_err(|e| {
        let s = e.to_string();
        if s.contains("PREMIUM_ACCOUNT_REQUIRED") || s.contains("TRANSCRIPTION_FAILED") {
            format!("Telegram transcription needs Telegram Premium ({})", s)
        } else {
            s
        }
    })?;
    let tl::enums::messages::TranscribedAudio::Audio(a) = res;
    Ok(json!({
        "text": a.text,
        "pending": a.pending,
        "transcription_id": a.transcription_id,
        "trial_remains_num": a.trial_remains_num,
    }))
}

async fn transcribe_native(
    handle: &TelegramSessionHandle,
    chat_id: i64,
    chat_type: &str,
    message_id: i32,
) -> Result<Transcript, String> {
    for _ in 0..8 {
        let v = native(handle, chat_id, chat_type, message_id).await?;
        let pending = v["pending"].as_bool().unwrap_or(false);
        let text = v["text"].as_str().unwrap_or("").trim().to_string();
        if !pending && !text.is_empty() {
            let dur = super::stream_server::media_for(handle, chat_id, chat_type, message_id)
                .await
                .ok()
                .and_then(|(_, m)| duration_ms(&m.media))
                .unwrap_or(0);
            return Ok(Transcript {
                segments: vec![Segment {
                    start_ms: 0,
                    end_ms: dur,
                    text: text.clone(),
                }],
                text,
                lang: String::new(),
                source: "telegram".into(),
                cached: false,
            });
        }
        tokio::time::sleep(std::time::Duration::from_secs(5)).await;
    }
    Err("Telegram is still transcribing this message; try again in a moment".into())
}

fn duration_ms(media: &tl::enums::MessageMedia) -> Option<i64> {
    let tl::enums::MessageMedia::Document(d) = media else {
        return None;
    };
    let tl::enums::Document::Document(doc) = d.document.as_ref()? else {
        return None;
    };
    doc.attributes.iter().find_map(|a| match a {
        tl::enums::DocumentAttribute::Video(v) => Some((v.duration * 1000.0) as i64),
        tl::enums::DocumentAttribute::Audio(a) => Some(a.duration as i64 * 1000),
        _ => None,
    })
}

pub async fn transcribe(
    handle: &TelegramSessionHandle,
    chat_id: i64,
    chat_type: &str,
    message_id: i32,
    lang: Option<&str>,
    model: Option<&str>,
    force: bool,
) -> Result<Transcript, String> {
    if !force {
        if let Some(t) = cached(chat_id, message_id) {
            return Ok(t);
        }
    }
    let local_err =
        match transcribe_local(handle, chat_id, chat_type, message_id, lang, model).await {
            Ok(t) => {
                save(chat_id, message_id, &t);
                return Ok(t);
            }
            Err(e) => {
                tracing::warn!("[telegram] local transcription failed: {}", e);
                e
            }
        };
    let t = match audio_endpoint() {
        Some((endpoint, key)) => {
            match transcribe_ai(
                handle, chat_id, chat_type, message_id, lang, &endpoint, &key,
            )
            .await
            {
                Ok(t) => t,
                Err(ai_err) => transcribe_native(handle, chat_id, chat_type, message_id)
                    .await
                    .map_err(|_| format!("{} / {}", local_err, ai_err))?,
            }
        }
        None => transcribe_native(handle, chat_id, chat_type, message_id)
            .await
            .map_err(|e| format!("Local transcription failed ({}). {}", local_err, e))?,
    };
    save(chat_id, message_id, &t);
    Ok(t)
}

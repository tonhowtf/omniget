//! Speech to text for meetings. Two backends:
//!
//! - **Local whisper.cpp** (the Meetily model: nothing leaves the machine).
//!   `whisper-cli` comes from the official release on Windows and Linux and
//!   from Homebrew (`brew install whisper-cpp`) on macOS; GGML models come
//!   from Hugging Face into `<app_data>/models/whisper/`.
//! - **The AI provider's audio endpoint** (OpenAI, or any OpenAI-compatible
//!   server such as Groq, speaches or a whisper.cpp server), asked for
//!   `verbose_json` so the segments keep their timestamps. Long recordings
//!   are cut into chunks under the 25 MB upload cap.

use std::path::{Path, PathBuf};

use anyhow::anyhow;
use serde::Serialize;

use super::settings::{MeetingSettings, TranscriptionBackend};
use super::Segment;
use crate::core::dependencies::{self, bin_name};
use crate::core::tools::{self, github, report, ProgressFn};

const REPO: &str = "ggml-org/whisper.cpp";
const HF_BASE: &str = "https://huggingface.co/ggerganov/whisper.cpp/resolve/main";
/// Upload chunk length for the API backend: 20 min of 16 kHz mono at
/// 32 kbps is ~4.8 MB, far below the 25 MB cap even with container overhead.
const API_CHUNK_SECONDS: u64 = 1200;

#[derive(Debug, Clone, Serialize)]
pub struct ModelInfo {
    pub id: String,
    pub size_mb: u64,
    pub note: String,
    pub installed: bool,
}

/// (id, approximate MB, short note)
const MODELS: &[(&str, u64, &str)] = &[
    ("base-q5_1", 60, "fast, basic quality"),
    ("small-q5_1", 190, "balanced for slower machines"),
    ("medium-q5_0", 540, "good quality, slower"),
    (
        "large-v3-turbo-q5_0",
        574,
        "recommended: near large-v3, much faster",
    ),
    ("large-v3-q5_0", 1080, "best quality, quantized"),
];

pub fn models_dir() -> Option<PathBuf> {
    crate::core::paths::app_data_dir().map(|d| d.join("models").join("whisper"))
}

fn model_path(id: &str) -> Option<PathBuf> {
    models_dir().map(|d| d.join(format!("ggml-{id}.bin")))
}

fn managed_dir() -> Option<PathBuf> {
    dependencies::managed_bin_dir().map(|d| d.join("whisper-cpp"))
}

pub fn list_models() -> Vec<ModelInfo> {
    MODELS
        .iter()
        .map(|(id, mb, note)| ModelInfo {
            id: id.to_string(),
            size_mb: *mb,
            note: note.to_string(),
            installed: model_path(id)
                .and_then(|p| std::fs::metadata(p).ok())
                .map(|m| m.len() > 0)
                .unwrap_or(false),
        })
        .collect()
}

pub async fn download_model(id: &str, progress: ProgressFn) -> anyhow::Result<PathBuf> {
    if !MODELS.iter().any(|(m, _, _)| *m == id) {
        return Err(anyhow!("unknown whisper model: {id}"));
    }
    let dest = model_path(id).ok_or_else(|| anyhow!("Could not determine data directory"))?;
    let url = format!("{HF_BASE}/ggml-{id}.bin");
    let client = tools::client()?;
    tools::download_to(
        &client,
        &url,
        &dest,
        &progress,
        &format!("whisper-model:{id}"),
    )
    .await?;
    Ok(dest)
}

pub fn remove_model(id: &str) -> anyhow::Result<()> {
    if let Some(p) = model_path(id) {
        if p.exists() {
            std::fs::remove_file(p)?;
        }
    }
    Ok(())
}

pub async fn locate() -> Option<PathBuf> {
    if let Some(custom) = crate::core::binary_overrides::get("whisper-cli") {
        return Some(custom);
    }
    if let Some(dir) = managed_dir() {
        if let Some(p) = github::find_file(&dir, &bin_name("whisper-cli")) {
            return Some(p);
        }
    }
    // Homebrew installs `whisper-cli`; older builds called it whisper-cpp.
    for name in ["whisper-cli", "whisper-cpp"] {
        if let Some((p, _)) = dependencies::find_tool_with_source(name).await {
            return Some(p);
        }
    }
    None
}

fn release_asset() -> Option<&'static str> {
    if cfg!(target_os = "windows") && cfg!(target_arch = "x86_64") {
        Some("whisper-bin-x64.zip")
    } else if cfg!(target_os = "linux") && cfg!(target_arch = "x86_64") {
        Some("whisper-bin-ubuntu-x64.tar.gz")
    } else if cfg!(target_os = "linux") && cfg!(target_arch = "aarch64") {
        Some("whisper-bin-ubuntu-arm64.tar.gz")
    } else {
        None
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct WhisperStatus {
    pub installed: bool,
    pub path: Option<String>,
    pub can_install: bool,
    /// Shown instead of an install button where upstream ships no binary.
    pub install_hint: String,
    pub models: Vec<ModelInfo>,
    /// The AI provider can transcribe (OpenAI or an OpenAI-compatible URL).
    pub api_available: bool,
}

pub async fn status() -> WhisperStatus {
    let path = locate().await;
    WhisperStatus {
        installed: path.is_some(),
        path: path.map(|p| p.to_string_lossy().to_string()),
        can_install: release_asset().is_some(),
        install_hint: if cfg!(target_os = "macos") {
            "brew install whisper-cpp".to_string()
        } else {
            String::new()
        },
        models: list_models(),
        api_available: crate::core::ai::transcription_target(&crate::core::ai::get()).is_ok(),
    }
}

pub async fn install(progress: ProgressFn) -> anyhow::Result<PathBuf> {
    let name = release_asset().ok_or_else(|| {
        anyhow!("whisper.cpp ships no binary for this system; install it with `brew install whisper-cpp`")
    })?;
    let dir = managed_dir().ok_or_else(|| anyhow!("Could not determine data directory"))?;
    let client = github::client()?;
    let asset = github::asset(&client, REPO, None, |n| n == name).await?;
    let data = github::download(&client, &asset, false, &progress, "whisper-cli").await?;
    let staging = dir.with_extension("new");
    let _ = std::fs::remove_dir_all(&staging);
    let staging_c = staging.clone();
    let asset_name = asset.name.clone();
    tokio::task::spawn_blocking(move || github::unpack(&data, &asset_name, &staging_c))
        .await
        .map_err(|e| anyhow!("Spawn blocking failed: {e}"))??;
    let Some(exe) = github::find_file(&staging, &bin_name("whisper-cli")) else {
        let _ = std::fs::remove_dir_all(&staging);
        return Err(anyhow!("the downloaded package has no whisper-cli"));
    };
    github::make_executable(&exe);
    github::swap_dir(&staging, &dir)?;
    github::strip_quarantine(&dir).await;
    std::fs::write(dir.join("VERSION"), &asset.tag).ok();
    github::find_file(&dir, &bin_name("whisper-cli"))
        .ok_or_else(|| anyhow!("whisper-cli vanished after the install"))
}

#[derive(Debug, Clone)]
pub struct Transcription {
    pub language: Option<String>,
    pub segments: Vec<Segment>,
    /// "whisper-local" or "whisper-api".
    pub source: &'static str,
}

/// Which backend `settings` resolves to right now, or why none can run.
pub async fn pick_backend(settings: &MeetingSettings) -> anyhow::Result<TranscriptionBackend> {
    let local_ready = || async {
        locate().await.is_some()
            && model_path(&settings.whisper_model)
                .map(|p| p.exists())
                .unwrap_or(false)
    };
    let api_ready = crate::core::ai::transcription_target(&crate::core::ai::get()).is_ok();
    match settings.transcription {
        TranscriptionBackend::Local => {
            if local_ready().await {
                Ok(TranscriptionBackend::Local)
            } else {
                Err(anyhow!(
                    "Local Whisper is not ready: install whisper.cpp and download the {} model in Meetings settings",
                    settings.whisper_model
                ))
            }
        }
        TranscriptionBackend::Api => {
            if api_ready {
                Ok(TranscriptionBackend::Api)
            } else {
                Err(anyhow!(
                    "Transcription through the AI provider needs OpenAI or an OpenAI-compatible endpoint"
                ))
            }
        }
        TranscriptionBackend::Auto => {
            if local_ready().await {
                Ok(TranscriptionBackend::Local)
            } else if api_ready {
                Ok(TranscriptionBackend::Api)
            } else {
                Err(anyhow!(
                    "No transcription engine: install local Whisper in Meetings settings, or set OpenAI or an OpenAI-compatible endpoint as the AI provider"
                ))
            }
        }
    }
}

pub async fn transcribe(
    input: &Path,
    settings: &MeetingSettings,
    progress: ProgressFn,
    progress_id: &str,
) -> anyhow::Result<Transcription> {
    match pick_backend(settings).await? {
        TranscriptionBackend::Api => transcribe_api(input, settings, progress, progress_id).await,
        _ => transcribe_local(input, settings, progress, progress_id).await,
    }
}

/// ffmpeg to whatever the backend wants, into a temp file.
async fn convert(input: &Path, args: &[&str], ext: &str) -> anyhow::Result<PathBuf> {
    let ffmpeg = dependencies::ensure_ffmpeg().await?;
    let out = tools::temp_dir().join(format!("meeting-{}.{ext}", uuid::Uuid::new_v4()));
    let output = crate::core::process::command(&ffmpeg)
        .args(["-y", "-hide_banner", "-loglevel", "error", "-i"])
        .arg(input)
        .args(args)
        .arg(&out)
        .output()
        .await?;
    if !output.status.success() {
        return Err(anyhow!(
            "ffmpeg could not convert the audio: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(out)
}

async fn transcribe_local(
    input: &Path,
    settings: &MeetingSettings,
    progress: ProgressFn,
    progress_id: &str,
) -> anyhow::Result<Transcription> {
    let bin = locate()
        .await
        .ok_or_else(|| anyhow!("whisper-cli is not installed"))?;
    let model = model_path(&settings.whisper_model)
        .filter(|p| p.exists())
        .ok_or_else(|| anyhow!("whisper model {} is not downloaded", settings.whisper_model))?;
    let wav = convert(
        input,
        &["-vn", "-ac", "1", "-ar", "16000", "-c:a", "pcm_s16le"],
        "wav",
    )
    .await?;
    let out_prefix = tools::temp_dir().join(format!("meeting-{}", uuid::Uuid::new_v4()));

    let mut cmd = crate::core::process::command(&bin);
    cmd.arg("-m").arg(&model).arg("-f").arg(&wav);
    cmd.args(["-oj", "-pp", "-np", "-of"]).arg(&out_prefix);
    let lang = settings.language.trim();
    cmd.args(["-l", if lang.is_empty() { "auto" } else { lang }]);
    cmd.stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped());
    let mut child = cmd
        .spawn()
        .map_err(|e| anyhow!("could not start whisper-cli: {e}"))?;

    // `-pp` writes "progress = 42%" to stderr.
    let stderr = child.stderr.take();
    let p2 = progress.clone();
    let id2 = progress_id.to_string();
    let stderr_task = tokio::spawn(async move {
        use tokio::io::{AsyncBufReadExt, BufReader};
        let mut tail = String::new();
        if let Some(err) = stderr {
            let mut lines = BufReader::new(err).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                if let Some(rest) = line.trim().strip_prefix("progress =") {
                    if let Ok(pct) = rest.trim().trim_end_matches('%').trim().parse::<u64>() {
                        report(&p2, &id2, "transcribing", pct.min(100), Some(100), None);
                    }
                } else if !line.trim().is_empty() {
                    tail = line;
                }
            }
        }
        tail
    });
    let status = child.wait().await?;
    let tail = stderr_task.await.unwrap_or_default();
    let _ = tokio::fs::remove_file(&wav).await;
    if !status.success() {
        return Err(anyhow!("whisper-cli failed: {tail}"));
    }
    let json_path = out_prefix.with_extension("json");
    let text = tokio::fs::read_to_string(&json_path).await?;
    let _ = tokio::fs::remove_file(&json_path).await;
    let (language, segments) = parse_whisper_cpp_json(&text)?;
    Ok(Transcription {
        language: Some(language).filter(|l| !l.is_empty()),
        segments,
        source: "whisper-local",
    })
}

fn parse_whisper_cpp_json(text: &str) -> anyhow::Result<(String, Vec<Segment>)> {
    let json: serde_json::Value = serde_json::from_str(text)?;
    let language = json["result"]["language"]
        .as_str()
        .unwrap_or("")
        .to_string();
    let mut segments = Vec::new();
    for it in json["transcription"].as_array().into_iter().flatten() {
        let from = it["offsets"]["from"].as_u64().unwrap_or(0);
        let to = it["offsets"]["to"].as_u64().unwrap_or(from);
        let t = it["text"].as_str().unwrap_or("").trim();
        if t.is_empty() || is_noise(t) {
            continue;
        }
        segments.push(Segment {
            start_ms: from,
            end_ms: to.max(from + 1),
            speaker: None,
            text: t.to_string(),
        });
    }
    Ok((language, segments))
}

/// Whisper's stock hallucinations on silence.
fn is_noise(t: &str) -> bool {
    let low = t.to_lowercase();
    matches!(
        low.trim_matches(|c: char| !c.is_alphanumeric() && c != ' '),
        "" | "blank_audio" | "silence" | "music" | "inaudible"
    ) || (low.starts_with('[') && low.ends_with(']'))
        || (low.starts_with('(') && low.ends_with(')'))
}

async fn transcribe_api(
    input: &Path,
    settings: &MeetingSettings,
    progress: ProgressFn,
    progress_id: &str,
) -> anyhow::Result<Transcription> {
    let cfg = crate::core::ai::get();
    let (endpoint, key) = crate::core::ai::transcription_target(&cfg).map_err(|e| anyhow!(e))?;

    // Cut into mono 16 kHz 32 kbps mp3 chunks; each keeps its offset.
    let ffmpeg = dependencies::ensure_ffmpeg().await?;
    let chunk_dir = tools::temp_dir().join(format!("meeting-chunks-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&chunk_dir)?;
    let pattern = chunk_dir.join("part-%04d.mp3");
    let output = crate::core::process::command(&ffmpeg)
        .args(["-y", "-hide_banner", "-loglevel", "error", "-i"])
        .arg(input)
        .args([
            "-vn",
            "-ac",
            "1",
            "-ar",
            "16000",
            "-b:a",
            "32k",
            "-f",
            "segment",
            "-segment_time",
        ])
        .arg(API_CHUNK_SECONDS.to_string())
        .args(["-reset_timestamps", "1"])
        .arg(&pattern)
        .output()
        .await?;
    if !output.status.success() {
        let _ = std::fs::remove_dir_all(&chunk_dir);
        return Err(anyhow!(
            "ffmpeg could not prepare the audio: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    let mut chunks: Vec<PathBuf> = std::fs::read_dir(&chunk_dir)?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "mp3"))
        .collect();
    chunks.sort();

    let client = tools::client()?;
    let lang = settings.language.trim();
    let mut segments = Vec::new();
    let mut language = None;
    let total = chunks.len() as u64;
    let result: anyhow::Result<()> = async {
        for (i, chunk) in chunks.iter().enumerate() {
            report(
                &progress,
                progress_id,
                "transcribing",
                i as u64 * 100 / total.max(1),
                Some(100),
                None,
            );
            let offset_ms = i as u64 * API_CHUNK_SECONDS * 1000;
            let bytes = tokio::fs::read(chunk).await?;
            let part = reqwest::multipart::Part::bytes(bytes)
                .file_name(format!("part-{i}.mp3"))
                .mime_str("audio/mpeg")?;
            let mut form = reqwest::multipart::Form::new()
                .text("model", "whisper-1")
                .text("response_format", "verbose_json")
                .text("timestamp_granularities[]", "segment")
                .part("file", part);
            if !lang.is_empty() && lang != "auto" {
                form = form.text("language", lang.to_string());
            }
            let mut req = client.post(&endpoint).multipart(form);
            if !key.is_empty() {
                req = req.bearer_auth(&key);
            }
            let resp = req.send().await?;
            let status = resp.status();
            let body = resp.text().await?;
            if !status.is_success() {
                return Err(anyhow!(
                    "transcription failed ({}): {}",
                    status.as_u16(),
                    body.chars().take(300).collect::<String>()
                ));
            }
            let (lang_found, mut segs) = parse_verbose_json(&body, offset_ms);
            if language.is_none() {
                language = lang_found;
            }
            segments.append(&mut segs);
        }
        Ok(())
    }
    .await;
    let _ = std::fs::remove_dir_all(&chunk_dir);
    result?;

    let seconds = crate::core::ffmpeg::probe(input)
        .await
        .map(|p| p.duration_seconds)
        .unwrap_or(0.0);
    crate::core::tools::usage::record_entry(crate::core::tools::usage::UsageEntry {
        characters: segments.iter().map(|s| s.text.chars().count() as u64).sum(),
        seconds,
        cost_usd: if endpoint.starts_with("https://api.openai.com") {
            None
        } else {
            Some(0.0)
        },
        ..crate::core::tools::usage::UsageEntry::now(
            "meeting-transcribe",
            &cfg.provider_id(),
            "whisper-1",
        )
    });
    Ok(Transcription {
        language,
        segments,
        source: "whisper-api",
    })
}

/// OpenAI `verbose_json` (`language`, `segments[{start,end,text}]` in
/// seconds). Servers that ignore the format and answer plain text still
/// produce one segment instead of an error.
fn parse_verbose_json(body: &str, offset_ms: u64) -> (Option<String>, Vec<Segment>) {
    let Ok(json) = serde_json::from_str::<serde_json::Value>(body) else {
        let t = body.trim();
        return (
            None,
            if t.is_empty() {
                vec![]
            } else {
                vec![Segment {
                    start_ms: offset_ms,
                    end_ms: offset_ms,
                    speaker: None,
                    text: t.to_string(),
                }]
            },
        );
    };
    let language = json["language"].as_str().map(normalize_language);
    let mut out = Vec::new();
    if let Some(segs) = json["segments"].as_array() {
        for s in segs {
            let text = s["text"].as_str().unwrap_or("").trim();
            if text.is_empty() || is_noise(text) {
                continue;
            }
            let start = (s["start"].as_f64().unwrap_or(0.0) * 1000.0) as u64 + offset_ms;
            let end = (s["end"].as_f64().unwrap_or(0.0) * 1000.0) as u64 + offset_ms;
            out.push(Segment {
                start_ms: start,
                end_ms: end.max(start),
                speaker: None,
                text: text.to_string(),
            });
        }
    } else if let Some(text) = json["text"].as_str().filter(|t| !t.trim().is_empty()) {
        out.push(Segment {
            start_ms: offset_ms,
            end_ms: offset_ms,
            speaker: None,
            text: text.trim().to_string(),
        });
    }
    (language, out)
}

/// OpenAI answers the language as a name ("portuguese"); keep ISO codes.
fn normalize_language(l: &str) -> String {
    match l.to_lowercase().as_str() {
        "english" => "en",
        "portuguese" => "pt",
        "spanish" => "es",
        "french" => "fr",
        "german" => "de",
        "italian" => "it",
        "russian" => "ru",
        "chinese" => "zh",
        "japanese" => "ja",
        "korean" => "ko",
        other => return other.to_string(),
    }
    .to_string()
}

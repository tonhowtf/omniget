use serde::Serialize;
use std::process::Stdio;
use tauri::{AppHandle, Emitter};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;

const WHISPER_SCRIPT: &str = r#"
import sys
import json
import time

video_path = sys.argv[1]
output_path = sys.argv[2]
model_size = sys.argv[3]
language = sys.argv[4]

try:
    from faster_whisper import WhisperModel
except ImportError:
    print(json.dumps({"error": "faster-whisper is not installed. Run: pip install --user faster-whisper"}), flush=True)
    sys.exit(2)

print(json.dumps({"stage": "loading_model", "model": model_size}), flush=True)
model = WhisperModel(model_size, device="cpu", compute_type="int8")

print(json.dumps({"stage": "transcribing"}), flush=True)
segments, info = model.transcribe(
    video_path,
    language=None if language in ("auto", "", None) else language,
    beam_size=1,
    vad_filter=True,
)

duration = info.duration or 1.0
lines = []
last_pct = -1

for seg in segments:
    text = seg.text.strip()
    if text:
        lines.append(text)
    pct = int(min(100, (seg.end / duration) * 100))
    if pct != last_pct:
        print(json.dumps({"stage": "progress", "percent": pct, "current": seg.end, "total": duration}), flush=True)
        last_pct = pct

full_text = "\n".join(lines)

try:
    with open(output_path, "w", encoding="utf-8") as f:
        f.write(full_text)
except Exception as e:
    print(json.dumps({"error": f"Failed to write output: {e}"}), flush=True)
    sys.exit(3)

print(json.dumps({
    "stage": "done",
    "language": info.language,
    "duration": duration,
    "text": full_text,
    "output_path": output_path,
}), flush=True)
"#;

#[derive(Debug, Serialize, Clone)]
pub struct TranscribeResult {
    pub language: String,
    pub duration: f64,
    pub text: String,
    pub output_path: String,
}

#[derive(Debug, Serialize, Clone)]
#[serde(tag = "stage", rename_all = "snake_case")]
pub enum TranscribeEvent {
    LoadingModel {
        model: String,
    },
    Transcribing,
    Progress {
        percent: u32,
        current: f64,
        total: f64,
    },
    Done {
        output_path: String,
    },
    Error {
        message: String,
    },
}

fn find_python() -> Result<std::path::PathBuf, String> {
    if let Ok(p) = which::which("python") {
        return Ok(p);
    }
    if let Ok(p) = which::which("python3") {
        return Ok(p);
    }
    if let Ok(p) = which::which("py") {
        return Ok(p);
    }
    Err("Python not found. Install Python 3.10+ and add it to PATH.".to_string())
}

#[tauri::command]
pub async fn transcribe_video(
    app: AppHandle,
    video_path: String,
    output_path: String,
    model_size: Option<String>,
    language: Option<String>,
    job_id: Option<String>,
) -> Result<TranscribeResult, String> {
    let python = find_python()?;

    let tmp_script = std::env::temp_dir().join("omniget_transcribe.py");
    tokio::fs::write(&tmp_script, WHISPER_SCRIPT)
        .await
        .map_err(|e| format!("Failed to write script: {}", e))?;

    let model = model_size.unwrap_or_else(|| "small".to_string());
    let lang = language.unwrap_or_else(|| "auto".to_string());
    let job = job_id.unwrap_or_else(|| "default".to_string());
    let event_name = format!("transcript:{}", job);

    let mut child = Command::new(&python)
        .arg("-u")
        .arg(&tmp_script)
        .arg(&video_path)
        .arg(&output_path)
        .arg(&model)
        .arg(&lang)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("Failed to spawn python: {}", e))?;

    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| "Failed to capture stdout".to_string())?;
    let mut reader = BufReader::new(stdout).lines();

    let mut result: Option<TranscribeResult> = None;
    let mut last_error: Option<String> = None;

    while let Some(line) = reader
        .next_line()
        .await
        .map_err(|e| format!("Read error: {}", e))?
    {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(trimmed) {
            if let Some(err) = v.get("error").and_then(|e| e.as_str()) {
                last_error = Some(err.to_string());
                let _ = app.emit(
                    &event_name,
                    TranscribeEvent::Error {
                        message: err.to_string(),
                    },
                );
                continue;
            }
            let stage = v.get("stage").and_then(|s| s.as_str()).unwrap_or("");
            match stage {
                "loading_model" => {
                    let m = v
                        .get("model")
                        .and_then(|m| m.as_str())
                        .unwrap_or(&model)
                        .to_string();
                    let _ = app.emit(&event_name, TranscribeEvent::LoadingModel { model: m });
                }
                "transcribing" => {
                    let _ = app.emit(&event_name, TranscribeEvent::Transcribing);
                }
                "progress" => {
                    let percent = v.get("percent").and_then(|p| p.as_u64()).unwrap_or(0) as u32;
                    let current = v.get("current").and_then(|c| c.as_f64()).unwrap_or(0.0);
                    let total = v.get("total").and_then(|c| c.as_f64()).unwrap_or(0.0);
                    let _ = app.emit(
                        &event_name,
                        TranscribeEvent::Progress {
                            percent,
                            current,
                            total,
                        },
                    );
                }
                "done" => {
                    let language = v
                        .get("language")
                        .and_then(|l| l.as_str())
                        .unwrap_or("")
                        .to_string();
                    let duration = v.get("duration").and_then(|d| d.as_f64()).unwrap_or(0.0);
                    let text = v
                        .get("text")
                        .and_then(|t| t.as_str())
                        .unwrap_or("")
                        .to_string();
                    let out = v
                        .get("output_path")
                        .and_then(|o| o.as_str())
                        .unwrap_or(&output_path)
                        .to_string();
                    let _ = app.emit(
                        &event_name,
                        TranscribeEvent::Done {
                            output_path: out.clone(),
                        },
                    );
                    result = Some(TranscribeResult {
                        language,
                        duration,
                        text,
                        output_path: out,
                    });
                }
                _ => {}
            }
        }
    }

    let status = child
        .wait()
        .await
        .map_err(|e| format!("Wait error: {}", e))?;

    if !status.success() {
        if let Some(err) = last_error {
            return Err(err);
        }
        return Err(format!(
            "Transcription process failed with status {}",
            status
        ));
    }

    result.ok_or_else(|| "Transcription completed but no result was returned".to_string())
}

#[tauri::command]
pub async fn write_text_file(path: String, content: String) -> Result<(), String> {
    tokio::fs::write(&path, content)
        .await
        .map_err(|e| format!("Failed to write {}: {}", path, e))
}

#[tauri::command]
pub async fn check_whisper_installed() -> Result<bool, String> {
    let python = match find_python() {
        Ok(p) => p,
        Err(_) => return Ok(false),
    };
    let output = Command::new(&python)
        .args(["-c", "import faster_whisper"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .await
        .map_err(|e| e.to_string())?;
    Ok(output.status.success())
}

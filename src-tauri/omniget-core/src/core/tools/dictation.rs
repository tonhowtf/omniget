//! Ditado (VoiceStudio, lote 2): um atalho global começa a gravar o microfone
//! com o FFmpeg, o segundo aperto para, o whisper.cpp local transcreve e o
//! texto é digitado onde o cursor estiver (via `enigo`) ou colado pela área de
//! transferência. Nada sai da máquina. O atalho em si mora no app (hotkey.rs).

use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Instant;

use anyhow::anyhow;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DictationOptions {
    /// id do modelo GGML do whisper (ver tools.whisper)
    #[serde(default = "base")]
    pub model: String,
    #[serde(default = "auto")]
    pub language: String,
    /// dispositivo de entrada (índice avfoundation, nome dshow, nome pulse); vazio = padrão
    #[serde(default)]
    pub device: String,
    /// "type" (digitar) | "paste" (colar) | "clipboard" (só copiar)
    #[serde(default = "type_mode")]
    pub output: String,
    /// espaço/nova linha ao final
    #[serde(default)]
    pub trailing_space: bool,
}

fn base() -> String {
    "base".into()
}
fn auto() -> String {
    "auto".into()
}
fn type_mode() -> String {
    "type".into()
}

#[derive(Debug, Clone, Serialize)]
pub struct AudioDevice {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct DictationState {
    /// "idle" | "recording" | "transcribing"
    pub phase: String,
    pub seconds: u64,
    pub last_text: String,
    pub error: Option<String>,
}

struct Recording {
    child: tokio::process::Child,
    wav: PathBuf,
    started: Instant,
}

static REC: Mutex<Option<Recording>> = Mutex::new(None);
static PHASE: Mutex<String> = Mutex::new(String::new());
static LAST: Mutex<Option<(String, Option<String>)>> = Mutex::new(None);
static OPTS: Mutex<Option<DictationOptions>> = Mutex::new(None);

fn set_phase(p: &str) {
    *PHASE.lock().unwrap_or_else(|e| e.into_inner()) = p.to_string();
}

pub fn state() -> DictationState {
    let phase = PHASE.lock().unwrap_or_else(|e| e.into_inner()).clone();
    let seconds = REC
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .as_ref()
        .map(|r| r.started.elapsed().as_secs())
        .unwrap_or(0);
    let (last_text, error) = LAST
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone()
        .unwrap_or_default();
    DictationState {
        phase: if phase.is_empty() {
            "idle".into()
        } else {
            phase
        },
        seconds,
        last_text,
        error,
    }
}

pub fn set_options(opts: DictationOptions) {
    *OPTS.lock().unwrap_or_else(|e| e.into_inner()) = Some(opts);
}

pub fn options() -> DictationOptions {
    OPTS.lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone()
        .unwrap_or_else(|| serde_json::from_str("{}").unwrap())
}

/// Dispositivos de entrada de áudio que o FFmpeg enxerga.
pub async fn devices() -> Vec<AudioDevice> {
    let Ok(ffmpeg) = crate::core::dependencies::ensure_ffmpeg().await else {
        return vec![];
    };
    let mut out = Vec::new();
    if cfg!(target_os = "macos") {
        let o = crate::core::process::command(&ffmpeg)
            .args([
                "-hide_banner",
                "-f",
                "avfoundation",
                "-list_devices",
                "true",
                "-i",
                "",
            ])
            .output()
            .await;
        if let Ok(o) = o {
            let text = String::from_utf8_lossy(&o.stderr);
            let mut in_audio = false;
            for l in text.lines() {
                if l.contains("audio devices") {
                    in_audio = true;
                    continue;
                }
                if l.contains("video devices") {
                    in_audio = false;
                }
                if in_audio {
                    if let Some(i) = l.find('[') {
                        if let Some(j) = l[i..].find(']') {
                            let idx = &l[i + 1..i + j];
                            let name = l[i + j + 1..].trim();
                            if idx.chars().all(|c| c.is_ascii_digit()) {
                                out.push(AudioDevice {
                                    id: idx.to_string(),
                                    name: name.to_string(),
                                });
                            }
                        }
                    }
                }
            }
        }
    } else if cfg!(target_os = "windows") {
        let o = crate::core::process::command(&ffmpeg)
            .args([
                "-hide_banner",
                "-list_devices",
                "true",
                "-f",
                "dshow",
                "-i",
                "dummy",
            ])
            .output()
            .await;
        if let Ok(o) = o {
            let text = String::from_utf8_lossy(&o.stderr);
            for l in text.lines() {
                if l.contains("(audio)") {
                    if let Some(name) = l.split('"').nth(1) {
                        out.push(AudioDevice {
                            id: name.to_string(),
                            name: name.to_string(),
                        });
                    }
                }
            }
        }
    } else {
        out.push(AudioDevice {
            id: "default".into(),
            name: "PulseAudio / PipeWire (default)".into(),
        });
        if let Ok(o) = crate::core::process::command("pactl")
            .args(["list", "short", "sources"])
            .output()
            .await
        {
            for l in String::from_utf8_lossy(&o.stdout).lines() {
                let cols: Vec<&str> = l.split('\t').collect();
                if cols.len() > 1 && !cols[1].ends_with(".monitor") {
                    out.push(AudioDevice {
                        id: cols[1].to_string(),
                        name: cols[1].to_string(),
                    });
                }
            }
        }
    }
    out
}

fn input_args(device: &str) -> Vec<String> {
    let dev = device.trim();
    if cfg!(target_os = "macos") {
        vec![
            "-f".into(),
            "avfoundation".into(),
            "-i".into(),
            format!(":{}", if dev.is_empty() { "0" } else { dev }),
        ]
    } else if cfg!(target_os = "windows") {
        vec![
            "-f".into(),
            "dshow".into(),
            "-i".into(),
            format!("audio={}", if dev.is_empty() { "default" } else { dev }),
        ]
    } else {
        vec![
            "-f".into(),
            "pulse".into(),
            "-i".into(),
            if dev.is_empty() {
                "default".into()
            } else {
                dev.to_string()
            },
        ]
    }
}

pub async fn start(progress: super::ProgressFn) -> anyhow::Result<()> {
    {
        let rec = REC.lock().unwrap_or_else(|e| e.into_inner());
        if rec.is_some() {
            return Err(anyhow!("ja esta gravando"));
        }
    }
    let ffmpeg = crate::core::dependencies::ensure_ffmpeg().await?;
    let opts = options();
    let wav = super::temp_dir().join(format!("dictation-{}.wav", uuid::Uuid::new_v4()));
    let mut cmd = crate::core::process::command(&ffmpeg);
    cmd.args(["-y", "-hide_banner", "-loglevel", "error"]);
    cmd.args(input_args(&opts.device));
    cmd.args(["-ac", "1", "-ar", "16000", "-c:a", "pcm_s16le"])
        .arg(&wav);
    cmd.stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped());
    let child = cmd.spawn().map_err(|e| anyhow!("ffmpeg: {}", e))?;
    *REC.lock().unwrap_or_else(|e| e.into_inner()) = Some(Recording {
        child,
        wav,
        started: Instant::now(),
    });
    set_phase("recording");
    *LAST.lock().unwrap_or_else(|e| e.into_inner()) = None;
    super::report(&progress, "dictation", "recording", 0, None, None);
    Ok(())
}

/// Para a gravação, transcreve e entrega o texto. Devolve o texto.
pub async fn stop(progress: super::ProgressFn) -> anyhow::Result<String> {
    let rec = REC.lock().unwrap_or_else(|e| e.into_inner()).take();
    let Some(mut rec) = rec else {
        return Err(anyhow!("nao esta gravando"));
    };
    set_phase("transcribing");
    super::report(&progress, "dictation", "transcribing", 0, None, None);
    let result: anyhow::Result<String> = async {
        if rec.started.elapsed().as_millis() < 400 {
            tokio::time::sleep(std::time::Duration::from_millis(400)).await;
        }
        // 'q' no stdin encerra o ffmpeg fechando o WAV corretamente.
        if let Some(mut stdin) = rec.child.stdin.take() {
            use tokio::io::AsyncWriteExt;
            let _ = stdin.write_all(b"q\n").await;
            let _ = stdin.flush().await;
        }
        let status =
            tokio::time::timeout(std::time::Duration::from_secs(8), rec.child.wait()).await;
        if status.is_err() {
            let _ = rec.child.kill().await;
        }
        if !rec.wav.exists() || std::fs::metadata(&rec.wav).map(|m| m.len()).unwrap_or(0) < 1000 {
            let mut err = String::new();
            if let Some(mut e) = rec.child.stderr.take() {
                use tokio::io::AsyncReadExt;
                let _ = e.read_to_string(&mut err).await;
            }
            return Err(anyhow!(
                "nada foi gravado (microfone/permissao?) {}",
                err.trim()
            ));
        }
        let opts = options();
        let out_dir = super::temp_dir().join("dictation-out");
        let _ = std::fs::create_dir_all(&out_dir);
        let t = super::whisper::transcribe(
            super::whisper::TranscribeOptions {
                input: rec.wav.to_string_lossy().to_string(),
                model: opts.model.clone(),
                language: opts.language.clone(),
                translate: false,
                max_len: 0,
                prompt: String::new(),
                output_dir: out_dir.to_string_lossy().to_string(),
                threads: 0,
            },
            progress.clone(),
        )
        .await?;
        let _ = std::fs::remove_file(&rec.wav);
        let _ = std::fs::remove_dir_all(&out_dir);
        let mut text = t.text.split_whitespace().collect::<Vec<_>>().join(" ");
        if text.is_empty() {
            return Err(anyhow!("nao entendi nada"));
        }
        if opts.trailing_space {
            text.push(' ');
        }
        Ok(text)
    }
    .await;
    set_phase("idle");
    match result {
        Ok(text) => {
            *LAST.lock().unwrap_or_else(|e| e.into_inner()) = Some((text.clone(), None));
            super::report(
                &progress,
                "dictation",
                "done",
                1,
                Some(1),
                Some(text.clone()),
            );
            Ok(text)
        }
        Err(e) => {
            *LAST.lock().unwrap_or_else(|e| e.into_inner()) =
                Some((String::new(), Some(e.to_string())));
            super::report(
                &progress,
                "dictation",
                "error",
                0,
                None,
                Some(e.to_string()),
            );
            Err(e)
        }
    }
}

pub fn is_recording() -> bool {
    REC.lock().unwrap_or_else(|e| e.into_inner()).is_some()
}

/// Digita o texto onde o cursor estiver.
pub fn type_text(text: &str) -> anyhow::Result<()> {
    use enigo::{Enigo, Keyboard, Settings};
    let mut enigo = Enigo::new(&Settings::default())
        .map_err(|e| anyhow!("teclado: {} (macOS: permissao de Acessibilidade)", e))?;
    // Um instante para o usuário soltar o atalho antes de digitar.
    std::thread::sleep(std::time::Duration::from_millis(150));
    enigo.text(text).map_err(|e| anyhow!("digitar: {}", e))?;
    Ok(())
}

/// Cmd+V / Ctrl+V (o app coloca o texto na área de transferência antes).
pub fn press_paste() -> anyhow::Result<()> {
    use enigo::{Direction, Enigo, Key, Keyboard, Settings};
    let mut enigo = Enigo::new(&Settings::default()).map_err(|e| anyhow!("teclado: {}", e))?;
    std::thread::sleep(std::time::Duration::from_millis(150));
    let modifier = if cfg!(target_os = "macos") {
        Key::Meta
    } else {
        Key::Control
    };
    enigo
        .key(modifier, Direction::Press)
        .map_err(|e| anyhow!("{}", e))?;
    enigo
        .key(Key::Unicode('v'), Direction::Click)
        .map_err(|e| anyhow!("{}", e))?;
    enigo
        .key(modifier, Direction::Release)
        .map_err(|e| anyhow!("{}", e))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults() {
        let o = options();
        assert_eq!(o.model, "base");
        assert_eq!(o.output, "type");
        assert!(input_args("").len() == 4);
        assert_eq!(state().phase, "idle");
    }
}

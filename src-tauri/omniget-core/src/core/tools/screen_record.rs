//! Gravar tela (estudo 28, ShareX): FFmpeg com o capturador nativo de cada
//! sistema (avfoundation, gdigrab, x11grab), microfone opcional e buffer de
//! replay: em vez de um arquivo só, o FFmpeg grava segmentos de 5 s num anel
//! e "salvar os últimos N segundos" concatena os segmentos mais recentes.
//! Áudio do sistema: no Windows via "Stereo Mix"/loopback quando existe; no
//! macOS exige um dispositivo virtual (BlackHole); no Linux o monitor do
//! PulseAudio.

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Instant;

use anyhow::anyhow;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize)]
pub struct Source {
    pub id: String,
    pub name: String,
    /// "screen" | "audio"
    pub kind: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecordOptions {
    /// tela (índice avfoundation, "desktop" no Windows, ":0.0" no Linux)
    #[serde(default)]
    pub screen: String,
    #[serde(default = "thirty")]
    pub fps: u32,
    /// microfone/dispositivo de áudio; vazio = sem áudio
    #[serde(default)]
    pub audio: String,
    #[serde(default)]
    pub output_dir: String,
    /// 0 = gravação normal; N = anel de replay com N segundos
    #[serde(default)]
    pub replay_seconds: u32,
    /// área x,y,largura,altura (vazio = tela inteira)
    #[serde(default)]
    pub area: Option<(u32, u32, u32, u32)>,
    #[serde(default = "twenty_three")]
    pub crf: u32,
    #[serde(default)]
    pub cursor: bool,
}

fn thirty() -> u32 {
    30
}
fn twenty_three() -> u32 {
    23
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct RecordState {
    pub running: bool,
    pub replay: bool,
    pub seconds: u64,
    pub output: Option<String>,
    pub last_saved: Option<String>,
    pub error: Option<String>,
}

struct Session {
    child: tokio::process::Child,
    started: Instant,
    output: Option<PathBuf>,
    ring_dir: Option<PathBuf>,
    opts: RecordOptions,
}

static SESSION: Mutex<Option<Session>> = Mutex::new(None);
static LAST_SAVED: Mutex<Option<String>> = Mutex::new(None);
static ERROR: Mutex<Option<String>> = Mutex::new(None);

pub fn state() -> RecordState {
    let s = SESSION.lock().unwrap_or_else(|e| e.into_inner());
    RecordState {
        running: s.is_some(),
        replay: s.as_ref().map(|s| s.ring_dir.is_some()).unwrap_or(false),
        seconds: s.as_ref().map(|s| s.started.elapsed().as_secs()).unwrap_or(0),
        output: s.as_ref().and_then(|s| s.output.as_ref().map(|p| p.to_string_lossy().to_string())),
        last_saved: LAST_SAVED.lock().unwrap_or_else(|e| e.into_inner()).clone(),
        error: ERROR.lock().unwrap_or_else(|e| e.into_inner()).clone(),
    }
}

/// Telas e dispositivos de áudio que o FFmpeg enxerga.
pub async fn sources() -> Vec<Source> {
    let mut out = Vec::new();
    if cfg!(target_os = "macos") {
        let Ok(ffmpeg) = crate::core::dependencies::ensure_ffmpeg().await else { return out };
        let o = crate::core::process::command(&ffmpeg).args(["-hide_banner", "-f", "avfoundation", "-list_devices", "true", "-i", ""]).output().await;
        if let Ok(o) = o {
            let text = String::from_utf8_lossy(&o.stderr);
            let mut kind = "";
            for l in text.lines() {
                if l.contains("video devices") {
                    kind = "screen";
                    continue;
                }
                if l.contains("audio devices") {
                    kind = "audio";
                    continue;
                }
                if kind.is_empty() {
                    continue;
                }
                if let Some(i) = l.find('[') {
                    if let Some(j) = l[i..].find(']') {
                        let idx = &l[i + 1..i + j];
                        let name = l[i + j + 1..].trim();
                        if idx.chars().all(|c| c.is_ascii_digit()) && (kind == "audio" || name.starts_with("Capture screen")) {
                            out.push(Source { id: idx.to_string(), name: name.to_string(), kind: kind.into() });
                        }
                    }
                }
            }
        }
    } else if cfg!(target_os = "windows") {
        out.push(Source { id: "desktop".into(), name: "Área de trabalho (todas as telas)".into(), kind: "screen".into() });
        for d in super::dictation::devices().await {
            out.push(Source { id: d.id, name: d.name, kind: "audio".into() });
        }
    } else {
        let display = std::env::var("DISPLAY").unwrap_or_else(|_| ":0".into());
        out.push(Source { id: format!("{}.0", display), name: format!("X11 {}", display), kind: "screen".into() });
        for d in super::dictation::devices().await {
            out.push(Source { id: d.id, name: d.name, kind: "audio".into() });
        }
        if let Ok(o) = crate::core::process::command("pactl").args(["list", "short", "sources"]).output().await {
            for l in String::from_utf8_lossy(&o.stdout).lines() {
                let cols: Vec<&str> = l.split('\t').collect();
                if cols.len() > 1 && cols[1].ends_with(".monitor") {
                    out.push(Source { id: cols[1].to_string(), name: format!("Áudio do sistema ({})", cols[1]), kind: "audio".into() });
                }
            }
        }
    }
    out
}

fn stamp() -> String {
    chrono::Local::now().format("%Y-%m-%d %H-%M-%S").to_string()
}

fn videos_dir() -> PathBuf {
    dirs::video_dir().or_else(dirs::home_dir).unwrap_or_default().join("OmniGet")
}

fn build_args(opts: &RecordOptions, target: &Path, ring: bool) -> Vec<String> {
    let mut a: Vec<String> = vec!["-y".into(), "-hide_banner".into(), "-loglevel".into(), "error".into()];
    let fps = opts.fps.clamp(5, 120).to_string();
    if cfg!(target_os = "macos") {
        a.extend(["-f".into(), "avfoundation".into(), "-framerate".into(), fps.clone(), "-capture_cursor".into(), if opts.cursor { "1".into() } else { "0".into() }, "-pix_fmt".into(), "uyvy422".into()]);
        let screen = if opts.screen.is_empty() { "1".to_string() } else { opts.screen.clone() };
        let audio = if opts.audio.is_empty() { "none".to_string() } else { opts.audio.clone() };
        a.extend(["-i".into(), format!("{}:{}", screen, audio)]);
    } else if cfg!(target_os = "windows") {
        a.extend(["-f".into(), "gdigrab".into(), "-framerate".into(), fps.clone(), "-draw_mouse".into(), if opts.cursor { "1".into() } else { "0".into() }]);
        if let Some((x, y, w, h)) = opts.area {
            a.extend(["-offset_x".into(), x.to_string(), "-offset_y".into(), y.to_string(), "-video_size".into(), format!("{}x{}", w, h)]);
        }
        a.extend(["-i".into(), if opts.screen.is_empty() { "desktop".into() } else { opts.screen.clone() }]);
        if !opts.audio.is_empty() {
            a.extend(["-f".into(), "dshow".into(), "-i".into(), format!("audio={}", opts.audio)]);
        }
    } else {
        a.extend(["-f".into(), "x11grab".into(), "-framerate".into(), fps.clone(), "-draw_mouse".into(), if opts.cursor { "1".into() } else { "0".into() }]);
        let mut input = if opts.screen.is_empty() { format!("{}.0", std::env::var("DISPLAY").unwrap_or_else(|_| ":0".into())) } else { opts.screen.clone() };
        if let Some((x, y, w, h)) = opts.area {
            a.extend(["-video_size".into(), format!("{}x{}", w, h)]);
            input = format!("{}+{},{}", input, x, y);
        }
        a.extend(["-i".into(), input]);
        if !opts.audio.is_empty() {
            a.extend(["-f".into(), "pulse".into(), "-i".into(), opts.audio.clone()]);
        }
    }
    if cfg!(target_os = "macos") {
        if let Some((x, y, w, h)) = opts.area {
            a.extend(["-vf".into(), format!("crop={}:{}:{}:{}", w, h, x, y)]);
        }
    }
    a.extend(["-c:v".into(), "libx264".into(), "-preset".into(), "veryfast".into(), "-crf".into(), opts.crf.clamp(10, 40).to_string(), "-pix_fmt".into(), "yuv420p".into()]);
    a.extend(["-c:a".into(), "aac".into(), "-b:a".into(), "160k".into()]);
    a.extend(["-movflags".into(), "+faststart".into()]);
    if ring {
        let wrap = (opts.replay_seconds.max(10) / 5 + 2).to_string();
        a.extend(["-f".into(), "segment".into(), "-segment_time".into(), "5".into(), "-segment_wrap".into(), wrap, "-reset_timestamps".into(), "1".into(), "-segment_format_options".into(), "movflags=+faststart".into()]);
        a.push(target.join("seg-%03d.mp4").to_string_lossy().to_string());
    } else {
        a.push(target.to_string_lossy().to_string());
    }
    a
}

pub async fn start(opts: RecordOptions) -> anyhow::Result<RecordState> {
    if SESSION.lock().unwrap_or_else(|e| e.into_inner()).is_some() {
        return Err(anyhow!("ja esta gravando"));
    }
    if cfg!(target_os = "linux") && std::env::var("WAYLAND_DISPLAY").is_ok() && std::env::var("DISPLAY").is_err() {
        return Err(anyhow!("sessao Wayland sem XWayland: o x11grab nao enxerga a tela. Use a captura do OmniDisc (PipeWire) ou entre numa sessao X11."));
    }
    let ffmpeg = crate::core::dependencies::ensure_ffmpeg().await?;
    let dir = if opts.output_dir.trim().is_empty() { videos_dir() } else { PathBuf::from(opts.output_dir.trim()) };
    std::fs::create_dir_all(&dir)?;
    let ring = opts.replay_seconds > 0;
    let (target, ring_dir, output) = if ring {
        let r = super::temp_dir().join(format!("replay-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&r)?;
        (r.clone(), Some(r), None)
    } else {
        let out = dir.join(format!("Gravacao {}.mp4", stamp()));
        (out.clone(), None, Some(out))
    };
    let args = build_args(&opts, &target, ring);
    let mut cmd = crate::core::process::command(&ffmpeg);
    cmd.args(&args).stdin(std::process::Stdio::piped()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::piped());
    let child = cmd.spawn().map_err(|e| anyhow!("ffmpeg: {}", e))?;
    *ERROR.lock().unwrap_or_else(|e| e.into_inner()) = None;
    *SESSION.lock().unwrap_or_else(|e| e.into_inner()) = Some(Session { child, started: Instant::now(), output, ring_dir, opts });
    // Se o ffmpeg morrer nos primeiros instantes (permissão, dispositivo), avisa.
    tokio::time::sleep(std::time::Duration::from_millis(900)).await;
    let died = {
        let mut s = SESSION.lock().unwrap_or_else(|e| e.into_inner());
        match s.as_mut() {
            Some(sess) => match sess.child.try_wait() {
                Ok(Some(_)) => Some(sess.child.stderr.take()),
                _ => None,
            },
            None => None,
        }
    };
    if let Some(stderr) = died {
        let mut msg = String::new();
        if let Some(mut e) = stderr {
            use tokio::io::AsyncReadExt;
            let _ = e.read_to_string(&mut msg).await;
        }
        *SESSION.lock().unwrap_or_else(|e| e.into_inner()) = None;
        let msg = if msg.trim().is_empty() { "ffmpeg encerrou na largada (permissao de Gravacao de Tela?)".to_string() } else { msg.trim().to_string() };
        *ERROR.lock().unwrap_or_else(|e| e.into_inner()) = Some(msg.clone());
        return Err(anyhow!(msg));
    }
    Ok(state())
}

async fn stop_child(mut child: tokio::process::Child) {
    if let Some(mut stdin) = child.stdin.take() {
        use tokio::io::AsyncWriteExt;
        let _ = stdin.write_all(b"q\n").await;
        let _ = stdin.flush().await;
    }
    if tokio::time::timeout(std::time::Duration::from_secs(10), child.wait()).await.is_err() {
        let _ = child.kill().await;
    }
}

/// Para a gravação. Normal: devolve o arquivo. Replay: descarta o anel.
pub async fn stop() -> anyhow::Result<RecordState> {
    let sess = SESSION.lock().unwrap_or_else(|e| e.into_inner()).take();
    let Some(sess) = sess else { return Err(anyhow!("nao esta gravando")) };
    stop_child(sess.child).await;
    if let Some(out) = &sess.output {
        *LAST_SAVED.lock().unwrap_or_else(|e| e.into_inner()) = Some(out.to_string_lossy().to_string());
    }
    if let Some(r) = &sess.ring_dir {
        let _ = std::fs::remove_dir_all(r);
    }
    Ok(state())
}

/// Replay: junta os últimos segmentos num arquivo na pasta de saída, sem
/// interromper a gravação do anel.
pub async fn save_replay() -> anyhow::Result<String> {
    let (ring, opts) = {
        let s = SESSION.lock().unwrap_or_else(|e| e.into_inner());
        let Some(sess) = s.as_ref() else { return Err(anyhow!("o replay nao esta ligado")) };
        let Some(r) = sess.ring_dir.clone() else { return Err(anyhow!("a gravacao atual nao e um replay")) };
        (r, sess.opts.clone())
    };
    let ffmpeg = crate::core::dependencies::ensure_ffmpeg().await?;
    let mut segs: Vec<(std::time::SystemTime, PathBuf)> = std::fs::read_dir(&ring)?
        .flatten()
        .filter_map(|e| {
            let p = e.path();
            let m = e.metadata().ok()?;
            if p.extension().map(|x| x == "mp4").unwrap_or(false) && m.len() > 0 {
                Some((m.modified().ok()?, p))
            } else {
                None
            }
        })
        .collect();
    segs.sort();
    // O último segmento ainda está sendo escrito; pega os anteriores.
    if segs.len() > 1 {
        segs.pop();
    }
    if segs.is_empty() {
        return Err(anyhow!("ainda nao ha nada no buffer"));
    }
    let want = (opts.replay_seconds.max(10) / 5) as usize;
    let chosen: Vec<PathBuf> = segs.iter().rev().take(want).map(|(_, p)| p.clone()).collect::<Vec<_>>().into_iter().rev().collect();
    let list = ring.join("concat.txt");
    std::fs::write(&list, chosen.iter().map(|p| format!("file '{}'", p.to_string_lossy().replace('\'', "'\\''"))).collect::<Vec<_>>().join("\n"))?;
    let dir = if opts.output_dir.trim().is_empty() { videos_dir() } else { PathBuf::from(opts.output_dir.trim()) };
    std::fs::create_dir_all(&dir)?;
    let out = dir.join(format!("Replay {}.mp4", stamp()));
    let o = crate::core::process::command(&ffmpeg)
        .args(["-y", "-hide_banner", "-loglevel", "error", "-f", "concat", "-safe", "0", "-i"])
        .arg(&list)
        .args(["-c", "copy", "-movflags", "+faststart"])
        .arg(&out)
        .output()
        .await?;
    if !o.status.success() {
        return Err(anyhow!("concat falhou: {}", String::from_utf8_lossy(&o.stderr).trim()));
    }
    let s = out.to_string_lossy().to_string();
    *LAST_SAVED.lock().unwrap_or_else(|e| e.into_inner()) = Some(s.clone());
    Ok(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn args_shape() {
        let o: RecordOptions = serde_json::from_str(r#"{"screen":"","fps":30,"replay_seconds":30}"#).unwrap();
        let a = build_args(&o, Path::new("/tmp/x"), true);
        assert!(a.iter().any(|s| s == "segment"));
        assert!(a.last().unwrap().ends_with("seg-%03d.mp4"));
        let b = build_args(&o, Path::new("/tmp/out.mp4"), false);
        assert!(b.contains(&"libx264".to_string()));
        assert_eq!(b.last().unwrap(), "/tmp/out.mp4");
        assert!(!state().running);
    }
}

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::SampleFormat;
use serde::Serialize;

use omniget_core::core::meetings::Track;

#[derive(Debug, Clone, Serialize)]
pub struct AudioDevice {
    pub id: String,
    pub name: String,
    pub default: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct CaptureSupport {
    pub inputs: Vec<AudioDevice>,
    pub system_audio: bool,
    pub system_audio_note: String,
}

pub fn support() -> CaptureSupport {
    let host = cpal::default_host();
    let default_in = host
        .default_input_device()
        .and_then(|d| d.id().ok())
        .map(|i| i.to_string());
    let inputs = host
        .input_devices()
        .map(|it| {
            it.filter_map(|d| {
                let id = d.id().ok()?.to_string();
                let name = d
                    .description()
                    .map(|x| x.name().to_string())
                    .unwrap_or_else(|_| id.clone());
                Some(AudioDevice {
                    default: default_in.as_deref() == Some(id.as_str()),
                    id,
                    name,
                })
            })
            .collect()
        })
        .unwrap_or_default();
    let (system_audio, note) = if cfg!(target_os = "macos") {
        (
            true,
            "macOS 14.6 or newer; the first recording asks for the System Audio Recording permission",
        )
    } else if cfg!(target_os = "linux") {
        (
            true,
            "Records the PulseAudio/PipeWire monitor of the default output",
        )
    } else {
        (cfg!(windows), "")
    };
    CaptureSupport {
        inputs,
        system_audio,
        system_audio_note: note.to_string(),
    }
}

struct TrackRunner {
    role: &'static str,
    file: String,
    level: Arc<AtomicU32>,
    handle: std::thread::JoinHandle<Result<(), String>>,
}

struct Active {
    meeting_id: String,
    started: Instant,
    stop: Arc<AtomicBool>,
    tracks: Vec<TrackRunner>,
}

static ACTIVE: Mutex<Option<Active>> = Mutex::new(None);

#[derive(Debug, Clone, Serialize)]
pub struct CaptureStatus {
    pub meeting_id: String,
    pub elapsed_ms: u64,
    pub mic_level: Option<f32>,
    pub system_level: Option<f32>,
}

pub fn status() -> Option<CaptureStatus> {
    let g = ACTIVE.lock().unwrap_or_else(|e| e.into_inner());
    let a = g.as_ref()?;
    let level = |role: &str| {
        a.tracks
            .iter()
            .find(|t| t.role == role)
            .map(|t| f32::from_bits(t.level.load(Ordering::Relaxed)))
    };
    Some(CaptureStatus {
        meeting_id: a.meeting_id.clone(),
        elapsed_ms: a.started.elapsed().as_millis() as u64,
        mic_level: level("mic"),
        system_level: level("system"),
    })
}

pub fn is_recording() -> bool {
    ACTIVE.lock().unwrap_or_else(|e| e.into_inner()).is_some()
}

pub fn start(
    meeting_id: &str,
    dir: &Path,
    mic: bool,
    system: bool,
    mic_device: Option<String>,
) -> Result<Vec<String>, String> {
    let mut g = ACTIVE.lock().unwrap_or_else(|e| e.into_inner());
    if g.is_some() {
        return Err("A meeting is already being recorded".into());
    }
    if !mic && !system {
        return Err("Pick the microphone, the system audio or both".into());
    }
    let stop = Arc::new(AtomicBool::new(false));
    let mut tracks = Vec::new();
    let mut warnings = Vec::new();
    if mic {
        match spawn_track("mic", dir.join("mic.wav"), stop.clone(), mic_device) {
            Ok(t) => tracks.push(t),
            Err(e) => warnings.push(format!("Microphone: {e}")),
        }
    }
    if system {
        let spawned = if cfg!(target_os = "linux") {
            spawn_ffmpeg_monitor(dir.join("system.wav"), stop.clone())
        } else {
            spawn_track("system", dir.join("system.wav"), stop.clone(), None)
        };
        match spawned {
            Ok(t) => tracks.push(t),
            Err(e) => warnings.push(format!("System audio: {e}")),
        }
    }
    if tracks.is_empty() {
        return Err(warnings.join("; "));
    }
    *g = Some(Active {
        meeting_id: meeting_id.to_string(),
        started: Instant::now(),
        stop,
        tracks,
    });
    Ok(warnings)
}

pub struct Stopped {
    pub meeting_id: String,
    pub duration_ms: u64,
    pub tracks: Vec<Track>,
    pub warnings: Vec<String>,
}

pub fn stop() -> Result<Stopped, String> {
    let active = ACTIVE
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .take()
        .ok_or("Nothing is being recorded")?;
    active.stop.store(true, Ordering::SeqCst);
    let duration_ms = active.started.elapsed().as_millis() as u64;
    let mut tracks = Vec::new();
    let mut warnings = Vec::new();
    for t in active.tracks {
        match t.handle.join() {
            Ok(Ok(())) => tracks.push(Track {
                role: t.role.to_string(),
                file: t.file,
            }),
            Ok(Err(e)) => warnings.push(format!("{}: {e}", t.role)),
            Err(_) => warnings.push(format!("{}: the recorder thread crashed", t.role)),
        }
    }
    Ok(Stopped {
        meeting_id: active.meeting_id,
        duration_ms,
        tracks,
        warnings,
    })
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default()
}

fn find_input(id: Option<&str>) -> Option<cpal::Device> {
    let host = cpal::default_host();
    let wanted = id.map(str::trim).filter(|s| !s.is_empty());
    match wanted {
        None => host.default_input_device(),
        Some(w) => host
            .input_devices()
            .ok()
            .and_then(|mut it| it.find(|d| d.id().map(|i| i.to_string() == w).unwrap_or(false)))
            .or_else(|| host.default_input_device()),
    }
}

fn spawn_track(
    role: &'static str,
    path: PathBuf,
    stop: Arc<AtomicBool>,
    device_id: Option<String>,
) -> Result<TrackRunner, String> {
    if role == "mic" {
        ensure_microphone_access()?;
    }
    let level = Arc::new(AtomicU32::new(0));
    let level_cb = level.clone();
    let (ready_tx, ready_rx) = mpsc::channel::<Result<(), String>>();
    let file = file_name(&path);
    let handle = std::thread::Builder::new()
        .name(format!("meeting-{role}"))
        .spawn(move || -> Result<(), String> {
            let host = cpal::default_host();
            let device = if role == "mic" {
                find_input(device_id.as_deref())
            } else {
                host.default_output_device()
            };
            let Some(device) = device else {
                let e = "no audio device found".to_string();
                let _ = ready_tx.send(Err(e.clone()));
                return Err(e);
            };
            let supported = if role == "mic" {
                device.default_input_config()
            } else {
                device.default_output_config()
            };
            let supported = match supported {
                Ok(s) => s,
                Err(e) => {
                    let _ = ready_tx.send(Err(e.to_string()));
                    return Err(e.to_string());
                }
            };
            let config: cpal::StreamConfig = supported.config();
            let channels = config.channels.max(1) as usize;
            let sample_rate = config.sample_rate;
            let spec = hound::WavSpec {
                channels: 1,
                sample_rate,
                bits_per_sample: 16,
                sample_format: hound::SampleFormat::Int,
            };
            let mut writer = match hound::WavWriter::create(&path, spec) {
                Ok(w) => w,
                Err(e) => {
                    let _ = ready_tx.send(Err(e.to_string()));
                    return Err(e.to_string());
                }
            };
            let (tx, rx) = mpsc::channel::<Vec<f32>>();
            let err_cb = move |e| tracing::warn!("[meetings] {role} stream error: {e}");
            let built = match supported.sample_format() {
                SampleFormat::F32 => device.build_input_stream(
                    config,
                    callback::<f32>(tx, channels, level_cb, |s| s),
                    err_cb,
                    None,
                ),
                SampleFormat::I16 => device.build_input_stream(
                    config,
                    callback::<i16>(tx, channels, level_cb, |s| s as f32 / 32_768.0),
                    err_cb,
                    None,
                ),
                SampleFormat::I32 => device.build_input_stream(
                    config,
                    callback::<i32>(tx, channels, level_cb, |s| s as f32 / 2_147_483_648.0),
                    err_cb,
                    None,
                ),
                SampleFormat::U16 => device.build_input_stream(
                    config,
                    callback::<u16>(tx, channels, level_cb, |s| (s as f32 - 32_768.0) / 32_768.0),
                    err_cb,
                    None,
                ),
                other => {
                    let e = format!("unsupported sample format {other:?}");
                    let _ = ready_tx.send(Err(e.clone()));
                    return Err(e);
                }
            };
            let stream = match built {
                Ok(s) => s,
                Err(e) => {
                    let e = if role == "system" && cfg!(target_os = "macos") {
                        format!("{e} (needs macOS 14.6+ and the System Audio Recording permission)")
                    } else {
                        e.to_string()
                    };
                    let _ = ready_tx.send(Err(e.clone()));
                    return Err(e);
                }
            };
            if let Err(e) = stream.play() {
                let _ = ready_tx.send(Err(e.to_string()));
                return Err(e.to_string());
            }
            let _ = ready_tx.send(Ok(()));
            let write = |writer: &mut hound::WavWriter<_>, chunk: Vec<f32>| {
                for s in chunk {
                    let v = (s.clamp(-1.0, 1.0) * 32_767.0) as i16;
                    let _ = writer.write_sample(v);
                }
            };
            while !stop.load(Ordering::SeqCst) {
                if let Ok(chunk) = rx.recv_timeout(Duration::from_millis(200)) {
                    write(&mut writer, chunk);
                }
            }
            drop(stream);
            while let Ok(chunk) = rx.try_recv() {
                write(&mut writer, chunk);
            }
            writer.finalize().map_err(|e| e.to_string())
        })
        .map_err(|e| e.to_string())?;
    match ready_rx.recv_timeout(Duration::from_secs(10)) {
        Ok(Ok(())) => Ok(TrackRunner {
            role,
            file,
            level,
            handle,
        }),
        Ok(Err(e)) => {
            let _ = handle.join();
            Err(e)
        }
        Err(_) => Err("the audio device did not start in time".into()),
    }
}

fn callback<T: Copy + Send + 'static>(
    tx: mpsc::Sender<Vec<f32>>,
    channels: usize,
    level: Arc<AtomicU32>,
    convert: impl Fn(T) -> f32 + Send + 'static,
) -> impl FnMut(&[T], &cpal::InputCallbackInfo) + Send + 'static {
    move |data: &[T], _| {
        let mut mono = Vec::with_capacity(data.len() / channels + 1);
        let mut sum = 0.0f32;
        for frame in data.chunks(channels) {
            let s = frame.iter().map(|x| convert(*x)).sum::<f32>() / frame.len() as f32;
            sum += s * s;
            mono.push(s);
        }
        if !mono.is_empty() {
            let rms = (sum / mono.len() as f32).sqrt();
            level.store(rms.to_bits(), Ordering::Relaxed);
        }
        let _ = tx.send(mono);
    }
}

fn spawn_ffmpeg_monitor(path: PathBuf, stop: Arc<AtomicBool>) -> Result<TrackRunner, String> {
    let ffmpeg = tauri::async_runtime::block_on(omniget_core::core::dependencies::ensure_ffmpeg())
        .map_err(|e| e.to_string())?;
    let mut child = omniget_core::core::process::std_command(&ffmpeg)
        .args([
            "-y",
            "-hide_banner",
            "-loglevel",
            "error",
            "-f",
            "pulse",
            "-i",
            "@DEFAULT_MONITOR@",
            "-ac",
            "1",
            "-ar",
            "16000",
            "-c:a",
            "pcm_s16le",
        ])
        .arg(&path)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| format!("could not start ffmpeg: {e}"))?;
    std::thread::sleep(Duration::from_millis(600));
    if let Ok(Some(status)) = child.try_wait() {
        let mut err = String::new();
        if let Some(mut e) = child.stderr.take() {
            use std::io::Read;
            let _ = e.read_to_string(&mut err);
        }
        return Err(format!("ffmpeg exited ({status}): {}", err.trim()));
    }
    let file = file_name(&path);
    let handle = std::thread::Builder::new()
        .name("meeting-system".into())
        .spawn(move || -> Result<(), String> {
            while !stop.load(Ordering::SeqCst) {
                std::thread::sleep(Duration::from_millis(200));
            }
            if let Some(mut stdin) = child.stdin.take() {
                use std::io::Write;
                let _ = stdin.write_all(b"q\n");
            }
            let deadline = Instant::now() + Duration::from_secs(5);
            loop {
                match child.try_wait() {
                    Ok(Some(_)) => return Ok(()),
                    Ok(None) if Instant::now() < deadline => {
                        std::thread::sleep(Duration::from_millis(100))
                    }
                    _ => {
                        let _ = child.kill();
                        return Ok(());
                    }
                }
            }
        })
        .map_err(|e| e.to_string())?;
    Ok(TrackRunner {
        role: "system",
        file,
        level: Arc::new(AtomicU32::new(0)),
        handle,
    })
}

#[cfg(target_os = "macos")]
fn ensure_microphone_access() -> Result<(), String> {
    use objc2::msg_send;
    use objc2::runtime::{AnyClass, Bool};
    use objc2_foundation::NSString;

    let Some(cls) = AnyClass::get(c"AVCaptureDevice") else {
        return Ok(());
    };
    let media = NSString::from_str("soun");
    let status: isize = unsafe { msg_send![cls, authorizationStatusForMediaType: &*media] };
    match status {
        3 => Ok(()),
        1 | 2 => Err(
            "microphone access is denied in System Settings > Privacy & Security > Microphone"
                .into(),
        ),
        _ => {
            let (tx, rx) = mpsc::channel();
            let handler = block2::RcBlock::new(move |granted: Bool| {
                let _ = tx.send(granted.as_bool());
            });
            let _: () = unsafe {
                msg_send![cls, requestAccessForMediaType: &*media, completionHandler: &*handler]
            };
            match rx.recv_timeout(Duration::from_secs(60)) {
                Ok(true) => Ok(()),
                _ => Err("microphone access was not granted".into()),
            }
        }
    }
}

#[cfg(not(target_os = "macos"))]
fn ensure_microphone_access() -> Result<(), String> {
    Ok(())
}

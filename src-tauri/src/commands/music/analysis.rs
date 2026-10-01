use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

use once_cell::sync::Lazy;
use rusqlite::params;
use serde::Serialize;
use tauri::{AppHandle, Emitter};
use tokio::io::AsyncReadExt;

use omniget_core::core::music::{dsp, loudness};

use super::db;

const RATE: u32 = 44_100;
const LOW_RATE: u32 = 11_025;
const MAX_LOW_SECONDS: usize = 30 * 60;
pub const PEAK_BUCKETS: usize = 1800;

static QUEUE: Lazy<Mutex<VecDeque<i64>>> = Lazy::new(|| Mutex::new(VecDeque::new()));
static RUNNING: AtomicBool = AtomicBool::new(false);
static CANCEL: AtomicBool = AtomicBool::new(false);
static TOTAL: Lazy<Mutex<(usize, usize)>> = Lazy::new(|| Mutex::new((0, 0)));

#[derive(Debug, Clone, Serialize)]
pub struct AnalysisResult {
    pub track_id: i64,
    pub loudness: Option<loudness::Loudness>,
    pub tempo: Option<dsp::Tempo>,
    pub key: Option<dsp::MusicalKey>,
    pub cutoff: Option<dsp::Cutoff>,
    pub bounds: Option<dsp::Bounds>,
    pub outro_s: Option<f32>,
    pub peaks: Vec<u8>,
}

pub async fn analyze_file(
    path: &str,
    duration_ms: i64,
    peaks_only: bool,
) -> Result<AnalysisResult, String> {
    let mut cmd = omniget_core::core::process::command("ffmpeg");
    cmd.args([
        "-hide_banner",
        "-nostdin",
        "-loglevel",
        "error",
        "-i",
        path,
        "-map",
        "0:a:0",
        "-vn",
    ]);
    let (channels, rate) = if peaks_only {
        (1u32, LOW_RATE)
    } else {
        (2u32, RATE)
    };
    cmd.args([
        "-ac",
        &channels.to_string(),
        "-ar",
        &rate.to_string(),
        "-f",
        "f32le",
        "pipe:1",
    ])
    .stdin(std::process::Stdio::null())
    .stdout(std::process::Stdio::piped())
    .stderr(std::process::Stdio::null())
    .kill_on_drop(true);
    let mut child = cmd.spawn().map_err(|e| format!("ffmpeg: {e}"))?;
    let mut out = child.stdout.take().ok_or("ffmpeg: no stdout")?;

    let expected = ((duration_ms.max(1) as f64 / 1000.0) * rate as f64) as usize;
    let mut meter = if peaks_only {
        None
    } else {
        loudness::Meter::new(2, RATE)
    };
    let mut cutoff = if peaks_only {
        None
    } else {
        Some(dsp::CutoffAccumulator::new(RATE, expected))
    };
    let mut low: Vec<f32> =
        Vec::with_capacity((expected / 4).min(MAX_LOW_SECONDS * LOW_RATE as usize));
    let decim = (rate / LOW_RATE) as usize;
    let mut acc = 0f32;
    let mut acc_n = 0usize;

    let mut buf = vec![0u8; 1 << 16];
    let mut carry: Vec<u8> = Vec::new();
    let mut frames: Vec<f32> = Vec::with_capacity(1 << 14);
    let mut mono: Vec<f32> = Vec::with_capacity(1 << 13);
    loop {
        if CANCEL.load(Ordering::SeqCst) {
            return Err("cancelled".into());
        }
        let n = out.read(&mut buf).await.map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        carry.extend_from_slice(&buf[..n]);
        let usable = carry.len() - carry.len() % (4 * channels as usize);
        frames.clear();
        frames.extend(
            carry[..usable]
                .chunks_exact(4)
                .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]])),
        );
        carry.drain(..usable);

        if let Some(m) = meter.as_mut() {
            m.push(&frames);
        }
        mono.clear();
        if channels == 2 {
            mono.extend(frames.chunks_exact(2).map(|f| (f[0] + f[1]) * 0.5));
        } else {
            mono.extend_from_slice(&frames);
        }
        if let Some(c) = cutoff.as_mut() {
            c.push(&mono);
        }
        for &v in &mono {
            acc += v;
            acc_n += 1;
            if acc_n == decim {
                if low.len() < MAX_LOW_SECONDS * LOW_RATE as usize {
                    low.push(acc / decim as f32);
                }
                acc = 0.0;
                acc_n = 0;
            }
        }
    }
    let _ = child.wait().await;
    if low.is_empty() {
        return Err("no audio decoded".into());
    }

    let peaks = dsp::peaks(&low, PEAK_BUCKETS);
    if peaks_only {
        return Ok(AnalysisResult {
            track_id: 0,
            loudness: None,
            tempo: None,
            key: None,
            cutoff: None,
            bounds: None,
            outro_s: None,
            peaks,
        });
    }
    let (tempo, key, bounds, outro) = tokio::task::spawn_blocking(move || {
        (
            dsp::bpm(&low, LOW_RATE),
            dsp::key(&low, LOW_RATE),
            dsp::silence_bounds(&low, LOW_RATE, -50.0),
            dsp::outro_start(&low, LOW_RATE),
        )
    })
    .await
    .map_err(|e| e.to_string())?;
    Ok(AnalysisResult {
        track_id: 0,
        loudness: meter.as_ref().and_then(|m| m.result()),
        tempo,
        key,
        cutoff: cutoff.and_then(|c| c.finish()),
        bounds,
        outro_s: outro,
        peaks,
    })
}

pub fn store_peaks(id: i64, peaks: &[u8]) -> Result<(), String> {
    db::with(|c| {
        c.execute(
            "INSERT INTO peaks (track_id, data) VALUES (?1, ?2)
             ON CONFLICT(track_id) DO UPDATE SET data = excluded.data",
            params![id, peaks],
        )
    })
    .map(|_| ())
}

pub fn load_peaks(id: i64) -> Option<Vec<u8>> {
    db::with(|c| {
        use rusqlite::OptionalExtension;
        c.query_row("SELECT data FROM peaks WHERE track_id = ?1", [id], |r| {
            r.get(0)
        })
        .optional()
    })
    .ok()
    .flatten()
}

fn store(r: &AnalysisResult) -> Result<(), String> {
    store_peaks(r.track_id, &r.peaks)?;
    db::with(|c| {
        c.execute(
            "UPDATE tracks SET
                loudness_lufs = ?2, loudness_range = ?3,
                rg_track_gain = COALESCE(?4, rg_track_gain), rg_track_peak = COALESCE(?5, rg_track_peak),
                bpm = COALESCE(?6, bpm),
                musical_key = COALESCE(?7, musical_key), camelot = COALESCE(?8, camelot),
                cutoff_hz = ?9, intro_s = ?10, end_s = ?11, outro_s = ?12, analyzed_at = ?13
             WHERE id = ?1",
            params![
                r.track_id,
                r.loudness.map(|l| l.integrated_lufs),
                r.loudness.map(|l| l.range_lu),
                r.loudness.map(|l| l.gain_db),
                r.loudness.map(|l| l.peak),
                r.tempo.filter(|t| t.confidence > 0.05).map(|t| t.bpm as f64),
                r.key.as_ref().map(|k| k.name.clone()),
                r.key.as_ref().map(|k| k.camelot.clone()),
                r.cutoff.map(|c| c.hz as f64),
                r.bounds.map(|b| b.start_s as f64),
                r.bounds.map(|b| b.end_s as f64),
                r.outro_s.map(|o| o as f64),
                db::now_ms(),
            ],
        )
    })
    .map(|_| ())
}

fn update_album_gains(ids: &[i64]) -> Result<(), String> {
    let tracks = db::tracks_by_ids(ids)?;
    let mut albums: std::collections::HashSet<(String, String)> = std::collections::HashSet::new();
    for t in &tracks {
        if !t.album.trim().is_empty() {
            let who = if t.album_artist.is_empty() {
                format!("dir:{}", t.folder)
            } else {
                t.album_artist.clone()
            };
            albums.insert((who, t.album.clone()));
        }
    }
    for (who, album) in albums {
        db::with(|c| {
            let mut st = c.prepare(
                "SELECT id, loudness_lufs, rg_track_peak, duration_ms FROM tracks
                 WHERE album = ?1 AND (CASE WHEN album_artist = '' THEN 'dir:' || folder ELSE album_artist END) = ?2",
            )?;
            let rows: Vec<(i64, Option<f64>, Option<f64>, i64)> = st
                .query_map(params![album, who], |r| {
                    Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))
                })?
                .collect::<rusqlite::Result<_>>()?;
            if rows.iter().any(|r| r.1.is_none()) {
                return Ok(());
            }
            let input: Vec<(f64, f64, f64)> = rows
                .iter()
                .map(|r| {
                    (
                        r.1.unwrap_or(-18.0),
                        r.2.unwrap_or(0.0),
                        r.3 as f64 / 1000.0,
                    )
                })
                .collect();
            if let Some(a) = loudness::album_from_tracks(&input) {
                let mut up = c.prepare(
                    "UPDATE tracks SET rg_album_gain = ?2, rg_album_peak = ?3 WHERE id = ?1",
                )?;
                for r in &rows {
                    up.execute(params![r.0, a.gain_db, a.peak])?;
                }
            }
            Ok(())
        })?;
    }
    Ok(())
}

#[derive(Serialize, Clone)]
struct Progress {
    done: usize,
    total: usize,
    track_id: i64,
    error: Option<String>,
}

pub fn enqueue(app: AppHandle, ids: Vec<i64>) {
    {
        let mut q = QUEUE.lock().unwrap_or_else(|e| e.into_inner());
        let mut t = TOTAL.lock().unwrap_or_else(|e| e.into_inner());
        for id in ids {
            if !q.contains(&id) {
                q.push_back(id);
                t.1 += 1;
            }
        }
    }
    if RUNNING.swap(true, Ordering::SeqCst) {
        return;
    }
    CANCEL.store(false, Ordering::SeqCst);
    tauri::async_runtime::spawn(async move {
        let mut batch = Vec::new();
        loop {
            let next = QUEUE.lock().unwrap_or_else(|e| e.into_inner()).pop_front();
            let Some(id) = next else { break };
            let mut err = None;
            match db::track(id) {
                Ok(t) => match analyze_file(&t.path, t.duration_ms, false).await {
                    Ok(mut r) => {
                        r.track_id = id;
                        if let Err(e) = store(&r) {
                            err = Some(e);
                        }
                        batch.push(id);
                    }
                    Err(e) => err = Some(e),
                },
                Err(e) => err = Some(e),
            }
            let (done, total) = {
                let mut t = TOTAL.lock().unwrap_or_else(|e| e.into_inner());
                t.0 += 1;
                *t
            };
            let _ = app.emit(
                "music-analysis-progress",
                Progress {
                    done,
                    total,
                    track_id: id,
                    error: err,
                },
            );
            if CANCEL.load(Ordering::SeqCst) {
                QUEUE.lock().unwrap_or_else(|e| e.into_inner()).clear();
                break;
            }
        }
        let _ = update_album_gains(&batch);
        *TOTAL.lock().unwrap_or_else(|e| e.into_inner()) = (0, 0);
        RUNNING.store(false, Ordering::SeqCst);
        let _ = app.emit("music-analysis-done", batch.len());
    });
}

pub fn cancel() {
    CANCEL.store(true, Ordering::SeqCst);
}

pub fn progress() -> (usize, usize, bool) {
    let t = *TOTAL.lock().unwrap_or_else(|e| e.into_inner());
    (t.0, t.1, RUNNING.load(Ordering::SeqCst))
}

#[derive(Serialize)]
pub struct Spectrogram {
    pub cols: usize,
    pub rows: usize,
    pub data: String,
    pub nyquist: f32,
    pub cutoff_hz: Option<f32>,
}

pub async fn spectrogram(path: &str, cols: usize, rows: usize) -> Result<Spectrogram, String> {
    let mut cmd = omniget_core::core::process::command("ffmpeg");
    cmd.args([
        "-hide_banner",
        "-nostdin",
        "-loglevel",
        "error",
        "-i",
        path,
        "-map",
        "0:a:0",
        "-vn",
    ])
    .args([
        "-ac", "1", "-ar", "44100", "-t", "600", "-f", "f32le", "pipe:1",
    ])
    .stdin(std::process::Stdio::null())
    .stdout(std::process::Stdio::piped())
    .stderr(std::process::Stdio::null())
    .kill_on_drop(true);
    let child = cmd.spawn().map_err(|e| format!("ffmpeg: {e}"))?;
    let out = child.wait_with_output().await.map_err(|e| e.to_string())?;
    let samples: Vec<f32> = out
        .stdout
        .chunks_exact(4)
        .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
        .collect();
    let (img, cutoff) = tokio::task::spawn_blocking(move || {
        (
            dsp::spectrogram(&samples, cols, rows),
            dsp::spectral_cutoff(&samples, 44100),
        )
    })
    .await
    .map_err(|e| e.to_string())?;
    use base64::Engine;
    Ok(Spectrogram {
        cols,
        rows,
        data: base64::engine::general_purpose::STANDARD.encode(img),
        nyquist: 22050.0,
        cutoff_hz: cutoff.map(|c| c.hz),
    })
}

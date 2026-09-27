//! Audio analysis over decoded mono PCM (f32, -1..1).
//!
//! Everything here is pure so it can be tested without ffmpeg: the caller
//! decodes (see `analysis::decode_args`) and hands the samples over.
//!
//! - `peaks`: the waveform drawn under the seek bar.
//! - `bpm`: spectral-flux onset envelope + autocorrelation with a log-normal
//!   prior around 120 BPM (the approach librosa's `beat.tempo` uses).
//! - `key`: 12-bin chroma matched against the Krumhansl-Kessler profiles.
//! - `spectral_cutoff`: highest band with real energy, to flag lossless files
//!   that were made from a lossy source (a 16 kHz wall means a 128k MP3).
//! - `silence_bounds` / `outro_start`: where the music starts and where the
//!   fade-out begins, for gapless trimming and automatic crossfades.

use rustfft::{num_complex::Complex, FftPlanner};
use serde::{Deserialize, Serialize};

/// Max |sample| per bucket, 255 = full scale. Not normalized per track, so
/// a quiet master looks quiet (the UI can still stretch it).
pub fn peaks(samples: &[f32], buckets: usize) -> Vec<u8> {
    if samples.is_empty() || buckets == 0 {
        return Vec::new();
    }
    let per = (samples.len() as f64 / buckets as f64).max(1.0);
    let n = buckets.min(samples.len());
    (0..n)
        .map(|b| {
            let s = (b as f64 * per) as usize;
            let e = (((b + 1) as f64 * per) as usize).clamp(s + 1, samples.len());
            let m = samples[s..e].iter().fold(0f32, |acc, v| acc.max(v.abs()));
            (m.min(1.0) * 255.0).round() as u8
        })
        .collect()
}

fn hann(n: usize) -> Vec<f32> {
    (0..n)
        .map(|i| 0.5 - 0.5 * (2.0 * std::f32::consts::PI * i as f32 / n as f32).cos())
        .collect()
}

/// Magnitude spectra of successive frames.
fn stft(samples: &[f32], frame: usize, hop: usize, mut f: impl FnMut(&[f32])) {
    if samples.len() < frame {
        return;
    }
    let mut planner = FftPlanner::<f32>::new();
    let fft = planner.plan_fft_forward(frame);
    let win = hann(frame);
    let mut buf = vec![Complex::new(0f32, 0f32); frame];
    let mut mags = vec![0f32; frame / 2];
    let mut pos = 0;
    while pos + frame <= samples.len() {
        for i in 0..frame {
            buf[i] = Complex::new(samples[pos + i] * win[i], 0.0);
        }
        fft.process(&mut buf);
        for (i, m) in mags.iter_mut().enumerate() {
            *m = buf[i].norm();
        }
        f(&mags);
        pos += hop;
    }
}

/// Averages pairs of samples until the rate is at most `target`.
pub fn downsample(samples: &[f32], rate: u32, target: u32) -> (Vec<f32>, u32) {
    let mut s = samples.to_vec();
    let mut r = rate;
    while r / 2 >= target && s.len() > 1 {
        s = s
            .chunks(2)
            .map(|c| c.iter().sum::<f32>() / c.len() as f32)
            .collect();
        r /= 2;
    }
    (s, r)
}

/// Onset strength per frame (log spectral flux, half-wave rectified, with
/// the local mean removed).
fn onset_envelope(samples: &[f32], rate: u32) -> (Vec<f32>, f32) {
    let frame = 1024;
    let hop = 256;
    let mut prev: Option<Vec<f32>> = None;
    let mut env = Vec::new();
    // Ignore sub-bass rumble and the top octave: kicks and snares live below 8 kHz.
    let max_bin = ((8000.0 / rate as f32) * frame as f32) as usize;
    stft(samples, frame, hop, |m| {
        let cur: Vec<f32> = m[..max_bin.min(m.len())]
            .iter()
            .map(|v| (1.0 + 1000.0 * v).ln())
            .collect();
        let flux = match &prev {
            Some(p) => cur
                .iter()
                .zip(p)
                .map(|(c, p)| (c - p).max(0.0))
                .sum::<f32>(),
            None => 0.0,
        };
        env.push(flux);
        prev = Some(cur);
    });
    // subtract a moving average (~0.5 s) to keep only the pulses
    let fps = rate as f32 / hop as f32;
    let w = (fps * 0.5) as usize;
    let mut out = vec![0f32; env.len()];
    let mut acc = 0f32;
    for i in 0..env.len() {
        acc += env[i];
        if i > w {
            acc -= env[i - w - 1];
        }
        let n = (i.min(w) + 1) as f32;
        out[i] = (env[i] - acc / n).max(0.0);
    }
    (out, fps)
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Tempo {
    pub bpm: f32,
    /// 0..1, how much the winning lag stands out from the rest.
    pub confidence: f32,
}

pub fn bpm(samples: &[f32], rate: u32) -> Option<Tempo> {
    let (s, r) = downsample(samples, rate, 22050);
    // 3 minutes are plenty and keep long mixes fast.
    let s = &s[..s.len().min(r as usize * 180)];
    let (env, fps) = onset_envelope(s, r);
    if env.len() < (fps * 8.0) as usize {
        return None;
    }
    let min_lag = (fps * 60.0 / 200.0).floor() as usize;
    let max_lag = (fps * 60.0 / 55.0).ceil() as usize;
    let mean = env.iter().sum::<f32>() / env.len() as f32;
    let centered: Vec<f32> = env.iter().map(|v| v - mean).collect();
    let energy: f32 = centered.iter().map(|v| v * v).sum();
    if energy <= f32::EPSILON {
        return None;
    }
    // Autocorrelation up to 4x the slowest lag, for the comb below.
    let acf_len = (max_lag * 4 + 8).min(centered.len() - 1);
    let acf: Vec<f32> = (0..=acf_len)
        .map(|lag| {
            centered[..centered.len() - lag]
                .iter()
                .zip(&centered[lag..])
                .map(|(a, b)| a * b)
                .sum::<f32>()
                / energy
        })
        .collect();
    // aubio's shift-invariant comb: a true beat period also lines up at
    // 2x, 3x, 4x its lag, which damps half/double-tempo errors.
    let comb = |lag: usize| -> f32 {
        let mut sum = 0f32;
        for a in 1..=4usize {
            for b in 1..(2 * a) {
                let i = lag * a + b - 1;
                if i < acf.len() {
                    sum += acf[i].max(0.0) / (2 * a - 1) as f32;
                }
            }
        }
        sum
    };
    let mut scores = Vec::with_capacity(max_lag + 1);
    for lag in 0..=max_lag {
        if lag < min_lag {
            scores.push(0.0);
            continue;
        }
        let bpm = 60.0 * fps / lag as f32;
        // log-normal prior centred on 120 BPM, one octave wide
        let prior = (-0.5 * ((bpm / 120.0).log2() / 1.0).powi(2)).exp();
        scores.push(comb(lag) * prior);
    }
    let (best, &best_score) = scores
        .iter()
        .enumerate()
        .max_by(|a, b| a.1.partial_cmp(b.1).unwrap_or(std::cmp::Ordering::Equal))?;
    if best_score <= 0.0 || best == 0 {
        return None;
    }
    // parabolic interpolation around the peak for sub-frame precision
    let lag = if best > min_lag && best + 1 < scores.len() {
        let (a, b, c) = (scores[best - 1], scores[best], scores[best + 1]);
        let d = a - 2.0 * b + c;
        if d.abs() > f32::EPSILON {
            best as f32 + 0.5 * (a - c) / d
        } else {
            best as f32
        }
    } else {
        best as f32
    };
    let mut bpm = 60.0 * fps / lag;
    // fold into the range most music is written in
    while bpm < 70.0 {
        bpm *= 2.0;
    }
    while bpm > 180.0 {
        bpm /= 2.0;
    }
    let avg = scores[min_lag..].iter().sum::<f32>() / (scores.len() - min_lag) as f32;
    let confidence = ((best_score - avg) / best_score).clamp(0.0, 1.0);
    Some(Tempo {
        bpm: (bpm * 10.0).round() / 10.0,
        confidence,
    })
}

const NOTES: [&str; 12] = [
    "C", "C#", "D", "Eb", "E", "F", "F#", "G", "Ab", "A", "Bb", "B",
];
const MAJOR: [f32; 12] = [
    6.35, 2.23, 3.48, 2.33, 4.38, 4.09, 2.52, 5.19, 2.39, 3.66, 2.29, 2.88,
];
const MINOR: [f32; 12] = [
    6.33, 2.68, 3.52, 5.38, 2.60, 3.53, 2.54, 4.75, 3.98, 2.69, 3.34, 3.17,
];
/// Camelot wheel position for each tonic, major (B) and minor (A).
const CAMELOT_MAJOR: [u8; 12] = [8, 3, 10, 5, 12, 7, 2, 9, 4, 11, 6, 1];
const CAMELOT_MINOR: [u8; 12] = [5, 12, 7, 2, 9, 4, 11, 6, 1, 8, 3, 10];

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MusicalKey {
    /// "A minor", "Eb major".
    pub name: String,
    /// DJ notation: "8A", "5B".
    pub camelot: String,
    pub confidence: f32,
}

fn pearson(a: &[f32; 12], b: &[f32]) -> f32 {
    let ma = a.iter().sum::<f32>() / 12.0;
    let mb = b.iter().sum::<f32>() / 12.0;
    let (mut num, mut da, mut db) = (0.0, 0.0, 0.0);
    for i in 0..12 {
        let x = a[i] - ma;
        let y = b[i] - mb;
        num += x * y;
        da += x * x;
        db += y * y;
    }
    if da <= 0.0 || db <= 0.0 {
        0.0
    } else {
        num / (da.sqrt() * db.sqrt())
    }
}

pub fn chroma(samples: &[f32], rate: u32) -> [f32; 12] {
    let (s, r) = downsample(samples, rate, 11025);
    let frame = 4096;
    let mut chroma = [0f32; 12];
    let bin_hz = r as f32 / frame as f32;
    // map each FFT bin (55 Hz .. 2 kHz) to a pitch class once
    let classes: Vec<Option<usize>> = (0..frame / 2)
        .map(|i| {
            let f = i as f32 * bin_hz;
            if !(55.0..=2000.0).contains(&f) {
                return None;
            }
            let midi = 69.0 + 12.0 * (f / 440.0).log2();
            Some(((midi.round() as i32).rem_euclid(12)) as usize)
        })
        .collect();
    stft(&s, frame, frame / 2, |m| {
        for (i, c) in classes.iter().enumerate() {
            if let Some(c) = c {
                chroma[*c] += m[i];
            }
        }
    });
    chroma
}

pub fn key(samples: &[f32], rate: u32) -> Option<MusicalKey> {
    let c = chroma(samples, rate);
    if c.iter().all(|v| *v <= 0.0) {
        return None;
    }
    let mut results: Vec<(f32, usize, bool)> = Vec::with_capacity(24);
    for tonic in 0..12 {
        let rotated: Vec<f32> = (0..12).map(|i| c[(i + tonic) % 12]).collect();
        results.push((pearson(&MAJOR, &rotated), tonic, true));
        results.push((pearson(&MINOR, &rotated), tonic, false));
    }
    results.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
    let (score, tonic, major) = results[0];
    let second = results.get(1).map(|r| r.0).unwrap_or(0.0);
    let name = format!("{} {}", NOTES[tonic], if major { "major" } else { "minor" });
    let camelot = if major {
        format!("{}B", CAMELOT_MAJOR[tonic])
    } else {
        format!("{}A", CAMELOT_MINOR[tonic])
    };
    Some(MusicalKey {
        name,
        camelot,
        // the gap to the runner-up (often the relative major/minor) is the signal
        confidence: ((score - second) * 5.0).clamp(0.0, 1.0),
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Cutoff {
    pub hz: f32,
    /// The file's own Nyquist, so the UI can tell "full band" from "cut".
    pub nyquist: f32,
}

/// Highest frequency where the average spectrum is still within 50 dB of the
/// loudest band. Lossy encoders low-pass hard (MP3 128k ~16 kHz, 192k ~19 kHz,
/// 320k ~20 kHz), so a "lossless" file with a wall there was transcoded.
pub fn spectral_cutoff(samples: &[f32], rate: u32) -> Option<Cutoff> {
    // sample up to 60 frames spread over the track
    spectral_cutoff_hop(samples, rate, (samples.len() / 60).max(4096))
}

fn spectral_cutoff_hop(samples: &[f32], rate: u32, hop: usize) -> Option<Cutoff> {
    let frame = 4096;
    let mut sum = vec![0f64; frame / 2];
    let mut frames = 0usize;
    stft(samples, frame, hop, |m| {
        for (s, v) in sum.iter_mut().zip(m) {
            *s += (*v as f64) * (*v as f64);
        }
        frames += 1;
    });
    if frames == 0 {
        return None;
    }
    let db: Vec<f64> = sum
        .iter()
        .map(|p| 10.0 * (p / frames as f64 + 1e-20).log10())
        .collect();
    // smooth over ~100 Hz so single bins don't decide
    let bin_hz = rate as f64 / frame as f64;
    let w = ((100.0 / bin_hz) as usize).max(1);
    let smooth: Vec<f64> = (0..db.len())
        .map(|i| {
            let s = i.saturating_sub(w);
            let e = (i + w + 1).min(db.len());
            db[s..e].iter().sum::<f64>() / (e - s) as f64
        })
        .collect();
    let ref_level = smooth.iter().cloned().fold(f64::MIN, f64::max);
    // A lossy encoder's low-pass is a cliff: > 25 dB lost within ~500 Hz,
    // landing near the floor. Natural roll-off is gradual. Scan from the top
    // and take the highest cliff; with none, the file is full band.
    let span = ((500.0 / bin_hz) as usize).max(2);
    let mut cut = None;
    for i in (span..smooth.len().saturating_sub(span)).rev() {
        let below = smooth[i - span..i].iter().sum::<f64>() / span as f64;
        let above = smooth[i..i + span].iter().sum::<f64>() / span as f64;
        if below - above > 25.0 && above < ref_level - 40.0 && below > ref_level - 70.0 {
            cut = Some(i);
            break;
        }
    }
    let cut = cut.unwrap_or_else(|| {
        smooth
            .iter()
            .rposition(|v| *v > ref_level - 90.0)
            .unwrap_or(smooth.len() - 1)
    });
    Some(Cutoff {
        hz: (cut as f64 * bin_hz) as f32,
        nyquist: rate as f32 / 2.0,
    })
}

/// Streaming version of `spectral_cutoff` for full-rate audio that is too
/// long to keep in memory: feed every sample, it keeps one frame every `stride`.
pub struct CutoffAccumulator {
    rate: u32,
    stride: usize,
    pending: Vec<f32>,
    seen_frames: usize,
    kept: Vec<f32>,
}

impl CutoffAccumulator {
    const FRAME: usize = 4096;

    /// `expected_samples` spreads ~60 frames over the track.
    pub fn new(rate: u32, expected_samples: usize) -> Self {
        Self {
            rate,
            stride: (expected_samples / Self::FRAME / 60).max(1),
            pending: Vec::with_capacity(Self::FRAME),
            seen_frames: 0,
            kept: Vec::new(),
        }
    }

    pub fn push(&mut self, mono: &[f32]) {
        for &v in mono {
            self.pending.push(v);
            if self.pending.len() == Self::FRAME {
                if self.seen_frames.is_multiple_of(self.stride) {
                    self.kept.extend_from_slice(&self.pending);
                }
                self.seen_frames += 1;
                self.pending.clear();
            }
        }
    }

    pub fn finish(self) -> Option<Cutoff> {
        spectral_cutoff_frames(&self.kept, self.rate)
    }
}

/// Like `spectral_cutoff` but every consecutive 4096-sample frame is used.
fn spectral_cutoff_frames(frames: &[f32], rate: u32) -> Option<Cutoff> {
    if frames.len() < 4096 {
        return None;
    }
    spectral_cutoff_hop(frames, rate, 4096)
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Bounds {
    /// First audible sample, seconds.
    pub start_s: f32,
    /// Last audible sample, seconds.
    pub end_s: f32,
}

fn rms_blocks(samples: &[f32], block: usize) -> Vec<f32> {
    samples
        .chunks(block)
        .map(|c| (c.iter().map(|v| v * v).sum::<f32>() / c.len() as f32).sqrt())
        .collect()
}

fn db(v: f32) -> f32 {
    20.0 * (v.max(1e-9)).log10()
}

/// Leading and trailing silence under `threshold_db` (dBFS), in 10 ms blocks.
pub fn silence_bounds(samples: &[f32], rate: u32, threshold_db: f32) -> Option<Bounds> {
    let block = (rate as usize / 100).max(1);
    let rms = rms_blocks(samples, block);
    let first = rms.iter().position(|v| db(*v) > threshold_db)?;
    let last = rms.iter().rposition(|v| db(*v) > threshold_db)?;
    let secs = |b: usize| (b * block) as f32 / rate as f32;
    Some(Bounds {
        start_s: secs(first),
        end_s: secs(last + 1).min(samples.len() as f32 / rate as f32),
    })
}

/// Where the ending fade starts: the last moment the 1 s loudness is within
/// 9 dB of the track's typical (median) level. Used to begin automatic
/// crossfades before the tail gets quiet.
pub fn outro_start(samples: &[f32], rate: u32) -> Option<f32> {
    let block = rate as usize;
    let rms = rms_blocks(samples, block);
    if rms.len() < 20 {
        return None;
    }
    let mut sorted: Vec<f32> = rms.iter().map(|v| db(*v)).collect();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let median = sorted[sorted.len() / 2];
    let last_loud = rms.iter().rposition(|v| db(*v) > median - 9.0)?;
    Some(((last_loud + 1) * block) as f32 / rate as f32)
}

/// Grayscale spectrogram, `cols` x `rows`, row 0 = highest frequency.
pub fn spectrogram(samples: &[f32], cols: usize, rows: usize) -> Vec<u8> {
    let frame = 2048;
    if samples.len() < frame || cols == 0 || rows == 0 {
        return vec![0; cols * rows];
    }
    let hop = ((samples.len() - frame) / cols).max(1);
    let mut columns: Vec<Vec<f32>> = Vec::with_capacity(cols);
    stft(samples, frame, hop, |m| {
        if columns.len() >= cols {
            return;
        }
        let bins = m.len();
        let col: Vec<f32> = (0..rows)
            .map(|r| {
                let lo = r * bins / rows;
                let hi = ((r + 1) * bins / rows).max(lo + 1);
                let p = m[lo..hi].iter().fold(0f32, |a, v| a.max(*v));
                db(p / frame as f32)
            })
            .collect();
        columns.push(col);
    });
    let mut out = vec![0u8; cols * rows];
    for (x, col) in columns.iter().enumerate() {
        for (r, v) in col.iter().enumerate() {
            // -120 dB .. 0 dB -> 0..255
            let norm = ((v + 120.0) / 120.0).clamp(0.0, 1.0);
            let y = rows - 1 - r;
            out[y * cols + x] = (norm * 255.0) as u8;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn clicks(bpm: f32, rate: u32, secs: f32) -> Vec<f32> {
        let n = (rate as f32 * secs) as usize;
        let period = (60.0 / bpm * rate as f32) as usize;
        let mut s = vec![0f32; n];
        let mut i = 0;
        while i < n {
            for k in 0..200.min(n - i) {
                // short decaying noise burst
                let noise = ((k * 7919) % 97) as f32 / 97.0 - 0.5;
                s[i + k] = noise * (1.0 - k as f32 / 200.0);
            }
            i += period;
        }
        s
    }

    fn sine(freqs: &[f32], rate: u32, secs: f32) -> Vec<f32> {
        let n = (rate as f32 * secs) as usize;
        (0..n)
            .map(|i| {
                // f64 phase: f32 time loses precision and adds broadband noise
                let t = i as f64 / rate as f64;
                (freqs
                    .iter()
                    .map(|f| (2.0 * std::f64::consts::PI * *f as f64 * t).sin())
                    .sum::<f64>()
                    / freqs.len() as f64) as f32
            })
            .collect()
    }

    fn noise(rate: u32, secs: f32) -> Vec<f32> {
        let mut x: u32 = 12345;
        (0..(rate as f32 * secs) as usize)
            .map(|_| {
                x ^= x << 13;
                x ^= x >> 17;
                x ^= x << 5;
                (x as f32 / u32::MAX as f32) - 0.5
            })
            .collect()
    }

    #[test]
    fn full_band_noise_has_no_cutoff() {
        let c = spectral_cutoff(&noise(44100, 3.0), 44100).unwrap();
        assert!(c.hz > 21000.0, "{}", c.hz);
    }

    #[test]
    fn detects_tempo_of_a_click_track() {
        for target in [90.0f32, 128.0, 140.0] {
            let s = clicks(target, 22050, 30.0);
            let t = bpm(&s, 22050).expect("tempo");
            assert!(
                (t.bpm - target).abs() < 2.0,
                "wanted {target}, got {}",
                t.bpm
            );
        }
    }

    #[test]
    fn detects_a_minor_chord() {
        // A3, C4, E4 plus A2 in the bass
        let s = sine(&[110.0, 220.0, 261.63, 329.63], 22050, 8.0);
        let k = key(&s, 22050).expect("key");
        assert!(k.name == "A minor" || k.name == "C major", "{}", k.name);
    }

    #[test]
    fn detects_c_major_chord() {
        let s = sine(&[130.81, 261.63, 329.63, 392.0], 22050, 8.0);
        let k = key(&s, 22050).unwrap();
        assert_eq!(k.name, "C major");
        assert_eq!(k.camelot, "8B");
    }

    #[test]
    fn finds_lowpass_wall() {
        // tones up to 15 kHz only at 44.1 kHz
        let s = sine(&[440.0, 3000.0, 9000.0, 15000.0], 44100, 3.0);
        let c = spectral_cutoff(&s, 44100).unwrap();
        assert!(c.hz > 14000.0 && c.hz < 17000.0, "{}", c.hz);
        assert_eq!(c.nyquist, 22050.0);
    }

    #[test]
    fn streaming_cutoff_matches() {
        let s = sine(&[440.0, 3000.0, 9000.0, 15000.0], 44100, 6.0);
        let mut acc = CutoffAccumulator::new(44100, s.len());
        for c in s.chunks(1000) {
            acc.push(c);
        }
        let c = acc.finish().unwrap();
        assert!(c.hz > 14000.0 && c.hz < 17000.0, "{}", c.hz);
    }

    #[test]
    fn finds_silence_bounds() {
        let rate = 8000;
        let mut s = vec![0f32; rate as usize];
        s.extend(sine(&[440.0], rate, 2.0));
        s.extend(vec![0f32; rate as usize / 2]);
        let b = silence_bounds(&s, rate, -50.0).unwrap();
        assert!((b.start_s - 1.0).abs() < 0.02, "{}", b.start_s);
        assert!((b.end_s - 3.0).abs() < 0.02, "{}", b.end_s);
    }

    #[test]
    fn peaks_keep_absolute_level() {
        let s = vec![0.0, 0.25, -0.5, 0.1];
        assert_eq!(peaks(&s, 2), vec![64, 128]);
    }

    #[test]
    fn spectrogram_has_requested_size() {
        let s = sine(&[1000.0], 8000, 2.0);
        assert_eq!(spectrogram(&s, 64, 32).len(), 64 * 32);
    }
}

use rustfft::{num_complex::Complex, FftPlanner};
use serde::{Deserialize, Serialize};

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

fn onset_envelope(samples: &[f32], rate: u32) -> (Vec<f32>, f32) {
    let frame = 1024;
    let hop = 256;
    let mut prev: Option<Vec<f32>> = None;
    let mut env = Vec::new();
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
    pub confidence: f32,
}

pub fn bpm(samples: &[f32], rate: u32) -> Option<Tempo> {
    let (s, r) = downsample(samples, rate, 22050);
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
const CAMELOT_MAJOR: [u8; 12] = [8, 3, 10, 5, 12, 7, 2, 9, 4, 11, 6, 1];
const CAMELOT_MINOR: [u8; 12] = [5, 12, 7, 2, 9, 4, 11, 6, 1, 8, 3, 10];

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MusicalKey {
    pub name: String,
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
        confidence: ((score - second) * 5.0).clamp(0.0, 1.0),
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Cutoff {
    pub hz: f32,
    pub nyquist: f32,
}

pub fn spectral_cutoff(samples: &[f32], rate: u32) -> Option<Cutoff> {
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

pub struct CutoffAccumulator {
    rate: u32,
    stride: usize,
    pending: Vec<f32>,
    seen_frames: usize,
    kept: Vec<f32>,
}

impl CutoffAccumulator {
    const FRAME: usize = 4096;

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

fn spectral_cutoff_frames(frames: &[f32], rate: u32) -> Option<Cutoff> {
    if frames.len() < 4096 {
        return None;
    }
    spectral_cutoff_hop(frames, rate, 4096)
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Bounds {
    pub start_s: f32,
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
            let norm = ((v + 120.0) / 120.0).clamp(0.0, 1.0);
            let y = rows - 1 - r;
            out[y * cols + x] = (norm * 255.0) as u8;
        }
    }
    out
}

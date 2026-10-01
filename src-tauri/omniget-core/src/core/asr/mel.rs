use std::sync::{Arc, OnceLock};

use rustfft::num_complex::Complex32;
use rustfft::{Fft, FftPlanner};

pub const SAMPLE_RATE: usize = 16_000;
pub const N_FFT: usize = 400;
pub const HOP: usize = 160;
pub const N_MELS: usize = 80;
pub const WINDOW_SAMPLES: usize = SAMPLE_RATE * 30;
pub const WINDOW_FRAMES: usize = WINDOW_SAMPLES / HOP;

const N_BINS: usize = N_FFT / 2 + 1;

fn hz_to_mel(f: f64) -> f64 {
    let f_sp = 200.0 / 3.0;
    let min_log_hz = 1000.0;
    let min_log_mel = min_log_hz / f_sp;
    let logstep = (6.4f64).ln() / 27.0;
    if f >= min_log_hz {
        min_log_mel + (f / min_log_hz).ln() / logstep
    } else {
        f / f_sp
    }
}

fn mel_to_hz(m: f64) -> f64 {
    let f_sp = 200.0 / 3.0;
    let min_log_hz = 1000.0;
    let min_log_mel = min_log_hz / f_sp;
    let logstep = (6.4f64).ln() / 27.0;
    if m >= min_log_mel {
        min_log_hz * (logstep * (m - min_log_mel)).exp()
    } else {
        f_sp * m
    }
}

pub fn mel_filters() -> &'static [f32] {
    static F: OnceLock<Vec<f32>> = OnceLock::new();
    F.get_or_init(|| {
        let fmax = SAMPLE_RATE as f64 / 2.0;
        let (lo, hi) = (hz_to_mel(0.0), hz_to_mel(fmax));
        let mel_f: Vec<f64> = (0..N_MELS + 2)
            .map(|i| mel_to_hz(lo + (hi - lo) * i as f64 / (N_MELS + 1) as f64))
            .collect();
        let fft_freqs: Vec<f64> = (0..N_BINS)
            .map(|k| k as f64 * SAMPLE_RATE as f64 / N_FFT as f64)
            .collect();
        let mut w = vec![0f32; N_MELS * N_BINS];
        for i in 0..N_MELS {
            let fdiff_lo = mel_f[i + 1] - mel_f[i];
            let fdiff_hi = mel_f[i + 2] - mel_f[i + 1];
            let enorm = 2.0 / (mel_f[i + 2] - mel_f[i]);
            for (k, f) in fft_freqs.iter().enumerate() {
                let lower = (f - mel_f[i]) / fdiff_lo;
                let upper = (mel_f[i + 2] - f) / fdiff_hi;
                let v = lower.min(upper).max(0.0);
                w[i * N_BINS + k] = (v * enorm) as f32;
            }
        }
        w
    })
}

fn hann() -> &'static [f32] {
    static W: OnceLock<Vec<f32>> = OnceLock::new();
    W.get_or_init(|| {
        (0..N_FFT)
            .map(|n| {
                (0.5 - 0.5 * (2.0 * std::f64::consts::PI * n as f64 / N_FFT as f64).cos()) as f32
            })
            .collect()
    })
}

fn fft() -> Arc<dyn Fft<f32>> {
    static P: OnceLock<Arc<dyn Fft<f32>>> = OnceLock::new();
    P.get_or_init(|| FftPlanner::new().plan_fft_forward(N_FFT))
        .clone()
}

fn reflect(x: &[f32], i: isize) -> f32 {
    let n = x.len() as isize;
    if n == 1 {
        return x[0];
    }
    let mut i = i;
    while i < 0 || i >= n {
        if i < 0 {
            i = -i;
        }
        if i >= n {
            i = 2 * (n - 1) - i;
        }
    }
    x[i as usize]
}

pub fn log_mel_window(samples: &[f32]) -> Vec<f32> {
    let mut audio = vec![0f32; WINDOW_SAMPLES];
    let n = samples.len().min(WINDOW_SAMPLES);
    audio[..n].copy_from_slice(&samples[..n]);

    let window = hann();
    let filters = mel_filters();
    let fft = fft();
    let pad = (N_FFT / 2) as isize;
    let mut buf = vec![Complex32::new(0.0, 0.0); N_FFT];
    let mut scratch = vec![Complex32::new(0.0, 0.0); fft.get_inplace_scratch_len()];
    let mut power = vec![0f32; N_BINS];
    let mut mel = vec![0f32; N_MELS * WINDOW_FRAMES];

    for t in 0..WINDOW_FRAMES {
        let start = (t * HOP) as isize - pad;
        for (j, slot) in buf.iter_mut().enumerate() {
            let s = reflect(&audio, start + j as isize);
            *slot = Complex32::new(s * window[j], 0.0);
        }
        fft.process_with_scratch(&mut buf, &mut scratch);
        for (k, p) in power.iter_mut().enumerate() {
            *p = buf[k].norm_sqr();
        }
        for m in 0..N_MELS {
            let row = &filters[m * N_BINS..(m + 1) * N_BINS];
            let v: f32 = row.iter().zip(power.iter()).map(|(a, b)| a * b).sum();
            mel[m * WINDOW_FRAMES + t] = v.max(1e-10).log10();
        }
    }
    let max = mel.iter().copied().fold(f32::MIN, f32::max);
    let floor = max - 8.0;
    for v in mel.iter_mut() {
        *v = (v.max(floor) + 4.0) / 4.0;
    }
    mel
}

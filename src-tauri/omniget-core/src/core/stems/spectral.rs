use std::f32::consts::PI;
use std::sync::Arc;

use rustfft::num_complex::Complex32;
use rustfft::{Fft, FftPlanner};

pub const N_FFT: usize = 4096;
pub const HOP: usize = 1024;
pub const BINS: usize = N_FFT / 2 + 1;

pub fn hann(n: usize) -> Vec<f32> {
    (0..n)
        .map(|i| 0.5 - 0.5 * (2.0 * PI * i as f32 / n as f32).cos())
        .collect()
}

pub struct Stft {
    window: Vec<f32>,
    forward: Arc<dyn Fft<f32>>,
    inverse: Arc<dyn Fft<f32>>,
    buf: Vec<Complex32>,
    scratch: Vec<Complex32>,
}

impl Default for Stft {
    fn default() -> Self {
        Self::new()
    }
}

impl Stft {
    pub fn new() -> Self {
        let mut planner = FftPlanner::<f32>::new();
        let forward = planner.plan_fft_forward(N_FFT);
        let inverse = planner.plan_fft_inverse(N_FFT);
        let scratch_len = forward
            .get_inplace_scratch_len()
            .max(inverse.get_inplace_scratch_len());
        Self {
            window: hann(N_FFT),
            forward,
            inverse,
            buf: vec![Complex32::default(); N_FFT],
            scratch: vec![Complex32::default(); scratch_len],
        }
    }

    pub fn window(&self) -> &[f32] {
        &self.window
    }

    pub fn analyze(&mut self, frame: &[f32], out: &mut [Complex32]) {
        for (i, b) in self.buf.iter_mut().enumerate() {
            *b = Complex32::new(frame[i] * self.window[i], 0.0);
        }
        self.forward
            .process_with_scratch(&mut self.buf, &mut self.scratch);
        out[..BINS].copy_from_slice(&self.buf[..BINS]);
    }

    pub fn synthesize(&mut self, spec: &[Complex32], out: &mut [f32]) {
        self.buf[..BINS].copy_from_slice(&spec[..BINS]);
        for (k, v) in spec.iter().enumerate().take(N_FFT - BINS + 1).skip(1) {
            self.buf[N_FFT - k] = v.conj();
        }
        self.inverse
            .process_with_scratch(&mut self.buf, &mut self.scratch);
        let scale = 1.0 / N_FFT as f32;
        for ((o, b), w) in out.iter_mut().zip(&self.buf).zip(&self.window) {
            *o = b.re * scale * w;
        }
    }
}

pub struct OverlapAdd {
    acc: Vec<f32>,
    norm: Vec<f32>,
    win_sq: Vec<f32>,
}

impl OverlapAdd {
    pub fn new(window: &[f32]) -> Self {
        Self {
            acc: vec![0.0; N_FFT],
            norm: vec![0.0; N_FFT],
            win_sq: window.iter().map(|w| w * w).collect(),
        }
    }

    pub fn push(&mut self, frame: &[f32], out: &mut Vec<f32>) {
        for ((a, n), (f, w)) in self
            .acc
            .iter_mut()
            .zip(self.norm.iter_mut())
            .zip(frame.iter().zip(&self.win_sq))
        {
            *a += f;
            *n += w;
        }
        out.extend((0..HOP).map(|i| {
            let n = self.norm[i];
            if n > 1e-8 {
                self.acc[i] / n
            } else {
                0.0
            }
        }));
        self.acc.copy_within(HOP.., 0);
        self.norm.copy_within(HOP.., 0);
        self.acc[N_FFT - HOP..].fill(0.0);
        self.norm[N_FFT - HOP..].fill(0.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hann_is_periodic() {
        let w = hann(8);
        assert_eq!(w[0], 0.0);
        assert!((w[4] - 1.0).abs() < 1e-6);
        assert!((w[2] - w[6]).abs() < 1e-6);
    }

    #[test]
    fn analysis_then_synthesis_rebuilds_the_signal() {
        let mut stft = Stft::new();
        let mut ola = OverlapAdd::new(stft.window());
        let len = N_FFT * 6;
        let signal: Vec<f32> = (0..len)
            .map(|i| (i as f32 * 0.013).sin() * 0.6 + (i as f32 * 0.21).cos() * 0.2)
            .collect();
        let mut spec = vec![Complex32::default(); BINS];
        let mut frame = vec![0.0f32; N_FFT];
        let mut out = Vec::new();
        let mut start = 0;
        while start + N_FFT <= len {
            stft.analyze(&signal[start..start + N_FFT], &mut spec);
            stft.synthesize(&spec, &mut frame);
            ola.push(&frame, &mut out);
            start += HOP;
        }
        for i in N_FFT..out.len() {
            assert!(
                (out[i] - signal[i]).abs() < 1e-4,
                "sample {i}: {} vs {}",
                out[i],
                signal[i]
            );
        }
    }

    #[test]
    fn spectrum_of_a_sine_peaks_at_its_bin() {
        let mut stft = Stft::new();
        let bin = 93;
        let frame: Vec<f32> = (0..N_FFT)
            .map(|i| (2.0 * PI * bin as f32 * i as f32 / N_FFT as f32).sin())
            .collect();
        let mut spec = vec![Complex32::default(); BINS];
        stft.analyze(&frame, &mut spec);
        let peak = spec
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.norm().total_cmp(&b.1.norm()))
            .map(|(i, _)| i);
        assert_eq!(peak, Some(bin));
    }
}

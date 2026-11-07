use std::path::PathBuf;

use anyhow::{anyhow, Context};
use rustfft::num_complex::Complex32;

use super::models::ModelSpec;
use super::spectral::{OverlapAdd, Stft, BINS, HOP, N_FFT};

pub const SAMPLE_RATE: u32 = 44_100;
pub const CHANNELS: usize = 2;
pub const SEG_FRAMES: usize = 512;
pub const MODEL_BINS: usize = 1024;
pub const SEG_LEN: usize = CHANNELS * SEG_FRAMES * MODEL_BINS;

const EPS: f32 = 1e-10;

#[derive(Debug)]
pub struct Cancelled;

impl std::fmt::Display for Cancelled {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("cancelled")
    }
}

impl std::error::Error for Cancelled {}

pub trait Separator {
    fn stems(&self) -> Vec<String>;
    fn estimate(&mut self, mags: &[f32]) -> anyhow::Result<Vec<Vec<f32>>>;
}

pub trait PcmSource {
    fn read(&mut self, out: &mut [f32]) -> anyhow::Result<usize>;
}

pub trait PcmSink {
    fn write(&mut self, interleaved: &[f32]) -> anyhow::Result<()>;
}

pub struct Control<'a> {
    pub cancelled: &'a (dyn Fn() -> bool + Sync),
    pub progress: &'a (dyn Fn(u64) + Sync),
}

struct Input<'a> {
    src: &'a mut dyn PcmSource,
    ch: [Vec<f32>; CHANNELS],
    base: usize,
    carry: Option<f32>,
    real: usize,
    eof: bool,
}

impl<'a> Input<'a> {
    fn new(src: &'a mut dyn PcmSource) -> Self {
        Self {
            src,
            ch: [vec![0.0; N_FFT], vec![0.0; N_FFT]],
            base: 0,
            carry: None,
            real: 0,
            eof: false,
        }
    }

    fn end(&self) -> usize {
        self.base + self.ch[0].len()
    }

    fn fill_to(&mut self, padded_end: usize) -> anyhow::Result<()> {
        let mut buf = vec![0.0f32; 1 << 15];
        while !self.eof && self.end() < padded_end {
            let n = self.src.read(&mut buf)?;
            if n == 0 {
                self.eof = true;
                for c in &mut self.ch {
                    c.extend(std::iter::repeat_n(0.0, N_FFT));
                }
                break;
            }
            let mut vals = buf[..n].iter().copied();
            if let Some(l) = self.carry.take() {
                match vals.next() {
                    Some(r) => self.push(l, r),
                    None => self.carry = Some(l),
                }
            }
            loop {
                match (vals.next(), vals.next()) {
                    (Some(l), Some(r)) => self.push(l, r),
                    (Some(l), None) => {
                        self.carry = Some(l);
                        break;
                    }
                    _ => break,
                }
            }
        }
        Ok(())
    }

    fn push(&mut self, l: f32, r: f32) {
        self.ch[0].push(l);
        self.ch[1].push(r);
        self.real += 1;
    }

    fn frame(&self, c: usize, f: usize) -> &[f32] {
        let at = f * HOP - self.base;
        &self.ch[c][at..at + N_FFT]
    }

    fn drop_before(&mut self, padded: usize) {
        if padded > self.base {
            let n = (padded - self.base).min(self.ch[0].len());
            for c in &mut self.ch {
                c.drain(..n);
            }
            self.base += n;
        }
    }

    fn last_frame(&self) -> Option<usize> {
        (self.eof && self.real > 0).then(|| (N_FFT + self.real - 1) / HOP)
    }
}

pub fn masks(est: &[Vec<f32>]) -> Vec<Vec<f32>> {
    let n = est.len();
    let len = est.first().map(Vec::len).unwrap_or(0);
    let mut out = vec![vec![0.0f32; len]; n];
    for i in 0..len {
        let denom: f32 = est.iter().map(|e| e[i] * e[i]).sum::<f32>() + EPS;
        for s in 0..n {
            out[s][i] = (est[s][i] * est[s][i] + EPS / n as f32) / denom;
        }
    }
    out
}

pub fn run(
    src: &mut dyn PcmSource,
    sep: &mut dyn Separator,
    sinks: &mut [&mut dyn PcmSink],
    ctl: &Control<'_>,
) -> anyhow::Result<u64> {
    let stems = sep.stems().len();
    if sinks.len() != stems {
        return Err(anyhow!("{} outputs for {} stems", sinks.len(), stems));
    }
    let mut input = Input::new(src);
    let mut stft = Stft::new();
    let mut olas: Vec<[OverlapAdd; CHANNELS]> = (0..stems)
        .map(|_| {
            [
                OverlapAdd::new(stft.window()),
                OverlapAdd::new(stft.window()),
            ]
        })
        .collect();
    let mut pending: Vec<[Vec<f32>; CHANNELS]> = (0..stems).map(|_| [vec![], vec![]]).collect();
    let mut out_pos = 0usize;
    let mut emitted = 0usize;
    let mut next = 0usize;
    let mut spec = vec![Complex32::default(); CHANNELS * SEG_FRAMES * BINS];
    let mut mags = vec![0.0f32; SEG_LEN];
    let mut frame = vec![0.0f32; N_FFT];
    let mut masked = vec![Complex32::default(); BINS];
    let mut interleaved = Vec::new();

    loop {
        if (ctl.cancelled)() {
            return Err(Cancelled.into());
        }
        input.fill_to((next + SEG_FRAMES - 1) * HOP + N_FFT)?;
        let count = match input.last_frame() {
            Some(last) if next > last => break,
            Some(last) => (last + 1 - next).min(SEG_FRAMES),
            None if input.eof => return Err(anyhow!("this file has no audio")),
            None => SEG_FRAMES,
        };

        mags.fill(0.0);
        for c in 0..CHANNELS {
            for t in 0..count {
                let s = &mut spec[(c * SEG_FRAMES + t) * BINS..][..BINS];
                stft.analyze(input.frame(c, next + t), s);
                let m = &mut mags[(c * SEG_FRAMES + t) * MODEL_BINS..][..MODEL_BINS];
                for (k, v) in m.iter_mut().enumerate() {
                    *v = s[k].norm();
                }
            }
        }

        let est = sep.estimate(&mags)?;
        if est.len() != stems || est.iter().any(|e| e.len() != SEG_LEN) {
            return Err(anyhow!("the model returned an unexpected shape"));
        }
        let mk = masks(&est);

        for (s, mask) in mk.iter().enumerate() {
            for c in 0..CHANNELS {
                for t in 0..count {
                    let row = &mask[(c * SEG_FRAMES + t) * MODEL_BINS..][..MODEL_BINS];
                    let high = row.iter().sum::<f32>() / MODEL_BINS as f32;
                    let sp = &spec[(c * SEG_FRAMES + t) * BINS..][..BINS];
                    for k in 0..BINS {
                        let g = if k < MODEL_BINS { row[k] } else { high };
                        masked[k] = sp[k] * g;
                    }
                    stft.synthesize(&masked, &mut frame);
                    olas[s][c].push(&frame, &mut pending[s][c]);
                }
            }
        }
        next += count;
        input.drop_before(next * HOP);

        let ready = pending[0][0].len();
        let start = N_FFT.saturating_sub(out_pos).min(ready);
        let limit = if input.eof {
            (N_FFT + input.real).saturating_sub(out_pos).min(ready)
        } else {
            ready
        };
        let take = limit.saturating_sub(start);
        for (s, sink) in sinks.iter_mut().enumerate() {
            interleaved.clear();
            interleaved.reserve(take * CHANNELS);
            let [l, r] = &pending[s];
            for i in start..start + take {
                interleaved.push(l[i]);
                interleaved.push(r[i]);
            }
            sink.write(&interleaved)
                .with_context(|| format!("writing the {} track", sep.stems()[s]))?;
            for ch in &mut pending[s] {
                ch.clear();
            }
        }
        out_pos += ready;
        emitted += take;
        (ctl.progress)(emitted as u64);
    }
    Ok(emitted as u64)
}

pub struct OrtSeparator {
    names: Vec<String>,
    sessions: Vec<ort::session::Session>,
}

impl OrtSeparator {
    pub fn load(m: &ModelSpec) -> anyhow::Result<Self> {
        let files = m
            .files
            .iter()
            .map(|f| {
                super::models::file_path(m, f)
                    .map(|p| (f.stem.to_string(), p))
                    .ok_or_else(|| anyhow!("no app data directory"))
            })
            .collect::<anyhow::Result<Vec<_>>>()?;
        Self::from_files(&files)
    }

    pub fn from_files(files: &[(String, PathBuf)]) -> anyhow::Result<Self> {
        crate::core::onnxrt::init()?;
        let mut names = Vec::new();
        let mut sessions = Vec::new();
        for (stem, path) in files {
            if !path.is_file() {
                return Err(anyhow!("the {stem} model is missing at {}", path.display()));
            }
            sessions.push(crate::core::tools::onnx::session_from_file(path)?);
            names.push(stem.clone());
        }
        Ok(Self { names, sessions })
    }
}

impl Separator for OrtSeparator {
    fn stems(&self) -> Vec<String> {
        self.names.clone()
    }

    fn estimate(&mut self, mags: &[f32]) -> anyhow::Result<Vec<Vec<f32>>> {
        use ort::value::Tensor;
        let shape = vec![CHANNELS as i64, 1, SEG_FRAMES as i64, MODEL_BINS as i64];
        let mut out = Vec::with_capacity(self.sessions.len());
        for (name, session) in self.names.iter().zip(self.sessions.iter_mut()) {
            let x = Tensor::from_array((shape.clone(), mags.to_vec()))
                .with_context(|| format!("building the {name} input"))?;
            let res = session
                .run(ort::inputs![x])
                .with_context(|| format!("running the {name} model"))?;
            let (_, y) = res[0]
                .try_extract_tensor::<f32>()
                .with_context(|| format!("reading the {name} output"))?;
            out.push(y.to_vec());
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    pub struct VecSource {
        data: Vec<f32>,
        pos: usize,
        chunk: usize,
    }

    impl PcmSource for VecSource {
        fn read(&mut self, out: &mut [f32]) -> anyhow::Result<usize> {
            let n = out.len().min(self.chunk).min(self.data.len() - self.pos);
            out[..n].copy_from_slice(&self.data[self.pos..self.pos + n]);
            self.pos += n;
            Ok(n)
        }
    }

    #[derive(Default)]
    struct VecSink(Vec<f32>);

    impl PcmSink for VecSink {
        fn write(&mut self, interleaved: &[f32]) -> anyhow::Result<()> {
            self.0.extend_from_slice(interleaved);
            Ok(())
        }
    }

    struct SplitAt(usize);

    impl Separator for SplitAt {
        fn stems(&self) -> Vec<String> {
            vec!["low".into(), "high".into()]
        }

        fn estimate(&mut self, mags: &[f32]) -> anyhow::Result<Vec<Vec<f32>>> {
            let mut low = mags.to_vec();
            let mut high = mags.to_vec();
            for (i, (l, h)) in low.iter_mut().zip(high.iter_mut()).enumerate() {
                if i % MODEL_BINS < self.0 {
                    *h = 0.0;
                } else {
                    *l = 0.0;
                }
            }
            Ok(vec![low, high])
        }
    }

    fn tone(freq: f32, i: usize) -> f32 {
        (2.0 * std::f32::consts::PI * freq * i as f32 / SAMPLE_RATE as f32).sin()
    }

    fn mix(frames: usize) -> Vec<f32> {
        let mut seed = 0x1234_5678u32;
        let mut noise = move || {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            (seed as f32 / u32::MAX as f32 - 0.5) * 0.02
        };
        let mut v = Vec::with_capacity(frames * 2);
        for i in 0..frames {
            let low = 0.5 * tone(220.0, i);
            let high = 0.3 * tone(6000.0, i);
            v.push(low + high + noise());
            v.push(low - high + noise());
        }
        v
    }

    fn rms(v: &[f32]) -> f32 {
        (v.iter().map(|x| x * x).sum::<f32>() / v.len().max(1) as f32).sqrt()
    }

    fn run_on(data: Vec<f32>, chunk: usize) -> (u64, Vec<f32>, Vec<f32>, usize) {
        let mut src = VecSource {
            data,
            pos: 0,
            chunk,
        };
        let mut low = VecSink::default();
        let mut high = VecSink::default();
        let calls = std::sync::atomic::AtomicUsize::new(0);
        let progress = |_: u64| {
            calls.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        };
        let ctl = Control {
            cancelled: &|| false,
            progress: &progress,
        };
        let n = run(
            &mut src,
            &mut SplitAt(140),
            &mut [&mut low, &mut high],
            &ctl,
        )
        .unwrap();
        (n, low.0, high.0, calls.into_inner())
    }

    #[test]
    fn keeps_the_exact_length_and_splits_by_frequency() {
        let frames = SAMPLE_RATE as usize * 15 + 777;
        let input = mix(frames);
        let (n, low, high, calls) = run_on(input.clone(), 4097);
        assert_eq!(n as usize, frames);
        assert_eq!(low.len(), frames * 2);
        assert_eq!(high.len(), frames * 2);
        assert!(calls >= 2, "progress is reported per block");

        let sum_err = low
            .iter()
            .zip(&high)
            .zip(&input)
            .map(|((l, h), x)| (l + h - x).abs())
            .fold(0.0f32, f32::max);
        assert!(
            sum_err < 1e-3,
            "the stems add back up to the mix ({sum_err})"
        );

        let left_low: Vec<f32> = low.iter().step_by(2).copied().collect();
        let left_high: Vec<f32> = high.iter().step_by(2).copied().collect();
        let want_low: Vec<f32> = (0..frames).map(|i| 0.5 * tone(220.0, i)).collect();
        let mid = frames / 2..frames / 2 + 44_100;
        let diff: Vec<f32> = left_low[mid.clone()]
            .iter()
            .zip(&want_low[mid.clone()])
            .map(|(a, b)| a - b)
            .collect();
        assert!(rms(&diff) < 0.03, "low stem rms error {}", rms(&diff));
        assert!(rms(&left_high[mid]) > 0.15);
    }

    #[test]
    fn short_clips_and_odd_reads_work() {
        let (n, low, _, _) = run_on(mix(300), 1);
        assert_eq!(n, 300);
        assert_eq!(low.len(), 600);
    }

    #[test]
    fn empty_input_is_an_error() {
        let mut src = VecSource {
            data: vec![],
            pos: 0,
            chunk: 10,
        };
        let mut a = VecSink::default();
        let mut b = VecSink::default();
        let ctl = Control {
            cancelled: &|| false,
            progress: &|_| {},
        };
        assert!(run(&mut src, &mut SplitAt(10), &mut [&mut a, &mut b], &ctl).is_err());
    }

    #[test]
    fn cancel_stops_before_the_next_block() {
        let mut src = VecSource {
            data: mix(SAMPLE_RATE as usize * 30),
            pos: 0,
            chunk: 1 << 15,
        };
        let mut a = VecSink::default();
        let mut b = VecSink::default();
        let seen = std::sync::atomic::AtomicUsize::new(0);
        let cancelled = || seen.load(std::sync::atomic::Ordering::Relaxed) > 0;
        let progress = |_: u64| {
            seen.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        };
        let ctl = Control {
            cancelled: &cancelled,
            progress: &progress,
        };
        let err = run(&mut src, &mut SplitAt(10), &mut [&mut a, &mut b], &ctl).unwrap_err();
        assert!(err.downcast_ref::<Cancelled>().is_some());
        assert!(a.0.len() < SAMPLE_RATE as usize * 30 * 2);
    }

    #[test]
    fn masks_add_up_to_one() {
        let est = vec![vec![3.0, 0.0, 1.0], vec![4.0, 0.0, 1.0]];
        let m = masks(&est);
        for (a, b) in m[0].iter().zip(&m[1]) {
            assert!((a + b - 1.0).abs() < 1e-6);
        }
        assert!((m[0][0] - 9.0 / 25.0).abs() < 1e-6);
        assert!((m[0][1] - 0.5).abs() < 1e-6);
    }
}

use ebur128::{EbuR128, Mode};
use serde::{Deserialize, Serialize};

pub const REPLAYGAIN_REFERENCE_LUFS: f64 = -18.0;

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Loudness {
    pub integrated_lufs: f64,
    pub range_lu: f64,
    pub peak: f64,
    pub gain_db: f64,
}

pub fn gain_for(integrated_lufs: f64) -> f64 {
    ((REPLAYGAIN_REFERENCE_LUFS - integrated_lufs) * 100.0).round() / 100.0
}

pub struct Meter {
    inner: EbuR128,
    channels: u32,
}

impl Meter {
    pub fn new(channels: u32, rate: u32) -> Option<Self> {
        let inner = EbuR128::new(
            channels,
            rate,
            Mode::I | Mode::LRA | Mode::TRUE_PEAK | Mode::HISTOGRAM,
        )
        .ok()?;
        Some(Self { inner, channels })
    }

    pub fn push(&mut self, interleaved: &[f32]) {
        let usable = interleaved.len() - interleaved.len() % self.channels as usize;
        if usable > 0 {
            let _ = self.inner.add_frames_f32(&interleaved[..usable]);
        }
    }

    pub fn result(&self) -> Option<Loudness> {
        let integrated = self.inner.loudness_global().ok()?;
        if !integrated.is_finite() {
            return None;
        }
        let range = self.inner.loudness_range().unwrap_or(0.0);
        let peak = (0..self.channels)
            .filter_map(|c| self.inner.true_peak(c).ok())
            .fold(0f64, f64::max);
        Some(Loudness {
            integrated_lufs: (integrated * 100.0).round() / 100.0,
            range_lu: (range * 100.0).round() / 100.0,
            peak,
            gain_db: gain_for(integrated),
        })
    }

    pub fn state(&self) -> &EbuR128 {
        &self.inner
    }
}

pub fn album(meters: &[&Meter]) -> Option<Loudness> {
    let states: Vec<&EbuR128> = meters.iter().map(|m| m.state()).collect();
    let integrated = EbuR128::loudness_global_multiple(states.iter().copied()).ok()?;
    if !integrated.is_finite() {
        return None;
    }
    let range = EbuR128::loudness_range_multiple(states.iter().copied()).unwrap_or(0.0);
    let peak = meters
        .iter()
        .filter_map(|m| m.result().map(|r| r.peak))
        .fold(0f64, f64::max);
    Some(Loudness {
        integrated_lufs: (integrated * 100.0).round() / 100.0,
        range_lu: (range * 100.0).round() / 100.0,
        peak,
        gain_db: gain_for(integrated),
    })
}

pub fn album_from_tracks(tracks: &[(f64, f64, f64)]) -> Option<Loudness> {
    let total: f64 = tracks.iter().map(|t| t.2).sum();
    if total <= 0.0 {
        return None;
    }
    let energy: f64 = tracks
        .iter()
        .map(|(l, _, d)| d * 10f64.powf(l / 10.0))
        .sum::<f64>()
        / total;
    let integrated = 10.0 * energy.log10();
    let peak = tracks.iter().map(|t| t.1).fold(0f64, f64::max);
    Some(Loudness {
        integrated_lufs: (integrated * 100.0).round() / 100.0,
        range_lu: 0.0,
        peak,
        gain_db: gain_for(integrated),
    })
}

pub fn format_gain_tag(db: f64) -> String {
    format!("{:+.2} dB", db)
}

pub fn format_peak_tag(peak: f64) -> String {
    format!("{:.6}", peak)
}

pub fn parse_gain_tag(s: &str) -> Option<f64> {
    let t = s
        .trim()
        .trim_end_matches(|c: char| c.is_alphabetic())
        .trim();
    t.trim_start_matches('+').parse::<f64>().ok()
}

pub fn r128_q78_to_replaygain_db(q78: i32) -> f64 {
    q78 as f64 / 256.0 + 5.0
}

//! Frametime statistics, always computed by OmniGet whatever tool captured
//! the frames, so a Windows, Linux and Mac session read the same way.
//!
//! The formulas are the ones of CapFrameX (MIT, "Copyright (c) 2018-2026
//! DevTechProfile", github.com/CXWorld/CapFrameX), file
//! `source/CapFrameX.Statistics.NetStandard/FrametimeStatisticProvider.cs`:
//!
//! - average FPS = `n * 1000 / Σft` (`GetFpsMetricValue`, `EMetric.Average`);
//! - percentiles with MathNet's `Quantile`, definition R-8
//!   (`SortedArrayStatistics.Quantile`); FPS percentiles run over `1000/ft`;
//! - "x% low average" = `1000 / mean(ft ≥ Q(ft, 1 - x))`
//!   (`GetPercentageHighAverageSequence`);
//! - "x% low integral" (MSI Afterburner's definition): sort frametimes from
//!   the slowest, add them up until the sum reaches `x · total time`, the
//!   frametime where it crossed is the answer (`GetPercentageHighIntegralSequence`);
//! - stutter: `ft[i] > 2.5 · ma[i]`, `ma` the causal moving average of
//!   `SampleBasedMovingAverage.cs` with a window of
//!   `Convert.ToInt32(sqrt(mean ft) · 10)` frames, where a frame more than 3×
//!   its predecessor counts as the predecessor (`GetMovingAverage`,
//!   `GetStutteringTimePercentage`, factor 2.5 in `CapFrameXConfiguration.cs`);
//! - low-FPS time: frames that are not stutters and run under 25 FPS
//!   (`GetLowFPSTimePercentage`);
//! - frame-to-frame variance buckets `<2, <4, <8, <12, ≥12 ms`
//!   (`GetVariancePercentages`);
//! - freeze: a frame of 100 ms or more (`cfx_find_freezes` in the CapFrameX MCP).
//!
//! Pure functions over `&[f64]`; the golden vectors in the tests include
//! the cases of CapFrameX's own `FrametimeStatisticProviderTest.cs`.

use serde::{Deserialize, Serialize};

pub const STUTTER_FACTOR: f64 = 2.5;
pub const FREEZE_MS: f64 = 100.0;
pub const LOW_FPS_THRESHOLD: f64 = 25.0;

/// MathNet `Quantile` (R-8, approximately median-unbiased) over a sequence
/// that is sorted ascending. `None` for an empty sequence or `p` off [0, 1].
pub fn quantile_sorted(sorted: &[f64], p: f64) -> Option<f64> {
    if sorted.is_empty() || !(0.0..=1.0).contains(&p) {
        return None;
    }
    let n = sorted.len();
    let h = (n as f64 + 1.0 / 3.0) * p + 1.0 / 3.0;
    let hf = h.floor() as usize;
    Some(if hf < 1 {
        sorted[0]
    } else if hf >= n {
        sorted[n - 1]
    } else {
        sorted[hf - 1] + (h - hf as f64) * (sorted[hf] - sorted[hf - 1])
    })
}

pub fn quantile(seq: &[f64], p: f64) -> Option<f64> {
    let mut s = seq.to_vec();
    s.sort_by(f64::total_cmp);
    quantile_sorted(&s, p)
}

fn sum(seq: &[f64]) -> f64 {
    seq.iter().sum()
}

/// Average FPS: total frames over total time.
pub fn fps_avg(ft: &[f64]) -> Option<f64> {
    let s = sum(ft);
    (!ft.is_empty() && s > 0.0).then(|| ft.len() as f64 * 1000.0 / s)
}

/// Mean of the frametimes at or above the `p` quantile (p = 0.99 for 1%).
pub fn high_average(ft: &[f64], p: f64) -> Option<f64> {
    let q = quantile(ft, p)?;
    let slow: Vec<f64> = ft.iter().copied().filter(|x| *x >= q).collect();
    (!slow.is_empty()).then(|| sum(&slow) / slow.len() as f64)
}

/// The frametime where the slowest frames, added up from the slowest,
/// reach `(1 - p)` of the total time (p = 0.99 for 1%).
pub fn high_integral(ft: &[f64], p: f64) -> Option<f64> {
    if ft.is_empty() {
        return None;
    }
    let mut desc = ft.to_vec();
    desc.sort_by(|a, b| b.total_cmp(a));
    let target = sum(&desc) * (1.0 - p);
    let mut acc = 0.0;
    let mut idx = 0;
    for (i, x) in desc.iter().enumerate() {
        acc += x;
        idx = i;
        if acc >= target {
            break;
        }
    }
    Some(desc[idx])
}

/// CapFrameX's moving-average window: `Convert.ToInt32(sqrt(mean) * 10)`,
/// which rounds half to even.
pub fn moving_window(ft: &[f64]) -> usize {
    if ft.is_empty() {
        return 1;
    }
    let mean = sum(ft) / ft.len() as f64;
    ((mean.sqrt() * 10.0).round_ties_even() as usize).max(1)
}

/// Causal sample-based moving average (`SampleBasedMovingAverage.cs`).
pub fn moving_average(ft: &[f64], window: usize) -> Vec<f64> {
    let window = window.max(1);
    let mut out = Vec::with_capacity(ft.len());
    for i in 0..ft.len() {
        let mut k = i as isize;
        let mut acc = 0.0;
        let mut count = 0usize;
        while k >= 0 {
            let ku = k as usize;
            if ku > 0 && ft[ku] > ft[ku - 1] * 3.0 {
                acc += ft[ku - 1];
            } else {
                acc += ft[ku];
            }
            count += 1;
            if count >= window {
                break;
            }
            k -= 1;
        }
        out.push(acc / count as f64);
    }
    out
}

/// Indexes of the frames above `factor ×` their moving average.
pub fn stutter_indexes(ft: &[f64], factor: f64) -> Vec<usize> {
    let ma = moving_average(ft, moving_window(ft));
    (0..ft.len()).filter(|&i| ft[i] > factor * ma[i]).collect()
}

/// Share of the frame-to-frame changes in each bucket `<2, <4, <8, <12, ≥12 ms`.
pub fn variance_buckets(ft: &[f64]) -> [f64; 5] {
    let mut b = [0usize; 5];
    let mut n = 0usize;
    for w in ft.windows(2) {
        if !w[0].is_finite() || !w[1].is_finite() {
            continue;
        }
        let d = (w[1] - w[0]).abs();
        let i = if d < 2.0 {
            0
        } else if d < 4.0 {
            1
        } else if d < 8.0 {
            2
        } else if d < 12.0 {
            3
        } else {
            4
        };
        b[i] += 1;
        n += 1;
    }
    if n == 0 {
        return [0.0; 5];
    }
    b.map(|c| round(c as f64 / n as f64, 4))
}

pub fn round(v: f64, digits: i32) -> f64 {
    let m = 10f64.powi(digits);
    (v * m).round() / m
}

fn r2(v: Option<f64>) -> Option<f64> {
    v.filter(|x| x.is_finite()).map(|x| round(x, 2))
}

fn to_fps(ms: Option<f64>) -> Option<f64> {
    ms.filter(|x| *x > 0.0).map(|x| 1000.0 / x)
}

/// One frame event worth pointing at in the timeline.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Event {
    pub t_ms: f64,
    pub frametime_ms: f64,
}

/// Everything the summary shows for one frametime sequence. `None` means
/// the number cannot be computed (empty or sampled capture).
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Stats {
    pub frames: usize,
    pub duration_s: f64,
    pub fps_avg: Option<f64>,
    pub frametime_avg_ms: Option<f64>,
    pub fps_min: Option<f64>,
    pub fps_max: Option<f64>,
    pub fps_p99: Option<f64>,
    pub fps_p95: Option<f64>,
    pub fps_median: Option<f64>,
    pub fps_p5: Option<f64>,
    pub fps_p1: Option<f64>,
    pub fps_p01: Option<f64>,
    pub frametime_median_ms: Option<f64>,
    pub frametime_p99_ms: Option<f64>,
    pub frametime_p99_9_ms: Option<f64>,
    /// 1% low by average (CapFrameX's default "1% low").
    pub low_1: Option<f64>,
    pub low_01: Option<f64>,
    /// 1% low by integral (MSI Afterburner's definition).
    pub low_1_integral: Option<f64>,
    pub low_01_integral: Option<f64>,
    pub stutters: usize,
    pub stutter_time_pct: Option<f64>,
    pub low_fps_time_pct: Option<f64>,
    pub freezes: usize,
    pub freeze_time_ms: f64,
    /// Share of frame-to-frame changes `<2, <4, <8, <12, ≥12 ms`.
    pub variance: [f64; 5],
}

/// Full statistics of a frametime sequence (ms). `t_ms` (same length, or
/// empty) places stutters and freezes on the timeline.
pub fn compute(ft: &[f64], t_ms: &[f64]) -> (Stats, Vec<Event>, Vec<Event>) {
    let ft: Vec<f64> = ft
        .iter()
        .copied()
        .filter(|x| x.is_finite() && *x > 0.0)
        .collect();
    if ft.is_empty() {
        return (Stats::default(), vec![], vec![]);
    }
    let total = sum(&ft);
    let mut sorted = ft.clone();
    sorted.sort_by(f64::total_cmp);
    let mut fps_sorted: Vec<f64> = ft.iter().map(|x| 1000.0 / x).collect();
    fps_sorted.sort_by(f64::total_cmp);
    let fq = |p: f64| r2(quantile_sorted(&fps_sorted, p));

    let stutter_idx = stutter_indexes(&ft, STUTTER_FACTOR);
    let ma = moving_average(&ft, moving_window(&ft));
    let stutter_time: f64 = stutter_idx.iter().map(|&i| ft[i]).sum();
    let low_fps_time: f64 = (0..ft.len())
        .filter(|&i| ft[i] <= STUTTER_FACTOR * ma[i] && 1000.0 / ft[i] < LOW_FPS_THRESHOLD)
        .map(|i| ft[i])
        .sum();
    let at = |i: usize| -> f64 {
        t_ms.get(i)
            .copied()
            .unwrap_or_else(|| ft[..i].iter().sum::<f64>())
    };
    let stutters: Vec<Event> = stutter_idx
        .iter()
        .map(|&i| Event {
            t_ms: round(at(i), 1),
            frametime_ms: round(ft[i], 2),
        })
        .collect();
    let freezes: Vec<Event> = (0..ft.len())
        .filter(|&i| ft[i] >= FREEZE_MS)
        .map(|i| Event {
            t_ms: round(at(i), 1),
            frametime_ms: round(ft[i], 2),
        })
        .collect();

    let stats = Stats {
        frames: ft.len(),
        duration_s: round(total / 1000.0, 3),
        fps_avg: r2(fps_avg(&ft)),
        frametime_avg_ms: r2(Some(total / ft.len() as f64)),
        fps_min: r2(to_fps(sorted.last().copied())),
        fps_max: r2(to_fps(sorted.first().copied())),
        fps_p99: fq(0.99),
        fps_p95: fq(0.95),
        fps_median: fq(0.5),
        fps_p5: fq(0.05),
        fps_p1: fq(0.01),
        fps_p01: fq(0.001),
        frametime_median_ms: r2(quantile_sorted(&sorted, 0.5)),
        frametime_p99_ms: r2(quantile_sorted(&sorted, 0.99)),
        frametime_p99_9_ms: r2(quantile_sorted(&sorted, 0.999)),
        low_1: r2(to_fps(high_average(&ft, 0.99))),
        low_01: r2(to_fps(high_average(&ft, 0.999))),
        low_1_integral: r2(to_fps(high_integral(&ft, 0.99))),
        low_01_integral: r2(to_fps(high_integral(&ft, 0.999))),
        stutters: stutters.len(),
        stutter_time_pct: r2(Some(100.0 * stutter_time / total)),
        low_fps_time_pct: r2(Some(100.0 * low_fps_time / total)),
        freezes: freezes.len(),
        freeze_time_ms: round(freezes.iter().map(|e| e.frametime_ms).sum(), 2),
        variance: variance_buckets(&ft),
    };
    (stats, stutters, freezes)
}

/// Metrics compared between sessions, and whether higher is better.
pub const COMPARED: &[(&str, bool)] = &[
    ("fps_avg", true),
    ("fps_median", true),
    ("low_1", true),
    ("low_01", true),
    ("low_1_integral", true),
    ("fps_p1", true),
    ("frametime_avg_ms", false),
    ("frametime_p99_ms", false),
    ("stutter_time_pct", false),
    ("low_fps_time_pct", false),
];

pub fn metric(s: &Stats, name: &str) -> Option<f64> {
    match name {
        "fps_avg" => s.fps_avg,
        "fps_median" => s.fps_median,
        "low_1" => s.low_1,
        "low_01" => s.low_01,
        "low_1_integral" => s.low_1_integral,
        "fps_p1" => s.fps_p1,
        "frametime_avg_ms" => s.frametime_avg_ms,
        "frametime_p99_ms" => s.frametime_p99_ms,
        "stutter_time_pct" => s.stutter_time_pct,
        "low_fps_time_pct" => s.low_fps_time_pct,
        _ => None,
    }
}

/// How much each compared metric moves between the two halves of one
/// session, in percent: the run-to-run noise a before/after difference has
/// to beat. For percentage metrics (stutter time) the spread is in points.
pub fn half_spread(ft: &[f64]) -> Vec<(&'static str, Option<f64>)> {
    let mid = ft.len() / 2;
    if mid < 50 {
        return COMPARED.iter().map(|(m, _)| (*m, None)).collect();
    }
    let (a, _, _) = compute(&ft[..mid], &[]);
    let (b, _, _) = compute(&ft[mid..], &[]);
    COMPARED
        .iter()
        .map(|(m, _)| {
            let spread = match (metric(&a, m), metric(&b, m)) {
                (Some(x), Some(y)) if m.ends_with("_pct") => Some(round((x - y).abs(), 2)),
                (Some(x), Some(y)) if x.abs() + y.abs() > 0.0 => {
                    Some(round(200.0 * (x - y).abs() / (x.abs() + y.abs()), 2))
                }
                (Some(_), Some(_)) => Some(0.0),
                _ => None,
            };
            (*m, spread)
        })
        .collect()
}

/// One line of a before/after comparison.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Delta {
    pub metric: String,
    pub before: Option<f64>,
    pub after: Option<f64>,
    pub delta: Option<f64>,
    /// Percent change; for `_pct` metrics the change in points.
    pub delta_pct: Option<f64>,
    /// The noise the change had to beat (largest half-to-half spread of
    /// the two sessions, at least 1%).
    pub noise_pct: Option<f64>,
    /// `better`, `worse`, `same` (within noise) or `unknown`.
    pub verdict: String,
}

pub fn compare(
    before: &Stats,
    after: &Stats,
    noise_before: &[(&str, Option<f64>)],
    noise_after: &[(&str, Option<f64>)],
) -> Vec<Delta> {
    let noise = |list: &[(&str, Option<f64>)], m: &str| {
        list.iter().find(|(k, _)| *k == m).and_then(|(_, v)| *v)
    };
    COMPARED
        .iter()
        .map(|(m, higher_better)| {
            let (b, a) = (metric(before, m), metric(after, m));
            let points = m.ends_with("_pct");
            let delta = match (b, a) {
                (Some(b), Some(a)) => Some(round(a - b, 2)),
                _ => None,
            };
            let delta_pct = match (b, a) {
                (Some(b), Some(a)) if points => Some(round(a - b, 2)),
                (Some(b), Some(a)) if b.abs() > 0.0 => Some(round(100.0 * (a - b) / b.abs(), 2)),
                _ => None,
            };
            let floor = if points { 0.5 } else { 1.0 };
            let noise_pct = match (noise(noise_before, m), noise(noise_after, m)) {
                (Some(x), Some(y)) => Some(x.max(y).max(floor)),
                (Some(x), None) | (None, Some(x)) => Some(x.max(floor)),
                (None, None) => None,
            };
            let verdict = match (delta_pct, noise_pct) {
                (Some(d), n) if d.abs() <= n.unwrap_or(floor) => "same",
                (Some(d), _) if (d > 0.0) == *higher_better => "better",
                (Some(_), _) => "worse",
                (None, _) => "unknown",
            };
            Delta {
                metric: m.to_string(),
                before: b,
                after: a,
                delta,
                delta_pct,
                noise_pct,
                verdict: verdict.to_string(),
            }
        })
        .collect()
}

/// Frametime histogram for the screen: `bins` equal buckets from the
/// fastest frame to the 99.9th percentile, plus a last bucket for the rest.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Histogram {
    pub start_ms: f64,
    pub width_ms: f64,
    pub counts: Vec<usize>,
    /// Frames slower than the last regular bucket.
    pub overflow: usize,
}

pub fn histogram(ft: &[f64], bins: usize) -> Histogram {
    let clean: Vec<f64> = ft
        .iter()
        .copied()
        .filter(|x| x.is_finite() && *x > 0.0)
        .collect();
    let bins = bins.max(1);
    if clean.is_empty() {
        return Histogram {
            start_ms: 0.0,
            width_ms: 1.0,
            counts: vec![0; bins],
            overflow: 0,
        };
    }
    let lo = clean.iter().copied().fold(f64::INFINITY, f64::min);
    let hi = quantile(&clean, 0.999).unwrap_or(lo).max(lo + 0.1);
    let start = (lo * 10.0).floor() / 10.0;
    let width = ((hi - start) / bins as f64).max(0.01);
    let mut counts = vec![0usize; bins];
    let mut overflow = 0;
    for x in clean {
        let i = ((x - start) / width).floor() as isize;
        if i >= bins as isize {
            if x <= hi + 1e-9 {
                counts[bins - 1] += 1;
            } else {
                overflow += 1;
            }
        } else {
            counts[i.max(0) as usize] += 1;
        }
    }
    Histogram {
        start_ms: round(start, 2),
        width_ms: round(width, 4),
        counts,
        overflow,
    }
}

/// Min/max downsampling for the frametime-over-time chart: at most
/// `points` buckets, each keeping its slowest and fastest frame so spikes
/// survive.
pub fn downsample(t_ms: &[f64], ft: &[f64], points: usize) -> Vec<[f64; 3]> {
    let n = ft.len().min(t_ms.len());
    if n == 0 {
        return vec![];
    }
    let points = points.max(1);
    let per = n.div_ceil(points);
    (0..n)
        .step_by(per)
        .map(|s| {
            let e = (s + per).min(n);
            let slice = &ft[s..e];
            let max = slice.iter().copied().fold(f64::MIN, f64::max);
            let min = slice.iter().copied().fold(f64::MAX, f64::min);
            [round(t_ms[s], 1), round(min, 2), round(max, 2)]
        })
        .collect()
}

use serde_json::{json, Value};

pub const LOW_CONFIDENCE_BELOW: usize = 10;
pub const HIGH_CONFIDENCE_FROM: usize = 20;

pub fn median(values: &[f64]) -> Option<f64> {
    if values.is_empty() {
        return None;
    }
    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    let mid = sorted.len() / 2;
    Some(if sorted.len().is_multiple_of(2) {
        (sorted[mid - 1] + sorted[mid]) / 2.0
    } else {
        sorted[mid]
    })
}

pub fn mean(values: &[f64]) -> Option<f64> {
    (!values.is_empty()).then(|| values.iter().sum::<f64>() / values.len() as f64)
}

pub fn confidence(sample: usize) -> &'static str {
    if sample < LOW_CONFIDENCE_BELOW {
        "low"
    } else if sample < HIGH_CONFIDENCE_FROM {
        "medium"
    } else {
        "high"
    }
}

fn rounded(value: f64) -> f64 {
    (value * 100.0).round() / 100.0
}

pub fn summary(values: &[f64]) -> Value {
    json!({
        "sample": values.len(),
        "median": median(values).map(rounded),
        "mean": mean(values).map(rounded),
        "min": values.iter().copied().min_by(f64::total_cmp),
        "max": values.iter().copied().max_by(f64::total_cmp),
        "confidence": confidence(values.len()),
    })
}

pub fn lift(value: f64, baseline: Option<f64>) -> Option<f64> {
    baseline.filter(|b| *b > 0.0).map(|b| rounded(value / b))
}

pub fn lift_label(lift: Option<f64>) -> Option<&'static str> {
    match lift? {
        l if l >= 5.0 => Some("huge"),
        l if l >= 2.0 => Some("strong"),
        l if l >= 1.5 => Some("mild"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_median_ignores_one_viral_value_and_the_mean_does_not() {
        let views = [100.0, 120.0, 90.0, 110.0, 5000.0];
        assert_eq!(median(&views), Some(110.0));
        assert_eq!(mean(&views), Some(1084.0));
        assert_eq!(median(&[4.0, 1.0, 3.0, 2.0]), Some(2.5));
        assert_eq!(median(&[]), None);
        assert_eq!(mean(&[]), None);
    }

    #[test]
    fn confidence_is_low_under_ten_and_high_from_twenty() {
        assert_eq!(confidence(0), "low");
        assert_eq!(confidence(9), "low");
        assert_eq!(confidence(10), "medium");
        assert_eq!(confidence(19), "medium");
        assert_eq!(confidence(20), "high");
    }

    #[test]
    fn a_summary_always_shows_the_sample_size_beside_median_and_mean() {
        let s = summary(&[10.0, 20.0, 60.0]);
        assert_eq!(s["sample"], 3);
        assert_eq!(s["median"], 20.0);
        assert_eq!(s["mean"], 30.0);
        assert_eq!(s["min"], 10.0);
        assert_eq!(s["max"], 60.0);
        assert_eq!(s["confidence"], "low");
        let empty = summary(&[]);
        assert_eq!(empty["sample"], 0);
        assert!(empty["median"].is_null());
    }

    #[test]
    fn lift_is_measured_against_the_own_median_with_vendor_labels() {
        assert_eq!(lift(500.0, Some(100.0)), Some(5.0));
        assert_eq!(lift(500.0, Some(0.0)), None);
        assert_eq!(lift(500.0, None), None);
        assert_eq!(lift_label(Some(5.0)), Some("huge"));
        assert_eq!(lift_label(Some(2.0)), Some("strong"));
        assert_eq!(lift_label(Some(1.5)), Some("mild"));
        assert_eq!(lift_label(Some(1.49)), None);
        assert_eq!(lift_label(None), None);
    }
}

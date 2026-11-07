use std::path::{Path, PathBuf};

pub fn recording_name(stamp: &chrono::NaiveDateTime, ext: &str) -> String {
    format!(
        "OmniGet Capture {}.{ext}",
        stamp.format("%Y-%m-%d %H-%M-%S")
    )
}

pub fn clock_label(seconds: f64) -> String {
    let total_ms = (seconds.max(0.0) * 1000.0).round() as u64;
    let h = total_ms / 3_600_000;
    let m = (total_ms / 60_000) % 60;
    let s = (total_ms / 1000) % 60;
    let ms = total_ms % 1000;
    if h > 0 {
        format!("{h}h{m:02}m{s:02}s{ms:03}")
    } else {
        format!("{m:02}m{s:02}s{ms:03}")
    }
}

pub fn stem_of(path: &Path) -> String {
    path.file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "video".to_string())
}

pub fn ext_of(path: &Path) -> String {
    path.extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase())
        .filter(|e| !e.is_empty())
        .unwrap_or_else(|| "mp4".to_string())
}

pub fn segment_name(stem: &str, start: f64, end: f64, ext: &str) -> String {
    format!(
        "{stem} [{} - {}].{ext}",
        clock_label(start),
        clock_label(end)
    )
}

pub fn merged_name(stem: &str, ext: &str) -> String {
    format!("{stem} (cut).{ext}")
}

pub fn snapshot_name(stem: &str, at: f64) -> String {
    format!("{stem} [{}].png", clock_label(at))
}

pub fn unique_path(dir: &Path, file_name: &str) -> PathBuf {
    unique_with(dir, file_name, |p| p.exists())
}

pub fn unique_with(dir: &Path, file_name: &str, taken: impl Fn(&Path) -> bool) -> PathBuf {
    let first = dir.join(file_name);
    if !taken(&first) {
        return first;
    }
    let p = Path::new(file_name);
    let stem = p
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();
    let ext = p
        .extension()
        .map(|e| format!(".{}", e.to_string_lossy()))
        .unwrap_or_default();
    let mut n = 2u32;
    loop {
        let candidate = dir.join(format!("{stem} ({n}){ext}"));
        if !taken(&candidate) {
            return candidate;
        }
        n += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn recording_name_has_timestamp() {
        let stamp = chrono::NaiveDate::from_ymd_opt(2026, 11, 5)
            .unwrap()
            .and_hms_opt(14, 3, 9)
            .unwrap();
        assert_eq!(
            recording_name(&stamp, "mp4"),
            "OmniGet Capture 2026-11-05 14-03-09.mp4"
        );
    }

    #[test]
    fn clock_labels_are_file_safe() {
        assert_eq!(clock_label(0.0), "00m00s000");
        assert_eq!(clock_label(62.5), "01m02s500");
        assert_eq!(clock_label(3723.004), "1h02m03s004");
        assert!(!clock_label(99.9).contains(':'));
    }

    #[test]
    fn unique_never_reuses_taken_names() {
        let taken: HashSet<PathBuf> =
            [PathBuf::from("/out/a.mp4"), PathBuf::from("/out/a (2).mp4")]
                .into_iter()
                .collect();
        let got = unique_with(Path::new("/out"), "a.mp4", |p| taken.contains(p));
        assert_eq!(got, PathBuf::from("/out/a (3).mp4"));
        let free = unique_with(Path::new("/out"), "b.mp4", |p| taken.contains(p));
        assert_eq!(free, PathBuf::from("/out/b.mp4"));
    }

    #[test]
    fn segment_and_snapshot_names() {
        assert_eq!(
            segment_name("clip", 1.0, 2.25, "mov"),
            "clip [00m01s000 - 00m02s250].mov"
        );
        assert_eq!(snapshot_name("clip", 3.0), "clip [00m03s000].png");
        assert_eq!(merged_name("clip", "mkv"), "clip (cut).mkv");
        assert_eq!(ext_of(Path::new("/a/B.MP4")), "mp4");
        assert_eq!(stem_of(Path::new("/a/B.MP4")), "B");
    }
}

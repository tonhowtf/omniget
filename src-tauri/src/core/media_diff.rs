use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct MediaSnapshot {
    pub duration_secs: Option<f64>,
    #[serde(default)]
    pub chapters: Vec<String>,
    pub sha256: Option<String>,
    pub title: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub enum Change {
    Duration { before: f64, after: f64, delta: f64 },
    ChaptersAdded(Vec<String>),
    ChaptersRemoved(Vec<String>),
    TitleChanged { before: String, after: String },
    ContentOnly,
}

pub const DURATION_TOLERANCE_SECS: f64 = 0.5;

pub fn diff(before: &MediaSnapshot, after: &MediaSnapshot) -> Vec<Change> {
    let mut changes = Vec::new();

    if let (Some(a), Some(b)) = (before.duration_secs, after.duration_secs) {
        let delta = b - a;
        if delta.abs() > DURATION_TOLERANCE_SECS {
            changes.push(Change::Duration {
                before: a,
                after: b,
                delta,
            });
        }
    }

    let removed: Vec<String> = before
        .chapters
        .iter()
        .filter(|c| !after.chapters.contains(c))
        .cloned()
        .collect();
    if !removed.is_empty() {
        changes.push(Change::ChaptersRemoved(removed));
    }

    let added: Vec<String> = after
        .chapters
        .iter()
        .filter(|c| !before.chapters.contains(c))
        .cloned()
        .collect();
    if !added.is_empty() {
        changes.push(Change::ChaptersAdded(added));
    }

    if let (Some(a), Some(b)) = (&before.title, &after.title) {
        if a != b {
            changes.push(Change::TitleChanged {
                before: a.clone(),
                after: b.clone(),
            });
        }
    }

    if changes.is_empty() {
        if let (Some(a), Some(b)) = (&before.sha256, &after.sha256) {
            if a != b {
                changes.push(Change::ContentOnly);
            }
        }
    }

    changes
}

pub fn summarize(changes: &[Change]) -> Option<String> {
    if changes.is_empty() {
        return None;
    }
    let parts: Vec<String> = changes
        .iter()
        .map(|c| match c {
            Change::Duration { delta, .. } if *delta < 0.0 => {
                format!("{:.0}s shorter", delta.abs())
            }
            Change::Duration { delta, .. } => format!("{delta:.0}s longer"),
            Change::ChaptersRemoved(v) => format!("{} chapter(s) removed", v.len()),
            Change::ChaptersAdded(v) => format!("{} chapter(s) added", v.len()),
            Change::TitleChanged { .. } => "title changed".to_string(),
            Change::ContentOnly => "re-encoded or re-uploaded".to_string(),
        })
        .collect();
    Some(parts.join(", "))
}

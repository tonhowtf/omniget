use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

#[derive(Debug, Clone, PartialEq)]
pub struct Active {
    pub cli: &'static str,
    pub project: String,
}

fn newest_jsonl(dir: &Path, depth: usize, best: &mut Option<(SystemTime, PathBuf)>) {
    let Ok(read) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in read.flatten() {
        let path = entry.path();
        let Ok(meta) = entry.metadata() else { continue };
        if meta.is_dir() {
            if depth > 0 {
                newest_jsonl(&path, depth - 1, best);
            }
        } else if path.extension().map(|e| e == "jsonl").unwrap_or(false) {
            if let Ok(m) = meta.modified() {
                if best.as_ref().map(|b| m > b.0).unwrap_or(true) {
                    *best = Some((m, path));
                }
            }
        }
    }
}

fn claude_project(path: &Path) -> String {
    path.parent()
        .and_then(|p| p.file_name())
        .map(|n| {
            n.to_string_lossy()
                .rsplit('-')
                .next()
                .unwrap_or("")
                .to_string()
        })
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "project".into())
}

pub fn poll(window: Duration) -> Vec<Active> {
    let Some(home) = dirs::home_dir() else {
        return Vec::new();
    };
    let now = SystemTime::now();
    let fresh = |t: SystemTime| now.duration_since(t).map(|d| d <= window).unwrap_or(true);
    let mut out = Vec::new();

    let mut best = None;
    newest_jsonl(&home.join(".claude").join("projects"), 1, &mut best);
    if let Some((t, path)) = best.filter(|b| fresh(b.0)) {
        let _ = t;
        out.push(Active {
            cli: "Claude Code",
            project: claude_project(&path),
        });
    }
    let mut best = None;
    newest_jsonl(&home.join(".codex").join("sessions"), 3, &mut best);
    if let Some((_, _path)) = best.filter(|b| fresh(b.0)) {
        out.push(Active {
            cli: "Codex",
            project: "terminal".into(),
        });
    }
    out
}

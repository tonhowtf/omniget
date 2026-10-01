use std::path::{Path, PathBuf};

use serde_json::{json, Value};

const BRIEF_MAX: usize = 8 * 1024;
const INDEX_MAX_NOTES: usize = 40;
const NOTE_MAX: usize = 256 * 1024;
pub const KB_DIR: &str = ".omniget/kb";

fn kb_dir(ws: &Path) -> PathBuf {
    ws.join(KB_DIR)
}

fn clip(s: &str, max: usize) -> &str {
    if s.len() <= max {
        return s;
    }
    let mut cut = max;
    while !s.is_char_boundary(cut) {
        cut -= 1;
    }
    &s[..cut]
}

fn title_of(text: &str, fallback: &str) -> String {
    text.lines()
        .find_map(|l| {
            l.trim()
                .strip_prefix('#')
                .map(|t| t.trim_start_matches('#').trim().to_string())
        })
        .filter(|t| !t.is_empty())
        .unwrap_or_else(|| fallback.to_string())
}

fn notes(ws: &Path) -> Vec<(String, String)> {
    let Ok(read) = std::fs::read_dir(kb_dir(ws)) else {
        return Vec::new();
    };
    let mut out: Vec<(String, String)> = read
        .flatten()
        .filter(|e| e.path().extension().map(|x| x == "md").unwrap_or(false))
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().to_string();
            let text = std::fs::read_to_string(e.path()).ok()?;
            Some((name, text))
        })
        .collect();
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

pub fn index_prompt(ws: &Path) -> String {
    let mut out = String::new();
    for name in ["AGENTS.md", "agents.md", "CLAUDE.md"] {
        if let Ok(text) = std::fs::read_to_string(ws.join(name)) {
            if !text.trim().is_empty() {
                out.push_str(&format!(
                    "# Project brief ({name})\n{}\n",
                    clip(&text, BRIEF_MAX)
                ));
                break;
            }
        }
    }
    let notes = notes(ws);
    if !notes.is_empty() {
        out.push_str("\n# Project knowledge base (.omniget/kb)\nNotes written by the team. Read one with kb_search; add what you learn with kb_write.\n");
        for (name, text) in notes.iter().take(INDEX_MAX_NOTES) {
            out.push_str(&format!("- {name}: {}\n", title_of(text, name)));
        }
        if notes.len() > INDEX_MAX_NOTES {
            out.push_str(&format!("- … and {} more\n", notes.len() - INDEX_MAX_NOTES));
        }
    }
    out
}

fn words(s: &str) -> Vec<String> {
    s.to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| w.len() > 2)
        .map(str::to_string)
        .collect()
}

fn sections(name: &str, text: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut head = title_of(text, name);
    let mut body = String::new();
    for line in text.lines() {
        if line.starts_with('#') && !body.trim().is_empty() {
            out.push((format!("{name} › {head}"), std::mem::take(&mut body)));
        }
        if let Some(h) = line.trim().strip_prefix('#') {
            head = h.trim_start_matches('#').trim().to_string();
        }
        body.push_str(line);
        body.push('\n');
    }
    if !body.trim().is_empty() {
        out.push((format!("{name} › {head}"), body));
    }
    out
}

pub async fn kb_search(a: Value) -> Result<Value, String> {
    let ws = super::code_tools::workspace().ok_or_else(|| {
        format!(
            "{}: attach a folder first",
            super::code_tools::ERR_NO_WORKSPACE
        )
    })?;
    let query = a
        .get("query")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_string();
    if query.is_empty() {
        return Err("query is required".into());
    }
    let k = a
        .get("limit")
        .and_then(Value::as_u64)
        .unwrap_or(5)
        .clamp(1, 20) as usize;
    let chunks: Vec<(String, String)> = notes(&ws)
        .iter()
        .flat_map(|(n, t)| sections(n, t))
        .collect();
    if chunks.is_empty() {
        return Ok(
            json!({ "hits": [], "note": "the knowledge base is empty; write the first note with kb_write" }),
        );
    }
    let texts: Vec<&str> = std::iter::once(query.as_str())
        .chain(chunks.iter().map(|c| c.1.as_str()))
        .collect();
    let owned: Vec<String> = texts.iter().map(|t| clip(t, 2000).to_string()).collect();
    let embedded = if !crate::core::embed::is_ready() {
        None
    } else {
        tokio::task::spawn_blocking(move || {
            let refs: Vec<&str> = owned.iter().map(String::as_str).collect();
            crate::core::embed::embed_batch(&refs).ok()
        })
        .await
        .ok()
        .flatten()
    };
    let (mode, ranked): (&str, Vec<(usize, f32)>) = match embedded {
        Some(vecs) if vecs.len() == chunks.len() + 1 => {
            let q: Vec<f32> = vecs[0].as_ref().to_vec();
            let corpus: Vec<Vec<f32>> = vecs[1..].iter().map(|v| v.as_ref().to_vec()).collect();
            ("semantic", crate::core::embed::top_k(&q, &corpus, k))
        }
        _ => {
            let q = words(&query);
            let mut scored: Vec<(usize, f32)> = chunks
                .iter()
                .enumerate()
                .map(|(i, (head, body))| {
                    let hay = words(&format!("{head} {body}"));
                    let hits = q
                        .iter()
                        .map(|w| hay.iter().filter(|h| h.contains(w.as_str())).count())
                        .sum::<usize>();
                    (i, hits as f32 / (hay.len() as f32).sqrt().max(1.0))
                })
                .filter(|(_, s)| *s > 0.0)
                .collect();
            scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
            scored.truncate(k);
            ("keyword", scored)
        }
    };
    let hits: Vec<Value> = ranked
        .into_iter()
        .map(|(i, score)| json!({ "where": chunks[i].0, "score": score, "text": clip(&chunks[i].1, 4000) }))
        .collect();
    Ok(json!({ "mode": mode, "hits": hits }))
}

fn slug(s: &str) -> String {
    let mut out = String::new();
    for c in s.trim().to_lowercase().chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c);
        } else if !out.ends_with('-') {
            out.push('-');
        }
    }
    out.trim_matches('-').chars().take(60).collect()
}

pub fn kb_write(a: &Value) -> Result<Value, String> {
    let ws = super::code_tools::workspace().ok_or_else(|| {
        format!(
            "{}: attach a folder first",
            super::code_tools::ERR_NO_WORKSPACE
        )
    })?;
    let title = a
        .get("title")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_string();
    let content = a
        .get("content")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let append = a.get("append").and_then(Value::as_bool).unwrap_or(false);
    let name = slug(&title);
    if name.is_empty() || content.trim().is_empty() {
        return Err("title and content are required".into());
    }
    if content.len() > NOTE_MAX {
        return Err("note too large (256 KB max): split it".into());
    }
    let dir = kb_dir(&ws);
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let path = dir.join(format!("{name}.md"));
    let body = match (append, std::fs::read_to_string(&path)) {
        (true, Ok(old)) => format!("{}\n\n{}\n", old.trim_end(), content.trim()),
        _ if content.trim_start().starts_with('#') => format!("{}\n", content.trim()),
        _ => format!("# {title}\n\n{}\n", content.trim()),
    };
    std::fs::write(&path, body).map_err(|e| e.to_string())?;
    Ok(json!({ "path": format!("{KB_DIR}/{name}.md"), "appended": append }))
}

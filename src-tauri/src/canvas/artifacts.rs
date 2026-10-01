use std::path::{Path, PathBuf};
use std::sync::Arc;

use omniget_core::core::canvas::layout::free_spot_in;
use omniget_core::core::canvas::model::{
    default_size, new_id, CanvasNode, Connection, ConnectionKind, NodeKind, Point,
};
use omniget_core::core::canvas::notes;
use omniget_core::core::canvas::ops::Op;
use omniget_core::core::canvas::store::CanvasStore;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

pub const MAX_BYTES: usize = 5 * 1024 * 1024;
pub const KINDS: &[&str] = &["html", "md", "svg"];

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Meta {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub created_at: String,
    pub source: String,
    pub origin: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(default)]
    pub size: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<String>,
}

pub struct SaveSpec {
    pub name: String,
    pub kind: Option<String>,
    pub content: Option<String>,
    pub path: Option<String>,
    pub url: Option<String>,
    pub origin: String,
    pub session: Option<String>,
}

fn store() -> Arc<CanvasStore> {
    crate::canvas::doc::host().store().clone()
}

pub fn valid_id(id: &str) -> bool {
    id.len() == 32
        && id
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
}

pub fn root(s: &CanvasStore, ws: &str) -> PathBuf {
    s.workspace_dir(ws).join("files").join("artifacts")
}

fn file_for(kind: &str) -> Option<&'static str> {
    Some(match kind {
        "html" => "index.html",
        "md" => "index.md",
        "svg" => "image.svg",
        _ => return None,
    })
}

fn kind_from_ext(p: &Path) -> Option<&'static str> {
    let ext = p.extension()?.to_str()?.to_ascii_lowercase();
    Some(match ext.as_str() {
        "html" | "htm" => "html",
        "md" | "markdown" | "txt" => "md",
        "svg" => "svg",
        _ => return None,
    })
}

fn looks_svg(text: &str) -> bool {
    let head: String = text.trim_start().chars().take(512).collect::<String>();
    let head = head.to_ascii_lowercase();
    head.starts_with("<svg") || (head.starts_with("<?xml") && head.contains("<svg"))
}

fn resolve_source(s: &CanvasStore, ws: &str, raw: &str) -> Result<PathBuf, String> {
    let raw = raw.trim();
    let mut p = if let Some(rest) = raw.strip_prefix("~/") {
        dirs::home_dir()
            .ok_or("CANVAS_IO: no home folder")?
            .join(rest)
    } else {
        PathBuf::from(raw)
    };
    if p.is_relative() {
        let base = s
            .workspace(ws)
            .ok()
            .and_then(|w| w.working_directory)
            .filter(|d| !d.trim().is_empty())
            .ok_or("CANVAS_INVALID: relative path and the workspace has no folder")?;
        p = PathBuf::from(base).join(p);
    }
    let real = p
        .canonicalize()
        .map_err(|_| format!("CANVAS_NOT_FOUND: no file at {}", p.display()))?;
    if !real.is_file() {
        return Err(format!("CANVAS_INVALID: {} is not a file", real.display()));
    }
    Ok(real)
}

pub fn save(ws: &str, spec: SaveSpec) -> Result<Meta, String> {
    let s = store();
    s.workspace(ws)?;
    let name = spec.name.trim().chars().take(120).collect::<String>();
    if name.is_empty() {
        return Err("MISSING_ARGUMENT: name".into());
    }
    let sources = [
        spec.content.is_some(),
        spec.path.is_some(),
        spec.url.is_some(),
    ]
    .iter()
    .filter(|x| **x)
    .count();
    if sources != 1 {
        return Err("INVALID_ARGUMENT: pass exactly one of content, path or url".into());
    }
    let id = uuid::Uuid::new_v4().simple().to_string();
    let dir = root(&s, ws).join(&id);
    let created_at = chrono::Utc::now().to_rfc3339();
    let session = spec
        .session
        .map(|v| v.trim().chars().take(80).collect::<String>())
        .filter(|v| !v.is_empty());
    let meta = if let Some(raw) = spec.url {
        let u = url::Url::parse(raw.trim()).map_err(|e| format!("INVALID_ARGUMENT: url ({e})"))?;
        if !matches!(u.scheme(), "https" | "http") || u.host_str().is_none() {
            return Err("INVALID_ARGUMENT: url must be http(s)".into());
        }
        if spec.kind.as_deref().is_some_and(|k| k != "link") {
            return Err("INVALID_ARGUMENT: a url artifact is a link; leave kind out".into());
        }
        Meta {
            id: id.clone(),
            name,
            kind: "link".into(),
            created_at,
            source: "url".into(),
            origin: spec.origin,
            file: None,
            url: Some(u.to_string()),
            size: 0,
            session,
        }
    } else {
        let (text, from_path) = match (spec.content, spec.path) {
            (Some(c), _) => (c, None),
            (None, Some(p)) => {
                let real = resolve_source(&s, ws, &p)?;
                let len = std::fs::metadata(&real)
                    .map_err(|e| format!("CANVAS_IO: {e}"))?
                    .len();
                if len as usize > MAX_BYTES {
                    return Err("CANVAS_TOO_LARGE: artifacts are up to 5 MB".into());
                }
                let text = std::fs::read_to_string(&real)
                    .map_err(|_| "CANVAS_INVALID: the file is not UTF-8 text".to_string())?;
                (text, Some(real))
            }
            _ => unreachable!(),
        };
        if text.len() > MAX_BYTES {
            return Err("CANVAS_TOO_LARGE: artifacts are up to 5 MB".into());
        }
        let kind = match (
            spec.kind.as_deref(),
            from_path.as_deref().and_then(kind_from_ext),
        ) {
            (Some(k), Some(e)) if k != e => {
                return Err(format!(
                    "INVALID_ARGUMENT: kind {k} does not match the file extension ({e})"
                ))
            }
            (Some(k), _) => k.to_string(),
            (None, Some(e)) => e.to_string(),
            (None, None) if from_path.is_some() => {
                return Err("INVALID_ARGUMENT: the file must be .html, .md or .svg".into())
            }
            (None, None) => {
                if looks_svg(&text) {
                    "svg".into()
                } else if text.to_ascii_lowercase().contains("<html")
                    || text
                        .trim_start()
                        .to_ascii_lowercase()
                        .starts_with("<!doctype")
                {
                    "html".into()
                } else {
                    "md".into()
                }
            }
        };
        let file = file_for(&kind)
            .ok_or_else(|| format!("INVALID_ARGUMENT: kind must be one of {}", KINDS.join(", ")))?;
        if kind == "svg" && !looks_svg(&text) {
            return Err("INVALID_ARGUMENT: the content is not an SVG document".into());
        }
        std::fs::create_dir_all(&dir).map_err(|e| format!("CANVAS_IO: {e}"))?;
        std::fs::write(dir.join(file), text.as_bytes()).map_err(|e| format!("CANVAS_IO: {e}"))?;
        Meta {
            id: id.clone(),
            name,
            kind,
            created_at,
            source: if from_path.is_some() {
                "path"
            } else {
                "content"
            }
            .into(),
            origin: spec.origin,
            file: Some(file.into()),
            url: None,
            size: text.len() as u64,
            session,
        }
    };
    std::fs::create_dir_all(&dir).map_err(|e| format!("CANVAS_IO: {e}"))?;
    let body = serde_json::to_vec_pretty(&meta).map_err(|e| e.to_string())?;
    std::fs::write(dir.join("meta.json"), body).map_err(|e| format!("CANVAS_IO: {e}"))?;
    Ok(meta)
}

pub fn get(ws: &str, id: &str) -> Result<Meta, String> {
    if !valid_id(id) {
        return Err(format!("CANVAS_NOT_FOUND: no artifact '{id}'"));
    }
    let s = store();
    let raw = std::fs::read(root(&s, ws).join(id).join("meta.json"))
        .map_err(|_| format!("CANVAS_NOT_FOUND: no artifact '{id}'"))?;
    let m: Meta = serde_json::from_slice(&raw).map_err(|e| format!("CANVAS_INVALID: {e}"))?;
    if m.id != id {
        return Err(format!("CANVAS_INVALID: artifact '{id}' is damaged"));
    }
    Ok(m)
}

pub fn list(ws: &str) -> Result<Vec<Meta>, String> {
    let s = store();
    s.workspace(ws)?;
    let mut out = Vec::new();
    let Ok(rd) = std::fs::read_dir(root(&s, ws)) else {
        return Ok(out);
    };
    for e in rd.flatten() {
        let name = e.file_name().to_string_lossy().to_string();
        if let Ok(m) = get(ws, &name) {
            out.push(m);
        }
    }
    out.sort_by(|a, b| b.created_at.cmp(&a.created_at));
    Ok(out)
}

pub fn read_text(ws: &str, m: &Meta) -> Result<String, String> {
    let file = m
        .file
        .as_deref()
        .ok_or("CANVAS_INVALID: link artifacts have no file")?;
    std::fs::read_to_string(root(&store(), ws).join(&m.id).join(file))
        .map_err(|e| format!("CANVAS_IO: {e}"))
}

pub fn view(ws: &str, m: &Meta) -> Value {
    let mut v = serde_json::to_value(m).unwrap_or(Value::Null);
    if let Some(file) = &m.file {
        v["path"] = json!(root(&store(), ws).join(&m.id).join(file).to_string_lossy());
    }
    if m.kind == "html" {
        if let Ok(u) = super::artifacts_http::url_for(ws, &m.id, "index.html") {
            v["localUrl"] = json!(u);
        }
    }
    v
}

fn cord(from: &str, to: &str, kind: ConnectionKind) -> Op {
    Op::AddConnection {
        connection: Connection {
            id: new_id(),
            from: from.to_string(),
            to: to.to_string(),
            kind,
            perms: vec![],
            condition: None,
            verify: None,
            label: None,
            points: vec![],
            from_note_id: None,
            to_note_id: None,
            is_active: None,
        },
    }
}

fn cord_kind(a: &NodeKind, b: &NodeKind) -> ConnectionKind {
    if matches!(a, NodeKind::Note | NodeKind::Stack)
        || matches!(b, NodeKind::Note | NodeKind::Stack)
    {
        ConnectionKind::Note
    } else if *a == NodeKind::Portal || *b == NodeKind::Portal {
        ConnectionKind::Portal
    } else {
        ConnectionKind::Message
    }
}

pub fn place(
    ws: &str,
    floor: &str,
    m: &Meta,
    at: Option<Point>,
    connect_from: Option<&str>,
) -> Result<Value, String> {
    let s = store();
    let doc = s.read(ws, floor, |d| d.clone())?;
    let from = match connect_from {
        Some(id) => Some(
            doc.node(id)
                .cloned()
                .ok_or_else(|| format!("CANVAS_INVALID: '{id}' is not on this floor"))?,
        ),
        None => None,
    };
    let kind = match m.kind.as_str() {
        "md" => NodeKind::Note,
        "svg" => NodeKind::Image,
        _ => NodeKind::Portal,
    };
    let size = default_size(&kind);
    let spot = match at {
        Some(p) => p,
        None => {
            let near = from.as_ref().map(|f| Point {
                x: f.x + f.w + 60.0,
                y: f.y,
            });
            free_spot_in(&doc, size.w, size.h, near, &[])
        }
    };
    let spot = Point {
        x: (spot.x / 20.0).round() * 20.0,
        y: (spot.y / 20.0).round() * 20.0,
    };
    let node_id = if kind == NodeKind::Note {
        let text = read_text(ws, m)?;
        let id = notes::create(&s, ws, floor, spot.x, spot.y, &text, "artifact")?;
        s.apply(
            ws,
            floor,
            vec![Op::update_node(
                &id,
                json!({ "title": m.name, "data": { "hasCustomName": true, "artifactId": m.id } }),
            )],
            "artifact",
            None,
        )?;
        id
    } else {
        let data = match m.kind.as_str() {
            "svg" => {
                let file = m.file.as_deref().unwrap_or("image.svg");
                json!({
                    "path": root(&s, ws).join(&m.id).join(file).to_string_lossy(),
                    "artifactId": m.id,
                })
            }
            "html" => json!({
                "url": super::artifacts_http::url_for(ws, &m.id, "index.html")?,
                "portalKind": "web",
                "storageScope": "isolated",
                "artifactId": m.id,
            }),
            _ => {
                let mut d = json!({
                    "url": m.url.clone().ok_or("CANVAS_INVALID: link artifact without url")?,
                    "portalKind": "web",
                    "storageScope": "isolated",
                    "artifactId": m.id,
                });
                if let Some(sess) = &m.session {
                    d["session"] = json!(sess);
                }
                d
            }
        };
        let id = new_id();
        let node = CanvasNode {
            id: id.clone(),
            kind: kind.clone(),
            x: spot.x,
            y: spot.y,
            w: size.w,
            h: size.h,
            z: doc.max_z() + 1.0,
            title: Some(m.name.clone()),
            subtitle: None,
            icon: None,
            color: None,
            group_id: None,
            locked: false,
            stack_id: None,
            data,
        };
        s.apply(ws, floor, vec![Op::AddNode { node }], "artifact", None)?;
        id
    };
    if let Some(f) = &from {
        s.apply(
            ws,
            floor,
            vec![cord(&f.id, &node_id, cord_kind(&f.kind, &kind))],
            "artifact",
            None,
        )?;
    }
    let node = s.read(ws, floor, |d| d.node(&node_id).cloned())?;
    Ok(json!({ "nodeId": node_id, "node": node, "artifact": view(ws, m) }))
}

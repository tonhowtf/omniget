//! Canvas clipboard of the app (Maestri M-B21, M-B30, M-B37): structural
//! copy, cut and paste of a selection across workspaces and floors, plus
//! pasting what other apps put on the OS clipboard (a link, an SVG, file
//! paths, an image, plain text) as canvas nodes.
//!
//! The clip itself is built and stamped by `omniget_core::core::canvas::clipboard`.
//! Here it is kept in memory and mirrored to the OS clipboard as text, so a
//! paste after a relaunch or from another window still finds it. The UI
//! (`canvas_clipboard_*` commands) and the MCP tool `canvas_clipboard` share
//! these functions.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use omniget_core::core::canvas::clipboard::{self as clip, Clip, ClipSelection, Placement};
use omniget_core::core::canvas::model::{default_size, new_id, CanvasNode, NodeKind, Point, GRID};
use omniget_core::core::canvas::ops::Op;
use omniget_core::core::canvas::store::CanvasStore;
use serde_json::{json, Map, Value};
use tauri::AppHandle;
use tauri_plugin_clipboard_manager::ClipboardExt;

use super::doc::host;

/// Origin of the document writes a paste makes.
pub const ORIGIN: &str = "clipboard";
const GAP: f64 = 20.0;

#[derive(Default)]
struct State {
    clip: Option<Clip>,
    /// The text we last wrote to the OS clipboard (clip JSON, or a file path).
    os_text: Option<String>,
    /// Repeated pastes of one clip on one floor cascade.
    pastes: HashMap<String, u32>,
}

fn state() -> &'static Mutex<State> {
    static S: OnceLock<Mutex<State>> = OnceLock::new();
    S.get_or_init(|| Mutex::new(State::default()))
}

fn lock() -> std::sync::MutexGuard<'static, State> {
    state().lock().unwrap_or_else(|e| e.into_inner())
}

fn store(app: &AppHandle) -> Arc<CanvasStore> {
    host().bind(app).store().clone()
}

/// Copies (or cuts) a selection of one floor. The OS clipboard gets the clip
/// as text; a single file node puts its path there instead (Maestri: ⌘C on a
/// file copies the file), the clip stays in memory for canvas pastes.
pub fn copy(
    app: &AppHandle,
    ws: &str,
    floor: &str,
    sel: &ClipSelection,
    cut: bool,
) -> Result<Value, String> {
    let s = store(app);
    let c = clip::copy_from_store(&s, ws, floor, sel)?;
    let single_file = (c.nodes.len() == 1 && c.drawings.is_empty())
        .then(|| &c.nodes[0])
        .filter(|n| n.kind == NodeKind::File)
        .and_then(|n| n.data.get("path").and_then(Value::as_str))
        .map(str::to_string);
    let text = single_file.unwrap_or_else(|| clip::encode_text(&c));
    let os = app.clipboard().write_text(text.clone()).is_ok();
    let summary = c.summary();
    {
        let mut st = lock();
        st.os_text = Some(text);
        st.pastes.retain(|k, _| !k.starts_with(&c.id));
        st.clip = Some(c.clone());
    }
    let mut removed = Vec::new();
    if cut {
        removed = remove(&s, ws, floor, &c, sel)?;
    }
    Ok(json!({
        "action": if cut { "cut" } else { "copy" },
        "clip": summary,
        "osClipboard": os,
        "removed": removed,
    }))
}

fn remove(
    s: &CanvasStore,
    ws: &str,
    floor: &str,
    c: &Clip,
    sel: &ClipSelection,
) -> Result<Vec<String>, String> {
    let doc = s.read(ws, floor, |d| d.clone())?;
    let mut ops = Vec::new();
    let mut ids = Vec::new();
    for n in &c.nodes {
        if let Some(live) = doc.node(&n.id) {
            ops.push(Op::RemoveNode { id: n.id.clone() });
            ids.push(n.id.clone());
            if let Some(t) = super::launch::terminal_of(live) {
                let _ = crate::pty::term().close(&t, true);
            }
        }
    }
    for d in &sel.drawing_ids {
        if doc.drawings.iter().any(|x| &x.id == d) {
            ops.push(Op::RemoveDrawing { id: d.clone() });
            ids.push(d.clone());
        }
    }
    if !ops.is_empty() {
        s.apply(ws, floor, ops, ORIGIN, None)?;
    }
    Ok(ids)
}

/// What a paste would use: ours (memory or decoded OS text) or foreign content.
enum Source {
    Ours(Box<Clip>),
    Foreign(Foreign),
}

/// One node a foreign paste creates: kind, title, data.
type ForeignItem = (NodeKind, String, Map<String, Value>);

enum Foreign {
    Url(String),
    Svg(String),
    Paths(Vec<PathBuf>),
    Image { w: u32, h: u32, rgba: Vec<u8> },
    Text(String),
}

fn read_source(app: &AppHandle) -> Option<Source> {
    let text = app
        .clipboard()
        .read_text()
        .ok()
        .filter(|t| !t.trim().is_empty());
    {
        let st = lock();
        if let (Some(t), Some(mine), Some(c)) = (&text, &st.os_text, &st.clip) {
            if t == mine {
                return Some(Source::Ours(Box::new(c.clone())));
            }
        }
        if text.is_none() {
            // Nothing readable as text (or a foreign image): the in-memory clip
            // still wins only when the OS clipboard has no image either.
            if let Ok(img) = app.clipboard().read_image() {
                return Some(Source::Foreign(Foreign::Image {
                    w: img.width(),
                    h: img.height(),
                    rgba: img.rgba().to_vec(),
                }));
            }
            return st.clip.clone().map(|c| Source::Ours(Box::new(c)));
        }
    }
    let t = text?;
    if let Some(c) = clip::decode_text(&t) {
        return Some(Source::Ours(Box::new(c)));
    }
    if let Some(f) = classify_text(&t) {
        return Some(Source::Foreign(f));
    }
    if let Ok(img) = app.clipboard().read_image() {
        return Some(Source::Foreign(Foreign::Image {
            w: img.width(),
            h: img.height(),
            rgba: img.rgba().to_vec(),
        }));
    }
    Some(Source::Foreign(Foreign::Text(t)))
}

/// Pure classification of a foreign clipboard text (SVG, one URL, paths).
fn classify_text(t: &str) -> Option<Foreign> {
    let s = t.trim();
    let lower = s.to_ascii_lowercase();
    if lower.starts_with("<svg") || (lower.starts_with("<?xml") && lower.contains("<svg")) {
        return Some(Foreign::Svg(s.to_string()));
    }
    let lines: Vec<&str> = s.lines().map(str::trim).filter(|l| !l.is_empty()).collect();
    if lines.is_empty() {
        return None;
    }
    let paths: Vec<PathBuf> = lines
        .iter()
        .filter_map(|l| {
            let p = if let Some(rest) = l.strip_prefix("file://") {
                let rest = rest.strip_prefix("localhost").unwrap_or(rest);
                PathBuf::from(urlencoding::decode(rest).ok()?.into_owned())
            } else {
                let unquoted = l.trim_matches(|c| c == '\'' || c == '"');
                match unquoted.strip_prefix("~/") {
                    Some(r) => dirs::home_dir()?.join(r),
                    None => PathBuf::from(unquoted),
                }
            };
            (p.is_absolute() && p.exists()).then_some(p)
        })
        .collect();
    if paths.len() == lines.len() {
        return Some(Foreign::Paths(paths));
    }
    if lines.len() == 1 && !s.contains(char::is_whitespace) {
        if let Ok(u) = url::Url::parse(s) {
            if matches!(u.scheme(), "http" | "https") && u.host_str().is_some() {
                return Some(Foreign::Url(u.to_string()));
            }
        }
        // `example.com/x` without a scheme.
        let host_like = s.split('/').next().is_some_and(|h| {
            h.contains('.')
                && !h.starts_with('.')
                && !h.ends_with('.')
                && h.chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-')
        });
        if s.starts_with("www.") || (host_like && s.contains('/') && !s.starts_with('/')) {
            if let Ok(u) = url::Url::parse(&format!("https://{s}")) {
                if u.host_str().is_some_and(|h| h.contains('.')) {
                    return Some(Foreign::Url(u.to_string()));
                }
            }
        }
    }
    None
}

/// Where the paste lands: explicit top-left, or the given centre (the UI's
/// view centre) with the cascade, or a centre estimated from the saved camera.
#[derive(Debug, Clone, Copy, Default)]
pub struct Where {
    pub at: Option<Point>,
    pub center: Option<Point>,
}

fn snap(v: f64) -> f64 {
    (v / GRID).round() * GRID
}

/// Cascade index of a repeated paste of the same thing on one floor.
fn next_repeat(key: String) -> u32 {
    let mut st = lock();
    let n = st.pastes.entry(key).or_insert(0);
    let r = *n;
    *n += 1;
    r
}

/// A stable key of foreign content, for the cascade of repeated pastes.
fn foreign_key(f: &Foreign) -> String {
    match f {
        Foreign::Url(u) => format!("url:{u}"),
        Foreign::Svg(t) => format!("svg:{}:{}", t.len(), t.chars().take(64).collect::<String>()),
        Foreign::Paths(ps) => format!("paths:{ps:?}"),
        Foreign::Image { w, h, rgba } => {
            let sum = rgba
                .iter()
                .step_by(97)
                .fold(0u64, |a, b| a.wrapping_mul(31).wrapping_add(*b as u64));
            format!("img:{w}x{h}:{sum}")
        }
        Foreign::Text(t) => format!("text:{t}"),
    }
}

fn centre_of(s: &CanvasStore, ws: &str, floor: &str, w: &Where) -> Result<Point, String> {
    if let Some(c) = w.center {
        return Ok(c);
    }
    let cam = s.read(ws, floor, |d| d.camera)?;
    let z = if cam.zoom > 0.0 { cam.zoom } else { 1.0 };
    // A typical window; the UI always sends its real centre.
    Ok(Point {
        x: cam.x + 640.0 / z,
        y: cam.y + 400.0 / z,
    })
}

/// Pastes the clipboard into a floor. Returns the ids of what was created.
pub fn paste(app: &AppHandle, ws: &str, floor: &str, mut w: Where) -> Result<Value, String> {
    let s = store(app);
    // Every stored frame sits on the 20 pt grid (M-B1).
    w.at = w.at.map(|p| Point {
        x: snap(p.x),
        y: snap(p.y),
    });
    s.read(ws, floor, |_| ())?;
    match read_source(app) {
        None => Err("CLIPBOARD_EMPTY: nothing to paste".into()),
        Some(Source::Ours(c)) => paste_clip(&s, ws, floor, &c, w),
        Some(Source::Foreign(f)) => paste_foreign(&s, ws, floor, f, w),
    }
}

fn paste_clip(s: &CanvasStore, ws: &str, floor: &str, c: &Clip, w: Where) -> Result<Value, String> {
    let placement = match w.at {
        Some(p) => Placement::At(p),
        None => {
            let repeat = next_repeat(format!("{}|{ws}|{floor}", c.id));
            Placement::Centered {
                center: centre_of(s, ws, floor, &w)?,
                repeat,
            }
        }
    };
    let r = clip::paste(s, ws, floor, c, placement, ORIGIN)?;
    for id in &r.terminals {
        let native = s
            .read(ws, floor, |d| {
                d.node(id).map(|n| super::launch::agent_of(n) == "native")
            })
            .ok()
            .flatten()
            .unwrap_or(true);
        if !native {
            super::launch::spawn_ensure(ws, floor, id);
        }
    }
    Ok(json!({
        "source": "canvas",
        "nodeIds": r.node_ids,
        "connectionIds": r.connection_ids,
        "drawingIds": r.drawing_ids,
        "groupIds": r.group_ids,
        "terminals": r.terminals,
        "rev": r.apply.rev,
    }))
}

fn base_name(p: &Path) -> String {
    p.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| p.to_string_lossy().into_owned())
}

fn has_ext(p: &Path, exts: &[&str]) -> bool {
    p.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| exts.iter().any(|x| e.eq_ignore_ascii_case(x)))
}

const IMAGE_EXT: &[&str] = &[
    "png", "jpg", "jpeg", "gif", "webp", "svg", "bmp", "heic", "avif",
];
const VIDEO_EXT: &[&str] = &["mp4", "mov", "webm", "m4v", "mkv"];

/// Node for a path: folder → file tree, note file → external note, else file.
fn node_for_path(p: &Path) -> (NodeKind, String, Map<String, Value>) {
    let name = base_name(p);
    let abs = p.to_string_lossy().into_owned();
    let mut data = Map::new();
    if p.is_dir() {
        data.insert("rootPath".into(), json!(abs));
        return (NodeKind::FileTree, name, data);
    }
    if has_ext(p, &["md", "markdown", "txt"]) {
        data.insert("path".into(), json!(abs));
        data.insert("isExternal".into(), json!(true));
        let title = p
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or(name);
        return (NodeKind::Note, title, data);
    }
    data.insert("path".into(), json!(abs));
    data.insert("displayName".into(), json!(name));
    if has_ext(p, IMAGE_EXT) {
        data.insert("isImage".into(), json!(true));
    }
    if has_ext(p, VIDEO_EXT) {
        data.insert("isVideo".into(), json!(true));
    }
    (NodeKind::File, name, data)
}

fn files_dir(s: &CanvasStore, ws: &str) -> Result<PathBuf, String> {
    let dir = s.workspace_dir(ws).join("files").join("pasted");
    std::fs::create_dir_all(&dir).map_err(|e| format!("CANVAS_IO: {e}"))?;
    Ok(dir)
}

fn paste_foreign(
    s: &CanvasStore,
    ws: &str,
    floor: &str,
    f: Foreign,
    w: Where,
) -> Result<Value, String> {
    let fkey = format!("{}|{ws}|{floor}", foreign_key(&f));
    let (source, items): (&str, Vec<ForeignItem>) = match f {
        Foreign::Url(u) => {
            let mut d = Map::new();
            d.insert("url".into(), json!(u));
            let title = url::Url::parse(&u)
                .ok()
                .and_then(|x| x.host_str().map(str::to_string))
                .unwrap_or_else(|| u.clone());
            ("link", vec![(NodeKind::Link, title, d)])
        }
        Foreign::Svg(text) => {
            let path = files_dir(s, ws)?.join(format!("pasted-{}.svg", &new_id()[..8]));
            std::fs::write(&path, text).map_err(|e| format!("CANVAS_IO: {e}"))?;
            ("svg", vec![node_for_path(&path)])
        }
        Foreign::Paths(ps) => ("files", ps.iter().map(|p| node_for_path(p)).collect()),
        Foreign::Image { w: iw, h: ih, rgba } => {
            let path = files_dir(s, ws)?.join(format!("pasted-{}.png", &new_id()[..8]));
            let img = image::RgbaImage::from_raw(iw, ih, rgba)
                .ok_or("CLIPBOARD_IMAGE: malformed image")?;
            img.save(&path).map_err(|e| format!("CANVAS_IO: {e}"))?;
            ("image", vec![node_for_path(&path)])
        }
        Foreign::Text(t) => {
            let mut d = Map::new();
            d.insert("text".into(), json!(t));
            ("text", vec![(NodeKind::Text, String::new(), d)])
        }
    };
    let doc = s.read(ws, floor, |d| d.clone())?;
    let sizes: Vec<_> = items.iter().map(|(k, _, _)| default_size(k)).collect();
    let total_w: f64 =
        sizes.iter().map(|z| z.w).sum::<f64>() + GAP * (sizes.len().saturating_sub(1)) as f64;
    let max_h = sizes.iter().map(|z| z.h).fold(0.0, f64::max);
    let top_left = match w.at {
        Some(p) => p,
        None => {
            // ⌘V of the same link/file twice cascades instead of stacking.
            let c = centre_of(s, ws, floor, &w)?;
            let step = clip::CASCADE * next_repeat(fkey) as f64;
            Point {
                x: snap(c.x - total_w / 2.0) + step,
                y: snap(c.y - max_h / 2.0) + step,
            }
        }
    };
    let mut z = doc.max_z() + 1.0;
    let mut x = top_left.x;
    let mut ops = Vec::new();
    let mut ids = Vec::new();
    for ((kind, title, data), size) in items.into_iter().zip(sizes) {
        let id = new_id();
        ids.push(id.clone());
        ops.push(Op::AddNode {
            node: CanvasNode {
                id,
                kind,
                x,
                y: top_left.y,
                w: size.w,
                h: size.h,
                z,
                title: (!title.is_empty()).then_some(title),
                subtitle: None,
                icon: None,
                color: None,
                group_id: None,
                locked: false,
                stack_id: None,
                data: Value::Object(data),
            },
        });
        x += size.w + GAP;
        z += 1.0;
    }
    let r = s.apply(ws, floor, ops, ORIGIN, None)?;
    Ok(json!({
        "source": source,
        "nodeIds": ids,
        "connectionIds": [],
        "drawingIds": [],
        "groupIds": [],
        "terminals": [],
        "rev": r.rev,
    }))
}

/// What the clipboard holds now, without pasting.
pub fn read(app: &AppHandle) -> Value {
    let canvas = lock().clip.as_ref().map(Clip::summary);
    let os = match read_source(app) {
        None => json!({ "kind": "empty" }),
        Some(Source::Ours(c)) => json!({ "kind": "canvas", "clipId": c.id }),
        Some(Source::Foreign(Foreign::Url(u))) => json!({ "kind": "link", "url": u }),
        Some(Source::Foreign(Foreign::Svg(t))) => json!({ "kind": "svg", "bytes": t.len() }),
        Some(Source::Foreign(Foreign::Paths(p))) => json!({
            "kind": "files",
            "paths": p.iter().map(|x| x.to_string_lossy().into_owned()).collect::<Vec<_>>(),
        }),
        Some(Source::Foreign(Foreign::Image { w, h, .. })) => {
            json!({ "kind": "image", "width": w, "height": h })
        }
        Some(Source::Foreign(Foreign::Text(t))) => {
            json!({ "kind": "text", "preview": t.chars().take(200).collect::<String>() })
        }
    };
    json!({ "canvas": canvas, "os": os })
}

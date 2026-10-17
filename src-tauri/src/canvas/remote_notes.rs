use std::collections::HashMap;
use std::path::{Path as FsPath, PathBuf};
use std::sync::Arc;

use axum::extract::{ConnectInfo, DefaultBodyLimit, Path, Query, Request};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use base64::Engine as _;
use omniget_core::core::canvas::model::{is_safe_id, NodeKind, NoteData};
use omniget_core::core::canvas::store::CanvasStore;
use omniget_core::core::canvas::{assets, chat, notes};
use serde_json::{json, Value};

use crate::local_bridge_remote::{self as lbr, Peer, RemoteDevice, Scope};

pub const IMAGE_MAX: usize = 15 * 1024 * 1024;
const SERVE_MAX: u64 = 32 * 1024 * 1024;
const IMAGE_EXTS: &[&str] = &["png", "jpg", "jpeg", "gif", "webp", "heic", "heif", "avif"];

fn store() -> Arc<CanvasStore> {
    super::doc::host().store().clone()
}

pub fn routes<S: Clone + Send + Sync + 'static>() -> Router<S> {
    Router::new()
        .route("/remote/api/notes", get(api_list))
        .route(
            "/remote/api/notes/{ws}/{floor}/{id}",
            get(api_read)
                .put(api_write)
                .layer(DefaultBodyLimit::max(notes::MAX_NOTE_BYTES * 2 + 4096)),
        )
        .route(
            "/remote/api/notes/{ws}/{floor}/{id}/image",
            post(api_image).layer(DefaultBodyLimit::max(IMAGE_MAX / 3 * 4 + 64 * 1024)),
        )
        .route("/remote/api/note-image", get(api_note_image))
}

#[allow(clippy::result_large_err)]
fn auth(headers: &HeaderMap, peer: &Peer, drive: bool) -> Result<RemoteDevice, Response> {
    let d = lbr::device_of(headers, peer).ok_or_else(|| {
        lbr::fail_response(
            StatusCode::UNAUTHORIZED,
            "unauthorized",
            "missing or invalid device token",
        )
    })?;
    if drive && d.scope != Scope::Drive {
        return Err(lbr::tag_device(
            lbr::fail_response(
                StatusCode::FORBIDDEN,
                "insufficient_scope",
                "read-only device",
            ),
            &d,
        ));
    }
    Ok(d)
}

fn reply(r: Result<Value, String>, d: &RemoteDevice) -> Response {
    let resp = match r {
        Ok(v) => Json(v).into_response(),
        Err(e) => {
            let code = e.split(':').next().unwrap_or("ERR").trim().to_string();
            let status = if code.ends_with("NOT_FOUND") {
                StatusCode::NOT_FOUND
            } else if code.ends_with("CONFLICT") {
                StatusCode::CONFLICT
            } else if code.ends_with("LOCKED") {
                StatusCode::LOCKED
            } else if code.ends_with("TOO_LARGE") {
                StatusCode::PAYLOAD_TOO_LARGE
            } else {
                StatusCode::BAD_REQUEST
            };
            lbr::fail_response(status, &code, e)
        }
    };
    lbr::tag_device(resp, d)
}

async fn blocking<T: Send + 'static>(
    f: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    tauri::async_runtime::spawn_blocking(f)
        .await
        .map_err(|e| format!("CANVAS_TASK: {e}"))?
}

fn check_ids(ids: &[&str]) -> Result<(), String> {
    if ids.iter().all(|i| is_safe_id(i)) {
        Ok(())
    } else {
        Err("CANVAS_INVALID: id".into())
    }
}

fn note_node(s: &CanvasStore, ws: &str, floor: &str, id: &str) -> Result<NoteData, String> {
    check_ids(&[ws, floor, id])?;
    let node = s
        .read(ws, floor, |d| d.node(id).cloned())?
        .ok_or_else(|| format!("CANVAS_NOT_FOUND: note {id}"))?;
    if node.kind != NodeKind::Note {
        return Err(format!("CANVAS_INVALID: node {id} is not a note"));
    }
    Ok(node.data_as::<NoteData>().unwrap_or_default())
}

pub fn list_all() -> Result<Value, String> {
    let s = store();
    let mut out = Vec::new();
    for summary in s.workspaces() {
        let ws = summary.workspace;
        let floors: HashMap<&str, &str> = ws
            .floors
            .iter()
            .map(|f| (f.id.as_str(), f.name.as_str()))
            .collect();
        let items: Vec<Value> = notes::list(&s, &ws.id)
            .unwrap_or_default()
            .into_iter()
            .filter_map(|n| {
                let (id, floor) = (n.node_id?, n.floor_id?);
                Some(json!({
                    "id": id,
                    "ws": ws.id,
                    "floor": floor,
                    "floorName": floors.get(floor.as_str()).copied().unwrap_or(""),
                    "title": n.display_name,
                    "updated": n.last_modified_at,
                    "preview": n.preview,
                    "external": n.is_external,
                }))
            })
            .collect();
        if items.is_empty() {
            continue;
        }
        out.push(json!({ "id": ws.id, "name": ws.name, "notes": items }));
    }
    Ok(json!({ "workspaces": out }))
}

pub fn read_note(ws: &str, floor: &str, id: &str) -> Result<Value, String> {
    let s = store();
    let data = note_node(&s, ws, floor, id)?;
    let c = notes::read(&s, ws, floor, id)?;
    Ok(json!({
        "id": id,
        "ws": ws,
        "floor": floor,
        "title": c.display_name,
        "text": c.text,
        "revision": c.revision,
        "external": c.is_external,
        "locked": data.is_content_locked,
        "updated": c.last_modified_at,
    }))
}

pub fn write_note(
    ws: &str,
    floor: &str,
    id: &str,
    text: &str,
    base: &str,
    origin: &str,
) -> Result<Value, String> {
    let s = store();
    note_node(&s, ws, floor, id)?;
    let c = notes::write(&s, ws, floor, id, text, Some(base), origin)?;
    Ok(json!({
        "id": id,
        "title": c.display_name,
        "revision": c.revision,
        "updated": c.last_modified_at,
    }))
}

pub fn sniff_image(bytes: &[u8]) -> Option<(&'static str, &'static str)> {
    if bytes.starts_with(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]) {
        return Some(("png", "image/png"));
    }
    if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        return Some(("jpg", "image/jpeg"));
    }
    if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        return Some(("gif", "image/gif"));
    }
    if bytes.len() >= 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        return Some(("webp", "image/webp"));
    }
    if bytes.len() >= 12 && &bytes[4..8] == b"ftyp" {
        return match &bytes[8..12] {
            b"heic" | b"heix" | b"hevc" | b"hevx" | b"heim" | b"heis" | b"mif1" | b"msf1" => {
                Some(("heic", "image/heic"))
            }
            b"avif" | b"avis" => Some(("avif", "image/avif")),
            _ => None,
        };
    }
    None
}

fn has_image_ext(p: &FsPath) -> bool {
    p.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| IMAGE_EXTS.iter().any(|x| e.eq_ignore_ascii_case(x)))
}

pub fn image_refs(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = text;
    while let Some(i) = rest.find("![") {
        rest = &rest[i + 2..];
        let Some(close) = rest.find("](") else {
            break;
        };
        if rest[..close].contains('\n') {
            continue;
        }
        let after = &rest[close + 2..];
        let target = match after.strip_prefix('<') {
            Some(inner) => inner.find(['>', '\n']).map(|j| &inner[..j]),
            None => {
                let end = after.find([')', ' ', '\t', '\n']).unwrap_or(after.len());
                Some(&after[..end])
            }
        };
        if let Some(t) = target.map(str::trim).filter(|t| !t.is_empty()) {
            out.push(t.to_string());
        }
        rest = after;
    }
    out
}

pub fn safe_image_path(
    raw: &str,
    bases: &[PathBuf],
    roots: &[PathBuf],
    referenced: bool,
) -> Option<PathBuf> {
    let raw = raw.trim();
    if raw.is_empty() || raw.len() > 4096 || raw.contains('\0') {
        return None;
    }
    let p = raw.strip_prefix("file://").unwrap_or(raw);
    if p.contains("://") {
        return None;
    }
    let candidates: Vec<PathBuf> = if let Some(rest) = p.strip_prefix("~/") {
        dirs::home_dir().map(|h| h.join(rest)).into_iter().collect()
    } else if FsPath::new(p).is_absolute() {
        vec![PathBuf::from(p)]
    } else {
        bases
            .iter()
            .filter(|b| b.is_absolute())
            .map(|b| b.join(p))
            .collect()
    };
    let roots: Vec<PathBuf> = roots
        .iter()
        .filter_map(|r| std::fs::canonicalize(r).ok())
        .collect();
    candidates.into_iter().find_map(|c| {
        let real = std::fs::canonicalize(&c).ok()?;
        if !real.is_file() || !has_image_ext(&real) {
            return None;
        }
        (referenced || roots.iter().any(|r| real.starts_with(r))).then_some(real)
    })
}

fn load_note_image(ws: &str, raw: &str) -> Result<(Vec<u8>, &'static str), String> {
    check_ids(&[ws])?;
    let s = store();
    let meta = s.workspace(ws)?;
    let mut referenced = false;
    let mut bases: Vec<PathBuf> = Vec::new();
    for n in notes::list(&s, ws)? {
        let text = std::fs::read_to_string(&n.path).unwrap_or_default();
        if !image_refs(&text).iter().any(|r| r == raw) {
            continue;
        }
        referenced = true;
        if n.is_external {
            if let Some(dir) = FsPath::new(&n.path).parent() {
                bases.push(dir.to_path_buf());
            }
        }
    }
    if let Some(wd) = meta.working_directory.filter(|w| !w.trim().is_empty()) {
        bases.push(PathBuf::from(wd));
    }
    bases.push(s.notes_dir(ws));
    let roots = vec![
        chat::inbox_attachments_dir(&s, ws)?,
        chat::chat_dir(&s, ws).join("attachments"),
        assets::assets_dir(&s.workspace_dir(ws)),
    ];
    let path = safe_image_path(raw, &bases, &roots, referenced)
        .ok_or("CANVAS_NOT_FOUND: image not available")?;
    let len = std::fs::metadata(&path)
        .map_err(|e| format!("CANVAS_NOT_FOUND: {e}"))?
        .len();
    if len > SERVE_MAX {
        return Err("CANVAS_TOO_LARGE: image is too large".into());
    }
    let bytes = std::fs::read(&path).map_err(|e| format!("CANVAS_NOT_FOUND: {e}"))?;
    let (_, mime) = sniff_image(&bytes).ok_or("CANVAS_INVALID: not a supported image")?;
    Ok((bytes, mime))
}

fn image_file_name(raw: &str, ext: &str) -> String {
    let base = raw.rsplit(['/', '\\']).next().unwrap_or("");
    let stem = base.rsplit_once('.').map(|(s, _)| s).unwrap_or(base);
    let clean: String = stem
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || "-_".contains(c) {
                c
            } else {
                '_'
            }
        })
        .take(60)
        .collect();
    let clean = clean.trim_matches('_');
    if clean.is_empty() {
        format!("photo.{ext}")
    } else {
        format!("{clean}.{ext}")
    }
}

pub fn image_markdown(label: &str, abs: &str) -> String {
    format!("![{}](<{abs}>)", label.replace(['[', ']'], ""))
}

pub fn save_image(
    ws: &str,
    floor: &str,
    id: &str,
    name: &str,
    bytes: &[u8],
) -> Result<Value, String> {
    if bytes.is_empty() {
        return Err("CANVAS_INVALID: empty image".into());
    }
    if bytes.len() > IMAGE_MAX {
        return Err("CANVAS_TOO_LARGE: images are up to 15 MB".into());
    }
    let (ext, mime) = sniff_image(bytes)
        .ok_or("CANVAS_INVALID: not a supported image (png, jpeg, gif, webp, heic)")?;
    let s = store();
    note_node(&s, ws, floor, id)?;
    let file = image_file_name(name, ext);
    let path = chat::save_inbox_attachment(&s, ws, &file, bytes)?;
    let abs = path.to_string_lossy().into_owned();
    Ok(json!({
        "path": abs,
        "markdown": image_markdown(&file, &abs),
        "mime": mime,
        "size": bytes.len(),
    }))
}

async fn api_list(ConnectInfo(peer): ConnectInfo<Peer>, headers: HeaderMap) -> Response {
    let d = match auth(&headers, &peer, false) {
        Ok(d) => d,
        Err(r) => return r,
    };
    reply(blocking(list_all).await, &d)
}

async fn api_read(
    ConnectInfo(peer): ConnectInfo<Peer>,
    headers: HeaderMap,
    Path((ws, floor, id)): Path<(String, String, String)>,
) -> Response {
    let d = match auth(&headers, &peer, false) {
        Ok(d) => d,
        Err(r) => return r,
    };
    reply(blocking(move || read_note(&ws, &floor, &id)).await, &d)
}

async fn api_write(
    ConnectInfo(peer): ConnectInfo<Peer>,
    headers: HeaderMap,
    Path((ws, floor, id)): Path<(String, String, String)>,
    Json(body): Json<Value>,
) -> Response {
    let d = match auth(&headers, &peer, true) {
        Ok(d) => d,
        Err(r) => return r,
    };
    let Some(text) = body["text"].as_str().map(str::to_string) else {
        return reply(Err("CANVAS_INVALID: text is required".into()), &d);
    };
    let Some(base) = body["revision"]
        .as_str()
        .map(str::to_string)
        .filter(|r| !r.is_empty())
    else {
        return reply(Err("CANVAS_INVALID: revision is required".into()), &d);
    };
    let origin = format!("remote:{}", d.id);
    let r = blocking(move || write_note(&ws, &floor, &id, &text, &base, &origin)).await;
    reply(r, &d)
}

async fn api_image(
    ConnectInfo(peer): ConnectInfo<Peer>,
    headers: HeaderMap,
    Path((ws, floor, id)): Path<(String, String, String)>,
    req: Request,
) -> Response {
    let d = match auth(&headers, &peer, true) {
        Ok(d) => d,
        Err(r) => return r,
    };
    let query_name = req
        .uri()
        .query()
        .and_then(|q| {
            url::form_urlencoded::parse(q.as_bytes())
                .find(|(k, _)| k == "filename")
                .map(|(_, v)| v.into_owned())
        })
        .unwrap_or_default();
    let is_json = headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.starts_with("application/json"));
    let body = match axum::body::to_bytes(req.into_body(), IMAGE_MAX / 3 * 4 + 64 * 1024).await {
        Ok(b) => b,
        Err(_) => return reply(Err("CANVAS_TOO_LARGE: images are up to 15 MB".into()), &d),
    };
    let (bytes, name) = if is_json {
        let v: Value = match serde_json::from_slice(&body) {
            Ok(v) => v,
            Err(e) => return reply(Err(format!("CANVAS_INVALID: {e}")), &d),
        };
        let raw = v["data"].as_str().unwrap_or("");
        let raw = raw.split_once("base64,").map(|(_, b)| b).unwrap_or(raw);
        let bytes = match base64::engine::general_purpose::STANDARD.decode(raw.trim()) {
            Ok(b) => b,
            Err(e) => return reply(Err(format!("CANVAS_INVALID: base64: {e}")), &d),
        };
        let name = v["name"].as_str().map(str::to_string).unwrap_or(query_name);
        (bytes, name)
    } else {
        (body.to_vec(), query_name)
    };
    let r = blocking(move || save_image(&ws, &floor, &id, &name, &bytes)).await;
    reply(r, &d)
}

async fn api_note_image(
    ConnectInfo(peer): ConnectInfo<Peer>,
    headers: HeaderMap,
    Query(q): Query<HashMap<String, String>>,
) -> Response {
    let d = match auth(&headers, &peer, false) {
        Ok(d) => d,
        Err(r) => return r,
    };
    let ws = q.get("ws").cloned().unwrap_or_default();
    let raw = q.get("path").cloned().unwrap_or_default();
    match blocking(move || load_note_image(&ws, &raw)).await {
        Ok((bytes, mime)) => lbr::tag_device(
            (
                [
                    (header::CONTENT_TYPE, mime),
                    (header::CACHE_CONTROL, "private, max-age=300"),
                    (
                        header::CONTENT_SECURITY_POLICY,
                        "default-src 'none'; sandbox",
                    ),
                ],
                bytes,
            )
                .into_response(),
            &d,
        ),
        Err(e) => reply(Err(e), &d),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png() -> Vec<u8> {
        let mut b = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
        b.extend_from_slice(&[0; 16]);
        b
    }

    #[test]
    fn sniff_recognizes_supported_images() {
        assert_eq!(sniff_image(&png()), Some(("png", "image/png")));
        assert_eq!(
            sniff_image(&[0xFF, 0xD8, 0xFF, 0xE0, 0, 0]),
            Some(("jpg", "image/jpeg"))
        );
        assert_eq!(sniff_image(b"GIF89a....."), Some(("gif", "image/gif")));
        assert_eq!(
            sniff_image(b"RIFF\x10\x00\x00\x00WEBPVP8 "),
            Some(("webp", "image/webp"))
        );
        assert_eq!(
            sniff_image(b"\x00\x00\x00\x18ftypheic\x00\x00\x00\x00"),
            Some(("heic", "image/heic"))
        );
        assert_eq!(
            sniff_image(b"\x00\x00\x00\x18ftypmif1\x00\x00\x00\x00"),
            Some(("heic", "image/heic"))
        );
    }

    #[test]
    fn sniff_rejects_non_images() {
        assert_eq!(sniff_image(b""), None);
        assert_eq!(
            sniff_image(b"<svg xmlns=\"http://www.w3.org/2000/svg\"/>"),
            None
        );
        assert_eq!(sniff_image(b"%PDF-1.7"), None);
        assert_eq!(sniff_image(b"\x00\x00\x00\x18ftypisom\x00\x00"), None);
        assert_eq!(sniff_image(b"RIFF\x10\x00\x00\x00WAVEfmt "), None);
        assert_eq!(sniff_image(&[0x89, b'P', b'N']), None);
    }

    #[test]
    fn image_refs_reads_markdown_targets() {
        let text = "# t\n![a](</tmp/x y.png>) and ![b](rel/pic.jpg \"title\")\n[link](not.png) ![broken\nalt](z.png) ![](<>)";
        assert_eq!(image_refs(text), vec!["/tmp/x y.png", "rel/pic.jpg"]);
    }

    #[test]
    fn image_markdown_strips_brackets() {
        assert_eq!(
            image_markdown("a[b].png", "/w/a.png"),
            "![ab.png](</w/a.png>)"
        );
        assert_eq!(image_file_name("../x/My Pic.HEIC", "heic"), "My_Pic.heic");
        assert_eq!(image_file_name("", "png"), "photo.png");
    }

    #[test]
    fn safe_path_requires_reference_or_asset_root() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("ws").join("inbox").join("attachments");
        let outside = dir.path().join("outside");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        let inside_img = root.join("a.png");
        let outside_img = outside.join("b.png");
        let secret = outside.join("secret.txt");
        std::fs::write(&inside_img, png()).unwrap();
        std::fs::write(&outside_img, png()).unwrap();
        std::fs::write(&secret, b"secret").unwrap();
        let roots = vec![root.clone()];
        let s = |p: &PathBuf| p.to_string_lossy().into_owned();

        assert!(safe_image_path(&s(&inside_img), &[], &roots, false).is_some());
        assert!(safe_image_path(&s(&outside_img), &[], &roots, false).is_none());
        assert!(safe_image_path(&s(&outside_img), &[], &roots, true).is_some());
        assert!(safe_image_path(&s(&secret), &[], &roots, true).is_none());

        let escape = format!("{}/../../../outside/b.png", s(&root));
        assert!(safe_image_path(&escape, &[], &roots, false).is_none());

        assert!(safe_image_path("a.png", std::slice::from_ref(&root), &roots, false).is_some());
        assert!(safe_image_path(
            "../../../outside/b.png",
            std::slice::from_ref(&root),
            &roots,
            false
        )
        .is_none());
        assert!(safe_image_path("a.png", &[PathBuf::from("rel")], &roots, true).is_none());
        assert!(safe_image_path("https://example.com/a.png", &[], &roots, true).is_none());
        assert!(safe_image_path("", &[], &roots, true).is_none());
        assert!(safe_image_path(&s(&root.join("missing.png")), &[], &roots, true).is_none());
    }

    #[cfg(unix)]
    #[test]
    fn safe_path_follows_symlinks_before_checking() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("assets");
        std::fs::create_dir_all(&root).unwrap();
        let secret = dir.path().join("secret.txt");
        std::fs::write(&secret, b"secret").unwrap();
        let outside_img = dir.path().join("c.png");
        std::fs::write(&outside_img, png()).unwrap();
        let disguised = root.join("evil.png");
        std::os::unix::fs::symlink(&secret, &disguised).unwrap();
        let linked = root.join("linked.png");
        std::os::unix::fs::symlink(&outside_img, &linked).unwrap();
        let roots = vec![root.clone()];
        assert!(safe_image_path(&disguised.to_string_lossy(), &[], &roots, true).is_none());
        assert!(safe_image_path(&linked.to_string_lossy(), &[], &roots, false).is_none());
    }
}

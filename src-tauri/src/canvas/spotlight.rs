//! Spotlight (Maestri: workspaces, terminals and note contents are found by
//! the system search; the result opens the item in the app, floors too).
//!
//! Decision (estudo 100, M-A9): no Core Spotlight binding exists in the
//! tree (no `objc2-core-spotlight`), so the canvas is mirrored as `.webloc`
//! files in `~/Library/Application Support/OmniGet Spotlight/<profile>/`,
//! a folder Spotlight indexes. Each file:
//! - is named after the item (`Title — Workspace.webloc`), which Spotlight
//!   matches by name;
//! - carries the searchable text (kind, workspace, floor, a note's body up
//!   to 8 KB) in `com.apple.metadata:kMDItemFinderComment`, a binary plist
//!   that Spotlight imports, so `mdfind <word of the note>` finds it;
//! - points at `omniget://canvas/open?ws=..&floor=..&node=..`, so opening
//!   the result opens OmniGet on that node ([`open_link`]).
//!
//! The mirror is rebuilt a few seconds after the canvas changes; only files
//! whose content changed are rewritten, and files of deleted items go away.
//! Nothing outside that folder is touched. macOS only; elsewhere a no-op.

use serde_json::json;
use tauri::{AppHandle, Emitter, Manager};

/// Handles `omniget://canvas/open?ws=&floor=&node=` (a Spotlight result, a
/// link in a note). Returns false for any other URL.
pub fn open_link(app: &AppHandle, raw: &str) -> bool {
    let Some(rest) = raw.strip_prefix("omniget://canvas") else {
        return false;
    };
    let query = rest.split_once('?').map(|(_, q)| q).unwrap_or("");
    let mut ws = String::new();
    let mut floor = String::new();
    let mut node = String::new();
    for (k, v) in url::form_urlencoded::parse(query.as_bytes()) {
        match k.as_ref() {
            "ws" | "workspace" => ws = v.into_owned(),
            "floor" => floor = v.into_owned(),
            "node" => node = v.into_owned(),
            _ => {}
        }
    }
    let store = super::doc::host().store().clone();
    if !node.is_empty() {
        if let Some((w, f)) = store.locate_node(&node) {
            ws = w;
            floor = f;
        }
    }
    if ws.is_empty() {
        return true;
    }
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.unminimize();
        let _ = w.show();
        let _ = w.set_focus();
    }
    let mut payload = json!({ "workspaceId": ws });
    if !floor.is_empty() {
        payload["floorId"] = json!(floor);
    }
    if !node.is_empty() {
        payload["nodeId"] = json!(node);
    }
    let _ = app.emit(crate::mcp::canvas::EVENT_NAVIGATE, payload.clone());
    if !node.is_empty() {
        let app = app.clone();
        tauri::async_runtime::spawn(async move {
            let _ = crate::mcp::canvas_ui::ui(&app, "focus", payload).await;
            super::cords::host().mark_seen(&node);
        });
    }
    true
}

#[cfg(target_os = "macos")]
mod mirror {
    use std::collections::{BTreeMap, HashSet};
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;
    use std::time::Duration;

    use omniget_core::core::canvas::model::NodeKind;
    use omniget_core::core::canvas::notes;
    use omniget_core::core::canvas::store::CanvasStore;

    const NOTE_TEXT_MAX: usize = 8 * 1024;
    const INDEX: &str = ".index.json";

    pub struct Item {
        pub file: String,
        pub url: String,
        pub text: String,
    }

    /// `~/Library/Application Support/OmniGet Spotlight/<profile key>`: one
    /// folder per data folder, so a test profile never mixes with the real one.
    pub fn dir() -> Option<PathBuf> {
        let data = omniget_core::core::paths::app_data_dir()?;
        let mut h: u64 = 0xcbf29ce484222325;
        for b in data.to_string_lossy().bytes() {
            h ^= b as u64;
            h = h.wrapping_mul(0x100000001b3);
        }
        Some(
            dirs::data_dir()?
                .join("OmniGet Spotlight")
                .join(format!("{:010x}", h & 0xff_ffff_ffff)),
        )
    }

    fn clean(s: &str) -> String {
        let s: String = s
            .chars()
            .map(|c| {
                if matches!(c, '/' | ':' | '\\') || c.is_control() {
                    '-'
                } else {
                    c
                }
            })
            .collect();
        let s = s.trim().trim_start_matches('.').to_string();
        s.chars().take(80).collect()
    }

    fn link(ws: &str, floor: Option<&str>, node: Option<&str>) -> String {
        let mut q = url::form_urlencoded::Serializer::new(String::new());
        q.append_pair("ws", ws);
        if let Some(f) = floor {
            q.append_pair("floor", f);
        }
        if let Some(n) = node {
            q.append_pair("node", n);
        }
        format!("omniget://canvas/open?{}", q.finish())
    }

    /// Every workspace and every titled node of every floor.
    pub fn items(store: &CanvasStore) -> Vec<Item> {
        let mut out = Vec::new();
        for w in store.workspaces() {
            let ws = &w.workspace;
            let wname = clean(&ws.name);
            out.push(Item {
                file: format!(
                    "{wname} — OmniGet workspace [{}].webloc",
                    &ws.id[..ws.id.len().min(6)]
                ),
                url: link(&ws.id, None, None),
                text: format!("OmniGet workspace {}", ws.name),
            });
            for f in &ws.floors {
                let nodes = store
                    .read(&ws.id, &f.id, |d| d.nodes.clone())
                    .unwrap_or_default();
                for n in nodes {
                    let kind = match n.kind {
                        NodeKind::Terminal | NodeKind::Agent => "terminal",
                        NodeKind::Note => "note",
                        NodeKind::Portal => "portal",
                        NodeKind::Text => "text",
                        NodeKind::File => "file",
                        NodeKind::FileTree => "file tree",
                        _ => continue,
                    };
                    let mut body = String::new();
                    if n.kind == NodeKind::Note {
                        if let Ok(c) = notes::read(store, &ws.id, &f.id, &n.id) {
                            body = c.text.chars().take(NOTE_TEXT_MAX).collect();
                        }
                    } else if n.kind == NodeKind::Text {
                        body = n
                            .data_str("text")
                            .unwrap_or("")
                            .chars()
                            .take(NOTE_TEXT_MAX)
                            .collect();
                    }
                    let title = n
                        .title
                        .clone()
                        .filter(|t| !t.trim().is_empty())
                        .or_else(|| {
                            body.lines()
                                .find(|l| !l.trim().is_empty())
                                .map(|l| l.trim_start_matches('#').trim().to_string())
                        })
                        .unwrap_or_else(|| kind.to_string());
                    let floor_name = if f.id == "ground" {
                        String::new()
                    } else {
                        format!(" · {}", f.name)
                    };
                    out.push(Item {
                        file: format!(
                            "{} — {}{} [{}].webloc",
                            clean(&title),
                            wname,
                            if floor_name.is_empty() {
                                String::new()
                            } else {
                                format!(" · {}", clean(&f.name))
                            },
                            &n.id[..n.id.len().min(6)]
                        ),
                        url: link(&ws.id, Some(&f.id), Some(&n.id)),
                        text: format!("OmniGet {kind} {title} in {}{floor_name}\n{body}", ws.name),
                    });
                }
            }
        }
        out
    }

    fn webloc(url: &str) -> String {
        let esc = url.replace('&', "&amp;").replace('<', "&lt;");
        format!("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n<plist version=\"1.0\">\n<dict>\n\t<key>URL</key>\n\t<string>{esc}</string>\n</dict>\n</plist>\n")
    }

    /// A binary plist holding one string (what Spotlight reads from a
    /// `com.apple.metadata:*` attribute).
    pub fn bplist_string(s: &str) -> Vec<u8> {
        let mut out = b"bplist00".to_vec();
        let ascii = s.is_ascii();
        let units: Vec<u16> = if ascii {
            vec![]
        } else {
            s.encode_utf16().collect()
        };
        let len = if ascii { s.len() } else { units.len() };
        let marker: u8 = if ascii { 0x50 } else { 0x60 };
        if len < 15 {
            out.push(marker | len as u8);
        } else {
            out.push(marker | 0x0f);
            if len < 0x100 {
                out.extend([0x10, len as u8]);
            } else if len < 0x10000 {
                out.push(0x11);
                out.extend((len as u16).to_be_bytes());
            } else {
                out.push(0x12);
                out.extend((len as u32).to_be_bytes());
            }
        }
        if ascii {
            out.extend(s.as_bytes());
        } else {
            for u in units {
                out.extend(u.to_be_bytes());
            }
        }
        let table = out.len() as u64;
        out.push(8); // offset of object 0
        out.extend([0u8; 6]);
        out.push(1); // offset int size
        out.push(1); // object ref size
        out.extend(1u64.to_be_bytes());
        out.extend(0u64.to_be_bytes());
        out.extend(table.to_be_bytes());
        out
    }

    fn set_comment(path: &Path, text: &str) -> bool {
        let hex: String = bplist_string(text)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        std::process::Command::new("/usr/bin/xattr")
            .args(["-wx", "com.apple.metadata:kMDItemFinderComment", &hex])
            .arg(path)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .is_ok_and(|s| s.success())
    }

    fn digest(item: &Item) -> String {
        let mut h: u64 = 0xcbf29ce484222325;
        for b in item.url.bytes().chain(item.text.bytes()) {
            h ^= b as u64;
            h = h.wrapping_mul(0x100000001b3);
        }
        format!("{h:016x}")
    }

    /// Brings the folder in line with the canvas. Returns files written.
    pub fn sync(store: &CanvasStore) -> Result<usize, String> {
        let Some(dir) = dir() else { return Ok(0) };
        let items = items(store);
        if items.is_empty() {
            if dir.is_dir() {
                let _ = std::fs::remove_dir_all(&dir);
            }
            return Ok(0);
        }
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let idx_path = dir.join(INDEX);
        let mut index: BTreeMap<String, String> = std::fs::read(&idx_path)
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default();
        let mut keep: HashSet<String> = HashSet::new();
        let mut written = 0;
        for it in &items {
            keep.insert(it.file.clone());
            let d = digest(it);
            let path = dir.join(&it.file);
            if index.get(&it.file) == Some(&d) && path.is_file() {
                continue;
            }
            if std::fs::write(&path, webloc(&it.url)).is_ok() && set_comment(&path, &it.text) {
                index.insert(it.file.clone(), d);
                written += 1;
            }
        }
        if let Ok(rd) = std::fs::read_dir(&dir) {
            for e in rd.flatten() {
                let name = e.file_name().to_string_lossy().into_owned();
                if name.ends_with(".webloc") && !keep.contains(&name) {
                    let _ = std::fs::remove_file(e.path());
                    index.remove(&name);
                }
            }
        }
        index.retain(|k, _| keep.contains(k));
        let _ = std::fs::write(
            &idx_path,
            serde_json::to_vec_pretty(&index).unwrap_or_default(),
        );
        Ok(written)
    }

    pub fn start(store: Arc<CanvasStore>) {
        let dirty = Arc::new(AtomicBool::new(true));
        let d = dirty.clone();
        store.subscribe(move |_| d.store(true, Ordering::Relaxed));
        std::thread::Builder::new()
            .name("canvas-spotlight".into())
            .spawn(move || loop {
                std::thread::sleep(Duration::from_secs(3));
                if dirty.swap(false, Ordering::Relaxed) {
                    if let Err(e) = sync(&store) {
                        tracing::debug!("[canvas] spotlight mirror: {e}");
                    }
                }
            })
            .ok();
    }
}

/// Starts the Spotlight mirror (once). `OMNIGET_NO_SPOTLIGHT=1` turns it off.
pub fn start() {
    #[cfg(target_os = "macos")]
    {
        static ONCE: std::sync::Once = std::sync::Once::new();
        if std::env::var_os("OMNIGET_NO_SPOTLIGHT").is_some() {
            return;
        }
        ONCE.call_once(|| mirror::start(super::doc::host().store().clone()));
    }
}

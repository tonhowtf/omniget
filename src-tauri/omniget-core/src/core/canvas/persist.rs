use super::model::{FloorDoc, Workspace, FLOOR_FORMAT, FLOOR_VERSION, WORKSPACE_VERSION};
use serde_json::Value;
use std::collections::HashMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant, SystemTime};

pub const SAVE_DEBOUNCE: Duration = Duration::from_millis(400);
pub const CAMERA_DEBOUNCE: Duration = Duration::from_secs(2);
pub const TRASH_TTL: Duration = Duration::from_secs(7 * 24 * 3600);

pub fn canvas_root() -> PathBuf {
    crate::core::paths::app_data_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("canvas")
}

pub fn workspace_dir(root: &Path, ws: &str) -> PathBuf {
    root.join(ws)
}
pub fn workspace_file(root: &Path, ws: &str) -> PathBuf {
    workspace_dir(root, ws).join("workspace.json")
}
pub fn floors_dir(root: &Path, ws: &str) -> PathBuf {
    workspace_dir(root, ws).join("floors")
}
pub fn floor_file(root: &Path, ws: &str, floor: &str) -> PathBuf {
    floors_dir(root, ws).join(format!("{floor}.json"))
}
pub fn notes_dir(root: &Path, ws: &str) -> PathBuf {
    workspace_dir(root, ws).join("notes")
}
pub fn trash_dir(root: &Path, ws: &str) -> PathBuf {
    workspace_dir(root, ws).join("trash")
}
pub fn roles_dir(root: &Path, ws: &str) -> PathBuf {
    workspace_dir(root, ws).join("roles")
}
pub fn inbox_dir(root: &Path, ws: &str, node_id: &str) -> PathBuf {
    workspace_dir(root, ws).join("inbox").join(node_id)
}

fn bak_path(path: &Path) -> PathBuf {
    let mut s = path.as_os_str().to_owned();
    s.push(".bak");
    PathBuf::from(s)
}

pub fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let dir = path.parent().ok_or("path has no parent")?;
    fs::create_dir_all(dir).map_err(|e| format!("CANVAS_IO: create {}: {e}", dir.display()))?;
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("file");
    let tmp = dir.join(format!(".{name}.{}.tmp", std::process::id()));
    {
        let mut f =
            fs::File::create(&tmp).map_err(|e| format!("CANVAS_IO: {}: {e}", tmp.display()))?;
        f.write_all(bytes)
            .and_then(|_| f.sync_all())
            .map_err(|e| format!("CANVAS_IO: {}: {e}", tmp.display()))?;
    }
    if path.exists() {
        let _ = fs::copy(path, bak_path(path));
    }
    fs::rename(&tmp, path).map_err(|e| {
        let _ = fs::remove_file(&tmp);
        format!("CANVAS_IO: rename to {}: {e}", path.display())
    })
}

fn read_json(path: &Path) -> Result<Option<Value>, String> {
    let mut last_err = None;
    for p in [path.to_path_buf(), bak_path(path)] {
        match fs::read(&p) {
            Ok(bytes) => match serde_json::from_slice::<Value>(&bytes) {
                Ok(v) => {
                    if p != path {
                        tracing::warn!(
                            "[canvas] {} unreadable, recovered from .bak",
                            path.display()
                        );
                    }
                    return Ok(Some(v));
                }
                Err(e) => last_err = Some(format!("CANVAS_IO: parse {}: {e}", p.display())),
            },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => last_err = Some(format!("CANVAS_IO: read {}: {e}", p.display())),
        }
    }
    match last_err {
        Some(e) => Err(e),
        None => Ok(None),
    }
}

pub struct Loaded<T> {
    pub value: T,
    pub read_only: bool,
}

pub fn load_workspace(root: &Path, ws: &str) -> Result<Option<Loaded<Workspace>>, String> {
    let Some(mut v) = read_json(&workspace_file(root, ws))? else {
        return Ok(None);
    };
    let version = v.get("version").and_then(|x| x.as_u64()).unwrap_or(1) as u32;
    let read_only = version > WORKSPACE_VERSION;
    if !read_only {
        migrate_workspace(&mut v, version);
    }
    let value: Workspace =
        serde_json::from_value(v).map_err(|e| format!("CANVAS_IO: workspace {ws}: {e}"))?;
    Ok(Some(Loaded { value, read_only }))
}

pub fn save_workspace(root: &Path, meta: &Workspace) -> Result<(), String> {
    let bytes = serde_json::to_vec_pretty(meta).map_err(|e| e.to_string())?;
    write_atomic(&workspace_file(root, &meta.id), &bytes)
}

pub fn load_floor(root: &Path, ws: &str, floor: &str) -> Result<Option<Loaded<FloorDoc>>, String> {
    let Some(mut v) = read_json(&floor_file(root, ws, floor))? else {
        return Ok(None);
    };
    let version = v.get("version").and_then(|x| x.as_u64()).unwrap_or(1) as u32;
    let read_only = version > FLOOR_VERSION;
    if !read_only {
        migrate_floor(&mut v, version);
    }
    let value: FloorDoc =
        serde_json::from_value(v).map_err(|e| format!("CANVAS_IO: floor {ws}/{floor}: {e}"))?;
    Ok(Some(Loaded { value, read_only }))
}

pub fn floor_bytes(doc: &FloorDoc) -> Result<Vec<u8>, String> {
    serde_json::to_vec_pretty(doc).map_err(|e| e.to_string())
}

fn migrate_floor(v: &mut Value, from: u32) {
    if let Some(obj) = v.as_object_mut() {
        if from < FLOOR_VERSION {
            tracing::info!("[canvas] migrating floor from v{from} to v{FLOOR_VERSION}");
        }
        obj.insert("format".into(), Value::String(FLOOR_FORMAT.into()));
        obj.insert("version".into(), Value::from(FLOOR_VERSION));
    }
}

fn migrate_workspace(v: &mut Value, from: u32) {
    if let Some(obj) = v.as_object_mut() {
        if from < WORKSPACE_VERSION {
            tracing::info!("[canvas] migrating workspace from v{from} to v{WORKSPACE_VERSION}");
        }
        obj.insert("version".into(), Value::from(WORKSPACE_VERSION));
    }
}

pub fn list_workspace_ids(root: &Path) -> Vec<String> {
    let Ok(rd) = fs::read_dir(root) else {
        return Vec::new();
    };
    rd.flatten()
        .filter(|e| e.path().is_dir())
        .filter_map(|e| e.file_name().to_str().map(str::to_string))
        .filter(|n| !n.starts_with('.'))
        .filter(|n| {
            let f = workspace_file(root, n);
            f.exists() || bak_path(&f).exists()
        })
        .collect()
}

pub fn move_to_trash(root: &Path, ws: &str, path: &Path) -> Result<PathBuf, String> {
    let dir = trash_dir(root, ws);
    fs::create_dir_all(&dir).map_err(|e| format!("CANVAS_IO: {e}"))?;
    let name = path.file_name().ok_or("no file name")?;
    let dest = dir.join(name);
    fs::rename(path, &dest)
        .or_else(|_| fs::copy(path, &dest).and_then(|_| fs::remove_file(path)))
        .map_err(|e| format!("CANVAS_IO: trash {}: {e}", path.display()))?;
    Ok(dest)
}

pub fn purge_trash(root: &Path, ws: &str) {
    let Ok(rd) = fs::read_dir(trash_dir(root, ws)) else {
        return;
    };
    let now = SystemTime::now();
    for e in rd.flatten() {
        let old = e
            .metadata()
            .and_then(|m| m.modified())
            .ok()
            .and_then(|m| now.duration_since(m).ok())
            .is_some_and(|age| age > TRASH_TTL);
        if old {
            let p = e.path();
            let _ = if p.is_dir() {
                fs::remove_dir_all(&p)
            } else {
                fs::remove_file(&p)
            };
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum SaveKey {
    Workspace(String),
    Floor(String, String),
}

type SaveFn = dyn Fn(&SaveKey) -> bool + Send + Sync;

struct SaverState {
    due: HashMap<SaveKey, Instant>,
    stop: bool,
}

pub struct Saver {
    state: Arc<(Mutex<SaverState>, Condvar)>,
}

impl Saver {
    pub fn spawn(save: Box<SaveFn>) -> Self {
        let state = Arc::new((
            Mutex::new(SaverState {
                due: HashMap::new(),
                stop: false,
            }),
            Condvar::new(),
        ));
        let st = state.clone();
        let _ = std::thread::Builder::new()
            .name("canvas-saver".into())
            .spawn(move || loop {
                let ready: Vec<SaveKey> = {
                    let (lock, cv) = &*st;
                    let mut g = lock.lock().unwrap_or_else(|e| e.into_inner());
                    loop {
                        if g.stop {
                            return;
                        }
                        let now = Instant::now();
                        let next = g.due.values().min().copied();
                        match next {
                            None => {
                                g = cv.wait(g).unwrap_or_else(|e| e.into_inner());
                            }
                            Some(t) if t > now => {
                                g = cv
                                    .wait_timeout(g, t - now)
                                    .unwrap_or_else(|e| e.into_inner())
                                    .0;
                            }
                            Some(_) => break,
                        }
                    }
                    let now = Instant::now();
                    let keys: Vec<SaveKey> = g
                        .due
                        .iter()
                        .filter(|(_, t)| **t <= now)
                        .map(|(k, _)| k.clone())
                        .collect();
                    for k in &keys {
                        g.due.remove(k);
                    }
                    keys
                };
                for k in ready {
                    if !save(&k) {
                        return;
                    }
                }
            });
        Self { state }
    }

    pub fn schedule(&self, key: SaveKey, delay: Duration) {
        let (lock, cv) = &*self.state;
        let mut g = lock.lock().unwrap_or_else(|e| e.into_inner());
        let at = Instant::now() + delay;
        let e = g.due.entry(key).or_insert(at);
        if at < *e {
            *e = at;
        }
        cv.notify_one();
    }

    pub fn take_all(&self) -> Vec<SaveKey> {
        let (lock, _) = &*self.state;
        let mut g = lock.lock().unwrap_or_else(|e| e.into_inner());
        g.due.drain().map(|(k, _)| k).collect()
    }
}

impl Drop for Saver {
    fn drop(&mut self) {
        let (lock, cv) = &*self.state;
        lock.lock().unwrap_or_else(|e| e.into_inner()).stop = true;
        cv.notify_one();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::canvas::model::{Camera, CanvasNode};

    #[test]
    fn floor_round_trip_with_bak_recovery() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let mut doc = FloorDoc {
            rev: 7,
            camera: Camera {
                x: 1.0,
                y: 2.0,
                zoom: 0.5,
            },
            ..Default::default()
        };
        let node: CanvasNode = serde_json::from_value(serde_json::json!({
            "id": "n1", "kind": "portal", "x": 0, "y": 0, "w": 10, "h": 10, "z": 0,
            "data": {"portalKind": "web", "url": "https://example.com", "future": true}
        }))
        .unwrap();
        doc.nodes.push(node);
        let path = floor_file(root, "ws", "ground");
        write_atomic(&path, &floor_bytes(&doc).unwrap()).unwrap();
        let back = load_floor(root, "ws", "ground").unwrap().unwrap();
        assert!(!back.read_only);
        assert_eq!(back.value, doc);

        doc.rev = 8;
        write_atomic(&path, &floor_bytes(&doc).unwrap()).unwrap();
        fs::write(&path, b"{ broken").unwrap();
        let rec = load_floor(root, "ws", "ground").unwrap().unwrap();
        assert_eq!(rec.value.rev, 7);

        let mut v: Value = serde_json::from_slice(&floor_bytes(&doc).unwrap()).unwrap();
        v["version"] = Value::from(FLOOR_VERSION + 1);
        fs::write(&path, serde_json::to_vec(&v).unwrap()).unwrap();
        assert!(load_floor(root, "ws", "ground").unwrap().unwrap().read_only);
    }
}

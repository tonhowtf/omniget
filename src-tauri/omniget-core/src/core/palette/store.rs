use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use serde::Serialize;
use serde_json::Value;

const MAX_BYTES: usize = 8 * 1024 * 1024;

static TMP_SEQ: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct LoadOutcome {
    pub data: Option<Value>,
    pub moved_aside: Option<String>,
}

pub fn store_path() -> Option<PathBuf> {
    crate::core::paths::app_data_dir().map(|d| d.join("palette").join("projects.json"))
}

fn no_dir() -> String {
    "The app data folder could not be found, so palettes cannot be saved.".to_string()
}

pub fn load() -> Result<LoadOutcome, String> {
    load_from(&store_path().ok_or_else(no_dir)?)
}

pub fn save(data: &Value) -> Result<(), String> {
    save_to(&store_path().ok_or_else(no_dir)?, data)
}

fn valid_shape(data: &Value) -> bool {
    data.as_object()
        .and_then(|o| o.get("projects"))
        .is_some_and(Value::is_array)
}

fn stamp() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0)
}

pub fn load_from(path: &Path) -> Result<LoadOutcome, String> {
    let bytes = match std::fs::read(path) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Ok(LoadOutcome {
                data: None,
                moved_aside: None,
            })
        }
        Err(e) => return Err(format!("Saved palettes could not be read: {e}.")),
    };
    match serde_json::from_slice::<Value>(&bytes) {
        Ok(data) if valid_shape(&data) => Ok(LoadOutcome {
            data: Some(data),
            moved_aside: None,
        }),
        _ => {
            let aside = path.with_file_name(format!("projects.damaged-{}.json", stamp()));
            std::fs::rename(path, &aside).map_err(|e| {
                format!("Saved palettes are damaged and could not be moved aside: {e}.")
            })?;
            Ok(LoadOutcome {
                data: None,
                moved_aside: Some(aside.to_string_lossy().into_owned()),
            })
        }
    }
}

pub fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let dir = path
        .parent()
        .ok_or_else(|| "The destination has no parent folder.".to_string())?;
    std::fs::create_dir_all(dir).map_err(|e| format!("The folder could not be created: {e}."))?;
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "palette".to_string());
    let seq = TMP_SEQ.fetch_add(1, Ordering::Relaxed);
    let tmp = dir.join(format!(
        ".{name}.tmp-{}-{}-{seq}",
        std::process::id(),
        stamp()
    ));
    let result = (|| {
        let mut file = std::fs::File::create(&tmp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        std::fs::rename(&tmp, path)
    })();
    if let Err(e) = result {
        let _ = std::fs::remove_file(&tmp);
        return Err(format!("The file could not be written: {e}."));
    }
    Ok(())
}

pub fn save_to(path: &Path, data: &Value) -> Result<(), String> {
    if !valid_shape(data) {
        return Err(
            "The palettes are not in the expected shape, so nothing was saved.".to_string(),
        );
    }
    let bytes = serde_json::to_vec_pretty(data).map_err(|e| e.to_string())?;
    if bytes.len() > MAX_BYTES {
        return Err("The palettes are larger than 8 MB, so nothing was saved. Delete some projects and try again.".to_string());
    }
    write_atomic(path, &bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn missing_file_is_empty() {
        let dir = tempfile::tempdir().unwrap();
        let out = load_from(&dir.path().join("projects.json")).unwrap();
        assert_eq!(out.data, None);
        assert_eq!(out.moved_aside, None);
    }

    #[test]
    fn round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("palette").join("projects.json");
        let data = json!({"version": 1, "activeId": "a", "projects": [{"id": "a"}]});
        save_to(&path, &data).unwrap();
        assert_eq!(load_from(&path).unwrap().data, Some(data));
        let leftovers: Vec<_> = std::fs::read_dir(path.parent().unwrap())
            .unwrap()
            .filter_map(Result::ok)
            .filter(|e| e.file_name().to_string_lossy().contains(".tmp-"))
            .collect();
        assert!(leftovers.is_empty());
    }

    #[test]
    fn concurrent_saves_never_collide() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("projects.json");
        let handles: Vec<_> = (0..8)
            .map(|t| {
                let path = path.clone();
                std::thread::spawn(move || {
                    for i in 0..40 {
                        let data =
                            json!({"version": 1, "activeId": format!("{t}-{i}"), "projects": []});
                        save_to(&path, &data).unwrap();
                    }
                })
            })
            .collect();
        for h in handles {
            h.join().unwrap();
        }
        let out = load_from(&path).unwrap().data.unwrap();
        assert_eq!(out["version"], 1);
        let leftovers = std::fs::read_dir(dir.path())
            .unwrap()
            .filter_map(Result::ok)
            .filter(|e| e.file_name().to_string_lossy().contains(".tmp-"))
            .count();
        assert_eq!(leftovers, 0);
    }

    #[test]
    fn rejects_wrong_shape() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("projects.json");
        assert!(save_to(&path, &json!({"projects": 3})).is_err());
        assert!(save_to(&path, &json!([1, 2])).is_err());
        assert!(!path.exists());
    }

    #[test]
    fn damaged_file_is_moved_aside() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("projects.json");
        std::fs::write(&path, b"{not json").unwrap();
        let out = load_from(&path).unwrap();
        assert_eq!(out.data, None);
        let aside = out.moved_aside.unwrap();
        assert!(Path::new(&aside).exists());
        assert!(!path.exists());
    }
}

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;

use super::names;
use super::{BoardError, MAX_SCENE_BYTES, MAX_THUMB_BYTES, THUMBS_DIR};

const TRASH_DIR: &str = ".trash";

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct BoardEntry {
    pub name: String,
    pub path: String,
    pub modified_ms: u64,
    pub size: u64,
    pub has_thumb: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct SavedInfo {
    pub modified_ms: u64,
    pub size: u64,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Deleted {
    pub method: &'static str,
    pub restore_path: Option<String>,
}

pub fn empty_scene() -> String {
    serde_json::json!({
        "type": "excalidraw",
        "version": 2,
        "source": "omniget",
        "elements": [],
        "appState": { "gridSize": 20, "viewBackgroundColor": "#ffffff" },
        "files": {}
    })
    .to_string()
}

pub fn validate_scene(bytes: &[u8]) -> Result<(), BoardError> {
    if bytes.len() as u64 > MAX_SCENE_BYTES {
        return Err(BoardError::TooLarge);
    }
    let value: serde_json::Value =
        serde_json::from_slice(bytes).map_err(|_| BoardError::NotScene)?;
    let obj = value.as_object().ok_or(BoardError::NotScene)?;
    let typed = match obj.get("type") {
        None => true,
        Some(t) => t.as_str() == Some("excalidraw"),
    };
    if !typed || !obj.get("elements").is_some_and(|e| e.is_array()) {
        return Err(BoardError::NotScene);
    }
    Ok(())
}

pub fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), BoardError> {
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .ok_or(BoardError::Outside)?;
    fs::create_dir_all(parent)?;
    let base = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let tmp = parent.join(format!(".{base}.{}.tmp", uuid::Uuid::new_v4().simple()));
    let result = (|| -> std::io::Result<()> {
        let mut file = fs::File::create(&tmp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        fs::rename(&tmp, path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result.map_err(BoardError::from)
}

fn millis(time: std::io::Result<SystemTime>) -> u64 {
    time.ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn thumb_path(dir: &Path, name: &str) -> PathBuf {
    dir.join(THUMBS_DIR).join(format!("{name}.png"))
}

fn board_path(dir: &Path, name: &str) -> PathBuf {
    dir.join(names::file_name(name))
}

fn exists_ci(dir: &Path, name: &str) -> bool {
    let wanted = names::file_name(name).to_lowercase();
    if board_path(dir, name).exists() {
        return true;
    }
    fs::read_dir(dir)
        .map(|rd| {
            rd.flatten()
                .any(|e| e.file_name().to_string_lossy().to_lowercase() == wanted)
        })
        .unwrap_or(false)
}

fn other_entry_named(dir: &Path, wanted: &str, source: &str) -> bool {
    wanted != source
        && fs::read_dir(dir)
            .map(|rd| {
                rd.flatten()
                    .any(|e| e.file_name().to_string_lossy() == wanted)
            })
            .unwrap_or(false)
}

fn parked_thumb(parked: &Path) -> Option<PathBuf> {
    let file = parked.file_name()?.to_string_lossy().into_owned();
    let stem = names::stem_of(&file)?;
    Some(parked.with_file_name(format!("{stem}.png")))
}

fn entry_for(dir: &Path, path: &Path) -> Result<BoardEntry, BoardError> {
    let file = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .ok_or(BoardError::NotFound)?;
    let name = names::stem_of(&file)
        .ok_or(BoardError::NotFound)?
        .to_string();
    let meta = fs::metadata(path).map_err(|_| BoardError::NotFound)?;
    Ok(BoardEntry {
        has_thumb: thumb_path(dir, &name).is_file(),
        path: path.to_string_lossy().into_owned(),
        modified_ms: millis(meta.modified()),
        size: meta.len(),
        name,
    })
}

pub fn ensure_dir(dir: &Path) -> Result<(), BoardError> {
    fs::create_dir_all(dir)?;
    Ok(())
}

pub fn list(dir: &Path) -> Result<Vec<BoardEntry>, BoardError> {
    if !dir.exists() {
        return Ok(Vec::new());
    }
    let mut out: Vec<BoardEntry> = fs::read_dir(dir)?
        .flatten()
        .filter(|e| e.file_type().map(|t| t.is_file()).unwrap_or(false))
        .filter(|e| {
            let n = e.file_name().to_string_lossy().into_owned();
            !n.starts_with('.') && names::stem_of(&n).is_some()
        })
        .filter_map(|e| entry_for(dir, &e.path()).ok())
        .collect();
    out.sort_by(|a, b| {
        b.modified_ms
            .cmp(&a.modified_ms)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    Ok(out)
}

pub fn resolve(dir: &Path, path: &Path) -> Result<PathBuf, BoardError> {
    let root = fs::canonicalize(dir).map_err(|_| BoardError::NoFolder)?;
    let parent = path.parent().ok_or(BoardError::Outside)?;
    let parent = fs::canonicalize(parent).map_err(|_| BoardError::Outside)?;
    if parent != root {
        return Err(BoardError::Outside);
    }
    let file = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .ok_or(BoardError::Outside)?;
    let stem = names::stem_of(&file).ok_or(BoardError::Outside)?;
    if stem.starts_with('.') || names::validate(stem).is_err() {
        return Err(BoardError::Outside);
    }
    Ok(root.join(file))
}

pub fn create(dir: &Path, name: Option<&str>, fallback: &str) -> Result<BoardEntry, BoardError> {
    ensure_dir(dir)?;
    let base = match name.map(str::trim).filter(|n| !n.is_empty()) {
        Some(n) => names::validate(n)?,
        None => names::validate(fallback).unwrap_or_else(|_| "Untitled".into()),
    };
    let name = names::unique(&base, |n| exists_ci(dir, n));
    let path = board_path(dir, &name);
    atomic_write(&path, empty_scene().as_bytes())?;
    entry_for(dir, &path)
}

pub fn read(path: &Path) -> Result<String, BoardError> {
    let meta = fs::metadata(path).map_err(|_| BoardError::NotFound)?;
    if meta.len() > MAX_SCENE_BYTES {
        return Err(BoardError::TooLarge);
    }
    let bytes = fs::read(path)?;
    validate_scene(&bytes)?;
    String::from_utf8(bytes).map_err(|_| BoardError::NotScene)
}

pub fn save(path: &Path, contents: &str) -> Result<SavedInfo, BoardError> {
    validate_scene(contents.as_bytes())?;
    atomic_write(path, contents.as_bytes())?;
    let meta = fs::metadata(path)?;
    Ok(SavedInfo {
        modified_ms: millis(meta.modified()),
        size: meta.len(),
    })
}

pub fn write_thumb(dir: &Path, name: &str, png: &[u8]) -> Result<(), BoardError> {
    if png.len() > MAX_THUMB_BYTES || !png.starts_with(b"\x89PNG") {
        return Err(BoardError::NotScene);
    }
    atomic_write(&thumb_path(dir, name), png)
}

pub fn read_thumb(dir: &Path, name: &str) -> Option<Vec<u8>> {
    fs::read(thumb_path(dir, name)).ok()
}

pub fn rename(dir: &Path, path: &Path, new_name: &str) -> Result<BoardEntry, BoardError> {
    let file = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .ok_or(BoardError::NotFound)?;
    let old = names::stem_of(&file)
        .ok_or(BoardError::NotFound)?
        .to_string();
    let new = names::validate(new_name)?;
    if !path.is_file() {
        return Err(BoardError::NotFound);
    }
    if new == old {
        return entry_for(dir, path);
    }
    let target = board_path(dir, &new);
    let case_only = new.to_lowercase() == old.to_lowercase();
    if case_only {
        if other_entry_named(dir, &names::file_name(&new), &file) {
            return Err(BoardError::Exists);
        }
    } else if exists_ci(dir, &new) {
        return Err(BoardError::Exists);
    }
    fs::rename(path, &target)?;
    let old_thumb = thumb_path(dir, &old);
    if old_thumb.is_file() {
        let _ = fs::rename(&old_thumb, thumb_path(dir, &new));
    }
    entry_for(dir, &target)
}

pub fn duplicate(dir: &Path, path: &Path, suffix: &str) -> Result<BoardEntry, BoardError> {
    let file = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .ok_or(BoardError::NotFound)?;
    let old = names::stem_of(&file)
        .ok_or(BoardError::NotFound)?
        .to_string();
    let contents = read(path)?;
    let base = names::validate(&format!("{old} {suffix}")).unwrap_or_else(|_| old.clone());
    let name = names::unique(&base, |n| exists_ci(dir, n));
    let target = board_path(dir, &name);
    atomic_write(&target, contents.as_bytes())?;
    if let Some(png) = read_thumb(dir, &old) {
        let _ = atomic_write(&thumb_path(dir, &name), &png);
    }
    entry_for(dir, &target)
}

pub fn delete(dir: &Path, path: &Path, use_os_trash: bool) -> Result<Deleted, BoardError> {
    let file = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .ok_or(BoardError::NotFound)?;
    let name = names::stem_of(&file)
        .ok_or(BoardError::NotFound)?
        .to_string();
    if !path.is_file() {
        return Err(BoardError::NotFound);
    }
    let thumb = thumb_path(dir, &name);
    if use_os_trash && trash::delete(path).is_ok() {
        let _ = fs::remove_file(&thumb);
        return Ok(Deleted {
            method: "os",
            restore_path: None,
        });
    }
    let bin = dir.join(TRASH_DIR);
    fs::create_dir_all(&bin)?;
    let stamp = millis(Ok(SystemTime::now()));
    let parked = bin.join(format!("{stamp}-{file}"));
    fs::rename(path, &parked)?;
    if thumb.is_file() {
        if let Some(target) = parked_thumb(&parked) {
            if fs::rename(&thumb, &target).is_err() {
                let _ = fs::remove_file(&thumb);
            }
        }
    }
    Ok(Deleted {
        method: "folder",
        restore_path: Some(parked.to_string_lossy().into_owned()),
    })
}

pub fn restore(dir: &Path, parked: &Path) -> Result<BoardEntry, BoardError> {
    let bin = fs::canonicalize(dir.join(TRASH_DIR)).map_err(|_| BoardError::NotFound)?;
    let parked = fs::canonicalize(parked).map_err(|_| BoardError::NotFound)?;
    if parked.parent() != Some(bin.as_path()) {
        return Err(BoardError::Outside);
    }
    let file = parked
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .ok_or(BoardError::NotFound)?;
    let original = file.split_once('-').map(|(_, rest)| rest).unwrap_or(&file);
    let stem = names::stem_of(original).ok_or(BoardError::NotFound)?;
    let name = names::unique(stem, |n| exists_ci(dir, n));
    let target = board_path(dir, &name);
    fs::rename(&parked, &target)?;
    if let Some(thumb) = parked_thumb(&parked).filter(|t| t.is_file()) {
        let dest = thumb_path(dir, &name);
        let moved = dest.parent().is_some_and(|p| fs::create_dir_all(p).is_ok())
            && fs::rename(&thumb, &dest).is_ok();
        if !moved {
            let _ = fs::remove_file(&thumb);
        }
    }
    entry_for(dir, &target)
}

pub fn import(dir: &Path, source: &Path) -> Result<BoardEntry, BoardError> {
    let contents = read(source)?;
    ensure_dir(dir)?;
    let file = source
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let stem = names::stem_of(&file).unwrap_or(&file);
    let base = names::validate(stem).unwrap_or_else(|_| "Imported".into());
    let name = names::unique(&base, |n| exists_ci(dir, n));
    let target = board_path(dir, &name);
    atomic_write(&target, contents.as_bytes())?;
    entry_for(dir, &target)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scene(marker: &str) -> String {
        serde_json::json!({ "type": "excalidraw", "version": 2, "elements": [{ "id": marker }], "appState": {}, "files": {} }).to_string()
    }

    #[test]
    fn create_list_and_sort() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        let a = create(dir, None, "Untitled").unwrap();
        let b = create(dir, None, "Untitled").unwrap();
        assert_eq!(a.name, "Untitled");
        assert_eq!(b.name, "Untitled 2");
        fs::write(dir.join(".hidden.excalidraw"), empty_scene()).unwrap();
        fs::write(dir.join("notes.txt"), "x").unwrap();
        let listed = list(dir).unwrap();
        assert_eq!(listed.len(), 2);
        assert!(listed[0].modified_ms >= listed[1].modified_ms);
        assert!(list(&dir.join("missing")).unwrap().is_empty());
    }

    #[test]
    fn save_is_atomic_and_validated() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        let entry = create(dir, Some("Plan"), "Untitled").unwrap();
        let path = PathBuf::from(&entry.path);
        let info = save(&path, &scene("one")).unwrap();
        assert!(info.size > 0);
        assert!(read(&path).unwrap().contains("one"));
        assert!(matches!(save(&path, "not json"), Err(BoardError::NotScene)));
        assert!(matches!(
            save(&path, r#"{"type":"other","elements":[]}"#),
            Err(BoardError::NotScene)
        ));
        assert!(read(&path).unwrap().contains("one"));
        let leftovers = fs::read_dir(dir)
            .unwrap()
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().ends_with(".tmp"))
            .count();
        assert_eq!(leftovers, 0);
    }

    #[test]
    fn resolve_keeps_paths_inside_folder() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("boards");
        fs::create_dir_all(&dir).unwrap();
        let entry = create(&dir, Some("Inside"), "Untitled").unwrap();
        assert!(resolve(&dir, Path::new(&entry.path)).is_ok());
        let outside = tmp.path().join("Outside.excalidraw");
        fs::write(&outside, empty_scene()).unwrap();
        assert!(matches!(resolve(&dir, &outside), Err(BoardError::Outside)));
        let sneaky = dir.join("..").join("Outside.excalidraw");
        assert!(matches!(resolve(&dir, &sneaky), Err(BoardError::Outside)));
        assert!(matches!(
            resolve(&dir, &dir.join("notes.txt")),
            Err(BoardError::Outside)
        ));
        assert!(matches!(
            resolve(&dir, &dir.join(".thumbs").join("x.excalidraw")),
            Err(BoardError::Outside)
        ));
    }

    #[test]
    fn rename_moves_thumb_and_refuses_overwrite() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        let a = create(dir, Some("Alpha"), "Untitled").unwrap();
        create(dir, Some("Beta"), "Untitled").unwrap();
        write_thumb(dir, "Alpha", b"\x89PNG fake").unwrap();
        let path = PathBuf::from(&a.path);
        assert!(matches!(
            rename(dir, &path, "Beta"),
            Err(BoardError::Exists)
        ));
        assert!(matches!(
            rename(dir, &path, "beta"),
            Err(BoardError::Exists)
        ));
        assert!(matches!(
            rename(dir, &path, "../evil"),
            Err(BoardError::NameInvalid)
        ));
        let renamed = rename(dir, &path, "Gamma").unwrap();
        assert_eq!(renamed.name, "Gamma");
        assert!(renamed.has_thumb);
        assert!(!path.exists());
        let cased = rename(dir, Path::new(&renamed.path), "GAMMA").unwrap();
        assert_eq!(cased.name, "GAMMA");
        assert_eq!(list(dir).unwrap().len(), 2);
    }

    #[test]
    fn case_only_rename_never_overwrites_a_sibling() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        let map = create(dir, Some("Map"), "Untitled").unwrap();
        save(Path::new(&map.path), &scene("lower")).unwrap();
        let upper = dir.join("MAP.excalidraw");
        let sensitive = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&upper)
            .is_ok();
        if !sensitive {
            return;
        }
        fs::write(&upper, scene("upper")).unwrap();
        assert!(matches!(
            rename(dir, Path::new(&map.path), "MAP"),
            Err(BoardError::Exists)
        ));
        assert!(read(&upper).unwrap().contains("upper"));
        assert!(read(Path::new(&map.path)).unwrap().contains("lower"));
    }

    #[test]
    fn mixed_case_extension_is_listed() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        fs::write(dir.join("Plan.Excalidraw"), empty_scene()).unwrap();
        let listed = list(dir).unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].name, "Plan");
        let other = tempfile::tempdir().unwrap();
        let imported = import(other.path(), &dir.join("Plan.Excalidraw")).unwrap();
        assert_eq!(imported.name, "Plan");
    }

    #[test]
    fn duplicate_and_import_pick_free_names() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("lib");
        let a = create(&dir, Some("Map"), "Untitled").unwrap();
        save(Path::new(&a.path), &scene("dup")).unwrap();
        let copy = duplicate(&dir, Path::new(&a.path), "copy").unwrap();
        assert_eq!(copy.name, "Map copy");
        assert!(read(Path::new(&copy.path)).unwrap().contains("dup"));
        let outside = tmp.path().join("Map.excalidraw");
        fs::write(&outside, scene("ext")).unwrap();
        let imported = import(&dir, &outside).unwrap();
        assert_eq!(imported.name, "Map 2");
        let junk = tmp.path().join("junk.excalidraw");
        fs::write(&junk, "{}").unwrap();
        assert!(matches!(import(&dir, &junk), Err(BoardError::NotScene)));
    }

    #[test]
    fn delete_to_folder_trash_and_restore() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        let a = create(dir, Some("Gone"), "Untitled").unwrap();
        write_thumb(dir, "Gone", b"\x89PNG fake").unwrap();
        let deleted = delete(dir, Path::new(&a.path), false).unwrap();
        assert_eq!(deleted.method, "folder");
        assert!(list(dir).unwrap().is_empty());
        assert!(!dir.join(THUMBS_DIR).join("Gone.png").exists());
        let parked = PathBuf::from(deleted.restore_path.unwrap());
        let back = restore(dir, &parked).unwrap();
        assert_eq!(back.name, "Gone");
        assert!(back.has_thumb);
        assert_eq!(read_thumb(dir, "Gone").unwrap(), b"\x89PNG fake");
        assert!(matches!(
            delete(dir, &dir.join("nope.excalidraw"), false),
            Err(BoardError::NotFound)
        ));
        let elsewhere = tmp.path().join("x.excalidraw");
        fs::write(&elsewhere, empty_scene()).unwrap();
        assert!(matches!(restore(dir, &elsewhere), Err(BoardError::Outside)));
    }
}

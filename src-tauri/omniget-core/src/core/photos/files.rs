use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::library::Library;

#[derive(Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum RemoveMode {
    Trash,
    Delete,
}

#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct Failure {
    pub path: String,
    pub error: String,
}

#[derive(Serialize, Clone, Debug, Default, PartialEq)]
pub struct FileReport {
    pub done: Vec<String>,
    pub moved_to: Vec<String>,
    pub failed: Vec<Failure>,
}

fn known(lib: &Library, path: &str) -> bool {
    matches!(lib.format_of(path), Ok(Some(_)))
}

fn unknown(path: &str) -> Failure {
    Failure {
        path: path.to_string(),
        error: "This photo is not in the library anymore.".to_string(),
    }
}

pub fn remove(lib: &Library, paths: &[String], mode: RemoveMode) -> FileReport {
    let mut report = FileReport::default();
    for path in paths {
        if !known(lib, path) {
            report.failed.push(unknown(path));
            continue;
        }
        let p = Path::new(path);
        let result = if !p.exists() {
            Ok(())
        } else {
            match mode {
                RemoveMode::Trash => trash::delete(p)
                    .map_err(|e| format!("This photo could not be moved to the trash: {e}.")),
                RemoveMode::Delete => std::fs::remove_file(p)
                    .map_err(|e| format!("This photo could not be deleted: {e}.")),
            }
        };
        match result {
            Ok(()) => report.done.push(path.clone()),
            Err(error) => report.failed.push(Failure {
                path: path.clone(),
                error,
            }),
        }
    }
    let _ = lib.forget_paths(&report.done);
    report
}

pub fn free_name(dir: &Path, name: &str) -> PathBuf {
    let p = Path::new(name);
    let stem = p
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "photo".to_string());
    let ext = p.extension().map(|e| e.to_string_lossy().into_owned());
    let mut n = 0u32;
    loop {
        let base = if n == 0 {
            stem.clone()
        } else {
            format!("{stem} ({n})")
        };
        let candidate = dir.join(match &ext {
            Some(e) => format!("{base}.{e}"),
            None => base,
        });
        if !candidate.exists() {
            return candidate;
        }
        n += 1;
    }
}

fn move_one(from: &Path, dir: &Path) -> Result<PathBuf, String> {
    let name = from
        .file_name()
        .ok_or_else(|| "This photo has no file name.".to_string())?
        .to_string_lossy()
        .into_owned();
    let same = |a: &Path, b: &Path| match (a.canonicalize(), b.canonicalize()) {
        (Ok(a), Ok(b)) => a == b,
        _ => a == b,
    };
    if from.parent().is_some_and(|p| same(p, dir)) {
        return Err("This photo is already in that folder.".to_string());
    }
    let to = free_name(dir, &name);
    if std::fs::rename(from, &to).is_ok() {
        return Ok(to);
    }
    if let Err(e) = std::fs::copy(from, &to) {
        let _ = std::fs::remove_file(&to);
        return Err(format!("This photo could not be moved: {e}."));
    }
    if let Err(e) = std::fs::remove_file(from) {
        let _ = std::fs::remove_file(&to);
        return Err(format!("This photo could not be moved: {e}."));
    }
    Ok(to)
}

pub fn move_to(lib: &Library, paths: &[String], dir: &Path, roots: &[PathBuf]) -> FileReport {
    let mut report = FileReport::default();
    if !dir.is_dir() {
        report.failed = paths
            .iter()
            .map(|p| Failure {
                path: p.clone(),
                error: "The destination folder doesn't exist.".to_string(),
            })
            .collect();
        return report;
    }
    let inside = roots.iter().any(|r| dir.starts_with(r));
    for path in paths {
        if !known(lib, path) {
            report.failed.push(unknown(path));
            continue;
        }
        match move_one(Path::new(path), dir) {
            Ok(to) => {
                let _ = if inside {
                    lib.relocate(path, &to)
                } else {
                    lib.forget_paths(std::slice::from_ref(path))
                };
                report.done.push(path.clone());
                report.moved_to.push(to.to_string_lossy().into_owned());
            }
            Err(error) => report.failed.push(Failure {
                path: path.clone(),
                error,
            }),
        }
    }
    report
}

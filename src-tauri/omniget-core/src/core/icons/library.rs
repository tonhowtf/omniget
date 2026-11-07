use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::cache::write_atomic;
use super::{api, IconData, IconsError};

pub const LIBRARY_VERSION: u32 = 1;
const MAX_COLLECTIONS: usize = 200;
const MAX_ICONS: usize = 5000;
const MAX_BODY: usize = 200_000;
const MAX_NAME: usize = 80;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SavedIcon {
    pub prefix: String,
    pub icon: IconData,
    #[serde(default)]
    pub set_name: Option<String>,
    #[serde(default)]
    pub license: Option<String>,
    #[serde(default)]
    pub license_spdx: Option<String>,
    #[serde(default)]
    pub added_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct NamedCollection {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub created_at: i64,
    #[serde(default)]
    pub icons: Vec<SavedIcon>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Library {
    pub version: u32,
    #[serde(default)]
    pub favorites: Vec<SavedIcon>,
    #[serde(default)]
    pub collections: Vec<NamedCollection>,
}

impl Default for Library {
    fn default() -> Self {
        Self {
            version: LIBRARY_VERSION,
            favorites: Vec::new(),
            collections: Vec::new(),
        }
    }
}

pub fn library_path() -> Option<PathBuf> {
    super::cache::icons_dir().map(|d| d.join("library.json"))
}

fn check_icon(saved: &SavedIcon) -> Result<(), IconsError> {
    if !api::valid_prefix(&saved.prefix) || !api::valid_name(&saved.icon.name) {
        return Err(IconsError::invalid(
            "bad_icon_id",
            format!(
                "\"{}:{}\" is not a valid icon id.",
                saved.prefix, saved.icon.name
            ),
        )
        .with("id", format!("{}:{}", saved.prefix, saved.icon.name)));
    }
    if saved.icon.body.len() > MAX_BODY {
        return Err(IconsError::invalid(
            "icon_too_large",
            format!("The icon \"{}\" is too large to save.", saved.icon.name),
        )
        .with("name", &saved.icon.name));
    }
    Ok(())
}

pub fn validate(lib: &mut Library) -> Result<(), IconsError> {
    lib.version = LIBRARY_VERSION;
    if lib.collections.len() > MAX_COLLECTIONS {
        return Err(IconsError::invalid(
            "too_many_collections",
            format!("You can keep up to {MAX_COLLECTIONS} collections."),
        )
        .with("max", MAX_COLLECTIONS));
    }
    let total = lib.favorites.len() + lib.collections.iter().map(|c| c.icons.len()).sum::<usize>();
    if total > MAX_ICONS {
        return Err(IconsError::invalid(
            "too_many_icons",
            format!("You can keep up to {MAX_ICONS} saved icons."),
        )
        .with("max", MAX_ICONS));
    }
    for saved in lib.favorites.iter() {
        check_icon(saved)?;
    }
    for c in lib.collections.iter_mut() {
        let name: String = c.name.trim().chars().take(MAX_NAME).collect();
        if name.is_empty() {
            return Err(IconsError::invalid(
                "collection_name",
                "A collection needs a name.",
            ));
        }
        c.name = name;
        if c.id.is_empty() || c.id.len() > 64 {
            return Err(IconsError::invalid(
                "collection_id",
                "A collection has an invalid id.",
            ));
        }
        for saved in c.icons.iter() {
            check_icon(saved)?;
        }
    }
    Ok(())
}

pub fn load_from(path: &Path) -> Library {
    match std::fs::read_to_string(path) {
        Ok(text) => match serde_json::from_str::<Library>(&text) {
            Ok(lib) => lib,
            Err(_) => {
                let backup =
                    path.with_extension(format!("corrupt-{}.json", chrono::Utc::now().timestamp()));
                let _ = std::fs::rename(path, backup);
                Library::default()
            }
        },
        Err(_) => Library::default(),
    }
}

pub fn save_to(path: &Path, mut lib: Library) -> Result<Library, IconsError> {
    validate(&mut lib)?;
    let bytes = serde_json::to_vec_pretty(&lib).map_err(|e| IconsError::io(e.to_string()))?;
    write_atomic(path, &bytes)?;
    Ok(lib)
}

pub fn safe_file_stem(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '_' || c == ' ' {
                c
            } else {
                '-'
            }
        })
        .collect();
    let trimmed = cleaned.trim().trim_matches('-').trim();
    let short: String = trimmed.chars().take(MAX_NAME).collect();
    if short.is_empty() {
        "icon".to_string()
    } else {
        short
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct ExportFile {
    pub name: String,
    pub svg: String,
}

pub fn write_folder(
    dir: &Path,
    folder: &str,
    files: &[ExportFile],
) -> Result<(PathBuf, usize), IconsError> {
    if files.is_empty() {
        return Err(IconsError::invalid(
            "export_empty",
            "This collection has no icons to export.",
        ));
    }
    let base = dir.join(safe_file_stem(folder));
    let mut target = base.clone();
    let mut n = 2;
    while target.exists() {
        target = PathBuf::from(format!("{} {n}", base.to_string_lossy()));
        n += 1;
    }
    std::fs::create_dir_all(&target)?;
    let mut used = std::collections::HashSet::new();
    for f in files {
        let stem = safe_file_stem(&f.name);
        let mut name = format!("{stem}.svg");
        let mut i = 2;
        while !used.insert(name.clone()) {
            name = format!("{stem}-{i}.svg");
            i += 1;
        }
        write_atomic(&target.join(name), f.svg.as_bytes())?;
    }
    Ok((target, files.len()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn saved(prefix: &str, name: &str) -> SavedIcon {
        SavedIcon {
            prefix: prefix.into(),
            icon: IconData {
                name: name.into(),
                body: "<path/>".into(),
                width: 24.0,
                height: 24.0,
                left: 0.0,
                top: 0.0,
                rotate: 0,
                h_flip: false,
                v_flip: false,
            },
            set_name: Some("Lucide".into()),
            license: Some("ISC".into()),
            license_spdx: Some("ISC".into()),
            added_at: 1,
        }
    }

    #[test]
    fn missing_file_loads_default() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(
            load_from(&dir.path().join("library.json")),
            Library::default()
        );
    }

    #[test]
    fn roundtrip_keeps_bodies() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("icons/library.json");
        let lib = Library {
            version: 0,
            favorites: vec![saved("lucide", "house")],
            collections: vec![NamedCollection {
                id: "c1".into(),
                name: "  Marketing site  ".into(),
                created_at: 2,
                icons: vec![saved("ph", "star")],
            }],
        };
        let stored = save_to(&path, lib).unwrap();
        assert_eq!(stored.version, LIBRARY_VERSION);
        assert_eq!(stored.collections[0].name, "Marketing site");
        let back = load_from(&path);
        assert_eq!(back, stored);
        assert_eq!(back.favorites[0].icon.body, "<path/>");
    }

    #[test]
    fn rejects_invalid_entries() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("library.json");
        let mut lib = Library::default();
        lib.favorites.push(saved("../evil", "x"));
        assert!(save_to(&path, lib).is_err());
        let mut lib = Library::default();
        lib.collections.push(NamedCollection {
            id: "a".into(),
            name: "   ".into(),
            created_at: 0,
            icons: vec![],
        });
        assert!(save_to(&path, lib).is_err());
        assert!(!path.exists());
    }

    #[test]
    fn corrupt_file_is_backed_up() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("library.json");
        std::fs::write(&path, "{broken").unwrap();
        assert_eq!(load_from(&path), Library::default());
        assert!(!path.exists());
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    }

    #[test]
    fn safe_stems() {
        assert_eq!(safe_file_stem("lucide:arrow-left"), "lucide-arrow-left");
        assert_eq!(safe_file_stem("../../etc/passwd"), "etc-passwd");
        assert_eq!(safe_file_stem("///"), "icon");
    }

    #[test]
    fn writes_folder_with_unique_names() {
        let dir = tempfile::tempdir().unwrap();
        let files = vec![
            ExportFile {
                name: "house".into(),
                svg: "<svg/>".into(),
            },
            ExportFile {
                name: "house".into(),
                svg: "<svg/>".into(),
            },
            ExportFile {
                name: "../up".into(),
                svg: "<svg/>".into(),
            },
        ];
        let (out, count) = write_folder(dir.path(), "My set", &files).unwrap();
        assert_eq!(count, 3);
        assert!(out.join("house.svg").exists());
        assert!(out.join("house-2.svg").exists());
        assert!(out.join("up.svg").exists());
        let (again, _) = write_folder(dir.path(), "My set", &files).unwrap();
        assert_ne!(again, out);
        assert!(write_folder(dir.path(), "x", &[]).is_err());
    }
}

use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Guard {
    Rule,
    Picked,
}

fn home() -> PathBuf {
    dirs::home_dir().unwrap_or_else(|| PathBuf::from("/nonexistent-home"))
}

fn roots_never_removed(home: &Path) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = vec![home.to_path_buf()];
    for d in [
        "Library",
        "Library/Caches",
        "Library/Logs",
        "Library/Application Support",
        "Library/Containers",
        "Library/Group Containers",
        ".cache",
        ".local",
        ".local/share",
        ".config",
        "AppData",
        "AppData/Local",
        "AppData/Roaming",
        "AppData/Local/Temp",
        "Downloads",
        "Applications",
        "Pictures",
        "Music",
        "Movies",
        "Videos",
    ] {
        v.push(home.join(d));
    }
    if cfg!(windows) {
        for d in [
            "C:/",
            "C:/Windows",
            "C:/Windows/Temp",
            "C:/Program Files",
            "C:/Program Files (x86)",
            "C:/ProgramData",
            "C:/Users",
        ] {
            v.push(PathBuf::from(d));
        }
    } else {
        for d in [
            "/",
            "/Applications",
            "/Library",
            "/Library/Caches",
            "/Library/Logs",
            "/System",
            "/Users",
            "/Volumes",
            "/home",
            "/tmp",
            "/private",
            "/private/tmp",
            "/private/var",
            "/var",
            "/var/tmp",
            "/var/log",
            "/var/cache",
            "/opt",
            "/usr",
            "/etc",
            "/bin",
            "/sbin",
            "/boot",
            "/root",
            "/snap",
            "/mnt",
            "/media",
            "/run",
            "/proc",
            "/sys",
            "/dev",
        ] {
            v.push(PathBuf::from(d));
        }
    }
    v
}

fn never_enter(home: &Path) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = [
        ".ssh",
        ".gnupg",
        ".aws",
        ".kube",
        ".password-store",
        "Library/Keychains",
        "Library/Mobile Documents",
        "Library/CloudStorage",
        "Library/Mail",
        "Library/Messages",
        "Library/Photos",
        "Library/Application Support/MobileSync",
        ".local/share/keyrings",
        "AppData/Roaming/Microsoft/Credentials",
        "AppData/Local/Microsoft/Credentials",
        "AppData/Roaming/Microsoft/Protect",
    ]
    .iter()
    .map(|d| home.join(d))
    .collect();
    if cfg!(windows) {
        for d in [
            "C:/Windows/System32",
            "C:/Windows/SysWOW64",
            "C:/Windows/WinSxS",
            "C:/Windows/Boot",
            "C:/Boot",
            "C:/Recovery",
            "C:/System Volume Information",
        ] {
            v.push(PathBuf::from(d));
        }
    } else {
        for d in [
            "/System",
            "/usr",
            "/bin",
            "/sbin",
            "/boot",
            "/etc",
            "/proc",
            "/sys",
            "/dev",
            "/private/etc",
            "/private/var/db",
            "/Library/Keychains",
            "/var/lib/dpkg",
            "/var/lib/rpm",
            "/var/lib/pacman",
        ] {
            v.push(PathBuf::from(d));
        }
    }
    v
}

fn rule_never_enters(home: &Path) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = [
        "Documents",
        "Desktop",
        "Dropbox",
        "OneDrive",
        "Google Drive",
        "iCloud Drive",
        "Pictures",
        "Music",
        "Movies",
        "Videos",
        "Library/Application Support/AddressBook",
        "Library/Calendars",
        "Library/Safari/Bookmarks.plist",
        ".mozilla/firefox/profiles.ini",
    ]
    .iter()
    .map(|d| home.join(d))
    .collect();
    for extra in crate::core::washmybelly::settings().protected_paths_extra {
        if !extra.trim().is_empty() {
            v.push(PathBuf::from(extra));
        }
    }
    v
}

pub fn normalize(p: &Path) -> Option<PathBuf> {
    let mut out = PathBuf::new();
    for c in p.components() {
        match c {
            Component::ParentDir => return None,
            Component::CurDir => {}
            other => out.push(other.as_os_str()),
        }
    }
    Some(out)
}

fn fold(p: &Path) -> String {
    let s = p.to_string_lossy().replace('\\', "/");
    let s = s.trim_end_matches('/').to_string();
    if cfg!(any(windows, target_os = "macos")) {
        s.to_lowercase()
    } else {
        s
    }
}

fn same(a: &Path, b: &Path) -> bool {
    fold(a) == fold(b)
}

fn inside(child: &Path, parent: &Path) -> bool {
    let c = fold(child);
    let p = fold(parent);
    if p.is_empty() {
        return true;
    }
    c == p || c.starts_with(&format!("{p}/"))
}

pub fn check(path: &Path, guard: Guard, root: Option<&Path>) -> Result<(), String> {
    check_with(path, guard, root, &home())
}

pub fn check_with(
    path: &Path,
    guard: Guard,
    root: Option<&Path>,
    home: &Path,
) -> Result<(), String> {
    if !path.is_absolute() {
        return Err("PROTECTED: relative path".into());
    }
    let path = normalize(path).ok_or("PROTECTED: path with ..")?;
    for r in roots_never_removed(home) {
        if same(&path, &r) {
            return Err(format!("PROTECTED: {} is a root folder", path.display()));
        }
    }
    for t in never_enter(home) {
        if inside(&path, &t) {
            return Err(format!(
                "PROTECTED: {} is inside {}",
                path.display(),
                t.display()
            ));
        }
    }
    if guard == Guard::Rule {
        for t in rule_never_enters(home) {
            if inside(&path, &t) {
                return Err(format!(
                    "PROTECTED: {} is inside {}",
                    path.display(),
                    t.display()
                ));
            }
        }
        if let Some(pattern) = super::clean::protection_match(&path) {
            return Err(format!("PROTECTED: {} matches {}", path.display(), pattern));
        }
    }
    if let Some(root) = root {
        let root = normalize(root).ok_or("PROTECTED: root with ..")?;
        if !inside(&path, &root) {
            return Err(format!(
                "PROTECTED: {} is outside {}",
                path.display(),
                root.display()
            ));
        }
    }
    let mut cur = PathBuf::new();
    for c in path.components() {
        cur.push(c.as_os_str());
        if let Ok(m) = std::fs::symlink_metadata(&cur) {
            if super::fsops::is_link(&m) && cur != path && !is_os_alias(&cur) {
                return Err(format!("PROTECTED: {} goes through a link", path.display()));
            }
        }
    }
    Ok(())
}

fn is_os_alias(p: &Path) -> bool {
    cfg!(target_os = "macos") && matches!(p.to_str(), Some("/var" | "/tmp" | "/etc"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn h() -> PathBuf {
        PathBuf::from(if cfg!(windows) {
            "C:/Users/u"
        } else if cfg!(target_os = "macos") {
            "/Users/u"
        } else {
            "/home/u"
        })
    }

    #[test]
    fn roots_and_documents_are_refused() {
        let home = h();
        assert!(check_with(&home, Guard::Picked, None, &home).is_err());
        assert!(check_with(&home.join("Documents/a.txt"), Guard::Rule, None, &home).is_err());
        assert!(check_with(&home.join(".ssh/id_rsa"), Guard::Picked, None, &home).is_err());
        assert!(check_with(&home.join(".cache"), Guard::Rule, None, &home).is_err());
    }

    #[test]
    fn a_picked_file_in_documents_is_fine() {
        let home = h();
        assert!(check_with(&home.join("Documents/big.iso"), Guard::Picked, None, &home).is_ok());
    }

    #[test]
    fn a_rule_stays_inside_its_root() {
        let home = h();
        let root = home.join(".cache/pip");
        assert!(check_with(&root.join("wheels"), Guard::Rule, Some(&root), &home).is_ok());
        assert!(check_with(&home.join(".cache/other"), Guard::Rule, Some(&root), &home).is_err());
        assert!(check_with(Path::new("relative/x"), Guard::Rule, None, &home).is_err());
        assert!(check_with(&root.join("../../x"), Guard::Rule, None, &home).is_err());
    }
}

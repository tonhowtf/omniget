//! One spelling per folder, for the job lock and the undo guard (roadmap
//! 1.5, "Foco de revisão" 4).
//!
//! Two jobs must never write in the same folder at once, and an undo must not
//! restore files while another agent works in an overlapping folder. Both
//! compare folders, and a folder has many spellings: a symlink to it, `..`
//! segments, a trailing slash, the Windows verbatim prefix `\\?\` that
//! `canonicalize` adds, `c:` against `C:`, `Repo` against `repo` on a
//! case-insensitive disk. A key that let two of those differ would let two
//! writers in. Idea from t3code's `isRestoreWorkspaceIsolated` (MIT, realpath
//! of both sides, "one inside the other" counts) and multica's in-place mutex
//! (idea only), written here from scratch.

use std::path::{Component, Path, PathBuf};

/// A folder, canonical: symlinks resolved, no verbatim prefix, `/` as the
/// separator, no trailing slash (except a root), case folded where the
/// platform's default disk ignores case.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct FolderKey(String);

/// Windows and macOS disks ignore case by default (NTFS, APFS). On a
/// case-sensitive volume there, folding can only merge two distinct folders
/// into one lock (one writer too few), never let two writers into one folder.
const FOLD_CASE: bool = cfg!(windows) || cfg!(target_os = "macos");

impl FolderKey {
    /// The key of a folder on this machine. A folder that does not exist
    /// (yet) still gets a stable key: its nearest existing ancestor is
    /// resolved and the rest is appended lexically.
    pub fn of(path: &Path) -> FolderKey {
        let resolved = resolve(path);
        Self::normalize(&resolved.to_string_lossy(), cfg!(windows), FOLD_CASE)
    }

    /// The string half of [`Self::of`], with the platform as parameters so
    /// the Windows rules are tested on every OS.
    pub fn normalize(raw: &str, windows: bool, fold_case: bool) -> FolderKey {
        let mut s = raw.to_string();
        if windows {
            if let Some(rest) = s.strip_prefix(r"\\?\UNC\") {
                s = format!(r"\\{rest}");
            } else if let Some(rest) = s.strip_prefix(r"\\?\") {
                s = rest.to_string();
            }
            s = s.replace('\\', "/");
        }
        let unc = windows && s.starts_with("//");
        let mut out = String::with_capacity(s.len() + 1);
        let mut slash = false;
        for c in s.chars() {
            if c == '/' {
                if slash {
                    continue;
                }
                slash = true;
            } else {
                slash = false;
            }
            out.push(c);
        }
        if unc {
            out.insert(0, '/');
        }
        let drive_root = |o: &str| windows && o.len() == 3 && o.as_bytes()[1] == b':';
        while out.len() > 1 && out.ends_with('/') && !drive_root(&out) {
            out.pop();
        }
        if windows && out.len() == 2 && out.as_bytes()[1] == b':' {
            out.push('/');
        }
        if fold_case {
            out = out.to_lowercase();
        }
        FolderKey(out)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The same folder, or one inside the other: a job in `/repo` and one in
    /// `/repo/sub` write into the same tree. `/repo` and `/repo2` do not.
    pub fn overlaps(&self, other: &FolderKey) -> bool {
        contains(&self.0, &other.0) || contains(&other.0, &self.0)
    }
}

impl std::fmt::Display for FolderKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

fn contains(outer: &str, inner: &str) -> bool {
    inner == outer
        || (inner.starts_with(outer)
            && (outer.ends_with('/') || inner.as_bytes().get(outer.len()) == Some(&b'/')))
}

/// Absolute, `.`/`..` folded, symlinks resolved as far as the path exists.
fn resolve(path: &Path) -> PathBuf {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .map(|d| d.join(path))
            .unwrap_or_else(|_| path.to_path_buf())
    };
    let mut lexical = PathBuf::new();
    for comp in absolute.components() {
        match comp {
            Component::CurDir => {}
            Component::ParentDir => {
                lexical.pop();
            }
            other => lexical.push(other.as_os_str()),
        }
    }
    let mut existing = lexical.clone();
    let mut rest: Vec<std::ffi::OsString> = Vec::new();
    loop {
        if let Ok(real) = existing.canonicalize() {
            let mut out = real;
            for part in rest.iter().rev() {
                out.push(part);
            }
            return out;
        }
        match (
            existing.file_name().map(|n| n.to_os_string()),
            existing.parent(),
        ) {
            (Some(name), Some(parent)) => {
                rest.push(name);
                existing = parent.to_path_buf();
            }
            _ => return lexical,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("omniget-fk-{name}-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[cfg(unix)]
    #[test]
    fn a_symlink_is_the_same_folder() {
        let root = temp("link");
        let real = root.join("shared");
        std::fs::create_dir_all(&real).unwrap();
        let link = root.join("alias");
        std::os::unix::fs::symlink(&real, &link).unwrap();
        assert_eq!(FolderKey::of(&real), FolderKey::of(&link));
        // `..` and a trailing slash are spellings too.
        let dotted = root.join("shared").join("..").join("shared/");
        assert_eq!(FolderKey::of(&real), FolderKey::of(&dotted));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn the_windows_verbatim_prefix_and_drive_case_do_not_matter() {
        let a = FolderKey::normalize(r"\\?\C:\Users\Ana\Repo", true, true);
        let b = FolderKey::normalize(r"c:\users\ana\repo\", true, true);
        let c = FolderKey::normalize("C:/Users/Ana/Repo", true, true);
        assert_eq!(a, b);
        assert_eq!(a, c);
        assert_eq!(a.as_str(), "c:/users/ana/repo");
        // The drive root keeps its slash; it contains every folder of the drive.
        let root = FolderKey::normalize(r"\\?\D:\", true, true);
        assert_eq!(root.as_str(), "d:/");
        assert!(root.overlaps(&FolderKey::normalize(r"D:\work", true, true)));
        assert!(!root.overlaps(&a));
    }

    #[test]
    fn unc_paths_keep_their_server() {
        let a = FolderKey::normalize(r"\\?\UNC\srv\share\proj", true, true);
        let b = FolderKey::normalize(r"\\SRV\Share\Proj", true, true);
        assert_eq!(a, b);
        assert_eq!(a.as_str(), "//srv/share/proj");
        let other = FolderKey::normalize(r"\\other\share\proj", true, true);
        assert!(!a.overlaps(&other));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn a_case_insensitive_mac_folder_matches_in_any_case() {
        let root = temp("case");
        std::fs::create_dir_all(root.join("Shared")).unwrap();
        assert_eq!(
            FolderKey::of(&root.join("Shared")),
            FolderKey::of(&root.join("SHARED"))
        );
        assert_eq!(
            FolderKey::of(&root.join("Shared")),
            FolderKey::of(&root.join("shared"))
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn linux_keeps_the_case() {
        let a = FolderKey::normalize("/home/ana/Repo", false, false);
        let b = FolderKey::normalize("/home/ana/repo", false, false);
        assert_ne!(a, b);
        assert!(!a.overlaps(&b));
        // A double slash is one separator.
        assert_eq!(FolderKey::normalize("/home//ana/Repo/", false, false), a);
    }

    #[test]
    fn containment_counts_both_ways_and_siblings_do_not() {
        let repo = FolderKey::normalize("/w/repo", false, false);
        let sub = FolderKey::normalize("/w/repo/src/lib", false, false);
        let sibling = FolderKey::normalize("/w/repo2", false, false);
        assert!(repo.overlaps(&repo));
        assert!(repo.overlaps(&sub));
        assert!(sub.overlaps(&repo));
        assert!(!repo.overlaps(&sibling));
        assert!(!sibling.overlaps(&sub));
        let slash = FolderKey::normalize("/", false, false);
        assert_eq!(slash.as_str(), "/");
        assert!(slash.overlaps(&sub));
    }

    #[test]
    fn a_missing_folder_is_still_a_stable_key() {
        let root = temp("missing");
        let gone = root.join("not-yet").join("x");
        let a = FolderKey::of(&gone);
        let b = FolderKey::of(&root.join("not-yet/./x/"));
        assert_eq!(a, b);
        // It still overlaps its existing parent.
        assert!(FolderKey::of(&root).overlaps(&a));
        let _ = std::fs::remove_dir_all(root);
    }
}

//! Read grants for trusted, fixed host engines. No directory-wide Homebrew grant.
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    path::{Path, PathBuf},
    time::Duration,
};
use tokio::io::AsyncReadExt;
fn error(code: &str) -> String {
    code.to_owned()
}
fn system(path: &Path) -> bool {
    path.starts_with("/System")
        || path.starts_with("/usr/lib")
        || path.starts_with("/Library/Apple/System/Library")
}
fn macho(path: &Path) -> Result<bool, String> {
    use std::io::Read;
    let mut file = std::fs::File::open(path).map_err(|_| error("ENGINE_FILE_UNAVAILABLE"))?;
    if !file
        .metadata()
        .map_err(|_| error("ENGINE_FILE_UNAVAILABLE"))?
        .is_file()
    {
        return Err(error("ENGINE_FILE_INVALID"));
    }
    let mut magic = [0; 4];
    if file.read_exact(&mut magic).is_err() {
        return Ok(false);
    }
    Ok(matches!(
        magic,
        [0xfe, 0xed, 0xfa, 0xce]
            | [0xce, 0xfa, 0xed, 0xfe]
            | [0xfe, 0xed, 0xfa, 0xcf]
            | [0xcf, 0xfa, 0xed, 0xfe]
            | [0xca, 0xfe, 0xba, 0xbe]
            | [0xbe, 0xba, 0xfe, 0xca]
            | [0xca, 0xfe, 0xba, 0xbf]
            | [0xbf, 0xba, 0xfe, 0xca]
    ))
}
async fn metadata(path: &Path) -> Result<String, String> {
    let mut child = tokio::process::Command::new("/usr/bin/otool")
        .arg("-L")
        .arg("-l")
        .arg(path)
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .map_err(|_| error("ENGINE_METADATA_UNAVAILABLE"))?;
    let mut output = Vec::new();
    let result = tokio::time::timeout(Duration::from_secs(5), async {
        child
            .stdout
            .take()
            .ok_or_else(|| error("ENGINE_METADATA_UNAVAILABLE"))?
            .take(262145)
            .read_to_end(&mut output)
            .await
            .map_err(|_| error("ENGINE_METADATA_FAILED"))?;
        if output.len() > 262144 {
            return Err(error("ENGINE_METADATA_LIMIT"));
        }
        let status = child
            .wait()
            .await
            .map_err(|_| error("ENGINE_METADATA_FAILED"))?;
        if !status.success() {
            return Err(error("ENGINE_METADATA_FAILED"));
        }
        String::from_utf8(output).map_err(|_| error("ENGINE_METADATA_INVALID"))
    })
    .await
    .map_err(|_| error("ENGINE_METADATA_TIMEOUT"))
    .and_then(|r| r);
    if result.is_err() {
        let _ = child.kill().await;
        let _ = child.wait().await;
    }
    result
}
fn expand(raw: &str, owner: &Path, executable: &Path) -> Option<PathBuf> {
    if let Some(tail) = raw.strip_prefix("@loader_path/") {
        Some(owner.parent()?.join(tail))
    } else if let Some(tail) = raw.strip_prefix("@executable_path/") {
        Some(executable.parent()?.join(tail))
    } else if Path::new(raw).is_absolute() {
        Some(raw.into())
    } else {
        None
    }
}
fn rpaths(text: &str, owner: &Path, executable: &Path) -> Vec<PathBuf> {
    let mut active = false;
    let mut result = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line == "cmd LC_RPATH" {
            active = true;
            continue;
        }
        if active {
            if let Some(path) = line
                .strip_prefix("path ")
                .and_then(|s| s.split_once(" (offset ").map(|(p, _)| p))
            {
                if let Some(path) = expand(path, owner, executable) {
                    result.push(path)
                }
                active = false;
            } else if line.starts_with("cmd ") {
                active = false
            }
        }
    }
    result
}
/// Identity of a granted file: replacing or rewriting it changes the inode,
/// size or change time (ctime cannot be set from user space).
#[cfg(unix)]
fn fingerprint(path: &Path) -> Option<(u64, u64, u64, i64, i64, i64, i64)> {
    use std::os::unix::fs::MetadataExt;
    let m = std::fs::metadata(path).ok()?;
    Some((
        m.dev(),
        m.ino(),
        m.size(),
        m.mtime(),
        m.mtime_nsec(),
        m.ctime(),
        m.ctime_nsec(),
    ))
}
#[cfg(not(unix))]
fn fingerprint(path: &Path) -> Option<(u64, u64, u64, i64, i64, i64, i64)> {
    let m = std::fs::metadata(path).ok()?;
    Some((0, 0, m.len(), 0, 0, 0, 0))
}
type Fingerprints = Vec<(PathBuf, (u64, u64, u64, i64, i64, i64, i64))>;
/// Last results per binary set. Every worker run used to spawn `otool` for
/// each binary and each non-system dylib again (Homebrew ffmpeg: dozens).
/// A hit is served only when every file of the result, and every requested
/// binary, still has the same identity; otherwise it is recomputed.
static CACHE: std::sync::Mutex<Option<BTreeMap<Vec<PathBuf>, (Vec<PathBuf>, Fingerprints)>>> =
    std::sync::Mutex::new(None);
fn fingerprints(paths: impl IntoIterator<Item = PathBuf>) -> Option<Fingerprints> {
    paths
        .into_iter()
        .map(|p| fingerprint(&p).map(|f| (p, f)))
        .collect()
}
fn cached(key: &[PathBuf]) -> Option<Vec<PathBuf>> {
    let guard = CACHE.lock().ok()?;
    let (files, prints) = guard.as_ref()?.get(key)?;
    let current = fingerprints(prints.iter().map(|(p, _)| p.clone()))?;
    (current == *prints).then(|| files.clone())
}
/// Paths must come from trusted tool discovery, never model arguments.
pub async fn dependency_files(binaries: &[PathBuf]) -> Result<Vec<PathBuf>, String> {
    let key = binaries.to_vec();
    if let Some(hit) = cached(&key) {
        return Ok(hit);
    }
    let files = dependency_files_uncached(binaries).await?;
    let watched = binaries.iter().cloned().chain(files.iter().cloned());
    if let Some(prints) = fingerprints(watched) {
        if let Ok(mut guard) = CACHE.lock() {
            let map = guard.get_or_insert_with(BTreeMap::new);
            if map.len() >= 16 {
                map.clear();
            }
            map.insert(key, (files.clone(), prints));
        }
    }
    Ok(files)
}
async fn dependency_files_uncached(binaries: &[PathBuf]) -> Result<Vec<PathBuf>, String> {
    #[cfg(not(target_os = "macos"))]
    {
        let _ = binaries;
        return Err(error("ENGINE_SANDBOX_UNAVAILABLE"));
    }
    #[cfg(target_os = "macos")]
    tokio::time::timeout(Duration::from_secs(45), async {
        if binaries.len() > 8 {
            return Err(error("ENGINE_DEPENDENCY_LIMIT"));
        }
        let mut all = BTreeSet::new();
        let mut cache = BTreeMap::<PathBuf, String>::new();
        for binary in binaries {
            let binary = binary
                .canonicalize()
                .map_err(|_| error("ENGINE_FILE_UNAVAILABLE"))?;
            let mut queue = VecDeque::from([(binary.clone(), Vec::<PathBuf>::new())]);
            let mut visited = BTreeSet::new();
            while let Some((path, inherited)) = queue.pop_front() {
                if system(&path) {
                    continue;
                }
                all.insert(path.clone());
                // dyld resolves directory aliases before the final dylib symlink.
                // Preserve each exact intermediate spelling for metadata grants.
                for ancestor in path.ancestors().skip(1) {
                    if let (Ok(base), Ok(tail)) =
                        (ancestor.canonicalize(), path.strip_prefix(ancestor))
                    {
                        let intermediate = base.join(tail);
                        if intermediate.is_file() {
                            all.insert(intermediate);
                        }
                    }
                }
                if all.len() > 256 {
                    return Err(error("ENGINE_DEPENDENCY_LIMIT"));
                }
                let path = path
                    .canonicalize()
                    .map_err(|_| error("ENGINE_FILE_UNAVAILABLE"))?;
                if !visited.insert(path.clone()) {
                    continue;
                }
                all.insert(path.clone());
                if all.len() > 256 {
                    return Err(error("ENGINE_DEPENDENCY_LIMIT"));
                }
                if !macho(&path)? {
                    continue;
                }
                let list = if let Some(text) = cache.get(&path) {
                    text.clone()
                } else {
                    let text = metadata(&path).await?;
                    cache.insert(path.clone(), text.clone());
                    text
                };
                let mut search = rpaths(&list, &path, &binary);
                search.extend(inherited);
                for line in list.lines() {
                    let Some((raw, _)) = line.trim().split_once(" (compatibility version ") else {
                        continue;
                    };
                    let candidate = if let Some(tail) = raw.strip_prefix("@rpath/") {
                        search
                            .iter()
                            .map(|p| p.join(tail))
                            .find(|p| p.is_file())
                            .ok_or_else(|| error("ENGINE_RPATH_UNRESOLVED"))?
                    } else {
                        expand(raw, &path, &binary)
                            .ok_or_else(|| error("ENGINE_DEPENDENCY_INVALID"))?
                    };
                    if !system(&candidate) {
                        queue.push_back((candidate, search.clone()))
                    }
                }
            }
        }
        Ok(all.into_iter().collect())
    })
    .await
    .map_err(|_| error("ENGINE_DEPENDENCY_TIMEOUT"))?
}
/// Pipe-only validation process: no network acquisition, no personal directory grants.
pub fn isolated_command(
    binary: &Path,
    read_files: &[PathBuf],
) -> Result<tokio::process::Command, String> {
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (binary, read_files);
        Err(error("ENGINE_SANDBOX_UNAVAILABLE"))
    }
    #[cfg(target_os = "macos")]
    {
        let binary = binary
            .canonicalize()
            .map_err(|_| error("ENGINE_FILE_UNAVAILABLE"))?;
        let mut profile=String::from("(version 1)(allow default)(deny network*)(deny file-read*)(deny file-write*)(deny mach-lookup)(deny mach-register)(deny appleevent-send)(allow file-read* (literal \"/\"))(allow file-read* (literal \"/private/var/select/sh\"))");
        for root in [
            "/System",
            "/usr",
            "/bin",
            "/sbin",
            "/Library",
            "/private/var/db/dyld",
        ] {
            if Path::new(root).exists() {
                profile.push_str(&format!(
                    "(allow file-read* (subpath {}))",
                    serde_json::to_string(root).unwrap()
                ));
            }
        }
        for file in std::iter::once(&binary).chain(read_files) {
            for ancestor in file.ancestors() {
                let raw = ancestor
                    .to_str()
                    .ok_or_else(|| error("ENGINE_FILE_INVALID"))?;
                profile.push_str(&format!(
                    "(allow file-read-metadata (literal {}))",
                    serde_json::to_string(raw).map_err(|_| error("ENGINE_FILE_INVALID"))?
                ));
            }
            let file = file
                .canonicalize()
                .map_err(|_| error("ENGINE_FILE_UNAVAILABLE"))?;
            if !file.is_file() {
                return Err(error("ENGINE_GRANT_MUST_BE_FILE"));
            }
            let file = file.to_str().ok_or_else(|| error("ENGINE_FILE_INVALID"))?;
            profile.push_str(&format!(
                "(allow file-read* (literal {}))",
                serde_json::to_string(file).map_err(|_| error("ENGINE_FILE_INVALID"))?
            ));
        }
        for device in ["/dev/null", "/dev/random", "/dev/urandom"] {
            profile.push_str(&format!(
                "(allow file-read* (literal {}))",
                serde_json::to_string(device).unwrap()
            ));
        }
        profile.push_str("(allow file-write* (literal \"/dev/null\"))");
        let mut command = tokio::process::Command::new("/usr/bin/sandbox-exec");
        command
            .arg("-p")
            .arg(profile)
            .arg(binary)
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .current_dir("/")
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true)
            .process_group(0);
        let max = unsafe { libc::getdtablesize() };
        if max < 3 {
            return Err(error("ENGINE_DESCRIPTOR_LIMIT"));
        }
        unsafe {
            command.pre_exec(move || {
                for fd in 3..max {
                    let flags = libc::fcntl(fd, libc::F_GETFD);
                    if flags >= 0 && libc::fcntl(fd, libc::F_SETFD, flags | libc::FD_CLOEXEC) < 0 {
                        return Err(std::io::Error::last_os_error());
                    }
                }
                Ok(())
            });
        }
        Ok(command)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    /// Opt-in: `OMNIGET_TEST_ENGINE_BINARIES=/abs/a:/abs/b` (real engines).
    #[cfg(target_os = "macos")]
    #[tokio::test]
    async fn dependency_files_second_run_is_served_from_cache() {
        let Some(list) = std::env::var_os("OMNIGET_TEST_ENGINE_BINARIES") else {
            eprintln!("skipped: OMNIGET_TEST_ENGINE_BINARIES unset");
            return;
        };
        let binaries: Vec<PathBuf> = std::env::split_paths(&list).collect();
        let t = std::time::Instant::now();
        let cold = dependency_files_uncached(&binaries).await.unwrap();
        let uncached = t.elapsed();
        let t = std::time::Instant::now();
        let first = dependency_files(&binaries).await.unwrap();
        let miss = t.elapsed();
        let t = std::time::Instant::now();
        let second = dependency_files(&binaries).await.unwrap();
        let hit = t.elapsed();
        eprintln!(
            "dependency_files: {} files, uncached {uncached:?}, miss {miss:?}, hit {hit:?}",
            cold.len()
        );
        assert_eq!(cold, first);
        assert_eq!(first, second);
        assert!(hit < uncached);
    }
    #[cfg(target_os = "macos")]
    #[tokio::test]
    async fn cache_is_invalidated_when_a_granted_file_changes() {
        let dir = std::env::temp_dir().join(format!("omniget-engine-cache-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let script = dir.join("engine.sh");
        std::fs::write(&script, b"#!/bin/sh\n").unwrap();
        let key = vec![script.clone()];
        let first = dependency_files(&key).await.unwrap();
        assert_eq!(cached(&key), Some(first.clone()));
        std::fs::write(&script, b"#!/bin/sh\necho changed\n").unwrap();
        assert_eq!(
            cached(&key),
            None,
            "stale grant served after the engine changed"
        );
        let replaced = dir.join("engine.new");
        std::fs::write(&replaced, b"#!/bin/sh\necho changed\n").unwrap();
        let _ = dependency_files(&key).await.unwrap();
        std::fs::rename(&replaced, &script).unwrap();
        assert_eq!(
            cached(&key),
            None,
            "stale grant served after the engine was replaced"
        );
        std::fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn loader_paths_are_specific() {
        let owner = Path::new("/opt/tools/lib/a.dylib");
        let exe = Path::new("/opt/tools/bin/tool");
        assert_eq!(
            expand("@loader_path/b.dylib", owner, exe).unwrap(),
            Path::new("/opt/tools/lib/b.dylib")
        );
        assert!(expand("relative.dylib", owner, exe).is_none());
        let parsed = rpaths(
            "cmd LC_RPATH\ncmdsize 40\npath @loader_path/deps (offset 12)\n",
            owner,
            exe,
        );
        assert_eq!(parsed, vec![PathBuf::from("/opt/tools/lib/deps")]);
    }
}

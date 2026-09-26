//! Descriptor-relative file access for locally granted roots.
//! No ambient path is accepted after a root has been opened. Symlinks and
//! multiply-linked files are deliberately unsupported at this boundary.
use sha2::{Digest, Sha256};
use std::fs::File;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Component, Path};

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct Identity {
    pub device: u64,
    pub inode: u64,
}

pub struct Root {
    directory: File,
    identity: Identity,
}
pub struct Snapshot {
    pub file: File,
    pub bytes: u64,
    pub digest: String,
    pub identity: Identity,
}

fn denied() -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::PermissionDenied, "FILE_ACCESS_DENIED")
}

#[cfg(unix)]
mod os {
    use super::*;
    use std::ffi::CString;
    use std::os::fd::{AsRawFd, FromRawFd};
    use std::os::unix::ffi::OsStrExt;
    use std::os::unix::fs::MetadataExt;
    /// New folders one create may add on the way to its file.
    const MAX_NEW_DIRS: usize = 8;

    pub fn identity(file: &File) -> std::io::Result<Identity> {
        let m = file.metadata()?;
        Ok(Identity {
            device: m.dev(),
            inode: m.ino(),
        })
    }
    pub fn name(value: &std::ffi::OsStr) -> std::io::Result<CString> {
        CString::new(value.as_bytes()).map_err(|_| denied())
    }
    pub fn at(dir: &File, n: &std::ffi::OsStr, flags: i32) -> std::io::Result<File> {
        let n = name(n)?;
        // O_NOFOLLOW applies to the actual object opened, not an earlier stat.
        let fd = unsafe {
            libc::openat(
                dir.as_raw_fd(),
                n.as_ptr(),
                flags | libc::O_CLOEXEC | libc::O_NOFOLLOW,
                0o600,
            )
        };
        if fd < 0 {
            return Err(std::io::Error::last_os_error());
        }
        Ok(unsafe { File::from_raw_fd(fd) })
    }
    pub fn root(path: &Path) -> std::io::Result<File> {
        if !path.is_absolute() {
            return Err(denied());
        }
        let mut dir = File::open("/")?;
        for part in path.components() {
            match part {
                Component::RootDir => {}
                Component::Normal(n) => dir = at(&dir, n, libc::O_RDONLY | libc::O_DIRECTORY)?,
                _ => return Err(denied()),
            }
        }
        Ok(dir)
    }
    pub fn parent(root: &File, path: &Path) -> std::io::Result<(File, std::ffi::OsString)> {
        // Reject empty/dot components too: Path::components normalizes these.
        let raw = path.as_os_str().as_bytes();
        if raw.is_empty()
            || raw
                .split(|b| *b == b'/')
                .any(|s| s.is_empty() || s == b"." || s == b"..")
        {
            return Err(denied());
        }
        let mut parts = path.components().peekable();
        let mut dir = root.try_clone()?;
        while let Some(part) = parts.next() {
            let Component::Normal(n) = part else {
                return Err(denied());
            };
            if parts.peek().is_none() {
                return Ok((dir, n.to_os_string()));
            }
            dir = at(&dir, n, libc::O_RDONLY | libc::O_DIRECTORY)?;
        }
        Err(denied())
    }
    pub fn read(root: &File, path: &Path) -> std::io::Result<File> {
        let (dir, n) = parent(root, path)?;
        // NONBLOCK prevents a malicious FIFO from hanging before fstat.
        let file = at(&dir, &n, libc::O_RDONLY | libc::O_NONBLOCK)?;
        let m = file.metadata()?;
        if !m.is_file() || m.nlink() != 1 {
            return Err(denied());
        }
        Ok(file)
    }
    /// Like [`parent`], but a missing intermediate folder is created with
    /// mkdirat on the pinned parent descriptor, then reopened with O_NOFOLLOW:
    /// a symlink or file planted at that name is never adopted.
    pub fn parent_creating(
        root: &File,
        path: &Path,
    ) -> std::io::Result<(File, std::ffi::OsString)> {
        let raw = path.as_os_str().as_bytes();
        if raw.is_empty()
            || raw
                .split(|b| *b == b'/')
                .any(|s| s.is_empty() || s == b"." || s == b"..")
        {
            return Err(denied());
        }
        let mut parts = path.components().peekable();
        let mut dir = root.try_clone()?;
        let mut created = 0;
        while let Some(part) = parts.next() {
            let Component::Normal(n) = part else {
                return Err(denied());
            };
            if parts.peek().is_none() {
                return Ok((dir, n.to_os_string()));
            }
            dir = match at(&dir, n, libc::O_RDONLY | libc::O_DIRECTORY) {
                Err(e) if e.kind() == std::io::ErrorKind::NotFound && created < MAX_NEW_DIRS => {
                    let c = name(n)?;
                    if unsafe { libc::mkdirat(dir.as_raw_fd(), c.as_ptr(), 0o755) } != 0 {
                        return Err(std::io::Error::last_os_error());
                    }
                    created += 1;
                    dir.sync_all()?;
                    at(&dir, n, libc::O_RDONLY | libc::O_DIRECTORY)?
                }
                other => other?,
            };
        }
        Err(denied())
    }
    pub fn create(root: &File, staging: &File, path: &Path, bytes: &[u8]) -> std::io::Result<()> {
        let (dir, n) = parent_creating(root, path)?;
        let tmp = format!(".omniget-{}.pending", uuid::Uuid::new_v4());
        let mut file = at(
            staging,
            std::ffi::OsStr::new(&tmp),
            libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL,
        )?;
        let tmp = name(std::ffi::OsStr::new(&tmp))?;
        let result = (|| {
            file.write_all(bytes)?;
            file.sync_all()?;
            let n = name(&n)?;
            // linkat is atomic and refuses ANY existing destination, including
            // a symlink swapped in after validation. Never truncate a target.
            if unsafe {
                libc::linkat(
                    staging.as_raw_fd(),
                    tmp.as_ptr(),
                    dir.as_raw_fd(),
                    n.as_ptr(),
                    0,
                )
            } != 0
            {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        })();
        if unsafe { libc::unlinkat(staging.as_raw_fd(), tmp.as_ptr(), 0) } != 0 {
            return Err(std::io::Error::last_os_error());
        }
        staging.sync_all()?;
        dir.sync_all()?;
        result
    }
}

impl Root {
    pub fn open(path: &Path, expected: Option<&Identity>) -> std::io::Result<Self> {
        #[cfg(unix)]
        {
            let directory = os::root(path)?;
            let identity = os::identity(&directory)?;
            if expected.is_some_and(|e| e != &identity) {
                return Err(denied());
            }
            Ok(Self {
                directory,
                identity,
            })
        }
        #[cfg(not(unix))]
        {
            let _ = (path, expected);
            Err(denied())
        }
    }
    /// Create one new child through the pinned directory, without following
    /// a replaced parent pathname or adopting an existing/symlink directory.
    pub fn create_directory(&self, relative: &Path) -> std::io::Result<()> {
        #[cfg(unix)]
        {
            use std::os::{fd::AsRawFd, unix::ffi::OsStrExt};
            let mut parts = relative.components();
            let Some(std::path::Component::Normal(name)) = parts.next() else {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    "single directory name required",
                ));
            };
            if parts.next().is_some() {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    "single directory name required",
                ));
            }
            let name = std::ffi::CString::new(name.as_bytes())
                .map_err(|_| std::io::Error::from(std::io::ErrorKind::InvalidInput))?;
            if unsafe { libc::mkdirat(self.directory.as_raw_fd(), name.as_ptr(), 0o700) } != 0 {
                return Err(std::io::Error::last_os_error());
            }
            self.directory.sync_all()?;
            Ok(())
        }
        #[cfg(not(unix))]
        {
            let _ = relative;
            Err(std::io::Error::new(
                std::io::ErrorKind::Unsupported,
                "descriptor directories unavailable",
            ))
        }
    }
    pub fn identity(&self) -> &Identity {
        &self.identity
    }
    pub fn snapshot(&self, relative: &Path, max_bytes: u64) -> std::io::Result<Snapshot> {
        #[cfg(unix)]
        {
            let mut file = os::read(&self.directory, relative)?;
            let before = file.metadata()?;
            if before.len() > max_bytes {
                return Err(denied());
            }
            let mut hash = Sha256::new();
            // Anonymous copy: later changes to the source inode cannot alter
            // bytes delivered under the recorded digest. Never expose this fd
            // or its private staging directory to an untrusted worker.
            let mut frozen = tempfile::tempfile()?;
            let mut total = 0u64;
            let mut buf = [0u8; 65536];
            loop {
                let n = file.read(&mut buf)?;
                if n == 0 {
                    break;
                }
                total = total.checked_add(n as u64).ok_or_else(denied)?;
                if total > max_bytes {
                    return Err(denied());
                }
                hash.update(&buf[..n]);
                frozen.write_all(&buf[..n])?;
            }
            let after = file.metadata()?;
            use std::os::unix::fs::MetadataExt;
            if before.len() != total
                || after.len() != total
                || before.mtime_nsec() != after.mtime_nsec()
                || before.mtime() != after.mtime()
                || before.ctime() != after.ctime()
                || before.ctime_nsec() != after.ctime_nsec()
                || after.nlink() != 1
            {
                return Err(denied());
            }
            frozen.seek(SeekFrom::Start(0))?;
            let identity = os::identity(&file)?;
            Ok(Snapshot {
                file: frozen,
                bytes: total,
                digest: format!("{:x}", hash.finalize()),
                identity,
            })
        }
        #[cfg(not(unix))]
        {
            let _ = (relative, max_bytes);
            Err(denied())
        }
    }
    /// New files only. `staging` MUST be outside all worker grants and denied
    /// by the worker sandbox, on the same filesystem. Mode 0700 alone does not
    /// protect a staging pathname from another process running as this user.
    /// The trusted coordinator supplies this capability, never an MCP caller.
    pub fn create_new(&self, staging: &Root, relative: &Path, bytes: &[u8]) -> std::io::Result<()> {
        if self.identity == staging.identity || self.identity.device != staging.identity.device {
            return Err(denied());
        }
        #[cfg(unix)]
        {
            os::create(&self.directory, &staging.directory, relative, bytes)
        }
        #[cfg(not(unix))]
        {
            let _ = (staging, relative, bytes);
            Err(denied())
        }
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::symlink;
    fn fixture() -> std::path::PathBuf {
        // canonicalize temp because macOS /tmp itself is a symlink.
        let p = std::env::temp_dir()
            .canonicalize()
            .unwrap()
            .join(format!("omniget-files-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&p).unwrap();
        p
    }
    #[test]
    fn object_bound_access_rejects_escape_links_and_overwrite() {
        let p = fixture();
        let root = Root::open(&p, None).unwrap();
        let staging_path = fixture();
        let staging = Root::open(&staging_path, None).unwrap();
        root.create_new(&staging, Path::new("valid"), b"verified bytes")
            .unwrap();
        assert!(root
            .create_new(&staging, Path::new("valid"), b"corrupt")
            .is_err());
        for path in ["../escape", "/etc/passwd", "./valid", "valid/../valid", ""] {
            assert!(root.snapshot(Path::new(path), 100).is_err());
        }
        symlink("valid", p.join("link")).unwrap();
        assert!(root.snapshot(Path::new("link"), 100).is_err());
        std::fs::hard_link(p.join("valid"), p.join("hard")).unwrap();
        assert!(root.snapshot(Path::new("valid"), 100).is_err());
        std::fs::remove_file(p.join("hard")).unwrap();
        let mut snap = root.snapshot(Path::new("valid"), 100).unwrap();
        std::fs::write(p.join("valid"), b"changed source").unwrap();
        std::fs::rename(p.join("valid"), p.join("old")).unwrap();
        symlink("/etc/passwd", p.join("valid")).unwrap();
        let mut data = String::new();
        snap.file.read_to_string(&mut data).unwrap();
        assert_eq!(data, "verified bytes");
        assert!(root.snapshot(Path::new("valid"), 100).is_err());
        std::fs::remove_dir_all(p).unwrap();
        std::fs::remove_dir_all(staging_path).unwrap();
    }
    #[test]
    fn create_adds_missing_folders_but_never_adopts_a_planted_link() {
        let p = fixture();
        let root = Root::open(&p, None).unwrap();
        let staging_path = fixture();
        let staging = Root::open(&staging_path, None).unwrap();
        root.create_new(&staging, Path::new("piloto/deep/resultado.md"), b"ok")
            .unwrap();
        assert_eq!(
            std::fs::read(p.join("piloto/deep/resultado.md")).unwrap(),
            b"ok"
        );
        let outside = fixture();
        symlink(&outside, p.join("planted")).unwrap();
        assert!(root
            .create_new(&staging, Path::new("planted/new/file"), b"no")
            .is_err());
        assert!(std::fs::read_dir(&outside).unwrap().next().is_none());
        let nine = (0..9)
            .map(|i| format!("d{i}"))
            .collect::<Vec<_>>()
            .join("/")
            + "/f";
        assert!(root.create_new(&staging, Path::new(&nine), b"no").is_err());
        for d in [p, staging_path, outside] {
            std::fs::remove_dir_all(d).unwrap();
        }
    }
    #[test]
    fn changed_root_identity_and_parent_symlink_are_denied() {
        let p = fixture();
        let root = Root::open(&p, None).unwrap();
        let staging_path = fixture();
        let staging = Root::open(&staging_path, None).unwrap();
        std::fs::create_dir(p.join("folder")).unwrap();
        symlink("folder", p.join("alias")).unwrap();
        assert!(root
            .create_new(&staging, Path::new("alias/file"), b"no")
            .is_err());
        let wrong = Identity {
            device: root.identity().device,
            inode: root.identity().inode + 1,
        };
        assert!(Root::open(&p, Some(&wrong)).is_err());
        assert!(root
            .create_new(&staging, Path::new("folder/file"), b"ok")
            .is_ok());
        assert!(root.snapshot(Path::new("folder/file"), 1).is_err());
        std::fs::remove_dir_all(p).unwrap();
        std::fs::remove_dir_all(staging_path).unwrap();
    }
}

// Descriptor-relative enumeration shared by bounded mission manifests.
#[cfg(unix)]
mod manifest_files {
    use super::*;
    use std::ffi::{CStr, CString, OsString};
    use std::os::fd::{AsRawFd, FromRawFd};
    use std::os::unix::ffi::{OsStrExt, OsStringExt};
    use std::os::unix::fs::MetadataExt;
    use std::path::PathBuf;
    fn open_dir_at(parent: &File, name: &std::ffi::OsStr) -> std::io::Result<File> {
        let name = CString::new(name.as_bytes()).map_err(|_| denied())?;
        let fd = unsafe {
            libc::openat(
                parent.as_raw_fd(),
                name.as_ptr(),
                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            )
        };
        if fd < 0 {
            return Err(std::io::Error::last_os_error());
        }
        Ok(unsafe { File::from_raw_fd(fd) })
    }
    struct Stream(*mut libc::DIR);
    impl Drop for Stream {
        fn drop(&mut self) {
            unsafe {
                libc::closedir(self.0);
            }
        }
    }
    fn stamp(m: &std::fs::Metadata) -> (u64, u64, i64, i64, i64, i64) {
        (
            m.dev(),
            m.ino(),
            m.mtime(),
            m.mtime_nsec(),
            m.ctime(),
            m.ctime_nsec(),
        )
    }
    fn walk(
        dir: &File,
        prefix: &Path,
        limit: usize,
        seen: &mut usize,
        out: &mut Vec<(PathBuf, bool)>,
    ) -> std::io::Result<()> {
        if prefix.components().count() > 64 {
            return Err(denied());
        }
        let before = dir.metadata()?;
        // A new open description keeps directory offsets independent of both
        // the root descriptor and subsequent manifest passes.
        let scan = open_dir_at(dir, std::ffi::OsStr::new("."))?;
        use std::os::fd::IntoRawFd;
        let fd = scan.into_raw_fd();
        let stream = unsafe { libc::fdopendir(fd) };
        if stream.is_null() {
            unsafe {
                libc::close(fd);
            }
            return Err(std::io::Error::last_os_error());
        }
        let stream = Stream(stream);
        loop {
            // POSIX readdir returns NULL for both EOF and error.
            #[cfg(target_os = "macos")]
            unsafe {
                *libc::__error() = 0;
            }
            #[cfg(any(target_os = "linux", target_os = "android"))]
            unsafe {
                *libc::__errno_location() = 0;
            }
            let entry = unsafe { libc::readdir(stream.0) };
            if entry.is_null() {
                let error = std::io::Error::last_os_error();
                if error.raw_os_error().unwrap_or(0) != 0 {
                    return Err(error);
                }
                break;
            }
            let name = unsafe { CStr::from_ptr((*entry).d_name.as_ptr()) }.to_bytes();
            if name == b"." || name == b".." {
                continue;
            }
            *seen = seen.checked_add(1).ok_or_else(denied)?;
            if *seen > limit {
                return Err(denied());
            }
            let name = OsString::from_vec(name.to_vec());
            let c_name = CString::new(name.as_bytes()).map_err(|_| denied())?;
            let fd = unsafe {
                libc::openat(
                    dir.as_raw_fd(),
                    c_name.as_ptr(),
                    libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC,
                )
            };
            if fd < 0 {
                return Err(std::io::Error::last_os_error());
            }
            let file = unsafe { File::from_raw_fd(fd) };
            let md = file.metadata()?;
            let relative = prefix.join(&name);
            if md.is_dir() {
                out.push((relative.clone(), true));
                walk(&file, &relative, limit, seen, out)?;
            } else if md.is_file() && md.nlink() == 1 {
                out.push((relative, false));
            } else {
                return Err(denied());
            }
        }
        if stamp(&before) != stamp(&dir.metadata()?) {
            return Err(denied());
        }
        Ok(())
    }
    pub fn entries(
        root: &File,
        relative: &Path,
        limit: usize,
    ) -> std::io::Result<Vec<(PathBuf, bool)>> {
        let directory = if relative == Path::new(".") {
            open_dir_at(root, std::ffi::OsStr::new("."))?
        } else {
            let (parent, name) = os::parent(root, relative)?;
            open_dir_at(&parent, &name)?
        };
        let prefix = if relative == Path::new(".") {
            Path::new("")
        } else {
            relative
        };
        let mut out = Vec::new();
        walk(&directory, prefix, limit, &mut 0, &mut out)?;
        out.sort();
        Ok(out)
    }

    fn tree_error(code: &str, detail: String) -> std::io::Error {
        std::io::Error::new(std::io::ErrorKind::Other, format!("{code}: {detail}"))
    }
    #[allow(clippy::too_many_arguments)]
    fn scan_walk(
        dir: &File,
        prefix: &Path,
        limit: usize,
        filter: &dyn Fn(&Path) -> super::ScanFilter,
        seen: &mut usize,
        out: &mut super::TreeScan,
    ) -> std::io::Result<()> {
        if prefix.components().count() > 64 {
            out.skipped.push((prefix.to_path_buf(), "too_deep"));
            return Ok(());
        }
        let before = dir.metadata()?;
        let scan = open_dir_at(dir, std::ffi::OsStr::new("."))?;
        use std::os::fd::IntoRawFd;
        let fd = scan.into_raw_fd();
        let stream = unsafe { libc::fdopendir(fd) };
        if stream.is_null() {
            unsafe {
                libc::close(fd);
            }
            return Err(std::io::Error::last_os_error());
        }
        let stream = Stream(stream);
        loop {
            #[cfg(target_os = "macos")]
            unsafe {
                *libc::__error() = 0;
            }
            #[cfg(any(target_os = "linux", target_os = "android"))]
            unsafe {
                *libc::__errno_location() = 0;
            }
            let entry = unsafe { libc::readdir(stream.0) };
            if entry.is_null() {
                let error = std::io::Error::last_os_error();
                if error.raw_os_error().unwrap_or(0) != 0 {
                    return Err(tree_error(
                        "FILE_TREE_READ_FAILED",
                        format!("{}: {error}", prefix.display()),
                    ));
                }
                break;
            }
            let name = unsafe { CStr::from_ptr((*entry).d_name.as_ptr()) }.to_bytes();
            if name == b"." || name == b".." {
                continue;
            }
            let name = OsString::from_vec(name.to_vec());
            let relative = prefix.join(&name);
            // Hidden/ignored subtrees are decided by name BEFORE any open and
            // never count toward the limit.
            let decision = filter(&relative);
            if decision == super::ScanFilter::Hide {
                continue;
            }
            *seen = seen.saturating_add(1);
            if *seen > limit {
                return Err(tree_error(
                    "FILE_TREE_LIMIT",
                    format!(
                        "more than {limit} entries; stopped at `{}`. Pass a narrower `path`",
                        relative.display()
                    ),
                ));
            }
            let c_name = CString::new(name.as_bytes()).map_err(|_| denied())?;
            let fd = unsafe {
                libc::openat(
                    dir.as_raw_fd(),
                    c_name.as_ptr(),
                    libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC,
                )
            };
            if fd < 0 {
                let e = std::io::Error::last_os_error();
                let why = match e.raw_os_error() {
                    Some(libc::ELOOP) | Some(libc::EMLINK) => "symlink",
                    Some(libc::ENOENT) => "vanished",
                    Some(libc::ENXIO) | Some(libc::EOPNOTSUPP) => "special",
                    _ => "unreadable",
                };
                out.skipped.push((relative, why));
                continue;
            }
            let file = unsafe { File::from_raw_fd(fd) };
            let Ok(md) = file.metadata() else {
                out.skipped.push((relative, "unreadable"));
                continue;
            };
            if md.is_dir() {
                out.entries.push((relative.clone(), true));
                if decision == super::ScanFilter::ListOnly {
                    out.skipped.push((relative, "ignored_dir"));
                    continue;
                }
                scan_walk(&file, &relative, limit, filter, seen, out)?;
            } else if md.is_file() && md.nlink() == 1 {
                out.entries.push((relative, false));
            } else if md.is_file() {
                out.skipped.push((relative, "hardlink"));
            } else {
                out.skipped.push((relative, "special"));
            }
        }
        if stamp(&before) != stamp(&dir.metadata()?) {
            // Discovery only: report the folder instead of failing the tree.
            out.skipped.push((
                if prefix.as_os_str().is_empty() {
                    PathBuf::from(".")
                } else {
                    prefix.to_path_buf()
                },
                "changed_while_listing",
            ));
        }
        Ok(())
    }
    pub fn scan(
        root: &File,
        relative: &Path,
        limit: usize,
        filter: &dyn Fn(&Path) -> super::ScanFilter,
    ) -> std::io::Result<super::TreeScan> {
        let directory = if relative == Path::new(".") {
            open_dir_at(root, std::ffi::OsStr::new("."))?
        } else {
            let (parent, name) = os::parent(root, relative)?;
            open_dir_at(&parent, &name)?
        };
        let prefix = if relative == Path::new(".") {
            Path::new("")
        } else {
            relative
        };
        let mut out = super::TreeScan::default();
        scan_walk(&directory, prefix, limit, filter, &mut 0, &mut out)?;
        out.entries.sort();
        out.skipped.sort();
        Ok(out)
    }
}

/// How [`Root::scan`] treats one entry, decided from its relative path alone
/// (before anything is opened).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScanFilter {
    /// Walk normally.
    Walk,
    /// Not reported, not opened, not counted (protected names).
    Hide,
    /// Reported as an entry but not descended (heavy ignored folders).
    ListOnly,
}

/// Result of a tolerant tree scan: symlinks, hard links, special files and
/// unreadable entries are reported in `skipped` with a reason instead of
/// failing the whole walk.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TreeScan {
    pub entries: Vec<(std::path::PathBuf, bool)>,
    pub skipped: Vec<(std::path::PathBuf, &'static str)>,
}

impl Root {
    /// Tolerant bounded walk for discovery tools (never for digests): unlike
    /// [`Root::entries`] it skips, and reports, objects it will not follow.
    /// Exceeding `max_entries` is an error naming the limit and the entry.
    pub fn scan(
        &self,
        relative: &Path,
        max_entries: usize,
        filter: &dyn Fn(&Path) -> ScanFilter,
    ) -> std::io::Result<TreeScan> {
        #[cfg(unix)]
        {
            manifest_files::scan(&self.directory, relative, max_entries, filter)
        }
        #[cfg(not(unix))]
        {
            let _ = (relative, max_entries, filter);
            Err(denied())
        }
    }
    /// Identity of the object at `relative` without following a final
    /// symlink (parents are opened with O_NOFOLLOW). Used to compare what a
    /// name really resolves to on case/normalization-insensitive filesystems.
    pub fn identity_of(&self, relative: &Path) -> std::io::Result<Identity> {
        #[cfg(unix)]
        {
            use std::os::fd::AsRawFd;
            let (dir, leaf) = os::parent(&self.directory, relative)?;
            let leaf = os::name(&leaf)?;
            let mut st: libc::stat = unsafe { std::mem::zeroed() };
            if unsafe {
                libc::fstatat(
                    dir.as_raw_fd(),
                    leaf.as_ptr(),
                    &mut st,
                    libc::AT_SYMLINK_NOFOLLOW,
                )
            } != 0
            {
                return Err(std::io::Error::last_os_error());
            }
            #[allow(clippy::unnecessary_cast)]
            Ok(Identity {
                device: st.st_dev as u64,
                inode: st.st_ino as u64,
            })
        }
        #[cfg(not(unix))]
        {
            let _ = relative;
            Err(denied())
        }
    }
}
impl Root {
    /// Bounded files manifest. Rejects every symlink, special object and
    /// multiply linked file; errors never become partial success. This is not
    /// an atomic snapshot of an entire changing directory tree.
    pub fn files(
        &self,
        relative: &Path,
        max_entries: usize,
    ) -> std::io::Result<Vec<std::path::PathBuf>> {
        #[cfg(unix)]
        {
            Ok(
                manifest_files::entries(&self.directory, relative, max_entries)?
                    .into_iter()
                    .filter_map(|(path, directory)| if directory { None } else { Some(path) })
                    .collect(),
            )
        }
        #[cfg(not(unix))]
        {
            let _ = (relative, max_entries);
            Err(denied())
        }
    }
}

impl Root {
    /// Deterministic bounded tree manifest; bool distinguishes directories,
    /// including empty ones, from regular files. Same restrictions as files().
    pub fn entries(
        &self,
        relative: &Path,
        max_entries: usize,
    ) -> std::io::Result<Vec<(std::path::PathBuf, bool)>> {
        #[cfg(unix)]
        {
            manifest_files::entries(&self.directory, relative, max_entries)
        }
        #[cfg(not(unix))]
        {
            let _ = (relative, max_entries);
            Err(denied())
        }
    }
}

// ───────────── Existing-file replacement (optimistic concurrency) ─────────────
//
// Contract of `Root::replace`:
// * The caller proves which revision it edited by passing the SHA-256 of the
//   bytes it read earlier. The target is re-read through the pinned parent
//   descriptor (openat + O_NOFOLLOW), must be a regular file with exactly one
//   link, owned by this user, and below the size bound, and its digest must
//   equal the expectation before any byte is written.
// * The new bytes are written and fsynced into a fresh file inside the
//   caller-supplied PRIVATE staging root (same filesystem, outside every
//   worker grant; see `create_new`). The staging name never lives inside the
//   workspace, so a workspace writer cannot rename or substitute it (audit S02).
// * Immediately before the commit the target name, the open target descriptor,
//   the parent path from the root and the staging name are re-checked (full
//   stat stamp including ctime, which userland cannot forge).
// * Commit is an atomic exchange of the two names (macOS renameatx_np
//   RENAME_SWAP, Linux renameat2 RENAME_EXCHANGE); nothing is ever truncated
//   in place, so the target name always resolves to either the complete old
//   file or the complete fsynced new file, including after a crash.
// * After the exchange the swapped-out object (now private) must be the same
//   inode, still singly linked, and still hash to the expected digest, the new
//   name must resolve to the staged inode and the parent path must still be the
//   same directory. Otherwise the call fails with FILE_CHANGED_DURING_COMMIT
//   and no concurrent bytes are dropped: if the name no longer holds our file
//   (a concurrent save renamed over it) the exchange is NOT reversed; if it
//   does, the exchange is reversed. Any object that ends up outside the name
//   and is not our own untouched staged file (the displaced old revision, a
//   new file someone appended to, a foreign file) is moved or copied to a
//   durable sibling `<name>.omniget-recovered-<ms>-<id>` and the error reads
//   `<CODE>; recovered: <relative path>`. The same holds for
//   FILE_COMMIT_ROLLBACK_FAILED, because the private staging root is a
//   temporary directory that its owner deletes.
//
// This is NOT a compare-and-swap in the kernel. Residual windows, documented:
// (1) a process that already holds a descriptor to the old inode and writes
//     AFTER the post-exchange verification writes into the replaced (now
//     unlinked) inode and that write is lost, as with every rename-based save;
// (2) between the exchange and the verification (microseconds) the new content
//     is visible under the name even if the commit is then reversed;
// (3) a directory moved out of the root between the last check and the
//     exchange receives the new file for that same short window before it is
//     swapped back. An exclusive advisory lock (flock) is held on the old
//     inode for the whole operation, so cooperating writers are excluded.
// Extended attributes/ACLs of the old file are not carried over; permission
// bits (0o777) are.

/// Stable error codes returned by [`Root::replace`].
pub mod replace_codes {
    pub const PATH_INVALID: &str = "FILE_PATH_INVALID";
    pub const NOT_FOUND: &str = "FILE_NOT_FOUND";
    pub const ACCESS_DENIED: &str = "FILE_ACCESS_DENIED";
    pub const SYMLINK: &str = "FILE_SYMLINK_REFUSED";
    pub const PARENT_NOT_DIRECTORY: &str = "FILE_PARENT_NOT_DIRECTORY";
    pub const NOT_REGULAR: &str = "FILE_NOT_REGULAR";
    pub const HARDLINKED: &str = "FILE_HARDLINKED";
    pub const OWNER: &str = "FILE_OWNER_MISMATCH";
    pub const SIZE: &str = "FILE_SIZE_LIMIT";
    pub const DIGEST_FORMAT: &str = "EXPECTED_SHA256_INVALID";
    pub const CONFLICT: &str = "FILE_REVISION_CONFLICT";
    pub const CHANGED: &str = "FILE_CHANGED_DURING_COMMIT";
    pub const LOCKED: &str = "FILE_LOCKED";
    pub const ROLLBACK_FAILED: &str = "FILE_COMMIT_ROLLBACK_FAILED";
    pub const UNSUPPORTED: &str = "ATOMIC_REPLACE_UNSUPPORTED";
    pub const STAGING: &str = "STAGING_UNAVAILABLE";
    pub const ROOT: &str = "WORKSPACE_ROOT_CHANGED";
    pub const IO: &str = "FILE_IO_ERROR";
}

#[derive(Clone, Debug, serde::Serialize, PartialEq, Eq)]
pub struct Replaced {
    pub previous_digest: String,
    pub digest: String,
    pub bytes: u64,
    pub identity: Identity,
}

#[cfg(test)]
pub mod replace_test_hook {
    //! Deterministic race injection for tests: called with the stage name
    //! ("after_read", "before_precheck", "before_swap", "after_swap").
    use std::cell::RefCell;
    type Hook = Box<dyn FnMut(&str)>;
    thread_local! { static HOOK: RefCell<Option<Hook>> = RefCell::new(None); }
    pub fn set(f: impl FnMut(&str) + 'static) {
        HOOK.with(|h| *h.borrow_mut() = Some(Box::new(f)));
    }
    pub fn clear() {
        HOOK.with(|h| *h.borrow_mut() = None);
    }
    pub(crate) fn fire(stage: &str) {
        let taken = HOOK.with(|h| h.borrow_mut().take());
        if let Some(mut f) = taken {
            f(stage);
            HOOK.with(|h| {
                let mut b = h.borrow_mut();
                if b.is_none() {
                    *b = Some(f);
                }
            });
        }
    }
}

fn valid_sha256(value: &str) -> Option<String> {
    let v = value.to_ascii_lowercase();
    (v.len() == 64 && v.bytes().all(|b| b.is_ascii_hexdigit())).then_some(v)
}

#[cfg(unix)]
mod replace_os {
    use super::replace_codes as c;
    use super::*;
    use std::ffi::CStr;
    use std::os::fd::AsRawFd;
    use std::os::unix::ffi::OsStrExt;

    fn fire(_stage: &str) {
        #[cfg(test)]
        super::replace_test_hook::fire(_stage);
    }

    #[derive(Clone, Copy, PartialEq, Eq, Debug)]
    struct Stamp {
        dev: u64,
        ino: u64,
        nlink: u64,
        size: i64,
        mtime: i64,
        mtime_nsec: i64,
        ctime: i64,
        ctime_nsec: i64,
        mode: u32,
        uid: u32,
    }
    #[allow(clippy::unnecessary_cast)]
    fn stamp(st: &libc::stat) -> Stamp {
        Stamp {
            dev: st.st_dev as u64,
            ino: st.st_ino as u64,
            nlink: st.st_nlink as u64,
            size: st.st_size as i64,
            mtime: st.st_mtime as i64,
            mtime_nsec: st.st_mtime_nsec as i64,
            ctime: st.st_ctime as i64,
            ctime_nsec: st.st_ctime_nsec as i64,
            mode: st.st_mode as u32,
            uid: st.st_uid as u32,
        }
    }
    fn fstat(file: &File) -> Result<Stamp, String> {
        let mut st: libc::stat = unsafe { std::mem::zeroed() };
        if unsafe { libc::fstat(file.as_raw_fd(), &mut st) } != 0 {
            return Err(c::IO.into());
        }
        Ok(stamp(&st))
    }
    fn stat_at(dir: &File, name: &CStr) -> std::io::Result<Stamp> {
        let mut st: libc::stat = unsafe { std::mem::zeroed() };
        if unsafe {
            libc::fstatat(
                dir.as_raw_fd(),
                name.as_ptr(),
                &mut st,
                libc::AT_SYMLINK_NOFOLLOW,
            )
        } != 0
        {
            return Err(std::io::Error::last_os_error());
        }
        Ok(stamp(&st))
    }
    fn is_regular(s: &Stamp) -> bool {
        s.mode & (libc::S_IFMT as u32) == libc::S_IFREG as u32
    }
    fn open_code(e: &std::io::Error) -> &'static str {
        match e.raw_os_error() {
            Some(libc::ENOENT) => c::NOT_FOUND,
            Some(libc::ELOOP) | Some(libc::EMLINK) => c::SYMLINK,
            Some(libc::ENOTDIR) => c::PARENT_NOT_DIRECTORY,
            Some(libc::EACCES) | Some(libc::EPERM) => c::ACCESS_DENIED,
            Some(libc::ENXIO) | Some(libc::EOPNOTSUPP) => c::NOT_REGULAR,
            _ if e.kind() == std::io::ErrorKind::PermissionDenied => c::ACCESS_DENIED,
            _ => c::IO,
        }
    }
    fn check_target(s: &Stamp, max: u64) -> Result<(), String> {
        if !is_regular(s) {
            return Err(c::NOT_REGULAR.into());
        }
        if s.nlink != 1 {
            return Err(c::HARDLINKED.into());
        }
        if s.uid != unsafe { libc::geteuid() } as u32 {
            return Err(c::OWNER.into());
        }
        if s.size < 0 || s.size as u64 > max {
            return Err(c::SIZE.into());
        }
        Ok(())
    }
    fn read_all(file: &mut File, max: u64) -> Result<(Vec<u8>, String), String> {
        file.seek(SeekFrom::Start(0)).map_err(|_| c::IO)?;
        let mut out = Vec::new();
        let mut buf = [0u8; 65536];
        loop {
            let n = match file.read(&mut buf) {
                Ok(n) => n,
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    return Err(c::NOT_REGULAR.into())
                }
                Err(_) => return Err(c::IO.into()),
            };
            if n == 0 {
                break;
            }
            if out.len() as u64 + n as u64 > max {
                return Err(c::SIZE.into());
            }
            out.extend_from_slice(&buf[..n]);
        }
        let digest = format!("{:x}", Sha256::digest(&out));
        Ok((out, digest))
    }
    fn swap(a_dir: &File, a: &CStr, b_dir: &File, b: &CStr) -> std::io::Result<()> {
        #[cfg(target_os = "macos")]
        let r = unsafe {
            libc::renameatx_np(
                a_dir.as_raw_fd(),
                a.as_ptr(),
                b_dir.as_raw_fd(),
                b.as_ptr(),
                libc::RENAME_SWAP,
            )
        };
        #[cfg(any(target_os = "linux", target_os = "android"))]
        let r = unsafe {
            libc::syscall(
                libc::SYS_renameat2,
                a_dir.as_raw_fd(),
                a.as_ptr(),
                b_dir.as_raw_fd(),
                b.as_ptr(),
                libc::RENAME_EXCHANGE,
            )
        } as i32;
        #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "android")))]
        let r = {
            let _ = (a_dir, a, b_dir, b);
            return Err(std::io::Error::from(std::io::ErrorKind::Unsupported));
        };
        if r != 0 {
            return Err(std::io::Error::last_os_error());
        }
        Ok(())
    }
    /// Move without replacing any existing destination (rollback fallback).
    fn move_noreplace(a_dir: &File, a: &CStr, b_dir: &File, b: &CStr) -> std::io::Result<()> {
        #[cfg(target_os = "macos")]
        let r = unsafe {
            libc::renameatx_np(
                a_dir.as_raw_fd(),
                a.as_ptr(),
                b_dir.as_raw_fd(),
                b.as_ptr(),
                libc::RENAME_EXCL,
            )
        };
        #[cfg(any(target_os = "linux", target_os = "android"))]
        let r = unsafe {
            libc::syscall(
                libc::SYS_renameat2,
                a_dir.as_raw_fd(),
                a.as_ptr(),
                b_dir.as_raw_fd(),
                b.as_ptr(),
                libc::RENAME_NOREPLACE,
            )
        } as i32;
        #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "android")))]
        let r = {
            let _ = (a_dir, a, b_dir, b);
            return Err(std::io::Error::from(std::io::ErrorKind::Unsupported));
        };
        if r != 0 {
            return Err(std::io::Error::last_os_error());
        }
        Ok(())
    }
    fn same_parent(root: &File, relative: &Path, expected: &Identity) -> bool {
        os::parent(root, relative)
            .ok()
            .and_then(|(d, _)| os::identity(&d).ok())
            .is_some_and(|i| &i == expected)
    }

    /// `<leaf>.omniget-recovered-<ms>-<id>`: visible next to the target, never
    /// under the reserved `.omniget-` prefix, and bounded to NAME_MAX.
    fn recovery_name(leaf: &std::ffi::OsStr) -> std::ffi::OsString {
        let lossy = leaf.to_string_lossy();
        let mut head = String::new();
        for ch in lossy.chars() {
            if head.len() + ch.len_utf8() > 160 {
                break;
            }
            head.push(ch);
        }
        let ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0);
        let id = uuid::Uuid::new_v4().simple().to_string();
        std::ffi::OsString::from(format!("{head}.omniget-recovered-{ms}-{}", &id[..8]))
    }
    fn recovered_path(relative: &Path, name: &std::ffi::OsStr) -> String {
        relative
            .parent()
            .unwrap_or(Path::new(""))
            .join(name)
            .to_string_lossy()
            .into_owned()
    }
    /// Moves the private staging object next to the target (durable), or
    /// copies its bytes there when the move is impossible. Returns the
    /// workspace-relative path, or the reason nothing could be kept.
    fn preserve_staged(
        staging: &File,
        tmp: &CStr,
        dir: &File,
        leaf: &std::ffi::OsStr,
        relative: &Path,
    ) -> String {
        let name = recovery_name(leaf);
        let Ok(name_c) = os::name(&name) else {
            return "unrecoverable: invalid name".into();
        };
        if move_noreplace(staging, tmp, dir, &name_c).is_ok() {
            let _ = dir.sync_all();
            let _ = staging.sync_all();
            return recovered_path(relative, &name);
        }
        let tmp_os = std::ffi::OsStr::from_bytes(tmp.to_bytes());
        match os::at(staging, tmp_os, libc::O_RDONLY | libc::O_NONBLOCK) {
            Ok(mut f) => {
                let kept = preserve_bytes_named(&mut f, dir, &name, relative, u64::MAX);
                if !kept.starts_with("unrecoverable") {
                    unsafe {
                        libc::unlinkat(staging.as_raw_fd(), tmp.as_ptr(), 0);
                    }
                }
                kept
            }
            Err(_) => "unrecoverable: staging object vanished".into(),
        }
    }
    fn preserve_bytes(
        f: &mut File,
        dir: &File,
        leaf: &std::ffi::OsStr,
        relative: &Path,
        max: u64,
    ) -> String {
        preserve_bytes_named(f, dir, &recovery_name(leaf), relative, max)
    }
    fn preserve_bytes_named(
        f: &mut File,
        dir: &File,
        name: &std::ffi::OsStr,
        relative: &Path,
        max: u64,
    ) -> String {
        let bytes = match read_all(f, max) {
            Ok((b, _)) => b,
            Err(e) => return format!("unrecoverable: {e}"),
        };
        let written =
            os::at(dir, name, libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL).and_then(|mut out| {
                out.write_all(&bytes)?;
                out.sync_all()
            });
        match written {
            Ok(()) => {
                let _ = dir.sync_all();
                recovered_path(relative, name)
            }
            Err(e) => format!("unrecoverable: {e}"),
        }
    }
    fn with_recovery(code: &str, kept: &[String]) -> String {
        if kept.is_empty() {
            code.to_string()
        } else {
            format!("{code}; recovered: {}", kept.join(", "))
        }
    }

    pub fn replace<F>(
        root: &File,
        staging: &File,
        relative: &Path,
        expected: &str,
        max: u64,
        transform: F,
    ) -> Result<Replaced, String>
    where
        F: FnOnce(&[u8]) -> Result<Vec<u8>, String>,
    {
        let (dir, leaf) = os::parent(root, relative).map_err(|e| open_code(&e))?;
        let dir_id = os::identity(&dir).map_err(|_| c::IO)?;
        let leaf_c = os::name(&leaf).map_err(|_| c::PATH_INVALID)?;
        // NONBLOCK: a FIFO swapped in must not hang the executor before fstat.
        let mut target =
            os::at(&dir, &leaf, libc::O_RDONLY | libc::O_NONBLOCK).map_err(|e| open_code(&e))?;
        let before = fstat(&target)?;
        check_target(&before, max)?;
        if unsafe { libc::flock(target.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
            let e = std::io::Error::last_os_error();
            return Err(if e.raw_os_error() == Some(libc::EWOULDBLOCK) {
                c::LOCKED
            } else {
                c::IO
            }
            .into());
        }
        let (old, previous) = read_all(&mut target, max)?;
        if previous != expected {
            return Err(c::CONFLICT.into());
        }
        if fstat(&target)? != before {
            return Err(c::CHANGED.into());
        }
        fire("after_read");
        let new = transform(&old)?;
        if new.len() as u64 > max {
            return Err(c::SIZE.into());
        }
        let digest = format!("{:x}", Sha256::digest(&new));

        let tmp = format!(".omniget-{}.replace", uuid::Uuid::new_v4());
        let tmp_os = std::ffi::OsStr::new(&tmp);
        let tmp_c = os::name(tmp_os).map_err(|_| c::STAGING)?;
        let mut staged = os::at(staging, tmp_os, libc::O_RDWR | libc::O_CREAT | libc::O_EXCL)
            .map_err(|_| c::STAGING)?;
        let unlink_tmp = || unsafe { libc::unlinkat(staging.as_raw_fd(), tmp_c.as_ptr(), 0) };
        let staged_st = (|| -> Result<Stamp, String> {
            staged.write_all(&new).map_err(|_| c::IO)?;
            if unsafe { libc::fchmod(staged.as_raw_fd(), (before.mode & 0o777) as libc::mode_t) }
                != 0
            {
                return Err(c::IO.into());
            }
            staged.sync_all().map_err(|_| c::IO)?;
            fstat(&staged)
        })();
        let staged_st = match staged_st {
            Ok(s) => s,
            Err(e) => {
                unlink_tmp();
                return Err(e);
            }
        };

        fire("before_precheck");
        let precheck = stat_at(&dir, &leaf_c).is_ok_and(|s| s == before)
            && fstat(&target).is_ok_and(|s| s == before)
            && same_parent(root, relative, &dir_id)
            && stat_at(staging, &tmp_c)
                .is_ok_and(|s| s.dev == staged_st.dev && s.ino == staged_st.ino);
        if !precheck {
            unlink_tmp();
            return Err(c::CHANGED.into());
        }

        fire("before_swap");
        if let Err(e) = swap(staging, &tmp_c, &dir, &leaf_c) {
            unlink_tmp();
            return Err(match e.raw_os_error() {
                Some(libc::EINVAL) | Some(libc::ENOTSUP) | Some(libc::ENOSYS)
                | Some(libc::EXDEV) => c::UNSUPPORTED,
                Some(libc::ENOENT) => c::CHANGED,
                _ => c::IO,
            }
            .into());
        }
        fire("after_swap");

        let name_is_new =
            stat_at(&dir, &leaf_c).is_ok_and(|s| s.dev == staged_st.dev && s.ino == staged_st.ino);
        let verified = name_is_new
            && same_parent(root, relative, &dir_id)
            && os::at(staging, tmp_os, libc::O_RDONLY | libc::O_NONBLOCK)
                .ok()
                .is_some_and(|mut f| {
                    fstat(&f).is_ok_and(|s| {
                        s.dev == before.dev && s.ino == before.ino && is_regular(&s) && s.nlink == 1
                    }) && read_all(&mut f, max).is_ok_and(|(_, d)| d == expected)
                });
        if !verified {
            // Audit F-R1/F-R2/F-R3: nothing a concurrent actor wrote may be
            // dropped. The staging root is a private temp dir deleted by the
            // caller, so every object that does not end up under the target
            // name and is not provably our own untouched bytes is moved to a
            // durable sibling `<name>.omniget-recovered-*` next to the target,
            // and the error names it.
            let untouched_new = |f: &mut File| {
                fstat(f).is_ok_and(|s| {
                    s.dev == staged_st.dev
                        && s.ino == staged_st.ino
                        && s.size == staged_st.size
                        && s.mtime == staged_st.mtime
                        && s.mtime_nsec == staged_st.mtime_nsec
                }) && read_all(f, max).is_ok_and(|(_, d)| d == digest)
            };
            if !name_is_new {
                // The name no longer holds our file: a concurrent save (rename
                // over it) or a delete won. Never swap its object away.
                let mut recovered = Vec::new();
                if stat_at(&dir, &leaf_c).is_err_and(|e| e.raw_os_error() == Some(libc::ENOENT))
                    && move_noreplace(staging, &tmp_c, &dir, &leaf_c).is_ok()
                {
                    // Name deleted: the revision the caller based the edit on returns.
                } else if stat_at(staging, &tmp_c).is_ok() {
                    recovered.push(preserve_staged(staging, &tmp_c, &dir, &leaf, relative));
                }
                // Our unlinked inode: if someone wrote into it through the name
                // before replacing it, keep those bytes too (read via our fd).
                if !untouched_new(&mut staged) {
                    recovered.push(preserve_bytes(&mut staged, &dir, &leaf, relative, max));
                }
                let _ = dir.sync_all();
                return Err(with_recovery(c::CHANGED, &recovered));
            }
            // Our file is under the name: reverse the exchange.
            if swap(staging, &tmp_c, &dir, &leaf_c).is_err() {
                // The name keeps the new revision; the swapped-out one must
                // survive the private staging dir.
                let kept = preserve_staged(staging, &tmp_c, &dir, &leaf, relative);
                let _ = dir.sync_all();
                return Err(with_recovery(c::ROLLBACK_FAILED, &[kept]));
            }
            // Whatever is now under the private name came back from the
            // workspace: drop it only if it is our own untouched staged file.
            let mut recovered = Vec::new();
            let ours_untouched = os::at(staging, tmp_os, libc::O_RDONLY | libc::O_NONBLOCK)
                .ok()
                .is_some_and(|mut f| untouched_new(&mut f));
            if ours_untouched {
                unlink_tmp();
            } else if stat_at(staging, &tmp_c).is_ok() {
                recovered.push(preserve_staged(staging, &tmp_c, &dir, &leaf, relative));
            }
            let _ = dir.sync_all();
            return Err(with_recovery(c::CHANGED, &recovered));
        }
        // Old revision now only exists privately; drop it and make both
        // directory entries durable.
        unlink_tmp();
        dir.sync_all().map_err(|_| c::IO)?;
        let _ = staging.sync_all();
        drop(target);
        Ok(Replaced {
            previous_digest: previous,
            digest,
            bytes: new.len() as u64,
            identity: Identity {
                device: staged_st.dev,
                inode: staged_st.ino,
            },
        })
    }
}

impl Root {
    /// Replace an existing regular file whose current SHA-256 equals
    /// `expected_sha256`. `transform` receives exactly the verified bytes and
    /// returns the new content. `staging` has the same requirements as in
    /// [`Root::create_new`]. See the contract above for residual races.
    pub fn replace<F>(
        &self,
        staging: &Root,
        relative: &Path,
        expected_sha256: &str,
        max_bytes: u64,
        transform: F,
    ) -> Result<Replaced, String>
    where
        F: FnOnce(&[u8]) -> Result<Vec<u8>, String>,
    {
        use replace_codes as c;
        if self.identity == staging.identity || self.identity.device != staging.identity.device {
            return Err(c::STAGING.into());
        }
        let expected = valid_sha256(expected_sha256).ok_or(c::DIGEST_FORMAT)?;
        let raw = relative.as_os_str().as_encoded_bytes();
        if raw.is_empty()
            || raw.starts_with(b"/")
            || raw
                .split(|b| *b == b'/')
                .any(|s| s.is_empty() || s == b"." || s == b"..")
        {
            return Err(c::PATH_INVALID.into());
        }
        #[cfg(unix)]
        {
            // The descriptor itself is pinned; this only guards against a
            // mismatched Root handed in by a caller.
            if os::identity(&self.directory).map_err(|_| c::IO)? != self.identity {
                return Err(c::ROOT.into());
            }
            replace_os::replace(
                &self.directory,
                &staging.directory,
                relative,
                &expected,
                max_bytes,
                transform,
            )
        }
        #[cfg(not(unix))]
        {
            let _ = (expected, max_bytes, transform);
            Err(c::UNSUPPORTED.into())
        }
    }
}

#[cfg(all(test, unix))]
mod replace_tests {
    use super::*;
    fn fixture() -> (tempfile::TempDir, Root, Root, std::path::PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path().canonicalize().unwrap();
        std::fs::create_dir(base.join("ws")).unwrap();
        std::fs::create_dir(base.join("stage")).unwrap();
        let root = Root::open(&base.join("ws"), None).unwrap();
        let stage = Root::open(&base.join("stage"), None).unwrap();
        (dir, root, stage, base.join("ws"))
    }
    fn sha(b: &[u8]) -> String {
        format!("{:x}", Sha256::digest(b))
    }
    #[test]
    fn replace_commits_verified_revision_and_new_inode() {
        let (_d, root, stage, ws) = fixture();
        std::fs::write(ws.join("f"), b"old").unwrap();
        let r = root
            .replace(&stage, Path::new("f"), &sha(b"old"), 64, |old| {
                assert_eq!(old, b"old");
                Ok(b"new".to_vec())
            })
            .unwrap();
        assert_eq!(
            (r.previous_digest, r.digest, r.bytes),
            (sha(b"old"), sha(b"new"), 3)
        );
        assert_eq!(std::fs::read(ws.join("f")).unwrap(), b"new");
        assert!(
            root.replace(&stage, Path::new("f"), &sha(b"old"), 64, |_| Ok(vec![]))
                .unwrap_err()
                == replace_codes::CONFLICT
        );
        assert_eq!(
            root.replace(&root, Path::new("f"), &sha(b"new"), 64, |_| Ok(vec![]))
                .unwrap_err(),
            replace_codes::STAGING
        );
        assert_eq!(
            root.replace(&stage, Path::new("f"), &sha(b"new"), 2, |_| Ok(vec![]))
                .unwrap_err(),
            replace_codes::SIZE
        );
        assert_eq!(
            root.replace(&stage, Path::new("f"), &sha(b"new"), 64, |_| Ok(vec![
                0;
                65
            ]))
            .unwrap_err(),
            replace_codes::SIZE
        );
        assert_eq!(
            root.replace(&stage, Path::new("../f"), &sha(b"new"), 64, |_| Ok(vec![]))
                .unwrap_err(),
            replace_codes::PATH_INVALID
        );
        assert_eq!(std::fs::read_dir(stage_path(&ws)).unwrap().count(), 0);
    }
    fn stage_path(ws: &Path) -> std::path::PathBuf {
        ws.parent().unwrap().join("stage")
    }
    #[test]
    fn replace_respects_cooperative_lock_and_swap_back_restores_foreign_object() {
        use std::os::fd::AsRawFd;
        let (_d, root, stage, ws) = fixture();
        std::fs::write(ws.join("f"), b"old").unwrap();
        let holder = File::open(ws.join("f")).unwrap();
        assert_eq!(unsafe { libc::flock(holder.as_raw_fd(), libc::LOCK_EX) }, 0);
        assert_eq!(
            root.replace(&stage, Path::new("f"), &sha(b"old"), 64, |_| Ok(
                b"new".to_vec()
            ))
            .unwrap_err(),
            replace_codes::LOCKED
        );
        drop(holder);
        // Foreign regular file substituted under the name right before the
        // exchange: the exchange is reversed and the foreign file stays.
        let w = ws.clone();
        replace_test_hook::set(move |s| {
            if s == "before_swap" {
                std::fs::rename(w.join("f"), w.join("f.orig")).unwrap();
                std::fs::write(w.join("f"), b"old").unwrap();
            }
        });
        let err = root
            .replace(&stage, Path::new("f"), &sha(b"old"), 64, |_| {
                Ok(b"new".to_vec())
            })
            .unwrap_err();
        replace_test_hook::clear();
        assert_eq!(err, replace_codes::CHANGED);
        assert_eq!(std::fs::read(ws.join("f")).unwrap(), b"old");
        assert_eq!(std::fs::read(ws.join("f.orig")).unwrap(), b"old");
        assert_eq!(std::fs::read_dir(stage_path(&ws)).unwrap().count(), 0);
    }

    fn recovered(ws: &Path, err: &str) -> Vec<u8> {
        let rel = err
            .split("; recovered: ")
            .nth(1)
            .unwrap_or_else(|| panic!("no recovery path in {err}"));
        assert!(
            rel.contains(".omniget-recovered-") && !rel.starts_with(".omniget-"),
            "{rel}"
        );
        std::fs::read(ws.join(rel)).unwrap()
    }
    /// Audit F-R1 (reproduction turned regression): a concurrent atomic save
    /// (write tmp + rename over the target) between the exchange and the
    /// verification wins; its bytes stay under the name and the displaced old
    /// revision is kept in a durable sibling, never in the private staging.
    #[test]
    fn f_r1_concurrent_atomic_save_after_swap_is_kept_and_old_revision_recovered() {
        let (_d, root, stage, ws) = fixture();
        std::fs::write(ws.join("f"), b"old").unwrap();
        let w = ws.clone();
        replace_test_hook::set(move |s| {
            if s == "after_swap" {
                std::fs::write(w.join(".f.editor-tmp"), b"EDITOR SAVE").unwrap();
                std::fs::rename(w.join(".f.editor-tmp"), w.join("f")).unwrap();
            }
        });
        let err = root
            .replace(&stage, Path::new("f"), &sha(b"old"), 64, |_| {
                Ok(b"ours".to_vec())
            })
            .unwrap_err();
        replace_test_hook::clear();
        assert!(err.starts_with(replace_codes::CHANGED), "{err}");
        assert_eq!(std::fs::read(ws.join("f")).unwrap(), b"EDITOR SAVE");
        assert_eq!(recovered(&ws, &err), b"old");
        assert_eq!(
            std::fs::read_dir(stage_path(&ws)).unwrap().count(),
            0,
            "nothing left in the private staging"
        );
    }
    /// Audit F-R2: bytes appended into the NEW file through the name after the
    /// exchange survive when the verification then fails for another reason
    /// (a stale writer touched the old inode).
    #[test]
    fn f_r2_bytes_appended_to_new_file_during_window_are_recovered() {
        use std::io::Write;
        let (_d, root, stage, ws) = fixture();
        std::fs::write(ws.join("f"), b"old").unwrap();
        let mut old_fd = std::fs::OpenOptions::new()
            .write(true)
            .open(ws.join("f"))
            .unwrap();
        let w = ws.clone();
        replace_test_hook::set(move |s| {
            if s == "after_swap" {
                let mut f = std::fs::OpenOptions::new()
                    .append(true)
                    .open(w.join("f"))
                    .unwrap();
                f.write_all(b" +THEIR APPEND").unwrap();
                old_fd.write_all(b"X").unwrap();
            }
        });
        let err = root
            .replace(&stage, Path::new("f"), &sha(b"old"), 64, |_| {
                Ok(b"ours".to_vec())
            })
            .unwrap_err();
        replace_test_hook::clear();
        assert!(err.starts_with(replace_codes::CHANGED), "{err}");
        assert_eq!(
            std::fs::read(ws.join("f")).unwrap(),
            b"Xld",
            "the stale writer's revision is back under the name"
        );
        assert_eq!(recovered(&ws, &err), b"ours +THEIR APPEND");
        assert_eq!(std::fs::read_dir(stage_path(&ws)).unwrap().count(), 0);
    }
    /// Audit F-R3: when the reverse exchange fails the swapped-out revision
    /// must survive the private staging dir (a TempDir its owner deletes).
    #[test]
    fn f_r3_rollback_failed_keeps_the_swapped_out_revision_outside_staging() {
        use std::io::Write;
        use std::os::unix::fs::PermissionsExt;
        let (_d, root, stage, ws) = fixture();
        std::fs::write(ws.join("f"), b"old").unwrap();
        let mut old_fd = std::fs::OpenOptions::new()
            .write(true)
            .open(ws.join("f"))
            .unwrap();
        let st = stage_path(&ws);
        let st2 = st.clone();
        replace_test_hook::set(move |s| {
            if s == "after_swap" {
                old_fd.write_all(b"X").unwrap();
                // Staging becomes unwritable: the reverse exchange cannot happen.
                std::fs::set_permissions(&st2, std::fs::Permissions::from_mode(0o500)).unwrap();
            }
        });
        let err = root
            .replace(&stage, Path::new("f"), &sha(b"old"), 64, |_| {
                Ok(b"ours".to_vec())
            })
            .unwrap_err();
        replace_test_hook::clear();
        std::fs::set_permissions(&st, std::fs::Permissions::from_mode(0o700)).unwrap();
        assert!(err.starts_with(replace_codes::ROLLBACK_FAILED), "{err}");
        assert_eq!(std::fs::read(ws.join("f")).unwrap(), b"ours");
        let kept = recovered(&ws, &err);
        assert_eq!(kept, b"Xld");
        // Deleting the staging dir (what its owner does) loses nothing.
        std::fs::remove_dir_all(&st).unwrap();
        assert_eq!(recovered(&ws, &err), b"Xld");
    }
    /// Audit F-EF2 support: the tolerant scan reports symlinks, FIFOs and hard
    /// links, lists ignored folders without descending, hides filtered names
    /// without counting them, and names the limit when it is exceeded.
    #[test]
    fn scan_skips_links_and_special_files_and_names_the_limit() {
        let (_d, root, _stage, ws) = fixture();
        std::fs::write(ws.join("a.txt"), b"a").unwrap();
        std::fs::create_dir_all(ws.join("node_modules/.bin")).unwrap();
        std::os::unix::fs::symlink("../x/cli.js", ws.join("node_modules/.bin/x")).unwrap();
        std::os::unix::fs::symlink("a.txt", ws.join("link")).unwrap();
        std::fs::hard_link(ws.join("a.txt"), ws.join("hard")).unwrap();
        let fifo = std::ffi::CString::new(ws.join("pipe").to_str().unwrap()).unwrap();
        assert_eq!(unsafe { libc::mkfifo(fifo.as_ptr(), 0o600) }, 0);
        std::fs::create_dir(ws.join(".git")).unwrap();
        for i in 0..20 {
            std::fs::write(ws.join(format!(".git/{i}")), b"").unwrap();
        }
        let filter = |p: &Path| match p.file_name().and_then(|n| n.to_str()) {
            Some(".git") => ScanFilter::Hide,
            Some("node_modules") => ScanFilter::ListOnly,
            _ => ScanFilter::Walk,
        };
        let scan = root.scan(Path::new("."), 10, &filter).unwrap();
        // a.txt and hard are the same inode (nlink 2): both reported, not listed.
        assert_eq!(
            scan.entries,
            vec![(std::path::PathBuf::from("node_modules"), true)]
        );
        assert!(
            !scan.entries.iter().any(|(p, _)| p.starts_with(".git")),
            "hidden names never listed"
        );
        let reasons: std::collections::BTreeMap<_, _> = scan.skipped.iter().cloned().collect();
        assert_eq!(reasons.get(Path::new("link")), Some(&"symlink"));
        assert_eq!(reasons.get(Path::new("hard")), Some(&"hardlink"));
        assert_eq!(reasons.get(Path::new("a.txt")), Some(&"hardlink"));
        assert_eq!(reasons.get(Path::new("pipe")), Some(&"special"));
        assert_eq!(reasons.get(Path::new("node_modules")), Some(&"ignored_dir"));
        let err = root
            .scan(Path::new("."), 2, &filter)
            .unwrap_err()
            .to_string();
        assert!(
            err.starts_with("FILE_TREE_LIMIT") && err.contains("more than 2 entries"),
            "{err}"
        );
        // The strict manifest keeps failing closed for digests.
        assert!(root.entries(Path::new("."), 1000).is_err());
    }
}

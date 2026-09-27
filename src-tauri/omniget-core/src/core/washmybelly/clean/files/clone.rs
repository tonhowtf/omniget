//! Replace a duplicate with an APFS clone of the kept file: both names stay,
//! the blocks are shared until one of them changes. Clone-swap idea from
//! MacSai `FileConsolidator.swift` (<https://github.com/iliyami/MacSai>,
//! BSD-3-Clause, Copyright (c) 2026, Mac Sai Contributors): clone next to the copy, check the
//! clone byte by byte, carry the copy's permissions and times over, then an
//! atomic `rename` on top. Any failure leaves the copy untouched.

use std::path::Path;

use super::super::super::fsops::Fingerprint;

/// Swap `replace` for a clone of `keep`. Revalidates `replace` against the
/// scan's fingerprint and the guard, and refuses unless both files hold the
/// same bytes. Returns the allocated bytes the copy held.
#[cfg(target_os = "macos")]
pub fn apfs_clone_replace(keep: &Path, replace: &Path, fp: &Fingerprint) -> Result<u64, String> {
    use std::os::unix::ffi::OsStrExt;
    use std::os::unix::fs::MetadataExt;

    use super::super::super::fsops;
    use super::super::super::protect::{self, Guard};

    protect::check(replace, Guard::Picked, None)?;
    protect::check(keep, Guard::Picked, None)?;
    fsops::unchanged(replace, fp)?;
    let km =
        std::fs::symlink_metadata(keep).map_err(|e| format!("GONE: {}: {e}", keep.display()))?;
    let rm = std::fs::symlink_metadata(replace)
        .map_err(|e| format!("GONE: {}: {e}", replace.display()))?;
    if !km.is_file() || !rm.is_file() || fsops::is_link(&km) || fsops::is_link(&rm) {
        return Err("CHANGED: both paths must be regular files".into());
    }
    if km.dev() != rm.dev() {
        return Err("CHANGED: the files are on different volumes".into());
    }
    if km.ino() == rm.ino() {
        return Err("CHANGED: both names are already the same file".into());
    }
    if km.len() != rm.len() {
        return Err("CHANGED: the files no longer have the same size".into());
    }
    let reclaimed = fsops::allocated(&rm);
    let name = replace
        .file_name()
        .ok_or("CHANGED: no file name")?
        .to_string_lossy()
        .to_string();
    let parent = replace.parent().ok_or("CHANGED: no parent folder")?;
    let tmp = parent.join(format!(
        ".{name}.wmb-clone-{}",
        uuid::Uuid::new_v4().simple()
    ));

    let c = |p: &Path| {
        std::ffi::CString::new(p.as_os_str().as_bytes()).map_err(|_| "path with NUL".to_string())
    };
    let (src, dst) = (c(keep)?, c(&tmp)?);
    const CLONE_NOFOLLOW: u32 = 0x0001;
    // SAFETY: two valid NUL-terminated paths; clonefile does not keep them.
    if unsafe { libc::clonefile(src.as_ptr(), dst.as_ptr(), CLONE_NOFOLLOW) } != 0 {
        return Err(format!(
            "clonefile failed: {}",
            std::io::Error::last_os_error()
        ));
    }
    let cleanup = |e: String| {
        let _ = std::fs::remove_file(&tmp);
        e
    };
    if !same_bytes(&tmp, replace).map_err(|e| cleanup(e.to_string()))? {
        return Err(cleanup(
            "CHANGED: the files no longer hold the same bytes".into(),
        ));
    }
    // The copy keeps its own times and permissions (times first: the mode
    // may be read-only).
    if let Ok(f) = std::fs::File::open(&tmp) {
        let mut times = std::fs::FileTimes::new();
        if let Ok(t) = rm.modified() {
            times = times.set_modified(t);
        }
        if let Ok(t) = rm.accessed() {
            times = times.set_accessed(t);
        }
        let _ = f.set_times(times);
    }
    let _ = std::fs::set_permissions(&tmp, rm.permissions());
    // Last look right before the swap.
    fsops::unchanged(replace, fp).map_err(&cleanup)?;
    std::fs::rename(&tmp, replace).map_err(|e| cleanup(format!("rename failed: {e}")))?;
    Ok(reclaimed)
}

#[cfg(not(target_os = "macos"))]
pub fn apfs_clone_replace(_keep: &Path, _replace: &Path, _fp: &Fingerprint) -> Result<u64, String> {
    Err("NOT_AVAILABLE: APFS clones exist only on macOS".into())
}

#[cfg(target_os = "macos")]
fn same_bytes(a: &Path, b: &Path) -> std::io::Result<bool> {
    let (mut fa, mut fb) = (std::fs::File::open(a)?, std::fs::File::open(b)?);
    let (mut ba, mut bb) = (vec![0u8; 1 << 20], vec![0u8; 1 << 20]);
    loop {
        let na = read_up_to(&mut fa, &mut ba)?;
        let nb = read_up_to(&mut fb, &mut bb)?;
        if na != nb || ba[..na] != bb[..nb] {
            return Ok(false);
        }
        if na == 0 {
            return Ok(true);
        }
    }
}

#[cfg(target_os = "macos")]
fn read_up_to(f: &mut std::fs::File, buf: &mut [u8]) -> std::io::Result<usize> {
    use std::io::Read;
    let mut n = 0;
    while n < buf.len() {
        match f.read(&mut buf[n..])? {
            0 => break,
            k => n += k,
        }
    }
    Ok(n)
}

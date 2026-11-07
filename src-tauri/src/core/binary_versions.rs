use std::path::{Path, PathBuf};

pub const KEEP_VERSIONS: usize = 3;

const SUFFIX: &str = ".omniget-prev.";

pub fn archive_path(binary: &Path, stamp_ms: u128) -> PathBuf {
    let name = binary
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("binary");
    binary.with_file_name(format!("{name}{SUFFIX}{stamp_ms}"))
}

pub fn parse_archive(file_name: &str, binary_name: &str) -> Option<u128> {
    let rest = file_name.strip_prefix(binary_name)?;
    let stamp = rest.strip_prefix(SUFFIX)?;
    stamp.parse().ok()
}

pub fn list_archived(binary: &Path) -> Vec<(u128, PathBuf)> {
    let (Some(dir), Some(name)) = (binary.parent(), binary.file_name().and_then(|n| n.to_str()))
    else {
        return Vec::new();
    };
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };

    let mut out: Vec<(u128, PathBuf)> = entries
        .filter_map(|e| e.ok())
        .filter_map(|e| {
            let fname = e.file_name();
            let fname = fname.to_str()?;
            parse_archive(fname, name).map(|stamp| (stamp, e.path()))
        })
        .collect();
    out.sort_by_key(|(stamp, _)| std::cmp::Reverse(*stamp));
    out
}

pub fn prune_list(archived: &[(u128, PathBuf)], keep: usize) -> Vec<PathBuf> {
    archived.iter().skip(keep).map(|(_, p)| p.clone()).collect()
}

pub fn archive_current(binary: &Path, stamp_ms: u128) -> std::io::Result<Option<PathBuf>> {
    if !binary.exists() {
        return Ok(None);
    }
    let dest = archive_path(binary, stamp_ms);
    std::fs::copy(binary, &dest)?;

    for old in prune_list(&list_archived(binary), KEEP_VERSIONS) {
        let _ = std::fs::remove_file(old);
    }
    Ok(Some(dest))
}

pub fn rollback_to(binary: &Path, archived: &Path) -> std::io::Result<()> {
    let name = binary
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or_else(|| std::io::Error::other("binary has no file name"))?;
    let arch_name = archived
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or_else(|| std::io::Error::other("archive has no file name"))?;

    if parse_archive(arch_name, name).is_none() {
        return Err(std::io::Error::other(format!(
            "{arch_name} is not an archive of {name}"
        )));
    }
    if archived.parent() != binary.parent() {
        return Err(std::io::Error::other(
            "archive must live beside the managed binary",
        ));
    }

    std::fs::rename(archived, binary)
}

#[cfg(test)]
mod archive_tests {
    use super::*;

    #[test]
    fn archiving_before_an_update_keeps_the_working_binary_in_place() {
        let dir = tempfile::tempdir().unwrap();
        let bin = dir.path().join("yt-dlp");
        std::fs::write(&bin, b"works").unwrap();
        let copy = archive_current(&bin, 1).unwrap().unwrap();
        assert_eq!(std::fs::read(&bin).unwrap(), b"works");
        assert_eq!(std::fs::read(&copy).unwrap(), b"works");
    }
}

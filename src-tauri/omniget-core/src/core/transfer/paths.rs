use std::path::{Path, PathBuf};

use super::TransferError;

const MAX_PATH_CHARS: usize = 4096;

fn invalid(message: String) -> TransferError {
    TransferError::InvalidInput(message)
}

pub fn normalize(path: &str) -> Result<String, TransferError> {
    if path.chars().count() > MAX_PATH_CHARS {
        return Err(invalid(format!(
            "path is longer than {MAX_PATH_CHARS} characters"
        )));
    }
    if path.chars().any(char::is_control) {
        return Err(invalid(
            "path has a line break or another control character".into(),
        ));
    }
    let mut parts = Vec::new();
    for part in path.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                return Err(invalid(
                    "path has a \"..\" component; pass the full path of the folder instead".into(),
                ))
            }
            other => parts.push(other),
        }
    }
    Ok(format!("/{}", parts.join("/")))
}

pub fn join(base: &str, input: &str) -> Result<String, TransferError> {
    if input.starts_with('/') {
        normalize(input)
    } else {
        normalize(&format!("{base}/{input}"))
    }
}

pub fn parent(path: &str) -> &str {
    match path.rfind('/') {
        Some(0) | None => "/",
        Some(at) => &path[..at],
    }
}

pub fn file_name(path: &str) -> Option<&str> {
    path.rsplit('/').next().filter(|name| !name.is_empty())
}

pub fn crumbs(path: &str) -> Vec<(String, String)> {
    let mut out = vec![("/".to_string(), "/".to_string())];
    let mut walked = String::new();
    for part in path.split('/').filter(|part| !part.is_empty()) {
        walked.push('/');
        walked.push_str(part);
        out.push((part.to_string(), walked.clone()));
    }
    out
}

pub fn safe_local_name(remote_name: &str) -> Result<&str, TransferError> {
    let unsafe_name = remote_name.is_empty()
        || remote_name == "."
        || remote_name == ".."
        || remote_name
            .chars()
            .any(|c| c.is_control() || matches!(c, '/' | '\\' | ':'));
    if unsafe_name {
        return Err(invalid(format!(
            "the remote name {remote_name:?} cannot be used as a local file name; pass saveTo with a full file path"
        )));
    }
    Ok(remote_name)
}

pub fn local_target(save_to: &Path, remote_path: &str) -> Result<PathBuf, TransferError> {
    if !save_to.is_absolute() {
        return Err(invalid("saveTo must be an absolute local path".into()));
    }
    if save_to.is_dir() {
        let name = file_name(remote_path).unwrap_or_default();
        return Ok(save_to.join(safe_local_name(name)?));
    }
    Ok(save_to.to_path_buf())
}

pub fn upload_target(remote: &str, local: &Path) -> Result<String, TransferError> {
    if remote.is_empty() || remote.ends_with('/') {
        let name = local
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| invalid("localPath has no file name".into()))?;
        return join(remote, name);
    }
    normalize(remote)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn refused(result: Result<String, TransferError>) -> bool {
        matches!(result, Err(TransferError::InvalidInput(_)))
    }

    #[test]
    fn normalizing_collapses_slashes_and_dots() {
        assert_eq!(normalize("/").unwrap(), "/");
        assert_eq!(normalize("").unwrap(), "/");
        assert_eq!(normalize("//switch//./nro/").unwrap(), "/switch/nro");
        assert_eq!(normalize("a/b").unwrap(), "/a/b");
        assert_eq!(
            normalize("/Meus Vídeos/a b.mp4").unwrap(),
            "/Meus Vídeos/a b.mp4"
        );
    }

    #[test]
    fn parent_references_are_refused_instead_of_resolved() {
        assert!(refused(normalize("/a/../etc/passwd")));
        assert!(refused(normalize("..")));
        assert!(refused(join("/home/me", "../../etc")));
        assert_eq!(normalize("/a/..b/c..").unwrap(), "/a/..b/c..");
    }

    #[test]
    fn line_breaks_and_control_bytes_cannot_smuggle_a_command() {
        assert!(refused(normalize("/a\r\nDELE /b")));
        assert!(refused(normalize("/a\nb")));
        assert!(refused(normalize("/a\0b")));
        assert!(refused(normalize("/a\u{7f}b")));
        assert!(refused(normalize(&"a/".repeat(3000))));
    }

    #[test]
    fn joining_keeps_absolute_input_and_appends_relative_input() {
        assert_eq!(
            join("/switch", "nro/app.nro").unwrap(),
            "/switch/nro/app.nro"
        );
        assert_eq!(join("/switch", "/atmosphere").unwrap(), "/atmosphere");
        assert_eq!(join("/", "a").unwrap(), "/a");
        assert_eq!(join("/switch/", "").unwrap(), "/switch");
    }

    #[test]
    fn parent_and_name_split_the_last_component() {
        assert_eq!(parent("/a/b/c.txt"), "/a/b");
        assert_eq!(parent("/c.txt"), "/");
        assert_eq!(parent("/"), "/");
        assert_eq!(file_name("/a/b/c.txt"), Some("c.txt"));
        assert_eq!(file_name("/"), None);
    }

    #[test]
    fn crumbs_walk_from_the_root_to_the_folder() {
        assert_eq!(
            crumbs("/switch/nro"),
            vec![
                ("/".to_string(), "/".to_string()),
                ("switch".to_string(), "/switch".to_string()),
                ("nro".to_string(), "/switch/nro".to_string()),
            ]
        );
        assert_eq!(crumbs("/"), vec![("/".to_string(), "/".to_string())]);
    }

    #[test]
    fn a_remote_name_cannot_choose_a_local_folder() {
        assert_eq!(safe_local_name("save.bin").unwrap(), "save.bin");
        for bad in ["", ".", "..", "a/b", "a\\b", "..\\x", "a\0b", "C:evil"] {
            assert!(
                matches!(safe_local_name(bad), Err(TransferError::InvalidInput(_))),
                "{bad:?}"
            );
        }
    }

    #[test]
    fn saving_into_a_folder_appends_the_remote_file_name() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(
            local_target(dir.path(), "/saves/zelda.bin").unwrap(),
            dir.path().join("zelda.bin")
        );
        let file = dir.path().join("renamed.bin");
        assert_eq!(local_target(&file, "/saves/zelda.bin").unwrap(), file);
        assert!(matches!(
            local_target(Path::new("relative.bin"), "/a"),
            Err(TransferError::InvalidInput(_))
        ));
    }

    #[test]
    fn uploading_to_a_folder_path_appends_the_local_file_name() {
        let local = Path::new("/tmp/app.nro");
        assert_eq!(upload_target("/switch/", local).unwrap(), "/switch/app.nro");
        assert_eq!(
            upload_target("/switch/other.nro", local).unwrap(),
            "/switch/other.nro"
        );
        assert_eq!(upload_target("/", local).unwrap(), "/app.nro");
        assert_eq!(upload_target("", local).unwrap(), "/app.nro");
    }
}

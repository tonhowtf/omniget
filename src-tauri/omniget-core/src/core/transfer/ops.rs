use serde::Serialize;
use std::path::{Path, PathBuf};

use super::listing::{self, Entry, Kind, Listing};
use super::{ftp, paths, sftp, Connection, Progress, Protocol, TransferError};

enum Session {
    Ftp(Box<ftp::Session>),
    Sftp(Box<sftp::Session>),
}

macro_rules! each {
    ($session:expr, $s:ident => $call:expr) => {
        match $session {
            Session::Ftp($s) => $call,
            Session::Sftp($s) => $call,
        }
    };
}

impl Session {
    async fn open(connection: &Connection) -> Result<Self, TransferError> {
        Ok(match connection.protocol {
            Protocol::Ftp | Protocol::Ftps => {
                Self::Ftp(Box::new(ftp::Session::open(connection).await?))
            }
            Protocol::Sftp => Self::Sftp(Box::new(sftp::Session::open(connection).await?)),
        })
    }

    async fn home(&mut self) -> Result<String, TransferError> {
        each!(self, s => s.home().await)
    }

    async fn read_dir(&mut self, path: &str) -> Result<(Vec<Entry>, usize), TransferError> {
        each!(self, s => s.read_dir(path).await)
    }

    async fn stat(&mut self, path: &str) -> Result<Option<Entry>, TransferError> {
        if path == "/" {
            return Ok(Some(Entry {
                name: "/".into(),
                kind: Kind::Dir,
                size: None,
                modified: None,
            }));
        }
        match self {
            Self::Sftp(s) => s.stat(path).await,
            Self::Ftp(s) => {
                let name = paths::file_name(path).unwrap_or_default();
                match s.read_dir(paths::parent(path)).await {
                    Ok((entries, _)) => Ok(entries.into_iter().find(|e| e.name == name)),
                    Err(TransferError::NotFound(_)) => Ok(None),
                    Err(e) => Err(e),
                }
            }
        }
    }

    async fn close(self) {
        each!(self, s => s.close().await)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Transferred {
    pub remote: String,
    pub local: PathBuf,
    pub bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Target {
    pub path: String,
    pub kind: Kind,
    pub size: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Managed {
    pub op: &'static str,
    pub path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub to: Option<String>,
    pub done: bool,
    pub dry_run: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target: Option<Target>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub would: Option<String>,
}

pub fn describe_delete(target: &Target) -> String {
    match (target.kind, target.size) {
        (Kind::Dir, _) => format!("remove the empty folder {}", target.path),
        (kind, Some(size)) => format!("delete the {} {} ({size} bytes)", kind.label(), target.path),
        (kind, None) => format!("delete the {} {}", kind.label(), target.path),
    }
}

pub fn same_size(path: &str, expected: Option<u64>, got: Option<u64>) -> Result<(), TransferError> {
    match (expected, got) {
        (Some(expected), Some(got)) if expected != got => Err(TransferError::Incomplete {
            path: path.to_string(),
            expected,
            got,
        }),
        _ => Ok(()),
    }
}

fn local_error(path: &Path, error: std::io::Error) -> TransferError {
    TransferError::Local {
        path: path.to_path_buf(),
        message: error.to_string(),
    }
}

async fn closing<T>(
    session: Session,
    result: Result<T, TransferError>,
) -> Result<T, TransferError> {
    session.close().await;
    result
}

async fn resolve(session: &mut Session, path: Option<&str>) -> Result<String, TransferError> {
    match path.filter(|path| !path.is_empty()) {
        Some(path) if path.starts_with('/') => paths::normalize(path),
        relative => {
            let home = paths::normalize(&session.home().await?)?;
            paths::join(&home, relative.unwrap_or_default())
        }
    }
}

pub async fn list(
    connection: &Connection,
    path: Option<&str>,
    limit: Option<usize>,
) -> Result<Listing, TransferError> {
    let mut session = Session::open(connection).await?;
    let result = async {
        let path = resolve(&mut session, path).await?;
        let (entries, skipped) = session.read_dir(&path).await?;
        Ok(listing::build(&path, entries, skipped, limit))
    }
    .await;
    closing(session, result).await
}

pub async fn get(
    connection: &Connection,
    remote: &str,
    save_to: &Path,
    overwrite: bool,
    total: &(dyn Fn(Option<u64>) + Send + Sync),
    progress: &Progress<'_>,
) -> Result<Transferred, TransferError> {
    if remote.is_empty() {
        return Err(TransferError::InvalidInput("path is required".into()));
    }
    let mut session = Session::open(connection).await?;
    let result = async {
        let remote = resolve(&mut session, Some(remote)).await?;
        let local = paths::local_target(save_to, &remote)?;
        if !overwrite && tokio::fs::try_exists(&local).await.unwrap_or(false) {
            return Err(TransferError::AlreadyExists(local.display().to_string()));
        }
        let entry = session
            .stat(&remote)
            .await?
            .ok_or_else(|| TransferError::NotFound(remote.clone()))?;
        if entry.kind == Kind::Dir {
            return Err(TransferError::NotAFile {
                path: remote,
                kind: "folder",
            });
        }
        total(entry.size);
        if let Some(parent) = local.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|e| local_error(parent, e))?;
        }
        let mut partial = local.clone().into_os_string();
        partial.push(".part");
        let partial = PathBuf::from(partial);
        let mut file = tokio::fs::File::create(&partial)
            .await
            .map_err(|e| local_error(&partial, e))?;
        let copied =
            each!(&mut session, s => s.download(&remote, &mut file, &local, progress).await);
        drop(file);
        let checked =
            copied.and_then(|bytes| same_size(&remote, entry.size, Some(bytes)).map(|()| bytes));
        let bytes = match checked {
            Ok(bytes) => bytes,
            Err(e) => {
                let _ = tokio::fs::remove_file(&partial).await;
                return Err(e);
            }
        };
        tokio::fs::rename(&partial, &local)
            .await
            .map_err(|e| local_error(&local, e))?;
        Ok(Transferred {
            remote,
            local,
            bytes,
        })
    }
    .await;
    closing(session, result).await
}

pub async fn put(
    connection: &Connection,
    local: &Path,
    remote: &str,
    overwrite: bool,
    total: &(dyn Fn(Option<u64>) + Send + Sync),
    progress: &Progress<'_>,
) -> Result<Transferred, TransferError> {
    if !local.is_absolute() {
        return Err(TransferError::InvalidInput(
            "localPath must be an absolute local path".into(),
        ));
    }
    let metadata = tokio::fs::metadata(local)
        .await
        .map_err(|e| local_error(local, e))?;
    if !metadata.is_file() {
        return Err(TransferError::InvalidInput(
            "localPath must be a file; folders are not uploaded".into(),
        ));
    }
    let mut session = Session::open(connection).await?;
    let result = async {
        let named = if remote.starts_with('/') {
            paths::upload_target(remote, local)?
        } else {
            let home = paths::normalize(&session.home().await?)?;
            let relative = if remote.is_empty() || remote.ends_with('/') {
                format!("{home}/{remote}/")
            } else {
                format!("{home}/{remote}")
            };
            paths::upload_target(&relative, local)?
        };
        match session.stat(&named).await? {
            Some(entry) if entry.kind == Kind::Dir => {
                return Err(TransferError::InvalidInput(format!(
                    "{named} is a folder; end path with / to upload into it"
                )))
            }
            Some(_) if !overwrite => return Err(TransferError::AlreadyExists(named)),
            _ => {}
        }
        total(Some(metadata.len()));
        let mut file = tokio::fs::File::open(local)
            .await
            .map_err(|e| local_error(local, e))?;
        let bytes = each!(&mut session, s => s.upload(&named, &mut file, local, progress).await)?;
        let stored = session.stat(&named).await?.and_then(|entry| entry.size);
        same_size(&named, Some(metadata.len()), stored)?;
        Ok(Transferred {
            remote: named,
            local: local.to_path_buf(),
            bytes,
        })
    }
    .await;
    closing(session, result).await
}

fn required(path: &str, name: &str) -> Result<String, TransferError> {
    if !path.starts_with('/') {
        return Err(TransferError::InvalidInput(format!(
            "{name} must be an absolute remote path starting with /"
        )));
    }
    let path = paths::normalize(path)?;
    if path == "/" {
        return Err(TransferError::InvalidInput(format!(
            "{name} cannot be the root folder"
        )));
    }
    Ok(path)
}

pub async fn mkdir(connection: &Connection, path: &str) -> Result<Managed, TransferError> {
    let path = required(path, "path")?;
    let mut session = Session::open(connection).await?;
    let result = async {
        if session.stat(&path).await?.is_some() {
            return Err(TransferError::AlreadyExists(path.clone()));
        }
        each!(&mut session, s => s.mkdir(&path).await)
    }
    .await;
    closing(session, result).await?;
    Ok(Managed {
        op: "mkdir",
        path,
        to: None,
        done: true,
        dry_run: false,
        target: None,
        would: None,
    })
}

pub async fn rename(
    connection: &Connection,
    path: &str,
    to: &str,
) -> Result<Managed, TransferError> {
    let path = required(path, "path")?;
    let to = if to.starts_with('/') {
        required(to, "to")?
    } else {
        required(&paths::join(paths::parent(&path), to)?, "to")?
    };
    if to == path {
        return Err(TransferError::InvalidInput("to is the same as path".into()));
    }
    let mut session = Session::open(connection).await?;
    let result = async {
        if session.stat(&path).await?.is_none() {
            return Err(TransferError::NotFound(path.clone()));
        }
        if session.stat(&to).await?.is_some() {
            return Err(TransferError::AlreadyExists(to.clone()));
        }
        each!(&mut session, s => s.rename(&path, &to).await)
    }
    .await;
    closing(session, result).await?;
    Ok(Managed {
        op: "rename",
        path,
        to: Some(to),
        done: true,
        dry_run: false,
        target: None,
        would: None,
    })
}

pub async fn delete(
    connection: &Connection,
    path: &str,
    confirm: bool,
) -> Result<Managed, TransferError> {
    let path = required(path, "path")?;
    let mut session = Session::open(connection).await?;
    let result = async {
        let entry = session
            .stat(&path)
            .await?
            .ok_or_else(|| TransferError::NotFound(path.clone()))?;
        if entry.kind == Kind::Dir && !session.read_dir(&path).await?.0.is_empty() {
            return Err(TransferError::NotEmpty(path.clone()));
        }
        let target = Target {
            path: path.clone(),
            kind: entry.kind,
            size: entry.size,
        };
        if confirm {
            if entry.kind == Kind::Dir {
                each!(&mut session, s => s.remove_dir(&path).await)?;
            } else {
                each!(&mut session, s => s.remove_file(&path).await)?;
            }
        }
        Ok(target)
    }
    .await;
    let target = closing(session, result).await?;
    Ok(Managed {
        op: "delete",
        path,
        to: None,
        done: confirm,
        dry_run: !confirm,
        would: (!confirm).then(|| describe_delete(&target)),
        target: Some(target),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_dry_run_says_exactly_what_would_go() {
        let file = Target {
            path: "/saves/zelda.bin".into(),
            kind: Kind::File,
            size: Some(2048),
        };
        assert_eq!(
            describe_delete(&file),
            "delete the file /saves/zelda.bin (2048 bytes)"
        );
        let folder = Target {
            path: "/saves/old".into(),
            kind: Kind::Dir,
            size: None,
        };
        assert_eq!(
            describe_delete(&folder),
            "remove the empty folder /saves/old"
        );
        let link = Target {
            path: "/current".into(),
            kind: Kind::Link,
            size: None,
        };
        assert_eq!(describe_delete(&link), "delete the link /current");
    }

    #[test]
    fn a_transfer_that_ends_with_another_size_is_reported_and_never_called_done() {
        assert_eq!(same_size("/a.bin", Some(10), Some(10)), Ok(()));
        assert_eq!(same_size("/a.bin", None, Some(10)), Ok(()));
        assert_eq!(same_size("/a.bin", Some(10), None), Ok(()));
        assert_eq!(
            same_size("/a.bin", Some(10), Some(7)),
            Err(TransferError::Incomplete {
                path: "/a.bin".into(),
                expected: 10,
                got: 7
            })
        );
        let error = same_size("/a.bin", Some(10), Some(7)).unwrap_err();
        assert_eq!(error.code(), "INCOMPLETE");
        assert!(error.retryable());
    }

    #[test]
    fn manage_paths_are_absolute_and_never_the_root() {
        assert_eq!(required("/a//b/", "path").unwrap(), "/a/b");
        for bad in ["", "a/b", "/", "//", "/a/../b"] {
            assert!(
                matches!(required(bad, "path"), Err(TransferError::InvalidInput(_))),
                "{bad:?}"
            );
        }
    }
}

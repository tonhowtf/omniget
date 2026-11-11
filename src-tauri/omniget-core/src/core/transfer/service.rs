use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

use super::profiles::{self, Draft, Profile};
use super::{known_hosts, Auth, Connection, TransferError};
use crate::core::secrets::SecretStore;

pub const SECRET_NAMESPACE: &str = "transfer";
const SERVERS_FILE: &str = "servers.json";
const KNOWN_HOSTS_FILE: &str = "known_hosts";

pub struct AppSecrets;

impl SecretStore for AppSecrets {
    fn get(&self, namespace: &str, account: &str) -> Result<Option<String>, String> {
        crate::core::secrets::get(namespace, account)
    }

    fn set(&self, namespace: &str, account: &str, value: &str) -> Result<(), String> {
        crate::core::secrets::set(namespace, account, value)
    }

    fn delete(&self, namespace: &str, account: &str) -> Result<(), String> {
        crate::core::secrets::delete(namespace, account)
    }
}

pub fn data_dir() -> Result<PathBuf, TransferError> {
    crate::core::paths::app_data_dir()
        .map(|dir| dir.join("transfer"))
        .ok_or_else(|| TransferError::Storage("the app data folder is unknown".into()))
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct TargetInput {
    pub server: Option<String>,
    pub protocol: Option<String>,
    pub host: Option<String>,
    pub port: Option<u32>,
    pub user: Option<String>,
    pub key_path: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerView {
    #[serde(flatten)]
    pub profile: Profile,
    pub has_secret: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Resolved {
    pub connection: Connection,
    pub start_path: Option<String>,
}

impl Resolved {
    pub fn path<'a>(&'a self, asked: Option<&'a str>) -> Option<&'a str> {
        asked
            .filter(|path| !path.is_empty())
            .or(self.start_path.as_deref())
    }
}

fn secret_failed(message: String) -> TransferError {
    TransferError::Storage(format!("the secret storage failed: {message}"))
}

fn given(value: &Option<String>) -> Option<&str> {
    value.as_deref().map(str::trim).filter(|v| !v.is_empty())
}

pub fn resolve(
    dir: &Path,
    secrets: &dyn SecretStore,
    input: &TargetInput,
) -> Result<Resolved, TransferError> {
    let known_hosts = dir.join(KNOWN_HOSTS_FILE);
    if let Some(name) = given(&input.server) {
        if given(&input.host).is_some() || given(&input.protocol).is_some() {
            return Err(TransferError::InvalidInput(
                "pass either server, or protocol and host, not both".into(),
            ));
        }
        let profile = profiles::find(&dir.join(SERVERS_FILE), name)?;
        let secret = secrets
            .get(SECRET_NAMESPACE, &profiles::secret_account(&profile.name))
            .map_err(secret_failed)?
            .filter(|secret| !secret.is_empty());
        let auth = match (&profile.key_path, secret) {
            (Some(path), passphrase) => Auth::Key {
                path: PathBuf::from(path),
                passphrase,
            },
            (None, Some(password)) => Auth::Password(password),
            (None, None) => Auth::Anonymous,
        };
        return Ok(Resolved {
            connection: Connection {
                protocol: profile.protocol,
                host: profile.host,
                port: profile.port,
                user: profile.user,
                auth,
                saved: true,
                known_hosts,
            },
            start_path: Some(profile.start_path).filter(|path| !path.is_empty()),
        });
    }
    let (Some(protocol), Some(host)) = (given(&input.protocol), given(&input.host)) else {
        return Err(TransferError::InvalidInput(
            "pass server (a saved name), or protocol and host".into(),
        ));
    };
    let protocol = profiles::protocol(protocol)?;
    let key = profiles::key_path(input.key_path.as_deref(), protocol)?;
    Ok(Resolved {
        connection: Connection {
            protocol,
            host: profiles::host(host)?,
            port: profiles::port(input.port, protocol)?,
            user: profiles::user(input.user.as_deref().unwrap_or_default(), protocol)?,
            auth: match key {
                Some(path) => Auth::Key {
                    path: PathBuf::from(path),
                    passphrase: None,
                },
                None => Auth::Anonymous,
            },
            saved: false,
            known_hosts,
        },
        start_path: None,
    })
}

fn view(profile: Profile, secrets: &dyn SecretStore) -> ServerView {
    let has_secret = secrets
        .get(SECRET_NAMESPACE, &profiles::secret_account(&profile.name))
        .ok()
        .flatten()
        .is_some_and(|secret| !secret.is_empty());
    ServerView {
        profile,
        has_secret,
    }
}

pub fn servers(dir: &Path, secrets: &dyn SecretStore) -> Result<Vec<ServerView>, TransferError> {
    Ok(profiles::load(&dir.join(SERVERS_FILE))?
        .into_iter()
        .map(|profile| view(profile, secrets))
        .collect())
}

pub fn save_server(
    dir: &Path,
    secrets: &dyn SecretStore,
    draft: &Draft,
    secret: Option<&str>,
) -> Result<ServerView, TransferError> {
    let profile = profiles::save(&dir.join(SERVERS_FILE), profiles::validate(draft)?)?;
    let account = profiles::secret_account(&profile.name);
    match secret {
        None => {}
        Some("") => secrets
            .delete(SECRET_NAMESPACE, &account)
            .map_err(secret_failed)?,
        Some(value) => secrets
            .set(SECRET_NAMESPACE, &account, value)
            .map_err(secret_failed)?,
    }
    Ok(view(profile, secrets))
}

pub fn remove_server(
    dir: &Path,
    secrets: &dyn SecretStore,
    name: &str,
) -> Result<Profile, TransferError> {
    let removed = profiles::remove(&dir.join(SERVERS_FILE), name)?;
    secrets
        .delete(SECRET_NAMESPACE, &profiles::secret_account(&removed.name))
        .map_err(secret_failed)?;
    Ok(removed)
}

pub fn forget_host(dir: &Path, host: &str, port: Option<u32>) -> Result<usize, TransferError> {
    let host = profiles::host(host)?;
    let port = port
        .and_then(|port| u16::try_from(port).ok())
        .filter(|port| *port != 0)
        .ok_or_else(|| TransferError::InvalidInput("port must be between 1 and 65535".into()))?;
    known_hosts::forget(&dir.join(KNOWN_HOSTS_FILE), &host, port)
}

pub fn url_of(input: &TargetInput) -> Option<String> {
    let host = profiles::host(given(&input.host)?).ok()?;
    let protocol = profiles::protocol(given(&input.protocol)?).ok()?;
    let port = profiles::port(input.port, protocol).ok()?;
    let host = if host.contains(':') {
        format!("[{host}]")
    } else {
        host
    };
    Some(format!("ftp://{host}:{port}"))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::core::transfer::Protocol;
    use std::collections::HashMap;
    use std::sync::Mutex;

    #[derive(Default)]
    pub(crate) struct MemorySecrets(pub Mutex<HashMap<String, String>>);

    impl SecretStore for MemorySecrets {
        fn get(&self, namespace: &str, account: &str) -> Result<Option<String>, String> {
            Ok(self
                .0
                .lock()
                .unwrap()
                .get(&format!("{namespace}/{account}"))
                .cloned())
        }

        fn set(&self, namespace: &str, account: &str, value: &str) -> Result<(), String> {
            self.0
                .lock()
                .unwrap()
                .insert(format!("{namespace}/{account}"), value.to_string());
            Ok(())
        }

        fn delete(&self, namespace: &str, account: &str) -> Result<(), String> {
            self.0
                .lock()
                .unwrap()
                .remove(&format!("{namespace}/{account}"));
            Ok(())
        }
    }

    fn draft(name: &str, protocol: &str, host: &str, user: &str) -> Draft {
        Draft {
            name: name.into(),
            protocol: protocol.into(),
            host: host.into(),
            user: user.into(),
            ..Draft::default()
        }
    }

    fn named(server: &str) -> TargetInput {
        TargetInput {
            server: Some(server.into()),
            ..TargetInput::default()
        }
    }

    #[test]
    fn the_password_lives_in_the_secret_store_and_never_in_the_file_or_the_view() {
        let dir = tempfile::tempdir().unwrap();
        let secrets = MemorySecrets::default();
        let saved = save_server(
            dir.path(),
            &secrets,
            &draft("NAS", "ftps", "nas.local", "me"),
            Some("hunter2"),
        )
        .unwrap();
        assert!(saved.has_secret);
        let file = std::fs::read_to_string(dir.path().join(SERVERS_FILE)).unwrap();
        assert!(!file.contains("hunter2"));
        let shown = serde_json::to_string(&servers(dir.path(), &secrets).unwrap()).unwrap();
        assert!(!shown.contains("hunter2"));
        assert!(shown.contains("\"hasSecret\":true"));
        assert!(shown.contains("\"name\":\"NAS\""));
        assert_eq!(
            secrets
                .get(SECRET_NAMESPACE, "server:nas")
                .unwrap()
                .as_deref(),
            Some("hunter2")
        );
    }

    #[test]
    fn a_saved_name_resolves_with_its_secret_and_start_path() {
        let dir = tempfile::tempdir().unwrap();
        let secrets = MemorySecrets::default();
        save_server(
            dir.path(),
            &secrets,
            &Draft {
                start_path: "/music".into(),
                ..draft("NAS", "ftps", "nas.local", "me")
            },
            Some("hunter2"),
        )
        .unwrap();
        let resolved = resolve(dir.path(), &secrets, &named("nas")).unwrap();
        assert_eq!(resolved.connection.auth, Auth::Password("hunter2".into()));
        assert!(resolved.connection.saved);
        assert_eq!(resolved.connection.port, 21);
        assert_eq!(
            resolved.connection.known_hosts,
            dir.path().join("known_hosts")
        );
        assert_eq!(resolved.path(None), Some("/music"));
        assert_eq!(resolved.path(Some("")), Some("/music"));
        assert_eq!(resolved.path(Some("/other")), Some("/other"));
    }

    #[test]
    fn saving_again_without_a_secret_keeps_it_and_an_empty_one_clears_it() {
        let dir = tempfile::tempdir().unwrap();
        let secrets = MemorySecrets::default();
        let nas = draft("NAS", "ftp", "nas.local", "me");
        save_server(dir.path(), &secrets, &nas, Some("hunter2")).unwrap();
        assert!(
            save_server(dir.path(), &secrets, &nas, None)
                .unwrap()
                .has_secret
        );
        assert!(
            !save_server(dir.path(), &secrets, &nas, Some(""))
                .unwrap()
                .has_secret
        );
        assert_eq!(
            resolve(dir.path(), &secrets, &named("NAS"))
                .unwrap()
                .connection
                .auth,
            Auth::Anonymous
        );
    }

    #[test]
    fn a_key_profile_uses_the_secret_as_its_passphrase() {
        let dir = tempfile::tempdir().unwrap();
        let secrets = MemorySecrets::default();
        let key = std::env::temp_dir().join("id_ed25519");
        save_server(
            dir.path(),
            &secrets,
            &Draft {
                key_path: Some(key.to_string_lossy().into_owned()),
                ..draft("box", "sftp", "box.local", "me")
            },
            Some("phrase"),
        )
        .unwrap();
        assert_eq!(
            resolve(dir.path(), &secrets, &named("box"))
                .unwrap()
                .connection
                .auth,
            Auth::Key {
                path: key,
                passphrase: Some("phrase".into())
            }
        );
    }

    #[test]
    fn inline_fields_make_an_anonymous_connection_without_touching_the_store() {
        let dir = tempfile::tempdir().unwrap();
        let secrets = MemorySecrets::default();
        let resolved = resolve(
            dir.path(),
            &secrets,
            &TargetInput {
                protocol: Some("ftp".into()),
                host: Some("192.168.0.42".into()),
                port: Some(5000),
                ..TargetInput::default()
            },
        )
        .unwrap();
        assert_eq!(resolved.connection.protocol, Protocol::Ftp);
        assert_eq!(resolved.connection.port, 5000);
        assert_eq!(resolved.connection.auth, Auth::Anonymous);
        assert!(!resolved.connection.saved);
        assert_eq!(resolved.path(None), None);
        assert!(!dir.path().join(SERVERS_FILE).exists());
    }

    #[test]
    fn a_target_is_a_saved_name_or_inline_fields_never_both_or_neither() {
        let dir = tempfile::tempdir().unwrap();
        let secrets = MemorySecrets::default();
        let both = TargetInput {
            host: Some("h".into()),
            ..named("nas")
        };
        assert!(matches!(
            resolve(dir.path(), &secrets, &both),
            Err(TransferError::InvalidInput(_))
        ));
        assert!(matches!(
            resolve(dir.path(), &secrets, &TargetInput::default()),
            Err(TransferError::InvalidInput(_))
        ));
        assert_eq!(
            resolve(dir.path(), &secrets, &named("ghost")),
            Err(TransferError::UnknownServer("ghost".into()))
        );
    }

    #[test]
    fn removing_a_server_also_removes_its_secret() {
        let dir = tempfile::tempdir().unwrap();
        let secrets = MemorySecrets::default();
        save_server(
            dir.path(),
            &secrets,
            &draft("NAS", "ftp", "nas.local", "me"),
            Some("hunter2"),
        )
        .unwrap();
        assert_eq!(
            remove_server(dir.path(), &secrets, "nas").unwrap().name,
            "NAS"
        );
        assert!(secrets.0.lock().unwrap().is_empty());
        assert_eq!(servers(dir.path(), &secrets).unwrap(), vec![]);
    }

    #[test]
    fn forgetting_a_host_needs_a_real_port() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(forget_host(dir.path(), "h", Some(22)).unwrap(), 0);
        assert!(matches!(
            forget_host(dir.path(), "h", None),
            Err(TransferError::InvalidInput(_))
        ));
    }

    #[test]
    fn only_inline_targets_expose_an_address_to_the_network_check() {
        let inline = TargetInput {
            protocol: Some("sftp".into()),
            host: Some("fe80::1".into()),
            ..TargetInput::default()
        };
        assert_eq!(url_of(&inline).as_deref(), Some("ftp://[fe80::1]:22"));
        assert_eq!(url_of(&named("nas")), None);
    }
}

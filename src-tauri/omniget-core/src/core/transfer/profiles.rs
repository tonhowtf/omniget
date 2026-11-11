use serde::{Deserialize, Serialize};
use std::path::Path;

use super::{paths, Protocol, TransferError};

pub const MAX_PROFILES: usize = 100;
const MAX_NAME_CHARS: usize = 64;
const MAX_HOST_CHARS: usize = 253;
const MAX_USER_CHARS: usize = 128;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Profile {
    pub name: String,
    pub protocol: Protocol,
    pub host: String,
    pub port: u16,
    pub user: String,
    pub start_path: String,
    pub key_path: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Draft {
    pub name: String,
    pub protocol: String,
    pub host: String,
    pub port: Option<u32>,
    pub user: String,
    pub start_path: String,
    pub key_path: Option<String>,
}

fn invalid(message: impl Into<String>) -> TransferError {
    TransferError::InvalidInput(message.into())
}

pub fn protocol(name: &str) -> Result<Protocol, TransferError> {
    match name.trim().to_ascii_lowercase().as_str() {
        "ftp" => Ok(Protocol::Ftp),
        "ftps" => Ok(Protocol::Ftps),
        "sftp" => Ok(Protocol::Sftp),
        _ => Err(invalid("protocol must be ftp, ftps or sftp")),
    }
}

pub fn host(input: &str) -> Result<String, TransferError> {
    let host = input.trim();
    let host = host
        .strip_prefix('[')
        .and_then(|rest| rest.strip_suffix(']'))
        .unwrap_or(host);
    let bad = host.is_empty()
        || host.chars().count() > MAX_HOST_CHARS
        || host
            .chars()
            .any(|c| c.is_control() || c.is_whitespace() || matches!(c, '/' | '@' | '[' | ']'));
    if bad {
        return Err(invalid(
            "host must be a bare name or address such as 192.168.0.42 or nas.local, without ftp://, user@ or a folder",
        ));
    }
    Ok(host.to_string())
}

pub fn port(input: Option<u32>, protocol: Protocol) -> Result<u16, TransferError> {
    match input {
        None => Ok(protocol.default_port()),
        Some(port) => u16::try_from(port)
            .ok()
            .filter(|port| *port != 0)
            .ok_or_else(|| invalid("port must be between 1 and 65535")),
    }
}

pub fn user(input: &str, protocol: Protocol) -> Result<String, TransferError> {
    let user = input.trim();
    if user.chars().count() > MAX_USER_CHARS || user.chars().any(char::is_control) {
        return Err(invalid(
            "user is too long or has a line break or control character",
        ));
    }
    if user.is_empty() && protocol == Protocol::Sftp {
        return Err(invalid("user is required for sftp"));
    }
    Ok(user.to_string())
}

pub fn key_path(input: Option<&str>, protocol: Protocol) -> Result<Option<String>, TransferError> {
    let Some(path) = input.map(str::trim).filter(|path| !path.is_empty()) else {
        return Ok(None);
    };
    if protocol != Protocol::Sftp {
        return Err(invalid("keyPath is only used with sftp"));
    }
    if !Path::new(path).is_absolute() {
        return Err(invalid(
            "keyPath must be an absolute path to a private key file",
        ));
    }
    Ok(Some(path.to_string()))
}

pub fn validate(draft: &Draft) -> Result<Profile, TransferError> {
    let name = draft.name.trim();
    if name.is_empty()
        || name.chars().count() > MAX_NAME_CHARS
        || name.chars().any(char::is_control)
    {
        return Err(invalid(format!(
            "name is required, at most {MAX_NAME_CHARS} characters, on one line"
        )));
    }
    let protocol = protocol(&draft.protocol)?;
    let start_path = if draft.start_path.trim().is_empty() {
        String::new()
    } else {
        paths::normalize(draft.start_path.trim())?
    };
    Ok(Profile {
        name: name.to_string(),
        protocol,
        host: host(&draft.host)?,
        port: port(draft.port, protocol)?,
        user: user(&draft.user, protocol)?,
        start_path,
        key_path: key_path(draft.key_path.as_deref(), protocol)?,
    })
}

pub fn secret_account(name: &str) -> String {
    format!("server:{}", name.to_lowercase())
}

fn same_name(profile: &Profile, name: &str) -> bool {
    profile.name.to_lowercase() == name.trim().to_lowercase()
}

pub fn load(file: &Path) -> Result<Vec<Profile>, TransferError> {
    let text = match std::fs::read_to_string(file) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(TransferError::Storage(e.to_string())),
    };
    serde_json::from_str(&text)
        .map_err(|e| TransferError::Storage(format!("{} cannot be read: {e}", file.display())))
}

fn store(file: &Path, profiles: &[Profile]) -> Result<(), TransferError> {
    let failed = |e: std::io::Error| TransferError::Storage(e.to_string());
    if let Some(parent) = file.parent() {
        std::fs::create_dir_all(parent).map_err(failed)?;
    }
    let text = serde_json::to_string_pretty(profiles)
        .map_err(|e| TransferError::Storage(e.to_string()))?;
    let partial = file.with_extension("json.part");
    std::fs::write(&partial, text).map_err(failed)?;
    std::fs::rename(&partial, file).map_err(failed)
}

pub fn find(file: &Path, name: &str) -> Result<Profile, TransferError> {
    load(file)?
        .into_iter()
        .find(|profile| same_name(profile, name))
        .ok_or_else(|| TransferError::UnknownServer(name.to_string()))
}

pub fn save(file: &Path, profile: Profile) -> Result<Profile, TransferError> {
    let mut profiles = load(file)?;
    profiles.retain(|saved| !same_name(saved, &profile.name));
    if profiles.len() >= MAX_PROFILES {
        return Err(invalid(format!(
            "{MAX_PROFILES} servers are already saved; remove one first"
        )));
    }
    profiles.push(profile.clone());
    profiles.sort_by_key(|saved| saved.name.to_lowercase());
    store(file, &profiles)?;
    Ok(profile)
}

pub fn remove(file: &Path, name: &str) -> Result<Profile, TransferError> {
    let mut profiles = load(file)?;
    let at = profiles
        .iter()
        .position(|profile| same_name(profile, name))
        .ok_or_else(|| TransferError::UnknownServer(name.to_string()))?;
    let removed = profiles.remove(at);
    store(file, &profiles)?;
    Ok(removed)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn draft(name: &str, protocol: &str, host: &str) -> Draft {
        Draft {
            name: name.into(),
            protocol: protocol.into(),
            host: host.into(),
            ..Draft::default()
        }
    }

    fn invalid<T: std::fmt::Debug>(result: Result<T, TransferError>, word: &str) {
        match result {
            Err(TransferError::InvalidInput(message)) => {
                assert!(message.contains(word), "{message}")
            }
            other => panic!("expected INVALID_INPUT about {word}, got {other:?}"),
        }
    }

    #[test]
    fn an_anonymous_console_server_needs_only_a_name_and_a_host() {
        let profile = validate(&Draft {
            port: Some(5000),
            ..draft("  Switch  ", "ftp", " 192.168.0.42 ")
        })
        .unwrap();
        assert_eq!(
            profile,
            Profile {
                name: "Switch".into(),
                protocol: Protocol::Ftp,
                host: "192.168.0.42".into(),
                port: 5000,
                user: String::new(),
                start_path: String::new(),
                key_path: None,
            }
        );
    }

    #[test]
    fn the_port_defaults_to_the_protocol_and_must_be_a_real_port() {
        assert_eq!(validate(&draft("a", "ftps", "h")).unwrap().port, 21);
        assert_eq!(
            validate(&Draft {
                user: "me".into(),
                ..draft("a", "sftp", "h")
            })
            .unwrap()
            .port,
            22
        );
        invalid(port(Some(0), Protocol::Ftp), "port");
        invalid(port(Some(70_000), Protocol::Ftp), "port");
    }

    #[test]
    fn the_protocol_is_a_closed_set() {
        assert_eq!(protocol("SFTP").unwrap(), Protocol::Sftp);
        invalid(protocol("scp"), "protocol");
        invalid(protocol(""), "protocol");
    }

    #[test]
    fn the_host_is_a_bare_name_or_address() {
        assert_eq!(host("NAS.local").unwrap(), "NAS.local");
        assert_eq!(host("::1").unwrap(), "::1");
        assert_eq!(host("[fe80::1]").unwrap(), "fe80::1");
        invalid(host(""), "host");
        invalid(host("ftp://nas.local"), "host");
        invalid(host("nas.local/folder"), "host");
        invalid(host("me@nas.local"), "host");
        invalid(host("nas local"), "host");
        invalid(host("nas\r\nQUIT"), "host");
    }

    #[test]
    fn sftp_needs_a_user_and_a_user_cannot_carry_a_line_break() {
        invalid(user("", Protocol::Sftp), "user");
        assert_eq!(user("", Protocol::Ftp).unwrap(), "");
        assert_eq!(user(" me ", Protocol::Sftp).unwrap(), "me");
        invalid(user("me\r\nDELE x", Protocol::Ftp), "user");
    }

    #[test]
    fn a_key_file_is_an_absolute_path_and_only_for_sftp() {
        let absolute = std::env::temp_dir().join("id_ed25519");
        let absolute = absolute.to_string_lossy();
        assert_eq!(
            key_path(Some(&absolute), Protocol::Sftp)
                .unwrap()
                .as_deref(),
            Some(&*absolute)
        );
        assert_eq!(key_path(Some("  "), Protocol::Sftp).unwrap(), None);
        assert_eq!(key_path(None, Protocol::Ftp).unwrap(), None);
        invalid(key_path(Some("id_ed25519"), Protocol::Sftp), "keyPath");
        invalid(key_path(Some(&absolute), Protocol::Ftp), "keyPath");
    }

    #[test]
    fn the_name_is_required_short_and_printable() {
        invalid(validate(&draft("  ", "ftp", "h")), "name");
        invalid(validate(&draft(&"n".repeat(65), "ftp", "h")), "name");
        invalid(validate(&draft("a\nb", "ftp", "h")), "name");
    }

    #[test]
    fn the_start_path_is_normalized_and_cannot_climb() {
        let profile = validate(&Draft {
            start_path: "switch//nro/".into(),
            ..draft("a", "ftp", "h")
        })
        .unwrap();
        assert_eq!(profile.start_path, "/switch/nro");
        invalid(
            validate(&Draft {
                start_path: "/a/../b".into(),
                ..draft("a", "ftp", "h")
            }),
            "..",
        );
    }

    #[test]
    fn saving_replaces_the_same_name_and_keeps_the_list_sorted() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("transfer/servers.json");
        assert_eq!(load(&file).unwrap(), vec![]);
        save(
            &file,
            validate(&draft("switch", "ftp", "10.0.0.2")).unwrap(),
        )
        .unwrap();
        save(&file, validate(&draft("Alpha", "ftp", "10.0.0.3")).unwrap()).unwrap();
        save(
            &file,
            validate(&draft("SWITCH", "ftp", "10.0.0.9")).unwrap(),
        )
        .unwrap();
        let saved = load(&file).unwrap();
        let seen: Vec<(&str, &str)> = saved
            .iter()
            .map(|p| (p.name.as_str(), p.host.as_str()))
            .collect();
        assert_eq!(seen, vec![("Alpha", "10.0.0.3"), ("SWITCH", "10.0.0.9")]);
        assert_eq!(find(&file, "switch").unwrap().host, "10.0.0.9");
    }

    #[test]
    fn a_missing_name_is_an_unknown_server() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("servers.json");
        assert_eq!(
            find(&file, "nope"),
            Err(TransferError::UnknownServer("nope".into()))
        );
        assert_eq!(
            remove(&file, "nope"),
            Err(TransferError::UnknownServer("nope".into()))
        );
    }

    #[test]
    fn removing_returns_the_profile_and_leaves_the_others() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("servers.json");
        save(&file, validate(&draft("a", "ftp", "h1")).unwrap()).unwrap();
        save(&file, validate(&draft("b", "ftp", "h2")).unwrap()).unwrap();
        assert_eq!(remove(&file, "A").unwrap().host, "h1");
        assert_eq!(load(&file).unwrap().len(), 1);
    }

    #[test]
    fn the_list_has_a_ceiling() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("servers.json");
        let many: Vec<Profile> = (0..MAX_PROFILES)
            .map(|n| validate(&draft(&format!("s{n}"), "ftp", "h")).unwrap())
            .collect();
        std::fs::write(&file, serde_json::to_string(&many).unwrap()).unwrap();
        invalid(
            save(&file, validate(&draft("one more", "ftp", "h")).unwrap()),
            "100",
        );
        save(&file, validate(&draft("s1", "ftp", "changed")).unwrap()).unwrap();
    }

    #[test]
    fn a_damaged_file_is_an_error_and_is_not_overwritten() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("servers.json");
        std::fs::write(&file, "{not json").unwrap();
        assert!(matches!(load(&file), Err(TransferError::Storage(_))));
        assert!(matches!(
            save(&file, validate(&draft("a", "ftp", "h")).unwrap()),
            Err(TransferError::Storage(_))
        ));
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "{not json");
    }

    #[test]
    fn the_secret_account_ignores_case_like_the_name_does() {
        assert_eq!(secret_account("Switch"), "server:switch");
        assert_eq!(secret_account("SWITCH"), secret_account("switch"));
    }
}

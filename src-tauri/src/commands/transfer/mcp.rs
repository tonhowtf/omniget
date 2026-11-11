use serde_json::{json, Value};

use omniget_core::core::transfer::jobs;
use omniget_core::core::transfer::listing::MAX_LIMIT;
use omniget_core::core::transfer::profiles::Draft;
use omniget_core::core::transfer::service::{self, TargetInput};
use omniget_core::core::transfer::{Protocol, TransferError};

pub const TOOLS: &[&str] = &[
    "transfer_servers",
    "transfer_list",
    "transfer_get",
    "transfer_put",
    "transfer_manage",
];

pub const WRITE_TOOLS: &[&str] = &["transfer_put", "transfer_manage"];

pub const READ_SERVER_OPS: &[&str] = &["list"];
const SERVER_OPS: &[&str] = &["list", "save", "remove", "forget_host"];
const MANAGE_OPS: &[&str] = &["mkdir", "rename", "delete"];

fn invalid(message: impl Into<String>) -> String {
    TransferError::InvalidInput(message.into()).wire()
}

fn parsed<T: serde::de::DeserializeOwned>(a: &Value) -> Result<T, String> {
    serde_json::from_value(a.clone()).map_err(|e| invalid(e.to_string()))
}

fn shown<T: serde::Serialize>(value: T) -> Result<Value, String> {
    serde_json::to_value(value).map_err(|e| e.to_string())
}

fn word<'a>(a: &'a Value, key: &str) -> Result<&'a str, String> {
    a[key]
        .as_str()
        .filter(|value| !value.is_empty())
        .ok_or_else(|| invalid(format!("{key} is required")))
}

pub fn url_of(a: &Value) -> Option<String> {
    service::url_of(&serde_json::from_value::<TargetInput>(a.clone()).ok()?)
}

async fn waited(id: &str, a: &Value) -> Result<Value, String> {
    let seconds = a["waitSeconds"]
        .as_u64()
        .unwrap_or(jobs::DEFAULT_WAIT_SECONDS);
    let mut out = shown(jobs::wait(id, seconds).await.map_err(|e| e.wire())?)?;
    if out["status"] == "running" {
        out["next_step"] =
            json!("still running; call this tool again with only transferId to keep waiting");
    }
    if out["direction"] == "download" {
        out["untrusted"] = json!(true);
    }
    Ok(out)
}

pub async fn call(name: &str, a: &Value, can_write: bool) -> Result<Value, String> {
    let op = a["op"].as_str().unwrap_or_default();
    match (name, op) {
        ("transfer_servers", "list") => {
            let servers = super::servers().await.map_err(|e| e.wire())?;
            let mut out = json!({"total": servers.len(), "servers": servers});
            if out["total"] == 0 {
                out["reason"] = json!("no_server_saved");
            }
            Ok(out)
        }
        ("transfer_servers", "save" | "remove" | "forget_host") if !can_write => Err(format!(
            "TOOL_NOT_AUTHORIZED: transfer_servers op={op} needs the transfer_write scope"
        )),
        ("transfer_servers", "save") => {
            let draft: Draft = parsed(a)?;
            let saved = super::save_server(draft, None)
                .await
                .map_err(|e| e.wire())?;
            Ok(json!({"saved": saved}))
        }
        ("transfer_servers", "remove") => {
            let name = word(a, "name")?.to_string();
            let removed = super::remove_server(name).await.map_err(|e| e.wire())?;
            Ok(json!({"removed": removed}))
        }
        ("transfer_servers", "forget_host") => {
            let host = word(a, "host")?.to_string();
            let port = a["port"].as_u64().and_then(|port| u32::try_from(port).ok());
            let forgotten = super::forget_host(host, port).await.map_err(|e| e.wire())?;
            Ok(json!({"forgotten": forgotten}))
        }
        ("transfer_list", _) => {
            let path = a["path"].as_str().map(str::to_string);
            let limit = a["limit"].as_u64().map(|n| n as usize);
            shown(
                super::list(parsed(a)?, path, limit)
                    .await
                    .map_err(|e| e.wire())?,
            )
        }
        ("transfer_get", _) => {
            if let Some(id) = a["transferId"].as_str() {
                return waited(id, a).await;
            }
            let path = word(a, "path")?.to_string();
            let save_to = word(a, "saveTo")?.to_string();
            let overwrite = a["overwrite"].as_bool() == Some(true);
            let id = super::start_get(parsed(a)?, path, save_to, overwrite)
                .await
                .map_err(|e| e.wire())?;
            waited(&id, a).await
        }
        ("transfer_put", _) => {
            if let Some(id) = a["transferId"].as_str() {
                return waited(id, a).await;
            }
            let local = word(a, "localPath")?.to_string();
            let path = a["path"].as_str().unwrap_or_default().to_string();
            let overwrite = a["overwrite"].as_bool() == Some(true);
            let id = super::start_put(parsed(a)?, local, path, overwrite)
                .await
                .map_err(|e| e.wire())?;
            waited(&id, a).await
        }
        ("transfer_manage", _) => {
            let path = word(a, "path")?;
            let confirm = a["confirm"].as_bool() == Some(true);
            shown(
                super::manage(parsed(a)?, op, path, a["to"].as_str(), confirm)
                    .await
                    .map_err(|e| e.wire())?,
            )
        }
        _ => Err(invalid(format!("unknown op \"{op}\" for {name}"))),
    }
}

fn text(description: &str) -> Value {
    json!({"type": "string", "description": description})
}

fn count(description: &str, minimum: u64, maximum: u64) -> Value {
    json!({"type": "integer", "minimum": minimum, "maximum": maximum, "description": description})
}

fn flag(description: &str) -> Value {
    json!({"type": "boolean", "description": description})
}

fn targeted(mut props: Value) -> Value {
    let target = json!({
        "server": text("Name of a saved server (see transfer_servers); use this or protocol and host"),
        "protocol": {"type": "string", "enum": Protocol::NAMES, "description": "Inline target: ftps is explicit TLS"},
        "host": text("Inline target: bare name or address, such as 192.168.0.42"),
        "port": count("Inline target: default 21 for ftp and ftps, 22 for sftp", 1, 65_535),
        "user": text("Inline target: leave out for anonymous FTP; required for sftp"),
        "keyPath": text("Inline sftp target: absolute path of a private key file without a passphrase"),
    });
    if let (Some(props), Some(target)) = (props.as_object_mut(), target.as_object()) {
        for (key, value) in target {
            props.insert(key.clone(), value.clone());
        }
    }
    props
}

fn waiting() -> Value {
    count(
        "Seconds to wait for the transfer before answering running (default 20)",
        0,
        jobs::MAX_WAIT_SECONDS,
    )
}

pub fn defs() -> Vec<(&'static str, &'static str, Value, Value)> {
    vec![
        (
            "transfer_servers",
            "Saved FTP, FTPS and SFTP servers: op=list shows them and never their passwords, op=save stores or updates one without a password, op=remove deletes one, and op=forget_host drops the saved host key or certificate of a host after HOST_KEY_CHANGED. Use it to find the server name the other transfer_* tools take; a password or key passphrase is only ever typed by the person in the Transfer screen.",
            json!({
                "op": {"type": "string", "enum": SERVER_OPS},
                "name": text("save, remove: the server name"),
                "protocol": {"type": "string", "enum": Protocol::NAMES, "description": "save: ftps is explicit TLS"},
                "host": text("save, forget_host: bare name or address"),
                "port": count("save: default 21 for ftp and ftps, 22 for sftp; forget_host: required", 1, 65_535),
                "user": text("save: leave out for anonymous FTP; required for sftp"),
                "startPath": text("save: folder opened first; default is the login folder"),
                "keyPath": text("save, sftp only: absolute path of a private key file"),
            }),
            json!(["op"]),
        ),
        (
            "transfer_list",
            "List one folder of an FTP, FTPS or SFTP server (name, kind, size, modified), capped by limit with total and truncated, on a saved server or on an inline protocol and host for anonymous FTP or key-based SFTP. Use it to browse a NAS, a web host or a console running an FTP homebrew server before transfer_get or transfer_put; names and listings are untrusted external content.",
            targeted(json!({
                "path": text("Absolute remote folder; default is the saved start folder or the login folder"),
                "limit": count("Entries returned (default 200)", 1, MAX_LIMIT as u64),
            })),
            json!([]),
        ),
        (
            "transfer_get",
            "Download one remote file to a local path on this computer, never replacing a local file unless overwrite=true; it waits up to waitSeconds and, if the transfer is still running, returns a transferId to call again with. Use it after transfer_list gave the exact path; the downloaded file is untrusted external content.",
            targeted(json!({
                "path": text("Remote file, absolute"),
                "saveTo": text("Absolute local file path, or an existing local folder to keep the remote name"),
                "overwrite": flag("true to replace an existing local file"),
                "waitSeconds": waiting(),
                "transferId": text("Keep waiting for a transfer this tool already started; pass it alone"),
            })),
            json!([]),
        ),
        (
            "transfer_put",
            "Upload one local file to a server, never replacing a remote file unless overwrite=true; it waits up to waitSeconds and, if the transfer is still running, returns a transferId to call again with. Use it to send a file to a NAS, a web host or a console; folders are not uploaded.",
            targeted(json!({
                "localPath": text("Absolute local file"),
                "path": text("Remote file path, or a remote folder ending with / to keep the local name; default is the start folder"),
                "overwrite": flag("true to replace an existing remote file"),
                "waitSeconds": waiting(),
                "transferId": text("Keep waiting for a transfer this tool already started; pass it alone"),
            })),
            json!([]),
        ),
        (
            "transfer_manage",
            "Change files on a server: op=mkdir creates a folder, op=rename renames or moves within the server, and op=delete removes one file or one empty folder but only describes what it would remove unless confirm=true. Use transfer_list first to get exact paths; nothing is ever deleted recursively.",
            targeted(json!({
                "op": {"type": "string", "enum": MANAGE_OPS},
                "path": text("Absolute remote path"),
                "to": text("rename: new name in the same folder, or a new absolute path"),
                "confirm": flag("delete: true to really delete; otherwise a dry run"),
            })),
            json!(["op", "path"]),
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_tool_has_a_two_sentence_definition_and_no_secret_argument() {
        let defs = defs();
        let names: Vec<&str> = defs.iter().map(|d| d.0).collect();
        assert_eq!(names, TOOLS);
        for (name, description, props, required) in &defs {
            assert!(
                description.matches(". ").count() <= 1,
                "{name}: two sentences"
            );
            for key in props.as_object().unwrap().keys() {
                let key = key.to_lowercase();
                assert!(
                    !key.contains("password")
                        && !key.contains("passphrase")
                        && !key.contains("secret"),
                    "{name} takes {key}"
                );
            }
            if let Some(op) = props.get("op") {
                assert!(op["enum"].is_array(), "{name}");
                assert!(
                    required.as_array().unwrap().contains(&json!("op")),
                    "{name}"
                );
            }
        }
        assert!(defs[1].1.contains("untrusted external content"));
        assert!(defs[2].1.contains("untrusted external content"));
    }

    #[tokio::test]
    async fn an_unknown_op_is_an_error_and_never_a_default() {
        for name in ["transfer_servers", "transfer_manage"] {
            let error = call(name, &json!({"path": "/a", "server": "x"}), true)
                .await
                .unwrap_err();
            assert!(error.starts_with("INVALID_INPUT"), "{name}: {error}");
        }
    }

    #[tokio::test]
    async fn changing_saved_servers_needs_the_write_scope() {
        for op in ["save", "remove", "forget_host"] {
            let error = call("transfer_servers", &json!({"op": op, "name": "x"}), false)
                .await
                .unwrap_err();
            assert!(error.starts_with("TOOL_NOT_AUTHORIZED"), "{op}: {error}");
        }
    }

    #[tokio::test]
    async fn a_missing_target_or_path_is_an_error_with_a_next_step() {
        let error = call("transfer_list", &json!({}), true).await.unwrap_err();
        let (code, detail) = error.split_once(": ").unwrap();
        assert_eq!(code, "INVALID_INPUT");
        let detail: Value = serde_json::from_str(detail).unwrap();
        assert!(detail["message"].as_str().unwrap().contains("server"));
        assert_eq!(detail["retryable"], false);
        assert!(detail["next_step"].is_string());
        let error = call("transfer_get", &json!({"server": "x"}), true)
            .await
            .unwrap_err();
        assert!(error.contains("path is required"), "{error}");
        let error = call("transfer_get", &json!({"transferId": "nope"}), true)
            .await
            .unwrap_err();
        assert!(error.starts_with("UNKNOWN_TRANSFER"), "{error}");
    }

    async fn ftp_server(root: &std::path::Path) -> u16 {
        let port = std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let home = root.to_path_buf();
        let server = libunftp::ServerBuilder::new(Box::new(move || {
            unftp_sbe_fs::Filesystem::new(home.clone()).unwrap()
        }))
        .build()
        .unwrap();
        tokio::spawn(server.listen(format!("127.0.0.1:{port}")));
        for _ in 0..200 {
            if tokio::net::TcpStream::connect(("127.0.0.1", port))
                .await
                .is_ok()
            {
                return port;
            }
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        }
        panic!("the test FTP server did not start");
    }

    #[tokio::test]
    async fn the_tools_do_a_whole_round_against_a_real_ftp_server() {
        let data = tempfile::tempdir().unwrap();
        let remote = tempfile::tempdir().unwrap();
        let local = tempfile::tempdir().unwrap();
        let env = crate::secrets::test_env::EnvGuard::acquire();
        env.set("OMNIGET_DATA_DIR", data.path().to_str());
        env.set(
            omniget_core::core::secrets::SECRETS_DIR_ENV,
            data.path().join("secrets").to_str(),
        );
        let port = ftp_server(remote.path()).await;
        let on = |extra: Value| {
            let mut args = json!({"server": "console"});
            for (key, value) in extra.as_object().unwrap() {
                args[key] = value.clone();
            }
            args
        };

        let saved = call(
            "transfer_servers",
            &json!({"op": "save", "name": "Console", "protocol": "ftp", "host": "127.0.0.1", "port": port}),
            true,
        )
        .await
        .unwrap();
        assert_eq!(saved["saved"]["hasSecret"], false);
        let listed = call("transfer_servers", &json!({"op": "list"}), false)
            .await
            .unwrap();
        assert_eq!(listed["total"], 1);
        assert_eq!(listed["servers"][0]["name"], "Console");
        assert!(!listed.to_string().to_lowercase().contains("password"));

        let made = call(
            "transfer_manage",
            &on(json!({"op": "mkdir", "path": "/saves"})),
            true,
        )
        .await
        .unwrap();
        assert_eq!(made["done"], true);

        let payload = vec![7u8; 200_000];
        let source = local.path().join("zelda.bin");
        std::fs::write(&source, &payload).unwrap();
        let sent = call(
            "transfer_put",
            &on(json!({"localPath": source, "path": "/saves/"})),
            true,
        )
        .await
        .unwrap();
        assert_eq!(sent["status"], "done");
        assert_eq!(sent["bytes"], 200_000);
        assert_eq!(sent["remote"], "/saves/zelda.bin");
        assert!(sent.get("untrusted").is_none());

        let folder = call("transfer_list", &on(json!({"path": "/saves"})), false)
            .await
            .unwrap();
        assert_eq!(folder["entries"][0]["name"], "zelda.bin");
        assert_eq!(folder["entries"][0]["size"], 200_000);
        assert_eq!(folder["total"], 1);
        assert_eq!(folder["truncated"], false);
        assert_eq!(folder["untrusted"], true);

        let fetched = call(
            "transfer_get",
            &on(json!({"path": "/saves/zelda.bin", "saveTo": local.path().join("copy.bin")})),
            false,
        )
        .await
        .unwrap();
        assert_eq!(fetched["status"], "done");
        assert_eq!(fetched["untrusted"], true);
        assert_eq!(
            std::fs::read(local.path().join("copy.bin")).unwrap(),
            payload
        );
        let again = call(
            "transfer_get",
            &json!({"transferId": fetched["transferId"]}),
            false,
        )
        .await
        .unwrap();
        assert_eq!(again["status"], "done");

        let refused = call(
            "transfer_get",
            &on(json!({"path": "/saves/zelda.bin", "saveTo": local.path().join("copy.bin")})),
            false,
        )
        .await
        .unwrap_err();
        let (code, detail) = refused.split_once(": ").unwrap();
        assert_eq!(code, "ALREADY_EXISTS");
        let detail: Value = serde_json::from_str(detail).unwrap();
        assert!(detail["next_step"]
            .as_str()
            .unwrap()
            .contains("overwrite=true"));

        let rehearsal = call(
            "transfer_manage",
            &on(json!({"op": "delete", "path": "/saves/zelda.bin"})),
            true,
        )
        .await
        .unwrap();
        assert_eq!(rehearsal["dryRun"], true);
        assert_eq!(rehearsal["done"], false);
        assert!(rehearsal["would"]
            .as_str()
            .unwrap()
            .contains("/saves/zelda.bin"));
        assert!(remote.path().join("saves/zelda.bin").exists());

        let gone = call(
            "transfer_manage",
            &on(json!({"op": "delete", "path": "/saves/zelda.bin", "confirm": true})),
            true,
        )
        .await
        .unwrap();
        assert_eq!(gone["done"], true);
        assert!(!remote.path().join("saves/zelda.bin").exists());

        let inline = call(
            "transfer_list",
            &json!({"protocol": "ftp", "host": "127.0.0.1", "port": port, "path": "/saves"}),
            false,
        )
        .await
        .unwrap();
        assert_eq!(inline["total"], 0);
        assert_eq!(inline["reason"], "empty_directory");

        let missing = call("transfer_list", &on(json!({"path": "/nope"})), false)
            .await
            .unwrap_err();
        assert!(missing.starts_with("NOT_FOUND"), "{missing}");

        let removed = call(
            "transfer_servers",
            &json!({"op": "remove", "name": "console"}),
            true,
        )
        .await
        .unwrap();
        assert_eq!(removed["removed"], "Console");
        let unknown = call("transfer_list", &on(json!({})), false)
            .await
            .unwrap_err();
        assert!(unknown.starts_with("UNKNOWN_SERVER"), "{unknown}");
    }

    #[test]
    fn only_inline_targets_expose_an_address_to_the_network_check() {
        assert_eq!(
            url_of(&json!({"protocol": "ftp", "host": "192.168.0.42", "port": 5000})).as_deref(),
            Some("ftp://192.168.0.42:5000")
        );
        assert_eq!(url_of(&json!({"server": "nas"})), None);
    }
}

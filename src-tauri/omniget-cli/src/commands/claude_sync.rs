use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{anyhow, bail, Context};
use omniget_core::core::llm::cli_runtime::accounts::{
    default_profile_dir, AccountStore, CliAccount, CliKind, SHARE_LINK,
};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};

const HOOK_MARK: &str = "# omniget: conta principal do Claude Code";

fn home() -> anyhow::Result<PathBuf> {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .ok_or_else(|| anyhow!("HOME não definido"))
}

pub fn main_script_path() -> Option<PathBuf> {
    omniget_core::core::paths::app_data_dir().map(|d| d.join("llm").join("claude-main.sh"))
}

fn sh_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

pub fn main_script(account: &CliAccount) -> String {
    if account.config_dir.as_os_str().is_empty() {
        format!("{HOOK_MARK}: {}\nunset CLAUDE_CONFIG_DIR\n", account.id)
    } else {
        format!(
            "{HOOK_MARK}: {}\nexport CLAUDE_CONFIG_DIR={}\n",
            account.id,
            sh_quote(&account.config_dir.display().to_string())
        )
    }
}

pub fn hook(script: &Path) -> String {
    format!(
        "\n{HOOK_MARK} (troque com: omniget claude --default <conta>)\n{}\n",
        hook_line(script)
    )
}

fn hook_line(script: &Path) -> String {
    format!(
        "claude() {{ ( command -v omniget >/dev/null 2>&1 && omniget claude --sync >/dev/null 2>&1; f={}; [ -z \"${{CLAUDE_CONFIG_DIR+x}}\" ] && [ -f \"$f\" ] && . \"$f\"; command claude \"$@\" ) }}",
        sh_quote(&script.display().to_string())
    )
}

pub fn write_main(account: &CliAccount) -> anyhow::Result<Vec<PathBuf>> {
    let script =
        main_script_path().ok_or_else(|| anyhow!("pasta de dados do OmniGet não encontrada"))?;
    if let Some(parent) = script.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&script, main_script(account))
        .with_context(|| format!("não consegui escrever {}", script.display()))?;
    let home = home()?;
    let mut installed = Vec::new();
    for (rc, always) in [(".zshrc", true), (".bashrc", false)] {
        let rc = home.join(rc);
        if !always && !rc.exists() {
            continue;
        }
        let text = std::fs::read_to_string(&rc).unwrap_or_default();
        if text.contains(HOOK_MARK) {
            let line = hook_line(&script);
            let mut after_mark = false;
            let mut changed = false;
            let lines: Vec<String> = text
                .lines()
                .map(|l| {
                    let out = if after_mark && l.starts_with("claude() {") && l != line {
                        changed = true;
                        line.clone()
                    } else {
                        l.to_string()
                    };
                    after_mark = l.starts_with(HOOK_MARK);
                    out
                })
                .collect();
            if changed {
                std::fs::write(&rc, lines.join("\n") + "\n")?;
                installed.push(rc);
            }
            continue;
        }
        let mut f = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&rc)
            .with_context(|| format!("não consegui abrir {}", rc.display()))?;
        std::io::Write::write_all(&mut f, hook(&script).as_bytes())?;
        installed.push(rc);
    }
    Ok(installed)
}

fn claude_json_of(account: &CliAccount, home: &Path) -> PathBuf {
    if account.config_dir.as_os_str().is_empty() {
        home.join(".claude.json")
    } else {
        account.config_dir.join(".claude.json")
    }
}

fn read_json(path: &Path) -> Option<Map<String, Value>> {
    let text = std::fs::read_to_string(path).ok()?;
    match serde_json::from_str(&text).ok()? {
        Value::Object(m) => Some(m),
        _ => None,
    }
}

fn write_json(path: &Path, map: &Map<String, Value>) -> anyhow::Result<()> {
    let text = serde_json::to_string_pretty(map)?;
    let tmp = path.with_extension("json.omniget-tmp");
    std::fs::write(&tmp, text)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600));
    }
    std::fs::rename(&tmp, path)?;
    Ok(())
}

pub fn merge3(
    base: Option<&Map<String, Value>>,
    sides: &[Option<Map<String, Value>>],
    main: Option<usize>,
) -> Map<String, Value> {
    let mut names: Vec<String> = base
        .map(|b| b.keys().cloned().collect())
        .unwrap_or_default();
    for side in sides.iter().flatten() {
        for k in side.keys() {
            if !names.contains(k) {
                names.push(k.clone());
            }
        }
    }
    let mut out = Map::new();
    for name in names {
        let was = base.and_then(|b| b.get(&name));
        let changed: Vec<(usize, Option<&Value>)> = sides
            .iter()
            .enumerate()
            .filter_map(|(i, s)| s.as_ref().map(|s| (i, s.get(&name))))
            .filter(|(_, v)| *v != was)
            .collect();
        let now = if base.is_none() {
            main.and_then(|m| sides[m].as_ref().and_then(|s| s.get(&name)))
                .or_else(|| changed.iter().find_map(|(_, v)| *v))
        } else if changed.is_empty() {
            was
        } else {
            changed
                .iter()
                .find(|(i, _)| Some(*i) == main)
                .unwrap_or(&changed[0])
                .1
        };
        if let Some(v) = now {
            out.insert(name, v.clone());
        }
    }
    out
}

fn sub_map(m: &Map<String, Value>, key: &str) -> Map<String, Value> {
    m.get(key)
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default()
}

fn base_path() -> Option<PathBuf> {
    omniget_core::core::paths::app_data_dir().map(|d| d.join("llm").join("claude-sync.json"))
}

fn keychain_service(config_dir: &Path) -> String {
    if config_dir.as_os_str().is_empty() {
        return "Claude Code-credentials".to_string();
    }
    let digest = Sha256::digest(config_dir.display().to_string().as_bytes());
    format!("Claude Code-credentials-{}", &hex::encode(digest)[..8])
}

#[cfg(target_os = "macos")]
fn read_creds(config_dir: &Path) -> Option<String> {
    let user = std::env::var("USER").unwrap_or_default();
    let out = Command::new("security")
        .args([
            "find-generic-password",
            "-a",
            &user,
            "-s",
            &keychain_service(config_dir),
            "-w",
        ])
        .output()
        .ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).trim_end().to_string())
}

#[cfg(target_os = "macos")]
fn put_keychain(service: &str, secret: &str) -> anyhow::Result<()> {
    let user = std::env::var("USER").unwrap_or_default();
    let hexed = hex::encode(secret.as_bytes());
    let line = format!("add-generic-password -U -a \"{user}\" -s \"{service}\" -X \"{hexed}\"\n");
    // WHY: `security -i` splits input lines past ~4 KB and saves the item truncated.
    let ok = if line.len() < 4000 {
        let mut child = Command::new("security")
            .arg("-i")
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()?;
        std::io::Write::write_all(child.stdin.as_mut().unwrap(), line.as_bytes())?;
        drop(child.stdin.take());
        child.wait()?.success()
    } else {
        Command::new("security")
            .args([
                "add-generic-password",
                "-U",
                "-a",
                &user,
                "-s",
                service,
                "-X",
                &hexed,
            ])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()?
            .success()
    };
    if !ok {
        bail!("o Keychain recusou a gravação");
    }
    Ok(())
}

#[cfg(target_os = "macos")]
fn write_creds(config_dir: &Path, secret: &str) -> anyhow::Result<()> {
    let service = keychain_service(config_dir);
    let before = read_creds(config_dir);
    let written = put_keychain(&service, secret);
    if written.is_ok() && read_creds(config_dir).as_deref() == Some(secret) {
        return Ok(());
    }
    if let Some(before) = before {
        let _ = put_keychain(&service, &before);
    }
    bail!("não consegui gravar o login no Keychain ({service}); o valor anterior foi mantido")
}

#[cfg(not(target_os = "macos"))]
fn creds_file(config_dir: &Path) -> Option<PathBuf> {
    if config_dir.as_os_str().is_empty() {
        home()
            .ok()
            .map(|h| h.join(".claude").join(".credentials.json"))
    } else {
        Some(config_dir.join(".credentials.json"))
    }
}

#[cfg(not(target_os = "macos"))]
fn read_creds(config_dir: &Path) -> Option<String> {
    std::fs::read_to_string(creds_file(config_dir)?).ok()
}

#[cfg(not(target_os = "macos"))]
fn write_creds(config_dir: &Path, secret: &str) -> anyhow::Result<()> {
    let path = creds_file(config_dir).ok_or_else(|| anyhow!("HOME não definido"))?;
    std::fs::write(&path, secret)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
    }
    Ok(())
}

fn copy_keychain(from: &Path, to: &Path) -> anyhow::Result<()> {
    let secret =
        read_creds(from).ok_or_else(|| anyhow!("login de {} não encontrado", from.display()))?;
    write_creds(to, &secret)
}

fn read_creds_map(config_dir: &Path) -> Option<Map<String, Value>> {
    read_creds(config_dir)
        .and_then(|t| serde_json::from_str::<Value>(&t).ok())
        .and_then(|v| v.as_object().cloned())
}

pub fn plan_logins(
    base: Option<&[String]>,
    sides: &[Option<Vec<String>>],
    gone_servers: &[String],
) -> (Vec<String>, Vec<String>) {
    let mut keep: Vec<String> = Vec::new();
    let mut drop: Vec<String> = Vec::new();
    for side in sides.iter().flatten() {
        for k in side {
            if !keep.contains(k) && !drop.contains(k) {
                keep.push(k.clone());
            }
        }
    }
    if let Some(base) = base {
        for k in base {
            let lost = sides.iter().flatten().any(|s| !s.contains(k));
            if lost && !drop.contains(k) {
                drop.push(k.clone());
            }
        }
    }
    for k in &keep {
        let server = k.split('|').next().unwrap_or_default();
        if gone_servers.iter().any(|g| g == server) && !drop.contains(k) {
            drop.push(k.clone());
        }
    }
    keep.retain(|k| !drop.contains(k));
    (keep, drop)
}

fn sync_mcp_logins(
    accounts: &[CliAccount],
    voters: &[bool],
    main: Option<usize>,
    base: Option<&[String]>,
    gone_servers: &[String],
) -> anyhow::Result<Vec<String>> {
    let creds: Vec<Option<Map<String, Value>>> = accounts
        .iter()
        .map(|a| read_creds_map(&a.config_dir))
        .collect();
    let keys: Vec<Option<Vec<String>>> = creds
        .iter()
        .zip(voters)
        .map(|(c, v)| {
            c.as_ref()
                .filter(|_| *v)
                .map(|c| sub_map(c, "mcpOAuth").keys().cloned().collect())
        })
        .collect();
    let (keep, drop) = plan_logins(base, &keys, gone_servers);
    let source = |k: &String| {
        main.into_iter().chain(0..creds.len()).find_map(|i| {
            creds[i]
                .as_ref()
                .and_then(|c| sub_map(c, "mcpOAuth").get(k).cloned())
        })
    };
    for a in accounts {
        let Some(mut c) = read_creds_map(&a.config_dir) else {
            continue;
        };
        let mut own = sub_map(&c, "mcpOAuth");
        let removed: Vec<String> = drop
            .iter()
            .filter(|k| own.contains_key(*k))
            .cloned()
            .collect();
        let added: Vec<String> = keep
            .iter()
            .filter(|k| !own.contains_key(*k))
            .cloned()
            .collect();
        if removed.is_empty() && added.is_empty() {
            continue;
        }
        for k in &removed {
            own.remove(k);
        }
        for k in &added {
            if let Some(v) = source(k) {
                own.insert(k.clone(), v);
            }
        }
        c.insert("mcpOAuth".into(), Value::Object(own));
        write_creds(&a.config_dir, &serde_json::to_string(&c)?)?;
        if !added.is_empty() {
            println!("{}: logins de MCP copiados ({})", a.label, added.join(", "));
        }
        if !removed.is_empty() {
            println!(
                "{}: logins de MCP removidos ({})",
                a.label,
                removed.join(", ")
            );
        }
    }
    Ok(keep)
}

fn adopt(store: &AccountStore, account: &CliAccount) -> anyhow::Result<CliAccount> {
    let dest = default_profile_dir(&account.id)
        .ok_or_else(|| anyhow!("pasta de perfis não encontrada"))?;
    if dest.exists() {
        bail!("{} já existe; não movi {}", dest.display(), account.id);
    }
    std::fs::create_dir_all(dest.parent().unwrap())?;
    let status = Command::new("cp")
        .arg("-Rp")
        .arg(&account.config_dir)
        .arg(&dest)
        .status()?;
    if !status.success() {
        bail!("cp falhou ao copiar {}", account.config_dir.display());
    }
    if let Err(e) = copy_keychain(&account.config_dir, &dest) {
        let _ = std::fs::remove_dir_all(&dest);
        return Err(e);
    }
    let mut moved = account.clone();
    moved.config_dir = dest;
    store.update(moved.clone()).map_err(|e| anyhow!("{e:?}"))?;
    Ok(moved)
}

fn link_shared(source: &Path, dest: &Path) -> anyhow::Result<Vec<String>> {
    std::fs::create_dir_all(dest)?;
    let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S");
    let mut linked = Vec::new();
    for name in SHARE_LINK {
        let from = source.join(name);
        if !from.exists() {
            continue;
        }
        let to = dest.join(name);
        if let Ok(meta) = to.symlink_metadata() {
            if meta.file_type().is_symlink()
                && std::fs::read_link(&to).ok().as_deref() == Some(&from)
            {
                continue;
            }
            let backup = dest.join(format!("{name}.antes-do-sync-{stamp}"));
            std::fs::rename(&to, &backup)?;
        }
        #[cfg(unix)]
        std::os::unix::fs::symlink(&from, &to)?;
        linked.push((*name).to_string());
    }
    Ok(linked)
}

pub fn sync(only: Option<&CliAccount>, main: Option<&CliAccount>) -> anyhow::Result<()> {
    let store = AccountStore::default_store()
        .ok_or_else(|| anyhow!("pasta de dados do OmniGet não encontrada"))?;
    let home = home()?;
    let source = home.join(".claude");
    let mut accounts: Vec<CliAccount> = store
        .list()
        .iter()
        .filter(|a| a.cli == CliKind::Claude && !a.disabled)
        .cloned()
        .collect();

    for a in accounts.iter_mut() {
        if super::claude::in_temp_dir(&a.config_dir) && a.config_dir.exists() {
            match adopt(&store, a) {
                Ok(moved) => {
                    println!(
                        "{}: login saiu da pasta temporária para {}",
                        a.label,
                        moved.config_dir.display()
                    );
                    *a = moved;
                }
                Err(e) => println!("{}: continua na pasta temporária ({e})", a.label),
            }
        }
    }

    for a in &accounts {
        if a.config_dir.as_os_str().is_empty() || a.config_dir == source {
            continue;
        }
        if only.is_some_and(|o| o.id != a.id) {
            continue;
        }
        let linked = link_shared(&source, &a.config_dir)?;
        if linked.is_empty() {
            println!("{}: arquivos já compartilhados", a.label);
        } else {
            println!(
                "{}: agora usa os de ~/.claude → {}",
                a.label,
                linked.join(", ")
            );
        }
    }

    let base_file =
        base_path().ok_or_else(|| anyhow!("pasta de dados do OmniGet não encontrada"))?;
    let base = read_json(&base_file);
    let known: Vec<String> = base
        .as_ref()
        .and_then(|b| b.get("accounts"))
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default();
    let voters: Vec<bool> = accounts
        .iter()
        .map(|a| base.is_none() || known.contains(&a.id))
        .collect();
    let paths: Vec<PathBuf> = accounts.iter().map(|a| claude_json_of(a, &home)).collect();
    let maps: Vec<_> = paths.iter().map(|p| read_json(p)).collect();
    let sides: Vec<Option<Map<String, Value>>> = maps
        .iter()
        .zip(&voters)
        .map(|(m, v)| m.as_ref().filter(|_| *v).map(|m| sub_map(m, "mcpServers")))
        .collect();
    let main_ix = main.and_then(|m| accounts.iter().position(|a| a.id == m.id));
    let base_servers = base.as_ref().map(|b| sub_map(b, "mcpServers"));
    let merged = merge3(base_servers.as_ref(), &sides, main_ix);
    let gone: Vec<String> = sides
        .iter()
        .flatten()
        .flat_map(|s| s.keys())
        .chain(base_servers.iter().flat_map(|b| b.keys()))
        .filter(|k| !merged.contains_key(*k))
        .cloned()
        .collect();
    for ((a, path), map) in accounts.iter().zip(&paths).zip(maps) {
        let mut map = map.unwrap_or_default();
        let before = sub_map(&map, "mcpServers");
        if before == merged {
            continue;
        }
        if !path.parent().is_some_and(|p| p.exists()) {
            continue;
        }
        let added: Vec<&str> = merged
            .keys()
            .filter(|k| !before.contains_key(*k))
            .map(String::as_str)
            .collect();
        let removed: Vec<&str> = before
            .keys()
            .filter(|k| !merged.contains_key(*k))
            .map(String::as_str)
            .collect();
        map.insert("mcpServers".into(), Value::Object(merged.clone()));
        write_json(path, &map)?;
        let mut what = Vec::new();
        if !added.is_empty() {
            what.push(format!("+ {}", added.join(", ")));
        }
        if !removed.is_empty() {
            what.push(format!("− {}", removed.join(", ")));
        }
        if what.is_empty() {
            what.push("configuração atualizada".into());
        }
        println!("{}: MCPs {}", a.label, what.join(" "));
    }
    let base_logins: Option<Vec<String>> = base.as_ref().map(|b| {
        b.get("mcpOAuth")
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(|v| v.as_str().map(str::to_owned))
                    .collect()
            })
            .unwrap_or_default()
    });
    let logins = sync_mcp_logins(&accounts, &voters, main_ix, base_logins.as_deref(), &gone)?;
    let mut snapshot = Map::new();
    snapshot.insert(
        "accounts".into(),
        Value::from(accounts.iter().map(|a| a.id.clone()).collect::<Vec<_>>()),
    );
    snapshot.insert("mcpServers".into(), Value::Object(merged.clone()));
    snapshot.insert("mcpOAuth".into(), Value::from(logins));
    write_json(&base_file, &snapshot)?;
    let names: Vec<&str> = merged.keys().map(String::as_str).collect();
    println!("MCPs em todas as contas: {}", names.join(", "));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn acc(id: &str, dir: &str) -> CliAccount {
        CliAccount {
            id: id.into(),
            cli: CliKind::Claude,
            config_dir: PathBuf::from(dir),
            label: id.into(),
            disabled: false,
            sandbox: Default::default(),
        }
    }

    #[test]
    fn keychain_service_matches_claude_code() {
        assert_eq!(
            keychain_service(Path::new(
                "/var/folders/bq/7dtfgtc12hg7yhdykjdh_23r0000gn/T/omniget-acc-1"
            )),
            "Claude Code-credentials-417fbab5"
        );
    }

    #[test]
    fn main_script_exports_or_unsets() {
        assert!(main_script(&acc("meu", "")).contains("unset CLAUDE_CONFIG_DIR"));
        assert!(main_script(&acc("b", "/a b/it's"))
            .contains("export CLAUDE_CONFIG_DIR='/a b/it'\\''s'"));
    }

    fn obj(v: Value) -> Option<Map<String, Value>> {
        v.as_object().cloned()
    }

    #[test]
    fn first_sync_is_a_union_where_the_main_account_wins() {
        let a = obj(json!({"x": 1, "y": 1}));
        let b = obj(json!({"y": 2, "z": 2}));
        let out = merge3(None, &[a, b, None], Some(1));
        assert_eq!(Value::Object(out), json!({"x": 1, "y": 2, "z": 2}));
    }

    #[test]
    fn a_server_removed_in_one_account_leaves_every_account() {
        let base = obj(json!({"x": 1, "y": 1})).unwrap();
        let out = merge3(
            Some(&base),
            &[obj(json!({"x": 1})), obj(json!({"x": 1, "y": 1}))],
            Some(1),
        );
        assert_eq!(Value::Object(out), json!({"x": 1}));
    }

    #[test]
    fn additions_and_edits_spread_and_the_main_wins_a_clash() {
        let base = obj(json!({"x": 1})).unwrap();
        let out = merge3(
            Some(&base),
            &[obj(json!({"x": 2, "n": 1})), obj(json!({"x": 3}))],
            Some(1),
        );
        assert_eq!(Value::Object(out), json!({"x": 3, "n": 1}));
    }

    #[test]
    fn an_account_that_does_not_vote_cannot_delete() {
        let base = obj(json!({"x": 1})).unwrap();
        let out = merge3(Some(&base), &[obj(json!({"x": 1})), None], None);
        assert_eq!(Value::Object(out), json!({"x": 1}));
    }

    #[test]
    fn logins_follow_removals_of_keys_and_of_servers() {
        let s = |v: &[&str]| Some(v.iter().map(|x| x.to_string()).collect::<Vec<_>>());
        let base: Vec<String> = vec!["a|1".into(), "b|1".into(), "c|1".into()];
        let (keep, drop) = plan_logins(
            Some(&base),
            &[s(&["a|1", "b|1", "c|1"]), s(&["a|1", "c|1", "d|1"])],
            &["c".to_string()],
        );
        assert_eq!(keep, vec!["a|1".to_string(), "d|1".to_string()]);
        assert_eq!(drop, vec!["b|1".to_string(), "c|1".to_string()]);
    }

    #[cfg(target_os = "macos")]
    #[test]
    #[ignore]
    fn keychain_round_trips_a_long_item() {
        let dir = Path::new("/tmp/omniget-claude-sync-roundtrip");
        for len in [100, 6000] {
            let secret = format!("{{\"x\":\"{}\"}}", "a".repeat(len));
            write_creds(dir, &secret).unwrap();
            assert_eq!(read_creds(dir).as_deref(), Some(secret.as_str()));
        }
        let _ = Command::new("security")
            .args(["delete-generic-password", "-s", &keychain_service(dir)])
            .output();
    }
}

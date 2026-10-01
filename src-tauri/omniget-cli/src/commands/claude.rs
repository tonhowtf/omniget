//! `omniget claude [account] [-- args]`: opens Claude Code in the terminal on
//! one of the accounts registered in the desktop app (/llm → Accounts).
//!
//! Same model as the app's "Enter" button (`login_plan`): the account is only
//! its config dir, passed as `CLAUDE_CONFIG_DIR`; the CLI keeps its own login
//! there and this command never reads a credential. API keys in the current
//! shell are removed (`SCRUB_ENV`) so they never decide which account bills.
//!
//! Who an account is (e-mail, plan) comes from `claude auth status --json`,
//! which Claude Code answers from its own login without printing any token.

use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{anyhow, bail, Context};
use omniget_core::core::llm::cli_runtime::accounts::{
    default_profile_dir, primary, set_primary, valid_id, AccountStore, CliAccount, CliKind,
    SandboxMode, SCRUB_ENV,
};
use serde_json::Value;

/// File with the id picked last, so a bare `omniget claude` repeats it.
fn last_path() -> Option<PathBuf> {
    omniget_core::core::paths::app_data_dir().map(|d| d.join("llm").join("cli-last-account"))
}

pub(crate) fn claude_accounts() -> Vec<CliAccount> {
    AccountStore::default_store()
        .map(|s| {
            s.list()
                .iter()
                .filter(|a| a.cli == CliKind::Claude && !a.disabled)
                .cloned()
                .collect()
        })
        .unwrap_or_default()
}

/// The main Claude account: the first one in the app's order (/llm → AI
/// connections, "Use now"), set from here with `--default`.
fn main_account() -> Option<CliAccount> {
    AccountStore::default_store().and_then(|s| primary(&s, CliKind::Claude))
}

/// What `omniget claude` was asked to do besides opening an account.
#[derive(Debug, Clone, Copy, Default)]
pub struct Flags {
    pub list: bool,
    pub add: bool,
    pub default: bool,
    pub pick: bool,
    pub remove: bool,
    pub safe: bool,
    pub no_splash: bool,
}

/// What `claude auth status` says about an account. Never holds a secret.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Identity {
    pub logged_in: bool,
    pub email: Option<String>,
    pub plan: Option<String>,
}

pub fn parse_identity(json: &str) -> Option<Identity> {
    let v: Value = serde_json::from_str(json).ok()?;
    let s = |k: &str| v.get(k).and_then(Value::as_str).map(str::to_owned);
    Some(Identity {
        logged_in: v.get("loggedIn").and_then(Value::as_bool).unwrap_or(false),
        email: s("email"),
        plan: s("subscriptionType"),
    })
}

/// The `claude` process for an account: config dir set (or the terminal's
/// default profile), API keys removed.
fn claude_command(account: Option<&CliAccount>) -> Command {
    let mut cmd = Command::new(CliKind::Claude.bin());
    for key in SCRUB_ENV {
        cmd.env_remove(key);
    }
    match account {
        Some(a) if !a.config_dir.as_os_str().is_empty() => {
            cmd.env(CliKind::Claude.config_env(), &a.config_dir);
        }
        _ => {
            cmd.env_remove(CliKind::Claude.config_env());
        }
    }
    cmd
}

fn identity(account: &CliAccount) -> Option<Identity> {
    let out = claude_command(Some(account))
        .args(["auth", "status", "--json"])
        .stdin(std::process::Stdio::null())
        .output()
        .ok()?;
    parse_identity(&String::from_utf8_lossy(&out.stdout))
}

/// Identities of several accounts, asked in parallel (one `claude` each).
pub(crate) fn identities(accounts: &[CliAccount]) -> Vec<Option<Identity>> {
    std::thread::scope(|s| {
        let handles: Vec<_> = accounts
            .iter()
            .map(|a| s.spawn(move || identity(a)))
            .collect();
        handles
            .into_iter()
            .map(|h| h.join().ok().flatten())
            .collect()
    })
}

/// "2.1.283" from `claude --version` ("2.1.283 (Claude Code)").
fn claude_version() -> Option<String> {
    let out = Command::new(CliKind::Claude.bin())
        .arg("--version")
        .stdin(std::process::Stdio::null())
        .output()
        .ok()?;
    parse_version(&String::from_utf8_lossy(&out.stdout))
}

pub fn parse_version(stdout: &str) -> Option<String> {
    let first = stdout.split_whitespace().next()?;
    first
        .chars()
        .next()
        .filter(|c| c.is_ascii_digit())
        .map(|_| first.to_string())
}

/// A config dir under the system temp folder is wiped by macOS after a few
/// days; history and settings of that account would vanish.
pub fn in_temp_dir(dir: &Path) -> bool {
    let s = dir.to_string_lossy();
    s.starts_with("/var/folders/")
        || s.starts_with("/private/var/folders/")
        || s.starts_with("/tmp/")
        || s.starts_with("/private/tmp/")
}

/// Matches an id, a label or the e-mail of the login, ignoring case; a unique
/// prefix also works.
pub fn find<'a>(
    accounts: &'a [CliAccount],
    emails: &[Option<String>],
    query: &str,
) -> Option<&'a CliAccount> {
    let q = query.trim().to_lowercase();
    let email = |i: usize| {
        emails
            .get(i)
            .cloned()
            .flatten()
            .unwrap_or_default()
            .to_lowercase()
    };
    let exact = accounts.iter().enumerate().find(|(i, a)| {
        a.id.to_lowercase() == q || a.label.to_lowercase() == q || (!q.is_empty() && email(*i) == q)
    });
    if let Some((_, a)) = exact {
        return Some(a);
    }
    let mut prefixed = accounts.iter().enumerate().filter(|(i, a)| {
        a.id.to_lowercase().starts_with(&q)
            || a.label.to_lowercase().starts_with(&q)
            || (!q.is_empty() && email(*i).starts_with(&q))
    });
    match (prefixed.next(), prefixed.next()) {
        (Some((_, one)), None) => Some(one),
        _ => None,
    }
}

fn describe(a: &CliAccount, who: Option<&Identity>) -> String {
    let mut s = format!("{} ({})", a.label, a.id);
    match who {
        Some(i) if i.logged_in => {
            let email = i.email.as_deref().unwrap_or("?");
            match i.plan.as_deref() {
                Some(p) => s.push_str(&format!(" — {email} · {p}")),
                None => s.push_str(&format!(" — {email}")),
            }
        }
        Some(_) => s.push_str(" — sem login"),
        None => {}
    }
    if a.config_dir.as_os_str().is_empty() {
        s.push_str(" [perfil padrão do terminal]");
    } else if in_temp_dir(&a.config_dir) {
        s.push_str(" [pasta temporária: pode sumir; recrie com --add --default]");
    }
    s
}

fn pick(accounts: &[CliAccount], who: &[Option<Identity>]) -> anyhow::Result<CliAccount> {
    // Enter keeps the main account; without one, the account picked last.
    let preferred = main_account().map(|a| a.id).or_else(|| {
        last_path()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .map(|s| s.trim().to_string())
    });
    let default = accounts
        .iter()
        .position(|a| Some(&a.id) == preferred.as_ref())
        .unwrap_or(0);
    let mut err = std::io::stderr();
    for (i, a) in accounts.iter().enumerate() {
        let mark = if i == default { "*" } else { " " };
        writeln!(err, "{mark} {}) {}", i + 1, describe(a, who[i].as_ref()))?;
    }
    write!(err, "Conta [{}]: ", default + 1)?;
    err.flush()?;
    let mut line = String::new();
    std::io::stdin().lock().read_line(&mut line)?;
    let line = line.trim();
    if line.is_empty() {
        return Ok(accounts[default].clone());
    }
    if let Ok(n) = line.parse::<usize>() {
        if (1..=accounts.len()).contains(&n) {
            return Ok(accounts[n - 1].clone());
        }
    }
    let emails: Vec<_> = who
        .iter()
        .map(|w| w.as_ref().and_then(|i| i.email.clone()))
        .collect();
    find(accounts, &emails, line)
        .cloned()
        .ok_or_else(|| anyhow!("nenhuma conta corresponde a {line:?}"))
}

/// Arguments for the `claude` process: the skip flag first (unless `safe`),
/// then whatever came after `--`.
pub fn claude_args(safe: bool, extra: &[String]) -> Vec<String> {
    let mut args = Vec::new();
    if !safe && !extra.iter().any(|a| a == "--dangerously-skip-permissions") {
        args.push("--dangerously-skip-permissions".to_string());
    }
    args.extend(extra.iter().cloned());
    args
}

/// An account id from a name: lowercase, `-` for anything else, never empty.
pub fn slug(name: &str) -> String {
    let mut out = String::new();
    for c in name.trim().to_lowercase().chars() {
        let c = if c.is_ascii_lowercase() || c.is_ascii_digit() {
            c
        } else {
            '-'
        };
        if !(c == '-' && (out.is_empty() || out.ends_with('-'))) {
            out.push(c);
        }
    }
    let out: String = out.trim_end_matches('-').chars().take(48).collect();
    if out.is_empty() {
        "claude".to_string()
    } else {
        out
    }
}

/// `--add [name]`: new account in the app's own profiles dir, then Claude
/// Code's login in it, then the account is named after who logged in.
fn add(name: Option<String>) -> anyhow::Result<Option<CliAccount>> {
    let store = AccountStore::default_store()
        .ok_or_else(|| anyhow!("pasta de dados do OmniGet não encontrada"))?;
    let taken: Vec<String> = store.list().iter().map(|a| a.id.clone()).collect();
    let base = slug(name.as_deref().unwrap_or("claude"));
    let mut id = base.clone();
    let mut n = 2;
    while taken.contains(&id) || !valid_id(&id) {
        id = format!("{base}-{n}");
        n += 1;
    }
    let dir = default_profile_dir(&id).ok_or_else(|| anyhow!("pasta de perfis não encontrada"))?;
    let account = store
        .create(CliAccount {
            id: id.clone(),
            cli: CliKind::Claude,
            config_dir: dir,
            label: name.clone().unwrap_or_else(|| id.clone()),
            disabled: false,
            sandbox: SandboxMode::Write,
        })
        .map_err(|e| anyhow!("{e:?}"))?;
    eprintln!(
        "omniget: conta {id} criada; abrindo o login do Claude Code (escolha a conta no navegador)"
    );
    let status = claude_command(Some(&account))
        .args(["auth", "login"])
        .status()
        .context("não consegui abrir o `claude`; ele está no PATH?")?;
    let who = identity(&account);
    match who.as_ref().filter(|i| i.logged_in) {
        Some(i) => {
            let email = i.email.clone().unwrap_or_default();
            let others: Vec<CliAccount> = claude_accounts()
                .into_iter()
                .filter(|a| a.id != id)
                .collect();
            let same: Vec<String> = others
                .iter()
                .zip(identities(&others))
                .filter(|(_, w)| {
                    w.as_ref().and_then(|w| w.email.clone()).as_deref() == Some(email.as_str())
                })
                .map(|(a, _)| a.label.clone())
                .collect();
            if name.is_none() && !email.is_empty() {
                let mut renamed = account.clone();
                renamed.label = email.clone();
                let _ = store.update(renamed);
            }
            println!(
                "Pronto: {email} · {}",
                i.plan.as_deref().unwrap_or("plano desconhecido")
            );
            if !same.is_empty() {
                println!(
                    "Aviso: esse login já está cadastrado como {}",
                    same.join(", ")
                );
            }
            println!(
                "Use: omniget claude {}",
                if name.is_some() { &id } else { &email }
            );
            Ok(Some(account))
        }
        None => {
            println!(
                "O login não foi concluído (saída {:?}). A conta {id} ficou criada; rode `omniget claude {id}` e use /login.",
                status.code()
            );
            Ok(None)
        }
    }
}

/// An account by id, label, e-mail or unique prefix. E-mails are only asked
/// for when the name did not match, so `omniget claude max` stays instant.
fn resolve(accounts: &[CliAccount], query: &str) -> anyhow::Result<CliAccount> {
    if let Some(a) = find(accounts, &[], query) {
        return Ok(a.clone());
    }
    let emails: Vec<_> = identities(accounts)
        .into_iter()
        .map(|w| w.and_then(|i| i.email))
        .collect();
    find(accounts, &emails, query)
        .cloned()
        .ok_or_else(|| anyhow!("conta {query:?} não encontrada; veja `omniget claude --list`"))
}

/// `--default`: the account goes to the head of the app's order, so the app
/// and a bare `omniget claude` both start on it.
fn make_main(account: &CliAccount) -> anyhow::Result<()> {
    let store = AccountStore::default_store()
        .ok_or_else(|| anyhow!("pasta de dados do OmniGet não encontrada"))?;
    set_primary(&store, &account.id).map_err(|e| anyhow!("{e:?}"))?;
    let who = identity(account);
    println!(
        "Conta principal: {}",
        describe(account, who.as_ref().filter(|i| i.logged_in))
    );
    println!("`omniget claude` abre nela sem perguntar, e o app tenta ela primeiro. Outra conta: omniget claude <conta>, ou --pick para escolher.");
    Ok(())
}

/// `--remove`: forgets the account, like the app's remove button. The login
/// folder stays, so nothing is logged out by accident.
fn remove(account: &CliAccount) -> anyhow::Result<()> {
    let store = AccountStore::default_store()
        .ok_or_else(|| anyhow!("pasta de dados do OmniGet não encontrada"))?;
    store.remove(&account.id).map_err(|e| anyhow!("{e:?}"))?;
    println!("Removida: {} ({})", account.label, account.id);
    if !account.config_dir.as_os_str().is_empty() {
        println!(
            "O login continua em {}; apague a pasta se não for usar mais.",
            account.config_dir.display()
        );
    }
    if let Some(next) = main_account() {
        println!("Conta principal agora: {} ({})", next.label, next.id);
    }
    Ok(())
}

pub fn execute(account: Option<String>, flags: Flags, extra: Vec<String>) -> anyhow::Result<()> {
    if flags.add {
        let added = add(account)?;
        if let (true, Some(a)) = (flags.default, added) {
            make_main(&a)?;
        }
        return Ok(());
    }
    let accounts = claude_accounts();
    if flags.list {
        if accounts.is_empty() {
            println!("Nenhuma conta Claude cadastrada. Crie uma com `omniget claude --add` ou no app em /llm → Contas.");
        }
        let main = main_account().map(|a| a.id);
        for (a, who) in accounts.iter().zip(identities(&accounts)) {
            let mark = if Some(&a.id) == main.as_ref() {
                "★"
            } else {
                " "
            };
            println!("{mark} {}", describe(a, who.as_ref()));
        }
        if main.is_some() {
            println!("★ = conta principal (omniget claude --default <conta> troca)");
        }
        return Ok(());
    }
    if flags.default || flags.remove {
        if accounts.is_empty() {
            bail!("nenhuma conta Claude cadastrada; crie uma com `omniget claude --add`");
        }
        let target = match &account {
            Some(q) => resolve(&accounts, q)?,
            None if flags.remove => {
                bail!("diga qual conta remover: omniget claude --remove <conta>")
            }
            None => pick(&accounts, &identities(&accounts))?,
        };
        return if flags.remove {
            remove(&target)
        } else {
            make_main(&target)
        };
    }
    let chosen: Option<CliAccount> = match (&account, accounts.len()) {
        (Some(q), _) => Some(resolve(&accounts, q)?),
        (None, 0) => None,
        (None, 1) => Some(accounts[0].clone()),
        (None, _) => match main_account().filter(|_| !flags.pick) {
            Some(main) => Some(main),
            None => Some(pick(&accounts, &identities(&accounts))?),
        },
    };

    if let Some(a) = &chosen {
        if !a.config_dir.as_os_str().is_empty() {
            std::fs::create_dir_all(&a.config_dir)
                .with_context(|| format!("não consegui criar {}", a.config_dir.display()))?;
        }
    }
    let mut cmd = claude_command(chosen.as_ref());
    cmd.args(claude_args(flags.safe, &extra));
    if let Some(a) = &chosen {
        if let Some(p) = last_path() {
            let _ = std::fs::write(p, &a.id);
        }
    }
    let splash = !flags.no_splash && std::env::var_os("OMNIGET_NO_SPLASH").is_none();
    if splash {
        // Claude Code's version and who is logged in, asked in parallel so the
        // splash costs one `claude` start, not two.
        let (version, who) = std::thread::scope(|s| {
            let v = s.spawn(claude_version);
            let w = s.spawn(|| chosen.as_ref().and_then(identity));
            (v.join().ok().flatten(), w.join().ok().flatten())
        });
        let account = chosen
            .as_ref()
            .map(|a| a.label.clone())
            .unwrap_or_else(|| "login padrão do terminal".to_string());
        let who = who.filter(|i| i.logged_in).map(|i| {
            format!(
                "{} · {}",
                i.email.unwrap_or_default(),
                i.plan.unwrap_or_else(|| "?".into())
            )
        });
        super::splash::print(version.as_deref(), &account, who.as_deref());
    } else if let Some(a) = &chosen {
        eprintln!("omniget: Claude Code na conta {}", describe(a, None));
    } else {
        eprintln!("omniget: nenhuma conta cadastrada no app; usando o login padrão do terminal");
    }

    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        let error = cmd.exec();
        bail!("não consegui abrir o `claude` ({error}); ele está no PATH?");
    }
    #[cfg(not(unix))]
    {
        let status = cmd
            .status()
            .context("não consegui abrir o `claude`; ele está no PATH?")?;
        std::process::exit(status.code().unwrap_or(1));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn acc(id: &str, label: &str) -> CliAccount {
        CliAccount {
            id: id.into(),
            cli: CliKind::Claude,
            config_dir: PathBuf::from(format!("/Users/x/{id}")),
            label: label.into(),
            disabled: false,
            sandbox: SandboxMode::default(),
        }
    }

    #[test]
    fn finds_by_id_label_email_and_unique_prefix() {
        let all = vec![acc("tonho", "Tonho"), acc("ela", "Namorada")];
        assert_eq!(find(&all, &[], "ELA").unwrap().id, "ela");
        assert_eq!(find(&all, &[], "namorada").unwrap().id, "ela");
        assert_eq!(find(&all, &[], "ton").unwrap().id, "tonho");
        assert!(find(&all, &[], "x").is_none());
        let emails = vec![Some("me@x.com".to_string()), Some("ela@y.com".to_string())];
        assert_eq!(find(&all, &emails, "ela@y.com").unwrap().id, "ela");
        assert_eq!(find(&all, &emails, "me@").unwrap().id, "tonho");
        let twins = vec![acc("ana1", "Ana 1"), acc("ana2", "Ana 2")];
        assert!(
            find(&twins, &[], "ana").is_none(),
            "ambiguous prefix must not pick one"
        );
    }

    #[test]
    fn skips_permissions_unless_safe_and_keeps_extra_args() {
        let extra = vec!["-c".to_string()];
        assert_eq!(
            claude_args(false, &extra),
            ["--dangerously-skip-permissions", "-c"]
        );
        assert_eq!(claude_args(true, &extra), ["-c"]);
        let dup = vec!["--dangerously-skip-permissions".to_string()];
        assert_eq!(claude_args(false, &dup), ["--dangerously-skip-permissions"]);
    }

    #[test]
    fn identity_comes_from_auth_status_without_secrets() {
        let json = r#"{"loggedIn":true,"authMethod":"claude.ai","email":"a@b.com","orgId":"x","subscriptionType":"max"}"#;
        let i = parse_identity(json).unwrap();
        assert!(i.logged_in);
        assert_eq!(i.email.as_deref(), Some("a@b.com"));
        assert_eq!(i.plan.as_deref(), Some("max"));
        assert!(!parse_identity(r#"{"loggedIn":false}"#).unwrap().logged_in);
        assert!(parse_identity("not json").is_none());
    }

    #[test]
    fn version_from_claude_output() {
        assert_eq!(
            parse_version("2.1.283 (Claude Code)\n").as_deref(),
            Some("2.1.283")
        );
        assert_eq!(parse_version("error"), None);
    }

    #[test]
    fn slug_and_temp_dir() {
        assert_eq!(slug("Namorada Ana!"), "namorada-ana");
        assert_eq!(slug("  "), "claude");
        assert!(valid_id(&slug("Conta Nova 2")));
        assert!(in_temp_dir(Path::new("/var/folders/bq/x/T/omniget-acc-1")));
        assert!(!in_temp_dir(Path::new(
            "/Users/x/Library/Application Support/wtf.tonho.omniget/llm/profiles/a"
        )));
    }
}

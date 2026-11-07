use std::io::{BufRead, IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{anyhow, bail, Context};
use omniget_core::core::llm::cli_runtime::accounts::{
    default_profile_dir, primary, set_primary, valid_id, AccountStore, CliAccount, CliKind,
    SandboxMode, SCRUB_ENV,
};
use serde_json::Value;

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

fn main_account() -> Option<CliAccount> {
    AccountStore::default_store().and_then(|s| primary(&s, CliKind::Claude))
}

#[derive(Debug, Clone, Copy, Default)]
pub struct Flags {
    pub list: bool,
    pub add: bool,
    pub default: bool,
    pub sync: bool,
    pub pick: bool,
    pub remove: bool,
    pub safe: bool,
    pub no_splash: bool,
}

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

pub fn in_temp_dir(dir: &Path) -> bool {
    let s = dir.to_string_lossy();
    s.starts_with("/var/folders/")
        || s.starts_with("/private/var/folders/")
        || s.starts_with("/tmp/")
        || s.starts_with("/private/tmp/")
}

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
        Some(_) => s.push_str(" — not logged in"),
        None => {}
    }
    if a.config_dir.as_os_str().is_empty() {
        s.push_str(" [terminal default profile]");
    } else if in_temp_dir(&a.config_dir) {
        s.push_str(" [temporary folder: it may disappear; recreate with --add --default]");
    }
    s
}

fn pick(accounts: &[CliAccount], who: &[Option<Identity>]) -> anyhow::Result<CliAccount> {
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
    write!(err, "Account [{}]: ", default + 1)?;
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
        .ok_or_else(|| anyhow!("no account matches {line:?}"))
}

pub fn claude_args(safe: bool, extra: &[String]) -> Vec<String> {
    let mut args = Vec::new();
    if !safe && !extra.iter().any(|a| a == "--dangerously-skip-permissions") {
        args.push("--dangerously-skip-permissions".to_string());
    }
    args.extend(extra.iter().cloned());
    args
}

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

fn add(name: Option<String>) -> anyhow::Result<Option<CliAccount>> {
    let store =
        AccountStore::default_store().ok_or_else(|| anyhow!("OmniGet data folder not found"))?;
    let taken: Vec<String> = store.list().iter().map(|a| a.id.clone()).collect();
    let base = slug(name.as_deref().unwrap_or("claude"));
    let mut id = base.clone();
    let mut n = 2;
    while taken.contains(&id) || !valid_id(&id) {
        id = format!("{base}-{n}");
        n += 1;
    }
    let dir = default_profile_dir(&id).ok_or_else(|| anyhow!("profiles folder not found"))?;
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
        "omniget: account {id} created; opening the Claude Code login (choose the account in the browser)"
    );
    let status = claude_command(Some(&account))
        .args(["auth", "login"])
        .status()
        .context("could not open `claude`; is it on your PATH?")?;
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
                "Done: {email} · {}",
                i.plan.as_deref().unwrap_or("unknown plan")
            );
            if !same.is_empty() {
                println!(
                    "Warning: this login is already registered as {}",
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
                "The login was not completed (exit {:?}). Account {id} was created; run `omniget claude {id}` and use /login.",
                status.code()
            );
            Ok(None)
        }
    }
}

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
        .ok_or_else(|| anyhow!("account {query:?} not found; see `omniget claude --list`"))
}

fn make_main(account: &CliAccount) -> anyhow::Result<()> {
    let store =
        AccountStore::default_store().ok_or_else(|| anyhow!("OmniGet data folder not found"))?;
    set_primary(&store, &account.id).map_err(|e| anyhow!("{e:?}"))?;
    let who = identity(account);
    println!(
        "Main account: {}",
        describe(account, who.as_ref().filter(|i| i.logged_in))
    );
    let installed = super::claude_sync::write_main(account)?;
    println!("`claude` in the terminal opens with it, the app tries it first and `omniget claude` marks it in the list (Enter opens it). To go straight to another account: omniget claude <account>.");
    for rc in installed {
        println!(
            "Installed the shortcut in {}; in terminals that are already open, run `exec $SHELL` once.",
            rc.display()
        );
    }
    Ok(())
}

fn remove(account: &CliAccount) -> anyhow::Result<()> {
    let store =
        AccountStore::default_store().ok_or_else(|| anyhow!("OmniGet data folder not found"))?;
    store.remove(&account.id).map_err(|e| anyhow!("{e:?}"))?;
    println!("Removed: {} ({})", account.label, account.id);
    if !account.config_dir.as_os_str().is_empty() {
        println!(
            "The login stays in {}; delete the folder if you will not use it again.",
            account.config_dir.display()
        );
    }
    if let Some(next) = main_account() {
        println!("Main account now: {} ({})", next.label, next.id);
    }
    Ok(())
}

pub fn execute(account: Option<String>, flags: Flags, extra: Vec<String>) -> anyhow::Result<()> {
    if flags.add {
        let added = add(account)?;
        if let Some(a) = &added {
            super::claude_sync::sync(Some(a), main_account().as_ref())?;
        }
        if let (true, Some(a)) = (flags.default, added) {
            make_main(&a)?;
        }
        return Ok(());
    }
    let accounts = claude_accounts();
    if flags.list {
        if accounts.is_empty() {
            println!("No Claude account registered. Create one with `omniget claude --add` or in the app under Settings → AI and agents.");
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
            println!("★ = main account (change it with omniget claude --default <account>)");
        }
        return Ok(());
    }
    if flags.sync {
        let only = account
            .as_deref()
            .map(|q| resolve(&accounts, q))
            .transpose()?;
        return super::claude_sync::sync(only.as_ref(), main_account().as_ref());
    }
    if flags.default || flags.remove {
        if accounts.is_empty() {
            bail!("no Claude account registered; create one with `omniget claude --add`");
        }
        let target = match &account {
            Some(q) => resolve(&accounts, q)?,
            None if flags.remove => {
                bail!("say which account to remove: omniget claude --remove <account>")
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
        (None, _) => {
            match main_account().filter(|_| !flags.pick && !std::io::stdin().is_terminal()) {
                Some(main) => Some(main),
                None => Some(pick(&accounts, &identities(&accounts))?),
            }
        }
    };

    if let Some(a) = &chosen {
        if !a.config_dir.as_os_str().is_empty() {
            std::fs::create_dir_all(&a.config_dir)
                .with_context(|| format!("could not create {}", a.config_dir.display()))?;
        }
    }
    if let Ok(me) = std::env::current_exe() {
        let _ = Command::new(me)
            .args(["claude", "--sync"])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();
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
        let (version, who) = std::thread::scope(|s| {
            let v = s.spawn(claude_version);
            let w = s.spawn(|| chosen.as_ref().and_then(identity));
            (v.join().ok().flatten(), w.join().ok().flatten())
        });
        let account = chosen
            .as_ref()
            .map(|a| a.label.clone())
            .unwrap_or_else(|| "terminal default login".to_string());
        let who = who.filter(|i| i.logged_in).map(|i| {
            format!(
                "{} · {}",
                i.email.unwrap_or_default(),
                i.plan.unwrap_or_else(|| "?".into())
            )
        });
        super::splash::print(version.as_deref(), &account, who.as_deref());
    } else if let Some(a) = &chosen {
        eprintln!("omniget: Claude Code on account {}", describe(a, None));
    } else {
        eprintln!("omniget: no account registered in the app; using the terminal default login");
    }

    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        let error = cmd.exec();
        bail!("could not open `claude` ({error}); is it on your PATH?");
    }
    #[cfg(not(unix))]
    {
        let status = cmd
            .status()
            .context("could not open `claude`; is it on your PATH?")?;
        std::process::exit(status.code().unwrap_or(1));
    }
}

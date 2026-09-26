//! `omniget claude [account] [-- args]`: opens Claude Code in the terminal on
//! one of the accounts registered in the desktop app (/llm → Accounts).
//!
//! Same model as the app's "Enter" button (`login_plan`): the account is only
//! its config dir, passed as `CLAUDE_CONFIG_DIR`; the CLI keeps its own login
//! there and this command never reads a credential. API keys in the current
//! shell are removed (`SCRUB_ENV`) so they never decide which account bills.

use std::io::{BufRead, Write};
use std::path::PathBuf;
use std::process::Command;

use anyhow::{anyhow, bail, Context};
use omniget_core::core::llm::cli_runtime::accounts::{
    AccountStore, CliAccount, CliKind, SCRUB_ENV,
};

/// File with the id picked last, so a bare `omniget claude` repeats it.
fn last_path() -> Option<PathBuf> {
    omniget_core::core::paths::app_data_dir().map(|d| d.join("llm").join("cli-last-account"))
}

fn claude_accounts() -> Vec<CliAccount> {
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

/// Matches an id or a label, ignoring case; a unique prefix also works.
pub fn find<'a>(accounts: &'a [CliAccount], query: &str) -> Option<&'a CliAccount> {
    let q = query.trim().to_lowercase();
    let exact = accounts
        .iter()
        .find(|a| a.id.to_lowercase() == q || a.label.to_lowercase() == q);
    if exact.is_some() {
        return exact;
    }
    let mut prefixed = accounts
        .iter()
        .filter(|a| a.id.to_lowercase().starts_with(&q) || a.label.to_lowercase().starts_with(&q));
    match (prefixed.next(), prefixed.next()) {
        (Some(one), None) => Some(one),
        _ => None,
    }
}

fn describe(a: &CliAccount) -> String {
    if a.config_dir.as_os_str().is_empty() {
        format!("{} ({}, perfil padrão do terminal)", a.label, a.id)
    } else {
        format!("{} ({})", a.label, a.id)
    }
}

fn pick(accounts: &[CliAccount]) -> anyhow::Result<CliAccount> {
    let last = last_path()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .map(|s| s.trim().to_string());
    let default = accounts
        .iter()
        .position(|a| Some(&a.id) == last.as_ref())
        .unwrap_or(0);
    let mut err = std::io::stderr();
    for (i, a) in accounts.iter().enumerate() {
        let mark = if i == default { "*" } else { " " };
        writeln!(err, "{mark} {}) {}", i + 1, describe(a))?;
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
    find(accounts, line)
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

pub fn execute(
    account: Option<String>,
    list: bool,
    safe: bool,
    extra: Vec<String>,
) -> anyhow::Result<()> {
    let accounts = claude_accounts();
    if list {
        if accounts.is_empty() {
            println!("Nenhuma conta Claude cadastrada. Crie uma no app em /llm → Contas.");
        }
        for a in &accounts {
            println!("{}", describe(a));
        }
        return Ok(());
    }
    let chosen: Option<CliAccount> =
        match (&account, accounts.len()) {
            (Some(q), _) => Some(find(&accounts, q).cloned().ok_or_else(|| {
                anyhow!("conta {q:?} não encontrada; veja `omniget claude --list`")
            })?),
            (None, 0) => None,
            (None, 1) => Some(accounts[0].clone()),
            (None, _) => Some(pick(&accounts)?),
        };

    let mut cmd = Command::new(CliKind::Claude.bin());
    cmd.args(claude_args(safe, &extra));
    for key in SCRUB_ENV {
        cmd.env_remove(key);
    }
    match &chosen {
        Some(a) if !a.config_dir.as_os_str().is_empty() => {
            std::fs::create_dir_all(&a.config_dir)
                .with_context(|| format!("não consegui criar {}", a.config_dir.display()))?;
            cmd.env(CliKind::Claude.config_env(), &a.config_dir);
        }
        // The account is the terminal's own default login.
        _ => {
            cmd.env_remove(CliKind::Claude.config_env());
        }
    }
    if let Some(a) = &chosen {
        if let Some(p) = last_path() {
            let _ = std::fs::write(p, &a.id);
        }
        eprintln!("omniget: Claude Code na conta {}", describe(a));
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
    use omniget_core::core::llm::cli_runtime::accounts::SandboxMode;

    fn acc(id: &str, label: &str) -> CliAccount {
        CliAccount {
            id: id.into(),
            cli: CliKind::Claude,
            config_dir: PathBuf::from(format!("/tmp/{id}")),
            label: label.into(),
            disabled: false,
            sandbox: SandboxMode::default(),
        }
    }

    #[test]
    fn finds_by_id_label_and_unique_prefix() {
        let all = vec![acc("tonho", "Tonho"), acc("ela", "Namorada")];
        assert_eq!(find(&all, "ELA").unwrap().id, "ela");
        assert_eq!(find(&all, "namorada").unwrap().id, "ela");
        assert_eq!(find(&all, "ton").unwrap().id, "tonho");
        assert!(find(&all, "x").is_none());
        let twins = vec![acc("ana1", "Ana 1"), acc("ana2", "Ana 2")];
        assert!(
            find(&twins, "ana").is_none(),
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
}

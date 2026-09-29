//! `llm_*` commands: CLI accounts (Claude Code / Codex) and the usage report.
//! Owned by f4-accounts-ui.
//!
//! This file is a thin seam between the Accounts tab and two core modules that
//! do the real work:
//!
//! - `core::llm::cli_runtime::accounts` (f4-cli-runtime) — the `AccountStore`
//!   over `<app_data>/llm/accounts.json`, `detect()`, `default_profile_dir`,
//!   `account_env`/`SCRUB_ENV`, and the profile hygiene (`share_profile`,
//!   `sanitize_claude_json`). Creating an account here goes through it, so the
//!   file this command writes is byte-for-byte the file the runtime reads.
//! - `core::llm::cli_usage` (f4-cli-usage) — the incremental scan of the CLIs'
//!   own JSONL history, priced with `pricing.rs`, plus the 5 h / 7 d windows
//!   with their provenance (`Real` from an official channel, `Estimated` from
//!   the JSONL).
//!
//! What this layer is still not allowed to do: it stores **pointers only** (an
//! id, a label, which CLI, a config directory). It never opens
//! `.credentials.json`, never reads the Keychain item, never refreshes a token
//! and never calls `/api/oauth/usage`. Signing in happens inside the CLI, in a
//! terminal the user sees (`estudos/74` §A.3, plan §9.2). Automatic rotation
//! ships **off** (plan §10 D-2).
//!
//! Two files under `<app_data>/llm/`:
//! - `accounts.json` — owned by `cli_runtime::accounts`; this layer only calls
//!   its store.
//! - `accounts-ui.json` — what only the UI owns: the fallback chain order and
//!   the rotation settings.
//!
//! Cost: `llm_accounts_list` reads two small JSON files and never scans. The
//! sweep (≈2 s cold over 500 MB, measured by f4-cli-usage) only runs inside
//! `llm_cli_usage_report`, on a click or when the tab opens, and it runs on a
//! blocking thread so it never sits on a Tauri command worker.

use std::path::{Path, PathBuf};

use omniget_core::core::llm::cli_runtime::accounts as store;
use omniget_core::core::llm::cli_runtime::accounts::{
    account_env, AccountStore, CliAccount, CliKind, SandboxMode, SCRUB_ENV, SHARE_LINK,
};
use omniget_core::core::llm::cli_usage;
use omniget_core::core::llm::cli_usage::windows::{WindowKind, WindowSource};
use serde::{Deserialize, Serialize};
use serde_json::Value;

// ---------------------------------------------------------------------------
// Stable error codes the UI maps (briefing §7).
// ---------------------------------------------------------------------------

pub const ERR_ACCOUNTS_IO: &str = "ERR_ACCOUNTS_IO";
pub const ERR_ACCOUNTS_MISSING: &str = "ERR_ACCOUNTS_MISSING";
pub const ERR_ACCOUNTS_CLI: &str = "ERR_ACCOUNTS_CLI";
pub const ERR_ACCOUNTS_TERMINAL: &str = "ERR_ACCOUNTS_TERMINAL";
pub const ERR_ACCOUNTS_STORE: &str = "ERR_ACCOUNTS_STORE";
pub const ERR_CLI_USAGE_UNAVAILABLE: &str = "ERR_CLI_USAGE_UNAVAILABLE";

/// Rotation default, plan §10 D-2: 90% of the worst known window.
pub const ROTATION_THRESHOLD: f32 = 0.90;
/// Cooldown between automatic switches, in seconds (plan §9.2).
pub const ROTATION_COOLDOWN_S: u32 = 300;
/// How many days of history the dashboard asks for by default.
pub const REPORT_DAYS: u32 = 30;

// ---------------------------------------------------------------------------
// Wire types (this layer owns the contract with `llm-accounts-store.svelte.ts`)
// ---------------------------------------------------------------------------

/// Rotation settings. `enabled` is false out of the box.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Rotation {
    pub enabled: bool,
    /// 0..1 of the worst window that triggers a switch.
    pub threshold: f32,
    pub cooldown_s: u32,
}

impl Default for Rotation {
    fn default() -> Self {
        Self {
            enabled: false,
            threshold: ROTATION_THRESHOLD,
            cooldown_s: ROTATION_COOLDOWN_S,
        }
    }
}

/// UI-owned preferences: the fallback chain and the rotation toggle.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Prefs {
    #[serde(default)]
    pub chain: Vec<String>,
    #[serde(default)]
    pub rotation: Rotation,
}

/// One usage window of one account, as `QuotaMeter` needs it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WindowView {
    pub account_id: String,
    /// `five_hour` or `seven_day`.
    pub window: String,
    /// 0..1 of the window already spent, or `None` when no ceiling is known
    /// for the plan — the UI then shows the token count and no bar, instead of
    /// a percentage nobody can back.
    pub used: Option<f32>,
    pub used_tokens: u64,
    pub resets_at_ms: Option<i64>,
    /// `reported` (official channel) or `estimated` (JSONL). Shown in the UI.
    pub source: String,
}

impl WindowView {
    fn from_core(w: &cli_usage::windows::UsageWindow) -> Self {
        Self {
            account_id: w.account.clone(),
            window: match w.window {
                WindowKind::FiveHours => "five_hour".into(),
                WindowKind::SevenDays => "seven_day".into(),
            },
            used: w.used_fraction,
            used_tokens: w.used_tokens,
            resets_at_ms: (w.resets_at > 0).then_some(w.resets_at),
            source: match w.source {
                WindowSource::Real => "reported".into(),
                WindowSource::Estimated => "estimated".into(),
            },
        }
    }
}

/// An account plus everything the card draws.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AccountView {
    pub id: String,
    pub cli: String,
    pub label: String,
    pub config_dir: PathBuf,
    pub disabled: bool,
    /// `read-only` (the default for every account) or `write`. The card prints
    /// it in words and asks before moving to `write`.
    pub sandbox: String,
    /// The flag the CLI actually receives for that mode, shown next to the
    /// label so the user can check it against the CLI's own documentation
    /// instead of trusting ours.
    pub sandbox_flag: String,
    pub windows: Vec<WindowView>,
    pub sessions: u32,
    /// Head of the chain: the one the router prefers right now.
    pub active: bool,
    pub last_error: Option<String>,
}

/// What `llm_accounts_list` answers: one round-trip for the whole tab.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AccountsSnapshot {
    pub accounts: Vec<AccountView>,
    /// Account ids in router order.
    pub chain: Vec<String>,
    pub rotation: Rotation,
    /// False while no quota source answered: the UI labels the meters instead
    /// of drawing a zero that looks like a measurement.
    pub quota_available: bool,
}

/// Cost (or tokens) of one key — a day or a model.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct UsageRow {
    pub key: String,
    pub calls: u64,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cost_usd: f64,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ToolRow {
    pub tool: String,
    pub calls: u32,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SessionRow {
    pub id: String,
    pub account_id: String,
    pub project: String,
    pub model: String,
    pub started_ms: i64,
    pub ended_ms: Option<i64>,
    pub turns: u32,
    pub cost_usd: f64,
}

/// The dashboard, in the shape the tab draws.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct UsageReportView {
    pub by_day: Vec<UsageRow>,
    pub by_model: Vec<UsageRow>,
    /// 7 rows (Mon..Sun) × 24 columns (local hour): tool calls.
    pub by_tool_hour: Vec<Vec<u32>>,
    pub top_tools: Vec<ToolRow>,
    pub sessions: Vec<SessionRow>,
    /// Windows of every account, so the cards get the `Estimated` ones too.
    pub windows: Vec<WindowView>,
    pub total_cost_usd: f64,
    pub scanned_files: u32,
    pub scan_ms: u64,
}

/// What [`login_plan`] produces: a script to write and a command to run.
#[derive(Debug, Clone, PartialEq)]
pub struct LoginPlan {
    pub script_path: PathBuf,
    pub script: String,
    pub program: String,
    pub args: Vec<String>,
}

// ---------------------------------------------------------------------------
// Pure helpers (unit-tested)
// ---------------------------------------------------------------------------

/// Keep the chain in the user's order, drop ids that no longer exist and append
/// accounts the chain never heard of, so a freshly created account is reachable
/// without editing the order by hand.
pub fn normalize_chain(chain: &[String], accounts: &[CliAccount]) -> Vec<String> {
    let mut out: Vec<String> = Vec::with_capacity(accounts.len());
    for id in chain {
        if accounts.iter().any(|a| &a.id == id) && !out.contains(id) {
            out.push(id.clone());
        }
    }
    for a in accounts {
        if !out.contains(&a.id) {
            out.push(a.id.clone());
        }
    }
    out
}

/// One-click switch: the account becomes the head of the chain, the rest keeps
/// its relative order. An unknown id leaves the chain untouched.
pub fn move_to_head(chain: &[String], id: &str) -> Vec<String> {
    if !chain.iter().any(|c| c == id) {
        return chain.to_vec();
    }
    let mut out = vec![id.to_string()];
    out.extend(chain.iter().filter(|c| c.as_str() != id).cloned());
    out
}

/// `Claude Max · pessoal` + `claude` → `claude-claude-max-pessoal`, made unique
/// against the ids already in use. The result always satisfies
/// `cli_runtime::accounts::valid_id` and is also a safe folder name.
pub fn account_id(cli: CliKind, label: &str, taken: &[String]) -> String {
    let slug: String = label
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    let mut squeezed = String::with_capacity(slug.len());
    let mut prev_dash = false;
    for c in slug.trim_matches('-').chars() {
        if c == '-' {
            if !prev_dash {
                squeezed.push(c);
            }
            prev_dash = true;
        } else {
            squeezed.push(c);
            prev_dash = false;
        }
    }
    let base = if squeezed.is_empty() {
        cli.as_str().to_string()
    } else {
        format!("{}-{}", cli.as_str(), squeezed)
    };
    let base: String = base.chars().take(48).collect();
    if !taken.iter().any(|t| t == &base) {
        return base;
    }
    let mut n = 2u32;
    loop {
        let candidate = format!("{}-{}", base, n);
        if !taken.iter().any(|t| t == &candidate) {
            return candidate;
        }
        n += 1;
    }
}

/// The literal flag the CLI gets for this mode, so the UI can print it.
///
/// Codex takes `--sandbox <mode>` (`read-only` is its own default) and Claude
/// takes `--permission-mode <mode>`. Neither "no rules at all" mode
/// (`danger-full-access`, `bypassPermissions`) comes from the account alone:
/// a `full` account only allows it for a turn whose run mode is
/// `full_access` (2.7), and the flag says so.
pub fn sandbox_flag(cli: CliKind, sandbox: SandboxMode) -> String {
    if sandbox == SandboxMode::Full {
        return match cli {
            CliKind::Codex => {
                "--sandbox workspace-write (danger-full-access with run mode full_access)".into()
            }
            CliKind::Claude => {
                "--permission-mode acceptEdits (bypassPermissions with run mode full_access)".into()
            }
        };
    }
    match cli {
        CliKind::Codex => format!(
            "--sandbox {}",
            omniget_core::core::llm::cli_runtime::codex::Sandbox::from(sandbox).as_str()
        ),
        CliKind::Claude => format!(
            "--permission-mode {}",
            omniget_core::core::llm::cli_runtime::claude::permission_mode(sandbox)
        ),
    }
}

/// Quote a path for a POSIX shell by single-quoting it.
fn sh_quote(path: &Path) -> String {
    format!("'{}'", path.to_string_lossy().replace('\'', "'\\''"))
}

/// Build the launcher for "enter": a script that exports the account's config
/// directory and runs the CLI, plus the terminal command that opens it.
///
/// The environment comes from `cli_runtime::accounts::account_env` and
/// `SCRUB_ENV`, the same two lists the runtime uses when it spawns a turn, so
/// an interactive login and an automated turn can never end up on different
/// accounts. The script never mentions a credential file: the CLI does the
/// login itself, inside the directory the env var points at.
///
/// `have_wt` only matters on Windows (Windows Terminal when present, `cmd`
/// otherwise).
pub fn login_plan(account: &CliAccount, script_dir: &Path, have_wt: bool) -> LoginPlan {
    let env = account_env(account);
    let bin = account.cli.bin();
    let id = &account.id;

    if cfg!(target_os = "windows") {
        let script_path = script_dir.join(format!("login-{}.cmd", id));
        let mut script = String::from("@echo off\r\n");
        for (key, value) in &env {
            script.push_str(&format!("set \"{key}={value}\"\r\n"));
        }
        for key in SCRUB_ENV {
            script.push_str(&format!("set \"{key}=\"\r\n"));
        }
        script.push_str(&format!("title OmniGet - {id}\r\n{bin}\r\n"));
        let (program, args) = if have_wt {
            (
                "wt".to_string(),
                vec![
                    "cmd".to_string(),
                    "/K".to_string(),
                    script_path.to_string_lossy().to_string(),
                ],
            )
        } else {
            (
                "cmd".to_string(),
                vec![
                    "/C".to_string(),
                    "start".to_string(),
                    String::new(),
                    "cmd".to_string(),
                    "/K".to_string(),
                    script_path.to_string_lossy().to_string(),
                ],
            )
        };
        return LoginPlan {
            script_path,
            script,
            program,
            args,
        };
    }

    // macOS opens `.command` files in Terminal.app; every other unix goes
    // through the distro's terminal alias.
    let ext = if cfg!(target_os = "macos") {
        "command"
    } else {
        "sh"
    };
    let script_path = script_dir.join(format!("login-{}.{}", id, ext));
    let mut script = String::from(
        "#!/bin/sh\n\
         # Opened by OmniGet so the CLI can sign this account in by itself.\n\
         # OmniGet never reads the credential this login writes.\n",
    );
    for (key, value) in &env {
        script.push_str(&format!("export {key}={}\n", sh_quote(Path::new(value))));
    }
    script.push_str(&format!("unset {}\n", SCRUB_ENV.join(" ")));
    script.push_str(&format!("exec {bin}\n"));
    let (program, args) = if cfg!(target_os = "macos") {
        (
            "open".to_string(),
            vec![
                "-a".to_string(),
                "Terminal".to_string(),
                script_path.to_string_lossy().to_string(),
            ],
        )
    } else {
        (
            "x-terminal-emulator".to_string(),
            vec![
                "-e".to_string(),
                "sh".to_string(),
                script_path.to_string_lossy().to_string(),
            ],
        )
    };
    LoginPlan {
        script_path,
        script,
        program,
        args,
    }
}

/// Fold the core report into the shape the tab draws. Pure, so the mapping is
/// testable without a disk full of history.
pub fn report_view(report: &cli_usage::CliUsageReport) -> UsageReportView {
    let row = |b: &omniget_core::core::tools::usage::Bucket| UsageRow {
        key: b.key.clone(),
        calls: b.calls,
        input_tokens: b.input_tokens,
        output_tokens: b.output_tokens,
        cost_usd: b.cost_usd,
    };
    let mut top_tools: Vec<ToolRow> = report
        .by_tool
        .iter()
        .map(|(tool, calls)| ToolRow {
            tool: tool.clone(),
            calls: *calls,
        })
        .collect();
    top_tools.sort_by(|a, b| b.calls.cmp(&a.calls).then_with(|| a.tool.cmp(&b.tool)));
    top_tools.truncate(8);

    let mut sessions: Vec<SessionRow> = report
        .sessions
        .iter()
        .map(|s| SessionRow {
            id: s.session_id.clone(),
            account_id: s.account.clone(),
            project: s.project.clone(),
            model: s.models.first().cloned().unwrap_or_default(),
            started_ms: s.started_at,
            ended_ms: (s.ended_at > 0).then_some(s.ended_at),
            turns: s.messages,
            cost_usd: s.cost_usd,
        })
        .collect();
    sessions.sort_by(|a, b| {
        b.cost_usd
            .partial_cmp(&a.cost_usd)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    sessions.truncate(12);

    UsageReportView {
        by_day: report.by_day.iter().map(row).collect(),
        by_model: report.by_model.iter().map(row).collect(),
        by_tool_hour: report.by_tool_hour.iter().map(|r| r.to_vec()).collect(),
        top_tools,
        sessions,
        windows: report.windows.iter().map(WindowView::from_core).collect(),
        total_cost_usd: report.totals.cost_usd,
        scanned_files: report.stats.files_seen,
        scan_ms: report.stats.elapsed_ms,
    }
}

/// Minutes east of UTC for the machine's own clock, so the day buckets and the
/// heat map land on the user's calendar and not on UTC's.
fn tz_offset_minutes() -> i32 {
    chrono::Local::now().offset().local_minus_utc() / 60
}

// ---------------------------------------------------------------------------
// Stores
// ---------------------------------------------------------------------------

fn account_store() -> Result<AccountStore, String> {
    AccountStore::default_store().ok_or_else(|| format!("{}: no app data dir", ERR_ACCOUNTS_IO))
}

fn llm_dir() -> Result<PathBuf, String> {
    let dir = omniget_core::core::paths::app_data_dir()
        .ok_or_else(|| format!("{}: no app data dir", ERR_ACCOUNTS_IO))?
        .join("llm");
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {}", ERR_ACCOUNTS_IO, e))?;
    Ok(dir)
}

fn prefs_path() -> Result<PathBuf, String> {
    Ok(llm_dir()?.join("accounts-ui.json"))
}

pub fn load_prefs() -> Result<Prefs, String> {
    let path = prefs_path()?;
    let Ok(body) = std::fs::read_to_string(&path) else {
        return Ok(Prefs::default());
    };
    Ok(serde_json::from_str::<Prefs>(&body).unwrap_or_default())
}

fn save_prefs(prefs: &Prefs) -> Result<(), String> {
    let path = prefs_path()?;
    let body =
        serde_json::to_string_pretty(prefs).map_err(|e| format!("{}: {}", ERR_ACCOUNTS_IO, e))?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, body).map_err(|e| format!("{}: {}", ERR_ACCOUNTS_IO, e))?;
    std::fs::rename(&tmp, &path).map_err(|e| format!("{}: {}", ERR_ACCOUNTS_IO, e))?;
    Ok(())
}

/// The windows the CLI runtime already cached from an official channel. A file
/// read, no scan: `llm_accounts_list` must stay instant. The `Estimated` ones
/// arrive later with the report.
fn cached_windows() -> Vec<WindowView> {
    let Some(text) = cli_usage::capacity_path().and_then(|p| std::fs::read_to_string(p).ok())
    else {
        return Vec::new();
    };
    let now = chrono::Utc::now().timestamp_millis();
    let records = cli_usage::windows::parse_capacity_cache(&text);
    cli_usage::windows::real_windows(&records, now)
        .iter()
        .map(WindowView::from_core)
        .collect()
}

/// Assemble what the tab draws out of the store, the prefs and the cache.
fn snapshot() -> Result<AccountsSnapshot, String> {
    let store = account_store()?;
    let accounts = store.list();
    let mut prefs = load_prefs()?;
    let chain = normalize_chain(&prefs.chain, &accounts);
    if chain != prefs.chain {
        prefs.chain = chain.clone();
        save_prefs(&prefs)?;
    }
    let windows = cached_windows();
    let head = chain.first().cloned();
    let views = accounts
        .iter()
        .map(|a| AccountView {
            id: a.id.clone(),
            cli: a.cli.as_str().to_string(),
            label: a.label.clone(),
            config_dir: a.config_dir.clone(),
            disabled: a.disabled,
            sandbox: a.sandbox.as_str().to_string(),
            sandbox_flag: sandbox_flag(a.cli, a.sandbox),
            windows: windows
                .iter()
                .filter(|w| w.account_id == a.id)
                .cloned()
                .collect(),
            sessions: 0,
            active: Some(&a.id) == head.as_ref() && !a.disabled,
            last_error: None,
        })
        .collect();
    Ok(AccountsSnapshot {
        accounts: views,
        chain,
        rotation: prefs.rotation,
        quota_available: !windows.is_empty(),
    })
}

/// Everything the user configured that is not identity, linked into a new
/// profile (`estudos/74` §A.3 item 2). Never the credential: `share_profile`
/// refuses the deny list even if it were asked.
fn share_into(account: &CliAccount) -> Vec<String> {
    let Some(home) = dirs::home_dir() else {
        return Vec::new();
    };
    let source = match account.cli {
        CliKind::Claude => home.join(".claude"),
        CliKind::Codex => home.join(".codex"),
    };
    if !source.is_dir() {
        return Vec::new();
    }
    let mut linked =
        store::share_profile(&source, &account.config_dir, SHARE_LINK).unwrap_or_default();
    // `.claude.json` carries the identity, so it is copied minus the account
    // fields instead of being linked.
    if account.cli == CliKind::Claude {
        for candidate in [home.join(".claude.json"), source.join(".claude.json")] {
            let Ok(text) = std::fs::read_to_string(&candidate) else {
                continue;
            };
            if let Ok(clean) = store::sanitize_claude_json(&text) {
                let dest = account.config_dir.join(".claude.json");
                if !dest.exists() && std::fs::write(&dest, clean).is_ok() {
                    linked.push(".claude.json".to_string());
                }
            }
            break;
        }
    }
    linked
}

// ---------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------

#[tauri::command]
pub async fn llm_accounts_list() -> Result<AccountsSnapshot, String> {
    snapshot()
}

/// Which CLIs exist on this machine. `cli_runtime::accounts::detect` runs
/// `--version` once per CLI and nothing else.
#[tauri::command]
pub async fn llm_accounts_detect() -> Result<Vec<store::CliDetected>, String> {
    Ok(store::detect().await)
}

/// Create an account: an id, a label and an isolated config directory. The
/// directory starts empty (the CLI writes its own credential there on
/// `/login`); with `share` the non-identity parts of the user's existing
/// profile are symlinked in.
#[tauri::command]
pub async fn llm_accounts_create(
    cli: String,
    label: String,
    share: Option<bool>,
    request_id: Option<String>,
) -> Result<AccountsSnapshot, String> {
    let kind = CliKind::parse(&cli).ok_or_else(|| format!("{}: {}", ERR_ACCOUNTS_CLI, cli))?;
    let store_ = account_store()?;
    let taken: Vec<String> = store_.list().iter().map(|a| a.id.clone()).collect();
    let label = if label.trim().is_empty() {
        kind.as_str().to_string()
    } else {
        label.trim().to_string()
    };
    let id = if let Some(request) = request_id {
        if request.is_empty()
            || request.len() > 100
            || !request
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-')
        {
            return Err(format!("{ERR_ACCOUNTS_IO}: invalid request id"));
        }
        let id = format!("setup-{request}");
        if let Some(existing) = store_.get(&id) {
            if existing.cli != kind || existing.label != label {
                return Err(format!("{ERR_ACCOUNTS_IO}: request id conflict"));
            }
            return snapshot();
        }
        id
    } else {
        account_id(kind, &label, &taken)
    };
    let config_dir = store::default_profile_dir(&id)
        .ok_or_else(|| format!("{}: no app data dir", ERR_ACCOUNTS_IO))?;
    let account = CliAccount {
        id,
        cli: kind,
        config_dir,
        label,
        disabled: false,
        // A new account never starts with the right to write.
        sandbox: SandboxMode::ReadOnly,
    };
    let account = store_
        .create(account)
        .map_err(|e| format!("{}: {}", ERR_ACCOUNTS_STORE, e))?;
    if share.unwrap_or(true) {
        let _ = share_into(&account);
    }
    let mut prefs = load_prefs()?;
    prefs.chain = normalize_chain(&prefs.chain, &store_.list());
    save_prefs(&prefs)?;
    snapshot()
}

/// Forget an account. The config directory stays on disk unless `purge` is
/// true, so a mis-click never destroys a login.
#[tauri::command]
pub async fn llm_accounts_remove(
    id: String,
    purge: Option<bool>,
) -> Result<AccountsSnapshot, String> {
    let store_ = account_store()?;
    let account = store_
        .get(&id)
        .ok_or_else(|| format!("{}: {}", ERR_ACCOUNTS_MISSING, id))?;
    store_
        .remove(&id)
        .map_err(|e| format!("{}: {}", ERR_ACCOUNTS_STORE, e))?;
    if purge.unwrap_or(false) {
        let profiles = llm_dir()?.join("profiles");
        if account.config_dir.starts_with(&profiles) {
            let _ = std::fs::remove_dir_all(&account.config_dir);
        }
    }
    let mut prefs = load_prefs()?;
    prefs.chain = normalize_chain(&prefs.chain, &store_.list());
    save_prefs(&prefs)?;
    snapshot()
}

/// "Enter": open a terminal already pointed at this account's config
/// directory, so the CLI's own `/login` runs there. OmniGet reads nothing that
/// the login writes.
#[tauri::command]
pub async fn llm_accounts_login(id: String) -> Result<Value, String> {
    let account = account_store()?
        .get(&id)
        .ok_or_else(|| format!("{}: {}", ERR_ACCOUNTS_MISSING, id))?;
    store::create_profile_dir(&account.config_dir)
        .map_err(|e| format!("{}: {}", ERR_ACCOUNTS_IO, e))?;
    let script_dir = llm_dir()?.join("launchers");
    std::fs::create_dir_all(&script_dir).map_err(|e| format!("{}: {}", ERR_ACCOUNTS_IO, e))?;

    let have_wt = cfg!(target_os = "windows")
        && omniget_core::core::dependencies::find_tool("wt")
            .await
            .is_some();
    let plan = login_plan(&account, &script_dir, have_wt);
    std::fs::write(&plan.script_path, &plan.script)
        .map_err(|e| format!("{}: {}", ERR_ACCOUNTS_IO, e))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&plan.script_path, std::fs::Permissions::from_mode(0o755));
    }

    let program = plan.program.clone();
    let args = plan.args.clone();
    let spawned = tokio::task::spawn_blocking(move || {
        omniget_core::core::process::std_command(Path::new(&program))
            .args(&args)
            .spawn()
            .map(|_| ())
            .map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| format!("{}: {}", ERR_ACCOUNTS_TERMINAL, e))?;
    spawned.map_err(|e| format!("{}: {}", ERR_ACCOUNTS_TERMINAL, e))?;

    Ok(serde_json::json!({
        "ok": true,
        "program": plan.program,
        "script": plan.script_path.to_string_lossy(),
    }))
}

/// Take an account out of the chain for a while without deleting it.
#[tauri::command]
pub async fn llm_accounts_set_disabled(
    id: String,
    disabled: bool,
) -> Result<AccountsSnapshot, String> {
    account_store()?
        .set_disabled(&id, disabled)
        .map_err(|e| format!("{}: {}", ERR_ACCOUNTS_STORE, e))?;
    snapshot()
}

/// Change what a turn on this account may touch.
///
/// Read-only is the default and the safe end; the UI asks for a confirmation
/// before sending `write`, and an unrecognised value parses back to read-only
/// rather than failing (`SandboxMode::parse`), so no typo can ever grant a
/// write. This only decides a command-line flag — it neither reads nor writes
/// anything inside the account's config directory.
#[tauri::command]
pub async fn llm_accounts_set_sandbox(
    id: String,
    sandbox: String,
) -> Result<AccountsSnapshot, String> {
    account_store()?
        .set_sandbox(&id, SandboxMode::parse(&sandbox))
        .map_err(|e| format!("{}: {}", ERR_ACCOUNTS_STORE, e))?;
    snapshot()
}

/// One-click switch: this account becomes the head of the fallback chain.
#[tauri::command]
pub async fn llm_accounts_activate(id: String) -> Result<AccountsSnapshot, String> {
    let store_ = account_store()?;
    let accounts = store_.list();
    if !accounts.iter().any(|a| a.id == id) {
        return Err(format!("{}: {}", ERR_ACCOUNTS_MISSING, id));
    }
    let mut prefs = load_prefs()?;
    prefs.chain = move_to_head(&normalize_chain(&prefs.chain, &accounts), &id);
    save_prefs(&prefs)?;
    snapshot()
}

/// Reorder the fallback chain (drag and drop in the UI).
#[tauri::command]
pub async fn llm_accounts_set_chain(chain: Vec<String>) -> Result<AccountsSnapshot, String> {
    let store_ = account_store()?;
    let mut prefs = load_prefs()?;
    prefs.chain = normalize_chain(&chain, &store_.list());
    save_prefs(&prefs)?;
    snapshot()
}

/// Read (no argument) or write the rotation settings. Off by default.
#[tauri::command]
pub async fn llm_accounts_rotation(rotation: Option<Rotation>) -> Result<Rotation, String> {
    let mut prefs = load_prefs()?;
    if let Some(next) = rotation {
        prefs.rotation = Rotation {
            enabled: next.enabled,
            threshold: next.threshold.clamp(0.1, 1.0),
            cooldown_s: next.cooldown_s.clamp(30, 3600),
        };
        save_prefs(&prefs)?;
    }
    Ok(prefs.rotation)
}

/// Cost per day, per model, tool heat map, sessions and the 5 h / 7 d windows,
/// read from the CLIs' own JSONL history by `cli_usage`.
///
/// The sweep is the expensive part (≈2 s cold over 500 MB, ~130 µs warm, as
/// f4-cli-usage measured), so the whole thing runs on a blocking thread: a
/// Tauri command worker must never sit on it. Nothing here runs at boot, and
/// nothing but the price table touches the network.
#[tauri::command]
pub async fn llm_cli_usage_report(days: Option<u32>) -> Result<UsageReportView, String> {
    let accounts: Vec<(String, PathBuf)> = account_store()?
        .list()
        .iter()
        .map(|a| (a.id.clone(), a.config_dir.clone()))
        .collect();
    let opts = cli_usage::ReportOptions {
        days: days.unwrap_or(REPORT_DAYS),
        tz_offset_minutes: tz_offset_minutes(),
        ..Default::default()
    };
    let handle = tokio::runtime::Handle::current();
    let report = tokio::task::spawn_blocking(move || {
        handle.block_on(cli_usage::report::report(&accounts, &opts))
    })
    .await
    .map_err(|e| format!("{}: {}", ERR_CLI_USAGE_UNAVAILABLE, e))?;
    Ok(report_view(&report))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn acc(id: &str) -> CliAccount {
        CliAccount {
            id: id.to_string(),
            cli: CliKind::Claude,
            config_dir: PathBuf::from("/tmp").join(id),
            label: id.to_string(),
            disabled: false,
            sandbox: SandboxMode::ReadOnly,
        }
    }

    /// The card shows a mode and the flag behind it; both must be the safe one
    /// out of the box, and the dangerous modes must be unreachable.
    #[test]
    fn the_sandbox_flag_is_the_cli_s_own_and_defaults_to_read_only() {
        assert_eq!(acc("x").sandbox, SandboxMode::ReadOnly);
        assert_eq!(
            sandbox_flag(CliKind::Codex, SandboxMode::ReadOnly),
            "--sandbox read-only"
        );
        assert_eq!(
            sandbox_flag(CliKind::Codex, SandboxMode::Write),
            "--sandbox workspace-write"
        );
        assert_eq!(
            sandbox_flag(CliKind::Claude, SandboxMode::ReadOnly),
            "--permission-mode plan"
        );
        assert_eq!(
            sandbox_flag(CliKind::Claude, SandboxMode::Write),
            "--permission-mode acceptEdits"
        );
        for cli in [CliKind::Claude, CliKind::Codex] {
            for mode in [SandboxMode::ReadOnly, SandboxMode::Write] {
                let flag = sandbox_flag(cli, mode);
                assert!(!flag.contains("danger"), "{flag}");
                assert!(!flag.contains("bypass"), "{flag}");
            }
        }
    }

    /// Whatever the UI sends, only the exact word for "write" grants a write.
    #[test]
    fn the_command_never_grants_a_write_by_accident() {
        for bad in [
            "",
            "READ-ONLY",
            "danger-full-access",
            "bypassPermissions",
            "sim",
        ] {
            assert_eq!(SandboxMode::parse(bad), SandboxMode::ReadOnly, "{bad}");
        }
        assert_eq!(SandboxMode::parse("write"), SandboxMode::Write);
    }

    #[test]
    fn rotation_is_off_by_default() {
        let r = Rotation::default();
        assert!(!r.enabled, "plan §10 D-2: rotation ships disabled");
        assert_eq!(r.threshold, ROTATION_THRESHOLD);
        assert_eq!(r.cooldown_s, ROTATION_COOLDOWN_S);
    }

    #[test]
    fn prefs_default_has_no_chain_and_no_rotation() {
        let p = Prefs::default();
        assert!(p.chain.is_empty());
        assert!(!p.rotation.enabled);
    }

    #[test]
    fn normalize_chain_keeps_order_drops_ghosts_appends_new() {
        let accounts = vec![acc("a"), acc("b"), acc("c")];
        let chain = vec!["c".to_string(), "ghost".to_string(), "a".to_string()];
        assert_eq!(normalize_chain(&chain, &accounts), vec!["c", "a", "b"]);
    }

    #[test]
    fn normalize_chain_deduplicates() {
        let accounts = vec![acc("a"), acc("b")];
        let chain = vec!["a".to_string(), "a".to_string()];
        assert_eq!(normalize_chain(&chain, &accounts), vec!["a", "b"]);
    }

    #[test]
    fn move_to_head_promotes_and_keeps_the_rest() {
        let chain = vec!["a".to_string(), "b".to_string(), "c".to_string()];
        assert_eq!(move_to_head(&chain, "c"), vec!["c", "a", "b"]);
        assert_eq!(move_to_head(&chain, "nope"), chain);
    }

    #[test]
    fn account_id_slugifies_and_dedupes() {
        let taken = vec!["claude-max-pessoal".to_string()];
        assert_eq!(
            account_id(CliKind::Claude, "Max  Pessoal", &[]),
            "claude-max-pessoal"
        );
        assert_eq!(
            account_id(CliKind::Claude, "Max · Pessoal!!", &taken),
            "claude-max-pessoal-2"
        );
        assert_eq!(account_id(CliKind::Codex, "   ", &[]), "codex");
    }

    /// The store refuses anything outside `[a-z0-9_-]{1,64}`, so the generator
    /// has to agree with it.
    #[test]
    fn account_id_is_always_valid_for_the_store() {
        let long = "A".repeat(200);
        for label in ["../../etc/passwd", "Ação Ünicode!", "", long.as_str()] {
            let id = account_id(CliKind::Claude, label, &[]);
            assert!(store::valid_id(&id), "{label:?} -> {id:?}");
            assert!(!id.contains('/'), "{id}");
            assert!(!id.contains(".."), "{id}");
        }
    }

    #[test]
    fn login_script_exports_the_config_dir_and_runs_the_cli() {
        let mut account = acc("claude-max");
        account.config_dir = PathBuf::from("/data/llm/profiles/claude-max");
        let plan = login_plan(&account, Path::new("/data/llm/launchers"), false);
        assert!(plan.script.contains("CLAUDE_CONFIG_DIR"), "{}", plan.script);
        assert!(
            plan.script.contains("/data/llm/profiles/claude-max"),
            "{}",
            plan.script
        );
        assert!(plan.script.contains("claude"), "{}", plan.script);
    }

    #[test]
    fn login_script_never_mentions_a_credential() {
        for cli in [CliKind::Claude, CliKind::Codex] {
            let mut account = acc("x");
            account.cli = cli;
            let plan = login_plan(&account, Path::new("/l"), false);
            let lower = plan.script.to_lowercase();
            for forbidden in [".credentials", "keychain", "oauth/usage", "access_token"] {
                assert!(!lower.contains(forbidden), "{cli} leaked {forbidden}");
            }
        }
    }

    /// The launcher must scrub exactly what the runtime scrubs, or an API key
    /// in OmniGet's environment would decide which account the CLI bills.
    #[test]
    fn login_script_scrubs_the_same_env_the_runtime_does() {
        let plan = login_plan(&acc("x"), Path::new("/l"), false);
        for key in SCRUB_ENV {
            assert!(
                plan.script.contains(key),
                "{key} missing from {}",
                plan.script
            );
        }
        assert!(
            plan.script.contains("CLAUDE_CODE_SUBPROCESS_ENV_SCRUB"),
            "{}",
            plan.script
        );
    }

    #[test]
    fn login_plan_uses_the_platform_terminal() {
        let mut account = acc("x");
        account.cli = CliKind::Codex;
        let plan = login_plan(&account, Path::new("/l"), false);
        if cfg!(target_os = "macos") {
            assert_eq!(plan.program, "open");
            assert_eq!(plan.args[0], "-a");
            assert_eq!(plan.args[1], "Terminal");
            assert!(plan.script_path.ends_with("login-x.command"));
        } else if cfg!(target_os = "windows") {
            assert_eq!(plan.program, "cmd");
            assert!(plan.script_path.ends_with("login-x.cmd"));
        } else {
            assert_eq!(plan.program, "x-terminal-emulator");
            assert!(plan.script_path.ends_with("login-x.sh"));
        }
    }

    #[test]
    fn login_plan_prefers_windows_terminal_when_present() {
        let plan = login_plan(&acc("x"), Path::new("/l"), true);
        if cfg!(target_os = "windows") {
            assert_eq!(plan.program, "wt");
        } else {
            assert_ne!(plan.program, "wt");
        }
    }

    #[test]
    fn codex_script_uses_codex_home() {
        let mut account = acc("c");
        account.cli = CliKind::Codex;
        let plan = login_plan(&account, Path::new("/l"), false);
        assert!(plan.script.contains("CODEX_HOME"), "{}", plan.script);
        assert!(
            !plan.script.contains("CLAUDE_CONFIG_DIR"),
            "{}",
            plan.script
        );
    }

    #[test]
    fn sh_quote_escapes_a_quote_in_the_path() {
        let q = sh_quote(Path::new("/tmp/it's here"));
        assert_eq!(q, "'/tmp/it'\\''s here'");
    }

    #[test]
    fn window_view_carries_its_provenance() {
        let real = cli_usage::windows::UsageWindow {
            account: "a".into(),
            window: WindowKind::FiveHours,
            used_tokens: 0,
            started_at: 1,
            resets_at: 2,
            used_fraction: Some(0.62),
            source: WindowSource::Real,
        };
        let estimated = cli_usage::windows::UsageWindow {
            account: "a".into(),
            window: WindowKind::SevenDays,
            used_tokens: 4_200,
            started_at: 1,
            resets_at: 0,
            used_fraction: None,
            source: WindowSource::Estimated,
        };
        let r = WindowView::from_core(&real);
        assert_eq!(r.window, "five_hour");
        assert_eq!(r.source, "reported");
        assert_eq!(r.used, Some(0.62));
        let e = WindowView::from_core(&estimated);
        assert_eq!(e.window, "seven_day");
        assert_eq!(e.source, "estimated");
        // No ceiling for the plan: tokens, and no invented percentage.
        assert_eq!(e.used, None);
        assert_eq!(e.used_tokens, 4_200);
        assert_eq!(e.resets_at_ms, None);
    }

    #[test]
    fn report_view_sorts_tools_and_sessions_and_keeps_the_grid_shape() {
        let mut report = cli_usage::CliUsageReport {
            by_tool: vec![
                ("Read".into(), 10),
                ("Bash".into(), 99),
                ("Edit".into(), 50),
            ],
            ..Default::default()
        };
        report.sessions = vec![
            cli_usage::SessionSummary {
                session_id: "cheap".into(),
                account: "a".into(),
                cost_usd: 0.5,
                messages: 3,
                started_at: 10,
                ended_at: 0,
                ..Default::default()
            },
            cli_usage::SessionSummary {
                session_id: "pricey".into(),
                account: "a".into(),
                cost_usd: 9.5,
                messages: 40,
                started_at: 20,
                ended_at: 30,
                ..Default::default()
            },
        ];
        report.by_tool_hour[2][5] = 7;
        let view = report_view(&report);
        assert_eq!(view.top_tools[0].tool, "Bash");
        assert_eq!(view.top_tools[0].calls, 99);
        assert_eq!(view.sessions[0].id, "pricey");
        assert_eq!(view.sessions[0].ended_ms, Some(30));
        // A live session has no end, and the UI must see that as null.
        assert_eq!(view.sessions[1].ended_ms, None);
        assert_eq!(view.by_tool_hour.len(), 7);
        assert_eq!(view.by_tool_hour[0].len(), 24);
        assert_eq!(view.by_tool_hour[2][5], 7);
    }

    #[test]
    fn report_view_maps_days_models_and_the_scan_stats() {
        use omniget_core::core::tools::usage::Bucket;
        let report = cli_usage::CliUsageReport {
            by_day: vec![Bucket {
                key: "2026-09-18".into(),
                calls: 4,
                cost_usd: 1.25,
                ..Default::default()
            }],
            by_model: vec![Bucket {
                key: "claude-sonnet-4-6".into(),
                calls: 9,
                input_tokens: 100,
                output_tokens: 20,
                cost_usd: 3.5,
                ..Default::default()
            }],
            stats: omniget_core::core::llm::cli_usage::ScanStats {
                files_seen: 318,
                elapsed_ms: 142,
                ..Default::default()
            },
            ..Default::default()
        };
        let view = report_view(&report);
        assert_eq!(view.by_day[0].key, "2026-09-18");
        assert_eq!(view.by_day[0].cost_usd, 1.25);
        assert_eq!(view.by_model[0].input_tokens, 100);
        assert_eq!(view.scanned_files, 318);
        assert_eq!(view.scan_ms, 142);
    }

    /// The tab and the runtime must read and write the very same file.
    #[test]
    fn the_store_is_the_runtimes_accounts_json() {
        let Some(expected) = omniget_core::core::paths::app_data_dir() else {
            return;
        };
        let Some(path) = store::default_path() else {
            return;
        };
        assert_eq!(path, expected.join("llm").join("accounts.json"));
    }

    #[test]
    fn timezone_offset_is_whole_minutes_within_a_day() {
        let tz = tz_offset_minutes();
        assert!((-14 * 60..=14 * 60).contains(&tz), "{tz}");
    }
}

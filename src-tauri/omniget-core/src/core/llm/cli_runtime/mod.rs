//! CLI agent runtime: drives an installed Claude Code or Codex inside an
//! isolated config directory. Owned by f4-cli-runtime.
//!
//! The second runtime of the roster (plan §2.1 line 14, §3 Fase 4). It adopts
//! the *model* of `claude-account`/`claude-swap` and none of their mechanism:
//! one config directory per account, the CLI itself logging in, and the app
//! never reading a credential, never refreshing a token and never calling an
//! undocumented usage endpoint (`estudos/74` §A.3, plan §9.2).
//!
//! Shape:
//!
//! ```text
//! AgentDef{ runtime: Cli{cli, account} }
//!   → CliRuntime::turn            (resolve account, binary, argv, env)
//!     → session::spawn_turn       (process, stdin = prompt, stdout = JSONL)
//!       → claude::ClaudeParser / codex::CodexParser  (line → CliSignal)
//!         → TurnEvent to the coordinator
//!         → RateSnapshot to CliCapacity → router::CapacitySource
//! ```
//!
//! Budget: nothing runs between turns, and switching accounts is one map of
//! environment variables, so it costs nothing measurable.

pub mod accounts;
pub mod claude;
pub mod codex;
pub mod codex_app_server;
pub mod parse;
pub mod session;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, RwLock};

use async_trait::async_trait;
use futures::stream::BoxStream;

use super::agent::{AgentDef, RuntimeKind};
use super::error::LlmError;
use super::router::{Capacity, CapacitySource};
use super::runtime::AgentRuntime;
use super::types::{TurnEvent, TurnRequest};

pub use accounts::{
    detect, AccountStore, CliAccount, CliDetected, CliKind, SandboxMode, DENY_SHARE, SCRUB_ENV,
    SHARE_LINK,
};
pub use parse::{CliSignal, QuotaSource, RateSnapshot, RateStatus, RateWindow};
pub use session::{LineParser, SpawnSpec, WatchdogConfig};

/// The CLI binary is not installed, or the account points nowhere.
pub const ERR_CLI_NOT_FOUND: &str = "ERR_CLI_NOT_FOUND";
/// The process could not be created, or died before saying anything.
pub const ERR_CLI_SPAWN: &str = "ERR_CLI_SPAWN";
/// The process exited non-zero; the message carries the code and the stderr tail.
pub const ERR_CLI_EXIT: &str = "ERR_CLI_EXIT";
/// The account has no login in its config directory (the CLI says so itself).
pub const ERR_CLI_AUTH: &str = "ERR_CLI_AUTH";
/// The account record is missing, duplicated or malformed.
pub const ERR_CLI_ACCOUNT: &str = "ERR_CLI_ACCOUNT";
/// Reading or writing `accounts.json` / a profile directory failed.
pub const ERR_CLI_IO: &str = "ERR_CLI_IO";
/// The CLI went silent past the watchdog's window (no stdout line for
/// `idle_timeout`, or `tool_idle_timeout` while one of its tools had not
/// answered). Its process tree was killed; never rerouted, because the turn
/// may already have had side effects.
pub const ERR_CLI_STALLED: &str = "ERR_CLI_STALLED";
/// The CLI printed nothing at all within `start_timeout` of its spawn: the
/// prompt never took. Its process tree was killed.
pub const ERR_CLI_NO_START: &str = "ERR_CLI_NO_START";
/// Quota exhausted. Defined by the router (F2) because that is what reroutes
/// on it; re-exported here so this module has one error surface.
pub use super::router::ERR_CLI_RATE;

/// Last known quota per account id, fed by `rate_limit_event` during turns and
/// read by the router before the next one.
///
/// This is the whole quota story on the OmniGet side: an in-memory map filled
/// by the official stream events. There is no polling loop, no HTTP call and
/// no credential read anywhere in it.
#[derive(Debug, Default)]
pub struct CliCapacity {
    per_account: RwLock<HashMap<String, RateSnapshot>>,
}

impl CliCapacity {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn record(&self, account_id: &str, snapshot: RateSnapshot) {
        if let Ok(mut map) = self.per_account.write() {
            map.insert(account_id.to_string(), snapshot);
        }
    }

    pub fn get(&self, account_id: &str) -> Option<RateSnapshot> {
        self.per_account.read().ok()?.get(account_id).cloned()
    }

    /// Same, knowing which CLI the account drives. A Codex account is flagged
    /// `code_specialist`, which is how the router prefers it for a
    /// `TaskKind::Code` turn without ever looking at an account itself.
    pub fn capacity_of_cli(
        &self,
        account_id: &str,
        disabled: bool,
        cli: Option<CliKind>,
    ) -> Capacity {
        if disabled {
            return Capacity::unavailable();
        }
        let code_specialist = cli == Some(CliKind::Codex);
        match self.get(account_id) {
            Some(snapshot) => Capacity {
                quota_remaining: snapshot.quota_remaining(),
                code_specialist,
                ..Capacity::default()
            },
            None => Capacity {
                code_specialist,
                ..Capacity::default()
            },
        }
    }
}

/// [`CapacitySource`] over a [`CliCapacity`] plus the account list, so the
/// router can ask about a `Candidate` without knowing any of this.
pub struct CliCapacitySource {
    capacity: Arc<CliCapacity>,
    accounts: Arc<AccountStore>,
}

impl CliCapacitySource {
    pub fn new(capacity: Arc<CliCapacity>, accounts: Arc<AccountStore>) -> Self {
        Self { capacity, accounts }
    }
}

impl CapacitySource for CliCapacitySource {
    fn capacity(&self, candidate: &super::agent::Candidate) -> Capacity {
        match &candidate.runtime {
            super::agent::CandidateRuntime::Cli { account_id } => {
                match self.accounts.get(account_id) {
                    Some(account) => self.capacity.capacity_of_cli(
                        account_id,
                        account.disabled,
                        Some(account.cli),
                    ),
                    // An account that no longer exists can never serve a turn.
                    None => Capacity::unavailable(),
                }
            }
            // Native candidates are somebody else's business.
            super::agent::CandidateRuntime::Native { .. } => Capacity::default(),
        }
    }
}

/// Options that do not come from the agent definition.
#[derive(Debug, Clone)]
pub struct CliRuntimeOptions {
    /// Ask the CLI for token-level deltas (Claude only).
    pub partial_messages: bool,
    /// Working directory for the child. `None` leaves it at ours.
    pub cwd: Option<PathBuf>,
    /// Where personal (projectless) conversations run: an empty private
    /// folder per bot, `<root>/<bot>`. `None` = `<llm dir>/sandboxes`.
    pub sandbox_root: Option<PathBuf>,
}

impl Default for CliRuntimeOptions {
    fn default() -> Self {
        Self {
            partial_messages: true,
            cwd: None,
            sandbox_root: None,
        }
    }
}

/// Extra variables for the turns of one conversation, set by whoever owns
/// it for as long as it runs: a background job hands its child its own
/// scoped bridge token (`OMNIGET_JOB_TOKEN`, 2.1) this way. Keyed like
/// `code_tools::set_conversation_workspace`, so the turn finds it by the same
/// conversation id. Never part of the argv.
fn conversation_envs(
) -> &'static std::sync::Mutex<HashMap<String, std::collections::BTreeMap<String, String>>> {
    static ENVS: std::sync::OnceLock<
        std::sync::Mutex<HashMap<String, std::collections::BTreeMap<String, String>>>,
    > = std::sync::OnceLock::new();
    ENVS.get_or_init(Default::default)
}

pub fn set_conversation_env(conversation: &str, env: std::collections::BTreeMap<String, String>) {
    conversation_envs()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(conversation.to_string(), env);
}

pub fn clear_conversation_env(conversation: &str) {
    conversation_envs()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .remove(conversation);
}

/// What [`set_conversation_env`] left for this conversation (empty if none).
pub fn conversation_env(conversation: &str) -> std::collections::BTreeMap<String, String> {
    conversation_envs()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(conversation)
        .cloned()
        .unwrap_or_default()
}

/// How one turn is launched, decided from the session record and the caps.
#[derive(Debug, Clone)]
pub struct LaunchPlan {
    pub cwd: Option<PathBuf>,
    pub projectless: bool,
    /// `Some(handle)` = resume the provider session (`--resume`).
    pub resume: Option<String>,
    pub resume_kind: crate::core::assist::runs::ResumeKind,
    pub mcp_config: Option<PathBuf>,
    pub permission_prompt_tool: Option<String>,
    pub restrict_tools: bool,
    /// An external (MCP-controlled) mission: no built-in tools at all, a
    /// private empty folder, only OmniGet's projection.
    pub external: bool,
    /// A project job whose CLI knows `--tools`, `--strict-mcp-config` and
    /// `--exclude-dynamic-system-prompt-sections`: launched lean (see
    /// [`claude::PROJECT_TOOLS`]) when the account can write.
    pub lean_project: bool,
    /// The CLI lists `--effort` in its help (see [`claude::PROJECT_EFFORT`]).
    pub effort_flag: bool,
    /// The turn's run mode (2.7): the job's or Loop's binding, else the
    /// chat's choice. `None` = nobody chose: the chat default `auto_edits`,
    /// which is the argv every turn had before.
    pub run_mode: Option<super::run_mode::RunMode>,
    /// `claude --help` lists `auto` among the `--permission-mode` choices.
    pub auto_mode: bool,
    /// `codex exec --help` lists `--approve-for-me` (Codex's `auto`).
    pub approve_for_me: bool,
    /// The turn's `--settings` file with OmniGet's permission hooks (3.4,
    /// [`super::claude_hooks`]). `None` = no hook binary or no `--settings`
    /// in this CLI: the argv of before.
    pub hook_settings: Option<PathBuf>,
    /// The turn whose file baselines the parser records (3.4).
    pub baseline_request: Option<String>,
    /// Codex over its app-server (3.6): who asks on behalf of the turn.
    /// `None` = the one-shot `exec --json` (older Codex, or forced).
    pub app_server: Option<AppServerTurn>,
    /// A side call (3.7, [`crate::core::llm::router::TaskKind::Side`]): the
    /// conversation id starts with `side-`. `None` = an ordinary turn.
    pub side: Option<SideLaunch>,
}

/// How lean a side call can be launched, from the CLI's help: each flag only
/// when the installed version lists it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SideLaunch {
    /// `--system-prompt` (ours replaces Claude Code's own).
    pub replace_system: bool,
    /// `--no-session-persistence`.
    pub no_persist: bool,
    /// `--tools` (an empty list: no built-in tools).
    pub no_tools: bool,
    /// `--strict-mcp-config` (none of the user's MCP servers).
    pub strict_mcp: bool,
}

/// The turn a Codex app-server conversation runs for: its approvals become
/// durable asks of this agent and request (`runs_permissions`).
#[derive(Debug, Clone)]
pub struct AppServerTurn {
    pub agent_id: String,
    pub request_id: String,
    pub conversation: String,
}

/// The CLI half of `CompositeRuntime`.
pub struct CliRuntime {
    accounts: Arc<AccountStore>,
    capacity: Arc<CliCapacity>,
    options: CliRuntimeOptions,
    /// Resolved binary per CLI, so `find_tool` runs once per process.
    binaries: RwLock<HashMap<CliKind, PathBuf>>,
    /// Probed (or pinned, in tests) capabilities per CLI.
    caps: RwLock<HashMap<CliKind, super::caps::RuntimeCaps>>,
    /// The windows every turn is launched under (the app pushes the user's
    /// settings; the shipped defaults until then).
    watchdog: RwLock<WatchdogConfig>,
    /// "Is a person being asked something for this request id?" — the
    /// broker's answer, so a turn waiting on a permission prompt is never
    /// taken for a stuck one.
    pause_probe: Option<PauseProbe>,
    /// Where a Codex app-server turn sends its approvals (weak: the runtime
    /// never keeps the broker alive). `None` = every question is declined
    /// with the reason, as `exec` did.
    broker: Option<std::sync::Weak<super::broker::ToolBroker>>,
}

/// See [`CliRuntime::with_pause_probe`].
pub type PauseProbe = Arc<dyn Fn(&str) -> bool + Send + Sync>;

impl std::fmt::Debug for CliRuntime {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CliRuntime")
            .field("accounts", &self.accounts.list().len())
            .finish()
    }
}

impl CliRuntime {
    pub fn new(accounts: Arc<AccountStore>, capacity: Arc<CliCapacity>) -> Self {
        Self {
            accounts,
            capacity,
            options: CliRuntimeOptions::default(),
            binaries: RwLock::new(HashMap::new()),
            caps: RwLock::new(HashMap::new()),
            watchdog: RwLock::new(WatchdogConfig::default()),
            pause_probe: None,
            broker: None,
        }
    }

    /// The broker that asks a person when Codex (app-server) wants a command
    /// or an edit approved.
    pub fn with_broker(mut self, broker: std::sync::Weak<super::broker::ToolBroker>) -> Self {
        self.broker = Some(broker);
        self
    }

    /// Freezes a turn's idle clock while `probe(request_id)` says a question
    /// of that turn waits for a person (`ToolBroker::has_pending`).
    pub fn with_pause_probe(mut self, probe: PauseProbe) -> Self {
        self.pause_probe = Some(probe);
        self
    }

    /// The windows the next turns run under. Turns already running keep the
    /// ones they started with.
    pub fn set_watchdog(&self, config: WatchdogConfig) {
        if let Ok(mut w) = self.watchdog.write() {
            *w = config;
        }
    }

    pub fn watchdog(&self) -> WatchdogConfig {
        self.watchdog.read().map(|w| *w).unwrap_or_default()
    }

    /// Pins the capabilities of a CLI (tests; a fake binary has no help).
    pub fn set_caps(&self, cli: CliKind, caps: super::caps::RuntimeCaps) {
        if let Ok(mut map) = self.caps.write() {
            map.insert(cli, caps);
        }
    }

    /// What the installed CLI supports, probed once per process.
    pub async fn caps_of(
        &self,
        cli: CliKind,
        binary: &std::path::Path,
    ) -> super::caps::RuntimeCaps {
        if let Some(c) = self.caps.read().ok().and_then(|m| m.get(&cli).cloned()) {
            return c;
        }
        let probed = super::caps::probe_binary(cli.bin(), binary).await;
        self.set_caps(cli, probed.clone());
        probed
    }

    fn sandbox_dir(&self, bot: &str) -> Option<PathBuf> {
        let root = self
            .options
            .sandbox_root
            .clone()
            .or_else(|| super::roster_store::llm_dir().map(|d| d.join("sandboxes")))?;
        let safe: String = bot
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                    c
                } else {
                    '_'
                }
            })
            .collect();
        let dir = root.join(if safe.is_empty() {
            "bot".to_string()
        } else {
            safe
        });
        std::fs::create_dir_all(&dir).ok()?;
        Some(dir)
    }

    pub fn capacity(&self) -> Arc<CliCapacity> {
        self.capacity.clone()
    }

    pub fn accounts(&self) -> Arc<AccountStore> {
        self.accounts.clone()
    }

    /// Pins a binary by hand. Used by the tests (and by the binary override
    /// the user can set) so no real CLI is needed to exercise the runtime.
    pub fn set_binary(&self, cli: CliKind, path: PathBuf) {
        if let Ok(mut map) = self.binaries.write() {
            map.insert(cli, path);
        }
    }

    async fn binary(&self, cli: CliKind) -> Result<PathBuf, LlmError> {
        if let Some(path) = self.binaries.read().ok().and_then(|m| m.get(&cli).cloned()) {
            return Ok(path);
        }
        let found = crate::core::dependencies::find_tool(cli.bin())
            .await
            .ok_or_else(|| {
                LlmError::new(
                    ERR_CLI_NOT_FOUND,
                    format!("`{}` is not installed or not on PATH", cli.bin()),
                )
            })?;
        self.set_binary(cli, found.clone());
        Ok(found)
    }

    fn account(&self, id: &str) -> Result<CliAccount, LlmError> {
        let account = self
            .accounts
            .get(id)
            .ok_or_else(|| LlmError::new(ERR_CLI_ACCOUNT, format!("no CLI account `{id}`")))?;
        if account.disabled {
            return Err(LlmError::new(
                ERR_CLI_ACCOUNT,
                format!("account `{id}` is disabled"),
            ));
        }
        if !account.config_dir.as_os_str().is_empty() && !account.config_dir.is_dir() {
            return Err(LlmError::new(
                ERR_CLI_NOT_FOUND,
                format!(
                    "the config dir of `{id}` is gone: {}",
                    account.config_dir.display()
                ),
            ));
        }
        Ok(account)
    }

    /// Builds everything the child needs, without spawning. Public so a test
    /// (and the verifier) can read the argv and the environment.
    pub fn plan(
        &self,
        account: &CliAccount,
        binary: PathBuf,
        req: &TurnRequest,
        system_prompt: Option<String>,
    ) -> (SpawnSpec, Arc<dyn LineParser>) {
        self.plan_with(account, binary, req, system_prompt, None)
    }

    /// [`Self::plan`] for a turn with a session decision. `None` keeps the
    /// legacy launch (no session record: the whole transcript on stdin).
    pub fn plan_with(
        &self,
        account: &CliAccount,
        binary: PathBuf,
        req: &TurnRequest,
        system_prompt: Option<String>,
        launch: Option<&LaunchPlan>,
    ) -> (SpawnSpec, Arc<dyn LineParser>) {
        let legacy_cwd =
            || crate::core::llm::code_tools::workspace().or_else(|| self.options.cwd.clone());
        let cwd = match launch {
            Some(l) => l.cwd.clone(),
            None => legacy_cwd(),
        };
        let env = accounts::account_env(account);
        let scrub: Vec<String> = SCRUB_ENV.iter().map(|s| s.to_string()).collect();
        match account.cli {
            CliKind::Claude => {
                let side = launch.and_then(|l| l.side);
                let resume = launch
                    .filter(|_| side.is_none())
                    .and_then(|l| l.resume.clone());
                // Resumed: the provider holds the history; only this turn's
                // instructions/context and the new message travel.
                let (system_prompt, stdin) = match &resume {
                    // A side call: the system prompt is the caller's (it
                    // replaces Claude Code's when the CLI allows), and only
                    // the conversation itself goes on stdin, no injected
                    // context (memory, KB) the auditor must not see.
                    None if side.is_some() => {
                        let talk: Vec<super::types::Message> = req
                            .messages
                            .iter()
                            .filter(|m| m.role != super::types::Role::System)
                            .cloned()
                            .collect();
                        (system_prompt, claude::render_prompt(&talk))
                    }
                    Some(_) => {
                        let sys = claude::system_text(&req.messages);
                        (
                            (!sys.trim().is_empty()).then_some(sys).or(system_prompt),
                            claude::last_user_text(&req.messages),
                        )
                    }
                    None => (system_prompt, claude::render_prompt(&req.messages)),
                };
                let restrict = launch
                    .map(|l| l.projectless && l.restrict_tools)
                    .unwrap_or(false);
                let external = launch.map(|l| l.external).unwrap_or(false);
                let mcp = launch
                    .and_then(|l| l.mcp_config.as_ref())
                    .map(|p| p.display().to_string());
                // Missions measured on 2.1.283: -25..-70% cost per job, mostly
                // the fixed context (user MCPs, agent tools) and cross-job
                // cache reuse; the batching line cuts turns.
                let lean = !external
                    && side.is_none()
                    && !restrict
                    && account.sandbox.writes()
                    && launch
                        .map(|l| l.lean_project && !l.projectless)
                        .unwrap_or(false);
                let system_prompt = if lean {
                    Some(match system_prompt {
                        Some(s) if !s.trim().is_empty() => {
                            format!("{s}\n\n{}", claude::PROJECT_BATCHING)
                        }
                        _ => claude::PROJECT_BATCHING.to_string(),
                    })
                } else {
                    system_prompt
                };
                // `None` without a turn context: the account's sandbox alone,
                // as before 2.7.
                let claude_permission = launch.filter(|_| !external).map(|l| {
                    super::run_mode::claude_permission(
                        l.run_mode.unwrap_or_default().under(account.sandbox),
                        l.auto_mode,
                    )
                });
                let has_prompt_tool = launch.is_some_and(|l| l.permission_prompt_tool.is_some());
                let args = claude::ClaudeArgs {
                    model: model_of(req),
                    system_prompt,
                    max_budget_usd: None,
                    partial_messages: self.options.partial_messages,
                    resume,
                    settings: launch
                        .filter(|_| !external)
                        .and_then(|l| l.hook_settings.as_ref())
                        .map(|p| p.display().to_string()),
                    // The turn's run mode under the account's ceiling. Always
                    // set, so a turn never inherits whatever the profile's
                    // `settings.json` happens to say.
                    permission_mode: Some(if external {
                        "default"
                    } else {
                        claude_permission
                            .map_or_else(|| claude::permission_mode(account.sandbox), |p| p.flag)
                    }),
                    allowed_tools: if mcp.is_some() {
                        vec![format!(
                            "mcp__{}",
                            crate::core::assist::projection::SERVER_NAME
                        )]
                    } else {
                        Vec::new()
                    },
                    mcp_config: mcp,
                    tools: if external || side.is_some_and(|s| s.no_tools) {
                        Some(String::new())
                    } else if lean {
                        Some(claude::PROJECT_TOOLS.to_string())
                    } else {
                        restrict.then(|| claude::PROJECTLESS_TOOLS.to_string())
                    },
                    disallowed_tools: if external {
                        vec![claude::EXTERNAL_DENIED.to_string()]
                    } else if side.is_some() {
                        Vec::new()
                    } else if restrict {
                        vec![claude::PROJECTLESS_DENIED.to_string()]
                    } else {
                        Vec::new()
                    },
                    permission_prompt_tool: launch.and_then(|l| l.permission_prompt_tool.clone()),
                    restricted: external,
                    // The app's projection (if any) stays; the user's own
                    // MCP servers do not ride along on a project job.
                    strict_mcp: external || lean || side.is_some_and(|s| s.strict_mcp),
                    exclude_dynamic: lean,
                    effort: launch
                        .filter(|l| l.effort_flag)
                        .and_then(|_| {
                            req.params
                                .reasoning_effort
                                .as_deref()
                                .and_then(claude::effort_level)
                                .or(lean.then_some(claude::PROJECT_EFFORT))
                                .or(side.map(|_| claude::SIDE_EFFORT))
                        })
                        .map(String::from),
                    replace_system_prompt: side.is_some_and(|s| s.replace_system),
                    no_session_persistence: side.is_some_and(|s| s.no_persist),
                };
                let mut env = env;
                if side.is_some() {
                    // Haiku thinks even at `--effort low` (823 of 862 output
                    // tokens on a verdict, 2.1.284): off, the audit costs ~6x less.
                    for (k, v) in claude::SIDE_ENV {
                        env.insert(k.to_string(), v.to_string());
                    }
                }
                let spec = SpawnSpec {
                    program: binary,
                    args: claude::argv(&args),
                    env,
                    scrub,
                    cwd,
                    stdin: Some(stdin),
                    // Filled by `turn` from the watchdog settings: never part
                    // of the argv, so the plan (and the missions baseline)
                    // stays byte-identical.
                    ..SpawnSpec::default()
                };
                let parser = claude::ClaudeParser::new(self.options.partial_messages);
                let parser = match claude_permission {
                    Some(p) => parser.with_permission(
                        p.flag == "auto",
                        super::run_mode::denial_note(p.applied, has_prompt_tool),
                    ),
                    None => parser,
                };
                let parser = match launch.and_then(|l| l.baseline_request.clone()) {
                    Some(request) => parser.with_baselines(request),
                    None => parser,
                };
                (spec, Arc::new(parser))
            }
            CliKind::Codex if launch.is_some_and(|l| l.app_server.is_some()) => {
                let l = launch.expect("checked above");
                let turn = l.app_server.clone().expect("checked above");
                let resumed = l.resume.clone();
                let mut instructions = system_prompt.unwrap_or_default();
                let sys = claude::system_text(&req.messages);
                if !sys.trim().is_empty() {
                    if !instructions.is_empty() {
                        instructions.push_str("\n\n");
                    }
                    instructions.push_str(&sys);
                }
                let history: Vec<_> = req
                    .messages
                    .iter()
                    .filter(|m| m.role != super::types::Role::System)
                    .cloned()
                    .collect();
                let last = claude::last_user_text(&req.messages);
                let replay = history
                    .iter()
                    .any(|m| m.role == super::types::Role::Assistant);
                let plan = codex_app_server::AppServerPlan {
                    cwd: cwd.as_ref().map(|p| p.display().to_string()),
                    model: model_of(req),
                    effort: req
                        .params
                        .reasoning_effort
                        .as_deref()
                        .and_then(codex_app_server::effort_of)
                        .or(l.side.map(|_| claude::SIDE_EFFORT.to_string())),
                    instructions: (!instructions.trim().is_empty()).then_some(instructions),
                    resume: resumed,
                    resume_text: last.clone(),
                    fresh_text: if replay {
                        codex::render_prompt(&history)
                    } else {
                        last
                    },
                    // A side call only answers: read-only, whatever the account allows.
                    effective: Some(l.run_mode.unwrap_or_default().under(if l.side.is_some() {
                        accounts::SandboxMode::ReadOnly
                    } else {
                        account.sandbox
                    })),
                };
                let spec = SpawnSpec {
                    program: binary,
                    args: vec!["app-server".into()],
                    env,
                    scrub,
                    cwd,
                    stdin: Some(plan.prelude()),
                    ..SpawnSpec::default()
                };
                let asker = self.broker.clone().map(|broker| codex_app_server::Asker {
                    broker,
                    agent_id: turn.agent_id,
                    request_id: turn.request_id,
                    conversation: turn.conversation,
                    registry: crate::core::assist::runs::active(),
                });
                (
                    spec,
                    Arc::new(codex_app_server::AppServerParser::new(plan, asker)),
                )
            }
            CliKind::Codex => {
                // With a turn context the run mode decides the flags, capped
                // by the account; without one, the account's sandbox alone.
                let permission = launch.map(|l| {
                    super::run_mode::codex_flags(
                        l.run_mode.unwrap_or_default().under(account.sandbox),
                        l.approve_for_me,
                    )
                });
                let args = codex::CodexArgs {
                    model: model_of(req),
                    cwd: self.options.cwd.as_ref().map(|p| p.display().to_string()),
                    sandbox: account.sandbox.into(),
                    resume: None,
                    permission: permission
                        .as_ref()
                        .map(|(flags, _)| flags.clone())
                        .unwrap_or_default(),
                };
                let spec = SpawnSpec {
                    program: binary,
                    args: codex::argv(&args),
                    env,
                    scrub,
                    cwd,
                    stdin: Some(codex::render_prompt(&req.messages)),
                    // Filled by `turn` from the watchdog settings: never part
                    // of the argv, so the plan (and the missions baseline)
                    // stays byte-identical.
                    ..SpawnSpec::default()
                };
                let parser = match permission {
                    Some((_, applied)) => codex::CodexParser::new()
                        .with_denial_note(super::run_mode::denial_note(applied, false)),
                    None => codex::CodexParser::new(),
                };
                (spec, Arc::new(parser))
            }
        }
    }
}

/// The CLI takes a model alias, not a `provider/model` pair. An empty or
/// synthetic model name means "whatever the CLI is configured for".
fn model_of(req: &TurnRequest) -> Option<String> {
    let model = req.model.model.trim();
    if model.is_empty() || model == "default" {
        None
    } else {
        Some(model.to_string())
    }
}

#[async_trait]
impl AgentRuntime for CliRuntime {
    async fn turn(
        &self,
        agent: &AgentDef,
        req: TurnRequest,
    ) -> Result<BoxStream<'static, TurnEvent>, LlmError> {
        let RuntimeKind::Cli { cli, account } = &agent.runtime else {
            return Err(LlmError::new(
                ERR_CLI_ACCOUNT,
                format!("agent `{}` is not a CLI agent", agent.id),
            ));
        };
        let kind = CliKind::parse(cli)
            .ok_or_else(|| LlmError::new(ERR_CLI_NOT_FOUND, format!("unknown CLI `{cli}`")))?;
        let account = self.account(account)?;
        if account.cli != kind {
            return Err(LlmError::new(
                ERR_CLI_ACCOUNT,
                format!(
                    "account `{}` is a {} account, not {kind}",
                    account.id, account.cli
                ),
            ));
        }
        let binary = self.binary(kind).await?;
        let system_prompt = if agent.system_prompt.trim().is_empty() {
            None
        } else {
            Some(agent.system_prompt.clone())
        };

        let capacity = self.capacity.clone();
        let account_id = account.id.clone();
        let mut hooks = session::TurnHooks {
            rate: Some(Arc::new(move |snapshot: RateSnapshot| {
                capacity.record(&account_id, snapshot);
            })),
            ..Default::default()
        };

        // Inside a coordinator turn: pin the session, decide resume vs
        // replay, pick the folder, project the assistant tools.
        let turn = super::code_tools::current_turn();
        let registry = crate::core::assist::runs::active();
        let mut launch: Option<LaunchPlan> = None;
        let mut turn_hooks: Option<super::claude_hooks::TurnHooks> = None;
        if let Some(turn) = &turn {
            let caps = self.caps_of(kind, &binary).await;
            let external = crate::core::assist::authority::external(&turn.conversation);
            // External missions never run the CLI in the granted workspace:
            // its files are reached only through the projection's checked tools.
            let workspace = if external {
                None
            } else {
                super::code_tools::workspace_of(&turn.conversation)
            };
            let projectless = workspace.is_none();
            let cwd = workspace.or_else(|| {
                self.sandbox_dir(&if external {
                    format!("external-{}", agent.id)
                } else {
                    agent.id.clone()
                })
            });
            let has_history = req
                .messages
                .iter()
                .any(|m| m.role == super::types::Role::Assistant);
            let mut plan = LaunchPlan {
                cwd: cwd.clone(),
                projectless,
                resume: None,
                resume_kind: if has_history {
                    crate::core::assist::runs::ResumeKind::Replay
                } else {
                    crate::core::assist::runs::ResumeKind::New
                },
                mcp_config: None,
                permission_prompt_tool: None,
                restrict_tools: caps.has_flag("--tools") || caps.has_flag("--disallowedTools"),
                external,
                lean_project: kind == CliKind::Claude
                    && caps.has_flag("--tools")
                    && caps.has_flag("--strict-mcp-config")
                    && caps.has_flag("--exclude-dynamic-system-prompt-sections"),
                effort_flag: kind == CliKind::Claude && caps.has_flag("--effort"),
                run_mode: super::run_mode::of(&turn.conversation),
                auto_mode: caps.permission_modes.iter().any(|m| m == "auto"),
                approve_for_me: caps.has_flag("--approve-for-me"),
                hook_settings: None,
                baseline_request: Some(turn.request.clone()),
                app_server: (kind == CliKind::Codex && codex_app_server::wanted(caps.app_server))
                    .then(|| AppServerTurn {
                        agent_id: agent.id.clone(),
                        request_id: turn.request.clone(),
                        conversation: turn.conversation.clone(),
                    }),
                side: crate::core::llm::router::is_side_conversation(&turn.conversation).then(
                    || SideLaunch {
                        replace_system: caps.has_flag("--system-prompt"),
                        no_persist: caps.has_flag("--no-session-persistence"),
                        no_tools: caps.has_flag("--tools"),
                        strict_mcp: caps.has_flag("--strict-mcp-config"),
                    },
                ),
            };
            // Codex resumes its thread only over the app-server; `exec`
            // keeps replaying the transcript.
            let native_resume = match kind {
                CliKind::Claude => caps.native_resume,
                CliKind::Codex => plan.app_server.is_some(),
            };
            if let Some(reg) = &registry {
                let pins = crate::core::assist::runs::SessionPins {
                    runtime: format!("cli:{kind}"),
                    account: Some(format!("{}@{}", account.id, account.config_dir.display())),
                    exe_version: caps.version.clone(),
                    cwd: cwd.as_ref().map(|p| p.display().to_string()),
                    context_kind: if projectless {
                        "projectless"
                    } else {
                        "project"
                    }
                    .into(),
                    native_resume,
                };
                match reg.open_session(&turn.conversation, &agent.id, &pins) {
                    Ok((sess, repinned)) => {
                        // A named session resumes even with no reply in our
                        // history: the app died mid-turn (3.1), and the CLI's
                        // own transcript holds what the agent already did.
                        let resumable = has_history || sess.provider_handle.is_some();
                        if native_resume && !repinned && resumable {
                            if let Some(handle) = sess.provider_handle.clone() {
                                plan.resume = Some(handle);
                                plan.resume_kind = crate::core::assist::runs::ResumeKind::Native;
                            }
                        }
                        let _ = reg.bind_session(
                            &turn.request,
                            &sess.id,
                            plan.resume_kind,
                            plan.cwd
                                .as_ref()
                                .map(|p| p.display().to_string())
                                .as_deref(),
                        );
                        let (r, sid) = (reg.clone(), sess.id.clone());
                        hooks.session = Some(Arc::new(move |handle: String| {
                            let _ = r.set_handle(&sid, &handle);
                        }));
                        let (r, run) = (reg.clone(), turn.request.clone());
                        hooks.spawned = Some(Arc::new(move |pid, launch_id: &str| {
                            let _ = r.set_process(&run, launch_id, pid);
                        }));
                    }
                    Err(e) => tracing::warn!("[cli] session record: {e}"),
                }
            }
            // The coordinator issued a projection for this turn: hand it to
            // Claude Code through a private config file.
            if kind == CliKind::Claude && caps.has_flag("--mcp-config") && plan.side.is_none() {
                if let Some(p) = crate::core::assist::projection::current_turn() {
                    let dir = std::env::temp_dir().join("omniget-mcp");
                    match crate::core::assist::projection::write_mcp_config(&dir, &p.url, &p.token)
                    {
                        Ok(path) => {
                            if caps.interactive_permissions {
                                plan.permission_prompt_tool = Some(format!(
                                    "mcp__{}__{}",
                                    crate::core::assist::projection::SERVER_NAME,
                                    crate::core::assist::projection::PERMISSION_TOOL
                                ));
                            }
                            plan.mcp_config = Some(path);
                        }
                        Err(e) => tracing::warn!("[cli] projection config: {e}"),
                    }
                }
            }
            // OmniGet's rules also judge the CLI's own tools (3.4): a
            // `PreToolUse` hook per call, from a private per-turn folder. A
            // side call (3.7) has no tools to judge: its argv stays lean.
            if kind == CliKind::Claude
                && !external
                && plan.side.is_none()
                && caps.has_flag("--settings")
            {
                match super::claude_hooks::resolve_shim() {
                    Some(shim) => {
                        let policy = super::claude_hooks::Policy {
                            agent: agent.id.clone(),
                            rules_file: super::perm::store_file(),
                            rules: Vec::new(),
                            run_mode: plan.run_mode,
                            workspace: (!projectless).then(|| plan.cwd.clone()).flatten(),
                        };
                        match super::claude_hooks::prepare(
                            &super::claude_hooks::default_root(),
                            &shim,
                            &policy,
                        ) {
                            Ok(h) => {
                                plan.hook_settings = Some(h.settings.clone());
                                turn_hooks = Some(h);
                            }
                            Err(e) => tracing::warn!("[cli] permission hooks: {e}"),
                        }
                    }
                    None => {
                        // Once per process at `warn`: a release without the
                        // bundled sidecar must be visible in the log.
                        static SAID: std::sync::atomic::AtomicBool =
                            std::sync::atomic::AtomicBool::new(false);
                        if !SAID.swap(true, std::sync::atomic::Ordering::Relaxed) {
                            tracing::warn!(
                                "[cli] no hook binary (omniget-hook sidecar / omniget-hook-shim): Claude Code turns run without OmniGet's permission hooks; its permission mode alone judges its tools"
                            );
                        } else {
                            tracing::debug!("[cli] no hook binary: turn without permission hooks");
                        }
                    }
                }
            }
            launch = Some(plan);
        }

        let (mut spec, parser) =
            self.plan_with(&account, binary, &req, system_prompt, launch.as_ref());
        // Timing only: the argv the plan built is left exactly as it is.
        self.watchdog().apply(&mut spec);
        if let Some(turn) = &turn {
            spec.env.extend(conversation_env(&turn.conversation));
        }
        if let (Some(probe), Some(turn)) = (&self.pause_probe, &turn) {
            let (probe, request) = (probe.clone(), turn.request.clone());
            hooks.paused = Some(Arc::new(move || probe(&request)));
        }
        let mcp_config = launch.as_ref().and_then(|l| l.mcp_config.clone());
        if mcp_config.is_some() || turn_hooks.is_some() {
            hooks.cleanup = Some(Arc::new(move || {
                if let Some(path) = &mcp_config {
                    let _ = std::fs::remove_file(path);
                }
                if let Some(h) = &turn_hooks {
                    h.cleanup();
                }
            }));
        }
        let stream = session::spawn_turn_with(spec, parser, req.cancel.clone(), hooks)?;
        // A resumed handle the provider no longer knows: forget it, so the
        // next turn replays instead of failing the same way.
        let forget = match (&launch, &registry, &turn) {
            (Some(l), Some(reg), Some(t)) if l.resume.is_some() => reg
                .session(&t.conversation, &agent.id)
                .map(|s| (reg.clone(), s.id)),
            _ => None,
        };
        Ok(match forget {
            Some((reg, sid)) => {
                use futures::StreamExt;
                stream
                    .inspect(move |e| {
                        if let TurnEvent::Error { error } = e {
                            let m = error.message.to_ascii_lowercase();
                            if m.contains("no conversation found")
                                || m.contains("session") && m.contains("not found")
                            {
                                let _ = reg.drop_handle(&sid);
                            }
                        }
                    })
                    .boxed()
            }
            None => stream,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::llm::agent::{AgentRole, Budget, ModelPolicy};

    use crate::core::llm::types::{GenParams, Message, ModelRef, ProviderId, Role};
    #[cfg(unix)]
    use futures::StreamExt;
    use tokio_util::sync::CancellationToken;

    fn tmp(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("omniget-cli-rt-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn store_with(root: &std::path::Path, cli: CliKind) -> Arc<AccountStore> {
        let store = AccountStore::at(root.join("accounts.json"));
        store
            .create(CliAccount {
                id: "max-1".into(),
                cli,
                config_dir: root.join("max-1"),
                label: "Max #1".into(),
                disabled: false,
                sandbox: SandboxMode::default(),
            })
            .unwrap();
        Arc::new(store)
    }

    fn cli_agent(cli: &str, account: &str) -> AgentDef {
        AgentDef {
            id: "worker".into(),
            name: "Worker".into(),
            role: AgentRole::Worker,
            system_prompt: "be brief".into(),
            model: ModelPolicy::Fixed {
                model: ModelRef {
                    provider: ProviderId::new("cli"),
                    model: "sonnet".into(),
                },
            },
            tools: vec![],
            skills: vec![],
            budget: Budget::default(),
            runtime: RuntimeKind::Cli {
                cli: cli.into(),
                account: account.into(),
            },
            skin: None,
        }
    }

    fn request(model: &str) -> TurnRequest {
        TurnRequest {
            model: ModelRef {
                provider: ProviderId::new("cli"),
                model: model.into(),
            },
            messages: vec![Message::text(Role::User, "responda ok")],
            tools: vec![],
            params: GenParams::default(),
            cancel: CancellationToken::new(),
            agent_id: Some("worker".into()),
        }
    }

    /// A shell script that replays a fixture and exits 0. The whole runtime is
    /// exercised end to end with no CLI installed.
    #[cfg(unix)]
    fn fake_cli_replaying(name: &str, fixture_name: &str) -> PathBuf {
        let fixture_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("cli_fixtures")
            .join(fixture_name);
        let path =
            std::env::temp_dir().join(format!("omniget-replay-{name}-{}.sh", std::process::id()));
        crate::core::process::test_exec::script(
            &path,
            &format!("cat > /dev/null\ncat {}\n", fixture_path.display()),
        );
        path
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_replayed_fixture_runs_a_whole_turn_through_the_runtime() {
        let root = tmp("replay");
        let accounts = store_with(&root, CliKind::Claude);
        let runtime = CliRuntime::new(accounts, Arc::new(CliCapacity::new()));
        let fake = fake_cli_replaying("hello", "claude-2.1.276-hello.jsonl");
        runtime.set_binary(CliKind::Claude, fake.clone());
        let events: Vec<TurnEvent> = runtime
            .turn(&cli_agent("claude", "max-1"), request("sonnet"))
            .await
            .unwrap()
            .collect()
            .await;
        let text: String = events
            .iter()
            .filter_map(|e| match e {
                TurnEvent::TextDelta { text } => Some(text.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(text, "ok");
        assert!(events.iter().any(|e| matches!(e, TurnEvent::Usage { .. })));
        let _ = std::fs::remove_file(fake);
        let _ = std::fs::remove_dir_all(&root);
    }
}

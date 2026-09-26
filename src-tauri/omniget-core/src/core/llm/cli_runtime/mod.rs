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
pub use session::{LineParser, SpawnSpec};

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
/// Quota exhausted. Defined by the router (F2) because that is what reroutes
/// on it; re-exported here so this module has one error surface.
pub use super::router::ERR_CLI_RATE;

/// Reads a fixture from `omniget-core/tests/cli_fixtures/`. Test-only.
#[cfg(test)]
pub(crate) fn fixture(name: &str) -> String {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("cli_fixtures")
        .join(name);
    std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("fixture {} is missing: {e}", path.display()))
}

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

    /// Capacity of one account, for [`CapacitySource`]. An account we have
    /// never heard from is available with an unknown window: the router only
    /// rejects on evidence.
    pub fn capacity_of(&self, account_id: &str, disabled: bool) -> Capacity {
        self.capacity_of_cli(account_id, disabled, None)
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
}

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
        }
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

    pub fn with_options(mut self, options: CliRuntimeOptions) -> Self {
        self.options = options;
        self
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
                let resume = launch.and_then(|l| l.resume.clone());
                // Resumed: the provider holds the history; only this turn's
                // instructions/context and the new message travel.
                let (system_prompt, stdin) = match &resume {
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
                    && !restrict
                    && account.sandbox == accounts::SandboxMode::Write
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
                let args = claude::ClaudeArgs {
                    model: model_of(req),
                    system_prompt,
                    max_budget_usd: None,
                    partial_messages: self.options.partial_messages,
                    resume,
                    settings: None,
                    // Claude's half of the account's sandbox knob. Always set,
                    // so a turn never inherits whatever the profile's
                    // `settings.json` happens to say.
                    permission_mode: Some(if external {
                        "default"
                    } else {
                        claude::permission_mode(account.sandbox)
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
                    tools: if external {
                        Some(String::new())
                    } else if lean {
                        Some(claude::PROJECT_TOOLS.to_string())
                    } else {
                        restrict.then(|| claude::PROJECTLESS_TOOLS.to_string())
                    },
                    disallowed_tools: if external {
                        vec![claude::EXTERNAL_DENIED.to_string()]
                    } else if restrict {
                        vec![claude::PROJECTLESS_DENIED.to_string()]
                    } else {
                        Vec::new()
                    },
                    permission_prompt_tool: launch.and_then(|l| l.permission_prompt_tool.clone()),
                    restricted: external,
                    // The app's projection (if any) stays; the user's own
                    // MCP servers do not ride along on a project job.
                    strict_mcp: external || lean,
                    exclude_dynamic: lean,
                    effort: launch
                        .filter(|l| l.effort_flag)
                        .and_then(|_| {
                            req.params
                                .reasoning_effort
                                .as_deref()
                                .and_then(claude::effort_level)
                                .or(lean.then_some(claude::PROJECT_EFFORT))
                        })
                        .map(String::from),
                };
                let spec = SpawnSpec {
                    program: binary,
                    args: claude::argv(&args),
                    env,
                    scrub,
                    cwd,
                    stdin: Some(stdin),
                };
                (
                    spec,
                    Arc::new(claude::ClaudeParser::new(self.options.partial_messages)),
                )
            }
            CliKind::Codex => {
                let args = codex::CodexArgs {
                    model: model_of(req),
                    cwd: self.options.cwd.as_ref().map(|p| p.display().to_string()),
                    sandbox: account.sandbox.into(),
                    resume: None,
                };
                let spec = SpawnSpec {
                    program: binary,
                    args: codex::argv(&args),
                    env,
                    scrub,
                    cwd,
                    stdin: Some(codex::render_prompt(&req.messages)),
                };
                (spec, Arc::new(codex::CodexParser::new()))
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
                    native_resume: caps.native_resume && kind == CliKind::Claude,
                };
                match reg.open_session(&turn.conversation, &agent.id, &pins) {
                    Ok((sess, repinned)) => {
                        if kind == CliKind::Claude && caps.native_resume && !repinned && has_history
                        {
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
            if kind == CliKind::Claude && caps.has_flag("--mcp-config") {
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
            launch = Some(plan);
        }

        let (spec, parser) = self.plan_with(&account, binary, &req, system_prompt, launch.as_ref());
        if let Some(path) = launch.as_ref().and_then(|l| l.mcp_config.clone()) {
            hooks.cleanup = Some(Arc::new(move || {
                let _ = std::fs::remove_file(&path);
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
    use crate::core::llm::agent::{AgentRole, Budget, Candidate, CandidateRuntime, ModelPolicy};
    use crate::core::llm::router::ERR_CLI_RATE as ROUTER_ERR_CLI_RATE;
    #[cfg(unix)]
    use crate::core::llm::runtime::{CompositeRuntime, NativeRuntime};
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
        use std::io::Write;
        use std::os::unix::fs::PermissionsExt;
        let fixture_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("cli_fixtures")
            .join(fixture_name);
        let path =
            std::env::temp_dir().join(format!("omniget-replay-{name}-{}.sh", std::process::id()));
        let mut f = std::fs::File::create(&path).unwrap();
        writeln!(f, "#!/bin/sh").unwrap();
        writeln!(f, "cat > /dev/null").unwrap();
        writeln!(f, "cat {}", fixture_path.display()).unwrap();
        drop(f);
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path
    }

    #[test]
    fn every_error_code_is_distinct_and_prefixed() {
        let codes = [
            ERR_CLI_NOT_FOUND,
            ERR_CLI_SPAWN,
            ERR_CLI_EXIT,
            ERR_CLI_AUTH,
            ERR_CLI_ACCOUNT,
            ERR_CLI_IO,
            ERR_CLI_RATE,
        ];
        for code in codes {
            assert!(code.starts_with("ERR_CLI_"), "{code}");
        }
        let mut sorted = codes.to_vec();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), codes.len());
        // The rate code is the router's, not a copy.
        assert_eq!(ERR_CLI_RATE, ROUTER_ERR_CLI_RATE);
    }

    #[test]
    fn plan_builds_an_isolated_claude_launch() {
        let root = tmp("plan-claude");
        let accounts = store_with(&root, CliKind::Claude);
        let runtime = CliRuntime::new(accounts.clone(), Arc::new(CliCapacity::new()));
        let account = accounts.get("max-1").unwrap();
        let (spec, _) = runtime.plan(
            &account,
            PathBuf::from("/usr/bin/claude"),
            &request("sonnet"),
            Some("be brief".into()),
        );
        assert_eq!(
            spec.env.get("CLAUDE_CONFIG_DIR").map(String::as_str),
            Some(root.join("max-1").display().to_string().as_str())
        );
        assert!(spec.scrub.contains(&"ANTHROPIC_API_KEY".to_string()));
        assert!(spec.args.contains(&"stream-json".to_string()));
        assert_eq!(spec.stdin.as_deref(), Some("User: responda ok"));
        // The prompt never reaches argv.
        assert!(!spec.args.iter().any(|a| a.contains("responda ok")));
        let _ = std::fs::remove_dir_all(&root);
    }

    /// A project job on a write account gets the lean launch: built-in tools
    /// cut to the coding set, no user MCP servers, per-machine sections out of
    /// the system prompt, and the batching instruction after the agent's own.
    #[test]
    fn a_project_write_job_gets_the_lean_launch() {
        let root = tmp("plan-lean");
        let accounts = store_with(&root, CliKind::Claude);
        accounts.set_sandbox("max-1", SandboxMode::Write).unwrap();
        let runtime = CliRuntime::new(accounts.clone(), Arc::new(CliCapacity::new()));
        let account = accounts.get("max-1").unwrap();
        let plan = |lean: bool, projectless: bool| LaunchPlan {
            cwd: Some(root.clone()),
            projectless,
            resume: None,
            resume_kind: crate::core::assist::runs::ResumeKind::New,
            mcp_config: None,
            permission_prompt_tool: None,
            restrict_tools: projectless,
            external: false,
            lean_project: lean,
            effort_flag: false,
        };
        let (spec, _) = runtime.plan_with(
            &account,
            PathBuf::from("/usr/bin/claude"),
            &request("sonnet"),
            Some("be brief".into()),
            Some(&plan(true, false)),
        );
        let a = &spec.args;
        let after = |f: &str| {
            a.iter()
                .position(|x| x == f)
                .map(|i| a[i + 1].clone())
                .unwrap_or_else(|| panic!("{f} missing: {a:?}"))
        };
        assert_eq!(after("--tools"), claude::PROJECT_TOOLS);
        assert!(a.iter().any(|x| x == "--strict-mcp-config"));
        assert!(a
            .iter()
            .any(|x| x == "--exclude-dynamic-system-prompt-sections"));
        let sys = after("--append-system-prompt");
        assert!(sys.starts_with("be brief"), "{sys}");
        assert!(sys.contains(claude::PROJECT_BATCHING));

        // A CLI without the flags (caps said no) keeps the old launch.
        let (spec, _) = runtime.plan_with(
            &account,
            PathBuf::from("/usr/bin/claude"),
            &request("sonnet"),
            Some("be brief".into()),
            Some(&plan(false, false)),
        );
        let joined = spec.args.join(" ");
        assert!(!joined.contains("--strict-mcp-config"), "{joined}");
        assert!(!joined.contains("--tools"), "{joined}");
        assert!(!joined.contains(claude::PROJECT_BATCHING));
        // A read-only account never gets it either.
        accounts
            .set_sandbox("max-1", SandboxMode::ReadOnly)
            .unwrap();
        let account = accounts.get("max-1").unwrap();
        let (spec, _) = runtime.plan_with(
            &account,
            PathBuf::from("/usr/bin/claude"),
            &request("sonnet"),
            None,
            Some(&plan(true, false)),
        );
        assert!(!spec.args.join(" ").contains("--strict-mcp-config"));
        let _ = std::fs::remove_dir_all(&root);
    }

    /// Missions on 2.1.283 (3 reps, sonnet): `--effort low` on a lean project
    /// job cut the median time of the 11 missions from 152.8 s to 125.6 s at
    /// the same cost and 33/33 success. An explicit `reasoning_effort` wins;
    /// a CLI without `--effort` never gets it.
    #[test]
    fn a_lean_project_job_runs_at_low_effort_unless_the_turn_asks() {
        let root = tmp("plan-effort");
        let accounts = store_with(&root, CliKind::Claude);
        accounts.set_sandbox("max-1", SandboxMode::Write).unwrap();
        let runtime = CliRuntime::new(accounts.clone(), Arc::new(CliCapacity::new()));
        let account = accounts.get("max-1").unwrap();
        let plan = |lean: bool, effort_flag: bool| LaunchPlan {
            cwd: Some(root.clone()),
            projectless: false,
            resume: None,
            resume_kind: crate::core::assist::runs::ResumeKind::New,
            mcp_config: None,
            permission_prompt_tool: None,
            restrict_tools: false,
            external: false,
            lean_project: lean,
            effort_flag,
        };
        let effort = |req: &TurnRequest, p: &LaunchPlan| {
            let (spec, _) = runtime.plan_with(
                &account,
                PathBuf::from("/usr/bin/claude"),
                req,
                None,
                Some(p),
            );
            spec.args
                .iter()
                .position(|x| x == "--effort")
                .map(|i| spec.args[i + 1].clone())
        };
        assert_eq!(
            effort(&request("sonnet"), &plan(true, true)).as_deref(),
            Some(claude::PROJECT_EFFORT)
        );
        let mut asked = request("sonnet");
        asked.params.reasoning_effort = Some("High".into());
        assert_eq!(effort(&asked, &plan(true, true)).as_deref(), Some("high"));
        asked.params.reasoning_effort = Some("minimal".into());
        assert_eq!(effort(&asked, &plan(true, true)).as_deref(), Some("low"));
        // Not lean: only an explicit ask sets it.
        assert_eq!(effort(&request("sonnet"), &plan(false, true)), None);
        asked.params.reasoning_effort = Some("medium".into());
        assert_eq!(
            effort(&asked, &plan(false, true)).as_deref(),
            Some("medium")
        );
        // A CLI that does not list `--effort` never gets it.
        assert_eq!(effort(&asked, &plan(true, false)), None);
        assert_eq!(effort(&request("sonnet"), &plan(true, false)), None);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn plan_builds_a_codex_launch_with_codex_home() {
        let root = tmp("plan-codex");
        let accounts = store_with(&root, CliKind::Codex);
        let runtime = CliRuntime::new(accounts.clone(), Arc::new(CliCapacity::new()));
        let account = accounts.get("max-1").unwrap();
        let (spec, _) = runtime.plan(
            &account,
            PathBuf::from("/usr/bin/codex"),
            &request("gpt-5-codex"),
            None,
        );
        assert!(spec.env.contains_key("CODEX_HOME"));
        assert!(!spec.env.contains_key("CLAUDE_CONFIG_DIR"));
        assert_eq!(spec.args.first().map(String::as_str), Some("exec"));
        let _ = std::fs::remove_dir_all(&root);
    }

    /// The default is never a write, for either CLI, and the user's choice is
    /// what the child actually gets.
    #[test]
    fn the_default_sandbox_is_read_only_for_both_clis() {
        for (cli, read_only, write) in [
            (CliKind::Claude, "--permission-mode plan", "acceptEdits"),
            (CliKind::Codex, "--sandbox read-only", "workspace-write"),
        ] {
            let root = tmp(&format!("sandbox-{cli}"));
            let accounts = store_with(&root, cli);
            let runtime = CliRuntime::new(accounts.clone(), Arc::new(CliCapacity::new()));
            let account = accounts.get("max-1").unwrap();
            assert_eq!(account.sandbox, SandboxMode::ReadOnly, "{cli}");
            let (spec, _) = runtime.plan(
                &account,
                PathBuf::from("/usr/bin/x"),
                &request("m"),
                Some("be brief".into()),
            );
            assert!(
                spec.args.join(" ").contains(read_only),
                "{cli}: {:?}",
                spec.args
            );

            accounts.set_sandbox("max-1", SandboxMode::Write).unwrap();
            let account = accounts.get("max-1").unwrap();
            let (spec, _) = runtime.plan(
                &account,
                PathBuf::from("/usr/bin/x"),
                &request("m"),
                Some("be brief".into()),
            );
            let joined = spec.args.join(" ");
            assert!(joined.contains(write), "{cli}: {joined}");
            // Neither CLI's "no rules at all" mode is reachable from an account.
            assert!(!joined.contains("danger-full-access"), "{cli}: {joined}");
            assert!(!joined.contains("bypassPermissions"), "{cli}: {joined}");
            assert!(!joined.contains("dangerously"), "{cli}: {joined}");
            let _ = std::fs::remove_dir_all(&root);
        }
    }

    #[test]
    fn an_empty_model_lets_the_cli_decide() {
        assert_eq!(model_of(&request("")), None);
        assert_eq!(model_of(&request("default")), None);
        assert_eq!(model_of(&request("opus")).as_deref(), Some("opus"));
    }

    #[tokio::test]
    async fn a_missing_account_is_err_cli_account() {
        let root = tmp("missing");
        let accounts = store_with(&root, CliKind::Claude);
        let runtime = CliRuntime::new(accounts, Arc::new(CliCapacity::new()));
        let err = runtime
            .turn(&cli_agent("claude", "nope"), request("sonnet"))
            .await
            .err()
            .unwrap();
        assert_eq!(err.code, ERR_CLI_ACCOUNT);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn a_disabled_account_never_spawns() {
        let root = tmp("disabled");
        let accounts = store_with(&root, CliKind::Claude);
        accounts.set_disabled("max-1", true).unwrap();
        let runtime = CliRuntime::new(accounts, Arc::new(CliCapacity::new()));
        let err = runtime
            .turn(&cli_agent("claude", "max-1"), request("sonnet"))
            .await
            .err()
            .unwrap();
        assert_eq!(err.code, ERR_CLI_ACCOUNT);
        assert!(err.message.contains("disabled"));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn an_account_of_the_other_cli_is_refused() {
        let root = tmp("mismatch");
        let accounts = store_with(&root, CliKind::Codex);
        let runtime = CliRuntime::new(accounts, Arc::new(CliCapacity::new()));
        let err = runtime
            .turn(&cli_agent("claude", "max-1"), request("sonnet"))
            .await
            .err()
            .unwrap();
        assert_eq!(err.code, ERR_CLI_ACCOUNT);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn an_unknown_cli_name_is_err_cli_not_found() {
        let root = tmp("unknown-cli");
        let accounts = store_with(&root, CliKind::Claude);
        let runtime = CliRuntime::new(accounts, Arc::new(CliCapacity::new()));
        let err = runtime
            .turn(&cli_agent("gemini", "max-1"), request("sonnet"))
            .await
            .err()
            .unwrap();
        assert_eq!(err.code, ERR_CLI_NOT_FOUND);
        let _ = std::fs::remove_dir_all(&root);
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

    #[cfg(unix)]
    #[tokio::test]
    async fn a_rate_limited_turn_fills_the_capacity_of_that_account() {
        let root = tmp("capacity");
        let accounts = store_with(&root, CliKind::Claude);
        let capacity = Arc::new(CliCapacity::new());
        let runtime = CliRuntime::new(accounts.clone(), capacity.clone());
        let fake = fake_cli_replaying("rate", "claude-2.1.276-rate-limit.jsonl");
        runtime.set_binary(CliKind::Claude, fake.clone());

        // Before the turn the router knows nothing and lets it through.
        let source = CliCapacitySource::new(capacity.clone(), accounts.clone());
        let candidate = Candidate {
            runtime: CandidateRuntime::Cli {
                account_id: "max-1".into(),
            },
            model: "opus".into(),
            max_cost_per_1k: None,
            min_context: 0,
        };
        assert_eq!(source.capacity(&candidate).quota_remaining, None);

        let events: Vec<TurnEvent> = runtime
            .turn(&cli_agent("claude", "max-1"), request("opus"))
            .await
            .unwrap()
            .collect()
            .await;
        let error = events
            .iter()
            .find_map(|e| match e {
                TurnEvent::Error { error } => Some(error.clone()),
                _ => None,
            })
            .expect("the rejected window must surface");
        assert_eq!(error.code, ERR_CLI_RATE);

        // After it, the router sees an exhausted account.
        assert_eq!(source.capacity(&candidate).quota_remaining, Some(0.0));
        // And a disabled account is simply unavailable.
        accounts.set_disabled("max-1", true).unwrap();
        assert!(!source.capacity(&candidate).available);
        let _ = std::fs::remove_file(fake);
        let _ = std::fs::remove_dir_all(&root);
    }

    /// The router's Codex preference is fed from here and from nowhere else:
    /// a Claude account is never a code specialist, a Codex one always is, and
    /// a disabled account is simply unavailable.
    #[test]
    fn only_a_codex_account_is_flagged_as_a_code_specialist() {
        use crate::core::llm::router::TaskKind;
        let root = tmp("code-specialist");
        let store = AccountStore::at(root.join("accounts.json"));
        for (id, cli) in [("claude-1", CliKind::Claude), ("codex-1", CliKind::Codex)] {
            store
                .create(CliAccount {
                    id: id.into(),
                    cli,
                    config_dir: root.join(id),
                    label: id.into(),
                    disabled: false,
                    sandbox: SandboxMode::default(),
                })
                .unwrap();
        }
        let accounts = Arc::new(store);
        let source = CliCapacitySource::new(Arc::new(CliCapacity::new()), accounts.clone());
        let candidate = |id: &str| Candidate {
            runtime: CandidateRuntime::Cli {
                account_id: id.into(),
            },
            model: "m".into(),
            max_cost_per_1k: None,
            min_context: 0,
        };
        assert!(!source.capacity(&candidate("claude-1")).code_specialist);
        assert!(source.capacity(&candidate("codex-1")).code_specialist);

        // End to end: a code turn over this source picks the Codex account.
        let chain = vec![candidate("claude-1"), candidate("codex-1")];
        let route = crate::core::llm::router::select_for(
            &chain,
            &source,
            &Default::default(),
            &Default::default(),
            std::time::Instant::now(),
            &[],
            TaskKind::Code,
        )
        .unwrap();
        assert_eq!(route.key, "cli:codex-1:m");
        assert!(route.preferred);

        // Disabled: unavailable beats any preference.
        accounts.set_disabled("codex-1", true).unwrap();
        assert!(!source.capacity(&candidate("codex-1")).available);
        let route = crate::core::llm::router::select_for(
            &chain,
            &source,
            &Default::default(),
            &Default::default(),
            std::time::Instant::now(),
            &[],
            TaskKind::Code,
        )
        .unwrap();
        assert_eq!(route.key, "cli:claude-1:m");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn capacity_of_an_unknown_account_is_unavailable() {
        let root = tmp("cap-unknown");
        let accounts = store_with(&root, CliKind::Claude);
        let source = CliCapacitySource::new(Arc::new(CliCapacity::new()), accounts);
        let candidate = Candidate {
            runtime: CandidateRuntime::Cli {
                account_id: "ghost".into(),
            },
            model: "opus".into(),
            max_cost_per_1k: None,
            min_context: 0,
        };
        assert!(!source.capacity(&candidate).available);
        // A native candidate is not this source's business.
        let native = Candidate {
            runtime: CandidateRuntime::Native {
                provider: ProviderId::new("openai"),
            },
            model: "gpt-4o-mini".into(),
            max_cost_per_1k: None,
            min_context: 0,
        };
        assert!(source.capacity(&native).available);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn the_composite_runtime_routes_cli_agents_here() {
        let root = tmp("composite");
        let accounts = store_with(&root, CliKind::Claude);
        let cli = CliRuntime::new(accounts, Arc::new(CliCapacity::new()));
        let fake = fake_cli_replaying("composite", "claude-2.1.276-hello.jsonl");
        cli.set_binary(CliKind::Claude, fake.clone());

        let composite =
            CompositeRuntime::new(Arc::new(NativeRuntime::new())).with_cli(Arc::new(cli));
        assert!(composite.has_cli());
        let events: Vec<TurnEvent> = composite
            .turn(&cli_agent("claude", "max-1"), request("sonnet"))
            .await
            .unwrap()
            .collect()
            .await;
        assert!(events.iter().any(|e| matches!(
            e,
            TurnEvent::TextDelta { text } if text == "ok"
        )));
        let _ = std::fs::remove_file(fake);
        let _ = std::fs::remove_dir_all(&root);
    }

    /// Runs the real `claude` in a throwaway config dir. Opt in with
    /// `OMNIGET_TEST_CLAUDE=1 cargo test -p omniget-core --features desktop
    /// cli_runtime:: -- --ignored --nocapture`. It costs a real turn on
    /// whatever account the user logs into that dir, so it never runs by
    /// default and never touches `~/.claude`.
    #[cfg(unix)]
    #[tokio::test]
    #[ignore = "spawns the installed claude; set OMNIGET_TEST_CLAUDE=1"]
    async fn live_claude_turn_in_a_throwaway_config_dir() {
        if std::env::var("OMNIGET_TEST_CLAUDE").ok().as_deref() != Some("1") {
            eprintln!("set OMNIGET_TEST_CLAUDE=1 to run this");
            return;
        }
        let root = tmp("live");
        let accounts = store_with(&root, CliKind::Claude);
        let capacity = Arc::new(CliCapacity::new());
        let runtime = CliRuntime::new(accounts, capacity.clone());
        let events: Vec<TurnEvent> = runtime
            .turn(&cli_agent("claude", "max-1"), request("sonnet"))
            .await
            .expect("claude must be installed for this test")
            .collect()
            .await;
        eprintln!("live events: {events:#?}");
        eprintln!("live capacity: {:?}", capacity.get("max-1"));
        assert!(!events.is_empty());
        // An empty config dir has no login, so the honest outcome is
        // ERR_CLI_AUTH until the user runs /login in that directory.
        let _ = std::fs::remove_dir_all(&root);
    }
}

#[cfg(test)]
mod durable_tests;

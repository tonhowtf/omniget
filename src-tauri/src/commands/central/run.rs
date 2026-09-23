//! Comandos `run_*` da Central (plano F9): o catálogo em execução. Loops do
//! catálogo no motor de Loops (`crate::jobs`), workflows como playbooks de
//! jobs encadeados, wizard de projeto, "Rodar agora" com o runner escolhido e
//! a receita de sandbox (Docker próprio / E2B). A lógica pura mora em
//! `omniget_core::core::agentkit_run`; aqui fica a costura com o roster, as
//! contas de CLI e os jobs.

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tauri::{AppHandle, Manager};

use omniget_core::core::agentkit::{self as ak, plan::PlanRequest, Scope};
use omniget_core::core::agentkit_run::{
    agent::{self as ra, AgentPrompt},
    playbook::{self as pb, Playbook, RunnerSpec},
    runner, sandbox, stack, wizard,
};

use crate::jobs::{self, LoopDef, PlaybookRun, PlaybookStepRun, RunSpec, TOOL_PREFIX};

const ERR: &str = "ERR_CENTRAL_RUN";

fn to_value<T: Serialize>(v: T) -> Result<Value, String> {
    serde_json::to_value(v).map_err(|e| format!("{ERR}: {e}"))
}

async fn blocking<T, F>(f: F) -> Result<T, String>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, String> + Send + 'static,
{
    tokio::task::spawn_blocking(f)
        .await
        .map_err(|e| format!("{ERR}: {e}"))?
}

// ------------------------------------------------------------------ runners

/// A runner resolved to what the Jobs engine takes.
#[derive(Debug, Clone)]
pub struct Resolved {
    /// Roster id, or `tool:<id>`.
    pub agent_id: String,
    pub label: String,
    pub model: Option<String>,
    pub permission: Option<String>,
}

impl Resolved {
    pub fn is_tool(&self) -> bool {
        self.agent_id.starts_with(TOOL_PREFIX)
    }

    /// The prompt and spec of a job with `role` as the agent: a tool runner
    /// gets it as its system prompt, a roster agent in front of the prompt.
    pub fn job(
        &self,
        role: Option<&AgentPrompt>,
        prompt: &str,
        sandbox: Option<sandbox::SandboxOpts>,
    ) -> (String, Option<Value>) {
        if self.is_tool() {
            let spec = RunSpec {
                system_prompt: role
                    .map(|r| r.system_prompt.clone())
                    .filter(|s| !s.is_empty()),
                agent_name: role.map(|r| r.name.clone()),
                model: self
                    .model
                    .clone()
                    .or_else(|| role.and_then(|r| r.model.clone())),
                permission: self.permission.clone(),
                sandbox,
            };
            (prompt.to_string(), serde_json::to_value(spec).ok())
        } else {
            (
                runner::prompt_with_role(role.map(|r| r.system_prompt.as_str()), prompt),
                None,
            )
        }
    }
}

fn llm(app: &AppHandle) -> std::sync::Arc<crate::llm_manager::LlmManager> {
    app.state::<crate::AppState>().llm.clone()
}

fn slug(s: &str) -> String {
    let out: String = s
        .to_ascii_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    out.trim_matches('-').to_string()
}

/// Id of the account that uses the CLI's default profile (terminal login),
/// created as `<cli>-terminal` (writes allowed unless `plan`) the first time.
fn terminal_account(
    llm: &crate::llm_manager::LlmManager,
    cli: &str,
    permission: Option<&str>,
) -> Result<String, String> {
    use omniget_core::core::llm::cli_runtime::{CliAccount, CliKind, SandboxMode};
    let kind = CliKind::parse(cli).ok_or_else(|| format!("{ERR}: unknown CLI `{cli}`"))?;
    let store = llm.accounts();
    if let Some(a) = store
        .list()
        .iter()
        .find(|a| a.cli == kind && !a.disabled && a.config_dir.as_os_str().is_empty())
    {
        return Ok(a.id.clone());
    }
    let id = format!("{cli}-terminal");
    if store.get(&id).is_some() {
        return Ok(id);
    }
    store
        .create(CliAccount {
            id: id.clone(),
            cli: kind,
            config_dir: PathBuf::new(),
            label: "Terminal login".into(),
            disabled: false,
            sandbox: if permission == Some("plan") {
                SandboxMode::ReadOnly
            } else {
                SandboxMode::Write
            },
        })
        .map_err(|e| format!("{ERR}: {}", e.message))?;
    Ok(id)
}

/// Runner → roster agent id (creating the agent for an account or an ACP
/// agent the first time) or `tool:<id>`.
pub fn resolve_runner(app: &AppHandle, r: &RunnerSpec) -> Result<Resolved, String> {
    use omniget_core::core::llm::agent::{AgentDef, AgentRole, ModelPolicy, RuntimeKind};
    use omniget_core::core::llm::types::{ModelRef, ProviderId};
    let llm = llm(app);
    let label = r.label();
    let base = Resolved {
        agent_id: String::new(),
        label: label.clone(),
        model: r.model.clone().filter(|m| !m.trim().is_empty()),
        permission: r.permission.clone().filter(|m| !m.trim().is_empty()),
    };
    match r.kind.as_str() {
        "tool" => {
            runner::runner_of(&r.id)?;
            Ok(Resolved {
                agent_id: format!("{TOOL_PREFIX}{}", r.id),
                ..base
            })
        }
        "agent" | "native" | "" => {
            let id = if r.id.is_empty() {
                "omni"
            } else {
                r.id.as_str()
            };
            if llm.agent(id).is_none() {
                return Err(format!("{ERR}: no agent `{id}` in the roster"));
            }
            Ok(Resolved {
                agent_id: id.to_string(),
                ..base
            })
        }
        "account" => {
            let cli = r.id.clone();
            if cli != "claude" && cli != "codex" {
                return Err(format!(
                    "{ERR}: accounts exist for claude and codex, not `{cli}`"
                ));
            }
            let mut account = r.account.clone().unwrap_or_default();
            if account.is_empty() {
                // The terminal's own login = an account with an empty config
                // dir (the CLI's default profile); reuse one or create it.
                account = terminal_account(&llm, &cli, r.permission.as_deref())?;
            }
            if llm.accounts().get(&account).is_none() {
                return Err(format!(
                    "{ERR}: no {cli} account `{account}` (LLM → Accounts)"
                ));
            }
            let model = base.model.clone().unwrap_or_else(|| "default".into());
            if let Some(a) = llm.roster().into_iter().find(|a| {
                matches!(&a.runtime, RuntimeKind::Cli { cli: c, account: acc } if c == &cli && acc == &account)
                    && matches!(&a.model, ModelPolicy::Fixed { model: m } if m.model == model)
            }) {
                return Ok(Resolved { agent_id: a.id, ..base });
            }
            let id = slug(&format!(
                "run-{cli}-{}-{model}",
                if account.is_empty() {
                    "default"
                } else {
                    &account
                }
            ));
            let agent = AgentDef {
                id: id.clone(),
                name: format!(
                    "{} · {}",
                    if cli == "claude" {
                        "Claude Code"
                    } else {
                        "Codex"
                    },
                    if account.is_empty() {
                        "default"
                    } else {
                        &account
                    }
                ),
                role: AgentRole::Worker,
                system_prompt: String::new(),
                model: ModelPolicy::Fixed {
                    model: ModelRef {
                        provider: ProviderId::new(cli.clone()),
                        model,
                    },
                },
                tools: Vec::new(),
                skills: Vec::new(),
                budget: Default::default(),
                runtime: RuntimeKind::Cli { cli, account },
                skin: None,
            };
            if llm.agent(&id).is_none() {
                llm.roster_create(agent)?;
            }
            Ok(Resolved {
                agent_id: id,
                ..base
            })
        }
        "acp" => {
            use omniget_core::core::llm::drivers::acp::agents;
            let spec = agents::spec(&r.id)
                .ok_or_else(|| format!("{ERR}: unknown ACP agent `{}`", r.id))?;
            let launch = agents::resolve(spec).ok_or_else(|| {
                format!(
                    "{ERR}: {} is not installed here (Central → Tools installs it)",
                    spec.name
                )
            })?;
            if let Some(a) = llm.roster().into_iter().find(|a| {
                matches!(&a.runtime, RuntimeKind::Acp { command, args } if command == &launch.command && args == &launch.args)
            }) {
                return Ok(Resolved { agent_id: a.id, ..base });
            }
            let id = slug(&format!("run-acp-{}", r.id));
            let agent = AgentDef {
                id: id.clone(),
                name: format!("{} (ACP)", spec.name),
                role: AgentRole::Worker,
                system_prompt: String::new(),
                model: ModelPolicy::Fixed {
                    model: ModelRef {
                        provider: ProviderId::new("acp"),
                        model: launch.command.clone(),
                    },
                },
                tools: Vec::new(),
                skills: Vec::new(),
                budget: Default::default(),
                runtime: RuntimeKind::Acp {
                    command: launch.command.clone(),
                    args: launch.args.clone(),
                },
                skin: None,
            };
            if llm.agent(&id).is_none() {
                llm.roster_create(agent)?;
            }
            Ok(Resolved {
                agent_id: id,
                ..base
            })
        }
        other => Err(format!(
            "{ERR}: unknown runner kind `{other}` (agent, account, acp, tool)"
        )),
    }
}

/// Every runner this machine offers: roster agents, CLI accounts, ACP agents
/// that resolve, and coding CLIs that run headless.
#[tauri::command]
pub async fn run_runners(app: AppHandle) -> Result<Value, String> {
    use omniget_core::core::llm::agent::RuntimeKind;
    let llm = llm(&app);
    let roster: Vec<Value> = llm
        .roster()
        .into_iter()
        .map(|a| {
            let runtime = match &a.runtime {
                RuntimeKind::Native => "native".to_string(),
                RuntimeKind::Cli { cli, .. } => format!("cli:{cli}"),
                RuntimeKind::Acp { .. } => "acp".to_string(),
            };
            json!({ "id": a.id, "name": a.name, "runtime": runtime })
        })
        .collect();
    let accounts: Vec<Value> = llm
        .accounts()
        .list()
        .iter()
        .map(
            |a| json!({ "id": a.id, "cli": a.cli.bin(), "label": a.label, "disabled": a.disabled }),
        )
        .collect();
    let acp = tokio::task::spawn_blocking(|| {
        use omniget_core::core::llm::drivers::acp::agents;
        agents::AGENTS
            .iter()
            .map(|s| {
                let l = agents::resolve(s);
                json!({ "id": s.id, "name": s.name, "available": l.is_some(), "command": l.map(|l| l.command) })
            })
            .collect::<Vec<Value>>()
    })
    .await
    .map_err(|e| e.to_string())?;
    let tools = tokio::task::spawn_blocking(runner::available_tools)
        .await
        .map_err(|e| e.to_string())?;
    Ok(json!({ "roster": roster, "accounts": accounts, "acp": acp, "tools": tools }))
}

/// Resolves an agent reference: catalog id (`cct:agents/…`), `path:agent:<file>`,
/// an `.md` file path, or empty (no role). Runs on a blocking thread.
pub fn agent_role(reference: &str) -> Result<Option<AgentPrompt>, String> {
    let r = reference.trim();
    if r.is_empty() {
        return Ok(None);
    }
    let as_path = PathBuf::from(r);
    if r.ends_with(".md") && as_path.is_file() {
        return ra::agent_from_file(&as_path).map(Some);
    }
    let c = super::agentkit::resolve_any(r)?;
    if let Some(body) = ra::command_prompt(&c, "") {
        return Ok(Some(AgentPrompt {
            id: c.id.clone(),
            name: c.name.clone(),
            system_prompt: body,
            ..Default::default()
        }));
    }
    ra::agent_prompt(&c).map(Some)
}

// ------------------------------------------------------------------ loops

/// Loop do catálogo → plano do Loop + plano de instalação dos componentes que
/// ele referencia (o próprio runbook incluso) nas ferramentas escolhidas.
#[tauri::command]
pub async fn run_loop_prepare(
    item_id: String,
    runner_spec: RunnerSpec,
    workspace: Option<String>,
    targets: Option<Vec<String>>,
    scope: Option<String>,
) -> Result<Value, String> {
    blocking(move || {
        let c = super::agentkit::resolve_any(&item_id)?;
        let lp =
            ak::convert::loop_runbook::loop_plan(&c, &runner_spec.label(), workspace.as_deref())?;
        let targets = targets.unwrap_or_default();
        let project = workspace
            .clone()
            .filter(|w| !w.trim().is_empty())
            .map(PathBuf::from);
        let plan = if targets.is_empty() {
            None
        } else {
            let env = super::agentkit::env()?;
            let scope = match scope.as_deref() {
                Some(s) => Some(Scope::parse(s)?),
                None => Some(if project.is_some() {
                    Scope::Project
                } else {
                    Scope::Global
                }),
            };
            Some(ak::plan::plan(
                &env,
                PlanRequest {
                    components: vec![c],
                    targets,
                    scope,
                    project_dir: project,
                    policy: ak::plan::ConflictPolicy::Rename,
                    secret_values: Default::default(),
                },
            )?)
        };
        Ok(json!({ "loop": to_value(lp)?, "plan": to_value(plan)? }))
    })
    .await
}

/// For the bridge / CLI (`omniget agent loop --catalog <id>`): a catalog loop
/// straight to a running Loop, with optional overrides and install targets.
#[derive(Debug, Clone, Deserialize)]
pub struct CatalogLoopRequest {
    pub id: String,
    pub runner: RunnerSpec,
    #[serde(default)]
    pub workspace: Option<String>,
    /// Tools to install the loop's referenced components into (project scope).
    #[serde(default)]
    pub targets: Vec<String>,
    #[serde(default)]
    pub schedule: Option<String>,
    #[serde(default)]
    pub check_command: Option<String>,
    #[serde(default)]
    pub max_rounds: Option<u32>,
    #[serde(default)]
    pub max_minutes: Option<u32>,
    #[serde(default)]
    pub max_cost_usd: Option<f64>,
    /// Keep the runbook's interval (default: rounds back to back).
    #[serde(default)]
    pub use_interval: bool,
}

/// Prepares (loop plan + install plan applied) and starts a catalog loop.
pub async fn start_catalog_loop(app: &AppHandle, r: CatalogLoopRequest) -> Result<LoopDef, String> {
    let prepared = run_loop_prepare(
        r.id.clone(),
        r.runner.clone(),
        r.workspace.clone(),
        Some(r.targets.clone()),
        None,
    )
    .await?;
    let lp: ak::convert::loop_runbook::LoopPlan =
        serde_json::from_value(prepared["loop"].clone()).map_err(|e| format!("{ERR}: {e}"))?;
    let plan_id = prepared["plan"]["id"].as_str().map(str::to_string);
    let schedule = r.schedule.clone().or_else(|| {
        r.use_interval
            .then(|| lp.interval.clone())
            .flatten()
            .filter(|i| lp.interval_secs.is_some() && !i.is_empty())
    });
    start_loop(
        app,
        LoopStart {
            runner: r.runner,
            name: lp.name.clone(),
            prompt: lp.prompt.clone(),
            workspace: r.workspace,
            schedule,
            check_command: r.check_command.or(lp.check_command.clone()),
            max_rounds: r.max_rounds.or(Some(lp.max_rounds)),
            max_minutes: r.max_minutes.or(Some(lp.max_minutes)),
            max_cost_usd: r.max_cost_usd,
            source: Some(r.id),
            agent: None,
            plan_id,
            sandbox: None,
        },
    )
    .await
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LoopStart {
    pub runner: RunnerSpec,
    pub name: String,
    pub prompt: String,
    #[serde(default)]
    pub workspace: Option<String>,
    #[serde(default)]
    pub schedule: Option<String>,
    #[serde(default)]
    pub check_command: Option<String>,
    #[serde(default)]
    pub max_rounds: Option<u32>,
    #[serde(default)]
    pub max_minutes: Option<u32>,
    #[serde(default)]
    pub max_cost_usd: Option<f64>,
    /// Catalog id of the loop.
    #[serde(default)]
    pub source: Option<String>,
    /// Agent used as the role of every round (catalog id / file).
    #[serde(default)]
    pub agent: Option<String>,
    /// Install plan (from `run_loop_prepare`) applied before the first round.
    #[serde(default)]
    pub plan_id: Option<String>,
    #[serde(default)]
    pub sandbox: Option<sandbox::SandboxOpts>,
}

/// Creates the Loop (after applying the install plan, when given).
pub async fn start_loop(app: &AppHandle, s: LoopStart) -> Result<LoopDef, String> {
    if let Some(plan_id) = s.plan_id.clone().filter(|p| !p.is_empty()) {
        blocking(move || {
            let plan = ak::plan::get(&plan_id)
                .ok_or_else(|| format!("{ERR}: install plan `{plan_id}` expired, prepare again"))?;
            let env = super::agentkit::env()?;
            ak::writer::apply(&env, &plan).map_err(String::from)?;
            Ok(())
        })
        .await?;
    }
    let resolved = resolve_runner(app, &s.runner)?;
    let agent = s.agent.clone().unwrap_or_default();
    let role = blocking(move || agent_role(&agent)).await?;
    let (prompt, spec) = resolved.job(role.as_ref(), &s.prompt, s.sandbox.clone());
    let def = LoopDef {
        name: s.name.clone(),
        agent_id: resolved.agent_id.clone(),
        prompt,
        workspace: s.workspace.clone().filter(|w| !w.trim().is_empty()),
        max_rounds: s.max_rounds.filter(|n| *n > 0),
        max_minutes: s.max_minutes.filter(|n| *n > 0),
        check_command: s.check_command.clone().filter(|c| !c.trim().is_empty()),
        schedule: s
            .schedule
            .clone()
            .filter(|c| !c.trim().is_empty() && c.trim() != "on-demand"),
        max_cost_usd: s.max_cost_usd.filter(|c| *c > 0.0),
        spec,
        source: s.source.clone(),
        ..Default::default()
    };
    jobs::get(app)?.loop_create(def)
}

#[tauri::command]
pub async fn run_loop_start(app: AppHandle, start: LoopStart) -> Result<Value, String> {
    to_value(start_loop(&app, start).await?)
}

// ------------------------------------------------------------------ run now

/// "Rodar agora": um Job com o agente (ou comando) como papel e o runner
/// escolhido; com `sandbox`, roda num container sobre uma cópia do projeto.
#[tauri::command]
pub async fn run_agent_now(
    app: AppHandle,
    agent: Option<String>,
    runner_spec: RunnerSpec,
    prompt: String,
    workspace: Option<String>,
    sandbox: Option<sandbox::SandboxOpts>,
) -> Result<Value, String> {
    let resolved = resolve_runner(&app, &runner_spec)?;
    if sandbox.is_some() && !resolved.is_tool() {
        return Err(format!(
            "{ERR}: the sandbox runs coding CLIs; pick a tool runner (claude, codex …)"
        ));
    }
    if prompt.trim().is_empty() {
        return Err(format!("{ERR}: empty prompt"));
    }
    let agent = agent.unwrap_or_default();
    let role = blocking(move || agent_role(&agent)).await?;
    let (p, spec) = resolved.job(role.as_ref(), &prompt, sandbox);
    to_value(jobs::get(&app)?.submit_spec(
        "run",
        &resolved.agent_id,
        &p,
        workspace.filter(|w| !w.trim().is_empty()),
        spec,
    )?)
}

/// Coding CLIs that can run headless here (for the runner pickers).
#[tauri::command]
pub async fn run_tools() -> Result<Value, String> {
    tokio::task::spawn_blocking(|| Ok(json!(runner::available_tools())))
        .await
        .map_err(|e| e.to_string())?
}

// ------------------------------------------------------------------ playbooks

/// Workflow do catálogo (ou um playbook montado na UI) → playbook resolvido.
#[tauri::command]
pub async fn run_playbook_prepare(
    item_id: Option<String>,
    playbook: Option<Playbook>,
    runner_spec: RunnerSpec,
    workspace: Option<String>,
) -> Result<Value, String> {
    blocking(move || {
        let p = match (playbook, item_id) {
            (Some(p), _) => p,
            (None, Some(id)) => {
                let c = super::agentkit::resolve_any(&id)?;
                pb::from_workflow(&c, runner_spec, workspace)?
            }
            _ => return Err(format!("{ERR}: give a workflow id or a playbook")),
        };
        to_value(p)
    })
    .await
}

/// Starts a playbook: installs its setup components (MCP servers) when
/// `targets` is given, resolves each step's runner and role, and chains the
/// steps as jobs visible in `/llm/jobs`.
#[tauri::command]
pub async fn run_playbook_start(
    app: AppHandle,
    playbook: Playbook,
    task: Option<String>,
    targets: Option<Vec<String>>,
) -> Result<Value, String> {
    let task = task.unwrap_or_default();
    if let Some(targets) = targets.filter(|t| !t.is_empty() && !playbook.setup.is_empty()) {
        let setup = playbook.setup.clone();
        let ws = playbook.workspace.clone();
        blocking(move || {
            let mut components = Vec::new();
            for id in &setup {
                components.push(super::agentkit::resolve_any(id)?);
            }
            let env = super::agentkit::env()?;
            let project = ws.filter(|w| !w.is_empty()).map(PathBuf::from);
            let plan = ak::plan::plan(
                &env,
                PlanRequest {
                    components,
                    targets,
                    scope: Some(if project.is_some() {
                        Scope::Project
                    } else {
                        Scope::Global
                    }),
                    project_dir: project,
                    policy: ak::plan::ConflictPolicy::Rename,
                    secret_values: Default::default(),
                },
            )?;
            ak::writer::apply(&env, &plan).map_err(String::from)?;
            Ok(())
        })
        .await?;
    }
    let mut steps = Vec::new();
    for s in &playbook.steps {
        let runner_spec = s.runner.clone().unwrap_or_else(|| playbook.runner.clone());
        let resolved = resolve_runner(&app, &runner_spec)?;
        let agent = s.agent.clone().unwrap_or_default();
        let command = s.command.clone().unwrap_or_default();
        let (role, cmd) = blocking(move || {
            let role = agent_role(&agent)?;
            let cmd = if command.is_empty() {
                None
            } else {
                agent_role(&command)?
            };
            Ok((role, cmd))
        })
        .await?;
        let mut step_task = s.prompt.clone();
        if let Some(c) = cmd {
            step_task = format!("{}\n\n{}", c.system_prompt, step_task)
                .trim()
                .to_string();
        }
        if !task.trim().is_empty() {
            step_task = format!("{step_task}\n\nOverall task: {}", task.trim());
        }
        let (prompt, spec) = resolved.job(role.as_ref(), &step_task, None);
        steps.push(PlaybookStepRun {
            name: s.name.clone(),
            agent_id: resolved.agent_id.clone(),
            prompt,
            spec,
            runner_label: Some(resolved.label.clone()),
            ..Default::default()
        });
    }
    let run = PlaybookRun {
        name: playbook.name.clone(),
        description: playbook.description.clone(),
        source: playbook.source.clone(),
        workspace: playbook.workspace.clone().filter(|w| !w.is_empty()),
        steps,
        ..Default::default()
    };
    to_value(jobs::get(&app)?.playbook_start(run)?)
}

#[tauri::command]
pub async fn run_playbooks_list(app: AppHandle) -> Result<Value, String> {
    to_value(jobs::get(&app)?.playbooks())
}

#[tauri::command]
pub async fn run_playbook_cancel(app: AppHandle, id: String) -> Result<Value, String> {
    to_value(jobs::get(&app)?.playbook_cancel(&id)?)
}

#[tauri::command]
pub async fn run_playbook_delete(app: AppHandle, id: String) -> Result<Value, String> {
    jobs::get(&app)?.playbook_delete(&id)?;
    Ok(json!({ "ok": true }))
}

// ------------------------------------------------------------------ wizard

/// Detecta a stack, sugere as peças dos templates do catálogo e as
/// ferramentas-alvo (instaladas ou com marcador no projeto).
#[tauri::command]
pub async fn run_wizard_detect(
    project_dir: String,
    extra_templates: Option<Vec<String>>,
) -> Result<Value, String> {
    let dir = PathBuf::from(&project_dir);
    let extra = extra_templates.unwrap_or_default();
    let session = wizard::start(&dir, &extra).await?;
    let dir2 = dir.clone();
    let detected = blocking(move || {
        let env = super::agentkit::env()?;
        Ok(ak::detect::detect_all(&env, Some(&dir2)))
    })
    .await?;
    let targets: Vec<Value> = detected
        .iter()
        .map(|d| {
            let in_project = !d.project_markers.is_empty();
            json!({
                "id": d.id, "name": d.name, "installed": d.installed, "beta": d.beta,
                "in_project": in_project, "tier": d.tier,
                "suggested": (d.installed && d.enabled_by_default) || in_project,
            })
        })
        .collect();
    let mut v = to_value(&session)?;
    v["targets"] = json!(targets);
    v["all_templates"] = json!(wizard::ALL_TEMPLATES);
    Ok(v)
}

/// Plano de instalação das peças escolhidas (e só delas) nas ferramentas
/// escolhidas, escopo projeto. Aplicar com `agentkit_apply(plan.id)`.
#[tauri::command]
pub async fn run_wizard_plan(
    session_id: String,
    selected: Vec<String>,
    targets: Vec<String>,
    agents_md: Option<String>,
    policy: Option<String>,
) -> Result<Value, String> {
    blocking(move || {
        if targets.is_empty() {
            return Err(format!("{ERR}: pick at least one tool"));
        }
        let (project, components) =
            wizard::selected_components(&session_id, &selected, agents_md.as_deref())?;
        let env = super::agentkit::env()?;
        let plan = ak::plan::plan(
            &env,
            PlanRequest {
                components,
                targets,
                scope: Some(Scope::Project),
                project_dir: Some(project),
                policy: super::agentkit::parse_policy(policy.as_deref()),
                secret_values: Default::default(),
            },
        )?;
        to_value(plan)
    })
    .await
}

/// "Revisar o setup com um agente": Job read-only com o runner escolhido.
#[tauri::command]
pub async fn run_wizard_review(
    app: AppHandle,
    project_dir: String,
    runner_spec: RunnerSpec,
) -> Result<Value, String> {
    let dir = PathBuf::from(&project_dir);
    let d2 = dir.clone();
    let prompt = blocking(move || {
        let st = stack::detect(&d2);
        let env = super::agentkit::env()?;
        let mut by_tool: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for rec in ak::writer::installed(&env, Some(&d2)) {
            by_tool
                .entry(rec.target.clone())
                .or_default()
                .push(rec.installed_name.clone());
        }
        Ok(wizard::review_prompt(&st, &by_tool))
    })
    .await?;
    let mut spec = runner_spec;
    if spec.kind == "tool" && spec.permission.is_none() {
        spec.permission = Some("plan".into());
    }
    let resolved = resolve_runner(&app, &spec)?;
    let (p, s) = resolved.job(None, &prompt, None);
    to_value(jobs::get(&app)?.submit_spec("run", &resolved.agent_id, &p, Some(project_dir), s)?)
}

// ------------------------------------------------------------------ sandbox

#[tauri::command]
pub async fn run_sandbox_status() -> Result<Value, String> {
    Ok(sandbox::status().await)
}

/// The generated Dockerfile and image tag of `tool` (shown before a run).
#[tauri::command]
pub async fn run_sandbox_dockerfile(tool: String) -> Result<Value, String> {
    let text = sandbox::docker::dockerfile(&tool)?;
    let tag = sandbox::docker::image_tag(&tool, &text);
    let dir = sandbox::root()?.join(&tool);
    Ok(json!({ "dockerfile": text, "tag": tag, "dir": dir }))
}

/// Diff of a sandboxed job (its id names the run folder).
#[tauri::command]
pub async fn run_sandbox_diff(job_id: String) -> Result<Value, String> {
    blocking(move || {
        let (rec, _) = sandbox::load_record(&job_id)?;
        let changes = sandbox::diff(&job_id)?;
        Ok(json!({ "record": to_value(rec)?, "changes": to_value(changes)? }))
    })
    .await
}

/// Writes the chosen files of a sandboxed job back into the project.
#[tauri::command]
pub async fn run_sandbox_apply(
    job_id: String,
    paths: Option<Vec<String>>,
) -> Result<Value, String> {
    blocking(move || {
        let (applied, skipped) = sandbox::apply(&job_id, &paths.unwrap_or_default())?;
        Ok(json!({ "applied": applied, "skipped": skipped }))
    })
    .await
}

#[tauri::command]
pub async fn run_sandbox_discard(job_id: String) -> Result<Value, String> {
    blocking(move || {
        sandbox::discard(&job_id)?;
        Ok(json!({ "ok": true }))
    })
    .await
}

/// Stack report of a folder (wizard header, "project context" of agents).
#[tauri::command]
pub async fn run_stack_detect(project_dir: String) -> Result<Value, String> {
    blocking(move || to_value(stack::detect(&PathBuf::from(project_dir)))).await
}

/// For the bridge / CLI: `omniget agent run <agent> --tool <x> "<prompt>"`.
#[derive(Debug, Clone, Deserialize)]
pub struct ToolRunRequest {
    /// Catalog id, `path:agent:<file>` or empty.
    #[serde(default)]
    pub agent: Option<String>,
    /// Markdown of an agent file the CLI read.
    #[serde(default)]
    pub agent_markdown: Option<String>,
    #[serde(default)]
    pub agent_name: Option<String>,
    /// `claude|codex|gemini|opencode|qwen|cursor|…|native`
    pub tool: String,
    pub prompt: String,
    #[serde(default)]
    pub project: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub permission: Option<String>,
    /// Roster agent for `--tool native` (default `omni`).
    #[serde(default)]
    pub native_agent: Option<String>,
    #[serde(default)]
    pub sandbox: Option<sandbox::SandboxOpts>,
    /// Append the project's stack ("PROJECT CONTEXT") to the prompt.
    #[serde(default)]
    pub context: bool,
}

/// Body of the bridge route: resolves the agent and submits the job.
pub async fn submit_tool_run(app: &AppHandle, req: ToolRunRequest) -> Result<jobs::Job, String> {
    let runner_spec = if req.tool == "native" {
        RunnerSpec {
            kind: "agent".into(),
            id: req.native_agent.clone().unwrap_or_else(|| "omni".into()),
            ..Default::default()
        }
    } else {
        RunnerSpec {
            kind: "tool".into(),
            id: req.tool.clone(),
            model: req.model.clone(),
            permission: req.permission.clone(),
            ..Default::default()
        }
    };
    let resolved = resolve_runner(app, &runner_spec)?;
    let (agent, md, name) = (
        req.agent.clone().unwrap_or_default(),
        req.agent_markdown.clone(),
        req.agent_name.clone(),
    );
    let role = blocking(move || match md {
        Some(md) => ra::agent_from_markdown(name.as_deref().unwrap_or("agent"), &md).map(Some),
        None => agent_role(&agent),
    })
    .await?;
    let mut prompt = req.prompt.clone();
    if req.context {
        if let Some(p) = req.project.clone() {
            let block =
                blocking(move || Ok(stack::context_block(&stack::detect(&PathBuf::from(p)))))
                    .await?;
            prompt = format!("{prompt}\n\n{block}");
        }
    }
    let (p, spec) = resolved.job(role.as_ref(), &prompt, req.sandbox.clone());
    jobs::get(app)?.submit_spec("run", &resolved.agent_id, &p, req.project.clone(), spec)
}

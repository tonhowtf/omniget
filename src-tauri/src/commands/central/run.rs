use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tauri::{AppHandle, Manager};

use omniget_core::core::agentkit::{self as ak, plan::PlanRequest, Scope};
use omniget_core::core::agentkit_run::{
    agent::{self as ra, AgentPrompt},
    playbook::RunnerSpec,
    runner, sandbox, stack,
};

use crate::jobs::{self, LoopDef, RunSpec, TOOL_PREFIX};

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

#[derive(Debug, Clone)]
pub struct Resolved {
    pub agent_id: String,
    pub label: String,
    pub model: Option<String>,
    pub permission: Option<String>,
}

impl Resolved {
    pub fn is_tool(&self) -> bool {
        self.agent_id.starts_with(TOOL_PREFIX)
    }

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

pub(crate) fn terminal_account(
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

#[derive(Debug, Clone, Deserialize)]
pub struct CatalogLoopRequest {
    pub id: String,
    pub runner: RunnerSpec,
    #[serde(default)]
    pub workspace: Option<String>,
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
    #[serde(default)]
    pub use_interval: bool,
}

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
    #[serde(default)]
    pub source: Option<String>,
    #[serde(default)]
    pub agent: Option<String>,
    #[serde(default)]
    pub plan_id: Option<String>,
    #[serde(default)]
    pub sandbox: Option<sandbox::SandboxOpts>,
}

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

#[derive(Debug, Clone, Deserialize)]
pub struct ToolRunRequest {
    #[serde(default)]
    pub agent: Option<String>,
    #[serde(default)]
    pub agent_markdown: Option<String>,
    #[serde(default)]
    pub agent_name: Option<String>,
    pub tool: String,
    pub prompt: String,
    #[serde(default)]
    pub project: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub permission: Option<String>,
    #[serde(default)]
    pub native_agent: Option<String>,
    #[serde(default)]
    pub sandbox: Option<sandbox::SandboxOpts>,
    #[serde(default)]
    pub context: bool,
}

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

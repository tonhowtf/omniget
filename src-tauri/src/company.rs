use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use omniget_core::core::company::heartbeat::{Claim, RunOutcome};
use omniget_core::core::company::org::Adapter;
use omniget_core::core::company::toolset::{self, ProvisionRequest};
use omniget_core::core::company::{self as co, gov::CostEvent, Actor, CompanyStore};
use omniget_core::core::llm::agent::{
    AgentDef, AgentRole, GrantMode, RuntimeKind, ToolGrant, ToolSource,
};
use serde_json::Value;
use tauri::{AppHandle, Emitter, Manager};
use tokio::sync::Notify;

pub const EVENT: &str = "company://changed";
pub const GLOBAL_MAX: usize = 8;
pub const RUN_DEADLINE_MS: u64 = 45 * 60 * 1000;
pub const SESSION_RUNS: i64 = 40;

pub struct Engine {
    app: AppHandle,
    store: Arc<CompanyStore>,
    wake: Notify,
    live: Mutex<HashMap<String, Option<String>>>,
}

static ENGINE: OnceLock<Arc<Engine>> = OnceLock::new();

pub fn get() -> Option<Arc<Engine>> {
    ENGINE.get().cloned()
}

pub fn store() -> Result<Arc<CompanyStore>, String> {
    co::store()
}

pub fn boot(app: &AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(Duration::from_secs(5)).await;
        let store = match co::store() {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!("[company] boot: {e}");
                return;
            }
        };
        let engine = Arc::new(Engine {
            app: app.clone(),
            store: store.clone(),
            wake: Notify::new(),
            live: Mutex::new(HashMap::new()),
        });
        if ENGINE.set(engine.clone()).is_err() {
            return;
        }
        {
            let app = app.clone();
            let weak = Arc::downgrade(&engine);
            store.on_change(move |c| {
                let _ = app.emit(EVENT, c);
                if matches!(
                    c.entity,
                    "wake" | "issue" | "approval" | "staff" | "company" | "budget"
                ) {
                    if let Some(e) = weak.upgrade() {
                        e.wake.notify_one();
                    }
                }
            });
        }
        {
            let app = app.clone();
            toolset::set_provision_hook(Arc::new(move |req: &ProvisionRequest| {
                provision(&app, req)
            }));
        }
        engine.recover();
        engine.run().await;
    });
}

fn llm(app: &AppHandle) -> Arc<crate::llm_manager::LlmManager> {
    app.state::<crate::AppState>().llm.clone()
}

fn with_company_grants(mut def: AgentDef) -> AgentDef {
    for name in toolset::tool_names() {
        let has = def
            .tools
            .iter()
            .any(|g| matches!(&g.source, ToolSource::Internal { name: n } if *n == name));
        if !has {
            def.tools.push(ToolGrant {
                source: ToolSource::Internal { name },
                mode: GrantMode::Auto,
            });
        }
    }
    def
}

fn default_template(llm: &crate::llm_manager::LlmManager) -> Option<AgentDef> {
    let roster = llm.roster();
    roster
        .iter()
        .find(|a| {
            matches!(&a.runtime, RuntimeKind::Cli { cli, .. } if cli == "claude")
                && !a.id.starts_with("co-")
        })
        .or_else(|| {
            roster
                .iter()
                .find(|a| !matches!(a.runtime, RuntimeKind::Native) && !a.id.starts_with("co-"))
        })
        .or_else(|| roster.iter().find(|a| !a.id.starts_with("co-")))
        .or_else(|| roster.first())
        .cloned()
}

fn provision(app: &AppHandle, req: &ProvisionRequest) -> Result<(), String> {
    let llm = llm(app);
    if llm.agent(&req.agent_id).is_some() {
        return Ok(());
    }
    let base: AgentDef = if let Some(def) = req
        .agent_def
        .clone()
        .and_then(|v| serde_json::from_value::<AgentDef>(v).ok())
    {
        def
    } else if let Some(t) = req.template_agent_id.as_deref().and_then(|t| llm.agent(t)) {
        t
    } else {
        default_template(&llm).ok_or_else(|| {
            format!(
                "{}: create an agent in /llm first; hires copy its runtime",
                co::ERR_COMPANY_INVALID
            )
        })?
    };
    let mut def = base;
    def.id = req.agent_id.clone();
    def.name = req.name.clone();
    def.role = AgentRole::Custom(req.role.clone());
    def.system_prompt = if req.instructions.trim().is_empty() {
        format!(
            "You are {}, the {} of a company of agents run by OmniGet.",
            req.name, req.role
        )
    } else {
        format!(
            "You are {}, the {} of a company of agents run by OmniGet.\n\n{}",
            req.name,
            req.role,
            req.instructions.trim()
        )
    };
    def.skin = None;
    llm.roster_create(with_company_grants(def)).map(|_| ())
}

fn ensure_agent(app: &AppHandle, s: &co::org::Staff) -> Result<(), String> {
    let llm = llm(app);
    match llm.agent(&s.agent_id) {
        Some(def) => {
            let with = with_company_grants(def.clone());
            if with.tools.len() != def.tools.len() {
                llm.roster_update(with)?;
            }
            Ok(())
        }
        None => provision(
            app,
            &ProvisionRequest {
                agent_id: s.agent_id.clone(),
                name: s.name.clone(),
                role: s.role.clone(),
                instructions: s.instructions.clone(),
                template_agent_id: None,
                agent_def: None,
            },
        ),
    }
}

fn audit_actor(company: &str, staff: &str) -> omniget_core::core::llm::audit::Actor {
    omniget_core::core::llm::audit::Actor::via("company", &format!("company:{company}:{staff}"))
}

fn failure_class(job: &crate::jobs::Job) -> Option<String> {
    match job.failure_reason.as_deref() {
        Some("limit") => Some("budget".into()),
        Some("app_restart") => Some("interrupted".into()),
        Some(r) => Some(r.into()),
        None => {
            let e = job.error.clone().unwrap_or_default().to_lowercase();
            if e.contains("rate limit") || e.contains("429") || e.contains("overloaded") {
                Some("rate_limit".into())
            } else if e.contains("spawn") || e.contains("not found") || e.contains("no such file") {
                Some("spawn".into())
            } else if job.state == "interrupted" {
                Some("interrupted".into())
            } else {
                Some("agent_error".into())
            }
        }
    }
}

fn cost_of(job: &crate::jobs::Job, provider: &str) -> CostEvent {
    let u = job.usage.clone().unwrap_or(Value::Null);
    let cost = u["cost_usd"].as_f64();
    CostEvent {
        provider: provider.into(),
        model: u["model"].as_str().unwrap_or("").into(),
        billing_type: if cost.is_some() {
            "metered"
        } else if provider == "local" {
            "local"
        } else {
            "subscription"
        }
        .into(),
        input_tokens: u["input_tokens"].as_i64().unwrap_or(0),
        cached_tokens: u["cache_read_tokens"].as_i64().unwrap_or(0),
        output_tokens: u["output_tokens"].as_i64().unwrap_or(0),
        cost_usd: cost,
    }
}

impl Engine {
    pub fn poke(&self) {
        self.wake.notify_one();
    }

    fn recover(&self) {
        let Ok(live) = self.store.live_runs() else {
            return;
        };
        let jobs = crate::jobs::try_get();
        for r in live {
            let alive = r
                .job_id
                .as_deref()
                .and_then(|j| jobs.as_ref().and_then(|x| x.job(j)))
                .is_some_and(|j| {
                    matches!(
                        j.state.as_str(),
                        "queued" | "running" | "waiting_workspace" | "waiting_approval"
                    )
                });
            if alive {
                if let Some(job_id) = r.job_id.clone() {
                    self.live
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .insert(r.id.clone(), Some(job_id.clone()));
                    let this = get();
                    let run_id = r.id.clone();
                    tauri::async_runtime::spawn(async move {
                        if let Some(this) = this {
                            this.follow(run_id, job_id).await;
                        }
                    });
                }
                continue;
            }
            let _ = self.store.run_finish(
                &r.id,
                RunOutcome {
                    state: "failed".into(),
                    failure_class: Some("interrupted".into()),
                    error: Some("the app closed while this run was working".into()),
                    ..Default::default()
                },
            );
        }
    }

    async fn run(self: Arc<Self>) {
        let mut last_reconcile = 0i64;
        loop {
            let now = co::now_ms();
            let _ = self.store.tick_timers(now);
            let _ = self.store.tick_routines(now);
            let _ = self.store.expire_approvals(now);
            if now - last_reconcile > 60_000 {
                last_reconcile = now;
                let _ = self.store.reconcile(now);
                self.enforce_pauses();
            }
            self.dispatch();
            tokio::select! {
                _ = self.wake.notified() => {
                    tokio::time::sleep(Duration::from_millis(150)).await;
                }
                _ = tokio::time::sleep(Duration::from_secs(2)) => {}
            }
        }
    }

    fn enforce_pauses(&self) {
        let live: Vec<(String, Option<String>)> = self
            .live
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        for (run_id, job) in live {
            let Ok(run) = self.store.run_get(&run_id) else {
                continue;
            };
            let Ok(s) = self.store.staff_get(&run.company_id, &run.staff_id) else {
                continue;
            };
            let Ok(c) = self.store.company_get(&run.company_id) else {
                continue;
            };
            let project = run
                .issue_id
                .as_deref()
                .and_then(|i| self.store.issue_get(&run.company_id, i).ok())
                .and_then(|i| i.project_id);
            let blocked = c.status != "active"
                || matches!(s.status.as_str(), "paused" | "terminated")
                || self
                    .store
                    .budget_block(&c.id, &s.id, project.as_deref())
                    .ok()
                    .flatten()
                    .is_some();
            if blocked {
                if let (Some(j), Some(jobs)) = (job, crate::jobs::try_get()) {
                    let _ = jobs.cancel_by(&j, audit_actor(&run.company_id, &run.staff_id));
                }
            }
        }
    }

    fn dispatch(self: &Arc<Self>) {
        let live = self.live.lock().unwrap_or_else(|e| e.into_inner()).len();
        let free = GLOBAL_MAX.saturating_sub(live);
        if free == 0 {
            return;
        }
        let claims = match self.store.claim(co::now_ms(), free) {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!("[company] claim: {e}");
                return;
            }
        };
        for claim in claims {
            self.live
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .insert(claim.run.id.clone(), None);
            let this = self.clone();
            tauri::async_runtime::spawn(async move {
                let run_id = claim.run.id.clone();
                let out = this.execute(claim).await;
                let summary = out.summary.clone();
                let run = this.store.run_finish(&run_id, out);
                if let (Ok(run), Some(text)) = (run, summary) {
                    if let (Some(issue), false) = (&run.issue_id, text.trim().is_empty()) {
                        let _ = this.store.comment_add(
                            &run.company_id,
                            issue,
                            &text.chars().take(4000).collect::<String>(),
                            Some(&format!("run-summary-{run_id}")),
                            &Actor::agent(&run.staff_id, Some(&run_id)),
                        );
                    }
                }
                this.live
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .remove(&run_id);
                this.wake.notify_one();
            });
        }
    }

    async fn execute(self: &Arc<Self>, claim: Claim) -> RunOutcome {
        let adapter = claim.staff.adapter.clone();
        let res = match adapter {
            Adapter::Roster => self.execute_roster(&claim).await,
            Adapter::Process {
                command,
                args,
                timeout_sec,
            } => {
                self.execute_process(&claim, &command, &args, timeout_sec)
                    .await
            }
            Adapter::Http { url, timeout_sec } => {
                self.execute_http(&claim, &url, timeout_sec).await
            }
        };
        res.unwrap_or_else(|e| RunOutcome {
            state: "failed".into(),
            failure_class: Some(if e.contains("create an agent") {
                "agent_error".into()
            } else {
                "spawn".into()
            }),
            error: Some(e),
            ..Default::default()
        })
    }

    fn workspace_of(&self, claim: &Claim) -> Option<String> {
        let project = claim
            .issue
            .as_ref()
            .and_then(|i| i.project_id.as_deref())
            .and_then(|p| self.store.project_get(&claim.staff.company_id, p).ok());
        project
            .and_then(|p| p.workspace)
            .or_else(|| claim.staff.workspace.clone())
            .or_else(|| {
                self.store
                    .company_get(&claim.staff.company_id)
                    .ok()
                    .and_then(|c| c.workspace)
            })
    }

    async fn workdir(&self, claim: &Claim) -> Option<String> {
        let base = self.workspace_of(claim)?;
        let (Some(issue), Some(pid)) = (
            claim.issue.as_ref(),
            claim.issue.as_ref().and_then(|i| i.project_id.clone()),
        ) else {
            return Some(base);
        };
        let isolated = self
            .store
            .project_get(&claim.staff.company_id, &pid)
            .map(|p| p.isolation == "worktree")
            .unwrap_or(false);
        if !isolated || !std::path::Path::new(&base).join(".git").exists() {
            return Some(base);
        }
        let req = omniget_core::core::vcs::worktree::WorktreeRequest {
            repo: base.clone().into(),
            thread: format!("company-{}", issue.id),
            branch: Some(format!("omniget/{}", issue.identifier)),
            ..Default::default()
        };
        let cancel = Arc::new(std::sync::atomic::AtomicBool::new(false));
        match omniget_core::core::vcs::worktree::create(&req, cancel, |_| {}).await {
            Ok(info) => {
                if !info.reused {
                    let _ = self.store.work_product_add(
                        &claim.staff.company_id,
                        &issue.id,
                        co::issues::NewWorkProduct {
                            kind: "branch".into(),
                            title: info.branch.clone(),
                            path: Some(info.path.to_string_lossy().into()),
                            summary: Some(format!(
                                "worktree from {} at {}",
                                info.base,
                                &info.base_commit[..info.base_commit.len().min(10)]
                            )),
                            url: None,
                        },
                        &Actor::system("worktree"),
                    );
                }
                Some(info.path.to_string_lossy().into())
            }
            Err(e) => {
                tracing::warn!(
                    "[company] worktree for {}: {e}; using the shared folder",
                    issue.identifier
                );
                Some(base)
            }
        }
    }

    async fn execute_roster(self: &Arc<Self>, claim: &Claim) -> Result<RunOutcome, String> {
        ensure_agent(&self.app, &claim.staff)?;
        let s = &claim.staff;
        let past = self
            .store
            .runs(&s.company_id, Some(&s.id), None, 1000)?
            .len() as i64;
        let generation = (past - 1).max(0) / SESSION_RUNS;
        let resumed = (past - 1) % SESSION_RUNS != 0;
        let conversation = format!("company:{}:{}:{generation}", s.company_id, s.id);
        let prompt = self.store.heartbeat_prompt(claim, resumed)?;
        let jobs = crate::jobs::get(&self.app)?;
        let workdir = self.workdir(claim).await;
        let job = jobs.prepare("company", &s.agent_id, &prompt, workdir, Some(conversation))?;
        crate::llm_audit::job_created(audit_actor(&s.company_id, &s.id), &job);
        self.store
            .run_set_job(&claim.run.id, &job.id, prompt.len())?;
        self.live
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(claim.run.id.clone(), Some(job.id.clone()));
        let store = self.store.clone();
        let (company, staff) = (s.company_id.clone(), s.id.clone());
        let project = claim.issue.as_ref().and_then(|i| i.project_id.clone());
        let check: crate::jobs::UsageCheck = Arc::new(move |tokens: u64, usd: Option<f64>| {
            match store.would_exceed(
                &company,
                &staff,
                project.as_deref(),
                tokens as i64,
                usd.unwrap_or(0.0),
            ) {
                Ok(Some(why)) => Err(format!("{}: {why}", co::ERR_COMPANY_BUDGET)),
                _ => Ok(()),
            }
        });
        let guard = crate::jobs::JobGuard {
            deadline_ms: Some(co::now_ms().max(0) as u64 + RUN_DEADLINE_MS),
            check: Some(check),
            deadline_reason: "the heartbeat run reached its 45-minute ceiling".into(),
        };
        let renew = {
            let store = self.store.clone();
            let run = claim.run.id.clone();
            tauri::async_runtime::spawn(async move {
                loop {
                    tokio::time::sleep(Duration::from_secs(300)).await;
                    let _ = store.run_renew_lease(&run);
                }
            })
        };
        let done = jobs.run_prepared_guarded(&job.id, guard).await;
        renew.abort();
        Ok(self.settle(claim, done))
    }

    async fn follow(self: Arc<Self>, run_id: String, job_id: String) {
        let Some(jobs) = crate::jobs::try_get() else {
            return;
        };
        loop {
            match jobs.job(&job_id) {
                Some(j)
                    if !matches!(
                        j.state.as_str(),
                        "queued" | "running" | "waiting_workspace" | "waiting_approval"
                    ) =>
                {
                    break
                }
                None => break,
                _ => tokio::time::sleep(Duration::from_secs(2)).await,
            }
        }
        let job = jobs.job(&job_id);
        let out = match self.store.run_get(&run_id) {
            Ok(run) => {
                let agent = self
                    .store
                    .staff_get(&run.company_id, &run.staff_id)
                    .map(|s| s.agent_id)
                    .unwrap_or_default();
                self.settle_job(&run.company_id, &run_id, &agent, job)
            }
            Err(_) => return,
        };
        let _ = self.store.run_finish(&run_id, out);
        self.live
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&run_id);
        self.wake.notify_one();
    }

    fn settle(&self, claim: &Claim, job: Option<crate::jobs::Job>) -> RunOutcome {
        self.settle_job(
            &claim.staff.company_id,
            &claim.run.id,
            &claim.staff.agent_id,
            job,
        )
    }

    fn settle_job(
        &self,
        company: &str,
        run_id: &str,
        agent_id: &str,
        job: Option<crate::jobs::Job>,
    ) -> RunOutcome {
        let Some(job) = job else {
            return RunOutcome {
                state: "failed".into(),
                failure_class: Some("no_start".into()),
                error: Some("the job vanished".into()),
                ..Default::default()
            };
        };
        let provider = match llm(&self.app).agent(agent_id).map(|a| a.runtime) {
            Some(RuntimeKind::Cli { cli, .. }) => cli,
            Some(RuntimeKind::Acp { command, .. }) => command,
            Some(RuntimeKind::Native) => "api".into(),
            None => String::new(),
        };
        if job.usage.is_some() {
            if let Ok(crossings) = self.store.record_cost(run_id, &cost_of(&job, &provider)) {
                self.cancel_crossed(company, &crossings);
            }
        }
        let state = match job.state.as_str() {
            "done" => "succeeded",
            "cancelled" => "cancelled",
            "failed" if job.failure_reason.as_deref() == Some("deadline") => "timed_out",
            _ => "failed",
        };
        RunOutcome {
            state: state.into(),
            failure_class: if state == "succeeded" || state == "cancelled" {
                None
            } else {
                failure_class(&job)
            },
            error: job.error.clone(),
            summary: job.result.clone().map(|r| r.chars().take(4000).collect()),
            job_id: Some(job.id.clone()),
        }
    }

    fn cancel_crossed(&self, company: &str, crossings: &[co::gov::Crossing]) {
        if !crossings.iter().any(|c| c.kind == "hard") {
            return;
        }
        let _ = company;
        self.enforce_pauses();
    }

    fn heartbeat_json(&self, claim: &Claim) -> Value {
        let prompt = self
            .store
            .heartbeat_prompt(claim, false)
            .unwrap_or_default();
        co::exec::heartbeat_json(claim, &prompt)
    }

    async fn execute_process(
        &self,
        claim: &Claim,
        command: &str,
        args: &[String],
        timeout: Option<u64>,
    ) -> Result<RunOutcome, String> {
        let body = self.heartbeat_json(claim);
        let ws = self.workdir(claim).await;
        let env = self
            .store
            .secret_env(&claim.staff.company_id, &claim.staff.id)
            .unwrap_or_default();
        Ok(co::exec::run_process(claim, &body, command, args, ws.as_deref(), timeout, &env).await)
    }

    async fn execute_http(
        &self,
        claim: &Claim,
        url: &str,
        timeout: Option<u64>,
    ) -> Result<RunOutcome, String> {
        let body = self.heartbeat_json(claim);
        Ok(co::exec::run_http(&body, url, timeout).await)
    }
}

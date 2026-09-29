//! A whole company as one JSON package (`omniget.company/1`), and the
//! built-in templates (which are packages too).
//!
//! Export never carries secret values: bindings leave as "fill me" entries.
//! Import is two steps: [`CompanyStore::import_preview`] returns the plan
//! and its hash; [`CompanyStore::import_apply`] refuses a package whose plan
//! hash differs (what you saw is what gets created), and creates everything
//! in one transaction with fresh ids — a failed import leaves nothing.
//! Paperclip's `companies.sh` is only in the README (area D §3); its v2
//! import/export is a server feature with a mandatory preview, which this
//! mirrors.

use std::collections::HashMap;

use rusqlite::params;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use super::gov::NewPolicy;
use super::issues::{issue_create_tx, NewIssue};
use super::org::{Adapter, Company, NewCompany};
use super::toolset::{provision, ProvisionRequest};
use super::{audit, changed, invalid, new_id, now_ms, sql, Actor, CompanyStore};

pub const FORMAT: &str = "omniget.company/1";

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Package {
    pub format: String,
    #[serde(default)]
    pub template_id: Option<String>,
    pub company: PkgCompany,
    #[serde(default)]
    pub staff: Vec<PkgStaff>,
    #[serde(default)]
    pub goals: Vec<PkgGoal>,
    #[serde(default)]
    pub projects: Vec<PkgProject>,
    #[serde(default)]
    pub routines: Vec<PkgRoutine>,
    #[serde(default)]
    pub budgets: Vec<PkgBudget>,
    #[serde(default)]
    pub labels: Vec<PkgLabel>,
    #[serde(default)]
    pub issues: Vec<PkgIssue>,
    #[serde(default)]
    pub secrets_needed: Vec<PkgSecret>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PkgCompany {
    pub name: String,
    #[serde(default)]
    pub mission: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub issue_prefix: Option<String>,
    #[serde(default)]
    pub require_hire_approval: Option<bool>,
    #[serde(default)]
    pub require_strategy_approval: Option<bool>,
    #[serde(default)]
    pub max_parallel: Option<i64>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PkgStaff {
    pub key: String,
    pub name: String,
    pub role: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub reports_to: Option<String>,
    #[serde(default)]
    pub capabilities: String,
    #[serde(default)]
    pub instructions: String,
    #[serde(default)]
    pub adapter: Adapter,
    #[serde(default)]
    pub can_hire: bool,
    #[serde(default)]
    pub heartbeat_cron: Option<String>,
    #[serde(default)]
    pub max_concurrent: Option<i64>,
    /// The roster agent definition to recreate (runtime, model, tools).
    #[serde(default)]
    pub agent: Option<Value>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PkgGoal {
    pub key: String,
    pub level: String,
    #[serde(default)]
    pub parent: Option<String>,
    pub title: String,
    #[serde(default)]
    pub body: String,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub owner: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PkgProject {
    pub key: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub goal: Option<String>,
    #[serde(default)]
    pub lead: Option<String>,
    #[serde(default)]
    pub isolation: Option<String>,
    #[serde(default)]
    pub color: Option<String>,
    #[serde(default)]
    pub status: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PkgRoutine {
    pub title: String,
    #[serde(default)]
    pub body: String,
    pub assignee: String,
    #[serde(default)]
    pub project: Option<String>,
    #[serde(default)]
    pub priority: Option<String>,
    pub cron: String,
    #[serde(default)]
    pub tz: Option<String>,
    #[serde(default)]
    pub catch_up: Option<String>,
    #[serde(default)]
    pub concurrency: Option<String>,
    #[serde(default)]
    pub enabled: Option<bool>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PkgBudget {
    pub scope_type: String,
    #[serde(default)]
    pub scope: Option<String>,
    pub metric: String,
    pub window: String,
    pub amount: f64,
    #[serde(default)]
    pub warn_pct: Option<i64>,
    #[serde(default)]
    pub hard_stop: Option<bool>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PkgLabel {
    pub name: String,
    #[serde(default)]
    pub color: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PkgIssue {
    pub key: String,
    pub title: String,
    #[serde(default)]
    pub body: String,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub priority: Option<String>,
    #[serde(default)]
    pub assignee: Option<String>,
    #[serde(default)]
    pub parent: Option<String>,
    #[serde(default)]
    pub project: Option<String>,
    #[serde(default)]
    pub goal: Option<String>,
    #[serde(default)]
    pub labels: Vec<String>,
    #[serde(default)]
    pub work_mode: Option<String>,
    #[serde(default)]
    pub review_policy: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PkgSecret {
    pub staff: String,
    pub env_key: String,
    pub secret_name: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ImportPlan {
    pub company: String,
    pub issue_prefix: String,
    pub staff: Vec<String>,
    pub goals: usize,
    pub projects: usize,
    pub routines: usize,
    pub budgets: usize,
    pub issues: usize,
    pub secrets_to_fill: Vec<String>,
    pub warnings: Vec<String>,
    pub plan_hash: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ImportOptions {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub mission: Option<String>,
    #[serde(default)]
    pub workspace: Option<String>,
    #[serde(default = "yes")]
    pub include_issues: bool,
    /// Roster agent whose runtime new agents copy when the package does not
    /// carry one (the app picks a sensible default when `None`).
    #[serde(default)]
    pub runtime_template: Option<String>,
}

fn yes() -> bool {
    true
}

fn validate(p: &Package) -> Result<Vec<String>, String> {
    if p.format != FORMAT {
        return Err(invalid(format!(
            "unknown package format `{}` (expected {FORMAT})",
            p.format
        )));
    }
    if p.company.name.trim().is_empty() {
        return Err(invalid("the package has no company name"));
    }
    let mut warnings = Vec::new();
    let keys: HashMap<&str, &PkgStaff> = p.staff.iter().map(|s| (s.key.as_str(), s)).collect();
    if keys.len() != p.staff.len() {
        return Err(invalid("two staff entries share a key"));
    }
    for s in &p.staff {
        if let Some(m) = &s.reports_to {
            if !keys.contains_key(m.as_str()) {
                return Err(invalid(format!("{} reports to unknown `{m}`", s.name)));
            }
        }
        if let Some(c) = &s.heartbeat_cron {
            super::routines::validate_cron(c)?;
        }
    }
    // Cycles in the imported org chart.
    for s in &p.staff {
        let mut cur = s.reports_to.clone();
        let mut hops = 0;
        while let Some(m) = cur {
            hops += 1;
            if m == s.key || hops > p.staff.len() {
                return Err(invalid(format!("the org chart loops at {}", s.name)));
            }
            cur = keys.get(m.as_str()).and_then(|x| x.reports_to.clone());
        }
    }
    let goal_keys: std::collections::HashSet<&str> =
        p.goals.iter().map(|g| g.key.as_str()).collect();
    for g in &p.goals {
        if let Some(par) = &g.parent {
            if !goal_keys.contains(par.as_str()) {
                warnings.push(format!(
                    "goal `{}` has an unknown parent; it goes under the mission",
                    g.title
                ));
            }
        }
    }
    for r in &p.routines {
        super::routines::validate_cron(&r.cron)?;
        if !keys.contains_key(r.assignee.as_str()) {
            return Err(invalid(format!(
                "routine `{}` is assigned to unknown `{}`",
                r.title, r.assignee
            )));
        }
    }
    for s in &p.staff {
        if s.agent.is_none() && matches!(s.adapter, Adapter::Roster) {
            warnings.push(format!(
                "{} gets the default runtime (the package has no agent definition)",
                s.name
            ));
        }
    }
    Ok(warnings)
}

fn plan_hash(p: &Package, o: &ImportOptions) -> String {
    let mut h = Sha256::new();
    h.update(serde_json::to_vec(p).unwrap_or_default());
    h.update(
        serde_json::to_vec(&(o.name.as_deref(), o.mission.as_deref(), o.include_issues))
            .unwrap_or_default(),
    );
    hex::encode(&h.finalize()[..16])
}

fn slug(s: &str) -> String {
    let mut out = String::new();
    for c in s.to_lowercase().chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c);
        } else if !out.ends_with('-') {
            out.push('-');
        }
    }
    out.trim_matches('-').chars().take(20).collect()
}

impl CompanyStore {
    pub fn export(&self, company: &str, include_issues: bool) -> Result<Package, String> {
        let c = self.company_get(company)?;
        let staff = self.staff_list(company)?;
        let key_of: HashMap<String, String> = staff
            .iter()
            .filter(|s| s.status != "terminated")
            .map(|s| (s.id.clone(), slug(&format!("{}-{}", s.role, s.name))))
            .collect();
        let goals = self.goal_list(company)?;
        let projects = self.project_list(company)?;
        let pkey: HashMap<String, String> = projects
            .iter()
            .map(|p| (p.id.clone(), slug(&p.name)))
            .collect();
        let secrets: Vec<PkgSecret> = self.read(|db| {
            let mut st = db.prepare("SELECT staff_id, env_key, secret_name FROM secret_bindings WHERE company_id = ?1")?;
            let rows = st.query_map(params![company], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?)))?;
            rows.collect::<Result<Vec<_>, _>>()
        })?
        .into_iter()
        .filter_map(|(s, e, n)| key_of.get(&s).map(|k| PkgSecret { staff: k.clone(), env_key: e, secret_name: n }))
        .collect();
        let issues = if include_issues {
            let all = self.issue_list(
                company,
                &super::issues::IssueFilter {
                    include_closed: false,
                    limit: Some(5000),
                    ..Default::default()
                },
            )?;
            all.iter()
                .map(|i| PkgIssue {
                    key: i.identifier.clone(),
                    title: i.title.clone(),
                    body: i.body.clone(),
                    status: Some(if i.status == "in_progress" {
                        "todo".into()
                    } else {
                        i.status.clone()
                    }),
                    priority: Some(i.priority.clone()),
                    assignee: i
                        .assignee_staff_id
                        .as_ref()
                        .and_then(|a| key_of.get(a).cloned()),
                    parent: i.parent_id.as_ref().and_then(|p| {
                        all.iter()
                            .find(|x| &x.id == p)
                            .map(|x| x.identifier.clone())
                    }),
                    project: i.project_id.as_ref().and_then(|p| pkey.get(p).cloned()),
                    goal: i.goal_id.clone(),
                    labels: Vec::new(),
                    work_mode: Some(i.work_mode.clone()),
                    review_policy: Some(i.review_policy.clone()),
                })
                .collect()
        } else {
            Vec::new()
        };
        Ok(Package {
            format: FORMAT.into(),
            template_id: c.template.clone(),
            company: PkgCompany {
                name: c.name.clone(),
                mission: c.mission.clone(),
                description: c.description.clone(),
                issue_prefix: Some(c.issue_prefix.clone()),
                require_hire_approval: Some(c.require_hire_approval),
                require_strategy_approval: Some(c.require_strategy_approval),
                max_parallel: Some(c.max_parallel),
            },
            staff: staff
                .iter()
                .filter(|s| s.status != "terminated")
                .map(|s| PkgStaff {
                    key: key_of[&s.id].clone(),
                    name: s.name.clone(),
                    role: s.role.clone(),
                    title: s.title.clone(),
                    reports_to: s.reports_to.as_ref().and_then(|m| key_of.get(m).cloned()),
                    capabilities: s.capabilities.clone(),
                    instructions: s.instructions.clone(),
                    adapter: s.adapter.clone(),
                    can_hire: s.can_hire,
                    heartbeat_cron: s.heartbeat_cron.clone(),
                    max_concurrent: Some(s.max_concurrent),
                    agent: agent_snapshot(&s.agent_id),
                })
                .collect(),
            goals: goals
                .iter()
                .filter(|g| !(g.level == "company" && g.parent_id.is_none()))
                .map(|g| PkgGoal {
                    key: g.id.clone(),
                    level: g.level.clone(),
                    parent: g
                        .parent_id
                        .clone()
                        .filter(|p| goals.iter().any(|x| &x.id == p && x.parent_id.is_some())),
                    title: g.title.clone(),
                    body: g.body.clone(),
                    status: Some(g.status.clone()),
                    owner: g
                        .owner_staff_id
                        .as_ref()
                        .and_then(|o| key_of.get(o).cloned()),
                })
                .collect(),
            projects: projects
                .iter()
                .map(|p| PkgProject {
                    key: pkey[&p.id].clone(),
                    name: p.name.clone(),
                    description: p.description.clone(),
                    goal: p.goal_id.clone(),
                    lead: p
                        .lead_staff_id
                        .as_ref()
                        .and_then(|l| key_of.get(l).cloned()),
                    isolation: Some(p.isolation.clone()),
                    color: p.color.clone(),
                    status: Some(p.status.clone()),
                })
                .collect(),
            routines: self
                .routine_list(company)?
                .into_iter()
                .filter_map(|r| {
                    Some(PkgRoutine {
                        assignee: key_of.get(&r.staff_id)?.clone(),
                        title: r.title,
                        body: r.body,
                        project: r.project_id.and_then(|p| pkey.get(&p).cloned()),
                        priority: Some(r.priority),
                        cron: r.cron,
                        tz: Some(r.tz),
                        catch_up: Some(r.catch_up),
                        concurrency: Some(r.concurrency),
                        enabled: Some(r.enabled),
                    })
                })
                .collect(),
            budgets: self
                .budget_status(company)?
                .into_iter()
                .map(|b| PkgBudget {
                    scope: match b.policy.scope_type.as_str() {
                        "agent" => key_of.get(&b.policy.scope_id).cloned(),
                        "project" => pkey.get(&b.policy.scope_id).cloned(),
                        _ => None,
                    },
                    scope_type: b.policy.scope_type,
                    metric: b.policy.metric,
                    window: b.policy.window,
                    amount: b.policy.amount,
                    warn_pct: Some(b.policy.warn_pct),
                    hard_stop: Some(b.policy.hard_stop),
                })
                .collect(),
            labels: self
                .labels_list(company)?
                .into_iter()
                .map(|l| PkgLabel {
                    name: l.name,
                    color: Some(l.color),
                })
                .collect(),
            issues,
            secrets_needed: secrets,
        })
    }

    pub fn import_preview(&self, p: &Package, o: &ImportOptions) -> Result<ImportPlan, String> {
        let warnings = validate(p)?;
        let name = o.name.clone().unwrap_or_else(|| p.company.name.clone());
        let prefix = p
            .company
            .issue_prefix
            .clone()
            .unwrap_or_else(|| super::org::derive_prefix(&name));
        let taken = self
            .company_list()?
            .iter()
            .any(|c| c.issue_prefix.eq_ignore_ascii_case(&prefix));
        let mut warnings = warnings;
        if taken {
            warnings.push(format!("the prefix {prefix} is taken; a digit is added"));
        }
        Ok(ImportPlan {
            company: name,
            issue_prefix: prefix,
            staff: p
                .staff
                .iter()
                .map(|s| format!("{} — {}", s.name, s.role))
                .collect(),
            goals: p.goals.len(),
            projects: p.projects.len(),
            routines: p.routines.len(),
            budgets: p.budgets.len(),
            issues: if o.include_issues { p.issues.len() } else { 0 },
            secrets_to_fill: p
                .secrets_needed
                .iter()
                .map(|s| format!("{} → {}", s.staff, s.env_key))
                .collect(),
            warnings,
            plan_hash: plan_hash(p, o),
        })
    }

    /// Creates the company from a previewed package. `expected_hash` must be
    /// the preview's `plan_hash`.
    pub fn import_apply(
        &self,
        p: &Package,
        o: &ImportOptions,
        expected_hash: &str,
        actor: &Actor,
    ) -> Result<Company, String> {
        validate(p)?;
        if plan_hash(p, o) != expected_hash {
            return Err(invalid(
                "the package changed since the preview; preview it again",
            ));
        }
        let name = o.name.clone().unwrap_or_else(|| p.company.name.clone());
        let company = self.company_create(
            NewCompany {
                name: name.clone(),
                mission: o
                    .mission
                    .clone()
                    .unwrap_or_else(|| p.company.mission.clone()),
                description: p.company.description.clone(),
                issue_prefix: p.company.issue_prefix.clone(),
                workspace: o.workspace.clone(),
                require_hire_approval: p.company.require_hire_approval,
                require_strategy_approval: p.company.require_strategy_approval,
                max_parallel: p.company.max_parallel,
                template: p.template_id.clone(),
            },
            actor,
        )?;
        let cid = company.id.clone();
        let prefix = company.issue_prefix.to_lowercase();
        // Roster agents first (outside the transaction: they live in the
        // roster). Their ids are fresh, so a failed import leaves only unused
        // roster entries, which the app removes with the company.
        let mut agent_of: HashMap<String, String> = HashMap::new();
        for s in &p.staff {
            let agent_id = format!("co-{prefix}-{}-{}", slug(&s.name), &new_id()[..4]);
            if matches!(s.adapter, Adapter::Roster) {
                provision(&ProvisionRequest {
                    agent_id: agent_id.clone(),
                    name: s.name.clone(),
                    role: s.role.clone(),
                    instructions: s.instructions.clone(),
                    template_agent_id: o.runtime_template.clone(),
                    agent_def: s.agent.clone(),
                })?;
            }
            agent_of.insert(s.key.clone(), agent_id);
        }
        let result = self.write(|tx, ch| {
            let now = now_ms();
            let mut sid: HashMap<String, String> = HashMap::new();
            // Managers before reports: insert all, then wire reports_to.
            for s in &p.staff {
                let id = new_id();
                tx.execute(
                    "INSERT INTO staff (id, company_id, agent_id, name, role, title, capabilities, instructions, adapter,
                       status, can_hire, heartbeat_cron, max_concurrent, created_ms, updated_ms)
                     VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,'idle',?10,?11,?12,?13,?13)",
                    params![id, cid, agent_of[&s.key], s.name, s.role, s.title, s.capabilities, s.instructions,
                        serde_json::to_string(&s.adapter).unwrap_or_else(|_| "{\"kind\":\"roster\"}".into()),
                        s.can_hire as i64, s.heartbeat_cron, s.max_concurrent.unwrap_or(1).clamp(1, 20), now],
                )
                .map_err(sql)?;
                sid.insert(s.key.clone(), id);
            }
            for s in &p.staff {
                if let Some(m) = &s.reports_to {
                    tx.execute("UPDATE staff SET reports_to = ?2 WHERE id = ?1", params![sid[&s.key], sid[m]]).map_err(sql)?;
                }
                super::routines::schedule_staff_heartbeat(tx, &cid, &sid[&s.key], now)?;
            }
            let root: String = tx
                .query_row("SELECT id FROM goals WHERE company_id = ?1 AND level = 'company' AND parent_id IS NULL", params![cid], |r| r.get(0))
                .map_err(sql)?;
            let mut gid: HashMap<String, String> = HashMap::new();
            // Parents before children: repeat until nothing is left.
            let mut left: Vec<&PkgGoal> = p.goals.iter().collect();
            let mut guard = 0;
            while !left.is_empty() && guard < 64 {
                guard += 1;
                left.retain(|g| {
                    let parent = match &g.parent {
                        Some(k) if p.goals.iter().any(|x| &x.key == k) => match gid.get(k) {
                            Some(id) => id.clone(),
                            None => return true,
                        },
                        _ => root.clone(),
                    };
                    let id = new_id();
                    let level = if g.level == "company" { "team" } else { g.level.as_str() };
                    let ok = tx.execute(
                        "INSERT INTO goals (id, company_id, level, parent_id, owner_staff_id, title, body, status, created_ms, updated_ms)
                         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?9)",
                        params![id, cid, level, parent, g.owner.as_ref().and_then(|o| sid.get(o)), g.title, g.body,
                            g.status.clone().unwrap_or_else(|| "active".into()), now],
                    );
                    if ok.is_ok() {
                        gid.insert(g.key.clone(), id);
                    }
                    false
                });
            }
            let mut pid: HashMap<String, String> = HashMap::new();
            for pr in &p.projects {
                let id = new_id();
                tx.execute(
                    "INSERT INTO projects (id, company_id, goal_id, lead_staff_id, name, description, status, workspace, isolation, color, created_ms, updated_ms)
                     VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?11)",
                    params![id, cid, pr.goal.as_ref().and_then(|g| gid.get(g)), pr.lead.as_ref().and_then(|l| sid.get(l)),
                        pr.name, pr.description, pr.status.clone().unwrap_or_else(|| "in_progress".into()), o.workspace,
                        pr.isolation.clone().unwrap_or_else(|| "shared".into()), pr.color, now],
                )
                .map_err(sql)?;
                pid.insert(pr.key.clone(), id);
            }
            for l in &p.labels {
                tx.execute(
                    "INSERT OR IGNORE INTO labels (id, company_id, name, color) VALUES (?1,?2,?3,?4)",
                    params![new_id(), cid, l.name, l.color.clone().unwrap_or_else(|| "#888888".into())],
                )
                .map_err(sql)?;
            }
            for r in &p.routines {
                let tz = r.tz.clone().unwrap_or_else(|| "local".into());
                let next = super::routines::next_after(&r.cron, &tz, now).ok();
                tx.execute(
                    "INSERT INTO routines (id, company_id, title, body, staff_id, project_id, priority, cron, tz, catch_up, concurrency,
                       enabled, next_run_ms, created_ms, updated_ms)
                     VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?14)",
                    params![new_id(), cid, r.title, r.body, sid[&r.assignee], r.project.as_ref().and_then(|x| pid.get(x)),
                        r.priority.clone().unwrap_or_else(|| "medium".into()), r.cron, tz,
                        r.catch_up.clone().unwrap_or_else(|| "once".into()), r.concurrency.clone().unwrap_or_else(|| "skip_if_active".into()),
                        r.enabled.unwrap_or(true) as i64, next, now],
                )
                .map_err(sql)?;
            }
            for sec in &p.secrets_needed {
                if let Some(s) = sid.get(&sec.staff) {
                    tx.execute(
                        "INSERT OR REPLACE INTO secret_bindings (company_id, staff_id, env_key, secret_name) VALUES (?1,?2,?3,?4)",
                        params![cid, s, sec.env_key, sec.secret_name],
                    )
                    .map_err(sql)?;
                }
            }
            if o.include_issues {
                let mut iid: HashMap<String, String> = HashMap::new();
                let mut left: Vec<&PkgIssue> = p.issues.iter().collect();
                let mut guard = 0;
                while !left.is_empty() && guard < 64 {
                    guard += 1;
                    let mut next_left = Vec::new();
                    for i in left {
                        let parent = match &i.parent {
                            Some(k) if p.issues.iter().any(|x| &x.key == k) => match iid.get(k) {
                                Some(v) => Some(v.clone()),
                                None => {
                                    next_left.push(i);
                                    continue;
                                }
                            },
                            _ => None,
                        };
                        let status = i.status.clone().filter(|s| matches!(s.as_str(), "backlog" | "todo" | "blocked")).unwrap_or_else(|| "todo".into());
                        let created = issue_create_tx(tx, ch, &cid, NewIssue {
                            title: i.title.clone(),
                            body: i.body.clone(),
                            parent_id: parent,
                            priority: i.priority.clone(),
                            status: Some(if status == "blocked" { "todo".into() } else { status }),
                            assignee_staff_id: i.assignee.as_ref().and_then(|a| sid.get(a).cloned()),
                            project_id: i.project.as_ref().and_then(|x| pid.get(x).cloned()),
                            goal_id: i.goal.as_ref().and_then(|g| gid.get(g).cloned()),
                            labels: i.labels.clone(),
                            work_mode: i.work_mode.clone(),
                            review_policy: i.review_policy.clone(),
                            origin_kind: Some("import".into()),
                            ..Default::default()
                        }, actor)?;
                        iid.insert(i.key.clone(), created.id);
                    }
                    left = next_left;
                }
            }
            audit::record(tx, &cid, actor, "company.imported", "company", &cid,
                Some(json!({"staff": p.staff.len(), "goals": p.goals.len(), "projects": p.projects.len(), "template": p.template_id})))?;
            changed(ch, &cid, "company", &cid);
            Ok(sid)
        });
        let sid = match result {
            Ok(s) => s,
            Err(e) => {
                let _ = self.company_delete(&cid, &Actor::user());
                return Err(e);
            }
        };
        for b in &p.budgets {
            let scope_id = match b.scope_type.as_str() {
                "agent" => b.scope.as_ref().and_then(|k| sid.get(k).cloned()),
                "company" => Some(cid.clone()),
                _ => None,
            };
            if scope_id.is_none() && b.scope_type != "company" {
                continue;
            }
            self.policy_upsert(
                &cid,
                NewPolicy {
                    scope_type: b.scope_type.clone(),
                    scope_id,
                    metric: b.metric.clone(),
                    window: b.window.clone(),
                    amount: b.amount,
                    warn_pct: b.warn_pct,
                    hard_stop: b.hard_stop,
                },
                &Actor::user(),
            )?;
        }
        self.company_get(&cid)
    }
}

/// The roster definition behind a staff member, as JSON (without secrets:
/// accounts are names, never keys).
fn agent_snapshot(agent_id: &str) -> Option<Value> {
    let dir = crate::core::llm::roster_store::llm_dir()?;
    let raw = std::fs::read_to_string(dir.join("roster.json")).ok()?;
    let v: Value = serde_json::from_str(&raw).ok()?;
    let list = v
        .get("agents")
        .and_then(Value::as_array)
        .or_else(|| v.as_array())?;
    list.iter()
        .find(|a| a.get("id").and_then(Value::as_str) == Some(agent_id))
        .cloned()
}

// ── Templates ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
pub struct TemplateInfo {
    pub id: String,
    pub name: String,
    pub summary: String,
    pub roles: Vec<String>,
}

#[allow(clippy::too_many_arguments)]
fn st(
    key: &str,
    name: &str,
    role: &str,
    title: &str,
    boss: Option<&str>,
    hire: bool,
    cron: Option<&str>,
    caps: &str,
    instr: &str,
) -> PkgStaff {
    PkgStaff {
        key: key.into(),
        name: name.into(),
        role: role.into(),
        title: title.into(),
        reports_to: boss.map(str::to_string),
        capabilities: caps.into(),
        instructions: instr.into(),
        can_hire: hire,
        heartbeat_cron: cron.map(str::to_string),
        ..Default::default()
    }
}

fn rt(title: &str, body: &str, who: &str, cron: &str) -> PkgRoutine {
    PkgRoutine {
        title: title.into(),
        body: body.into(),
        assignee: who.into(),
        cron: cron.into(),
        ..Default::default()
    }
}

const CEO_INSTR: &str = "Turn the mission into a short strategy, then into goals and issues for your reports. Delegate; do not do the work yourself. Each heartbeat: read what changed, unblock people, re-prioritise, and report progress to the owner in one comment on the company's top issue. Hire only when a role is clearly missing.";
const LEAD_INSTR: &str = "Break the work you receive into small issues for your reports, review what they deliver (move it to done or back to todo with precise notes), and escalate what you cannot decide.";
const IC_INSTR: &str = "Take your issues one at a time: checkout, do the work, leave a work product or a clear comment, and move the issue. If something blocks you, set blocked with the exact reason and mention who can unblock it.";

pub fn templates() -> Vec<Package> {
    let base = |id: &str, name: &str, mission: &str, desc: &str| Package {
        format: FORMAT.into(),
        template_id: Some(id.into()),
        company: PkgCompany {
            name: name.into(),
            mission: mission.into(),
            description: desc.into(),
            require_hire_approval: Some(true),
            ..Default::default()
        },
        ..Default::default()
    };
    let mut startup = base(
        "startup",
        "Startup",
        "Ship a product people pay for.",
        "A small company: a CEO who plans, a CTO with an engineer and a designer, and a marketer.",
    );
    startup.staff = vec![
        st(
            "ceo",
            "Ada",
            "CEO",
            "Chief Executive Officer",
            None,
            true,
            Some("0 9 * * 1-5"),
            "Strategy, goals, priorities, hiring, reporting to the owner.",
            CEO_INSTR,
        ),
        st(
            "cto",
            "Linus",
            "CTO",
            "Chief Technology Officer",
            Some("ceo"),
            true,
            None,
            "Technical plan, architecture, code review, engineering delivery.",
            LEAD_INSTR,
        ),
        st(
            "engineer",
            "Grace",
            "Engineer",
            "Software Engineer",
            Some("cto"),
            false,
            None,
            "Implement features and fixes with tests.",
            IC_INSTR,
        ),
        st(
            "designer",
            "Dieter",
            "Designer",
            "Product Designer",
            Some("cto"),
            false,
            None,
            "User flows, UI specs and copy for the product.",
            IC_INSTR,
        ),
        st(
            "marketer",
            "Seth",
            "Marketer",
            "Growth Marketer",
            Some("ceo"),
            false,
            None,
            "Positioning, launch plan, content and channels.",
            IC_INSTR,
        ),
    ];
    startup.projects = vec![PkgProject {
        key: "product".into(),
        name: "Product".into(),
        description: "The thing we ship.".into(),
        lead: Some("cto".into()),
        ..Default::default()
    }];
    startup.routines = vec![rt("Weekly review {{week}}", "Review last week's work against the goals, update priorities, and post a summary for the owner.", "ceo", "0 9 * * 1")];
    startup.labels = vec![
        PkgLabel {
            name: "bug".into(),
            color: Some("#e5484d".into()),
        },
        PkgLabel {
            name: "feature".into(),
            color: Some("#0090ff".into()),
        },
    ];

    let mut dev = base(
        "dev_shop",
        "Dev shop",
        "Deliver the backlog with tests and reviews.",
        "A tech lead, two engineers and a reviewer who closes nothing without checking it.",
    );
    dev.staff = vec![
        st("lead", "Margaret", "Tech Lead", "Tech Lead", None, true, Some("0 10 * * 1-5"), "Plan the sprint, split issues, assign, unblock.", LEAD_INSTR),
        st("eng1", "Ken", "Engineer", "Backend Engineer", Some("lead"), false, None, "Backend and data work.", IC_INSTR),
        st("eng2", "Barbara", "Engineer", "Frontend Engineer", Some("lead"), false, None, "Frontend and UX work.", IC_INSTR),
        st("qa", "Edsger", "Reviewer", "QA Reviewer", Some("lead"), false, None, "Review work in review: run the checks, read the diff, accept or send back with notes.", "Only review. Never implement. Close an issue only when you verified it; otherwise move it back to todo with numbered notes."),
    ];
    dev.routines = vec![rt(
        "Standup {{date}}",
        "Summarise yesterday, today and blockers for each engineer.",
        "lead",
        "0 10 * * 1-5",
    )];

    let mut content = base(
        "content_studio",
        "Content studio",
        "Publish content that grows the audience every week.",
        "An editor who plans, a writer, an SEO analyst and a social media manager.",
    );
    content.staff = vec![
        st(
            "editor",
            "Maya",
            "Editor-in-chief",
            "Editor-in-chief",
            None,
            true,
            Some("0 9 * * 1"),
            "Editorial calendar, briefs, final approval.",
            CEO_INSTR,
        ),
        st(
            "writer",
            "Toni",
            "Writer",
            "Staff Writer",
            Some("editor"),
            false,
            None,
            "Drafts from briefs, revisions from notes.",
            IC_INSTR,
        ),
        st(
            "seo",
            "Rand",
            "SEO analyst",
            "SEO Analyst",
            Some("editor"),
            false,
            None,
            "Keyword research, briefs, on-page checks.",
            IC_INSTR,
        ),
        st(
            "social",
            "Gary",
            "Social media",
            "Social Media Manager",
            Some("editor"),
            false,
            None,
            "Posts and threads for each published piece.",
            IC_INSTR,
        ),
    ];
    content.routines = vec![rt("Content plan {{week}}", "Plan this week's pieces: one issue per piece with a brief, assigned to the writer, with SEO and social subtasks.", "editor", "0 9 * * 1")];

    let mut lab = base(
        "research_lab",
        "Research lab",
        "Answer the question with sources you can defend.",
        "A principal investigator, a researcher and an analyst who checks the numbers.",
    );
    lab.staff = vec![
        st(
            "pi",
            "Marie",
            "Principal investigator",
            "Principal Investigator",
            None,
            true,
            None,
            "Frame the questions, split the work, write the conclusions.",
            LEAD_INSTR,
        ),
        st(
            "researcher",
            "Rosalind",
            "Researcher",
            "Researcher",
            Some("pi"),
            false,
            None,
            "Find and read sources; write notes with citations.",
            IC_INSTR,
        ),
        st(
            "analyst",
            "John",
            "Analyst",
            "Data Analyst",
            Some("pi"),
            false,
            None,
            "Check numbers, run analyses, flag weak evidence.",
            IC_INSTR,
        ),
    ];

    let mut solo = base(
        "solo",
        "Solo founder",
        "Keep the founder's week moving.",
        "One chief of staff agent who triages, plans and follows up for the owner.",
    );
    solo.staff = vec![st("cos", "Alfred", "Chief of staff", "Chief of Staff", None, true, Some("0 8 * * 1-5"), "Triage, plan the day, follow up, draft replies.", "Each morning: list what is due, what is blocked, and what the owner should decide. Keep issues tidy. Hire help only with the owner's approval.")];

    vec![startup, dev, content, lab, solo]
}

pub fn template_list() -> Vec<TemplateInfo> {
    templates()
        .into_iter()
        .map(|t| TemplateInfo {
            id: t.template_id.clone().unwrap_or_default(),
            name: t.company.name.clone(),
            summary: t.company.description.clone(),
            roles: t.staff.iter().map(|s| s.role.clone()).collect(),
        })
        .collect()
}

pub fn template(id: &str) -> Option<Package> {
    templates()
        .into_iter()
        .find(|t| t.template_id.as_deref() == Some(id))
}

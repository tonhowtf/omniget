//! One dispatcher for every surface: the window (`company_rpc` command), the
//! local bridge (`POST /v1/company/rpc`), the CLI (`omniget company …`) and
//! the external MCP. Same method names, same arguments, same rules — the
//! surface only decides the [`Actor`] (and whether it may write).
//!
//! Methods are `area.verb`; arguments are one JSON object. Read-only methods
//! are listed in [`READ_ONLY`] so a read scope can be enforced by name.

use serde::de::DeserializeOwned;
use serde_json::{json, Value};

use super::audit::AuditQuery;
use super::gov::{NewApproval, NewPolicy};
use super::issues::{IssueFilter, IssuePatch, NewIssue, NewWorkProduct};
use super::org::{
    CompanyPatch, GoalPatch, NewCompany, NewGoal, NewProject, NewStaff, ProjectPatch, StaffPatch,
};
use super::portable::{ImportOptions, Package};
use super::routines::NewRoutine;
use super::{invalid, Actor, CompanyStore};

pub const READ_ONLY: &[&str] = &[
    "company.list",
    "company.get",
    "company.dashboard",
    "company.templates",
    "company.export",
    "staff.list",
    "staff.chart",
    "staff.get",
    "goal.list",
    "project.list",
    "issue.list",
    "issue.get",
    "issue.search",
    "issue.revisions",
    "label.list",
    "inbox.list",
    "approval.list",
    "approval.get",
    "budget.status",
    "budget.incidents",
    "cost.summary",
    "run.list",
    "run.get",
    "wake.list",
    "routine.list",
    "audit.list",
    "audit.verify",
    "audit.csv",
    "import.preview",
    "company.stats",
    "staff.scorecards",
    "secret.list",
];

/// Owner decisions: only the OmniGet window takes them (area C §3.8 — in
/// Paperclip any local request without a token acts as the board).
pub const OWNER_ONLY: &[&str] = &[
    "approval.decide",
    "budget.set",
    "budget.delete",
    "budget.resolve",
    "company.delete",
    "company.update",
    "company.status",
    "staff.status",
    "staff.update",
    "import.apply",
    "issue.delete",
    "issue.doc_lock",
    "secret.set",
    "secret.bind",
    "secret.unbind",
];

pub fn is_owner_only(method: &str) -> bool {
    OWNER_ONLY.contains(&method)
}

/// [`dispatch`] for a surface outside the window (bridge, CLI, MCP): owner
/// decisions are refused whatever the token allows.
pub fn dispatch_external(
    st: &CompanyStore,
    method: &str,
    a: &Value,
    channel: &str,
    write: bool,
) -> Result<Value, String> {
    if is_owner_only(method) {
        return Err(format!(
            "{}: `{method}` is decided in the OmniGet window",
            super::ERR_COMPANY_FORBIDDEN
        ));
    }
    dispatch(st, method, a, &Actor::user_via(channel), write)
}

pub fn is_read_only(method: &str) -> bool {
    READ_ONLY.contains(&method)
}

fn arg<T: DeserializeOwned>(a: &Value, k: &str) -> Result<T, String> {
    serde_json::from_value(a.get(k).cloned().unwrap_or(Value::Null))
        .map_err(|e| invalid(format!("`{k}`: {e}")))
}

fn opt<T: DeserializeOwned>(a: &Value, k: &str) -> Result<Option<T>, String> {
    match a.get(k) {
        None | Some(Value::Null) => Ok(None),
        Some(v) => serde_json::from_value(v.clone())
            .map(Some)
            .map_err(|e| invalid(format!("`{k}`: {e}"))),
    }
}

fn s(a: &Value, k: &str) -> Result<String, String> {
    a.get(k)
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| invalid(format!("`{k}` is required")))
}

fn os(a: &Value, k: &str) -> Option<String> {
    a.get(k).and_then(Value::as_str).map(str::to_string)
}

fn ok<T: serde::Serialize>(v: T) -> Result<Value, String> {
    serde_json::to_value(v).map_err(invalid)
}

/// Runs `method` as `actor`. `write` = false refuses anything outside
/// [`READ_ONLY`].
pub fn dispatch(
    st: &CompanyStore,
    method: &str,
    a: &Value,
    actor: &Actor,
    write: bool,
) -> Result<Value, String> {
    if !write && !is_read_only(method) {
        return Err(format!(
            "{}: `{method}` needs write access",
            super::ERR_COMPANY_FORBIDDEN
        ));
    }
    let c = || s(a, "company");
    match method {
        // Companies
        "company.list" => ok(st.company_list()?),
        "company.get" => ok(st.company_get(&c()?)?),
        "company.create" => ok(st.company_create(arg::<NewCompany>(a, "company")?, actor)?),
        "company.update" => {
            ok(st.company_update(&c()?, arg::<CompanyPatch>(a, "patch")?, actor)?)
        }
        "company.status" => {
            ok(st.company_set_status(&c()?, &s(a, "status")?, os(a, "reason").as_deref(), actor)?)
        }
        "company.delete" => {
            st.company_delete(&c()?, actor)?;
            ok(json!({"ok": true}))
        }
        "company.dashboard" => st.dashboard(&c()?),
        "company.stats" => st.heartbeat_stats(&c()?),
        "company.templates" => ok(super::portable::template_list()),
        "company.export" => ok(st.export(
            &c()?,
            a.get("include_issues")
                .and_then(Value::as_bool)
                .unwrap_or(false),
        )?),
        "import.preview" => {
            let pkg = package_of(a)?;
            ok(st.import_preview(
                &pkg,
                &opt::<ImportOptions>(a, "options")?.unwrap_or_default(),
            )?)
        }
        "import.apply" => {
            let pkg = package_of(a)?;
            let o = opt::<ImportOptions>(a, "options")?.unwrap_or_default();
            ok(st.import_apply(&pkg, &o, &s(a, "plan_hash")?, actor)?)
        }
        // Staff
        "staff.list" => ok(st.staff_list(&c()?)?),
        "staff.get" => ok(st.staff_get(&c()?, &s(a, "staff")?)?),
        "staff.chart" => ok(st.org_chart(&c()?)?),
        "staff.hire" => ok(st.staff_hire(&c()?, arg::<NewStaff>(a, "staff")?, actor)?),
        "staff.update" => ok(st.staff_update(
            &c()?,
            &s(a, "staff")?,
            arg::<StaffPatch>(a, "patch")?,
            actor,
        )?),
        "staff.status" => ok(st.staff_set_status(
            &c()?,
            &s(a, "staff")?,
            &s(a, "action")?,
            os(a, "reason").as_deref(),
            actor,
        )?),
        // Goals and projects
        "goal.list" => ok(st.goal_list(&c()?)?),
        "goal.create" => ok(st.goal_create(&c()?, arg::<NewGoal>(a, "goal")?, actor)?),
        "goal.update" => ok(st.goal_update(
            &c()?,
            &s(a, "goal_id")?,
            arg::<GoalPatch>(a, "patch")?,
            actor,
        )?),
        "goal.delete" => {
            st.goal_delete(&c()?, &s(a, "goal_id")?, actor)?;
            ok(json!({"ok": true}))
        }
        "project.list" => ok(st.project_list(&c()?)?),
        "project.create" => {
            ok(st.project_create(&c()?, arg::<NewProject>(a, "project")?, actor)?)
        }
        "project.update" => ok(st.project_update(
            &c()?,
            &s(a, "project_id")?,
            arg::<ProjectPatch>(a, "patch")?,
            actor,
        )?),
        "project.delete" => {
            st.project_delete(&c()?, &s(a, "project_id")?, actor)?;
            ok(json!({"ok": true}))
        }
        // Issues
        "issue.list" => {
            ok(st.issue_list(&c()?, &opt::<IssueFilter>(a, "filter")?.unwrap_or_default())?)
        }
        "issue.get" => ok(st.issue_detail(&c()?, &s(a, "issue")?)?),
        "issue.create" => ok(st.issue_create(&c()?, arg::<NewIssue>(a, "issue")?, actor)?),
        "issue.update" => ok(st.issue_update(
            &c()?,
            &s(a, "issue")?,
            arg::<IssuePatch>(a, "patch")?,
            actor,
        )?),
        "issue.status" => ok(st.issue_transition(
            &c()?,
            &s(a, "issue")?,
            &s(a, "status")?,
            os(a, "note").as_deref(),
            a.get("expected_seq").and_then(Value::as_i64),
            actor,
        )?),
        "issue.checkout" => ok(st.issue_checkout(
            &c()?,
            &s(a, "issue")?,
            &s(a, "staff")?,
            &s(a, "run")?,
            a.get("lease_ms").and_then(Value::as_i64),
        )?),
        "issue.release" => {
            st.issue_release(&c()?, &s(a, "issue")?, &s(a, "run")?)?;
            ok(json!({"ok": true}))
        }
        "issue.delete" => {
            st.issue_delete(&c()?, &s(a, "issue")?, actor)?;
            ok(json!({"ok": true}))
        }
        "issue.tree" => {
            ok(json!({"changed": st.issue_tree(&c()?, &s(a, "issue")?, &s(a, "action")?, actor)?}))
        }
        "issue.comment" => ok(st.comment_add(
            &c()?,
            &s(a, "issue")?,
            &s(a, "body")?,
            os(a, "client_req").as_deref(),
            actor,
        )?),
        "issue.doc_put" => ok(st.doc_put(
            &c()?,
            &s(a, "issue")?,
            &s(a, "key")?,
            &s(a, "body")?,
            a.get("base_rev").and_then(Value::as_i64),
            os(a, "summary").as_deref(),
            actor,
        )?),
        "issue.doc_lock" => {
            st.doc_lock(
                &c()?,
                &s(a, "issue")?,
                &s(a, "key")?,
                a.get("locked").and_then(Value::as_bool).unwrap_or(true),
                actor,
            )?;
            ok(json!({"ok": true}))
        }
        "issue.revisions" => ok(st.doc_revisions(&c()?, &s(a, "issue")?, &s(a, "key")?)?),
        "issue.relate" => {
            st.relate(&c()?, &s(a, "from")?, &s(a, "to")?, &s(a, "kind")?, actor)?;
            ok(json!({"ok": true}))
        }
        "issue.unrelate" => {
            st.unrelate(&c()?, &s(a, "from")?, &s(a, "to")?, &s(a, "kind")?, actor)?;
            ok(json!({"ok": true}))
        }
        "issue.labels" => ok(st.issue_set_labels(
            &c()?,
            &s(a, "issue")?,
            &arg::<Vec<String>>(a, "labels")?,
            actor,
        )?),
        "issue.work_product" => ok(st.work_product_add(
            &c()?,
            &s(a, "issue")?,
            arg::<NewWorkProduct>(a, "work_product")?,
            actor,
        )?),
        "issue.work_product_review" => {
            st.work_product_review(&c()?, &s(a, "work_product")?, &s(a, "verdict")?, actor)?;
            ok(json!({"ok": true}))
        }
        "issue.search" => ok(st.issue_search(
            &c()?,
            &s(a, "query")?,
            a.get("limit").and_then(Value::as_u64).unwrap_or(30) as u32,
        )?),
        "label.list" => ok(st.labels_list(&c()?)?),
        "inbox.list" => {
            let company = c()?;
            ok(json!({
                "issues": st.inbox(&company)?,
                "approvals": st.approvals(Some(&company), Some("open"))?,
                "incidents": st.incidents(&company, true)?,
            }))
        }
        "inbox.mark" => {
            st.inbox_mark(
                &c()?,
                &s(a, "issue")?,
                &s(a, "action")?,
                a.get("until_ms").and_then(Value::as_i64),
            )?;
            ok(json!({"ok": true}))
        }
        // Governance
        "approval.list" => {
            ok(st.approvals(os(a, "company").as_deref(), os(a, "state").as_deref())?)
        }
        "approval.get" => ok(st.approval_get(&s(a, "approval")?)?),
        "approval.request" => {
            ok(st.approval_request(&c()?, arg::<NewApproval>(a, "approval")?, actor)?)
        }
        "approval.decide" => ok(st.approval_decide(
            &s(a, "approval")?,
            &s(a, "decision")?,
            os(a, "chosen").as_deref(),
            os(a, "note").as_deref(),
            a.get("expected_version").and_then(Value::as_i64),
            actor,
        )?),
        "budget.status" => ok(st.budget_status(&c()?)?),
        "budget.set" => ok(st.policy_upsert(&c()?, arg::<NewPolicy>(a, "policy")?, actor)?),
        "budget.delete" => {
            st.policy_delete(&c()?, &s(a, "policy_id")?, actor)?;
            ok(json!({"ok": true}))
        }
        "budget.incidents" => ok(st.incidents(
            &c()?,
            a.get("open_only").and_then(Value::as_bool).unwrap_or(false),
        )?),
        "budget.resolve" => ok(st.incident_resolve(
            &s(a, "incident")?,
            a.get("raise_to").and_then(Value::as_f64),
            actor,
        )?),
        "cost.summary" => {
            ok(st.cost_summary(&c()?, os(a, "window").as_deref().unwrap_or("month"))?)
        }
        // Heartbeats
        "run.list" => ok(st.runs(
            &c()?,
            os(a, "staff").as_deref(),
            os(a, "issue").as_deref(),
            a.get("limit").and_then(Value::as_u64).unwrap_or(50) as u32,
        )?),
        "run.get" => ok(st.run_get(&s(a, "run")?)?),
        "wake.list" => ok(st.wakes(
            &c()?,
            os(a, "state").as_deref(),
            a.get("limit").and_then(Value::as_u64).unwrap_or(100) as u32,
        )?),
        "staff.wake" => ok(
            json!({"wake": st.wake(&c()?, &s(a, "staff")?, os(a, "issue").as_deref(), os(a, "reason").as_deref().unwrap_or("woken by the owner"), actor)?}),
        ),
        "staff.scorecards" => ok(st.scorecards(
            &c()?,
            a.get("since_ms")
                .and_then(Value::as_i64)
                .unwrap_or(super::now_ms() - 30 * 86_400_000),
        )?),
        // Secrets (values only from the window)
        "secret.list" => ok(st.secret_bindings(&c()?)?),
        "secret.set" => {
            st.secret_set(&c()?, &s(a, "name")?, &s(a, "value")?, actor)?;
            ok(json!({"ok": true}))
        }
        "secret.bind" => {
            st.secret_bind(
                &c()?,
                &s(a, "staff")?,
                &s(a, "env_key")?,
                &s(a, "name")?,
                actor,
            )?;
            ok(json!({"ok": true}))
        }
        "secret.unbind" => {
            st.secret_unbind(&c()?, &s(a, "staff")?, &s(a, "env_key")?, actor)?;
            ok(json!({"ok": true}))
        }
        // Routines
        "routine.list" => ok(st.routine_list(&c()?)?),
        "routine.create" => {
            ok(st.routine_create(&c()?, arg::<NewRoutine>(a, "routine")?, actor)?)
        }
        "routine.update" => ok(st.routine_update(
            &c()?,
            &s(a, "routine_id")?,
            arg::<NewRoutine>(a, "routine")?,
            actor,
        )?),
        "routine.delete" => {
            st.routine_delete(&c()?, &s(a, "routine_id")?, actor)?;
            ok(json!({"ok": true}))
        }
        "routine.fire" => {
            ok(json!({"issue": st.routine_fire(&c()?, &s(a, "routine_id")?, actor)?}))
        }
        // Audit
        "audit.list" => {
            ok(st.audit_list(&c()?, &opt::<AuditQuery>(a, "query")?.unwrap_or_default())?)
        }
        "audit.verify" => ok(json!({"lines": st.audit_verify(&c()?)?, "intact": true})),
        "audit.csv" => ok(json!({"csv": st.audit_csv(&c()?)?})),
        other => Err(invalid(format!("unknown method `{other}`"))),
    }
}

fn package_of(a: &Value) -> Result<Package, String> {
    if let Some(t) = a.get("template").and_then(Value::as_str) {
        return super::portable::template(t).ok_or_else(|| invalid(format!("no template `{t}`")));
    }
    arg::<Package>(a, "package")
}

impl CompanyStore {
    /// Everything the company page shows at first paint, in one read.
    pub fn dashboard(&self, company: &str) -> Result<Value, String> {
        let c = self.company_get(company)?;
        let staff = self.staff_list(company)?;
        let counts: Vec<(String, i64)> = self.read(|db| {
            let mut stmt = db.prepare(
                "SELECT status, COUNT(*) FROM issues WHERE company_id = ?1 GROUP BY status",
            )?;
            let rows =
                stmt.query_map(rusqlite::params![company], |r| Ok((r.get(0)?, r.get(1)?)))?;
            rows.collect()
        })?;
        let runs = self.runs(company, None, None, 20)?;
        let approvals = self.approvals(Some(company), Some("open"))?;
        let costs = self.cost_summary(company, "month")?;
        let budgets = self.budget_status(company)?;
        let goals = self.goal_list(company)?;
        let projects = self.project_list(company)?;
        let live: Vec<&super::org::Staff> =
            staff.iter().filter(|s| s.status == "running").collect();
        Ok(json!({
            "company": c,
            "staff": staff,
            "issue_counts": counts.into_iter().map(|(k, v)| (k, json!(v))).collect::<serde_json::Map<String, Value>>(),
            "recent_runs": runs,
            "open_approvals": approvals.len(),
            "approvals": approvals,
            "costs": costs,
            "budgets": budgets,
            "goals": goals,
            "projects": projects,
            "running": live.len(),
        }))
    }
}

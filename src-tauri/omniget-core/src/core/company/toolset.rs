use std::sync::{Arc, OnceLock};

use async_trait::async_trait;
use serde_json::{json, Value};

use super::gov::NewApproval;
use super::issues::{IssueFilter, IssuePatch, NewIssue, NewWorkProduct};
use super::org::{Adapter, NewStaff, Staff};
use super::routines::NewRoutine;
use super::{store, Actor, CompanyStore, ERR_COMPANY_FORBIDDEN};
use crate::core::assist::ctx::AssistCtx;
use crate::core::assist::tools::AssistToolset;
use crate::core::llm::types::ToolSpec;

pub const ERR_COMPANY_TOOL: &str = "ERR_COMPANY_TOOL";

#[derive(Debug, Clone)]
pub struct ProvisionRequest {
    pub agent_id: String,
    pub name: String,
    pub role: String,
    pub instructions: String,
    pub template_agent_id: Option<String>,
    pub agent_def: Option<serde_json::Value>,
}

pub type ProvisionHook = Arc<dyn Fn(&ProvisionRequest) -> Result<(), String> + Send + Sync>;
static PROVISION: OnceLock<ProvisionHook> = OnceLock::new();

pub fn set_provision_hook(h: ProvisionHook) {
    let _ = PROVISION.set(h);
}

pub fn provision(req: &ProvisionRequest) -> Result<(), String> {
    match PROVISION.get() {
        Some(h) => h(req),
        None => Ok(()),
    }
}

pub fn tool_names() -> Vec<String> {
    specs().into_iter().map(|s| s.name).collect()
}

pub fn toolset() -> Arc<dyn AssistToolset> {
    Arc::new(CompanyTools)
}

struct CompanyTools;

fn spec(name: &str, description: &str, props: Value, required: &[&str]) -> ToolSpec {
    ToolSpec {
        name: name.into(),
        description: description.into(),
        input_schema: json!({"type": "object", "properties": props, "required": required, "additionalProperties": false}),
    }
}

fn specs() -> Vec<ToolSpec> {
    let id = json!({"type": "string", "description": "Issue id or identifier like ACME-12"});
    vec![
        spec("company_whoami", "Who you are in the company: your role, manager, reports, the mission and your budget.", json!({}), &[]),
        spec("company_my_issues", "Your open issues, most urgent first.", json!({"status": {"type": "array", "items": {"type": "string"}}}), &[]),
        spec("company_issues", "List the company's issues with filters.", json!({
            "status": {"type": "array", "items": {"type": "string"}}, "assignee": {"type": "string", "description": "Name, role or id"},
            "project": {"type": "string"}, "parent": {"type": "string"}, "label": {"type": "string"}, "limit": {"type": "integer"}}), &[]),
        spec("company_issue_get", "An issue with its comments, documents, work products, blockers and subtasks.", json!({"issue": id}), &["issue"]),
        spec("company_issue_create", "File an issue. Delegate by setting `parent` (your issue) and `assignee` (one of your reports).", json!({
            "title": {"type": "string"}, "body": {"type": "string"}, "assignee": {"type": "string", "description": "Name, role or id; `owner` for the human"},
            "parent": {"type": "string"}, "priority": {"type": "string", "enum": ["critical","high","medium","low"]},
            "project": {"type": "string"}, "blocked_by": {"type": "array", "items": {"type": "string"}},
            "labels": {"type": "array", "items": {"type": "string"}}, "work_mode": {"type": "string", "enum": ["standard","ask","planning"]},
            "review_policy": {"type": "string", "enum": ["none","anyone","not_creator","human_only"]}}), &["title"]),
        spec("company_issue_update", "Change an issue's title, description, priority, assignee or parent.", json!({
            "issue": id, "title": {"type": "string"}, "body": {"type": "string"}, "priority": {"type": "string"},
            "assignee": {"type": "string"}, "parent": {"type": "string"}, "expected_seq": {"type": "integer"}}), &["issue"]),
        spec("company_issue_status", "Move an issue: todo, in_progress, in_review, blocked (say why in `note`), done, cancelled. With a review policy, `done` goes to review first.", json!({
            "issue": id, "status": {"type": "string", "enum": ["backlog","todo","in_progress","in_review","blocked","done","cancelled"]},
            "note": {"type": "string"}}), &["issue", "status"]),
        spec("company_checkout", "Take an issue for this run (exactly one run holds it). Do this before working on it.", json!({"issue": id}), &["issue"]),
        spec("company_comment", "Comment on an issue. @name wakes that person; @owner reaches the human.", json!({"issue": id, "body": {"type": "string"}}), &["issue", "body"]),
        spec("company_doc_get", "Read a document of an issue (plan, spec, notes…).", json!({"issue": id, "key": {"type": "string"}}), &["issue", "key"]),
        spec("company_doc_put", "Write a document of an issue. Pass `base_rev` from the last read to avoid overwriting someone else.", json!({
            "issue": id, "key": {"type": "string"}, "body": {"type": "string"}, "base_rev": {"type": "integer"}, "summary": {"type": "string"}}), &["issue", "key", "body"]),
        spec("company_work_product", "Attach a deliverable to an issue: a file path, a URL, a PR, a report.", json!({
            "issue": id, "kind": {"type": "string"}, "title": {"type": "string"}, "url": {"type": "string"}, "path": {"type": "string"}, "summary": {"type": "string"}}), &["issue", "kind"]),
        spec("company_relate", "Link two issues: `blocks` (from blocks to), `related`, `duplicate`.", json!({
            "from": id, "to": id, "kind": {"type": "string", "enum": ["blocks","related","duplicate"]}}), &["from", "to", "kind"]),
        spec("company_search", "Search issues by words or identifier.", json!({"query": {"type": "string"}, "limit": {"type": "integer"}}), &["query"]),
        spec("company_org_chart", "The org chart: who reports to whom, roles and status.", json!({}), &[]),
        spec("company_goals", "The company's goals, from the mission down.", json!({}), &[]),
        spec("company_hire", "Hire someone into your branch (needs hiring rights; may wait for the owner's approval).", json!({
            "name": {"type": "string"}, "role": {"type": "string"}, "title": {"type": "string"},
            "responsibilities": {"type": "string"}, "instructions": {"type": "string"},
            "heartbeat_cron": {"type": "string", "description": "Optional cron for a scheduled check-in"}}), &["name", "role", "responsibilities"]),
        spec("company_request_approval", "Ask the owner to approve something (`strategy` for the company plan, `board_request` for anything else). Then stop; you are woken with the answer.", json!({
            "kind": {"type": "string", "enum": ["strategy","board_request"]}, "title": {"type": "string"}, "body": {"type": "string"}, "issue": {"type": "string"}}), &["kind", "title", "body"]),
        spec("company_ask_owner", "Ask the owner to choose between options. Then stop; you are woken with the choice (or `expired`).", json!({
            "question": {"type": "string"}, "options": {"type": "array", "items": {"type": "string"}, "minItems": 2},
            "details": {"type": "string"}, "issue": {"type": "string"}, "expires_hours": {"type": "number"}}), &["question", "options"]),
        spec("company_budget", "Your spend and the limits that apply to you.", json!({}), &[]),
        spec("company_routine_create", "Schedule recurring work: files an issue for an assignee on a cron (e.g. `0 9 * * 1` = Mondays 9:00).", json!({
            "title": {"type": "string"}, "body": {"type": "string"}, "assignee": {"type": "string"}, "cron": {"type": "string"},
            "priority": {"type": "string"}}), &["title", "cron"]),
        spec("company_wake", "Wake one of your reports now about an issue.", json!({"who": {"type": "string"}, "issue": {"type": "string"}, "reason": {"type": "string"}}), &["who"]),
    ]
}

fn err(msg: impl std::fmt::Display) -> String {
    format!("{ERR_COMPANY_TOOL}: {msg}")
}

fn s(input: &Value, k: &str) -> Option<String> {
    input
        .get(k)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(str::to_string)
}

fn req(input: &Value, k: &str) -> Result<String, String> {
    s(input, k).ok_or_else(|| err(format!("`{k}` is required")))
}

fn strs(input: &Value, k: &str) -> Vec<String> {
    input
        .get(k)
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

fn whoami(st: &CompanyStore, ctx: &AssistCtx) -> Result<Staff, String> {
    if let Some(conv) = &ctx.conversation_id {
        if let Some(rest) = conv.strip_prefix("company:") {
            if let Some((_, sid)) = rest.split_once(':') {
                let sid = sid.split(':').next().unwrap_or(sid);
                if let Ok(s) = st.staff_find(sid) {
                    return Ok(s);
                }
            }
        }
    }
    let bot = ctx
        .bot_id
        .as_deref()
        .ok_or_else(|| format!("{ERR_COMPANY_FORBIDDEN}: no agent identity"))?;
    let mut all = st.staff_by_agent(bot)?;
    match all.len() {
        0 => Err(format!(
            "{ERR_COMPANY_FORBIDDEN}: this agent works for no company"
        )),
        1 => Ok(all.remove(0)),
        _ => Err(err(
            "this agent works for several companies; open the run from the company",
        )),
    }
}

fn person(st: &CompanyStore, company: &str, who: &str) -> Result<Staff, String> {
    let all = st.staff_list(company)?;
    let w = who.trim().to_lowercase();
    let norm = |x: &str| x.to_lowercase().replace(['_', ' '], "-");
    all.iter()
        .filter(|s| s.status != "terminated")
        .find(|s| s.id == who || norm(&s.name) == norm(&w) || s.name.to_lowercase() == w)
        .or_else(|| {
            all.iter()
                .filter(|s| s.status != "terminated")
                .find(|s| norm(&s.role) == norm(&w) || norm(&s.title) == norm(&w))
        })
        .cloned()
        .ok_or_else(|| err(format!("nobody called `{who}` works here")))
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
    out.trim_matches('-').chars().take(24).collect()
}

pub(crate) fn staff_json(s: &Staff) -> Value {
    json!({"id": s.id, "name": s.name, "role": s.role, "title": s.title, "status": s.status, "reports_to": s.reports_to})
}

fn issue_json(i: &super::issues::Issue) -> Value {
    json!({"id": i.id, "identifier": i.identifier, "title": i.title, "status": i.status, "priority": i.priority,
           "assignee": i.assignee_staff_id, "assignee_owner": i.assignee_user, "parent": i.parent_id, "seq": i.seq,
           "work_mode": i.work_mode, "review_policy": i.review_policy})
}

#[async_trait]
impl AssistToolset for CompanyTools {
    fn name(&self) -> &'static str {
        "company"
    }

    fn specs(&self) -> Vec<ToolSpec> {
        specs()
    }

    async fn call(&self, ctx: &AssistCtx, tool: &str, input: Value) -> Result<Value, String> {
        let st = store()?;
        let me = whoami(&st, ctx)?;
        let run = st.live_run_of(&me.id)?.map(|r| r.id);
        let actor = Actor::agent(&me.id, run.as_deref());
        let c = me.company_id.clone();
        call(&st, &me, &actor, &c, run.as_deref(), tool, &input)
    }
}

pub fn call(
    st: &CompanyStore,
    me: &Staff,
    actor: &Actor,
    c: &str,
    run: Option<&str>,
    tool: &str,
    input: &Value,
) -> Result<Value, String> {
    match tool {
        "company_whoami" => {
            let company = st.company_get(c)?;
            let chain = st.chain_of_command(c, &me.id)?;
            let reports = st.staff_reports(c, &me.id)?;
            let budget = st.budget_status(c)?;
            let mine: Vec<Value> = budget
                .iter()
                .filter(|b| b.policy.scope_type == "company" || b.policy.scope_id == me.id)
                .map(|b| json!({"scope": b.policy.scope_type, "metric": b.policy.metric, "window": b.policy.window,
                                "used": b.used, "limit": b.policy.amount}))
                .collect();
            Ok(
                json!({"you": staff_json(me), "can_hire": me.can_hire, "company": {"name": company.name, "mission": company.mission, "status": company.status},
                      "manager": chain.first().map(staff_json), "reports": reports.iter().map(staff_json).collect::<Vec<_>>(), "budgets": mine}),
            )
        }
        "company_my_issues" => {
            let issues = st.issue_list(
                c,
                &IssueFilter {
                    assignee_staff_id: Some(me.id.clone()),
                    status: strs(input, "status"),
                    ..Default::default()
                },
            )?;
            Ok(json!({"issues": issues.iter().map(issue_json).collect::<Vec<_>>()}))
        }
        "company_issues" => {
            let assignee = match s(input, "assignee") {
                Some(a) => Some(person(st, c, &a)?.id),
                None => None,
            };
            let project = match s(input, "project") {
                Some(p) => st
                    .project_list(c)?
                    .into_iter()
                    .find(|x| x.id == p || x.name.eq_ignore_ascii_case(&p))
                    .map(|x| x.id),
                None => None,
            };
            let parent = match s(input, "parent") {
                Some(p) => Some(st.issue_get(c, &p)?.id),
                None => None,
            };
            let issues = st.issue_list(
                c,
                &IssueFilter {
                    status: strs(input, "status"),
                    assignee_staff_id: assignee,
                    project_id: project,
                    parent_id: parent,
                    label: s(input, "label"),
                    limit: input
                        .get("limit")
                        .and_then(Value::as_u64)
                        .map(|v| v as u32)
                        .or(Some(50)),
                    ..Default::default()
                },
            )?;
            Ok(json!({"issues": issues.iter().map(issue_json).collect::<Vec<_>>()}))
        }
        "company_issue_get" => {
            let d = st.issue_detail(c, &req(input, "issue")?)?;
            let names: std::collections::HashMap<String, String> = st
                .staff_list(c)?
                .into_iter()
                .map(|s| (s.id, s.name))
                .collect();
            Ok(json!({
                "issue": issue_json(&d.issue), "body": d.issue.body, "blocked_reason": d.issue.blocked_reason,
                "comments": d.comments.iter().rev().take(40).rev().map(|x| json!({"by": match x.author_kind.as_str() {
                    "user" => "owner".to_string(), "agent" => x.author_staff_id.as_ref().and_then(|i| names.get(i).cloned()).unwrap_or_else(|| "agent".into()), _ => "system".into() },
                    "body": x.body, "at": x.created_ms})).collect::<Vec<_>>(),
                "documents": d.documents.iter().map(|x| json!({"key": x.key, "rev": x.rev, "chars": x.body.len()})).collect::<Vec<_>>(),
                "work_products": d.work_products, "labels": d.labels.iter().map(|l| l.name.clone()).collect::<Vec<_>>(),
                "blocked_by": d.blocked_by, "blocking": d.blocking, "subtasks": d.children,
            }))
        }
        "company_issue_create" => {
            let (assignee, owner) = match s(input, "assignee").as_deref() {
                Some("owner") | Some("@owner") => (None, true),
                Some(a) => (Some(person(st, c, a)?.id), false),
                None => (None, false),
            };
            let project = match s(input, "project") {
                Some(p) => st
                    .project_list(c)?
                    .into_iter()
                    .find(|x| x.id == p || x.name.eq_ignore_ascii_case(&p))
                    .map(|x| x.id),
                None => None,
            };
            let i = st.issue_create(
                c,
                NewIssue {
                    title: req(input, "title")?,
                    body: s(input, "body").unwrap_or_default(),
                    parent_id: s(input, "parent"),
                    priority: s(input, "priority"),
                    assignee_staff_id: assignee,
                    assignee_user: owner,
                    project_id: project,
                    blocked_by: strs(input, "blocked_by"),
                    labels: strs(input, "labels"),
                    work_mode: s(input, "work_mode"),
                    review_policy: s(input, "review_policy"),
                    ..Default::default()
                },
                actor,
            )?;
            Ok(issue_json(&i))
        }
        "company_issue_update" => {
            let assignee = match s(input, "assignee").as_deref() {
                Some("owner") | Some("@owner") => None,
                Some(a) => Some(Some(person(st, c, a)?.id)),
                None => None,
            };
            let to_owner = matches!(s(input, "assignee").as_deref(), Some("owner" | "@owner"));
            let i = st.issue_update(
                c,
                &req(input, "issue")?,
                IssuePatch {
                    title: s(input, "title"),
                    body: s(input, "body"),
                    priority: s(input, "priority"),
                    assignee_staff_id: assignee,
                    assignee_user: to_owner.then_some(true),
                    parent_id: s(input, "parent").map(Some),
                    expected_seq: input.get("expected_seq").and_then(Value::as_i64),
                    ..Default::default()
                },
                actor,
            )?;
            Ok(issue_json(&i))
        }
        "company_issue_status" => {
            let i = st.issue_transition(
                c,
                &req(input, "issue")?,
                &req(input, "status")?,
                s(input, "note").as_deref(),
                None,
                actor,
            )?;
            Ok(issue_json(&i))
        }
        "company_checkout" => {
            let run = run.ok_or_else(|| err("no live run to hold the issue"))?;
            let i = st.issue_checkout(c, &req(input, "issue")?, &me.id, run, None)?;
            Ok(issue_json(&i))
        }
        "company_comment" => {
            let cm = st.comment_add(c, &req(input, "issue")?, &req(input, "body")?, None, actor)?;
            Ok(json!({"id": cm.id, "at": cm.created_ms}))
        }
        "company_doc_get" => {
            let d = st.issue_detail(c, &req(input, "issue")?)?;
            let key = req(input, "key")?.to_ascii_lowercase();
            let doc = d
                .documents
                .into_iter()
                .find(|x| x.key == key)
                .ok_or_else(|| err(format!("no `{key}` document yet")))?;
            Ok(json!({"key": doc.key, "rev": doc.rev, "body": doc.body, "locked": doc.locked}))
        }
        "company_doc_put" => {
            let d = st.doc_put(
                c,
                &req(input, "issue")?,
                &req(input, "key")?,
                input.get("body").and_then(Value::as_str).unwrap_or(""),
                input.get("base_rev").and_then(Value::as_i64),
                s(input, "summary").as_deref(),
                actor,
            )?;
            Ok(json!({"key": d.key, "rev": d.rev}))
        }
        "company_work_product" => {
            let wp = st.work_product_add(
                c,
                &req(input, "issue")?,
                NewWorkProduct {
                    kind: req(input, "kind")?,
                    title: s(input, "title").unwrap_or_default(),
                    url: s(input, "url"),
                    path: s(input, "path"),
                    summary: s(input, "summary"),
                },
                actor,
            )?;
            Ok(json!({"id": wp.id, "review": wp.review}))
        }
        "company_relate" => {
            st.relate(
                c,
                &req(input, "from")?,
                &req(input, "to")?,
                &req(input, "kind")?,
                actor,
            )?;
            Ok(json!({"ok": true}))
        }
        "company_search" => {
            let hits = st.issue_search(
                c,
                &req(input, "query")?,
                input.get("limit").and_then(Value::as_u64).unwrap_or(20) as u32,
            )?;
            Ok(json!({"hits": hits.iter().map(|h| issue_json(&h.issue)).collect::<Vec<_>>()}))
        }
        "company_org_chart" => {
            fn flat(n: &super::org::ChartNode, depth: usize, out: &mut Vec<Value>) {
                out.push(json!({"depth": depth, "name": n.staff.name, "role": n.staff.role, "title": n.staff.title, "status": n.staff.status, "id": n.staff.id}));
                for r in &n.reports {
                    flat(r, depth + 1, out);
                }
            }
            let mut out = Vec::new();
            for n in st.org_chart(c)? {
                flat(&n, 0, &mut out);
            }
            Ok(json!({"people": out}))
        }
        "company_goals" => {
            let g = st.goal_list(c)?;
            Ok(
                json!({"goals": g.iter().map(|x| json!({"id": x.id, "level": x.level, "title": x.title, "status": x.status, "parent": x.parent_id})).collect::<Vec<_>>()}),
            )
        }
        "company_hire" => {
            let name = req(input, "name")?;
            let company = st.company_get(c)?;
            let agent_id = format!(
                "co-{}-{}-{}",
                company.issue_prefix.to_lowercase(),
                slug(&name),
                &super::new_id()[..4]
            );
            let instructions = s(input, "instructions").unwrap_or_default();
            provision(&ProvisionRequest {
                agent_id: agent_id.clone(),
                name: name.clone(),
                role: req(input, "role")?,
                instructions: instructions.clone(),
                template_agent_id: Some(me.agent_id.clone()),
                agent_def: None,
            })?;
            let hired = st.staff_hire(
                c,
                NewStaff {
                    agent_id,
                    name,
                    role: req(input, "role")?,
                    title: s(input, "title").unwrap_or_default(),
                    reports_to: Some(me.id.clone()),
                    capabilities: req(input, "responsibilities")?,
                    instructions,
                    adapter: Adapter::Roster,
                    heartbeat_cron: s(input, "heartbeat_cron"),
                    ..Default::default()
                },
                actor,
            )?;
            Ok(
                json!({"hired": staff_json(&hired), "waiting_for_owner": hired.status == "pending_approval"}),
            )
        }
        "company_request_approval" => {
            let issue = match s(input, "issue") {
                Some(i) => Some(st.issue_get(c, &i)?.id),
                None => None,
            };
            let a = st.approval_request(
                c,
                NewApproval {
                    kind: req(input, "kind")?,
                    title: req(input, "title")?,
                    body: req(input, "body")?,
                    issue_id: issue,
                    ..Default::default()
                },
                actor,
            )?;
            Ok(
                json!({"approval": a.id, "state": a.state, "next": "stop now; you are woken with the answer"}),
            )
        }
        "company_ask_owner" => {
            let issue = match s(input, "issue") {
                Some(i) => Some(st.issue_get(c, &i)?.id),
                None => None,
            };
            let hours = input
                .get("expires_hours")
                .and_then(Value::as_f64)
                .unwrap_or(24.0 * 7.0)
                .clamp(0.1, 24.0 * 30.0);
            let options = strs(input, "options");
            let a = st.approval_request(c, NewApproval {
                kind: "decision".into(),
                title: req(input, "question")?,
                body: s(input, "details").unwrap_or_default(),
                issue_id: issue,
                payload: json!({"question": s(input, "question"), "options": options, "asked_by": me.id}),
                options: Some(options),
                expires_ms: Some(super::now_ms() + (hours * 3_600_000.0) as i64),
            }, actor)?;
            Ok(json!({"decision": a.id, "next": "stop now; you are woken with the choice"}))
        }
        "company_budget" => {
            let all = st.budget_status(c)?;
            let v: Vec<Value> = all
                .iter()
                .filter(|b| b.policy.scope_type != "agent" || b.policy.scope_id == me.id)
                .map(|b| json!({"scope": b.policy.scope_type, "metric": b.policy.metric, "window": b.policy.window,
                                "used": b.used, "limit": b.policy.amount, "pct": b.pct, "blocked": b.open_incident.as_ref().is_some_and(|i| i.kind == "hard")}))
                .collect();
            Ok(json!({"budgets": v}))
        }
        "company_routine_create" => {
            let who = match s(input, "assignee") {
                Some(a) => person(st, c, &a)?.id,
                None => me.id.clone(),
            };
            let r = st.routine_create(
                c,
                NewRoutine {
                    title: req(input, "title")?,
                    body: s(input, "body").unwrap_or_default(),
                    staff_id: who,
                    priority: s(input, "priority"),
                    cron: req(input, "cron")?,
                    ..Default::default()
                },
                actor,
            )?;
            Ok(json!({"routine": r.id, "next_run_ms": r.next_run_ms}))
        }
        "company_wake" => {
            let who = person(st, c, &req(input, "who")?)?;
            if who.reports_to.as_deref() != Some(me.id.as_str()) && who.id != me.id {
                return Err(format!(
                    "{ERR_COMPANY_FORBIDDEN}: you wake only your direct reports"
                ));
            }
            let id = st.wake(
                c,
                &who.id,
                s(input, "issue").as_deref(),
                &s(input, "reason").unwrap_or_else(|| format!("{} asked you to look", me.name)),
                actor,
            )?;
            Ok(json!({"wake": id}))
        }
        other => Err(err(format!("unknown tool `{other}`"))),
    }
}

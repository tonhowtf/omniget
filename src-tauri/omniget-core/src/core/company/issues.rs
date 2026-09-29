//! Issues: the unit of work of a company.
//!
//! One door for status changes ([`CompanyStore::issue_transition`]): the UI,
//! the bridge, the MCP tools and the scheduler all go through it, so the
//! transition table and the guards (assignee for `in_progress`, blockers
//! resolved, review policy for the verdict) hold everywhere. Paperclip's
//! `assertTransition` accepts any pair and the guards live in some routes
//! only (area A §3).
//!
//! Side effects are wakeups written in the same transaction: assigning wakes
//! the assignee, a mention wakes the mentioned, `in_review` wakes the
//! reviewer, `done` wakes whoever was waiting on it (dependents, the parent
//! once every child is finished).
//!
//! Checkout is a lease: one `UPDATE … WHERE (lease is free or expired)`, so
//! exactly one run wins and a dead run's lease frees itself by time.

use std::collections::HashSet;

use rusqlite::{params, OptionalExtension, Row, Transaction};
use serde::{Deserialize, Serialize};
use serde_json::json;

use super::heartbeat::{enqueue_wake, WakeSource};
use super::org::{get_company, get_staff, staff_row, Staff, STAFF_COLS};
use super::{
    audit, changed, conflict, forbidden, invalid, new_id, not_found, now_ms, sql, Actor, Change,
    CompanyStore,
};

/// How deep agents may delegate (issue created by an agent from an issue
/// created by an agent…). Stops runaway delegation loops.
pub const MAX_REQUEST_DEPTH: i64 = 8;
/// Default checkout lease; the engine renews it while the run is alive.
pub const LEASE_MS: i64 = 30 * 60 * 1000;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Issue {
    pub id: String,
    pub company_id: String,
    pub number: i64,
    pub identifier: String,
    pub project_id: Option<String>,
    pub goal_id: Option<String>,
    pub parent_id: Option<String>,
    pub title: String,
    pub body: String,
    pub status: String,
    pub seq: i64,
    pub priority: String,
    pub work_mode: String,
    pub review_policy: String,
    pub reviewer_staff_id: Option<String>,
    pub assignee_staff_id: Option<String>,
    pub assignee_user: bool,
    pub created_by_staff_id: Option<String>,
    pub created_by_user: bool,
    pub lease_run_id: Option<String>,
    pub lease_until_ms: Option<i64>,
    pub blocked_reason: Option<String>,
    pub billing_code: Option<String>,
    pub request_depth: i64,
    pub board_rank: f64,
    pub origin_kind: String,
    pub origin_id: Option<String>,
    pub created_ms: i64,
    pub updated_ms: i64,
    pub started_ms: Option<i64>,
    pub completed_ms: Option<i64>,
    pub cancelled_ms: Option<i64>,
    /// Label names; filled by [`CompanyStore::issue_list`] only.
    #[serde(default)]
    pub labels: Vec<String>,
}

pub(crate) const ISSUE_COLS: &str = "id, company_id, number, identifier, project_id, goal_id, parent_id,
  title, body, status, seq, priority, work_mode, review_policy, reviewer_staff_id, assignee_staff_id,
  assignee_user, created_by_staff_id, created_by_user, lease_run_id, lease_until_ms, blocked_reason,
  billing_code, request_depth, board_rank, origin_kind, origin_id, created_ms, updated_ms, started_ms,
  completed_ms, cancelled_ms";

pub(crate) fn issue_row(r: &Row) -> rusqlite::Result<Issue> {
    Ok(Issue {
        id: r.get(0)?,
        company_id: r.get(1)?,
        number: r.get(2)?,
        identifier: r.get(3)?,
        project_id: r.get(4)?,
        goal_id: r.get(5)?,
        parent_id: r.get(6)?,
        title: r.get(7)?,
        body: r.get(8)?,
        status: r.get(9)?,
        seq: r.get(10)?,
        priority: r.get(11)?,
        work_mode: r.get(12)?,
        review_policy: r.get(13)?,
        reviewer_staff_id: r.get(14)?,
        assignee_staff_id: r.get(15)?,
        assignee_user: r.get::<_, i64>(16)? != 0,
        created_by_staff_id: r.get(17)?,
        created_by_user: r.get::<_, i64>(18)? != 0,
        lease_run_id: r.get(19)?,
        lease_until_ms: r.get(20)?,
        blocked_reason: r.get(21)?,
        billing_code: r.get(22)?,
        request_depth: r.get(23)?,
        board_rank: r.get(24)?,
        origin_kind: r.get(25)?,
        origin_id: r.get(26)?,
        created_ms: r.get(27)?,
        updated_ms: r.get(28)?,
        started_ms: r.get(29)?,
        completed_ms: r.get(30)?,
        cancelled_ms: r.get(31)?,
        labels: Vec::new(),
    })
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct NewIssue {
    pub title: String,
    #[serde(default)]
    pub body: String,
    #[serde(default)]
    pub project_id: Option<String>,
    #[serde(default)]
    pub goal_id: Option<String>,
    #[serde(default)]
    pub parent_id: Option<String>,
    #[serde(default)]
    pub priority: Option<String>,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub assignee_staff_id: Option<String>,
    #[serde(default)]
    pub assignee_user: bool,
    #[serde(default)]
    pub work_mode: Option<String>,
    #[serde(default)]
    pub review_policy: Option<String>,
    #[serde(default)]
    pub reviewer_staff_id: Option<String>,
    #[serde(default)]
    pub labels: Vec<String>,
    #[serde(default)]
    pub blocked_by: Vec<String>,
    #[serde(default)]
    pub billing_code: Option<String>,
    #[serde(default)]
    pub origin_kind: Option<String>,
    #[serde(default)]
    pub origin_id: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct IssuePatch {
    pub title: Option<String>,
    pub body: Option<String>,
    pub priority: Option<String>,
    #[serde(default, deserialize_with = "super::double_option")]
    pub project_id: Option<Option<String>>,
    #[serde(default, deserialize_with = "super::double_option")]
    pub goal_id: Option<Option<String>>,
    #[serde(default, deserialize_with = "super::double_option")]
    pub parent_id: Option<Option<String>>,
    #[serde(default, deserialize_with = "super::double_option")]
    pub assignee_staff_id: Option<Option<String>>,
    pub assignee_user: Option<bool>,
    pub work_mode: Option<String>,
    pub review_policy: Option<String>,
    #[serde(default, deserialize_with = "super::double_option")]
    pub reviewer_staff_id: Option<Option<String>>,
    #[serde(default, deserialize_with = "super::double_option")]
    pub billing_code: Option<Option<String>>,
    pub board_rank: Option<f64>,
    /// Optimistic check: refuse if the issue moved since this `seq`.
    pub expected_seq: Option<i64>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct IssueFilter {
    #[serde(default)]
    pub status: Vec<String>,
    pub assignee_staff_id: Option<String>,
    pub assignee_user: Option<bool>,
    pub project_id: Option<String>,
    pub parent_id: Option<String>,
    pub goal_id: Option<String>,
    pub label: Option<String>,
    #[serde(default)]
    pub include_closed: bool,
    pub limit: Option<u32>,
    pub offset: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Comment {
    pub id: String,
    pub issue_id: String,
    pub author_kind: String,
    pub author_staff_id: Option<String>,
    pub run_id: Option<String>,
    pub body: String,
    pub created_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Document {
    pub id: String,
    pub issue_id: String,
    pub key: String,
    pub rev: i64,
    pub body: String,
    pub locked: bool,
    pub updated_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DocumentRev {
    pub rev: i64,
    pub body: String,
    pub summary: Option<String>,
    pub author: String,
    pub run_id: Option<String>,
    pub created_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkProduct {
    pub id: String,
    pub issue_id: String,
    pub kind: String,
    pub title: String,
    pub url: Option<String>,
    pub path: Option<String>,
    pub summary: Option<String>,
    pub run_id: Option<String>,
    pub review: String,
    pub created_ms: i64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct NewWorkProduct {
    pub kind: String,
    #[serde(default)]
    pub title: String,
    pub url: Option<String>,
    pub path: Option<String>,
    pub summary: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Label {
    pub id: String,
    pub name: String,
    pub color: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IssueLink {
    pub id: String,
    pub identifier: String,
    pub title: String,
    pub status: String,
    pub kind: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct IssueDetail {
    pub issue: Issue,
    pub comments: Vec<Comment>,
    pub documents: Vec<Document>,
    pub work_products: Vec<WorkProduct>,
    pub labels: Vec<Label>,
    pub blocked_by: Vec<IssueLink>,
    pub blocking: Vec<IssueLink>,
    pub related: Vec<IssueLink>,
    pub children: Vec<IssueLink>,
    pub assignee: Option<Staff>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SearchHit {
    pub issue: Issue,
    pub score: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct InboxItem {
    pub issue: Issue,
    pub why: String,
    pub read: bool,
}

fn priority_ok(p: &str) -> bool {
    matches!(p, "critical" | "high" | "medium" | "low")
}

pub(crate) fn priority_rank(p: &str) -> i64 {
    match p {
        "critical" => 0,
        "high" => 1,
        "medium" => 2,
        _ => 3,
    }
}

/// The transition table (SPEC-implementation §8.2, plus reopen for the
/// owner and `todo → backlog`).
pub fn can_go(from: &str, to: &str) -> bool {
    matches!(
        (from, to),
        ("backlog", "todo" | "cancelled")
            | ("todo", "in_progress" | "blocked" | "cancelled" | "backlog")
            | (
                "in_progress",
                "in_review" | "blocked" | "done" | "cancelled" | "todo"
            )
            | ("in_review", "in_progress" | "done" | "cancelled" | "todo")
            | ("blocked", "todo" | "in_progress" | "cancelled")
            | ("done" | "cancelled", "todo")
    )
}

pub(crate) fn get_issue(
    tx: &rusqlite::Connection,
    company: &str,
    id: &str,
) -> Result<Issue, String> {
    tx.query_row(
        &format!("SELECT {ISSUE_COLS} FROM issues WHERE company_id = ?1 AND (id = ?2 OR identifier = ?2 COLLATE NOCASE)"),
        params![company, id],
        issue_row,
    )
    .optional()
    .map_err(sql)?
    .ok_or_else(|| not_found("issue", id))
}

fn open_blockers(
    tx: &rusqlite::Connection,
    company: &str,
    id: &str,
) -> Result<Vec<String>, String> {
    let mut st = tx
        .prepare(
            "SELECT i.identifier FROM relations r JOIN issues i ON i.id = r.from_id
             WHERE r.company_id = ?1 AND r.to_id = ?2 AND r.kind = 'blocks'
               AND i.status NOT IN ('done','cancelled')",
        )
        .map_err(sql)?;
    let rows = st
        .query_map(params![company, id], |r| r.get::<_, String>(0))
        .map_err(sql)?;
    rows.collect::<Result<Vec<_>, _>>().map_err(sql)
}

fn parent_cycle(
    tx: &rusqlite::Connection,
    company: &str,
    id: &str,
    parent: &str,
) -> Result<bool, String> {
    if id == parent {
        return Ok(true);
    }
    tx.query_row(
        "WITH RECURSIVE up(id, depth) AS (
           SELECT parent_id, 1 FROM issues WHERE company_id = ?1 AND id = ?2
           UNION
           SELECT i.parent_id, up.depth + 1 FROM issues i JOIN up ON i.id = up.id
           WHERE i.company_id = ?1 AND up.depth < 256
         ) SELECT EXISTS (SELECT 1 FROM up WHERE id = ?3)",
        params![company, parent, id],
        |r| r.get::<_, bool>(0),
    )
    .map_err(sql)
}

fn blocks_cycle(
    tx: &rusqlite::Connection,
    company: &str,
    from: &str,
    to: &str,
) -> Result<bool, String> {
    // Adding from → to closes a loop if `from` is already reachable from `to`.
    tx.query_row(
        "WITH RECURSIVE down(id, depth) AS (
           SELECT to_id, 1 FROM relations WHERE company_id = ?1 AND from_id = ?2 AND kind = 'blocks'
           UNION
           SELECT r.to_id, down.depth + 1 FROM relations r JOIN down ON r.from_id = down.id
           WHERE r.company_id = ?1 AND r.kind = 'blocks' AND down.depth < 512
         ) SELECT EXISTS (SELECT 1 FROM down WHERE id = ?3)",
        params![company, to, from],
        |r| r.get::<_, bool>(0),
    )
    .map_err(sql)
}

/// `@name` mentions → staff ids (by name or role, case-insensitive,
/// spaces as `-` or `_` also match).
pub(crate) fn mentions(
    tx: &rusqlite::Connection,
    company: &str,
    body: &str,
) -> Result<Vec<Staff>, String> {
    let tags: HashSet<String> = body
        .split(|c: char| {
            c.is_whitespace() || matches!(c, ',' | '.' | ';' | ':' | '!' | '?' | '(' | ')')
        })
        .filter_map(|w| w.strip_prefix('@'))
        .filter(|w| !w.is_empty())
        .map(|w| w.to_lowercase())
        .collect();
    if tags.is_empty() {
        return Ok(Vec::new());
    }
    let mut st = tx
        .prepare(&format!(
            "SELECT {STAFF_COLS} FROM staff WHERE company_id = ?1 AND status NOT IN ('terminated','pending_approval')"
        ))
        .map_err(sql)?;
    let all = st
        .query_map(params![company], staff_row)
        .map_err(sql)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(sql)?;
    let norm = |s: &str| s.to_lowercase().replace([' ', '_'], "-");
    Ok(all
        .into_iter()
        .filter(|s| {
            tags.iter().any(|t| {
                let t = t.replace('_', "-");
                norm(&s.name) == t || norm(&s.role) == t || norm(&s.title) == t
            })
        })
        .collect())
}

fn system_comment(tx: &Transaction, company: &str, issue: &str, body: &str) -> Result<(), String> {
    tx.execute(
        "INSERT INTO comments (id, company_id, issue_id, author_kind, body, created_ms)
         VALUES (?1, ?2, ?3, 'system', ?4, ?5)",
        params![new_id(), company, issue, body, now_ms()],
    )
    .map_err(sql)?;
    Ok(())
}

fn wake_assignee(
    tx: &Transaction,
    i: &Issue,
    source: WakeSource,
    reason: &str,
    actor: &Actor,
) -> Result<(), String> {
    if let Some(s) = &i.assignee_staff_id {
        // An agent does not wake itself for its own change.
        if actor.staff_id() != Some(s.as_str()) {
            enqueue_wake(
                tx,
                &i.company_id,
                s,
                Some(&i.id),
                source,
                reason,
                json!({}),
                priority_rank(&i.priority),
            )?;
        }
    }
    Ok(())
}

/// After `id` reached `done`: wake the assignees of issues it was blocking
/// (when nothing else blocks them) and the parent's assignee once every
/// child is finished.
fn wake_waiters(tx: &Transaction, company: &str, id: &str, actor: &Actor) -> Result<(), String> {
    let mut st = tx
        .prepare(&format!(
            "SELECT {ISSUE_COLS} FROM issues WHERE company_id = ?1 AND id IN
               (SELECT to_id FROM relations WHERE company_id = ?1 AND from_id = ?2 AND kind = 'blocks')
             AND status NOT IN ('done','cancelled')"
        ))
        .map_err(sql)?;
    let dependents = st
        .query_map(params![company, id], issue_row)
        .map_err(sql)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(sql)?;
    for d in dependents {
        if open_blockers(tx, company, &d.id)?.is_empty() {
            if d.status == "blocked" {
                tx.execute(
                    "UPDATE issues SET status = 'todo', blocked_reason = NULL, seq = seq + 1, updated_ms = ?2 WHERE id = ?1",
                    params![d.id, now_ms()],
                )
                .map_err(sql)?;
            }
            wake_assignee(tx, &d, WakeSource::Dependency, "a blocker is done", actor)?;
        }
    }
    let parent: Option<String> = tx
        .query_row(
            "SELECT parent_id FROM issues WHERE id = ?1",
            params![id],
            |r| r.get(0),
        )
        .map_err(sql)?;
    if let Some(p) = parent {
        let open: i64 = tx
            .query_row(
                "SELECT COUNT(*) FROM issues WHERE company_id = ?1 AND parent_id = ?2 AND status NOT IN ('done','cancelled')",
                params![company, p],
                |r| r.get(0),
            )
            .map_err(sql)?;
        if open == 0 {
            let pi = get_issue(tx, company, &p)?;
            if !matches!(pi.status.as_str(), "done" | "cancelled") {
                wake_assignee(
                    tx,
                    &pi,
                    WakeSource::ChildrenDone,
                    "every subtask is finished",
                    actor,
                )?;
            }
        }
    }
    Ok(())
}

fn links(
    tx: &rusqlite::Connection,
    q: &str,
    company: &str,
    id: &str,
) -> Result<Vec<IssueLink>, String> {
    let mut st = tx.prepare(q).map_err(sql)?;
    let rows = st
        .query_map(params![company, id], |r| {
            Ok(IssueLink {
                id: r.get(0)?,
                identifier: r.get(1)?,
                title: r.get(2)?,
                status: r.get(3)?,
                kind: r.get(4)?,
            })
        })
        .map_err(sql)?;
    rows.collect::<Result<Vec<_>, _>>().map_err(sql)
}

fn stamp(i: &mut Issue, to: &str, now: i64) {
    if to == "in_progress" && i.started_ms.is_none() {
        i.started_ms = Some(now);
    }
    if to == "done" {
        i.completed_ms = Some(now);
    }
    if to == "cancelled" {
        i.cancelled_ms = Some(now);
    }
    if to == "todo" {
        i.completed_ms = None;
        i.cancelled_ms = None;
    }
}

impl CompanyStore {
    pub fn issue_create(
        &self,
        company: &str,
        new: NewIssue,
        actor: &Actor,
    ) -> Result<Issue, String> {
        self.write(|tx, ch| issue_create_tx(tx, ch, company, new, actor))
    }

    pub fn issue_get(&self, company: &str, id: &str) -> Result<Issue, String> {
        self.read(|c| {
            c.query_row(
                &format!("SELECT {ISSUE_COLS} FROM issues WHERE company_id = ?1 AND (id = ?2 OR identifier = ?2 COLLATE NOCASE)"),
                params![company, id],
                issue_row,
            )
            .optional()
        })?
        .ok_or_else(|| not_found("issue", id))
    }

    pub fn issue_detail(&self, company: &str, id: &str) -> Result<IssueDetail, String> {
        let issue = self.issue_get(company, id)?;
        let iid = issue.id.clone();
        self.read(|c| {
            let map_err = |e: String| rusqlite::Error::InvalidParameterName(e);
            let comments = {
                let mut st = c.prepare(
                    "SELECT id, issue_id, author_kind, author_staff_id, run_id, body, created_ms
                     FROM comments WHERE issue_id = ?1 ORDER BY created_ms, rowid",
                )?;
                let rows = st.query_map(params![iid], |r| {
                    Ok(Comment {
                        id: r.get(0)?,
                        issue_id: r.get(1)?,
                        author_kind: r.get(2)?,
                        author_staff_id: r.get(3)?,
                        run_id: r.get(4)?,
                        body: r.get(5)?,
                        created_ms: r.get(6)?,
                    })
                })?;
                rows.collect::<Result<Vec<_>, _>>()?
            };
            let documents = {
                let mut st = c.prepare(
                    "SELECT id, issue_id, key, rev, body, locked, updated_ms FROM documents WHERE issue_id = ?1 ORDER BY key",
                )?;
                let rows = st.query_map(params![iid], doc_row)?;
                rows.collect::<Result<Vec<_>, _>>()?
            };
            let work_products = {
                let mut st = c.prepare(
                    "SELECT id, issue_id, kind, title, url, path, summary, run_id, review, created_ms
                     FROM work_products WHERE issue_id = ?1 ORDER BY created_ms",
                )?;
                let rows = st.query_map(params![iid], wp_row)?;
                rows.collect::<Result<Vec<_>, _>>()?
            };
            let labels = {
                let mut st = c.prepare(
                    "SELECT l.id, l.name, l.color FROM labels l JOIN issue_labels il ON il.label_id = l.id
                     WHERE il.issue_id = ?1 ORDER BY l.name",
                )?;
                let rows = st.query_map(params![iid], |r| {
                    Ok(Label { id: r.get(0)?, name: r.get(1)?, color: r.get(2)? })
                })?;
                rows.collect::<Result<Vec<_>, _>>()?
            };
            let blocked_by = links(c, "SELECT i.id, i.identifier, i.title, i.status, r.kind FROM relations r JOIN issues i ON i.id = r.from_id WHERE r.company_id = ?1 AND r.to_id = ?2 AND r.kind = 'blocks'", company, &iid).map_err(map_err)?;
            let blocking = links(c, "SELECT i.id, i.identifier, i.title, i.status, r.kind FROM relations r JOIN issues i ON i.id = r.to_id WHERE r.company_id = ?1 AND r.from_id = ?2 AND r.kind = 'blocks'", company, &iid).map_err(map_err)?;
            let related = links(c, "SELECT i.id, i.identifier, i.title, i.status, r.kind FROM relations r JOIN issues i ON i.id = CASE WHEN r.from_id = ?2 THEN r.to_id ELSE r.from_id END WHERE r.company_id = ?1 AND (r.from_id = ?2 OR r.to_id = ?2) AND r.kind <> 'blocks'", company, &iid).map_err(map_err)?;
            let children = links(c, "SELECT id, identifier, title, status, 'child' FROM issues WHERE company_id = ?1 AND parent_id = ?2 ORDER BY number", company, &iid).map_err(map_err)?;
            let assignee = match &issue.assignee_staff_id {
                Some(s) => get_staff(c, company, s).ok(),
                None => None,
            };
            Ok(IssueDetail {
                issue: issue.clone(),
                comments,
                documents,
                work_products,
                labels,
                blocked_by,
                blocking,
                related,
                children,
                assignee,
            })
        })
    }

    pub fn issue_list(&self, company: &str, f: &IssueFilter) -> Result<Vec<Issue>, String> {
        let limit = f.limit.unwrap_or(500).clamp(1, 5000) as i64;
        let offset = f.offset.unwrap_or(0) as i64;
        let statuses = if f.status.is_empty() {
            None
        } else {
            Some(serde_json::to_string(&f.status).unwrap_or_default())
        };
        self.read(|c| {
            let mut st = c.prepare(&format!(
                "SELECT {ISSUE_COLS} FROM issues
                 WHERE company_id = ?1
                   AND (?2 IS NULL OR status IN (SELECT value FROM json_each(?2)))
                   AND (?2 IS NOT NULL OR ?3 = 1 OR status NOT IN ('done','cancelled'))
                   AND (?4 IS NULL OR assignee_staff_id = ?4)
                   AND (?5 IS NULL OR assignee_user = ?5)
                   AND (?6 IS NULL OR project_id = ?6)
                   AND (?7 IS NULL OR parent_id = ?7)
                   AND (?8 IS NULL OR goal_id = ?8)
                   AND (?9 IS NULL OR id IN (SELECT il.issue_id FROM issue_labels il JOIN labels l ON l.id = il.label_id
                                             WHERE l.company_id = ?1 AND l.name = ?9 COLLATE NOCASE))
                 ORDER BY CASE priority WHEN 'critical' THEN 0 WHEN 'high' THEN 1 WHEN 'medium' THEN 2 ELSE 3 END,
                          board_rank, number DESC
                 LIMIT ?10 OFFSET ?11"
            ))?;
            let rows = st.query_map(
                params![
                    company,
                    statuses,
                    f.include_closed as i64,
                    f.assignee_staff_id,
                    f.assignee_user.map(|b| b as i64),
                    f.project_id,
                    f.parent_id,
                    f.goal_id,
                    f.label,
                    limit,
                    offset
                ],
                issue_row,
            )?;
            let mut list: Vec<Issue> = rows.collect::<Result<_, _>>()?;
            let mut st = c.prepare(
                "SELECT il.issue_id, l.name FROM issue_labels il JOIN labels l ON l.id = il.label_id
                 WHERE l.company_id = ?1 ORDER BY l.name",
            )?;
            let mut by: std::collections::HashMap<String, Vec<String>> = std::collections::HashMap::new();
            for row in st.query_map(params![company], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))? {
                let (i, n) = row?;
                by.entry(i).or_default().push(n);
            }
            for i in &mut list {
                if let Some(l) = by.remove(&i.id) {
                    i.labels = l;
                }
            }
            Ok(list)
        })
    }

    pub fn issue_update(
        &self,
        company: &str,
        id: &str,
        patch: IssuePatch,
        actor: &Actor,
    ) -> Result<Issue, String> {
        self.write(|tx, ch| {
            let mut i = get_issue(tx, company, id)?;
            let id = i.id.clone();
            if let Some(seq) = patch.expected_seq {
                if seq != i.seq {
                    return Err(conflict(format!("{} changed (seq {} ≠ {seq})", i.identifier, i.seq)));
                }
            }
            let before_assignee = i.assignee_staff_id.clone();
            let mut fields = Vec::new();
            if let Some(v) = patch.title {
                if v.trim().is_empty() {
                    return Err(invalid("an issue needs a title"));
                }
                i.title = v.trim().into();
                fields.push("title");
            }
            if let Some(v) = patch.body {
                i.body = v;
                fields.push("body");
            }
            if let Some(v) = patch.priority {
                if !priority_ok(&v) {
                    return Err(invalid("priority is critical, high, medium or low"));
                }
                i.priority = v;
                fields.push("priority");
            }
            if let Some(v) = patch.project_id {
                i.project_id = v;
                fields.push("project");
            }
            if let Some(v) = patch.goal_id {
                i.goal_id = v;
                fields.push("goal");
            }
            if let Some(v) = patch.parent_id {
                if let Some(p) = &v {
                    let p = get_issue(tx, company, p)?.id;
                    if parent_cycle(tx, company, &id, &p)? {
                        return Err(invalid("that would make a loop of subtasks"));
                    }
                    i.parent_id = Some(p);
                } else {
                    i.parent_id = None;
                }
                fields.push("parent");
            }
            if let Some(v) = patch.assignee_staff_id {
                if let Some(s) = &v {
                    let st = get_staff(tx, company, s)?;
                    if matches!(st.status.as_str(), "terminated" | "pending_approval") {
                        return Err(invalid(format!("{} cannot take work ({})", st.name, st.status)));
                    }
                    i.assignee_user = false;
                }
                i.assignee_staff_id = v;
                if i.assignee_staff_id.is_none() && i.status == "in_progress" && !i.assignee_user {
                    i.status = "todo".into();
                }
                i.lease_run_id = None;
                i.lease_until_ms = None;
                fields.push("assignee");
            }
            if let Some(v) = patch.assignee_user {
                i.assignee_user = v;
                if v {
                    i.assignee_staff_id = None;
                }
                fields.push("assignee");
            }
            if let Some(v) = patch.work_mode {
                if !matches!(v.as_str(), "standard" | "ask" | "planning") {
                    return Err(invalid("work mode is standard, ask or planning"));
                }
                i.work_mode = v;
                fields.push("work_mode");
            }
            if let Some(v) = patch.review_policy {
                if !matches!(v.as_str(), "none" | "anyone" | "not_creator" | "human_only") {
                    return Err(invalid("unknown review policy"));
                }
                // Loosening a restrictive policy is the owner's call.
                if !actor.is_user() && i.review_policy != "none" && v != i.review_policy {
                    return Err(forbidden("only the owner changes a review policy"));
                }
                i.review_policy = v;
                fields.push("review_policy");
            }
            if let Some(v) = patch.reviewer_staff_id {
                if let Some(s) = &v {
                    get_staff(tx, company, s)?;
                }
                i.reviewer_staff_id = v;
                fields.push("reviewer");
            }
            if let Some(v) = patch.billing_code {
                i.billing_code = v;
            }
            if let Some(v) = patch.board_rank {
                i.board_rank = v;
            }
            let now = now_ms();
            tx.execute(
                "UPDATE issues SET title=?2, body=?3, priority=?4, project_id=?5, goal_id=?6, parent_id=?7,
                   assignee_staff_id=?8, assignee_user=?9, work_mode=?10, review_policy=?11, reviewer_staff_id=?12,
                   billing_code=?13, board_rank=?14, status=?15, lease_run_id=?16, lease_until_ms=?17,
                   seq = seq + 1, updated_ms=?18
                 WHERE id = ?1",
                params![
                    id, i.title, i.body, i.priority, i.project_id, i.goal_id, i.parent_id,
                    i.assignee_staff_id, i.assignee_user as i64, i.work_mode, i.review_policy,
                    i.reviewer_staff_id, i.billing_code, i.board_rank, i.status, i.lease_run_id,
                    i.lease_until_ms, now
                ],
            )
            .map_err(|e| match e {
                rusqlite::Error::SqliteFailure(f, _) if f.code == rusqlite::ErrorCode::ConstraintViolation => {
                    invalid("a linked project, goal, parent or agent is not in this company")
                }
                e => sql(e),
            })?;
            if i.assignee_staff_id.is_some()
                && i.assignee_staff_id != before_assignee
                && !matches!(i.status.as_str(), "done" | "cancelled" | "backlog")
            {
                wake_assignee(tx, &i, WakeSource::Assignment, "you were assigned", actor)?;
            }
            if !fields.is_empty() {
                audit::record(tx, company, actor, "issue.updated", "issue", &id, Some(json!({"fields": fields})))?;
            }
            changed(ch, company, "issue", &id);
            get_issue(tx, company, &id)
        })
    }

    /// The one door for status changes.
    pub fn issue_transition(
        &self,
        company: &str,
        id: &str,
        to: &str,
        note: Option<&str>,
        expected_seq: Option<i64>,
        actor: &Actor,
    ) -> Result<Issue, String> {
        self.write(|tx, ch| transition_tx(tx, ch, company, id, to, note, expected_seq, actor))
    }

    /// Atomic checkout: exactly one run holds an issue at a time. Also moves
    /// `todo`/`blocked`/`backlog` to `in_progress` and makes the caller the
    /// assignee if nobody is.
    pub fn issue_checkout(
        &self,
        company: &str,
        id: &str,
        staff: &str,
        run_id: &str,
        lease_ms: Option<i64>,
    ) -> Result<Issue, String> {
        self.write(|tx, ch| checkout_tx(tx, ch, company, id, staff, run_id, lease_ms))
    }

    /// Frees the lease held by `run_id` (no-op if another run holds it).
    pub fn issue_release(&self, company: &str, id: &str, run_id: &str) -> Result<(), String> {
        self.write(|tx, ch| {
            let n = tx
                .execute(
                    "UPDATE issues SET lease_run_id = NULL, lease_until_ms = NULL, updated_ms = ?3
                     WHERE company_id = ?1 AND id = ?2 AND lease_run_id = ?4",
                    params![company, id, now_ms(), run_id],
                )
                .map_err(sql)?;
            if n > 0 {
                changed(ch, company, "issue", id);
            }
            Ok(())
        })
    }

    pub fn comment_add(
        &self,
        company: &str,
        issue: &str,
        body: &str,
        client_req: Option<&str>,
        actor: &Actor,
    ) -> Result<Comment, String> {
        self.write(|tx, ch| comment_tx(tx, ch, company, issue, body, client_req, actor))
    }

    #[allow(clippy::too_many_arguments)]
    pub fn doc_put(
        &self,
        company: &str,
        issue: &str,
        key: &str,
        body: &str,
        base_rev: Option<i64>,
        summary: Option<&str>,
        actor: &Actor,
    ) -> Result<Document, String> {
        let key = key.trim().to_ascii_lowercase();
        if key.is_empty()
            || key.len() > 64
            || !key
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        {
            return Err(invalid("a document key is 1-64 letters, digits, - or _"));
        }
        self.write(|tx, ch| {
            let i = get_issue(tx, company, issue)?;
            let cur = tx
                .query_row(
                    "SELECT id, issue_id, key, rev, body, locked, updated_ms FROM documents WHERE issue_id = ?1 AND key = ?2",
                    params![i.id, key],
                    doc_row,
                )
                .optional()
                .map_err(sql)?;
            let now = now_ms();
            let author = match actor {
                Actor::Agent { staff_id, .. } => staff_id.clone(),
                Actor::User { .. } => "owner".into(),
                Actor::System { what } => what.clone(),
            };
            let doc = match cur {
                Some(d) => {
                    if d.locked && !actor.is_user() {
                        return Err(conflict(format!("the `{key}` document is locked by the owner")));
                    }
                    if let Some(b) = base_rev {
                        if b != d.rev {
                            return Err(conflict(format!("`{key}` is at revision {}, not {b}", d.rev)));
                        }
                    }
                    if d.body == body {
                        return Ok(d);
                    }
                    let rev = d.rev + 1;
                    tx.execute(
                        "UPDATE documents SET rev = ?2, body = ?3, updated_ms = ?4 WHERE id = ?1",
                        params![d.id, rev, body, now],
                    )
                    .map_err(sql)?;
                    tx.execute(
                        "INSERT INTO document_revs (doc_id, rev, body, summary, author, run_id, created_ms) VALUES (?1,?2,?3,?4,?5,?6,?7)",
                        params![d.id, rev, body, summary, author, actor.run_id(), now],
                    )
                    .map_err(sql)?;
                    Document { rev, body: body.into(), updated_ms: now, ..d }
                }
                None => {
                    let did = new_id();
                    tx.execute(
                        "INSERT INTO documents (id, company_id, issue_id, key, rev, body, updated_ms) VALUES (?1,?2,?3,?4,1,?5,?6)",
                        params![did, company, i.id, key, body, now],
                    )
                    .map_err(sql)?;
                    tx.execute(
                        "INSERT INTO document_revs (doc_id, rev, body, summary, author, run_id, created_ms) VALUES (?1,1,?2,?3,?4,?5,?6)",
                        params![did, body, summary, author, actor.run_id(), now],
                    )
                    .map_err(sql)?;
                    Document { id: did, issue_id: i.id.clone(), key: key.clone(), rev: 1, body: body.into(), locked: false, updated_ms: now }
                }
            };
            // A new plan revision in planning mode asks the owner to approve
            // exactly that revision (Paperclip ties plan approval to it).
            if key == "plan" && i.work_mode == "planning" && !actor.is_user() {
                super::gov::open_approval(
                    tx,
                    company,
                    super::gov::NewApproval {
                        kind: "plan".into(),
                        title: format!("Approve the plan for {}", i.identifier),
                        body: body.chars().take(4000).collect(),
                        issue_id: Some(i.id.clone()),
                        payload: json!({"issue_id": i.id, "doc_id": doc.id, "rev": doc.rev}),
                        options: None,
                        expires_ms: None,
                    },
                    actor,
                    ch,
                )?;
            }
            audit::record(tx, company, actor, "document.saved", "issue", &i.id, Some(json!({"key": key, "rev": doc.rev})))?;
            changed(ch, company, "issue", &i.id);
            Ok(doc)
        })
    }

    pub fn doc_lock(
        &self,
        company: &str,
        issue: &str,
        key: &str,
        locked: bool,
        actor: &Actor,
    ) -> Result<(), String> {
        if !actor.is_user() {
            return Err(forbidden("only the owner locks documents"));
        }
        self.write(|tx, ch| {
            let i = get_issue(tx, company, issue)?;
            tx.execute(
                "UPDATE documents SET locked = ?3 WHERE issue_id = ?1 AND key = ?2",
                params![i.id, key, locked as i64],
            )
            .map_err(sql)?;
            changed(ch, company, "issue", &i.id);
            Ok(())
        })
    }

    pub fn doc_revisions(
        &self,
        company: &str,
        issue: &str,
        key: &str,
    ) -> Result<Vec<DocumentRev>, String> {
        let i = self.issue_get(company, issue)?;
        self.read(|c| {
            let mut st = c.prepare(
                "SELECT r.rev, r.body, r.summary, r.author, r.run_id, r.created_ms FROM document_revs r
                 JOIN documents d ON d.id = r.doc_id WHERE d.issue_id = ?1 AND d.key = ?2 ORDER BY r.rev DESC",
            )?;
            let rows = st.query_map(params![i.id, key], |r| {
                Ok(DocumentRev {
                    rev: r.get(0)?,
                    body: r.get(1)?,
                    summary: r.get(2)?,
                    author: r.get(3)?,
                    run_id: r.get(4)?,
                    created_ms: r.get(5)?,
                })
            })?;
            rows.collect()
        })
    }

    pub fn relate(
        &self,
        company: &str,
        from: &str,
        to: &str,
        kind: &str,
        actor: &Actor,
    ) -> Result<(), String> {
        if !matches!(kind, "blocks" | "related" | "duplicate") {
            return Err(invalid("relation is blocks, related or duplicate"));
        }
        self.write(|tx, ch| {
            let a = get_issue(tx, company, from)?;
            let b = get_issue(tx, company, to)?;
            if a.id == b.id {
                return Err(invalid("an issue cannot relate to itself"));
            }
            if kind == "blocks" && blocks_cycle(tx, company, &a.id, &b.id)? {
                return Err(invalid(format!("{} already waits on {}", a.identifier, b.identifier)));
            }
            tx.execute(
                "INSERT OR IGNORE INTO relations (company_id, from_id, to_id, kind, created_ms) VALUES (?1,?2,?3,?4,?5)",
                params![company, a.id, b.id, kind, now_ms()],
            )
            .map_err(sql)?;
            audit::record(tx, company, actor, "issue.related", "issue", &a.id, Some(json!({"to": b.id, "kind": kind})))?;
            changed(ch, company, "issue", &a.id);
            changed(ch, company, "issue", &b.id);
            Ok(())
        })
    }

    pub fn unrelate(
        &self,
        company: &str,
        from: &str,
        to: &str,
        kind: &str,
        actor: &Actor,
    ) -> Result<(), String> {
        self.write(|tx, ch| {
            let a = get_issue(tx, company, from)?;
            let b = get_issue(tx, company, to)?;
            tx.execute(
                "DELETE FROM relations WHERE company_id = ?1 AND from_id = ?2 AND to_id = ?3 AND kind = ?4",
                params![company, a.id, b.id, kind],
            )
            .map_err(sql)?;
            if kind == "blocks" && b.status == "blocked" && open_blockers(tx, company, &b.id)?.is_empty() {
                wake_assignee(tx, &b, WakeSource::Dependency, "no longer blocked", actor)?;
            }
            audit::record(tx, company, actor, "issue.unrelated", "issue", &a.id, Some(json!({"to": b.id, "kind": kind})))?;
            changed(ch, company, "issue", &a.id);
            changed(ch, company, "issue", &b.id);
            Ok(())
        })
    }

    pub fn labels_list(&self, company: &str) -> Result<Vec<Label>, String> {
        self.read(|c| {
            let mut st = c.prepare(
                "SELECT id, name, color FROM labels WHERE company_id = ?1 ORDER BY name",
            )?;
            let rows = st.query_map(params![company], |r| {
                Ok(Label {
                    id: r.get(0)?,
                    name: r.get(1)?,
                    color: r.get(2)?,
                })
            })?;
            rows.collect()
        })
    }

    /// Replaces the labels of an issue; unknown names are created.
    pub fn issue_set_labels(
        &self,
        company: &str,
        issue: &str,
        names: &[String],
        actor: &Actor,
    ) -> Result<Vec<Label>, String> {
        self.write(|tx, ch| {
            let i = get_issue(tx, company, issue)?;
            set_labels_tx(tx, company, &i.id, names)?;
            audit::record(
                tx,
                company,
                actor,
                "issue.labels",
                "issue",
                &i.id,
                Some(json!({"labels": names})),
            )?;
            changed(ch, company, "issue", &i.id);
            Ok(())
        })?;
        Ok(self.issue_detail(company, issue)?.labels)
    }

    pub fn work_product_add(
        &self,
        company: &str,
        issue: &str,
        wp: NewWorkProduct,
        actor: &Actor,
    ) -> Result<WorkProduct, String> {
        if wp.kind.trim().is_empty() {
            return Err(invalid(
                "a work product needs a kind (file, url, pr, artifact, report…)",
            ));
        }
        self.write(|tx, ch| {
            let i = get_issue(tx, company, issue)?;
            let id = new_id();
            let now = now_ms();
            let review = if i.review_policy == "none" { "none" } else { "pending" };
            tx.execute(
                "INSERT INTO work_products (id, company_id, issue_id, kind, title, url, path, summary, run_id, review, created_ms)
                 VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)",
                params![id, company, i.id, wp.kind.trim(), wp.title, wp.url, wp.path, wp.summary, actor.run_id(), review, now],
            )
            .map_err(sql)?;
            audit::record(tx, company, actor, "work_product.added", "issue", &i.id, Some(json!({"kind": wp.kind, "title": wp.title})))?;
            changed(ch, company, "issue", &i.id);
            Ok(WorkProduct {
                id,
                issue_id: i.id,
                kind: wp.kind,
                title: wp.title,
                url: wp.url,
                path: wp.path,
                summary: wp.summary,
                run_id: actor.run_id().map(str::to_string),
                review: review.into(),
                created_ms: now,
            })
        })
    }

    pub fn work_product_review(
        &self,
        company: &str,
        id: &str,
        verdict: &str,
        actor: &Actor,
    ) -> Result<(), String> {
        if !matches!(verdict, "accepted" | "rejected" | "pending") {
            return Err(invalid("verdict is accepted, rejected or pending"));
        }
        self.write(|tx, ch| {
            let issue: String = tx
                .query_row(
                    "SELECT issue_id FROM work_products WHERE company_id = ?1 AND id = ?2",
                    params![company, id],
                    |r| r.get(0),
                )
                .optional()
                .map_err(sql)?
                .ok_or_else(|| not_found("work product", id))?;
            let i = get_issue(tx, company, &issue)?;
            if actor.staff_id().is_some() && actor.staff_id() == i.assignee_staff_id.as_deref() {
                return Err(forbidden("the author does not review its own work"));
            }
            tx.execute(
                "UPDATE work_products SET review = ?2 WHERE id = ?1",
                params![id, verdict],
            )
            .map_err(sql)?;
            audit::record(
                tx,
                company,
                actor,
                &format!("work_product.{verdict}"),
                "issue",
                &issue,
                Some(json!({"work_product": id})),
            )?;
            changed(ch, company, "issue", &issue);
            Ok(())
        })
    }

    /// Full-text search (FTS5 trigram, substring-capable like pg_trgm) with
    /// identifier matches first. Queries under 3 characters fall back to a
    /// prefix match on the identifier and title.
    pub fn issue_search(
        &self,
        company: &str,
        q: &str,
        limit: u32,
    ) -> Result<Vec<SearchHit>, String> {
        let q = q.trim();
        if q.is_empty() {
            return Ok(Vec::new());
        }
        let limit = limit.clamp(1, 200) as i64;
        self.read(|c| {
            let mut out: Vec<SearchHit> = Vec::new();
            let mut seen = HashSet::new();
            // 1. exact identifier.
            if let Some(i) = c
                .query_row(
                    &format!("SELECT {ISSUE_COLS} FROM issues WHERE company_id = ?1 AND identifier = ?2 COLLATE NOCASE"),
                    params![company, q],
                    issue_row,
                )
                .optional()?
            {
                seen.insert(i.id.clone());
                out.push(SearchHit { issue: i, score: 1000.0 });
            }
            if q.chars().count() < 3 {
                let like = format!("{}%", q.replace(['%', '_'], ""));
                let mut st = c.prepare(&format!(
                    "SELECT {ISSUE_COLS} FROM issues WHERE company_id = ?1 AND (title LIKE ?2 OR identifier LIKE ?2) LIMIT ?3"
                ))?;
                for i in st.query_map(params![company, like, limit], issue_row)? {
                    let i = i?;
                    if seen.insert(i.id.clone()) {
                        out.push(SearchHit { issue: i, score: 1.0 });
                    }
                }
                return Ok(out);
            }
            let phrase = format!("\"{}\"", q.replace('"', "\"\""));
            let mut st = c.prepare(&format!(
                "SELECT {cols}, bm25(issues_fts, 10.0, 5.0, 1.0) AS rank FROM issues_fts
                 JOIN issues i ON i.rowid = issues_fts.rowid
                 WHERE issues_fts MATCH ?2 AND i.company_id = ?1
                 ORDER BY (i.status IN ('done','cancelled')), rank LIMIT ?3",
                cols = ISSUE_COLS
                    .split(',')
                    .map(|c| format!("i.{}", c.trim()))
                    .collect::<Vec<_>>()
                    .join(", ")
            ))?;
            let rows = st.query_map(params![company, phrase, limit], |r| {
                let rank: f64 = r.get(32)?;
                Ok((issue_row(r)?, rank))
            })?;
            for row in rows {
                let (i, rank) = row?;
                if seen.insert(i.id.clone()) {
                    out.push(SearchHit { issue: i, score: -rank });
                }
            }
            Ok(out)
        })
    }

    /// What needs the owner: issues assigned to the owner, reviews only a
    /// human can close, blocked work, and unread agent comments on the
    /// owner's issues. Pending approvals come from [`gov`](super::gov).
    pub fn inbox(&self, company: &str) -> Result<Vec<InboxItem>, String> {
        let now = now_ms();
        self.read(|c| {
            let mut st = c.prepare(&format!(
                "SELECT {cols},
                   CASE
                     WHEN i.status = 'in_review' AND (i.review_policy = 'human_only' OR i.assignee_user = 1) THEN 'review'
                     WHEN i.status = 'blocked' THEN 'blocked'
                     WHEN i.assignee_user = 1 THEN 'assigned'
                     ELSE 'mentioned'
                   END AS why,
                   COALESCE(s.read_ms >= i.updated_ms, 0) AS read
                 FROM issues i LEFT JOIN inbox_state s ON s.issue_id = i.id
                 WHERE i.company_id = ?1
                   AND i.status NOT IN ('done','cancelled')
                   AND (s.archived_ms IS NULL OR s.archived_ms < i.updated_ms)
                   AND (s.snooze_until_ms IS NULL OR s.snooze_until_ms < ?2)
                   AND (i.assignee_user = 1
                        OR (i.status = 'in_review' AND i.review_policy = 'human_only')
                        OR i.status = 'blocked'
                        OR EXISTS (SELECT 1 FROM comments cm WHERE cm.issue_id = i.id
                                    AND cm.author_kind = 'agent' AND cm.body LIKE '%@owner%'))
                 ORDER BY read, CASE i.priority WHEN 'critical' THEN 0 WHEN 'high' THEN 1 WHEN 'medium' THEN 2 ELSE 3 END,
                          i.updated_ms DESC
                 LIMIT 500",
                cols = ISSUE_COLS
                    .split(',')
                    .map(|c| format!("i.{}", c.trim()))
                    .collect::<Vec<_>>()
                    .join(", ")
            ))?;
            let rows = st.query_map(params![company, now], |r| {
                Ok(InboxItem { issue: issue_row(r)?, why: r.get(32)?, read: r.get::<_, i64>(33)? != 0 })
            })?;
            rows.collect()
        })
    }

    pub fn inbox_mark(
        &self,
        company: &str,
        issue: &str,
        action: &str,
        until_ms: Option<i64>,
    ) -> Result<(), String> {
        let now = now_ms();
        self.write(|tx, ch| {
            let i = get_issue(tx, company, issue)?;
            let sql_text = match action {
                "read" => "INSERT INTO inbox_state (issue_id, read_ms) VALUES (?1, ?2) ON CONFLICT(issue_id) DO UPDATE SET read_ms = ?2",
                "unread" => "INSERT INTO inbox_state (issue_id, read_ms) VALUES (?1, NULL) ON CONFLICT(issue_id) DO UPDATE SET read_ms = NULL",
                "archive" => "INSERT INTO inbox_state (issue_id, archived_ms) VALUES (?1, ?2) ON CONFLICT(issue_id) DO UPDATE SET archived_ms = ?2",
                "snooze" => "INSERT INTO inbox_state (issue_id, snooze_until_ms) VALUES (?1, ?2) ON CONFLICT(issue_id) DO UPDATE SET snooze_until_ms = ?2",
                _ => return Err(invalid("action is read, unread, archive or snooze")),
            };
            let v = if action == "snooze" { until_ms.unwrap_or(now + 24 * 3600 * 1000) } else { now };
            tx.execute(sql_text, params![i.id, v]).map_err(sql)?;
            changed(ch, company, "inbox", &i.id);
            Ok(())
        })
    }

    /// Tree control: cancel, hold (block) or release a whole subtree in one
    /// transaction. Returns how many issues changed. Queued wakeups of the
    /// subtree are cancelled so nothing starts after the hold.
    pub fn issue_tree(
        &self,
        company: &str,
        root: &str,
        action: &str,
        actor: &Actor,
    ) -> Result<u64, String> {
        if !matches!(action, "cancel" | "hold" | "release") {
            return Err(invalid("tree action is cancel, hold or release"));
        }
        self.write(|tx, ch| {
            let r = get_issue(tx, company, root)?;
            let mut st = tx
                .prepare(
                    "WITH RECURSIVE t(id, depth) AS (
                       SELECT ?2, 0 UNION
                       SELECT i.id, t.depth + 1 FROM issues i JOIN t ON i.parent_id = t.id
                       WHERE i.company_id = ?1 AND t.depth < 64)
                     SELECT id FROM t",
                )
                .map_err(sql)?;
            let ids: Vec<String> = st
                .query_map(params![company, r.id], |row| row.get(0))
                .map_err(sql)?
                .collect::<Result<_, _>>()
                .map_err(sql)?;
            drop(st);
            let now = now_ms();
            let json_ids = serde_json::to_string(&ids).unwrap_or_default();
            let n = match action {
                "cancel" => tx.execute(
                    "UPDATE issues SET status = 'cancelled', cancelled_ms = ?3, lease_run_id = NULL, lease_until_ms = NULL,
                       seq = seq + 1, updated_ms = ?3
                     WHERE company_id = ?1 AND id IN (SELECT value FROM json_each(?2)) AND status NOT IN ('done','cancelled')",
                    params![company, json_ids, now],
                ),
                "hold" => tx.execute(
                    "UPDATE issues SET status = 'blocked', blocked_reason = 'hold:' || ?4, lease_run_id = NULL,
                       lease_until_ms = NULL, seq = seq + 1, updated_ms = ?3
                     WHERE company_id = ?1 AND id IN (SELECT value FROM json_each(?2)) AND status NOT IN ('done','cancelled','blocked')",
                    params![company, json_ids, now, r.id],
                ),
                _ => tx.execute(
                    "UPDATE issues SET status = 'todo', blocked_reason = NULL, seq = seq + 1, updated_ms = ?3
                     WHERE company_id = ?1 AND id IN (SELECT value FROM json_each(?2)) AND blocked_reason = 'hold:' || ?4",
                    params![company, json_ids, now, r.id],
                ),
            }
            .map_err(sql)?;
            if action != "release" {
                tx.execute(
                    "UPDATE wakeups SET state = 'cancelled', skip_reason = 'tree_' || ?3, updated_ms = ?4
                     WHERE company_id = ?1 AND state = 'queued' AND issue_id IN (SELECT value FROM json_each(?2))",
                    params![company, json_ids, action, now],
                )
                .map_err(sql)?;
            } else {
                // Wake the assignees of what was released.
                let mut st = tx
                    .prepare(&format!(
                        "SELECT {ISSUE_COLS} FROM issues WHERE company_id = ?1 AND id IN (SELECT value FROM json_each(?2)) AND status = 'todo'"
                    ))
                    .map_err(sql)?;
                let back = st
                    .query_map(params![company, json_ids], issue_row)
                    .map_err(sql)?
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(sql)?;
                drop(st);
                for i in back {
                    wake_assignee(tx, &i, WakeSource::Dependency, "the hold was released", actor)?;
                }
            }
            audit::record(tx, company, actor, &format!("issue.tree_{action}"), "issue", &r.id, Some(json!({"count": n})))?;
            for id in &ids {
                changed(ch, company, "issue", id);
            }
            Ok(n as u64)
        })
    }

    pub fn issue_delete(&self, company: &str, id: &str, actor: &Actor) -> Result<(), String> {
        if !actor.is_user() {
            return Err(forbidden(
                "agents cancel issues; only the owner deletes them",
            ));
        }
        self.write(|tx, ch| {
            let i = get_issue(tx, company, id)?;
            tx.execute("UPDATE issues SET parent_id = ?3 WHERE company_id = ?1 AND parent_id = ?2", params![company, i.id, i.parent_id]).map_err(sql)?;
            tx.execute("DELETE FROM issue_labels WHERE issue_id = ?1", params![i.id]).map_err(sql)?;
            tx.execute("DELETE FROM inbox_state WHERE issue_id = ?1", params![i.id]).map_err(sql)?;
            tx.execute("UPDATE wakeups SET state = 'cancelled', skip_reason = 'issue_deleted' WHERE issue_id = ?1 AND state = 'queued'", params![i.id]).map_err(sql)?;
            tx.execute("DELETE FROM issues WHERE id = ?1", params![i.id]).map_err(sql)?;
            audit::record(tx, company, actor, "issue.deleted", "issue", &i.id, Some(json!({"identifier": i.identifier, "title": i.title})))?;
            changed(ch, company, "issue", &i.id);
            Ok(())
        })
    }
}

fn doc_row(r: &Row) -> rusqlite::Result<Document> {
    Ok(Document {
        id: r.get(0)?,
        issue_id: r.get(1)?,
        key: r.get(2)?,
        rev: r.get(3)?,
        body: r.get(4)?,
        locked: r.get::<_, i64>(5)? != 0,
        updated_ms: r.get(6)?,
    })
}

fn wp_row(r: &Row) -> rusqlite::Result<WorkProduct> {
    Ok(WorkProduct {
        id: r.get(0)?,
        issue_id: r.get(1)?,
        kind: r.get(2)?,
        title: r.get(3)?,
        url: r.get(4)?,
        path: r.get(5)?,
        summary: r.get(6)?,
        run_id: r.get(7)?,
        review: r.get(8)?,
        created_ms: r.get(9)?,
    })
}

fn set_labels_tx(
    tx: &Transaction,
    company: &str,
    issue: &str,
    names: &[String],
) -> Result<(), String> {
    tx.execute(
        "DELETE FROM issue_labels WHERE issue_id = ?1",
        params![issue],
    )
    .map_err(sql)?;
    for n in names {
        let n = n.trim();
        if n.is_empty() {
            continue;
        }
        let lid: String = match tx
            .query_row(
                "SELECT id FROM labels WHERE company_id = ?1 AND name = ?2 COLLATE NOCASE",
                params![company, n],
                |r| r.get(0),
            )
            .optional()
            .map_err(sql)?
        {
            Some(id) => id,
            None => {
                let id = new_id();
                let palette = [
                    "#e5484d", "#f76b15", "#ffc53d", "#30a46c", "#12a594", "#0090ff", "#8e4ec6",
                    "#d6409f",
                ];
                let color = palette[n.bytes().map(|b| b as usize).sum::<usize>() % palette.len()];
                tx.execute(
                    "INSERT INTO labels (id, company_id, name, color) VALUES (?1,?2,?3,?4)",
                    params![id, company, n, color],
                )
                .map_err(sql)?;
                id
            }
        };
        tx.execute(
            "INSERT OR IGNORE INTO issue_labels (issue_id, label_id) VALUES (?1, ?2)",
            params![issue, lid],
        )
        .map_err(sql)?;
    }
    Ok(())
}

pub(crate) fn issue_create_tx(
    tx: &Transaction,
    ch: &mut Vec<Change>,
    company: &str,
    new: NewIssue,
    actor: &Actor,
) -> Result<Issue, String> {
    let title = new.title.trim();
    if title.is_empty() {
        return Err(invalid("an issue needs a title"));
    }
    let priority = new.priority.clone().unwrap_or_else(|| "medium".into());
    if !priority_ok(&priority) {
        return Err(invalid("priority is critical, high, medium or low"));
    }
    let c = get_company(tx, company)?;
    if c.status == "archived" {
        return Err(conflict("the company is archived"));
    }
    let mut depth = 0;
    let mut goal = new.goal_id.clone();
    let mut project = new.project_id.clone();
    let parent = match &new.parent_id {
        Some(p) => {
            let pi = get_issue(tx, company, p)?;
            depth = pi.request_depth;
            goal = goal.or(pi.goal_id.clone());
            project = project.or(pi.project_id.clone());
            Some(pi.id)
        }
        None => None,
    };
    if actor.staff_id().is_some() {
        depth += 1;
        if depth > MAX_REQUEST_DEPTH {
            return Err(forbidden(format!(
                "delegation is {depth} levels deep; the limit is {MAX_REQUEST_DEPTH} (ask the owner)"
            )));
        }
    }
    let mut assignee = new.assignee_staff_id.clone();
    if let Some(s) = &assignee {
        let st = get_staff(tx, company, s)?;
        if matches!(st.status.as_str(), "terminated" | "pending_approval") {
            return Err(invalid(format!(
                "{} cannot take work ({})",
                st.name, st.status
            )));
        }
    }
    if new.assignee_user {
        assignee = None;
    }
    let mut status = new.status.clone().unwrap_or_else(|| "todo".into());
    if !matches!(status.as_str(), "backlog" | "todo" | "blocked") {
        return Err(invalid("a new issue starts in backlog, todo or blocked"));
    }
    if !new.blocked_by.is_empty() && status == "todo" {
        status = "blocked".into();
    }
    let work_mode = new.work_mode.clone().unwrap_or_else(|| "standard".into());
    let review = new.review_policy.clone().unwrap_or_else(|| "none".into());
    let number: i64 = tx
        .query_row(
            "UPDATE companies SET issue_counter = issue_counter + 1 WHERE id = ?1 RETURNING issue_counter",
            params![company],
            |r| r.get(0),
        )
        .map_err(sql)?;
    let identifier = format!("{}-{number}", c.issue_prefix);
    let id = new_id();
    let now = now_ms();
    tx.execute(
        "INSERT INTO issues (id, company_id, number, identifier, project_id, goal_id, parent_id, title, body,
           status, priority, work_mode, review_policy, reviewer_staff_id, assignee_staff_id, assignee_user,
           created_by_staff_id, created_by_user, billing_code, request_depth, board_rank, origin_kind,
           origin_id, created_ms, updated_ms)
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19,?20,?21,?22,?23,?24,?24)",
        params![
            id, company, number, identifier, project, goal, parent, title, new.body, status, priority,
            work_mode, review, new.reviewer_staff_id, assignee, new.assignee_user as i64,
            actor.staff_id(), actor.is_user() as i64, new.billing_code, depth, number as f64,
            new.origin_kind.clone().unwrap_or_else(|| if actor.is_user() { "manual".into() } else { "agent".into() }),
            new.origin_id, now
        ],
    )
    .map_err(|e| match e {
        rusqlite::Error::SqliteFailure(f, _) if f.code == rusqlite::ErrorCode::ConstraintViolation => {
            invalid("a linked project, goal, parent or agent is not in this company, or a value is unknown")
        }
        e => sql(e),
    })?;
    for b in &new.blocked_by {
        let bi = get_issue(tx, company, b)?;
        tx.execute(
            "INSERT OR IGNORE INTO relations (company_id, from_id, to_id, kind, created_ms) VALUES (?1,?2,?3,'blocks',?4)",
            params![company, bi.id, id, now],
        )
        .map_err(sql)?;
    }
    if status == "blocked"
        && open_blockers(tx, company, &id)?.is_empty()
        && !new.blocked_by.is_empty()
    {
        tx.execute(
            "UPDATE issues SET status = 'todo' WHERE id = ?1",
            params![id],
        )
        .map_err(sql)?;
    }
    if !new.labels.is_empty() {
        set_labels_tx(tx, company, &id, &new.labels)?;
    }
    let issue = get_issue(tx, company, &id)?;
    if issue.status == "todo" {
        wake_assignee(
            tx,
            &issue,
            WakeSource::Assignment,
            "new issue for you",
            actor,
        )?;
    }
    audit::record(
        tx,
        company,
        actor,
        "issue.created",
        "issue",
        &id,
        Some(
            json!({"identifier": identifier, "title": title, "assignee": issue.assignee_staff_id}),
        ),
    )?;
    changed(ch, company, "issue", &id);
    Ok(issue)
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn transition_tx(
    tx: &Transaction,
    ch: &mut Vec<Change>,
    company: &str,
    id: &str,
    to: &str,
    note: Option<&str>,
    expected_seq: Option<i64>,
    actor: &Actor,
) -> Result<Issue, String> {
    let mut i = get_issue(tx, company, id)?;
    let from = i.status.clone();
    if let Some(seq) = expected_seq {
        if seq != i.seq {
            return Err(conflict(format!(
                "{} changed (seq {} ≠ {seq})",
                i.identifier, i.seq
            )));
        }
    }
    if from == to {
        return Ok(i);
    }
    if !can_go(&from, to) {
        return Err(invalid(format!(
            "{} cannot go from {from} to {to}",
            i.identifier
        )));
    }
    if matches!(from.as_str(), "done" | "cancelled") && !actor.is_user() {
        return Err(forbidden("only the owner reopens finished work"));
    }
    if to == "in_progress" {
        if i.assignee_staff_id.is_none() && !i.assignee_user {
            if let Some(me) = actor.staff_id() {
                i.assignee_staff_id = Some(me.to_string());
            } else {
                return Err(invalid("in progress needs an assignee"));
            }
        }
        let open = open_blockers(tx, company, &i.id)?;
        if !open.is_empty() {
            return Err(conflict(format!("still blocked by {}", open.join(", "))));
        }
    }
    if to == "blocked"
        && note.map(str::trim).unwrap_or("").is_empty()
        && open_blockers(tx, company, &i.id)?.is_empty()
    {
        return Err(invalid(
            "say what it is waiting for (a note or a blocker issue)",
        ));
    }
    // Review gate: with a policy, an agent sends work to review instead of
    // closing it, and the verdict obeys the policy under this row lock.
    let mut target = to.to_string();
    if to == "done" && i.review_policy != "none" && from != "in_review" && !actor.is_user() {
        target = "in_review".into();
    }
    if from == "in_review" && matches!(target.as_str(), "done") {
        let allowed = match (i.review_policy.as_str(), actor) {
            ("none", _) => true,
            (_, Actor::User { .. }) => true,
            (_, Actor::System { .. }) => false,
            ("human_only", _) => false,
            ("not_creator", Actor::Agent { staff_id, .. }) => {
                Some(staff_id.as_str()) != i.created_by_staff_id.as_deref()
                    && Some(staff_id.as_str()) != i.assignee_staff_id.as_deref()
            }
            (_, Actor::Agent { staff_id, .. }) => {
                Some(staff_id.as_str()) != i.assignee_staff_id.as_deref()
            }
        };
        if !allowed {
            return Err(forbidden(format!(
                "the {} review policy does not let you close this",
                i.review_policy
            )));
        }
    }
    let now = now_ms();
    stamp(&mut i, &target, now);
    let release = matches!(
        target.as_str(),
        "done" | "cancelled" | "in_review" | "blocked" | "todo" | "backlog"
    );
    let blocked_reason = if target == "blocked" {
        note.map(str::to_string)
    } else {
        None
    };
    tx.execute(
        "UPDATE issues SET status = ?2, assignee_staff_id = ?3, started_ms = ?4, completed_ms = ?5, cancelled_ms = ?6,
           blocked_reason = ?7,
           lease_run_id = CASE WHEN ?8 THEN NULL ELSE lease_run_id END,
           lease_until_ms = CASE WHEN ?8 THEN NULL ELSE lease_until_ms END,
           seq = seq + 1, updated_ms = ?9
         WHERE id = ?1",
        params![i.id, target, i.assignee_staff_id, i.started_ms, i.completed_ms, i.cancelled_ms, blocked_reason, release, now],
    )
    .map_err(sql)?;
    let mut line = format!("{from} → {target}");
    if target != to {
        line.push_str(" (review required)");
    }
    if let Some(n) = note.filter(|n| !n.trim().is_empty()) {
        line.push_str(": ");
        line.push_str(n.trim());
    }
    system_comment(tx, company, &i.id, &line)?;
    match target.as_str() {
        "done" => wake_waiters(tx, company, &i.id, actor)?,
        "in_review" => {
            if let Some(r) = &i.reviewer_staff_id {
                enqueue_wake(
                    tx,
                    company,
                    r,
                    Some(&i.id),
                    WakeSource::Review,
                    "review requested",
                    json!({}),
                    priority_rank(&i.priority),
                )?;
            } else if let Some(s) = &i.assignee_staff_id {
                // Default reviewer: the assignee's manager.
                let mgr: Option<String> = tx
                    .query_row(
                        "SELECT reports_to FROM staff WHERE id = ?1",
                        params![s],
                        |r| r.get(0),
                    )
                    .optional()
                    .map_err(sql)?
                    .flatten();
                if let (Some(m), false) = (mgr, i.review_policy == "human_only") {
                    enqueue_wake(
                        tx,
                        company,
                        &m,
                        Some(&i.id),
                        WakeSource::Review,
                        "review your report's work",
                        json!({}),
                        priority_rank(&i.priority),
                    )?;
                }
            }
        }
        "todo" if from == "in_review" => {
            wake_assignee(
                tx,
                &get_issue(tx, company, &i.id)?,
                WakeSource::Review,
                "changes requested",
                actor,
            )?;
        }
        _ => {}
    }
    audit::record(
        tx,
        company,
        actor,
        "issue.status",
        "issue",
        &i.id,
        Some(json!({"from": from, "to": target, "note": note})),
    )?;
    changed(ch, company, "issue", &i.id);
    get_issue(tx, company, &i.id)
}

pub(crate) fn checkout_tx(
    tx: &Transaction,
    ch: &mut Vec<Change>,
    company: &str,
    id: &str,
    staff: &str,
    run_id: &str,
    lease_ms: Option<i64>,
) -> Result<Issue, String> {
    let i = get_issue(tx, company, id)?;
    let s = get_staff(tx, company, staff)?;
    if matches!(
        s.status.as_str(),
        "terminated" | "pending_approval" | "paused"
    ) {
        return Err(forbidden(format!("{} is {}", s.name, s.status)));
    }
    if let Some(other) = i.assignee_staff_id.as_deref() {
        if other != staff {
            return Err(conflict(format!(
                "{} is assigned to someone else",
                i.identifier
            )));
        }
    }
    let open = open_blockers(tx, company, &i.id)?;
    if !open.is_empty() {
        return Err(conflict(format!("still blocked by {}", open.join(", "))));
    }
    let now = now_ms();
    let until = now + lease_ms.unwrap_or(LEASE_MS);
    let got = tx
        .query_row(
            &format!(
                "UPDATE issues SET status = CASE WHEN status IN ('todo','backlog','blocked') THEN 'in_progress' ELSE status END,
                   assignee_staff_id = ?3, assignee_user = 0, lease_run_id = ?4, lease_until_ms = ?5,
                   started_ms = COALESCE(started_ms, ?6), blocked_reason = NULL, seq = seq + 1, updated_ms = ?6
                 WHERE company_id = ?1 AND id = ?2
                   AND status IN ('todo','backlog','blocked','in_progress','in_review')
                   AND (lease_run_id IS NULL OR lease_until_ms < ?6 OR lease_run_id = ?4)
                 RETURNING {ISSUE_COLS}"
            ),
            params![company, i.id, staff, run_id, until, now],
            issue_row,
        )
        .optional()
        .map_err(sql)?;
    match got {
        Some(issue) => {
            changed(ch, company, "issue", &issue.id);
            Ok(issue)
        }
        None => Err(conflict(format!(
            "{} is checked out by another run",
            i.identifier
        ))),
    }
}

pub(crate) fn comment_tx(
    tx: &Transaction,
    ch: &mut Vec<Change>,
    company: &str,
    issue: &str,
    body: &str,
    client_req: Option<&str>,
    actor: &Actor,
) -> Result<Comment, String> {
    let body = body.trim();
    if body.is_empty() {
        return Err(invalid("an empty comment"));
    }
    let i = get_issue(tx, company, issue)?;
    if let Some(req) = client_req {
        if let Some(c) = tx
            .query_row(
                "SELECT id, issue_id, author_kind, author_staff_id, run_id, body, created_ms FROM comments WHERE issue_id = ?1 AND client_req = ?2",
                params![i.id, req],
                |r| {
                    Ok(Comment {
                        id: r.get(0)?,
                        issue_id: r.get(1)?,
                        author_kind: r.get(2)?,
                        author_staff_id: r.get(3)?,
                        run_id: r.get(4)?,
                        body: r.get(5)?,
                        created_ms: r.get(6)?,
                    })
                },
            )
            .optional()
            .map_err(sql)?
        {
            return Ok(c);
        }
    }
    let id = new_id();
    let now = now_ms();
    tx.execute(
        "INSERT INTO comments (id, company_id, issue_id, author_kind, author_staff_id, run_id, body, client_req, created_ms)
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)",
        params![id, company, i.id, actor.kind(), actor.staff_id(), actor.run_id(), body, client_req, now],
    )
    .map_err(sql)?;
    tx.execute(
        "UPDATE issues SET updated_ms = ?2 WHERE id = ?1",
        params![i.id, now],
    )
    .map_err(sql)?;
    let mut woken: HashSet<String> = HashSet::new();
    for s in mentions(tx, company, body)? {
        if actor.staff_id() != Some(s.id.as_str()) && woken.insert(s.id.clone()) {
            enqueue_wake(
                tx,
                company,
                &s.id,
                Some(&i.id),
                WakeSource::Mention,
                "you were mentioned",
                json!({"comment_id": id}),
                priority_rank(&i.priority),
            )?;
        }
    }
    // The owner talking on an agent's issue wakes that agent.
    if actor.is_user() {
        if let Some(a) = &i.assignee_staff_id {
            if woken.insert(a.clone()) && !matches!(i.status.as_str(), "done" | "cancelled") {
                enqueue_wake(
                    tx,
                    company,
                    a,
                    Some(&i.id),
                    WakeSource::Comment,
                    "the owner commented",
                    json!({"comment_id": id}),
                    priority_rank(&i.priority),
                )?;
            }
        }
    }
    audit::record(
        tx,
        company,
        actor,
        "comment.added",
        "issue",
        &i.id,
        Some(json!({"comment_id": id})),
    )?;
    changed(ch, company, "issue", &i.id);
    Ok(Comment {
        id,
        issue_id: i.id,
        author_kind: actor.kind().into(),
        author_staff_id: actor.staff_id().map(str::to_string),
        run_id: actor.run_id().map(str::to_string),
        body: body.into(),
        created_ms: now,
    })
}

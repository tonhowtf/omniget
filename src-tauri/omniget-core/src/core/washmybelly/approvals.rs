//! Approval requests from external MCP clients. A client never approves
//! anything: `wmb_execute` from outside creates a request, the OmniGet window
//! shows it, and the person allows or denies. The client polls
//! `wmb_plan_show` (or `wmb_receipts` for a restore) until it sees the result.

use rusqlite::{params, OptionalExtension};
use serde::Serialize;
use serde_json::{json, Value};

use super::plan::{self, PlanStatus};
use super::store::{now, Ctx};

/// A request waits this long for the person before it expires.
pub const APPROVAL_TTL_SECS: i64 = 600;

#[derive(Debug, Clone, Serialize)]
pub struct Approval {
    pub id: String,
    /// `execute:<plan_id>` or `restore:<receipt_id>`.
    pub target: String,
    pub client: String,
    pub created: i64,
    pub status: String,
    pub receipt_id: Option<String>,
}

fn row(r: &rusqlite::Row<'_>) -> rusqlite::Result<Approval> {
    Ok(Approval {
        id: r.get(0)?,
        target: r.get(1)?,
        client: r.get(2)?,
        created: r.get(3)?,
        status: r.get(4)?,
        receipt_id: r.get(5)?,
    })
}

fn get(ctx: &Ctx, id: &str) -> Result<Approval, String> {
    ctx.open()?
        .query_row(
            "SELECT id, plan_id, client, created, status, receipt_id FROM approvals WHERE id=?1",
            params![id],
            row,
        )
        .optional()
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("APPROVAL_NOT_FOUND: {id}"))
}

/// Ask the person to execute `plan_id` for `client`. A second ask for the
/// same plan returns the pending request instead of stacking prompts.
pub fn request_execute(ctx: &Ctx, plan_id: &str, client: &str) -> Result<Approval, String> {
    let plan = plan::load(ctx, plan_id)?;
    match plan.status {
        PlanStatus::Ready | PlanStatus::PendingApproval => {}
        PlanStatus::Expired => return Err("PLAN_EXPIRED: scan again".into()),
        other => return Err(format!("PLAN_NOT_READY: {other:?}")),
    }
    request(ctx, &format!("execute:{plan_id}"), client).inspect(|_| {
        let _ = plan::set_status(ctx, plan_id, PlanStatus::PendingApproval);
    })
}

/// Ask the person to undo `receipt_id` for `client`.
pub fn request_restore(ctx: &Ctx, receipt_id: &str, client: &str) -> Result<Approval, String> {
    super::receipt::load(ctx, receipt_id)?;
    request(ctx, &format!("restore:{receipt_id}"), client)
}

fn request(ctx: &Ctx, target: &str, client: &str) -> Result<Approval, String> {
    let db = ctx.open()?;
    let pending: Option<String> = db
        .query_row(
            "SELECT id FROM approvals WHERE plan_id=?1 AND status='pending' AND created > ?2",
            params![target, now() - APPROVAL_TTL_SECS],
            |r| r.get(0),
        )
        .optional()
        .map_err(|e| e.to_string())?;
    if let Some(id) = pending {
        return get(ctx, &id);
    }
    let id = format!("appr_{}", uuid::Uuid::new_v4().simple());
    db.execute(
        "INSERT INTO approvals(id, plan_id, client, created, status) VALUES(?1, ?2, ?3, ?4, 'pending')",
        params![id, target, client, now()],
    )
    .map_err(|e| e.to_string())?;
    get(ctx, &id)
}

/// Pending requests, newest first, each with what the person needs to judge
/// it (the plan preview or the receipt summary).
pub fn pending(ctx: &Ctx) -> Result<Vec<Value>, String> {
    let db = ctx.open()?;
    db.execute(
        "UPDATE approvals SET status='expired' WHERE status='pending' AND created <= ?1",
        params![now() - APPROVAL_TTL_SECS],
    )
    .map_err(|e| e.to_string())?;
    let mut st = db
        .prepare("SELECT id, plan_id, client, created, status, receipt_id FROM approvals WHERE status='pending' ORDER BY created DESC")
        .map_err(|e| e.to_string())?;
    let list: Vec<Approval> = st
        .query_map([], row)
        .map_err(|e| e.to_string())?
        .flatten()
        .collect();
    Ok(list
        .into_iter()
        .map(|a| {
            let detail = match a.target.split_once(':') {
                Some(("execute", pid)) => plan::load(ctx, pid).map(|p| p.preview()).unwrap_or(Value::Null),
                Some(("restore", rid)) => super::receipt::load(ctx, rid).map(|r| r.summary()).unwrap_or(Value::Null),
                _ => Value::Null,
            };
            json!({ "approval": a, "detail": detail })
        })
        .collect())
}

/// The person's answer. Allow runs the plan (or the restore) right here and
/// records the receipt; deny marks the plan denied.
pub fn answer(ctx: &Ctx, id: &str, allow: bool) -> Result<Value, String> {
    let a = get(ctx, id)?;
    if a.status != "pending" {
        return Err(format!("APPROVAL_{}", a.status.to_uppercase()));
    }
    if now() - a.created > APPROVAL_TTL_SECS {
        set(ctx, id, "expired", None)?;
        return Err("APPROVAL_EXPIRED".into());
    }
    let (kind, target) = a.target.split_once(':').ok_or("bad approval target")?;
    if !allow {
        set(ctx, id, "denied", None)?;
        if kind == "execute" {
            plan::set_status(ctx, target, PlanStatus::Denied)?;
        }
        return Ok(json!({ "approval_id": id, "status": "denied" }));
    }
    set(ctx, id, "approved", None)?;
    match kind {
        "execute" => {
            let r = plan::execute(ctx, target)?;
            set(ctx, id, "executed", Some(&r.id))?;
            Ok(json!({ "approval_id": id, "status": "executed", "receipt": r.summary() }))
        }
        "restore" => {
            let items = super::receipt::restore(ctx, target)?;
            set(ctx, id, "executed", Some(target))?;
            Ok(json!({ "approval_id": id, "status": "executed", "receipt_id": target, "items": items }))
        }
        _ => Err("bad approval target".into()),
    }
}

fn set(ctx: &Ctx, id: &str, status: &str, receipt: Option<&str>) -> Result<(), String> {
    ctx.open()?
        .execute(
            "UPDATE approvals SET status=?2, receipt_id=COALESCE(?3, receipt_id) WHERE id=?1",
            params![id, status, receipt],
        )
        .map_err(|e| e.to_string())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::washmybelly::fsops;
    use crate::core::washmybelly::plan::{Op, PlanItem, Source};
    use crate::core::washmybelly::protect::Guard;
    use crate::core::washmybelly::{Destination, Risk};

    fn one_file_plan(ctx: &Ctx, dir: &std::path::Path) -> (String, std::path::PathBuf) {
        let root = dir.join("junk");
        std::fs::create_dir_all(&root).unwrap();
        let f = root.join("x.tmp");
        std::fs::write(&f, "x").unwrap();
        let item = PlanItem {
            id: "a:1".into(),
            label: "x".into(),
            detail: None,
            group: String::new(),
            bytes: 1,
            risk: Risk::Reversible,
            admin: false,
            op: Op::RemovePath {
                path: f.clone(),
                destination: Destination::Quarantine,
                fp: fsops::fingerprint(&f).unwrap(),
                root: Some(root),
                guard: Guard::Rule,
                contents_only: false,
            },
        };
        (plan::offer(ctx, "cleanup", vec![item], Source::External).unwrap().id, f)
    }

    #[test]
    fn nothing_runs_until_the_person_allows() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = Ctx::at(&dir.path().join("state"));
        let (plan_id, f) = one_file_plan(&ctx, dir.path());
        let a = request_execute(&ctx, &plan_id, "Codex").unwrap();
        assert!(f.exists());
        assert_eq!(plan::load(&ctx, &plan_id).unwrap().status, PlanStatus::PendingApproval);
        // A second ask does not stack a second prompt.
        assert_eq!(request_execute(&ctx, &plan_id, "Codex").unwrap().id, a.id);
        assert_eq!(pending(&ctx).unwrap().len(), 1);
        let out = answer(&ctx, &a.id, true).unwrap();
        assert_eq!(out["status"], "executed");
        assert!(!f.exists());
        assert_eq!(plan::load(&ctx, &plan_id).unwrap().status, PlanStatus::Executed);
        assert!(answer(&ctx, &a.id, true).is_err(), "an answer counts once");
    }

    #[test]
    fn a_denied_plan_never_runs() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = Ctx::at(&dir.path().join("state"));
        let (plan_id, f) = one_file_plan(&ctx, dir.path());
        let a = request_execute(&ctx, &plan_id, "Codex").unwrap();
        answer(&ctx, &a.id, false).unwrap();
        assert!(f.exists());
        assert!(plan::execute(&ctx, &plan_id).is_err());
    }
}

use rusqlite::{params, OptionalExtension};
use serde::Serialize;
use serde_json::{json, Value};

use super::plan::{self, PlanStatus};
use super::store::{now, Ctx};

pub const APPROVAL_TTL_SECS: i64 = 600;

#[derive(Debug, Clone, Serialize)]
pub struct Approval {
    pub id: String,
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
                Some(("execute", pid)) => plan::load(ctx, pid)
                    .map(|p| p.preview())
                    .unwrap_or(Value::Null),
                Some(("restore", rid)) => super::receipt::load(ctx, rid)
                    .map(|r| r.summary())
                    .unwrap_or(Value::Null),
                _ => Value::Null,
            };
            json!({ "approval": a, "detail": detail })
        })
        .collect())
}

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
            Ok(
                json!({ "approval_id": id, "status": "executed", "receipt_id": target, "items": items }),
            )
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

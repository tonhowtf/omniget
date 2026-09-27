//! Receipts: what a plan actually did, and how to undo it.

use std::path::PathBuf;

use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::plan::Plan;
use super::store::{now, Ctx};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ItemStatus {
    Done,
    Skipped,
    Failed,
}

/// How to bring one item back.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Undo {
    Quarantine {
        from: PathBuf,
        to: PathBuf,
    },
    Trash {
        original: PathBuf,
    },
    TrashMany {
        originals: Vec<PathBuf>,
    },
    /// The module restores its snapshot (registry value, service state,
    /// startup item, privacy setting...). `args` usually holds a snapshot id.
    Module {
        module: String,
        action: String,
        args: Value,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ItemResult {
    pub item_id: String,
    pub label: String,
    pub status: ItemStatus,
    pub detail: String,
    pub bytes: u64,
    #[serde(default)]
    pub undo: Option<Undo>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Receipt {
    pub id: String,
    pub plan_id: String,
    pub kind: String,
    pub created: i64,
    pub items: Vec<ItemResult>,
    pub bytes_freed: u64,
    pub restorable: bool,
    #[serde(default)]
    pub restored_at: Option<i64>,
}

impl Receipt {
    pub fn new(id: String, plan: &Plan, items: Vec<ItemResult>) -> Receipt {
        Receipt {
            id,
            plan_id: plan.id.clone(),
            kind: plan.kind.clone(),
            created: now(),
            bytes_freed: items
                .iter()
                .filter(|i| i.status == ItemStatus::Done)
                .map(|i| i.bytes)
                .sum(),
            restorable: items.iter().any(|i| i.undo.is_some()),
            items,
            restored_at: None,
        }
    }

    pub fn summary(&self) -> Value {
        let count = |s: ItemStatus| self.items.iter().filter(|i| i.status == s).count();
        json!({
            "receipt_id": self.id,
            "plan_id": self.plan_id,
            "kind": self.kind,
            "created": self.created,
            "bytes_freed": self.bytes_freed,
            "done": count(ItemStatus::Done),
            "skipped": count(ItemStatus::Skipped),
            "failed": count(ItemStatus::Failed),
            "restorable": self.restorable && self.restored_at.is_none(),
            "restored_at": self.restored_at,
            "items": self.items.iter().take(300).map(|i| json!({
                "item_id": i.item_id, "label": i.label, "status": i.status,
                "detail": i.detail, "bytes": i.bytes, "undo": i.undo.as_ref().map(undo_kind),
            })).collect::<Vec<_>>(),
        })
    }
}

fn undo_kind(u: &Undo) -> &'static str {
    match u {
        Undo::Quarantine { .. } => "quarantine",
        Undo::Trash { .. } | Undo::TrashMany { .. } => "trash",
        Undo::Module { .. } => "snapshot",
    }
}

pub fn save(ctx: &Ctx, r: &Receipt) -> Result<(), String> {
    let body = serde_json::to_string(r).map_err(|e| e.to_string())?;
    ctx.open()?
        .execute(
            "INSERT INTO receipts(id, plan_id, created, body) VALUES(?1, ?2, ?3, ?4)
             ON CONFLICT(id) DO UPDATE SET body=excluded.body",
            params![r.id, r.plan_id, r.created, body],
        )
        .map_err(|e| e.to_string())?;
    Ok(())
}

pub fn load(ctx: &Ctx, id: &str) -> Result<Receipt, String> {
    let body: Option<String> = ctx
        .open()?
        .query_row("SELECT body FROM receipts WHERE id=?1", params![id], |r| {
            r.get(0)
        })
        .optional()
        .map_err(|e| e.to_string())?;
    serde_json::from_str(&body.ok_or_else(|| format!("RECEIPT_NOT_FOUND: {id}"))?)
        .map_err(|e| e.to_string())
}

pub fn list(ctx: &Ctx, limit: usize) -> Result<Vec<Receipt>, String> {
    let db = ctx.open()?;
    let mut st = db
        .prepare("SELECT body FROM receipts ORDER BY created DESC LIMIT ?1")
        .map_err(|e| e.to_string())?;
    let rows = st
        .query_map(params![limit as i64], |r| r.get::<_, String>(0))
        .map_err(|e| e.to_string())?;
    Ok(rows
        .flatten()
        .filter_map(|b| serde_json::from_str(&b).ok())
        .collect())
}

/// Undo what a receipt did, item by item. Returns one line per item.
pub fn restore(ctx: &Ctx, id: &str) -> Result<Vec<Value>, String> {
    let mut r = load(ctx, id)?;
    if r.restored_at.is_some() {
        return Err("ALREADY_RESTORED".into());
    }
    let mut out = Vec::new();
    for it in r.items.iter().rev() {
        let Some(undo) = &it.undo else { continue };
        let res = match undo {
            Undo::Quarantine { from, to } => {
                super::quarantine::restore(from, to).map(|_| "back from quarantine".to_string())
            }
            Undo::Trash { original } => restore_from_trash(std::slice::from_ref(original)),
            Undo::TrashMany { originals } => restore_from_trash(originals),
            Undo::Module {
                module,
                action,
                args,
            } => super::plan::module_undo(ctx, module, action, args),
        };
        out.push(match res {
            Ok(detail) => {
                json!({ "item_id": it.item_id, "label": it.label, "ok": true, "detail": detail })
            }
            Err(e) => json!({ "item_id": it.item_id, "label": it.label, "ok": false, "detail": e }),
        });
    }
    r.restored_at = Some(now());
    save(ctx, &r)?;
    Ok(out)
}

/// The Trash can put items back on Windows and Linux. macOS offers no API
/// for it, so the person uses "Put Back" in Finder.
fn restore_from_trash(originals: &[PathBuf]) -> Result<String, String> {
    #[cfg(any(
        windows,
        all(
            unix,
            not(target_os = "macos"),
            not(target_os = "ios"),
            not(target_os = "android")
        )
    ))]
    {
        let items = trash::os_limited::list().map_err(|e| e.to_string())?;
        let wanted: Vec<_> = items
            .into_iter()
            .filter(|i| originals.iter().any(|o| i.original_path() == *o))
            .collect();
        if wanted.is_empty() {
            return Err("not in the Trash anymore".into());
        }
        let n = wanted.len();
        trash::os_limited::restore_all(wanted).map_err(|e| e.to_string())?;
        Ok(format!("{n} item(s) back from the Trash"))
    }
    #[cfg(target_os = "macos")]
    {
        let _ = originals;
        Err("TRASH_MANUAL: open the Trash in Finder and use Put Back".into())
    }
}

pub async fn tool_list(a: Value) -> Result<Value, String> {
    let limit = crate::core::llm::tool_table::num(&a, "limit")
        .unwrap_or(20)
        .min(100) as usize;
    let one = crate::core::llm::tool_table::s(&a, "receipt_id");
    super::tools::blocking(move || {
        let ctx = Ctx::app()?;
        if !one.is_empty() {
            return Ok(json!({ "receipts": [load(&ctx, &one)?.summary()] }));
        }
        let list = list(&ctx, limit)?;
        Ok(json!({ "receipts": list.iter().map(|r| {
            let mut s = r.summary();
            if let Some(o) = s.as_object_mut() { o.remove("items"); }
            s
        }).collect::<Vec<_>>() }))
    })
    .await
}

pub async fn tool_restore(a: Value) -> Result<Value, String> {
    let id = crate::core::llm::tool_table::need_str(&a, "receipt_id")?;
    super::tools::blocking(move || {
        let ctx = Ctx::app()?;
        let items = restore(&ctx, &id)?;
        Ok(json!({ "receipt_id": id, "items": items }))
    })
    .await
}

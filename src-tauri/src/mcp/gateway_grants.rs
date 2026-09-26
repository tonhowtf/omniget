//! Local opt-in for the separate gateway; never selected by an MCP caller.
use super::{
    policy::{self, Principal},
    worker,
};
use rusqlite::{params, OptionalExtension};
use serde::Serialize;

#[derive(Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Boundary {
    pub write_enabled: bool,
    pub transfer_enabled: bool,
    pub boundary_version: u8,
}
fn db() -> Result<rusqlite::Connection, String> {
    let c = policy::db()?;
    c.execute_batch("CREATE TABLE IF NOT EXISTS gateway_grants(principal TEXT PRIMARY KEY,write_enabled INTEGER NOT NULL DEFAULT 0,transfer_enabled INTEGER NOT NULL DEFAULT 0);").map_err(|_|"GATEWAY_GRANT_UNAVAILABLE")?;
    Ok(c)
}
pub fn get(p: &Principal) -> Result<Boundary, String> {
    policy::active(p)?;
    let bits: Option<(bool, bool)> = db()?
        .query_row(
            "SELECT write_enabled,transfer_enabled FROM gateway_grants WHERE principal=?1",
            [&p.id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()
        .map_err(|_| "GATEWAY_GRANT_UNAVAILABLE")?;
    let (write, transfer) = bits.unwrap_or_default();
    Ok(Boundary {
        write_enabled: write && worker::isolation_available(),
        transfer_enabled: transfer
            && worker::isolation_available()
            && p.scopes.iter().any(|s| s == "transfer"),
        boundary_version: 1,
    })
}
/// Trusted Tauri surface only. Turning either bit off requires no working worker.
pub fn set(principal: &str, write: bool, transfer: bool) -> Result<(), String> {
    if (write || transfer) && !worker::isolation_available() {
        return Err("EXECUTION_ISOLATION_REQUIRED".into());
    }
    let mut c = db()?;
    let tx = c
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .map_err(|_| "GATEWAY_GRANT_UNAVAILABLE")?;
    let scopes: String = tx
        .query_row(
            "SELECT scopes FROM clients WHERE id=?1 AND revoked=0",
            [principal],
            |r| r.get(0),
        )
        .map_err(|_| "INVALID_CLIENT")?;
    let scopes: Vec<String> = serde_json::from_str(&scopes).map_err(|_| "INVALID_CLIENT")?;
    if transfer && !scopes.iter().any(|s| s == "transfer") {
        return Err("TRANSFER_NOT_GRANTED".into());
    }
    if write
        && !scopes.iter().any(|s| {
            matches!(
                s.as_str(),
                "enqueue" | "control" | "orchestration_create" | "orchestration_control"
            )
        })
    {
        return Err("WRITE_NOT_GRANTED".into());
    }
    tx.execute("INSERT INTO gateway_grants VALUES(?1,?2,?3) ON CONFLICT(principal) DO UPDATE SET write_enabled=excluded.write_enabled,transfer_enabled=excluded.transfer_enabled",params![principal,write,transfer]).map_err(|_|"GATEWAY_GRANT_UNAVAILABLE")?;
    tx.commit().map_err(|_| "GATEWAY_GRANT_UNAVAILABLE".into())
}

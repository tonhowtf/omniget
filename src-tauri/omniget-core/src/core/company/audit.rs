use rusqlite::{params, OptionalExtension, Transaction};
use serde::Serialize;
use serde_json::Value;
use sha2::{Digest, Sha256};

use super::{now_ms, sql, Actor, CompanyStore};

const GENESIS: &str = "0000000000000000000000000000000000000000000000000000000000000000";

#[derive(Debug, Clone, Serialize)]
pub struct AuditRow {
    pub seq: i64,
    pub company_id: String,
    pub ts_ms: i64,
    pub actor_kind: String,
    pub actor_id: String,
    pub channel: String,
    pub action: String,
    pub entity_kind: String,
    pub entity_id: String,
    pub run_id: Option<String>,
    pub details: Option<Value>,
    pub hash: String,
}

#[allow(clippy::too_many_arguments)]
fn line_hash(
    prev: &str,
    company: &str,
    ts: i64,
    actor_kind: &str,
    actor_id: &str,
    channel: &str,
    action: &str,
    entity_kind: &str,
    entity_id: &str,
    run_id: Option<&str>,
    details: Option<&str>,
) -> String {
    let mut h = Sha256::new();
    for part in [
        prev,
        company,
        &ts.to_string(),
        actor_kind,
        actor_id,
        channel,
        action,
        entity_kind,
        entity_id,
        run_id.unwrap_or(""),
        details.unwrap_or(""),
    ] {
        h.update((part.len() as u64).to_le_bytes());
        h.update(part.as_bytes());
    }
    hex::encode(h.finalize())
}

pub fn redact(v: &Value) -> Value {
    match v {
        Value::Object(m) => Value::Object(
            m.iter()
                .map(|(k, v)| {
                    let lk = k.to_ascii_lowercase();
                    let secret = [
                        "token",
                        "secret",
                        "password",
                        "api_key",
                        "apikey",
                        "authorization",
                    ]
                    .iter()
                    .any(|s| lk.contains(s));
                    if secret {
                        (k.clone(), Value::String("[redacted]".into()))
                    } else {
                        (k.clone(), redact(v))
                    }
                })
                .collect(),
        ),
        Value::Array(a) => Value::Array(a.iter().map(redact).collect()),
        Value::String(s) if looks_secret(s) => Value::String("[redacted]".into()),
        other => other.clone(),
    }
}

fn looks_secret(s: &str) -> bool {
    let t = s.trim();
    t.starts_with("sk-")
        || t.starts_with("Bearer ")
        || t.starts_with("ghp_")
        || t.starts_with("xox")
        || (t.len() > 40 && t.matches('.').count() == 2 && !t.contains(' '))
}

pub(crate) fn record(
    tx: &Transaction,
    company: &str,
    actor: &Actor,
    action: &str,
    entity_kind: &str,
    entity_id: &str,
    details: Option<Value>,
) -> Result<(), String> {
    let prev: String = tx
        .query_row(
            "SELECT hash FROM audit WHERE company_id = ?1 ORDER BY seq DESC LIMIT 1",
            params![company],
            |r| r.get(0),
        )
        .optional()
        .map_err(sql)?
        .unwrap_or_else(|| GENESIS.to_string());
    let ts = now_ms();
    let details = details.map(|d| redact(&d).to_string());
    let (kind, id, channel) = (actor.kind(), actor.id(), actor.channel().to_string());
    let run = actor.run_id().map(str::to_string);
    let hash = line_hash(
        &prev,
        company,
        ts,
        kind,
        &id,
        &channel,
        action,
        entity_kind,
        entity_id,
        run.as_deref(),
        details.as_deref(),
    );
    tx.execute(
        "INSERT INTO audit (company_id, ts_ms, actor_kind, actor_id, channel, action, entity_kind,
           entity_id, run_id, details, prev_hash, hash)
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12)",
        params![
            company,
            ts,
            kind,
            id,
            channel,
            action,
            entity_kind,
            entity_id,
            run,
            details,
            prev,
            hash
        ],
    )
    .map_err(sql)?;
    Ok(())
}

#[derive(Debug, Clone, Default, serde::Deserialize)]
pub struct AuditQuery {
    pub entity_kind: Option<String>,
    pub entity_id: Option<String>,
    pub actor_id: Option<String>,
    pub before_seq: Option<i64>,
    pub limit: Option<u32>,
}

impl CompanyStore {
    pub fn audit_list(&self, company: &str, q: &AuditQuery) -> Result<Vec<AuditRow>, String> {
        let limit = q.limit.unwrap_or(100).clamp(1, 1000) as i64;
        self.read(|c| {
            let mut st = c.prepare(
                "SELECT seq, company_id, ts_ms, actor_kind, actor_id, channel, action, entity_kind,
                        entity_id, run_id, details, hash
                 FROM audit
                 WHERE company_id = ?1
                   AND (?2 IS NULL OR entity_kind = ?2)
                   AND (?3 IS NULL OR entity_id = ?3)
                   AND (?4 IS NULL OR actor_id = ?4)
                   AND (?5 IS NULL OR seq < ?5)
                 ORDER BY seq DESC LIMIT ?6",
            )?;
            let rows = st.query_map(
                params![
                    company,
                    q.entity_kind,
                    q.entity_id,
                    q.actor_id,
                    q.before_seq,
                    limit
                ],
                |r| {
                    let details: Option<String> = r.get(10)?;
                    Ok(AuditRow {
                        seq: r.get(0)?,
                        company_id: r.get(1)?,
                        ts_ms: r.get(2)?,
                        actor_kind: r.get(3)?,
                        actor_id: r.get(4)?,
                        channel: r.get(5)?,
                        action: r.get(6)?,
                        entity_kind: r.get(7)?,
                        entity_id: r.get(8)?,
                        run_id: r.get(9)?,
                        details: details.and_then(|d| serde_json::from_str(&d).ok()),
                        hash: r.get(11)?,
                    })
                },
            )?;
            rows.collect()
        })
    }

    pub fn audit_verify(&self, company: &str) -> Result<u64, String> {
        type Line = (
            i64,
            i64,
            String,
            String,
            String,
            String,
            String,
            String,
            Option<String>,
            Option<String>,
            String,
            String,
        );
        let rows: Vec<Line> = self.read(|c| {
            let mut st = c.prepare(
                "SELECT seq, ts_ms, actor_kind, actor_id, channel, action, entity_kind, entity_id,
                        run_id, details, prev_hash, hash
                 FROM audit WHERE company_id = ?1 ORDER BY seq",
            )?;
            let rows = st.query_map(params![company], |r| {
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    r.get(2)?,
                    r.get(3)?,
                    r.get(4)?,
                    r.get(5)?,
                    r.get(6)?,
                    r.get(7)?,
                    r.get(8)?,
                    r.get(9)?,
                    r.get(10)?,
                    r.get(11)?,
                ))
            })?;
            rows.collect()
        })?;
        let mut prev = GENESIS.to_string();
        for (seq, ts, ak, ai, ch, action, ek, ei, run, details, prev_hash, hash) in &rows {
            let want = line_hash(
                &prev,
                company,
                *ts,
                ak,
                ai,
                ch,
                action,
                ek,
                ei,
                run.as_deref(),
                details.as_deref(),
            );
            if *prev_hash != prev || *hash != want {
                return Err(format!(
                    "{}: audit chain broken at seq {seq}",
                    super::ERR_COMPANY
                ));
            }
            prev = hash.clone();
        }
        Ok(rows.len() as u64)
    }

    pub fn audit_csv(&self, company: &str) -> Result<String, String> {
        let mut rows = self.audit_list(
            company,
            &AuditQuery {
                limit: Some(1000),
                ..Default::default()
            },
        )?;
        let mut all = Vec::new();
        while !rows.is_empty() {
            let last = rows.last().map(|r| r.seq);
            all.extend(rows);
            rows = self.audit_list(
                company,
                &AuditQuery {
                    limit: Some(1000),
                    before_seq: last,
                    ..Default::default()
                },
            )?;
        }
        all.reverse();
        let esc = |s: &str| format!("\"{}\"", s.replace('"', "\"\""));
        let mut out = String::from("seq,ts_ms,actor_kind,actor_id,channel,action,entity_kind,entity_id,run_id,details,hash\n");
        for r in all {
            out.push_str(&format!(
                "{},{},{},{},{},{},{},{},{},{},{}\n",
                r.seq,
                r.ts_ms,
                r.actor_kind,
                esc(&r.actor_id),
                r.channel,
                esc(&r.action),
                r.entity_kind,
                esc(&r.entity_id),
                r.run_id.unwrap_or_default(),
                esc(&r.details.map(|d| d.to_string()).unwrap_or_default()),
                r.hash
            ));
        }
        Ok(out)
    }
}

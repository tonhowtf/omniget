//! Writing, correcting, forgetting and listing records. Every read goes
//! through [`scope_filter`], so a scope the caller cannot read never reaches
//! a result, a count or an error message.

use rusqlite::types::Value as SqlValue;
use rusqlite::{params, Connection, OptionalExtension, Row};
use serde::{Deserialize, Serialize};

use super::super::ctx::{AssistCtx, Scope};
use super::super::db::AssistDb;
use super::super::tools::ERR_ASSIST_SCOPE;
use super::{
    content_hash, not_found, origin_hash, record_hash, search, Category, MemoryRecord, NewMemory,
    Source, Status, ERR_MEMORY_FORGOTTEN, ERR_MEMORY_INACTIVE, ERR_MEMORY_INVALID,
    MAX_CONTENT_CHARS,
};

/// Columns of `memory_records r`, in the order [`row_to_record`] reads them.
pub(super) const COLS: &str = "r.id, r.scope, r.category, r.subject, r.content, r.data, \
    r.source_kind, r.source_id, r.source_conversation, r.author, r.source_ms, r.confidence, \
    r.evidence, r.valid_until, r.version, r.chain, r.supersedes, r.status, r.created_ms, r.updated_ms";

fn bad_column(i: usize, what: &str) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(
        i,
        rusqlite::types::Type::Text,
        format!("unknown {what}").into(),
    )
}

pub(super) fn row_to_record(r: &Row) -> rusqlite::Result<MemoryRecord> {
    let scope: String = r.get(1)?;
    let category: String = r.get(2)?;
    let data: Option<String> = r.get(5)?;
    let status: String = r.get(17)?;
    Ok(MemoryRecord {
        id: r.get(0)?,
        scope: Scope::parse(&scope).ok_or_else(|| bad_column(1, "scope"))?,
        category: Category::parse(&category).ok_or_else(|| bad_column(2, "category"))?,
        subject: r.get(3)?,
        content: r.get(4)?,
        data: data.and_then(|d| serde_json::from_str(&d).ok()),
        source: Source {
            kind: r.get(6)?,
            id: r.get(7)?,
            conversation: r.get(8)?,
            author: r.get(9)?,
            at_ms: r.get(10)?,
        },
        confidence: r.get(11)?,
        evidence: r.get(12)?,
        valid_until: r.get(13)?,
        version: r.get(14)?,
        chain: r.get(15)?,
        supersedes: r.get(16)?,
        status: Status::parse(&status).ok_or_else(|| bad_column(17, "status"))?,
        created_ms: r.get(18)?,
        updated_ms: r.get(19)?,
    })
}

/// `col IN (<readable scopes>)`, with its parameters. The UI principal reads
/// every scope of its own; a ctx with no readable scope reads nothing.
pub(super) fn scope_filter(ctx: &AssistCtx, col: &str) -> (String, Vec<SqlValue>) {
    if ctx.is_user_ui() {
        return ("1=1".into(), Vec::new());
    }
    let keys = ctx.readable_keys();
    if keys.is_empty() {
        return ("0=1".into(), Vec::new());
    }
    let marks = vec!["?"; keys.len()].join(",");
    (
        format!("{col} IN ({marks})"),
        keys.into_iter().map(SqlValue::Text).collect(),
    )
}

pub(super) fn query_records(
    conn: &Connection,
    sql: &str,
    params: Vec<SqlValue>,
) -> rusqlite::Result<Vec<MemoryRecord>> {
    let mut stmt = conn.prepare(sql)?;
    let rows = stmt.query_map(rusqlite::params_from_iter(params), row_to_record)?;
    rows.collect()
}

pub(super) fn insert(
    conn: &Connection,
    principal: &str,
    rec: &MemoryRecord,
    origin: &str,
    content: &str,
) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO memory_records(id, principal, scope, category, subject, content, data,
            source_kind, source_id, source_conversation, author, source_ms, confidence, evidence,
            valid_until, version, chain, supersedes, status, origin_hash, content_hash,
            created_ms, updated_ms)
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19,?20,?21,?22,?23)",
        params![
            rec.id,
            principal,
            rec.scope.key(),
            rec.category.as_str(),
            rec.subject,
            rec.content,
            rec.data.as_ref().map(|d| d.to_string()),
            rec.source.kind,
            rec.source.id,
            rec.source.conversation,
            rec.source.author,
            rec.source.at_ms,
            rec.confidence,
            rec.evidence,
            rec.valid_until,
            rec.version,
            rec.chain,
            rec.supersedes,
            rec.status.as_str(),
            origin,
            content,
            rec.created_ms,
            rec.updated_ms,
        ],
    )?;
    if rec.status == Status::Active {
        index_one(conn, rec)?;
    }
    Ok(())
}

pub(super) fn index_one(conn: &Connection, rec: &MemoryRecord) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO memory_fts(content, subject, record_id, scope) VALUES (?1, ?2, ?3, ?4)",
        params![rec.content, rec.subject, rec.id, rec.scope.key()],
    )?;
    Ok(())
}

fn set_status(conn: &Connection, id: &str, status: Status, now: i64) -> rusqlite::Result<()> {
    conn.execute(
        "UPDATE memory_records SET status = ?2, updated_ms = ?3 WHERE id = ?1",
        params![id, status.as_str(), now],
    )?;
    conn.execute("DELETE FROM memory_fts WHERE record_id = ?1", [id])?;
    Ok(())
}

pub(super) fn clear_cache(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute("DELETE FROM memory_profile_cache", [])?;
    Ok(())
}

fn tombstoned(conn: &Connection, hashes: &[&str]) -> rusqlite::Result<Vec<String>> {
    let marks = vec!["?"; hashes.len()].join(",");
    let mut stmt = conn.prepare(&format!(
        "SELECT hash FROM memory_tombstones WHERE hash IN ({marks})"
    ))?;
    let rows = stmt.query_map(rusqlite::params_from_iter(hashes.iter()), |r| r.get(0))?;
    rows.collect()
}

fn db_err(e: rusqlite::Error) -> String {
    format!("{}: {e}", super::super::db::ERR_ASSIST_DB)
}

fn clean_content(s: &str) -> Result<String, String> {
    let t = s.trim();
    if t.is_empty() {
        return Err(format!("{ERR_MEMORY_INVALID}: the memory is empty"));
    }
    if t.chars().count() > MAX_CONTENT_CHARS {
        return Err(format!(
            "{ERR_MEMORY_INVALID}: keep one memory under {MAX_CONTENT_CHARS} characters"
        ));
    }
    Ok(t.to_string())
}

fn clean_subject(s: Option<&str>) -> Option<String> {
    s.map(|s| s.trim().to_lowercase())
        .filter(|s| !s.is_empty())
        .map(|s| s.chars().take(120).collect())
}

/// End of the local day `now` falls in (default validity of temporary context).
pub(crate) fn end_of_local_day(now: i64) -> i64 {
    use chrono::{Local, TimeZone};
    let Some(dt) = Local.timestamp_millis_opt(now).single() else {
        return now + 24 * 3_600_000;
    };
    dt.date_naive()
        .succ_opt()
        .and_then(|d| d.and_hms_opt(0, 0, 0))
        .and_then(|n| Local.from_local_datetime(&n).earliest())
        .map(|d| d.timestamp_millis())
        .unwrap_or(now + 24 * 3_600_000)
}

fn clamp_confidence(c: f64) -> f64 {
    if c.is_finite() {
        c.clamp(0.0, 1.0)
    } else {
        0.5
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Written {
    pub record: MemoryRecord,
    /// False when an equal record already existed (idempotent write) or a
    /// stated preference already covers the subject.
    pub created: bool,
    /// `duplicate` | `declared_exists` | `downgraded_to_candidate`.
    pub note: Option<String>,
}

/// Writes one record. Idempotent per origin; a subject already covered by an
/// active record of the same group is superseded (a new version of that
/// chain), except that an inference never displaces a stated preference.
pub fn remember(
    db: &AssistDb,
    ctx: &AssistCtx,
    new: NewMemory,
    now: i64,
) -> Result<Written, String> {
    if !ctx.can_write(&new.scope) {
        return Err(format!(
            "{ERR_ASSIST_SCOPE}: this conversation cannot save memories there"
        ));
    }
    let content = clean_content(&new.content)?;
    let subject = clean_subject(new.subject.as_deref());
    let scope_key = new.scope.key();
    let principal = ctx.principal.clone();
    let oh = origin_hash(
        &principal,
        &scope_key,
        &new.source.kind,
        new.source.id.as_deref(),
        &content,
    );
    let ch = content_hash(&principal, &scope_key, &content);
    let valid_until = match new.category {
        Category::Temporary => Some(new.valid_until.unwrap_or_else(|| end_of_local_day(now))),
        _ => new.valid_until,
    };
    if let Some(v) = valid_until {
        if v <= now {
            return Err(format!("{ERR_MEMORY_INVALID}: the validity already ended"));
        }
    }

    db.tx(|tx| {
        // Same origin, same content: the same record.
        let existing = query_records(
            tx,
            &format!("SELECT {COLS} FROM memory_records r WHERE r.principal = ? AND r.origin_hash = ? LIMIT 1"),
            vec![principal.clone().into(), oh.clone().into()],
        )
        .map_err(db_err)?;
        if let Some(rec) = existing.into_iter().next() {
            return Ok(Written { record: rec, created: false, note: Some("duplicate".into()) });
        }
        let hits = tombstoned(tx, &[&oh, &ch]).map_err(db_err)?;
        if !hits.is_empty() {
            // Only the person, on the memory screen, may bring back
            // something they asked to forget.
            if ctx.is_user_ui() && new.source.author == "user" {
                for h in &hits {
                    tx.execute("DELETE FROM memory_tombstones WHERE hash = ?1", [h]).map_err(db_err)?;
                }
            } else {
                return Err(format!(
                    "{ERR_MEMORY_FORGOTTEN}: the person asked to forget this; only they can add it again on the memory screen"
                ));
            }
        }
        let same = query_records(
            tx,
            &format!("SELECT {COLS} FROM memory_records r WHERE r.principal = ? AND r.scope = ? AND r.content_hash = ? AND r.category = ? AND r.status = 'active' LIMIT 1"),
            vec![principal.clone().into(), scope_key.clone().into(), ch.clone().into(), new.category.as_str().to_string().into()],
        )
        .map_err(db_err)?;
        if let Some(rec) = same.into_iter().next() {
            return Ok(Written { record: rec, created: false, note: Some("duplicate".into()) });
        }

        let id = super::super::new_id();
        let mut chain = id.clone();
        let mut version = 1;
        let mut supersedes = None;
        if let (Some(subj), Some(group)) = (&subject, new.category.conflict_group()) {
            let cats: Vec<&str> = [Category::Declared, Category::Inference, Category::Temporary, Category::Procedural]
                .into_iter()
                .filter(|c| c.conflict_group() == Some(group))
                .map(Category::as_str)
                .collect();
            let marks = vec!["?"; cats.len()].join(",");
            let mut p: Vec<SqlValue> = vec![principal.clone().into(), scope_key.clone().into(), subj.clone().into()];
            p.extend(cats.iter().map(|c| SqlValue::Text(c.to_string())));
            let current = query_records(
                tx,
                &format!("SELECT {COLS} FROM memory_records r WHERE r.principal = ? AND r.scope = ? AND r.subject = ? AND r.status = 'active' AND r.category IN ({marks}) ORDER BY r.updated_ms DESC"),
                p,
            )
            .map_err(db_err)?;
            if new.category == Category::Inference {
                if let Some(declared) = current.iter().find(|r| r.category == Category::Declared) {
                    return Ok(Written { record: declared.clone(), created: false, note: Some("declared_exists".into()) });
                }
            }
            if let Some(head) = current.first() {
                chain = head.chain.clone();
                supersedes = Some(head.id.clone());
                version = max_version(tx, &chain).map_err(db_err)? + 1;
                for old in &current {
                    set_status(tx, &old.id, Status::Superseded, now).map_err(db_err)?;
                }
            }
        }
        let rec = MemoryRecord {
            id,
            scope: new.scope.clone(),
            category: new.category,
            subject,
            content: content.clone(),
            data: new.data.clone(),
            source: new.source.clone(),
            confidence: clamp_confidence(new.confidence.unwrap_or(new.category.default_confidence())),
            evidence: new.evidence.clone().filter(|e| !e.trim().is_empty()),
            valid_until,
            version,
            chain,
            supersedes,
            status: Status::Active,
            created_ms: now,
            updated_ms: now,
        };
        insert(tx, &principal, &rec, &oh, &ch).map_err(db_err)?;
        clear_cache(tx).map_err(db_err)?;
        Ok(Written { record: rec, created: true, note: None })
    })
}

fn max_version(conn: &Connection, chain: &str) -> rusqlite::Result<i64> {
    conn.query_row(
        "SELECT COALESCE(MAX(version), 0) FROM memory_records WHERE chain = ?1",
        [chain],
        |r| r.get(0),
    )
}

/// One record the caller may read, whatever its status.
pub fn get(db: &AssistDb, ctx: &AssistCtx, id: &str) -> Result<MemoryRecord, String> {
    let (filter, mut p) = scope_filter(ctx, "r.scope");
    let mut params: Vec<SqlValue> = vec![ctx.principal.clone().into(), id.to_string().into()];
    params.append(&mut p);
    db.with(|c| {
        query_records(
            c,
            &format!("SELECT {COLS} FROM memory_records r WHERE r.principal = ? AND r.id = ? AND {filter}"),
            params,
        )
    })?
    .into_iter()
    .next()
    .ok_or_else(not_found)
}

fn writable(ctx: &AssistCtx, rec: &MemoryRecord) -> Result<(), String> {
    if ctx.can_write(&rec.scope) {
        Ok(())
    } else {
        Err(format!(
            "{ERR_ASSIST_SCOPE}: this conversation can read this memory but not change it"
        ))
    }
}

fn active(db: &AssistDb, ctx: &AssistCtx, rec: &MemoryRecord) -> Result<(), String> {
    if rec.status == Status::Active {
        return Ok(());
    }
    let (filter, mut p) = scope_filter(ctx, "r.scope");
    let mut params: Vec<SqlValue> = vec![ctx.principal.clone().into(), rec.chain.clone().into()];
    params.append(&mut p);
    let current = db
        .with(|c| {
            query_records(
                c,
                &format!("SELECT {COLS} FROM memory_records r WHERE r.principal = ? AND r.chain = ? AND r.status = 'active' AND {filter} LIMIT 1"),
                params,
            )
        })?
        .into_iter()
        .next();
    Err(match current {
        Some(cur) => format!(
            "{ERR_MEMORY_INACTIVE}: this memory was already replaced; change the current version `{}`",
            cur.id
        ),
        None => format!("{ERR_MEMORY_INACTIVE}: this memory is no longer active"),
    })
}

/// A new version of an active record.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Correction {
    pub content: String,
    #[serde(default)]
    pub category: Option<Category>,
    #[serde(default)]
    pub subject: Option<String>,
    #[serde(default)]
    pub evidence: Option<String>,
    #[serde(default)]
    pub confidence: Option<f64>,
    #[serde(default)]
    pub valid_until: Option<i64>,
}

/// "Actually…": the record becomes `superseded` and a new version of its
/// chain takes its place. Returns the new version.
pub fn correct(
    db: &AssistDb,
    ctx: &AssistCtx,
    id: &str,
    c: Correction,
    source: Source,
    now: i64,
) -> Result<MemoryRecord, String> {
    let old = get(db, ctx, id)?;
    writable(ctx, &old)?;
    active(db, ctx, &old)?;
    let content = clean_content(&c.content)?;
    let category = c.category.unwrap_or(old.category);
    let scope_key = old.scope.key();
    let principal = ctx.principal.clone();
    let oh = origin_hash(
        &principal,
        &scope_key,
        &source.kind,
        source.id.as_deref(),
        &content,
    );
    let ch = content_hash(&principal, &scope_key, &content);
    let valid_until = match category {
        Category::Temporary => Some(
            c.valid_until
                .or(old.valid_until)
                .unwrap_or_else(|| end_of_local_day(now)),
        ),
        _ => c.valid_until,
    };
    db.tx(|tx| {
        let hits = tombstoned(tx, &[&oh, &ch]).map_err(db_err)?;
        if !hits.is_empty() {
            if ctx.is_user_ui() && source.author == "user" {
                for h in &hits {
                    tx.execute("DELETE FROM memory_tombstones WHERE hash = ?1", [h]).map_err(db_err)?;
                }
            } else {
                return Err(format!(
                    "{ERR_MEMORY_FORGOTTEN}: the person asked to forget this; only they can add it again on the memory screen"
                ));
            }
        }
        let version = max_version(tx, &old.chain).map_err(db_err)? + 1;
        set_status(tx, &old.id, Status::Superseded, now).map_err(db_err)?;
        // A different origin can hash like an older version of this same
        // chain; the origin index is not unique, so the chain stays intact.
        let rec = MemoryRecord {
            id: super::super::new_id(),
            scope: old.scope.clone(),
            category,
            subject: clean_subject(c.subject.as_deref()).or(old.subject.clone()),
            content,
            data: None,
            source: source.clone(),
            confidence: clamp_confidence(c.confidence.unwrap_or(category.default_confidence())),
            evidence: c.evidence.clone().or_else(|| {
                (category == Category::Inference || old.category == Category::Inference)
                    .then(|| old.evidence.clone())
                    .flatten()
            }),
            valid_until,
            version,
            chain: old.chain.clone(),
            supersedes: Some(old.id.clone()),
            status: Status::Active,
            created_ms: now,
            updated_ms: now,
        };
        insert(tx, &principal, &rec, &oh, &ch).map_err(db_err)?;
        clear_cache(tx).map_err(db_err)?;
        Ok(rec)
    })
}

/// "That was not it": the record leaves recall and profile, and stays in the
/// history as withdrawn (unlike [`forget`]).
pub fn retract(db: &AssistDb, ctx: &AssistCtx, id: &str, now: i64) -> Result<MemoryRecord, String> {
    let old = get(db, ctx, id)?;
    writable(ctx, &old)?;
    active(db, ctx, &old)?;
    db.tx(|tx| {
        set_status(tx, &old.id, Status::Retracted, now).map_err(db_err)?;
        clear_cache(tx).map_err(db_err)
    })?;
    get(db, ctx, id)
}

/// The person confirms a candidate: a new version, stated by them.
pub fn confirm(db: &AssistDb, ctx: &AssistCtx, id: &str, now: i64) -> Result<MemoryRecord, String> {
    if !ctx.is_user_ui() {
        return Err(format!(
            "{ERR_ASSIST_SCOPE}: only the person can confirm a guess"
        ));
    }
    let old = get(db, ctx, id)?;
    if old.category != Category::Inference {
        return Err(format!(
            "{ERR_MEMORY_INVALID}: only guesses need confirming"
        ));
    }
    correct(
        db,
        ctx,
        id,
        Correction {
            content: old.content.clone(),
            category: Some(Category::Declared),
            subject: old.subject.clone(),
            evidence: old.evidence.clone(),
            confidence: Some(1.0),
            valid_until: None,
        },
        Source {
            kind: "user".into(),
            id: Some(format!("confirm:{}", old.id)),
            conversation: None,
            author: "user".into(),
            at_ms: now,
        },
        now,
    )
}

/// Removes the record's whole chain (every version), any other record with
/// the same text in the same scope, their index rows and the cached
/// profiles, and leaves hashes behind so nothing brings them back. Returns
/// how many rows were removed.
pub fn forget(db: &AssistDb, ctx: &AssistCtx, id: &str, now: i64) -> Result<usize, String> {
    let rec = get(db, ctx, id)?;
    writable(ctx, &rec)?;
    let principal = ctx.principal.clone();
    // Overwrite freed pages instead of leaving the text in them.
    db.with(|c| c.execute_batch("PRAGMA secure_delete = ON;"))?;
    let removed = db.tx(|tx| {
        let mut chains: Vec<String> = vec![rec.chain.clone()];
        let hashes: Vec<String> = {
            let mut s = tx
                .prepare("SELECT content_hash FROM memory_records WHERE principal = ?1 AND chain = ?2")
                .map_err(db_err)?;
            let rows = s
                .query_map(params![principal, rec.chain], |r| r.get::<_, String>(0))
                .map_err(db_err)?;
            rows.collect::<rusqlite::Result<_>>().map_err(db_err)?
        };
        for h in &hashes {
            let mut s = tx
                .prepare("SELECT DISTINCT chain FROM memory_records WHERE principal = ?1 AND content_hash = ?2")
                .map_err(db_err)?;
            let rows = s
                .query_map(params![principal, h], |r| r.get::<_, String>(0))
                .map_err(db_err)?;
            for c in rows {
                let c = c.map_err(db_err)?;
                if !chains.contains(&c) {
                    chains.push(c);
                }
            }
        }
        // The same words may live on as a film reaction in a reading
        // journey; forgetting here must reach that copy too.
        let texts: Vec<String> = {
            let mut s = tx
                .prepare("SELECT content FROM memory_records WHERE principal = ?1 AND chain = ?2")
                .map_err(db_err)?;
            let mut out = Vec::new();
            for chain in &chains {
                let rows = s
                    .query_map(params![principal, chain], |r| r.get::<_, String>(0))
                    .map_err(db_err)?;
                for t in rows {
                    out.push(t.map_err(db_err)?);
                }
            }
            out
        };
        let journey_scopes = (!ctx.is_user_ui()).then(|| {
            ctx.writable.iter().map(|s| s.key()).collect::<Vec<String>>()
        });
        crate::core::assist::reading::store::forget_reactions_mentioned(
            tx,
            &principal,
            journey_scopes.as_deref(),
            &texts,
        )
        .map_err(db_err)?;
        let mut removed = 0usize;
        for chain in &chains {
            let rows: Vec<(String, String, String, String)> = {
                let mut s = tx
                    .prepare("SELECT id, scope, origin_hash, content_hash FROM memory_records WHERE principal = ?1 AND chain = ?2")
                    .map_err(db_err)?;
                let rows = s
                    .query_map(params![principal, chain], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
                    .map_err(db_err)?;
                rows.collect::<rusqlite::Result<_>>().map_err(db_err)?
            };
            for (rid, scope, oh, ch) in &rows {
                for (hash, kind) in [(oh.clone(), "origin"), (ch.clone(), "content"), (record_hash(rid), "record")] {
                    tx.execute(
                        "INSERT OR IGNORE INTO memory_tombstones(hash, principal, scope, kind, forgotten_ms) VALUES (?1, ?2, ?3, ?4, ?5)",
                        params![hash, principal, scope, kind, now],
                    )
                    .map_err(db_err)?;
                }
                tx.execute("DELETE FROM memory_fts WHERE record_id = ?1", [rid]).map_err(db_err)?;
            }
            removed += tx
                .execute("DELETE FROM memory_records WHERE principal = ?1 AND chain = ?2", params![principal, chain])
                .map_err(db_err)?;
        }
        clear_cache(tx).map_err(db_err)?;
        Ok(removed)
    })?;
    search::compact(db);
    Ok(removed)
}

/// Every version of the record's chain the caller may read, oldest first.
pub fn history(db: &AssistDb, ctx: &AssistCtx, id: &str) -> Result<Vec<MemoryRecord>, String> {
    let rec = get(db, ctx, id)?;
    let (filter, mut p) = scope_filter(ctx, "r.scope");
    let mut params: Vec<SqlValue> = vec![ctx.principal.clone().into(), rec.chain.into()];
    params.append(&mut p);
    db.with(|c| {
        query_records(
            c,
            &format!("SELECT {COLS} FROM memory_records r WHERE r.principal = ? AND r.chain = ? AND {filter} ORDER BY r.version"),
            params,
        )
    })
}

/// Marks temporary context whose validity ended as `expired`.
pub fn expire_due(db: &AssistDb, now: i64) -> Result<usize, String> {
    db.tx(|tx| {
        let ids: Vec<String> = {
            let mut s = tx
                .prepare("SELECT id FROM memory_records WHERE status = 'active' AND valid_until IS NOT NULL AND valid_until <= ?1")
                .map_err(db_err)?;
            let rows = s.query_map([now], |r| r.get(0)).map_err(db_err)?;
            rows.collect::<rusqlite::Result<_>>().map_err(db_err)?
        };
        for id in &ids {
            set_status(tx, id, Status::Expired, now).map_err(db_err)?;
        }
        if !ids.is_empty() {
            clear_cache(tx).map_err(db_err)?;
        }
        Ok(ids.len())
    })
}

/// Records the caller may read, optionally of one scope; active ones first.
pub fn list(
    db: &AssistDb,
    ctx: &AssistCtx,
    scope: Option<&Scope>,
    include_inactive: bool,
    now: i64,
) -> Result<Vec<MemoryRecord>, String> {
    expire_due(db, now)?;
    if let Some(s) = scope {
        if !ctx.can_read(s) {
            return Ok(Vec::new());
        }
    }
    let (filter, mut p) = scope_filter(ctx, "r.scope");
    let mut params: Vec<SqlValue> = vec![ctx.principal.clone().into()];
    let mut sql = format!("SELECT {COLS} FROM memory_records r WHERE r.principal = ? AND {filter}");
    params.append(&mut p);
    if let Some(s) = scope {
        sql.push_str(" AND r.scope = ?");
        params.push(s.key().into());
    }
    if !include_inactive {
        sql.push_str(" AND r.status = 'active'");
    }
    sql.push_str(" ORDER BY (r.status <> 'active'), r.category, r.updated_ms DESC LIMIT 2000");
    db.with(|c| query_records(c, &sql, params))
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ScopeSummary {
    pub scope: Scope,
    pub active: i64,
    pub candidates: i64,
    pub inactive: i64,
}

/// Scopes that hold records the caller may read, with counts.
pub fn scopes(db: &AssistDb, ctx: &AssistCtx, now: i64) -> Result<Vec<ScopeSummary>, String> {
    expire_due(db, now)?;
    let (filter, mut p) = scope_filter(ctx, "r.scope");
    let mut params: Vec<SqlValue> = vec![ctx.principal.clone().into()];
    params.append(&mut p);
    let rows: Vec<(String, i64, i64, i64)> = db.with(|c| {
        let mut s = c.prepare(&format!(
            "SELECT r.scope,
                SUM(r.status = 'active'),
                SUM(r.status = 'active' AND r.category = 'inference'),
                SUM(r.status <> 'active')
             FROM memory_records r WHERE r.principal = ? AND {filter}
             GROUP BY r.scope ORDER BY r.scope"
        ))?;
        let rows = s.query_map(rusqlite::params_from_iter(params), |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))
        })?;
        rows.collect()
    })?;
    Ok(rows
        .into_iter()
        .filter_map(|(k, a, c, i)| {
            Scope::parse(&k).map(|scope| ScopeSummary {
                scope,
                active: a,
                candidates: c,
                inactive: i,
            })
        })
        .collect())
}

/// Whether a record id exists at all, any principal or scope. Internal to
/// import (never exposed through a ctx).
pub(super) fn exists_id(conn: &Connection, id: &str) -> rusqlite::Result<Option<String>> {
    conn.query_row(
        "SELECT chain FROM memory_records WHERE id = ?1",
        [id],
        |r| r.get(0),
    )
    .optional()
}

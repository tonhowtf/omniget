//! Export (versioned JSON + readable Markdown), validated import, and the
//! app's own database backups that may still hold forgotten text.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use rusqlite::params;
use serde::{Deserialize, Serialize};

use super::super::ctx::{AssistCtx, Scope};
use super::super::db::AssistDb;
use super::records::{exists_id, insert, query_records, scope_filter, COLS};
use super::search::{compact, rebuild_index};
use super::{
    content_hash, origin_hash, record_hash, Category, MemoryRecord, Status, ERR_MEMORY_IMPORT,
    MAX_CONTENT_CHARS,
};

pub const EXPORT_FORMAT: &str = "omniget.memory";
pub const EXPORT_VERSION: u32 = 1;

/// A forgotten record's fingerprint: a hash, never the text.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Tombstone {
    pub hash: String,
    pub scope: String,
    pub kind: String,
    pub forgotten_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExportFile {
    pub format: String,
    pub version: u32,
    pub exported_ms: i64,
    pub records: Vec<MemoryRecord>,
    #[serde(default)]
    pub tombstones: Vec<Tombstone>,
}

fn db_err(e: rusqlite::Error) -> String {
    format!("{}: {e}", super::super::db::ERR_ASSIST_DB)
}

fn gather(db: &AssistDb, ctx: &AssistCtx) -> Result<(Vec<MemoryRecord>, Vec<Tombstone>), String> {
    let (filter, fp) = scope_filter(ctx, "r.scope");
    let mut params: Vec<rusqlite::types::Value> = vec![ctx.principal.clone().into()];
    params.extend(fp.clone());
    let records = db.with(|c| {
        query_records(
            c,
            &format!("SELECT {COLS} FROM memory_records r WHERE r.principal = ? AND {filter} ORDER BY r.scope, r.chain, r.version"),
            params,
        )
    })?;
    let (tfilter, tp) = scope_filter(ctx, "t.scope");
    let mut tparams: Vec<rusqlite::types::Value> = vec![ctx.principal.clone().into()];
    tparams.extend(tp);
    let tombstones = db.with(|c| {
        let mut s = c.prepare(&format!(
            "SELECT t.hash, t.scope, t.kind, t.forgotten_ms FROM memory_tombstones t WHERE t.principal = ? AND {tfilter} ORDER BY t.forgotten_ms, t.hash"
        ))?;
        let rows = s.query_map(rusqlite::params_from_iter(tparams), |r| {
            Ok(Tombstone { hash: r.get(0)?, scope: r.get(1)?, kind: r.get(2)?, forgotten_ms: r.get(3)? })
        })?;
        rows.collect()
    })?;
    Ok((records, tombstones))
}

/// Every record the caller may read (all versions, all statuses) plus the
/// tombstones of those scopes, as a versioned JSON document.
pub fn export_json(db: &AssistDb, ctx: &AssistCtx, now: i64) -> Result<String, String> {
    let (records, tombstones) = gather(db, ctx)?;
    let file = ExportFile {
        format: EXPORT_FORMAT.into(),
        version: EXPORT_VERSION,
        exported_ms: now,
        records,
        tombstones,
    };
    serde_json::to_string_pretty(&file).map_err(|e| format!("{ERR_MEMORY_IMPORT}: {e}"))
}

fn scope_title(s: &Scope) -> String {
    match s {
        Scope::User => "Your profile (shared by your personal bots)".into(),
        Scope::Bot { bot } => format!("Private memory of bot `{bot}`"),
        Scope::Room { conversation } => format!("Shared memory of room `{conversation}`"),
    }
}

/// A readable copy of the active records. Import reads only the JSON export.
pub fn export_markdown(db: &AssistDb, ctx: &AssistCtx, now: i64) -> Result<String, String> {
    let (records, tombstones) = gather(db, ctx)?;
    let mut out = String::new();
    out.push_str("# OmniGet memory\n\n");
    out.push_str(&format!(
        "Exported {}. Only the JSON export can be imported back.\n",
        chrono::DateTime::from_timestamp_millis(now)
            .map(|d| d.format("%Y-%m-%d %H:%M UTC").to_string())
            .unwrap_or_default()
    ));
    let inactive = records
        .iter()
        .filter(|r| r.status != Status::Active)
        .count();
    out.push_str(&format!(
        "Earlier versions and withdrawn items: {inactive} (in the JSON export). Forgotten items: {} fingerprints, no text.\n",
        tombstones.len()
    ));
    let mut by_scope: Vec<Scope> = records.iter().map(|r| r.scope.clone()).collect();
    by_scope.sort();
    by_scope.dedup();
    let sections = [
        (Category::Declared, "Stated by you"),
        (Category::Temporary, "Only for a while"),
        (Category::Procedural, "Working notes"),
        (Category::Inference, "Guesses waiting for your confirmation"),
        (Category::Observation, "Events"),
    ];
    for scope in by_scope {
        out.push_str(&format!("\n## {}\n", scope_title(&scope)));
        for (cat, title) in sections {
            let items: Vec<&MemoryRecord> = records
                .iter()
                .filter(|r| r.scope == scope && r.category == cat && r.status == Status::Active)
                .collect();
            if items.is_empty() {
                continue;
            }
            out.push_str(&format!("\n### {title}\n\n"));
            for r in items {
                out.push_str(&format!(
                    "- {} {}\n",
                    r.content.replace('\n', " "),
                    super::search::source_line(r)
                ));
            }
        }
    }
    Ok(out)
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct ImportReport {
    pub imported: usize,
    /// Already present (same id or same origin); left untouched.
    pub duplicates: usize,
    /// Forgotten here or in the file; not restored (see `allow_resurrect`).
    pub skipped_forgotten: usize,
    /// Restored although forgotten, because the person chose to.
    pub resurrected: usize,
    /// Two active records about the same subject: the newer one stays active.
    pub conflicts_resolved: usize,
    pub tombstones_added: usize,
    /// One line per record that failed validation.
    pub rejected: Vec<String>,
}

fn is_hash(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit())
}

/// Imports a JSON export. Validates format and version, keeps ids and links
/// (a version whose predecessor is missing is rejected), skips duplicates,
/// and never restores a forgotten record unless `allow_resurrect` is set.
pub fn import_json(
    db: &AssistDb,
    ctx: &AssistCtx,
    text: &str,
    allow_resurrect: bool,
    now: i64,
) -> Result<ImportReport, String> {
    let head: serde_json::Value = serde_json::from_str(text)
        .map_err(|_| format!("{ERR_MEMORY_IMPORT}: this file is not a memory export"))?;
    if head.get("format").and_then(|v| v.as_str()) != Some(EXPORT_FORMAT) {
        return Err(format!(
            "{ERR_MEMORY_IMPORT}: this file is not a memory export"
        ));
    }
    match head.get("version").and_then(|v| v.as_u64()) {
        Some(v) if v == EXPORT_VERSION as u64 => {}
        Some(v) if v > EXPORT_VERSION as u64 => {
            return Err(format!(
                "{ERR_MEMORY_IMPORT}: the export is from a newer version (format {v}); update the app first"
            ))
        }
        _ => return Err(format!("{ERR_MEMORY_IMPORT}: unknown export version")),
    }
    let file: ExportFile = serde_json::from_value(head)
        .map_err(|e| format!("{ERR_MEMORY_IMPORT}: the export is damaged ({e})"))?;
    let principal = ctx.principal.clone();

    let report = db.tx(|tx| {
        let mut rep = ImportReport::default();
        for t in &file.tombstones {
            let scope_ok = Scope::parse(&t.scope).map(|s| ctx.can_write(&s)).unwrap_or(false);
            if !is_hash(&t.hash) || !["content", "origin", "record"].contains(&t.kind.as_str()) || !scope_ok {
                rep.rejected.push("a forgotten-item fingerprint is invalid".into());
                continue;
            }
            rep.tombstones_added += tx
                .execute(
                    "INSERT OR IGNORE INTO memory_tombstones(hash, principal, scope, kind, forgotten_ms) VALUES (?1, ?2, ?3, ?4, ?5)",
                    params![t.hash, principal, t.scope, t.kind, t.forgotten_ms],
                )
                .map_err(db_err)?;
        }

        let mut records = file.records.clone();
        records.sort_by(|a, b| (a.chain.as_str(), a.version).cmp(&(b.chain.as_str(), b.version)));
        // file id → id in this database; file chain → chain in this database.
        let mut idmap: HashMap<String, String> = HashMap::new();
        let mut chainmap: HashMap<String, String> = HashMap::new();
        let mut forgotten: HashSet<String> = HashSet::new();
        let mut touched: HashSet<String> = HashSet::new();

        for (i, rec) in records.iter().enumerate() {
            let label = format!("record {}", i + 1);
            if !ctx.can_write(&rec.scope) {
                rep.rejected.push(format!("{label}: not a scope you can write"));
                continue;
            }
            let content = rec.content.trim();
            if content.is_empty() || content.chars().count() > MAX_CONTENT_CHARS || rec.id.trim().is_empty() || rec.version < 1 {
                rep.rejected.push(format!("{label}: invalid content, id or version"));
                continue;
            }
            // Links: a predecessor must come from the file or exist here.
            if forgotten.contains(&rec.chain) || rec.supersedes.as_ref().map(|s| forgotten.contains(s)).unwrap_or(false) {
                forgotten.insert(rec.id.clone());
                rep.skipped_forgotten += 1;
                continue;
            }
            let chain = if rec.chain == rec.id {
                rec.id.clone()
            } else if let Some(c) = chainmap.get(&rec.chain) {
                c.clone()
            } else if let Some(c) = exists_id(tx, &rec.chain).map_err(db_err)? {
                c
            } else {
                rep.rejected.push(format!("{label}: its first version is missing"));
                continue;
            };
            let supersedes = match &rec.supersedes {
                None => None,
                Some(s) => match idmap.get(s) {
                    Some(m) => Some(m.clone()),
                    None if exists_id(tx, s).map_err(db_err)?.is_some() => Some(s.clone()),
                    None => {
                        rep.rejected.push(format!("{label}: the version it replaces is missing"));
                        continue;
                    }
                },
            };
            let scope_key = rec.scope.key();
            let oh = origin_hash(&principal, &scope_key, &rec.source.kind, rec.source.id.as_deref(), content);
            let ch = content_hash(&principal, &scope_key, content);
            let rh = record_hash(&rec.id);
            let hits: Vec<String> = {
                let mut s = tx
                    .prepare("SELECT hash FROM memory_tombstones WHERE hash IN (?1, ?2, ?3)")
                    .map_err(db_err)?;
                let rows = s.query_map(params![oh, ch, rh], |r| r.get(0)).map_err(db_err)?;
                rows.collect::<rusqlite::Result<_>>().map_err(db_err)?
            };
            if !hits.is_empty() {
                if !allow_resurrect {
                    forgotten.insert(rec.id.clone());
                    rep.skipped_forgotten += 1;
                    continue;
                }
                for h in &hits {
                    tx.execute("DELETE FROM memory_tombstones WHERE hash = ?1", [h]).map_err(db_err)?;
                }
                rep.resurrected += 1;
            }
            if let Some(existing_chain) = exists_id(tx, &rec.id).map_err(db_err)? {
                idmap.insert(rec.id.clone(), rec.id.clone());
                chainmap.entry(rec.chain.clone()).or_insert(existing_chain);
                rep.duplicates += 1;
                continue;
            }
            let dup: Option<(String, String)> = {
                use rusqlite::OptionalExtension;
                tx.query_row(
                    "SELECT id, chain FROM memory_records WHERE principal = ?1 AND origin_hash = ?2 LIMIT 1",
                    params![principal, oh],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .optional()
                .map_err(db_err)?
            };
            if let Some((did, dchain)) = dup {
                idmap.insert(rec.id.clone(), did);
                chainmap.entry(rec.chain.clone()).or_insert(dchain);
                rep.duplicates += 1;
                continue;
            }

            let mut out = rec.clone();
            out.content = content.to_string();
            out.chain = chain.clone();
            out.supersedes = supersedes;
            out.confidence = if out.confidence.is_finite() { out.confidence.clamp(0.0, 1.0) } else { 0.5 };
            if out.status == Status::Active && out.valid_until.map(|v| v <= now).unwrap_or(false) {
                out.status = Status::Expired;
            }
            let taken: bool = tx
                .query_row(
                    "SELECT count(*) FROM memory_records WHERE chain = ?1 AND version = ?2",
                    params![chain, out.version],
                    |r| r.get::<_, i64>(0),
                )
                .map_err(db_err)?
                > 0;
            if taken {
                out.version = tx
                    .query_row("SELECT COALESCE(MAX(version),0)+1 FROM memory_records WHERE chain = ?1", [&chain], |r| r.get(0))
                    .map_err(db_err)?;
            }
            // One active record per subject: the newer stays active; a guess
            // never displaces a stated preference.
            if out.status == Status::Active {
                if let (Some(subj), Some(group)) = (&out.subject, out.category.conflict_group()) {
                    let cats: Vec<Category> = [Category::Declared, Category::Inference, Category::Temporary, Category::Procedural]
                        .into_iter()
                        .filter(|c| c.conflict_group() == Some(group))
                        .collect();
                    let current = query_records(
                        tx,
                        &format!("SELECT {COLS} FROM memory_records r WHERE r.principal = ? AND r.scope = ? AND r.subject = ? AND r.status = 'active' AND r.chain <> ?"),
                        vec![principal.clone().into(), scope_key.clone().into(), subj.clone().into(), chain.clone().into()],
                    )
                    .map_err(db_err)?;
                    for other in current.iter().filter(|r| cats.contains(&r.category)) {
                        rep.conflicts_resolved += 1;
                        let keep_other = (other.category == Category::Declared && out.category == Category::Inference)
                            || (!(out.category == Category::Declared && other.category == Category::Inference)
                                && other.updated_ms >= out.updated_ms);
                        if keep_other {
                            out.status = Status::Superseded;
                        } else {
                            tx.execute(
                                "UPDATE memory_records SET status = 'superseded', updated_ms = ?2 WHERE id = ?1",
                                params![other.id, now],
                            )
                            .map_err(db_err)?;
                        }
                    }
                }
            }
            insert(tx, &principal, &out, &oh, &ch).map_err(db_err)?;
            idmap.insert(rec.id.clone(), out.id.clone());
            chainmap.entry(rec.chain.clone()).or_insert(chain.clone());
            touched.insert(chain);
            rep.imported += 1;
        }
        // One active version per chain: the highest.
        for chain in &touched {
            tx.execute(
                "UPDATE memory_records SET status = 'superseded'
                 WHERE chain = ?1 AND status = 'active'
                   AND version < (SELECT MAX(version) FROM memory_records WHERE chain = ?1 AND status = 'active')",
                [chain],
            )
            .map_err(db_err)?;
        }
        rebuild_index(tx).map_err(db_err)?;
        Ok(rep)
    })?;
    compact(db);
    Ok(report)
}

/// The app's own copies of the database (`assist.db.bak-*`), taken before
/// migrations. They can hold forgotten text until they rotate out.
pub fn backup_files(db: &AssistDb) -> Vec<PathBuf> {
    let Some(path) = db.path() else {
        return Vec::new();
    };
    let (Some(dir), Some(name)) = (path.parent(), path.file_name()) else {
        return Vec::new();
    };
    let prefix = format!("{}.bak-", name.to_string_lossy());
    let mut out: Vec<PathBuf> = std::fs::read_dir(dir)
        .map(|rd| {
            rd.filter_map(Result::ok)
                .map(|e| e.path())
                .filter(|p| {
                    p.file_name()
                        .map(|n| n.to_string_lossy().starts_with(&prefix))
                        .unwrap_or(false)
                })
                .collect()
        })
        .unwrap_or_default();
    out.sort();
    out
}

/// Deletes those backups. Returns how many files were removed.
pub fn delete_backups(db: &AssistDb) -> Result<usize, String> {
    let mut n = 0;
    for p in backup_files(db) {
        std::fs::remove_file(&p).map_err(|e| {
            format!(
                "{}: removing a backup: {e}",
                super::super::db::ERR_ASSIST_DB
            )
        })?;
        n += 1;
    }
    Ok(n)
}

//! The derived memory graph: when a fact happened, its vector, the entities it
//! mentions and its links to other facts (Hindsight's "memory units" and
//! "links", on the same SQLite file as the records).
//!
//! Everything here hangs off `memory_records.id` and is derived: forgetting a
//! record calls [`purge`], which drops its vector, entity refs, links, the
//! entity notes that quoted it and any entity left without a record, so no
//! forgotten text survives in a summary.

use std::collections::{HashMap, HashSet};

use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

/// Semantic links: at most this many neighbours per new fact…
pub const SEMANTIC_LINKS: usize = 5;
/// …and only above this cosine.
pub const SEMANTIC_LINK_MIN: f32 = 0.72;
/// Two entity names closer than this (bigram Dice) are the same entity.
pub const ENTITY_MERGE_MIN: f64 = 0.86;

/// An entity as extraction or the model names it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EntityRef {
    pub name: String,
    #[serde(default = "default_kind")]
    pub kind: String,
}

fn default_kind() -> String {
    "thing".into()
}

/// One entity of the principal, for the UI and for query matching.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Entity {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub mentions: i64,
    pub last_ms: i64,
    #[serde(default)]
    pub note: Option<String>,
}

// ── Text helpers ─────────────────────────────────────────────────────────

pub(crate) fn fold(s: &str) -> String {
    s.chars()
        .flat_map(char::to_lowercase)
        .map(|c| match c {
            'á' | 'à' | 'â' | 'ã' | 'ä' => 'a',
            'é' | 'è' | 'ê' | 'ë' => 'e',
            'í' | 'ì' | 'î' | 'ï' => 'i',
            'ó' | 'ò' | 'ô' | 'õ' | 'ö' => 'o',
            'ú' | 'ù' | 'û' | 'ü' => 'u',
            'ç' => 'c',
            'ñ' => 'n',
            other => other,
        })
        .collect()
}

const ARTICLES: &[&str] = &[
    "o", "a", "os", "as", "um", "uma", "the", "an", "el", "la", "los", "las", "le", "les", "meu",
    "minha", "my",
];

/// The key two spellings of one entity share: folded, punctuation gone,
/// leading articles dropped ("a Maria" = "Maria" = "maria").
pub fn entity_norm(name: &str) -> String {
    let folded = fold(name);
    let words: Vec<&str> = folded
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .collect();
    let start = words
        .iter()
        .position(|w| !ARTICLES.contains(w))
        .unwrap_or(words.len());
    words[start..].join(" ")
}

fn bigrams(s: &str) -> Vec<(char, char)> {
    let chars: Vec<char> = s.chars().filter(|c| !c.is_whitespace()).collect();
    chars.windows(2).map(|w| (w[0], w[1])).collect()
}

/// Sørensen–Dice over character bigrams, 0..1.
pub fn dice(a: &str, b: &str) -> f64 {
    let (x, y) = (bigrams(a), bigrams(b));
    if x.is_empty() || y.is_empty() {
        return if a == b { 1.0 } else { 0.0 };
    }
    let mut pool: HashMap<(char, char), usize> = HashMap::new();
    for g in &y {
        *pool.entry(*g).or_default() += 1;
    }
    let mut shared = 0usize;
    for g in &x {
        if let Some(n) = pool.get_mut(g) {
            if *n > 0 {
                *n -= 1;
                shared += 1;
            }
        }
    }
    2.0 * shared as f64 / (x.len() + y.len()) as f64
}

pub(crate) fn vec_to_blob(v: &[f32]) -> Vec<u8> {
    let mut out = Vec::with_capacity(v.len() * 4);
    for x in v {
        out.extend_from_slice(&x.to_le_bytes());
    }
    out
}

pub(crate) fn blob_to_vec(b: &[u8]) -> Vec<f32> {
    b.chunks_exact(4)
        .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect()
}

pub(crate) fn dot(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() {
        return 0.0;
    }
    a.iter().zip(b).map(|(x, y)| x * y).sum()
}

// ── Writes ───────────────────────────────────────────────────────────────

/// Finds or creates the entity `name` for `principal`. Exact key first, then
/// a near spelling of the same kind ("Joao Silva" / "João da Silva").
pub(crate) fn upsert_entity(
    conn: &Connection,
    principal: &str,
    e: &EntityRef,
    now: i64,
) -> rusqlite::Result<Option<String>> {
    let norm = entity_norm(&e.name);
    if norm.chars().count() < 2 || norm.chars().count() > 80 {
        return Ok(None);
    }
    let kind = e.kind.trim().to_lowercase();
    let kind = if kind.is_empty() {
        default_kind()
    } else {
        kind
    };
    let exact: Option<String> = conn
        .query_row(
            "SELECT id FROM memory_entities WHERE principal = ?1 AND norm = ?2",
            params![principal, norm],
            |r| r.get(0),
        )
        .optional()?;
    let id = match exact {
        Some(id) => Some(id),
        None => {
            let mut stmt = conn.prepare(
                "SELECT id, norm FROM memory_entities WHERE principal = ?1 AND kind = ?2",
            )?;
            let rows = stmt.query_map(params![principal, kind], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
            })?;
            let mut best: Option<(f64, String)> = None;
            for row in rows {
                let (id, other) = row?;
                // Both names must share their first letter: "Ana" never
                // swallows "Hana".
                if other.chars().next() != norm.chars().next() {
                    continue;
                }
                let s = dice(&norm, &other);
                if s >= ENTITY_MERGE_MIN && best.as_ref().map(|b| s > b.0).unwrap_or(true) {
                    best = Some((s, id));
                }
            }
            best.map(|b| b.1)
        }
    };
    match id {
        Some(id) => {
            conn.execute(
                "UPDATE memory_entities SET mentions = mentions + 1, last_ms = ?2 WHERE id = ?1",
                params![id, now],
            )?;
            Ok(Some(id))
        }
        None => {
            let id = super::super::new_id();
            conn.execute(
                "INSERT INTO memory_entities(id, principal, name, norm, kind, mentions, first_ms, last_ms)
                 VALUES (?1, ?2, ?3, ?4, ?5, 1, ?6, ?6)",
                params![id, principal, e.name.trim(), norm, kind, now],
            )?;
            Ok(Some(id))
        }
    }
}

/// When the fact happened (ms range) and how many times it was confirmed.
pub(crate) fn set_meta(
    conn: &Connection,
    record_id: &str,
    occurred: (Option<i64>, Option<i64>),
) -> rusqlite::Result<()> {
    let (start, end) = occurred;
    let end = match (start, end) {
        (Some(s), Some(e)) if e < s => Some(s),
        _ => end,
    };
    conn.execute(
        "INSERT INTO memory_meta(record_id, occurred_start, occurred_end) VALUES (?1, ?2, ?3)
         ON CONFLICT(record_id) DO UPDATE SET
           occurred_start = COALESCE(excluded.occurred_start, occurred_start),
           occurred_end = COALESCE(excluded.occurred_end, occurred_end)",
        params![record_id, start, end.or(start)],
    )?;
    Ok(())
}

/// The same fact arrived again: one more proof.
pub(crate) fn reinforce(conn: &Connection, record_id: &str) -> rusqlite::Result<i64> {
    conn.execute(
        "INSERT INTO memory_meta(record_id, proofs) VALUES (?1, 2)
         ON CONFLICT(record_id) DO UPDATE SET proofs = proofs + 1",
        [record_id],
    )?;
    conn.query_row(
        "SELECT proofs FROM memory_meta WHERE record_id = ?1",
        [record_id],
        |r| r.get(0),
    )
}

/// Recall used these facts in a turn.
pub(crate) fn touch(conn: &Connection, ids: &[String], now: i64) -> rusqlite::Result<()> {
    let mut stmt = conn.prepare(
        "INSERT INTO memory_meta(record_id, hits, last_hit_ms) VALUES (?1, 1, ?2)
         ON CONFLICT(record_id) DO UPDATE SET hits = hits + 1, last_hit_ms = ?2",
    )?;
    for id in ids {
        stmt.execute(params![id, now])?;
    }
    Ok(())
}

pub(crate) fn attach_entities(
    conn: &Connection,
    principal: &str,
    record_id: &str,
    scope: &str,
    entities: &[EntityRef],
    now: i64,
) -> rusqlite::Result<Vec<String>> {
    let mut ids = Vec::new();
    for e in entities.iter().take(12) {
        if let Some(id) = upsert_entity(conn, principal, e, now)? {
            if ids.contains(&id) {
                continue;
            }
            conn.execute(
                "INSERT OR IGNORE INTO memory_entity_refs(record_id, entity_id) VALUES (?1, ?2)",
                params![record_id, id],
            )?;
            conn.execute(
                "INSERT OR IGNORE INTO memory_dirty_entities(entity_id, scope) VALUES (?1, ?2)",
                params![id, scope],
            )?;
            ids.push(id);
        }
    }
    Ok(ids)
}

pub(crate) fn store_vector(
    conn: &Connection,
    record_id: &str,
    source: &str,
    v: &[f32],
) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT OR REPLACE INTO memory_vectors(record_id, source, dim, vec) VALUES (?1, ?2, ?3, ?4)",
        params![record_id, source, v.len() as i64, vec_to_blob(v)],
    )?;
    Ok(())
}

pub(crate) fn add_link(
    conn: &Connection,
    src: &str,
    dst: &str,
    kind: &str,
    weight: f32,
) -> rusqlite::Result<()> {
    if src == dst {
        return Ok(());
    }
    conn.execute(
        "INSERT INTO memory_links(src, dst, kind, weight) VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT(src, dst, kind) DO UPDATE SET weight = MAX(weight, excluded.weight)",
        params![src, dst, kind, weight as f64],
    )?;
    Ok(())
}

/// Active records of `principal` with a vector from `source`, as
/// `(id, scope, vector)`.
pub(crate) fn active_vectors(
    conn: &Connection,
    principal: &str,
    source: &str,
) -> rusqlite::Result<Vec<(String, String, Vec<f32>)>> {
    let mut stmt = conn.prepare(
        "SELECT r.id, r.scope, v.vec FROM memory_vectors v JOIN memory_records r ON r.id = v.record_id
         WHERE r.principal = ?1 AND r.status = 'active' AND v.source = ?2",
    )?;
    let rows = stmt.query_map(params![principal, source], |r| {
        let b: Vec<u8> = r.get(2)?;
        Ok((r.get(0)?, r.get(1)?, blob_to_vec(&b)))
    })?;
    rows.collect()
}

/// Links a new fact to its nearest active neighbours of the same principal.
pub(crate) fn link_semantic(
    conn: &Connection,
    principal: &str,
    record_id: &str,
    source: &str,
    v: &[f32],
) -> rusqlite::Result<usize> {
    let mut near: Vec<(f32, String)> = active_vectors(conn, principal, source)?
        .into_iter()
        .filter(|(id, _, _)| id != record_id)
        .map(|(id, _, w)| (dot(v, &w), id))
        .filter(|(s, _)| *s >= super::retrieve::cutoffs(source).link_min)
        .collect();
    near.sort_by(|a, b| b.0.total_cmp(&a.0));
    near.truncate(SEMANTIC_LINKS);
    for (s, id) in &near {
        add_link(conn, record_id, id, "semantic", *s)?;
        add_link(conn, id, record_id, "semantic", *s)?;
    }
    Ok(near.len())
}

/// Entities whose name appears in `text` (whole words), longest first.
pub(crate) fn entities_in_text(
    conn: &Connection,
    principal: &str,
    text: &str,
) -> rusqlite::Result<Vec<String>> {
    let hay = format!(" {} ", entity_norm(text));
    let mut stmt = conn.prepare("SELECT id, norm FROM memory_entities WHERE principal = ?1")?;
    let rows = stmt.query_map([principal], |r| {
        Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
    })?;
    let mut found: Vec<(usize, String)> = Vec::new();
    for row in rows {
        let (id, norm) = row?;
        if norm.chars().count() >= 3 && hay.contains(&format!(" {norm} ")) {
            found.push((norm.len(), id));
        }
    }
    found.sort_by_key(|f| std::cmp::Reverse(f.0));
    Ok(found.into_iter().map(|f| f.1).take(8).collect())
}

// ── Forgetting ───────────────────────────────────────────────────────────

/// Drops everything derived from `ids`: meta, vectors, refs, links, notes that
/// quoted them, and entities no record mentions any more.
pub(crate) fn purge(conn: &Connection, ids: &[String]) -> rusqlite::Result<()> {
    if ids.is_empty() {
        return Ok(());
    }
    let set: HashSet<&str> = ids.iter().map(String::as_str).collect();
    let mut touched: HashSet<String> = HashSet::new();
    for id in ids {
        {
            let mut s =
                conn.prepare("SELECT entity_id FROM memory_entity_refs WHERE record_id = ?1")?;
            let rows = s.query_map([id], |r| r.get::<_, String>(0))?;
            for r in rows {
                touched.insert(r?);
            }
        }
        conn.execute("DELETE FROM memory_meta WHERE record_id = ?1", [id])?;
        conn.execute("DELETE FROM memory_vectors WHERE record_id = ?1", [id])?;
        conn.execute("DELETE FROM memory_entity_refs WHERE record_id = ?1", [id])?;
        conn.execute("DELETE FROM memory_links WHERE src = ?1 OR dst = ?1", [id])?;
    }
    // A note that quoted a forgotten fact goes whole; it is rebuilt from what
    // is left on the next consolidation.
    let notes: Vec<(String, String, String)> = {
        let mut s = conn.prepare("SELECT entity_id, scope, ids FROM memory_entity_notes")?;
        let rows = s.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?;
        rows.collect::<rusqlite::Result<_>>()?
    };
    for (entity, scope, quoted) in notes {
        let quoted: Vec<String> = serde_json::from_str(&quoted).unwrap_or_default();
        if quoted.iter().any(|q| set.contains(q.as_str())) {
            conn.execute(
                "DELETE FROM memory_entity_notes WHERE entity_id = ?1 AND scope = ?2",
                params![entity, scope],
            )?;
            conn.execute(
                "INSERT OR IGNORE INTO memory_dirty_entities(entity_id, scope) VALUES (?1, ?2)",
                params![entity, scope],
            )?;
        }
    }
    for e in touched {
        let left: i64 = conn.query_row(
            "SELECT COUNT(*) FROM memory_entity_refs WHERE entity_id = ?1",
            [&e],
            |r| r.get(0),
        )?;
        if left == 0 {
            conn.execute("DELETE FROM memory_entities WHERE id = ?1", [&e])?;
            conn.execute("DELETE FROM memory_entity_notes WHERE entity_id = ?1", [&e])?;
            conn.execute(
                "DELETE FROM memory_dirty_entities WHERE entity_id = ?1",
                [&e],
            )?;
        }
    }
    Ok(())
}

/// Derived rows whose record is gone (a crash between writes, an old import).
pub(crate) fn sweep_orphans(conn: &Connection) -> rusqlite::Result<()> {
    let ids: Vec<String> = {
        let mut s = conn.prepare(
            "SELECT record_id FROM memory_vectors WHERE record_id NOT IN (SELECT id FROM memory_records)
             UNION SELECT record_id FROM memory_meta WHERE record_id NOT IN (SELECT id FROM memory_records)
             UNION SELECT record_id FROM memory_entity_refs WHERE record_id NOT IN (SELECT id FROM memory_records)",
        )?;
        let rows = s.query_map([], |r| r.get(0))?;
        rows.collect::<rusqlite::Result<_>>()?
    };
    purge(conn, &ids)?;
    conn.execute(
        "DELETE FROM memory_links WHERE src NOT IN (SELECT id FROM memory_records)
            OR dst NOT IN (SELECT id FROM memory_records)",
        [],
    )?;
    Ok(())
}

// ── Reads for the UI ─────────────────────────────────────────────────────

/// Entities of `principal` mentioned by at least one record the caller can
/// read, most mentioned first, with the entity note of the first readable
/// scope that has one.
pub(crate) fn list_entities(
    conn: &Connection,
    principal: &str,
    readable: Option<&[String]>,
    limit: usize,
) -> rusqlite::Result<Vec<Entity>> {
    let (filter, scopes): (String, Vec<String>) = match readable {
        None => ("1=1".into(), Vec::new()),
        Some([]) => ("0=1".into(), Vec::new()),
        Some(keys) => (
            format!("r.scope IN ({})", vec!["?"; keys.len()].join(",")),
            keys.to_vec(),
        ),
    };
    let sql = format!(
        "SELECT e.id, e.name, e.kind, COUNT(DISTINCT r.id) AS n, e.last_ms
         FROM memory_entities e
         JOIN memory_entity_refs x ON x.entity_id = e.id
         JOIN memory_records r ON r.id = x.record_id AND r.status = 'active'
         WHERE e.principal = ? AND {filter}
         GROUP BY e.id ORDER BY n DESC, e.last_ms DESC LIMIT ?"
    );
    let mut p: Vec<rusqlite::types::Value> = vec![principal.to_string().into()];
    p.extend(scopes.iter().cloned().map(rusqlite::types::Value::Text));
    p.push((limit as i64).into());
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(rusqlite::params_from_iter(p), |r| {
        Ok(Entity {
            id: r.get(0)?,
            name: r.get(1)?,
            kind: r.get(2)?,
            mentions: r.get(3)?,
            last_ms: r.get(4)?,
            note: None,
        })
    })?;
    let mut out: Vec<Entity> = rows.collect::<rusqlite::Result<_>>()?;
    for e in &mut out {
        let mut s =
            conn.prepare("SELECT scope, text FROM memory_entity_notes WHERE entity_id = ?1")?;
        let rows = s.query_map([&e.id], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
        })?;
        for row in rows {
            let (scope, text) = row?;
            if readable.map(|k| k.contains(&scope)).unwrap_or(true) {
                e.note = Some(text);
                break;
            }
        }
    }
    Ok(out)
}

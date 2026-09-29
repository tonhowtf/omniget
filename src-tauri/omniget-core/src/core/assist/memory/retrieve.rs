//! Hybrid recall, after Hindsight's four-way retrieval (TEMPR): keyword
//! (FTS5 bm25), meaning (local vectors), graph (entities named in the query,
//! links and shared entities of the best seeds) and time (the window the
//! query names). Each arm ranks on its own, Reciprocal Rank Fusion (k = 60)
//! merges them, and a light rerank folds in recency, temporal proximity, how
//! many times a fact was confirmed and the confidence of guesses.
//!
//! Differences from Hindsight, on purpose:
//! - no server, no Postgres: one SQLite file and a brute-force dot product
//!   over 384-d vectors (a personal memory holds thousands of facts, not
//!   millions; 10k facts score in a few ms);
//! - the scope filter runs in SQL on every arm and again on the final fetch,
//!   so a graph walk can never surface a record the caller may not read;
//! - every hit carries `why` (which arms found it and with what score), so
//!   the UI and the model can see why a memory came back.

use std::collections::{HashMap, HashSet};

use rusqlite::types::Value as SqlValue;
use serde::{Deserialize, Serialize};

use super::super::ctx::AssistCtx;
use super::super::db::AssistDb;
use super::graph::{self, dot};
use super::records::{expire_due, query_records, scope_filter, COLS};
use super::search::fts_query;
use super::temporal::{self, Window};
use super::{Category, MemoryRecord};

/// RRF constant (Hindsight and the original paper use 60).
pub const RRF_K: f32 = 60.0;
/// Each arm contributes at most this many candidates before fusion.
pub const ARM_CAP: usize = 50;
/// Cosine below this is noise for MiniLM.
pub const SEMANTIC_MIN: f32 = 0.22;
/// Seeds for the graph walk need at least this cosine.
pub const SEED_MIN: f32 = 0.40;
/// Recency half-life: a fact this old is neutral, younger ones gain.
pub const RECENCY_HALF_LIFE_DAYS: f32 = 90.0;
/// Cosine cut-offs per vector source. Measured on the benchmark's facts
/// (`bench::similarity_calibration`): MiniLM spreads unrelated facts around
/// 0.31 (max 0.82), e5 packs them around 0.81 (max 0.94) with paraphrases at
/// 0.92–0.99, so one set of numbers cannot serve both.
#[derive(Debug, Clone, Copy)]
pub struct Cutoffs {
    /// Below this a vector hit is noise. Kept low: the arm is a wide net for
    /// fusion and the reranker, which do the ordering.
    pub semantic_min: f32,
    /// Seeds of the graph walk: at least this…
    pub seed_min: f32,
    /// …and within this much of the query's best match.
    pub seed_spread: f32,
    /// Semantic links between facts.
    pub link_min: f32,
    /// Two facts this close are the same fact.
    pub same_fact: f32,
}

pub fn cutoffs(source: &str) -> Cutoffs {
    match source {
        "e5" => Cutoffs {
            semantic_min: 0.70,
            seed_min: 0.74,
            seed_spread: 0.06,
            link_min: 0.88,
            same_fact: 0.95,
        },
        _ => Cutoffs {
            semantic_min: SEMANTIC_MIN,
            seed_min: SEED_MIN,
            seed_spread: 1.0,
            link_min: super::graph::SEMANTIC_LINK_MIN,
            same_fact: 0.92,
        },
    }
}

/// Switches the benchmark flips to compare configurations in one process.
pub(crate) mod bench_knobs {
    use std::sync::atomic::AtomicBool;
    /// Use the English MiniLM even when e5 is on disk.
    pub static MINILM: AtomicBool = AtomicBool::new(false);
    /// Skip the cross-encoder.
    pub static NO_RERANK: AtomicBool = AtomicBool::new(false);
}

/// Candidates the cross-encoder reads per query.
pub const RERANK_POOL: usize = 30;

/// The vector source memory uses: the multilingual e5 when it is on disk,
/// else the English MiniLM, else none (keyword, graph and time still work).
/// Always a local model, so memory works offline; vectors of another source
/// are ignored until [`super::retain::backfill_vectors`] redoes them.
pub fn vector_source() -> Option<&'static str> {
    use crate::core::embed::xlmr::{is_ready, Model};
    if is_ready(Model::E5) && !bench_knobs::MINILM.load(std::sync::atomic::Ordering::Relaxed) {
        Some("e5")
    } else if crate::core::embed::is_ready() {
        Some("onnx")
    } else {
        None
    }
}

/// Vectors of stored texts in the current source. **Blocking.**
pub fn embed_passages(texts: &[&str]) -> Option<(&'static str, Vec<Vec<f32>>)> {
    let src = vector_source()?;
    let vecs = match src {
        "e5" => crate::core::embed::xlmr::embed(texts, false).ok()?,
        _ => crate::core::embed::embed_batch(texts)
            .ok()?
            .into_iter()
            .map(|d| d.to_vec())
            .collect(),
    };
    Some((src, vecs))
}

/// Why a memory came back.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Why {
    /// Arms that found it: `keyword`, `semantic`, `graph`, `temporal`.
    pub arms: Vec<String>,
    #[serde(default)]
    pub semantic: Option<f32>,
    #[serde(default)]
    pub keyword_rank: Option<usize>,
    #[serde(default)]
    pub graph: Option<f32>,
    #[serde(default)]
    pub temporal: Option<f32>,
    pub rrf: f32,
    /// Cross-encoder relevance, 0..1, when the reranker ran.
    #[serde(default)]
    pub rerank: Option<f32>,
    /// How many times the fact was stated again.
    pub proofs: i64,
    #[serde(default)]
    pub occurred_start: Option<i64>,
    #[serde(default)]
    pub occurred_end: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Hit {
    pub record: MemoryRecord,
    pub score: f32,
    pub why: Why,
}

/// What the query looked like to recall.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Probe {
    pub hits: Vec<Hit>,
    /// Entities named in the query.
    pub entities: Vec<String>,
    #[serde(default)]
    pub window: Option<(i64, i64)>,
    /// False when the local model was not ready and meaning was skipped.
    pub semantic: bool,
}

/// The query vector, when the local model is loaded. Never downloads.
pub fn embed_query(text: &str) -> Option<Vec<f32>> {
    let t: String = text.chars().take(2000).collect();
    match vector_source()? {
        "e5" => crate::core::embed::xlmr::embed(&[t.as_str()], true)
            .ok()
            .and_then(|v| v.into_iter().next()),
        _ => crate::core::embed::embed_batch(&[t.as_str()])
            .ok()
            .and_then(|v| v.into_iter().next())
            .map(|d| d.to_vec()),
    }
}

fn rank_ids(mut scored: Vec<(String, f32)>) -> Vec<(String, f32)> {
    scored.sort_by(|a, b| b.1.total_cmp(&a.1));
    scored.truncate(ARM_CAP);
    scored
}

struct Meta {
    start: Option<i64>,
    end: Option<i64>,
    proofs: i64,
}

/// Hybrid recall over what `ctx` may read. `qvec` is the query vector (pass
/// [`embed_query`]'s result); without it the meaning arm is skipped and the
/// graph walk seeds from keywords and named entities only.
pub fn hybrid(
    db: &AssistDb,
    ctx: &AssistCtx,
    query: &str,
    qvec: Option<&[f32]>,
    limit: usize,
    now: i64,
) -> Result<Probe, String> {
    expire_due(db, now)?;
    let limit = limit.clamp(1, 100);
    let window = temporal::parse(query, now);
    let principal = ctx.principal.clone();
    let readable: Option<HashSet<String>> =
        (!ctx.is_user_ui()).then(|| ctx.readable_keys().into_iter().collect());
    let can = |scope: &str| readable.as_ref().map(|r| r.contains(scope)).unwrap_or(true);

    // Arm 1 — keyword.
    let keyword: Vec<(String, f32)> = match fts_query(query) {
        Some(q) => {
            let (ff, fp) = scope_filter(ctx, "f.scope");
            let mut p: Vec<SqlValue> = vec![q.into(), principal.clone().into()];
            p.extend(fp);
            p.push((ARM_CAP as i64).into());
            db.with(|c| {
                let mut s = c.prepare(&format!(
                    "SELECT f.record_id, bm25(memory_fts, 1.0, 0.5) FROM memory_fts f
                     JOIN memory_records r ON r.id = f.record_id
                     WHERE memory_fts MATCH ? AND r.principal = ? AND r.status = 'active' AND {ff}
                     ORDER BY bm25(memory_fts, 1.0, 0.5) LIMIT ?"
                ))?;
                let rows = s.query_map(rusqlite::params_from_iter(p), |r| {
                    Ok((r.get::<_, String>(0)?, -(r.get::<_, f64>(1)? as f32)))
                })?;
                rows.collect::<rusqlite::Result<Vec<_>>>()
            })?
        }
        None => Vec::new(),
    };

    // Arm 2 — meaning.
    let mut sims: HashMap<String, f32> = HashMap::new();
    if let (Some(q), Some(src)) = (qvec, vector_source()) {
        let all = db.with(|c| graph::active_vectors(c, &principal, src))?;
        for (id, scope, v) in all {
            if can(&scope) {
                sims.insert(id, dot(q, &v));
            }
        }
    }
    let cut = cutoffs(vector_source().unwrap_or("onnx"));
    let semantic = rank_ids(
        sims.iter()
            .filter(|(_, s)| **s >= cut.semantic_min)
            .map(|(id, s)| (id.clone(), *s))
            .collect(),
    );

    // Arm 3 — graph: spreading activation over facts, their entities and
    // their links, two hops deep (Hindsight stops at one, which is why
    // multi-hop is its weakest category).
    let named = db.with(|c| graph::entities_in_text(c, &principal, query))?;
    let mut seeds: Vec<(String, f32)> = semantic
        .iter()
        .filter(|(_, s)| {
            *s >= cut.seed_min
                && *s >= semantic.first().map(|b| b.1).unwrap_or(0.0) - cut.seed_spread
        })
        .take(10)
        .map(|(id, s)| (id.clone(), *s))
        .collect();
    for (id, _) in keyword.iter().take(5) {
        if !seeds.iter().any(|(s, _)| s == id) {
            seeds.push((id.clone(), 0.5));
        }
    }
    let graph_scores: HashMap<String, f32> = db.with(|c| {
        let mut score: HashMap<String, f32> = HashMap::new();
        // Records naming the query's entities: tanh(shared × 0.5), like
        // Hindsight's entity expansion (1 → 0.46, 2 → 0.76, 3 → 0.91).
        let mut shared: HashMap<String, f32> = HashMap::new();
        {
            let mut s = c.prepare(
                "SELECT x.record_id FROM memory_entity_refs x WHERE x.entity_id = ?1 LIMIT 200",
            )?;
            for e in &named {
                let rows = s.query_map([e], |r| r.get::<_, String>(0))?;
                for r in rows {
                    *shared.entry(r?).or_default() += 1.0;
                }
            }
        }
        for (id, n) in shared {
            *score.entry(id).or_default() += (n * 0.5).tanh();
        }
        let mut links = c.prepare(
            "SELECT dst, kind, weight FROM memory_links WHERE src = ?1 ORDER BY weight DESC LIMIT 20",
        )?;
        let mut via = c.prepare(
            "SELECT y.record_id, COUNT(*) FROM memory_entity_refs x
             JOIN memory_entity_refs y ON y.entity_id = x.entity_id AND y.record_id != x.record_id
             WHERE x.record_id = ?1 GROUP BY y.record_id ORDER BY COUNT(*) DESC LIMIT 40",
        )?;
        let mut spread = |from: &[(String, f32)],
                          score: &mut HashMap<String, f32>|
         -> rusqlite::Result<HashMap<String, f32>> {
            let mut reached: HashMap<String, f32> = HashMap::new();
            for (seed, w) in from {
                let rows = links.query_map([seed], |r| {
                    Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, f64>(2)? as f32))
                })?;
                for row in rows {
                    let (dst, kind, lw) = row?;
                    let k = if kind == "causal" { 1.0 } else { 0.8 };
                    *reached.entry(dst).or_default() += w * lw * k;
                }
                let rows =
                    via.query_map([seed], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))?;
                for row in rows {
                    let (dst, n) = row?;
                    *reached.entry(dst).or_default() += w * (n as f32 * 0.5).tanh() * 0.7;
                }
            }
            for (id, v) in &reached {
                *score.entry(id.clone()).or_default() += v;
            }
            Ok(reached)
        };
        let hop1 = spread(&seeds, &mut score)?;
        // Second hop from the best facts the first one reached (not the
        // seeds), at half strength.
        let mut next: Vec<(String, f32)> = hop1
            .into_iter()
            .filter(|(id, _)| !seeds.iter().any(|(s, _)| s == id))
            .collect();
        next.sort_by(|a, b| b.1.total_cmp(&a.1));
        next.truncate(8);
        let top = next.first().map(|n| n.1).unwrap_or(1.0).max(1e-6);
        let next: Vec<(String, f32)> = next
            .into_iter()
            .map(|(id, v)| (id, 0.5 * v / top))
            .collect();
        spread(&next, &mut score)?;
        Ok(score)
    })?;
    let graph_arm = rank_ids(graph_scores.iter().map(|(k, v)| (k.clone(), *v)).collect());

    // Arm 4 — time.
    let temporal_arm: Vec<(String, f32)> = match window {
        Some(win) => {
            let (rf, rp) = scope_filter(ctx, "r.scope");
            let mut p: Vec<SqlValue> = vec![principal.clone().into()];
            p.extend(rp);
            let pad = (win.end - win.start).max(7 * 86_400_000);
            p.push((win.start - pad).into());
            p.push((win.end + pad).into());
            let rows: Vec<(String, i64, i64)> = db.with(|c| {
                let mut s = c.prepare(&format!(
                    "SELECT r.id, COALESCE(m.occurred_start, r.source_ms), COALESCE(m.occurred_end, m.occurred_start, r.source_ms)
                     FROM memory_records r LEFT JOIN memory_meta m ON m.record_id = r.id
                     WHERE r.principal = ? AND r.status = 'active' AND {rf}
                       AND COALESCE(m.occurred_end, m.occurred_start, r.source_ms) >= ?
                       AND COALESCE(m.occurred_start, r.source_ms) < ?
                     LIMIT 500"
                ))?;
                let rows = s.query_map(rusqlite::params_from_iter(p), |r| {
                    Ok((r.get(0)?, r.get(1)?, r.get(2)?))
                })?;
                rows.collect::<rusqlite::Result<Vec<_>>>()
            })?;
            rank_ids(
                rows.into_iter()
                    .map(|(id, s, e)| {
                        let prox = temporal::proximity(&win, s, e);
                        // Among facts of the same period, the one that also
                        // means something close to the query wins.
                        let sem = sims.get(&id).copied().unwrap_or(0.0).max(0.0);
                        (id, prox + 0.5 * sem)
                    })
                    .filter(|(_, s)| *s > 0.0)
                    .collect(),
            )
        }
        None => Vec::new(),
    };

    // Fusion.
    let arms: [(&str, &Vec<(String, f32)>); 4] = [
        ("semantic", &semantic),
        ("keyword", &keyword),
        ("graph", &graph_arm),
        ("temporal", &temporal_arm),
    ];
    let mut fused: HashMap<String, Why> = HashMap::new();
    for (name, list) in arms {
        for (rank, (id, s)) in list.iter().enumerate() {
            let w = fused.entry(id.clone()).or_default();
            w.rrf += 1.0 / (RRF_K + rank as f32 + 1.0);
            w.arms.push(name.to_string());
            match name {
                "keyword" => w.keyword_rank = Some(rank + 1),
                "graph" => w.graph = Some(*s),
                "temporal" => w.temporal = Some(*s),
                _ => {}
            }
        }
    }
    for (id, w) in fused.iter_mut() {
        w.semantic = sims.get(id).copied();
    }
    let mut pool: Vec<(String, Why)> = fused.into_iter().collect();
    pool.sort_by(|a, b| b.1.rrf.total_cmp(&a.1.rrf));
    // What the reranker reads: round-robin over the arms (meaning first), so
    // each arm's best hits get a slot. Cutting by fused score instead lets a
    // well-linked arm crowd out the one fact only meaning found (Hindsight
    // avoids it by reranking 300 candidates; this keeps it to 30).
    let mut shortlist: Vec<String> = Vec::new();
    let depth = arms.iter().map(|(_, l)| l.len()).max().unwrap_or(0);
    'fill: for i in 0..depth {
        for (_, list) in &arms {
            if let Some((id, _)) = list.get(i) {
                if !shortlist.contains(id) {
                    shortlist.push(id.clone());
                    if shortlist.len() >= RERANK_POOL {
                        break 'fill;
                    }
                }
            }
        }
    }
    let keep: HashSet<&String> = shortlist.iter().collect();
    let mut kept = 0usize;
    pool.retain(|(id, _)| {
        let rrf_top = kept < limit * 4;
        let ok = rrf_top || keep.contains(id);
        if ok {
            kept += 1;
        }
        ok
    });

    // Final fetch through the scope filter, then rerank.
    let ids: Vec<String> = pool.iter().map(|p| p.0.clone()).collect();
    let records = fetch(db, ctx, &ids, now)?;
    let metas = db.with(|c| {
        let mut out: HashMap<String, Meta> = HashMap::new();
        let mut s = c.prepare(
            "SELECT occurred_start, occurred_end, proofs FROM memory_meta WHERE record_id = ?1",
        )?;
        for id in &ids {
            use rusqlite::OptionalExtension;
            if let Some(m) = s
                .query_row([id], |r| {
                    Ok(Meta {
                        start: r.get(0)?,
                        end: r.get(1)?,
                        proofs: r.get(2)?,
                    })
                })
                .optional()?
            {
                out.insert(id.clone(), m);
            }
        }
        Ok(out)
    })?;
    let max_rrf = pool.first().map(|p| p.1.rrf).unwrap_or(1.0).max(1e-6);
    // The cross-encoder reads the query with each of the best candidates,
    // each prefixed with its date like Hindsight does, so "when" questions
    // see the date.
    let ce: HashMap<String, f32> =
        if crate::core::embed::xlmr::is_ready(crate::core::embed::xlmr::Model::Reranker)
            && !bench_knobs::NO_RERANK.load(std::sync::atomic::Ordering::Relaxed)
        {
            let top: Vec<(&String, String)> = shortlist
                .iter()
                .filter_map(|id| records.get(id).map(|r| (id, r)))
                .map(|(id, r)| {
                    let when = metas
                        .get(id)
                        .and_then(|m| m.start)
                        .unwrap_or(r.source.at_ms);
                    (
                        id,
                        format!("[Date: {}] {}", super::search::day(when), r.content),
                    )
                })
                .collect();
            let docs: Vec<&str> = top.iter().map(|t| t.1.as_str()).collect();
            match crate::core::embed::xlmr::rerank(query, &docs) {
                Ok(scores) => top
                    .iter()
                    .zip(scores)
                    .map(|((id, _), s)| ((*id).clone(), s))
                    .collect(),
                Err(_) => HashMap::new(),
            }
        } else {
            HashMap::new()
        };
    let mut hits: Vec<Hit> = Vec::new();
    for (id, mut why) in pool {
        let Some(rec) = records.get(&id) else {
            continue;
        };
        let meta = metas.get(&id);
        why.proofs = meta.map(|m| m.proofs).unwrap_or(1);
        why.occurred_start = meta.and_then(|m| m.start);
        why.occurred_end = meta.and_then(|m| m.end);
        why.rerank = ce.get(&id).copied();
        let score = rerank(rec, &why, window.as_ref(), max_rrf, now, !ce.is_empty());
        hits.push(Hit {
            record: rec.clone(),
            score,
            why,
        });
    }
    hits.sort_by(|a, b| b.score.total_cmp(&a.score));
    // One version per chain, one copy per text.
    let mut seen = HashSet::new();
    hits.retain(|h| seen.insert(super::normalize(&h.record.content)));
    hits.truncate(limit);
    let names = db.with(|c| {
        let mut s = c.prepare("SELECT name FROM memory_entities WHERE id = ?1")?;
        let mut out = Vec::new();
        for id in &named {
            if let Ok(n) = s.query_row([id], |r| r.get::<_, String>(0)) {
                out.push(n);
            }
        }
        Ok(out)
    })?;
    Ok(Probe {
        hits,
        entities: names,
        window: window.map(|w| (w.start, w.end)),
        semantic: qvec.is_some(),
    })
}

/// Hindsight multiplies a cross-encoder score by recency (±10 %), temporal
/// proximity (±10 %) and evidence (±5 %). Same here, over the multilingual
/// cross-encoder when it is on disk; without it, the base is the normalized
/// fusion score blended with the raw cosine.
fn rerank(
    rec: &MemoryRecord,
    why: &Why,
    window: Option<&Window>,
    max_rrf: f32,
    now: i64,
    reranked: bool,
) -> f32 {
    let fused = why.rrf / max_rrf;
    let base = if reranked {
        // Hindsight drops the fusion score once the cross-encoder ran; we
        // keep a little of it so a candidate the reranker did not read (past
        // the pool) or scored near zero still orders by fusion.
        0.85 * why.rerank.unwrap_or(0.0) + 0.15 * fused
    } else {
        0.7 * fused + 0.3 * why.semantic.unwrap_or(0.0).max(0.0)
    };
    let when = why
        .occurred_end
        .or(why.occurred_start)
        .unwrap_or(rec.source.at_ms);
    let age_days = ((now - when).max(0) as f32) / 86_400_000.0;
    let recency = 0.5f32.powf(age_days / RECENCY_HALF_LIFE_DAYS);
    let mut s = base * (1.0 + 0.2 * (recency - 0.5));
    if let Some(win) = window {
        let start = why.occurred_start.unwrap_or(rec.source.at_ms);
        let end = why.occurred_end.unwrap_or(start);
        let prox = temporal::proximity(win, start, end);
        let alpha = if win.soft { 0.2 } else { 0.6 };
        s *= 1.0 + alpha * (prox - 0.5);
    }
    let proofs = (why.proofs.max(1) as f32).ln().min(2.0) / 2.0;
    s *= 1.0 + 0.1 * proofs;
    if rec.category == Category::Inference {
        s *= 0.7 + 0.3 * rec.confidence as f32;
    }
    s
}

fn fetch(
    db: &AssistDb,
    ctx: &AssistCtx,
    ids: &[String],
    now: i64,
) -> Result<HashMap<String, MemoryRecord>, String> {
    if ids.is_empty() {
        return Ok(HashMap::new());
    }
    let (rf, rp) = scope_filter(ctx, "r.scope");
    let mut p: Vec<SqlValue> = vec![ctx.principal.clone().into(), now.into()];
    p.extend(rp);
    p.extend(ids.iter().cloned().map(SqlValue::Text));
    let marks = vec!["?"; ids.len()].join(",");
    let rows = db.with(|c| {
        query_records(
            c,
            &format!(
                "SELECT {COLS} FROM memory_records r
                 WHERE r.principal = ? AND r.status = 'active'
                   AND (r.valid_until IS NULL OR r.valid_until > ?) AND {rf} AND r.id IN ({marks})"
            ),
            p,
        )
    })?;
    Ok(rows.into_iter().map(|r| (r.id.clone(), r)).collect())
}

/// Entity notes (Hindsight's observations / mental models) for the entities
/// named in `query`, from the scopes `ctx` may read.
pub fn notes_for(
    db: &AssistDb,
    ctx: &AssistCtx,
    query: &str,
) -> Result<Vec<(String, String)>, String> {
    let named = db.with(|c| graph::entities_in_text(c, &ctx.principal, query))?;
    let readable: Option<HashSet<String>> =
        (!ctx.is_user_ui()).then(|| ctx.readable_keys().into_iter().collect());
    db.with(|c| {
        let mut s = c.prepare(
            "SELECT e.name, n.scope, n.text FROM memory_entity_notes n
             JOIN memory_entities e ON e.id = n.entity_id WHERE n.entity_id = ?1",
        )?;
        let mut out = Vec::new();
        for id in &named {
            let rows = s.query_map([id], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                ))
            })?;
            for row in rows {
                let (name, scope, text) = row?;
                if readable
                    .as_ref()
                    .map(|r| r.contains(&scope))
                    .unwrap_or(true)
                {
                    out.push((name, text));
                    break;
                }
            }
        }
        Ok(out)
    })
}

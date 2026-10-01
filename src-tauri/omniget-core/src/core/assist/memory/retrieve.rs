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

pub const RRF_K: f32 = 60.0;
pub const ARM_CAP: usize = 50;
pub const SEMANTIC_MIN: f32 = 0.22;
pub const SEED_MIN: f32 = 0.40;
pub const RECENCY_HALF_LIFE_DAYS: f32 = 90.0;
#[derive(Debug, Clone, Copy)]
pub struct Cutoffs {
    pub semantic_min: f32,
    pub seed_min: f32,
    pub seed_spread: f32,
    pub link_min: f32,
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

pub(crate) mod bench_knobs {
    use std::sync::atomic::AtomicBool;
    pub static MINILM: AtomicBool = AtomicBool::new(false);
    pub static NO_RERANK: AtomicBool = AtomicBool::new(false);
}

pub const RERANK_POOL: usize = 30;

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

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Why {
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
    #[serde(default)]
    pub rerank: Option<f32>,
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

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Probe {
    pub hits: Vec<Hit>,
    pub entities: Vec<String>,
    #[serde(default)]
    pub window: Option<(i64, i64)>,
    pub semantic: bool,
}

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
                        let sem = sims.get(&id).copied().unwrap_or(0.0).max(0.0);
                        (id, prox + 0.5 * sem)
                    })
                    .filter(|(_, s)| *s > 0.0)
                    .collect(),
            )
        }
        None => Vec::new(),
    };

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

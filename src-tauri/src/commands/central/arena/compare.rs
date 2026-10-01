use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::time::Duration;

use omniget_core::core::tools::pricing::ModelPrice;
use serde::Serialize;

use super::{gctx, EntrySpec, ScoreRow};

pub const TYPICAL_INPUT_TOKENS: u64 = 60_000;
pub const TYPICAL_OUTPUT_TOKENS: u64 = 4_000;

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct EntryEstimate {
    pub driver: String,
    pub model: Option<String>,
    pub usd: Option<f64>,
    pub source: String,
    pub runs: i64,
    pub avg_duration_ms: Option<f64>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ArenaEstimate {
    pub entries: Vec<EntryEstimate>,
    pub total_usd: Option<f64>,
    pub unknown: usize,
}

fn same_model(a: Option<&str>, b: Option<&str>) -> bool {
    let n = |m: Option<&str>| {
        m.map(str::trim)
            .filter(|m| !m.is_empty())
            .map(str::to_string)
    };
    n(a) == n(b)
}

pub fn estimate_from(
    specs: &[EntrySpec],
    board: &[ScoreRow],
    prices: &[Option<ModelPrice>],
) -> ArenaEstimate {
    let entries: Vec<EntryEstimate> = specs
        .iter()
        .enumerate()
        .map(|(i, s)| {
            let model = s.model.clone().filter(|m| !m.trim().is_empty());
            let hist = board.iter().find(|r| {
                r.driver == s.driver
                    && same_model(r.model.as_deref(), model.as_deref())
                    && r.runs > 0
                    && r.avg_cost_usd.is_some()
            });
            if let Some(r) = hist {
                return EntryEstimate {
                    driver: s.driver.clone(),
                    model,
                    usd: r.avg_cost_usd,
                    source: "history".into(),
                    runs: r.runs,
                    avg_duration_ms: r.avg_duration_ms,
                };
            }
            let priced = prices.get(i).cloned().flatten().and_then(|p| {
                omniget_core::core::tools::pricing::cost(
                    &p,
                    TYPICAL_INPUT_TOKENS,
                    TYPICAL_OUTPUT_TOKENS,
                )
            });
            EntryEstimate {
                driver: s.driver.clone(),
                model,
                usd: priced,
                source: if priced.is_some() { "price" } else { "unknown" }.into(),
                runs: 0,
                avg_duration_ms: None,
            }
        })
        .collect();
    let known: Vec<f64> = entries.iter().filter_map(|e| e.usd).collect();
    ArenaEstimate {
        unknown: entries.len() - known.len(),
        total_usd: (!known.is_empty()).then(|| known.iter().sum()),
        entries,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Touch {
    pub status: char,
    pub blob: String,
    pub additions: Option<i64>,
    pub deletions: Option<i64>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CompareCell {
    pub entry_id: String,
    pub status: String,
    pub additions: Option<i64>,
    pub deletions: Option<i64>,
    pub variant: usize,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CompareFile {
    pub path: String,
    pub divergent: bool,
    pub touched_by: usize,
    pub variants: usize,
    pub cells: Vec<CompareCell>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub struct ArenaCompare {
    pub arena_id: String,
    pub entries: Vec<String>,
    pub pending: Vec<String>,
    pub files: Vec<CompareFile>,
    pub divergent: usize,
    pub agreed: usize,
}

pub fn divergence(per_entry: &[(String, BTreeMap<String, Touch>)]) -> Vec<CompareFile> {
    let paths: BTreeSet<&String> = per_entry.iter().flat_map(|(_, m)| m.keys()).collect();
    let mut files: Vec<CompareFile> = paths
        .into_iter()
        .map(|path| {
            let mut seen: Vec<String> = Vec::new();
            let mut base_used = false;
            let cells: Vec<CompareCell> = per_entry
                .iter()
                .map(|(id, m)| match m.get(path) {
                    None => {
                        base_used = true;
                        CompareCell {
                            entry_id: id.clone(),
                            status: "=".into(),
                            additions: None,
                            deletions: None,
                            variant: 0,
                        }
                    }
                    Some(t) => {
                        let key = format!("{}:{}", t.status, t.blob);
                        let n = match seen.iter().position(|k| *k == key) {
                            Some(i) => i + 1,
                            None => {
                                seen.push(key);
                                seen.len()
                            }
                        };
                        CompareCell {
                            entry_id: id.clone(),
                            status: t.status.to_string(),
                            additions: t.additions,
                            deletions: t.deletions,
                            variant: n,
                        }
                    }
                })
                .collect();
            let variants = seen.len() + usize::from(base_used);
            CompareFile {
                path: path.clone(),
                divergent: variants > 1,
                touched_by: cells.iter().filter(|c| c.status != "=").count(),
                variants,
                cells,
            }
        })
        .collect();
    files.sort_by(|a, b| b.divergent.cmp(&a.divergent).then(a.path.cmp(&b.path)));
    files
}

pub async fn touches(
    root: &Path,
    base: &str,
    commit: &str,
) -> Result<BTreeMap<String, Touch>, String> {
    let ctx = gctx(root);
    let run = |args: Vec<String>| {
        let ctx = ctx.clone();
        async move {
            ctx.git()
                .args(args)
                .timeout(Some(Duration::from_secs(60)))
                .run()
                .await
                .map(|o| o.stdout)
                .map_err(|e| e.to_string())
        }
    };
    let raw = run(vec![
        "diff".into(),
        "--raw".into(),
        "-z".into(),
        "--no-renames".into(),
        "--no-abbrev".into(),
        base.into(),
        commit.into(),
    ])
    .await?;
    let num = run(vec![
        "diff".into(),
        "--numstat".into(),
        "-z".into(),
        "--no-renames".into(),
        base.into(),
        commit.into(),
    ])
    .await?;
    let mut out = parse_raw(&raw);
    for (path, a, d) in parse_numstat(&num) {
        if let Some(t) = out.get_mut(&path) {
            t.additions = a;
            t.deletions = d;
        }
    }
    Ok(out)
}

pub fn parse_raw(bytes: &[u8]) -> BTreeMap<String, Touch> {
    let parts: Vec<&[u8]> = bytes.split(|b| *b == 0).collect();
    let mut out = BTreeMap::new();
    let mut i = 0;
    while i + 1 < parts.len() {
        let meta = String::from_utf8_lossy(parts[i]);
        let path = String::from_utf8_lossy(parts[i + 1]).into_owned();
        i += 2;
        let f: Vec<&str> = meta.trim_start_matches(':').split_whitespace().collect();
        if f.len() < 5 || path.is_empty() {
            continue;
        }
        let status = f[4].chars().next().unwrap_or('M');
        out.insert(
            path,
            Touch {
                status: if matches!(status, 'A' | 'D') {
                    status
                } else {
                    'M'
                },
                blob: f[3].to_string(),
                additions: None,
                deletions: None,
            },
        );
    }
    out
}

pub fn parse_numstat(bytes: &[u8]) -> Vec<(String, Option<i64>, Option<i64>)> {
    bytes
        .split(|b| *b == 0)
        .filter_map(|rec| {
            let s = String::from_utf8_lossy(rec);
            let mut it = s.splitn(3, '\t');
            let a = it.next()?.trim().to_string();
            let d = it.next()?.to_string();
            let p = it.next()?.to_string();
            (!p.is_empty()).then(|| (p, a.parse().ok(), d.parse().ok()))
        })
        .collect()
}

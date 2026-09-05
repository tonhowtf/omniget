//! Ledger local de uso de IA (estudo 17, ccusage): cada chamada a LLM/TTS
//! vira uma linha em `<app_data>/tools/ai_usage.jsonl`. O custo é calculado
//! na leitura, com a tabela de preços do momento, para poder recalcular.

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UsageEntry {
    pub ts: String,
    pub task: String,
    pub provider: String,
    pub model: String,
    #[serde(default)]
    pub input_tokens: u64,
    #[serde(default)]
    pub output_tokens: u64,
    #[serde(default)]
    pub characters: u64,
    #[serde(default)]
    pub seconds: f64,
    /// Custo informado na hora (ex.: 0 para Ollama). `None` = calcular.
    #[serde(default)]
    pub cost_usd: Option<f64>,
}

fn path() -> Option<PathBuf> {
    super::tools_dir().map(|d| d.join("ai_usage.jsonl"))
}

/// Grava sem bloquear quem chamou; erro de disco vira só um log.
pub fn record(
    task: &str,
    provider: &str,
    model: &str,
    input_tokens: u64,
    output_tokens: u64,
    cost_usd: Option<f64>,
) {
    let entry = UsageEntry {
        ts: chrono::Utc::now().to_rfc3339(),
        task: task.to_string(),
        provider: provider.to_string(),
        model: model.to_string(),
        input_tokens,
        output_tokens,
        characters: 0,
        seconds: 0.0,
        cost_usd,
    };
    let Some(p) = path() else { return };
    if let Some(parent) = p.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let Ok(line) = serde_json::to_string(&entry) else {
        return;
    };
    use std::io::Write;
    match std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&p)
    {
        Ok(mut f) => {
            let _ = writeln!(f, "{}", line);
        }
        Err(e) => tracing::warn!("[usage] nao gravou: {}", e),
    }
}

pub fn read_all() -> Vec<UsageEntry> {
    let Some(p) = path() else { return vec![] };
    let Ok(text) = std::fs::read_to_string(&p) else {
        return vec![];
    };
    text.lines()
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect()
}

pub fn clear() -> anyhow::Result<()> {
    if let Some(p) = path() {
        if p.exists() {
            std::fs::remove_file(p)?;
        }
    }
    Ok(())
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct Bucket {
    pub key: String,
    pub calls: u64,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cost_usd: f64,
    pub unknown_price: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct UsageReport {
    pub since: String,
    pub calls: u64,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cost_usd: f64,
    pub unknown_price: u64,
    pub by_day: Vec<Bucket>,
    pub by_model: Vec<Bucket>,
    pub by_task: Vec<Bucket>,
    pub entries_path: Option<String>,
}

pub async fn report(days: u32) -> UsageReport {
    let since = chrono::Utc::now() - chrono::Duration::days(days.max(1) as i64);
    let entries: Vec<UsageEntry> = read_all()
        .into_iter()
        .filter(|e| {
            chrono::DateTime::parse_from_rfc3339(&e.ts)
                .map(|t| t.with_timezone(&chrono::Utc) >= since)
                .unwrap_or(false)
        })
        .collect();
    let mut by_day: BTreeMap<String, Bucket> = BTreeMap::new();
    let mut by_model: BTreeMap<String, Bucket> = BTreeMap::new();
    let mut by_task: BTreeMap<String, Bucket> = BTreeMap::new();
    let mut total = Bucket::default();
    let mut price_cache: std::collections::HashMap<String, Option<super::pricing::ModelPrice>> =
        Default::default();
    for e in &entries {
        let cost = match e.cost_usd {
            Some(c) => Some(c),
            None => {
                let p = match price_cache.get(&e.model) {
                    Some(p) => p.clone(),
                    None => {
                        let p = super::pricing::price_for(&e.model).await;
                        price_cache.insert(e.model.clone(), p.clone());
                        p
                    }
                };
                p.and_then(|p| super::pricing::cost(&p, e.input_tokens, e.output_tokens))
            }
        };
        let day = e.ts.get(..10).unwrap_or("").to_string();
        for (map, key) in [
            (&mut by_day, day),
            (&mut by_model, e.model.clone()),
            (&mut by_task, e.task.clone()),
        ] {
            let b = map.entry(key.clone()).or_insert_with(|| Bucket {
                key,
                ..Default::default()
            });
            b.calls += 1;
            b.input_tokens += e.input_tokens;
            b.output_tokens += e.output_tokens;
            match cost {
                Some(c) => b.cost_usd += c,
                None => b.unknown_price += 1,
            }
        }
        total.calls += 1;
        total.input_tokens += e.input_tokens;
        total.output_tokens += e.output_tokens;
        match cost {
            Some(c) => total.cost_usd += c,
            None => total.unknown_price += 1,
        }
    }
    let mut by_model: Vec<Bucket> = by_model.into_values().collect();
    by_model.sort_by(|a, b| {
        b.cost_usd
            .partial_cmp(&a.cost_usd)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(b.calls.cmp(&a.calls))
    });
    UsageReport {
        since: since.to_rfc3339(),
        calls: total.calls,
        input_tokens: total.input_tokens,
        output_tokens: total.output_tokens,
        cost_usd: total.cost_usd,
        unknown_price: total.unknown_price,
        by_day: by_day.into_values().collect(),
        by_model,
        by_task: by_task.into_values().collect(),
        entries_path: path().map(|p| p.to_string_lossy().to_string()),
    }
}

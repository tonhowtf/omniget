//! Ledger local de uso de IA (estudo 17, ccusage): cada chamada a LLM/TTS
//! vira uma linha em `<app_data>/tools/ai_usage.jsonl`. O custo é calculado
//! na leitura, com a tabela de preços do momento, para poder recalcular.

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct UsageEntry {
    pub ts: String,
    /// Texto livre: "chat", "grok", "transcribe", "humanize", ou o que o agente
    /// quiser. A UI agrupa por ele sem tabela fechada.
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
    // ── Campos da expansao LLM (f2-llm-providers). Todos com `default`, e por
    // isso as linhas antigas do JSONL continuam sendo lidas sem migracao.
    /// Agente dono da chamada (`AgentDef::id`); vazio quando veio do app.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub agent_id: String,
    /// Tokens de entrada que o provedor serviu do cache de prompt.
    #[serde(default)]
    pub cache_read_tokens: u64,
    /// Tokens gravados no cache de prompt (Anthropic cobra a mais por eles).
    #[serde(default)]
    pub cache_write_tokens: u64,
    /// Latencia ate o primeiro token, em ms (so no streaming).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub first_token_ms: Option<u32>,
}

impl UsageEntry {
    /// Entrada carimbada com o instante atual; os campos de numero ficam em
    /// zero para quem so tem alguns deles.
    pub fn now(task: &str, provider: &str, model: &str) -> Self {
        Self {
            ts: chrono::Utc::now().to_rfc3339(),
            task: task.to_string(),
            provider: provider.to_string(),
            model: model.to_string(),
            ..Self::default()
        }
    }
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
    record_entry(UsageEntry {
        input_tokens,
        output_tokens,
        cost_usd,
        ..UsageEntry::now(task, provider, model)
    });
}

/// Grava uma linha inteira (com agente, cache e latencia). Mesma politica do
/// `record`: erro de disco vira log, nunca erro para quem chamou.
pub fn record_entry(mut entry: UsageEntry) {
    if entry.ts.is_empty() {
        entry.ts = chrono::Utc::now().to_rfc3339();
    }
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
    #[serde(default)]
    pub cache_read_tokens: u64,
    #[serde(default)]
    pub cache_write_tokens: u64,
    pub cost_usd: f64,
    pub unknown_price: u64,
}

/// Custo de uma linha com a tabela de precos, levando o cache em conta: token
/// servido do cache sai do preco de entrada e entra pelo preco de leitura de
/// cache; token gravado no cache entra pelo preco de escrita. Sem esses precos
/// na tabela, tudo volta a valer o preco de entrada (o que o `pricing::cost`
/// ja fazia).
///
/// Convencao do `UsageEntry`: `input_tokens` e o total de entrada, cache
/// incluso, como o `prompt_tokens` da OpenAI. Os clientes normalizam a
/// Anthropic (que reporta os tres separados) para essa mesma forma.
pub fn cost_of(price: &super::pricing::ModelPrice, e: &UsageEntry) -> Option<f64> {
    let in_per = price.input_per_m? / 1_000_000.0;
    let out_per = price.output_per_m.unwrap_or(0.0) / 1_000_000.0;
    let read_per = price.cache_read_per_m.map(|v| v / 1e6).unwrap_or(in_per);
    let write_per = price.cache_write_per_m.map(|v| v / 1e6).unwrap_or(in_per);
    let cached = e.cache_read_tokens.min(e.input_tokens);
    let written = e.cache_write_tokens.min(e.input_tokens - cached);
    let fresh = e.input_tokens - cached - written;
    Some(
        fresh as f64 * in_per
            + cached as f64 * read_per
            + written as f64 * write_per
            + e.output_tokens as f64 * out_per,
    )
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
    /// Chave = `agent_id`; linhas sem agente caem em "app".
    pub by_agent: Vec<Bucket>,
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
    let mut by_agent: BTreeMap<String, Bucket> = BTreeMap::new();
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
                p.and_then(|p| cost_of(&p, e))
            }
        };
        let day = e.ts.get(..10).unwrap_or("").to_string();
        let agent = if e.agent_id.is_empty() {
            "app".to_string()
        } else {
            e.agent_id.clone()
        };
        for (map, key) in [
            (&mut by_day, day),
            (&mut by_model, e.model.clone()),
            (&mut by_task, e.task.clone()),
            (&mut by_agent, agent),
        ] {
            let b = map.entry(key.clone()).or_insert_with(|| Bucket {
                key,
                ..Default::default()
            });
            b.calls += 1;
            b.input_tokens += e.input_tokens;
            b.output_tokens += e.output_tokens;
            b.cache_read_tokens += e.cache_read_tokens;
            b.cache_write_tokens += e.cache_write_tokens;
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
        by_agent: by_agent.into_values().collect(),
        entries_path: path().map(|p| p.to_string_lossy().to_string()),
    }
}

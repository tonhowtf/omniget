//! Análise por sessão e agregados sobre o índice.
//!
//! Por sessão (sobre a `Session` carregada): duração, tempo do agente ×
//! tempo de pensar da pessoa, eficiência de cache, custo (gravado pela
//! ferramenta tem prioridade; o resto sai da tabela LiteLLM), mix de modelos,
//! tools, componentes (subagentes, slash commands, skills, MCP) e dicas.
//!
//! Agregados (sobre as tabelas do índice): uso por dia/semana/mês/modelo/
//! ferramenta/projeto/conta/hora, heatmap de um ano com sequências, agentes
//! e padrões A→B→C, grafo de time e retrospectiva de período.

use std::collections::{BTreeMap, HashMap, HashSet};

use chrono::Datelike;
use rusqlite::params_from_iter;
use serde::Serialize;

use super::cost::{CostSummary, PriceBook};
use super::index::{self, ListFilter, SessionIndex};
use super::model::{Role, Session, SessionMeta, TokenUsage, ToolStatus};
use super::util;

// ============================================================================
// Por sessão
// ============================================================================

#[derive(Debug, Clone, Default, Serialize)]
pub struct TimeBreakdown {
    /// Da mensagem da pessoa até a última resposta antes da próxima mensagem.
    pub agent_ms: u64,
    /// Da última resposta até a próxima mensagem da pessoa (lacunas < 1 h).
    pub think_ms: u64,
    /// Lacunas ≥ 1 h (fora do teclado): não contam como pensar.
    pub away_ms: u64,
    pub agent_pct: f64,
    pub think_pct: f64,
    pub exchanges: u32,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct CacheStats {
    pub read: u64,
    pub write: u64,
    /// `read / (read + write)`, como o original.
    pub efficiency: f64,
    /// Fração da entrada servida do cache: `read / (input + read + write)`.
    pub hit_ratio: f64,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct ModelShare {
    pub model: String,
    pub responses: u32,
    pub pct: f64,
    pub tokens: u64,
    pub cost_usd: Option<f64>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct ToolStat {
    pub name: String,
    pub raw: Vec<String>,
    pub count: u32,
    pub errors: u32,
    pub avg_ms: Option<u64>,
    pub total_ms: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct TimelineItem {
    pub turn: usize,
    pub ts: String,
    pub name: String,
    pub raw: String,
    pub status: ToolStatus,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct Components {
    pub agents: Vec<(String, u32)>,
    pub slash_commands: Vec<(String, u32)>,
    pub skills: Vec<(String, u32)>,
    pub mcp_servers: Vec<(String, u32)>,
}

/// Dica por regra. `code` vira chave de i18n
/// (`llm.central.sessions.tips.<code>`); `value` é o número que disparou.
#[derive(Debug, Clone, Serialize)]
pub struct Tip {
    pub code: String,
    /// `info` | `warn`.
    pub severity: String,
    pub value: f64,
    /// Texto em inglês de reserva.
    pub message: String,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct Counts {
    pub turns: u32,
    pub user: u32,
    pub assistant: u32,
    pub tool_calls: u32,
    pub tool_errors: u32,
}

#[derive(Debug, Clone, Serialize)]
pub struct SessionAnalysis {
    pub meta: SessionMeta,
    pub duration_ms: u64,
    pub time: TimeBreakdown,
    pub cache: CacheStats,
    pub usage: TokenUsage,
    pub cost: CostSummary,
    pub models: Vec<ModelShare>,
    pub tools: Vec<ToolStat>,
    pub timeline: Vec<TimelineItem>,
    pub components: Components,
    pub counts: Counts,
    /// Maior contexto de uma resposta (entrada + cache).
    pub max_context: u64,
    pub tips: Vec<Tip>,
}

fn sorted_counts(m: HashMap<String, u32>) -> Vec<(String, u32)> {
    let mut v: Vec<(String, u32)> = m.into_iter().collect();
    v.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    v
}

const WAIT_CAP_MS: i64 = 3_600_000;

/// `recorded_session_cost`: custo gravado no nível da sessão (Claude
/// `cost-state`, total do Crush/Goose), usado quando as `Turn` não trazem.
pub fn analyze(
    s: &Session,
    book: &PriceBook,
    recorded_session_cost: Option<f64>,
) -> SessionAnalysis {
    let times: Vec<i64> = s.turns.iter().map(|t| util::turn_ms(&t.ts)).collect();
    let valid: Vec<i64> = times.iter().copied().filter(|t| *t > 0).collect();
    let duration_ms = match (valid.iter().min(), valid.iter().max()) {
        (Some(a), Some(b)) => (b - a) as u64,
        _ => 0,
    };

    // Tempo: blocos "pessoa fala → agente trabalha até a próxima fala".
    let mut time = TimeBreakdown::default();
    let mut i = 0;
    while i < s.turns.len() {
        if s.turns[i].role != Role::User || times[i] == 0 {
            i += 1;
            continue;
        }
        let start = times[i];
        let mut j = i + 1;
        let mut last_agent = None;
        while j < s.turns.len() && s.turns[j].role != Role::User {
            if s.turns[j].role == Role::Assistant && times[j] > 0 {
                last_agent = Some(times[j]);
            }
            j += 1;
        }
        if let Some(end) = last_agent {
            time.exchanges += 1;
            if end > start {
                time.agent_ms += (end - start) as u64;
            }
            if j < s.turns.len() && times[j] > end {
                let gap = times[j] - end;
                if gap < WAIT_CAP_MS {
                    time.think_ms += gap as u64;
                } else {
                    time.away_ms += gap as u64;
                }
            }
        }
        i = j;
    }
    let tot = (time.agent_ms + time.think_ms) as f64;
    if tot > 0.0 {
        time.agent_pct = time.agent_ms as f64 * 100.0 / tot;
        time.think_pct = time.think_ms as f64 * 100.0 / tot;
    }

    let mut usage = TokenUsage::default();
    let mut counts = Counts {
        turns: s.turns.len() as u32,
        ..Default::default()
    };
    let mut model_resp: HashMap<String, (u32, u64, TokenUsage)> = HashMap::new();
    let mut tools: BTreeMap<String, ToolStat> = BTreeMap::new();
    let mut timeline = Vec::new();
    let mut agents: HashMap<String, u32> = HashMap::new();
    let mut slash: HashMap<String, u32> = HashMap::new();
    let mut skills: HashMap<String, u32> = HashMap::new();
    let mut mcp: HashMap<String, u32> = HashMap::new();
    let mut cost = CostSummary::default();
    let mut max_context = 0u64;
    let mut reads_per_file: HashMap<String, u32> = HashMap::new();
    let mut any_turn_cost = false;
    for (ti, t) in s.turns.iter().enumerate() {
        usage.add(&t.usage);
        match t.role {
            Role::User => counts.user += 1,
            Role::Assistant => counts.assistant += 1,
            _ => {}
        }
        if matches!(t.role, Role::User | Role::System) {
            for c in index::slash_commands(&t.text) {
                *slash.entry(c).or_default() += 1;
            }
        }
        if t.role == Role::Assistant {
            let ctx = t.usage.input + t.usage.cache_read + t.usage.cache_write;
            max_context = max_context.max(ctx);
            if let Some(m) = &t.model {
                if m != "<synthetic>" {
                    let e = model_resp.entry(m.clone()).or_default();
                    e.0 += 1;
                    e.1 += t.usage.total();
                    e.2.add(&t.usage);
                }
            }
        }
        match t.cost_usd {
            Some(c) => {
                any_turn_cost = true;
                cost.add_recorded(c);
            }
            None => {
                if recorded_session_cost.is_none() {
                    cost.add_usage(book, t.model.as_deref().unwrap_or(""), &t.usage);
                }
            }
        }
        for c in &t.tool_calls {
            counts.tool_calls += 1;
            let st = tools
                .entry(c.name_canonical.clone())
                .or_insert_with(|| ToolStat {
                    name: c.name_canonical.clone(),
                    ..Default::default()
                });
            st.count += 1;
            if !st.raw.contains(&c.name_raw) {
                st.raw.push(c.name_raw.clone());
            }
            if c.status == ToolStatus::Error {
                st.errors += 1;
                counts.tool_errors += 1;
            }
            if let Some(ms) = c.ms {
                st.total_ms += ms;
            }
            if timeline.len() < 5000 {
                timeline.push(TimelineItem {
                    turn: ti,
                    ts: t.ts.clone(),
                    name: c.name_canonical.clone(),
                    raw: c.name_raw.clone(),
                    status: c.status,
                });
            }
            match c.name_canonical.as_str() {
                "Agent" => {
                    *agents
                        .entry(c.subagent.clone().unwrap_or_else(|| "agent".into()))
                        .or_default() += 1
                }
                "Skill" => {
                    let n = util::str_of(&c.input, "skill")
                        .or_else(|| util::str_of(&c.input, "command"))
                        .or_else(|| util::str_of(&c.input, "name"))
                        .unwrap_or("?")
                        .to_string();
                    *skills.entry(n).or_default() += 1;
                }
                "Mcp" => {
                    let r = c
                        .name_raw
                        .trim_start_matches("mcp__")
                        .trim_start_matches("mcp_");
                    let server = r.split("__").next().unwrap_or(r).to_string();
                    *mcp.entry(server).or_default() += 1;
                }
                "Read" => {
                    if let Some(p) = util::str_of(&c.input, "file_path")
                        .or_else(|| util::str_of(&c.input, "path"))
                        .or_else(|| util::str_of(&c.input, "absolute_path"))
                    {
                        *reads_per_file.entry(p.to_string()).or_default() += 1;
                    }
                }
                _ => {}
            }
            if c.name_raw == "SlashCommand" {
                if let Some(cmd) = util::str_of(&c.input, "command") {
                    let name = cmd.split_whitespace().next().unwrap_or(cmd).to_string();
                    *slash.entry(name).or_default() += 1;
                }
            }
        }
    }
    if let Some(rc) = recorded_session_cost.filter(|_| !any_turn_cost) {
        cost.add_recorded(rc);
    }
    cost.finish();
    for st in tools.values_mut() {
        let timed = s
            .turns
            .iter()
            .flat_map(|t| t.tool_calls.iter())
            .filter(|c| c.name_canonical == st.name && c.ms.is_some())
            .count() as u64;
        if timed > 0 {
            st.avg_ms = Some(st.total_ms / timed);
        }
    }
    let total_resp: u32 = model_resp.values().map(|v| v.0).sum();
    let mut models: Vec<ModelShare> = model_resp
        .into_iter()
        .map(|(m, (n, tok, u))| ModelShare {
            cost_usd: book.cost(&m, &u),
            pct: if total_resp > 0 {
                n as f64 * 100.0 / total_resp as f64
            } else {
                0.0
            },
            model: m,
            responses: n,
            tokens: tok,
        })
        .collect();
    models.sort_by(|a, b| b.responses.cmp(&a.responses));
    let mut tools: Vec<ToolStat> = tools.into_values().collect();
    tools.sort_by(|a, b| b.count.cmp(&a.count));
    let cache_den = (usage.cache_read + usage.cache_write) as f64;
    let cache = CacheStats {
        read: usage.cache_read,
        write: usage.cache_write,
        efficiency: if cache_den > 0.0 {
            usage.cache_read as f64 * 100.0 / cache_den
        } else {
            0.0
        },
        hit_ratio: {
            let d = (usage.input + usage.cache_read + usage.cache_write) as f64;
            if d > 0.0 {
                usage.cache_read as f64 * 100.0 / d
            } else {
                0.0
            }
        },
    };
    let components = Components {
        agents: sorted_counts(agents),
        slash_commands: sorted_counts(slash),
        skills: sorted_counts(skills),
        mcp_servers: sorted_counts(mcp),
    };
    let tips = tips_for(&TipInput {
        cache: &cache,
        usage: &usage,
        counts: &counts,
        models: models.len(),
        time: &time,
        cost: cost.cost_usd,
        max_context,
        max_reads_same_file: reads_per_file.values().copied().max().unwrap_or(0),
        agents: components.agents.iter().map(|a| a.1).sum(),
        unpriced: cost.unpriced_tokens,
    });
    let mut meta = s.meta.clone();
    meta.usage = usage.clone();
    meta.cost_usd = cost.cost_usd;
    SessionAnalysis {
        meta,
        duration_ms,
        time,
        cache,
        usage,
        cost,
        models,
        tools,
        timeline,
        components,
        counts,
        max_context,
        tips,
    }
}

struct TipInput<'a> {
    cache: &'a CacheStats,
    usage: &'a TokenUsage,
    counts: &'a Counts,
    models: usize,
    time: &'a TimeBreakdown,
    cost: f64,
    max_context: u64,
    max_reads_same_file: u32,
    agents: u32,
    unpriced: u64,
}

fn tip(code: &str, severity: &str, value: f64, message: &str) -> Tip {
    Tip {
        code: code.into(),
        severity: severity.into(),
        value,
        message: message.into(),
    }
}

fn tips_for(x: &TipInput) -> Vec<Tip> {
    let mut v = Vec::new();
    let input_all = x.usage.input + x.usage.cache_read + x.usage.cache_write;
    // As seis do original.
    if x.cache.read + x.cache.write > 0 && x.cache.efficiency < 20.0 {
        v.push(tip(
            "low_cache",
            "warn",
            x.cache.efficiency,
            "Cache efficiency is below 20%: keep the system prompt and early context stable so the cache can be reused.",
        ));
    }
    if x.counts.tool_calls > 50 {
        v.push(tip(
            "many_tools",
            "info",
            x.counts.tool_calls as f64,
            "More than 50 tool calls: break the task into smaller steps or give the agent more precise instructions.",
        ));
    }
    if input_all > 0 && (x.usage.output + x.usage.reasoning) > 2 * input_all {
        v.push(tip(
            "output_heavy",
            "info",
            (x.usage.output + x.usage.reasoning) as f64 / input_all as f64,
            "Output is more than twice the input: ask for shorter answers or diffs instead of whole files.",
        ));
    }
    if x.counts.turns > 100 {
        v.push(tip(
            "long_session",
            "info",
            x.counts.turns as f64,
            "Over 100 messages: start a fresh session per task or compact the context.",
        ));
    }
    if x.models > 1 {
        v.push(tip(
            "multiple_models",
            "info",
            x.models as f64,
            "Several models were used: switching models invalidates the prompt cache.",
        ));
    }
    if x.time.agent_pct > 70.0 && x.time.exchanges >= 3 {
        v.push(tip(
            "high_wait",
            "info",
            x.time.agent_pct,
            "Most of the time was spent waiting for the agent: run long work in the background or in parallel sessions.",
        ));
    }
    // Extras.
    if x.counts.tool_calls >= 10 && x.counts.tool_errors * 5 > x.counts.tool_calls {
        v.push(tip(
            "tool_errors",
            "warn",
            x.counts.tool_errors as f64 * 100.0 / x.counts.tool_calls as f64,
            "More than 20% of tool calls failed: check permissions, paths and missing commands.",
        ));
    }
    if x.max_context > 150_000 {
        v.push(tip(
            "big_context",
            "warn",
            x.max_context as f64,
            "A single request carried more than 150k tokens of context: compact or split the session.",
        ));
    }
    if x.max_reads_same_file > 5 {
        v.push(tip(
            "repeated_reads",
            "info",
            x.max_reads_same_file as f64,
            "The same file was read more than 5 times: point the agent to the relevant lines or keep a summary.",
        ));
    }
    if x.cost > 10.0 {
        v.push(tip(
            "expensive",
            "warn",
            x.cost,
            "This session cost more than $10: consider a cheaper model for exploration and subagents for side tasks.",
        ));
    }
    if x.agents == 0 && x.counts.tool_calls > 80 {
        v.push(tip(
            "use_subagents",
            "info",
            x.counts.tool_calls as f64,
            "Many tool calls and no subagent: delegating searches to subagents keeps the main context small.",
        ));
    }
    if x.unpriced > 0 {
        v.push(tip(
            "unpriced_model",
            "info",
            x.unpriced as f64,
            "Some tokens belong to a model without a known price: the cost shown is a lower bound.",
        ));
    }
    v
}

// ============================================================================
// Agregados
// ============================================================================

#[derive(Debug, Clone, Default, Serialize)]
pub struct UsageBucket {
    pub key: String,
    pub usage: TokenUsage,
    pub total_tokens: u64,
    pub cost_usd: f64,
    pub recorded_usd: f64,
    pub unpriced_tokens: u64,
    pub responses: u64,
    pub sessions: u64,
}

impl UsageBucket {
    fn absorb(&mut self, o: &UsageBucket) {
        self.usage.add(&o.usage);
        self.total_tokens += o.total_tokens;
        self.cost_usd += o.cost_usd;
        self.recorded_usd += o.recorded_usd;
        self.unpriced_tokens += o.unpriced_tokens;
        self.responses += o.responses;
    }
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct UsageReport {
    pub group_by: String,
    pub rows: Vec<UsageBucket>,
    pub totals: UsageBucket,
    pub unpriced_models: Vec<String>,
}

fn group_expr(group_by: &str) -> &'static str {
    match group_by {
        "week" => "strftime('%Y-W%W', u.day)",
        "month" => "substr(u.day, 1, 7)",
        "model" => "COALESCE(NULLIF(u.model,''),'?')",
        "tool" => "u.tool",
        "project" => "COALESCE(s.project_path,'')",
        "account" => "COALESCE(s.account,'')",
        "hour" => "printf('%02d', u.hour)",
        "session" => "u.tool || '/' || u.session_id",
        _ => "u.day",
    }
}

pub fn usage(idx: &SessionIndex, f: &ListFilter, group_by: &str, book: &PriceBook) -> UsageReport {
    let key = group_expr(group_by);
    let (w, p) = index::usage_where(f, "u");
    let sql = format!(
        "SELECT {key} AS k, COALESCE(u.model,''), SUM(u.input), SUM(u.output), SUM(u.cache_read), SUM(u.cache_write), SUM(u.reasoning),
                COALESCE(SUM(u.cost),0),
                SUM(CASE WHEN u.cost IS NULL THEN u.input ELSE 0 END),
                SUM(CASE WHEN u.cost IS NULL THEN u.output ELSE 0 END),
                SUM(CASE WHEN u.cost IS NULL THEN u.cache_read ELSE 0 END),
                SUM(CASE WHEN u.cost IS NULL THEN u.cache_write ELSE 0 END),
                SUM(CASE WHEN u.cost IS NULL THEN u.reasoning ELSE 0 END),
                COUNT(*)
         FROM usage u JOIN sessions s ON s.tool=u.tool AND s.id=u.session_id
         WHERE {w} GROUP BY k, u.model"
    );
    let mut buckets: BTreeMap<String, UsageBucket> = BTreeMap::new();
    let mut unpriced: HashSet<String> = HashSet::new();
    if let Ok(mut st) = idx.conn().prepare(&sql) {
        let rows = st.query_map(params_from_iter(p.iter()), |r| {
            let g = |i: usize| -> rusqlite::Result<u64> {
                Ok(r.get::<_, Option<i64>>(i)?.unwrap_or(0).max(0) as u64)
            };
            Ok((
                r.get::<_, Option<String>>(0)?.unwrap_or_default(),
                r.get::<_, String>(1)?,
                TokenUsage {
                    input: g(2)?,
                    output: g(3)?,
                    cache_read: g(4)?,
                    cache_write: g(5)?,
                    reasoning: g(6)?,
                },
                r.get::<_, f64>(7)?,
                TokenUsage {
                    input: g(8)?,
                    output: g(9)?,
                    cache_read: g(10)?,
                    cache_write: g(11)?,
                    reasoning: g(12)?,
                },
                g(13)?,
            ))
        });
        if let Ok(it) = rows {
            for (k, model, u, recorded, unpriced_u, n) in it.flatten() {
                let b = buckets.entry(k.clone()).or_insert_with(|| UsageBucket {
                    key: k.clone(),
                    ..Default::default()
                });
                b.usage.add(&u);
                b.total_tokens += u.total();
                b.recorded_usd += recorded;
                b.cost_usd += recorded;
                b.responses += n;
                if unpriced_u.total() > 0 {
                    match book.cost(&model, &unpriced_u) {
                        Some(c) => b.cost_usd += c,
                        None => {
                            b.unpriced_tokens += unpriced_u.total();
                            unpriced.insert(if model.is_empty() {
                                "?".into()
                            } else {
                                model.clone()
                            });
                        }
                    }
                }
            }
        }
    }
    // Sessões distintas por chave (não dá para somar entre modelos).
    let sql2 = format!(
        "SELECT {key} AS k, COUNT(DISTINCT u.tool || '/' || u.session_id)
         FROM usage u JOIN sessions s ON s.tool=u.tool AND s.id=u.session_id WHERE {w} GROUP BY k"
    );
    if let Ok(mut st) = idx.conn().prepare(&sql2) {
        if let Ok(rows) = st.query_map(params_from_iter(p.iter()), |r| {
            Ok((
                r.get::<_, Option<String>>(0)?.unwrap_or_default(),
                r.get::<_, i64>(1)?,
            ))
        }) {
            for (k, n) in rows.flatten() {
                if let Some(b) = buckets.get_mut(&k) {
                    b.sessions = n as u64;
                }
            }
        }
    }
    let mut totals = UsageBucket {
        key: "total".into(),
        ..Default::default()
    };
    for b in buckets.values() {
        totals.absorb(b);
    }
    let sql3 = format!(
        "SELECT COUNT(DISTINCT u.tool || '/' || u.session_id) FROM usage u JOIN sessions s ON s.tool=u.tool AND s.id=u.session_id WHERE {w}"
    );
    totals.sessions = idx
        .conn()
        .query_row(&sql3, params_from_iter(p.iter()), |r| r.get::<_, i64>(0))
        .unwrap_or(0) as u64;
    let mut rows: Vec<UsageBucket> = buckets.into_values().collect();
    if !matches!(group_by, "day" | "week" | "month" | "hour" | "") {
        rows.sort_by(|a, b| b.total_tokens.cmp(&a.total_tokens));
    }
    let mut unpriced: Vec<String> = unpriced.into_iter().collect();
    unpriced.sort();
    UsageReport {
        group_by: if group_by.is_empty() {
            "day".into()
        } else {
            group_by.to_string()
        },
        rows,
        totals,
        unpriced_models: unpriced,
    }
}

// --- heatmap -----------------------------------------------------------------

#[derive(Debug, Clone, Default, Serialize)]
pub struct HeatDay {
    pub date: String,
    pub sessions: u32,
    pub messages: u32,
    pub user_messages: u32,
    pub tokens: u64,
    pub tool_calls: u32,
    /// 0..=4 pelos quartis dos dias ativos (mensagens).
    pub level: u8,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct HeatStats {
    pub active_days: u32,
    pub current_streak: u32,
    pub longest_streak: u32,
    pub longest_streak_start: Option<String>,
    pub longest_streak_end: Option<String>,
    pub total_sessions: u32,
    pub total_messages: u32,
    pub total_tokens: u64,
    pub total_tool_calls: u32,
    pub peak_hour: Option<u32>,
    pub hours: Vec<u32>,
    /// 0 = segunda … 6 = domingo.
    pub peak_weekday: Option<u32>,
    pub weekdays: Vec<u32>,
    pub busiest_day: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct Heatmap {
    pub from: String,
    pub to: String,
    pub days: Vec<HeatDay>,
    pub stats: HeatStats,
}

fn today() -> chrono::NaiveDate {
    chrono::Local::now().date_naive()
}

fn parse_day(s: &str) -> Option<chrono::NaiveDate> {
    chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d").ok()
}

fn fill_range(f: &ListFilter, default_days: i64) -> (chrono::NaiveDate, chrono::NaiveDate) {
    let to = f.to.as_deref().and_then(parse_day).unwrap_or_else(today);
    let from = f
        .from
        .as_deref()
        .and_then(parse_day)
        .unwrap_or_else(|| to - chrono::Duration::days(default_days - 1));
    (from, to)
}

fn day_map<T>(
    idx: &SessionIndex,
    sql: &str,
    p: &[String],
    mut read: impl FnMut(&rusqlite::Row) -> rusqlite::Result<(String, T)>,
) -> HashMap<String, T> {
    let mut out = HashMap::new();
    if let Ok(mut st) = idx.conn().prepare(sql) {
        if let Ok(rows) = st.query_map(params_from_iter(p.iter()), |r| read(r)) {
            for (k, v) in rows.flatten() {
                out.insert(k, v);
            }
        }
    }
    out
}

pub fn heatmap(idx: &SessionIndex, f: &ListFilter) -> Heatmap {
    let (from, to) = fill_range(f, 365);
    let mut f = f.clone();
    f.from = Some(from.format("%Y-%m-%d").to_string());
    f.to = Some(to.format("%Y-%m-%d").to_string());
    let (wm, pm) = index::usage_where(&f, "m");
    let msgs = day_map(
        idx,
        &format!(
            "SELECT m.day, COUNT(*), SUM(m.role='user'), COUNT(DISTINCT m.tool||'/'||m.session_id)
             FROM msgs m JOIN sessions s ON s.tool=m.tool AND s.id=m.session_id WHERE {wm} GROUP BY m.day"
        ),
        &pm,
        |r| {
            Ok((
                r.get::<_, Option<String>>(0)?.unwrap_or_default(),
                (r.get::<_, i64>(1)? as u32, r.get::<_, i64>(2)? as u32, r.get::<_, i64>(3)? as u32),
            ))
        },
    );
    let (wu, pu) = index::usage_where(&f, "u");
    let toks = day_map(
        idx,
        &format!(
            "SELECT u.day, SUM(u.input+u.output+u.cache_read+u.cache_write+u.reasoning)
             FROM usage u JOIN sessions s ON s.tool=u.tool AND s.id=u.session_id WHERE {wu} GROUP BY u.day"
        ),
        &pu,
        |r| Ok((r.get::<_, Option<String>>(0)?.unwrap_or_default(), r.get::<_, i64>(1)? as u64)),
    );
    let (wc, pc) = index::usage_where(&f, "c");
    let calls = day_map(
        idx,
        &format!(
            "SELECT c.day, COUNT(*) FROM calls c JOIN sessions s ON s.tool=c.tool AND s.id=c.session_id
             WHERE {wc} AND c.name <> 'SlashCommand' GROUP BY c.day"
        ),
        &pc,
        |r| Ok((r.get::<_, Option<String>>(0)?.unwrap_or_default(), r.get::<_, i64>(1)? as u32)),
    );
    let mut days = Vec::new();
    let mut d = from;
    while d <= to {
        let key = d.format("%Y-%m-%d").to_string();
        let (messages, user, sessions) = msgs.get(&key).copied().unwrap_or((0, 0, 0));
        days.push(HeatDay {
            tokens: toks.get(&key).copied().unwrap_or(0),
            tool_calls: calls.get(&key).copied().unwrap_or(0),
            date: key,
            sessions,
            messages,
            user_messages: user,
            level: 0,
        });
        d += chrono::Duration::days(1);
    }
    // Níveis pelos quartis dos dias ativos.
    let mut active: Vec<u32> = days
        .iter()
        .filter(|d| d.messages > 0)
        .map(|d| d.messages)
        .collect();
    active.sort();
    let q = |p: f64| -> u32 {
        if active.is_empty() {
            0
        } else {
            active[((active.len() - 1) as f64 * p).round() as usize]
        }
    };
    let (q1, q2, q3) = (q(0.25), q(0.5), q(0.75));
    for day in &mut days {
        day.level = if day.messages == 0 {
            0
        } else if day.messages <= q1 {
            1
        } else if day.messages <= q2 {
            2
        } else if day.messages <= q3 {
            3
        } else {
            4
        };
    }
    let stats = heat_stats(idx, &f, &days);
    Heatmap {
        from: f.from.clone().unwrap_or_default(),
        to: f.to.clone().unwrap_or_default(),
        days,
        stats,
    }
}

fn streaks(days: &[HeatDay]) -> (u32, u32, Option<String>, Option<String>) {
    let mut longest = 0;
    let mut run = 0;
    let mut run_start: Option<&str> = None;
    let mut best: (Option<String>, Option<String>) = (None, None);
    for d in days {
        if d.messages > 0 {
            if run == 0 {
                run_start = Some(&d.date);
            }
            run += 1;
            if run > longest {
                longest = run;
                best = (run_start.map(|s| s.to_string()), Some(d.date.clone()));
            }
        } else {
            run = 0;
        }
    }
    // Sequência atual: termina hoje, ou ontem se hoje ainda está vazio.
    let mut current = 0;
    let mut it = days.iter().rev().peekable();
    if let Some(last) = it.peek() {
        if last.messages == 0 && last.date == today().format("%Y-%m-%d").to_string() {
            it.next();
        }
    }
    for d in it {
        if d.messages > 0 {
            current += 1;
        } else {
            break;
        }
    }
    (current, longest, best.0, best.1)
}

fn heat_stats(idx: &SessionIndex, f: &ListFilter, days: &[HeatDay]) -> HeatStats {
    let (current, longest, ls, le) = streaks(days);
    let mut hours = vec![0u32; 24];
    let (wm, pm) = index::usage_where(f, "m");
    if let Ok(mut st) = idx.conn().prepare(&format!(
        "SELECT m.hour, COUNT(*) FROM msgs m JOIN sessions s ON s.tool=m.tool AND s.id=m.session_id WHERE {wm} GROUP BY m.hour"
    )) {
        if let Ok(rows) = st.query_map(params_from_iter(pm.iter()), |r| {
            Ok((r.get::<_, Option<i64>>(0)?.unwrap_or(0), r.get::<_, i64>(1)?))
        }) {
            for (h, n) in rows.flatten() {
                if (0..24).contains(&h) {
                    hours[h as usize] = n as u32;
                }
            }
        }
    }
    let mut weekdays = vec![0u32; 7];
    for d in days {
        if let Some(nd) = parse_day(&d.date) {
            weekdays[nd.weekday().num_days_from_monday() as usize] += d.messages;
        }
    }
    let argmax = |v: &[u32]| -> Option<u32> {
        let (i, m) = v.iter().enumerate().max_by_key(|(_, n)| **n)?;
        (*m > 0).then_some(i as u32)
    };
    let busiest = days
        .iter()
        .max_by_key(|d| d.messages)
        .filter(|d| d.messages > 0);
    let (ws, ps) = index::usage_where(f, "m");
    let total_sessions: u32 = idx
        .conn()
        .query_row(
            &format!(
                "SELECT COUNT(DISTINCT m.tool||'/'||m.session_id) FROM msgs m JOIN sessions s ON s.tool=m.tool AND s.id=m.session_id WHERE {ws}"
            ),
            params_from_iter(ps.iter()),
            |r| r.get::<_, i64>(0),
        )
        .unwrap_or(0) as u32;
    HeatStats {
        active_days: days.iter().filter(|d| d.messages > 0).count() as u32,
        current_streak: current,
        longest_streak: longest,
        longest_streak_start: ls,
        longest_streak_end: le,
        total_sessions,
        total_messages: days.iter().map(|d| d.messages).sum(),
        total_tokens: days.iter().map(|d| d.tokens).sum(),
        total_tool_calls: days.iter().map(|d| d.tool_calls).sum(),
        peak_hour: argmax(&hours),
        hours,
        peak_weekday: argmax(&weekdays),
        weekdays,
        busiest_day: busiest.map(|d| d.date.clone()),
    }
}

// --- agentes e subagentes --------------------------------------------------

#[derive(Debug, Clone, Default, Serialize)]
pub struct AgentStat {
    pub name: String,
    pub invocations: u32,
    pub sessions: u32,
    pub tools: Vec<String>,
    pub first: Option<String>,
    pub last: Option<String>,
    pub hourly: Vec<u32>,
    pub daily: Vec<(String, u32)>,
    pub prompts: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct Pattern {
    pub pattern: String,
    pub count: u32,
}

#[derive(Debug, Clone, Serialize)]
pub struct AgentEvent {
    pub ts: String,
    pub tool: String,
    pub session_id: String,
    pub agent: String,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct AgentsReport {
    pub agents: Vec<AgentStat>,
    pub patterns: Vec<Pattern>,
    pub timeline: Vec<AgentEvent>,
    pub popular_hours: Vec<u32>,
    pub by_day: Vec<(String, u32)>,
    pub by_tool: Vec<(String, u32)>,
    /// Sessões-filhas no índice (arquivos de subagente) por ferramenta.
    pub subagent_sessions: Vec<(String, u32)>,
    pub total_invocations: u32,
    pub agent_types: u32,
    pub top_agent: Option<String>,
    pub avg_invocations_per_type: f64,
    /// % dos tipos usados mais de uma vez.
    pub adoption_rate: f64,
}

const PATTERN_GAP_MS: i64 = 30 * 60 * 1000;

pub fn agents(idx: &SessionIndex, f: &ListFilter) -> AgentsReport {
    let (w, p) = index::usage_where(f, "c");
    let sql = format!(
        "SELECT c.tool, c.session_id, c.ts_ms, c.day, c.hour, COALESCE(c.subagent, c.raw), c.detail
         FROM calls c JOIN sessions s ON s.tool=c.tool AND s.id=c.session_id
         WHERE {w} AND c.name='Agent' ORDER BY c.tool, c.session_id, c.ts_ms"
    );
    struct Ev {
        tool: String,
        session: String,
        ts: i64,
        day: String,
        hour: i64,
        agent: String,
        detail: Option<String>,
    }
    let mut evs: Vec<Ev> = Vec::new();
    if let Ok(mut st) = idx.conn().prepare(&sql) {
        if let Ok(rows) = st.query_map(params_from_iter(p.iter()), |r| {
            Ok(Ev {
                tool: r.get(0)?,
                session: r.get(1)?,
                ts: r.get::<_, Option<i64>>(2)?.unwrap_or(0),
                day: r.get::<_, Option<String>>(3)?.unwrap_or_default(),
                hour: r.get::<_, Option<i64>>(4)?.unwrap_or(0),
                agent: r
                    .get::<_, Option<String>>(5)?
                    .unwrap_or_else(|| "agent".into()),
                detail: r.get(6)?,
            })
        }) {
            evs = rows.flatten().collect();
        }
    }
    let mut per: BTreeMap<
        String,
        (
            AgentStat,
            HashSet<String>,
            BTreeMap<String, u32>,
            HashSet<String>,
        ),
    > = BTreeMap::new();
    let mut popular = vec![0u32; 24];
    let mut by_day: BTreeMap<String, u32> = BTreeMap::new();
    let mut by_tool: BTreeMap<String, u32> = BTreeMap::new();
    let mut patterns: HashMap<String, u32> = HashMap::new();
    let mut seq: Vec<String> = Vec::new();
    let mut prev: Option<(&str, &str, i64)> = None;
    let flush = |seq: &mut Vec<String>, patterns: &mut HashMap<String, u32>| {
        if seq.len() >= 2 {
            *patterns.entry(seq.join(" → ")).or_default() += 1;
        }
        seq.clear();
    };
    for e in &evs {
        let entry = per.entry(e.agent.clone()).or_insert_with(|| {
            (
                AgentStat {
                    name: e.agent.clone(),
                    hourly: vec![0; 24],
                    ..Default::default()
                },
                HashSet::new(),
                BTreeMap::new(),
                HashSet::new(),
            )
        });
        entry.0.invocations += 1;
        entry.1.insert(format!("{}/{}", e.tool, e.session));
        *entry.2.entry(e.day.clone()).or_default() += 1;
        entry.3.insert(e.tool.clone());
        if (0..24).contains(&e.hour) {
            entry.0.hourly[e.hour as usize] += 1;
            popular[e.hour as usize] += 1;
        }
        let ts = util::ms_to_rfc3339(e.ts);
        if entry.0.first.is_none() || entry.0.first.as_deref() > Some(ts.as_str()) {
            entry.0.first = Some(ts.clone());
        }
        if entry.0.last.as_deref() < Some(ts.as_str()) {
            entry.0.last = Some(ts.clone());
        }
        if let Some(d) = &e.detail {
            if entry.0.prompts.len() < 5 && !entry.0.prompts.contains(d) {
                entry.0.prompts.push(d.clone());
            }
        }
        *by_day.entry(e.day.clone()).or_default() += 1;
        *by_tool.entry(e.tool.clone()).or_default() += 1;
        // Padrões: sequência na mesma sessão com intervalos < 30 min.
        let same = matches!(prev, Some((t, s, pts)) if t == e.tool && s == e.session && e.ts - pts < PATTERN_GAP_MS);
        if !same {
            flush(&mut seq, &mut patterns);
        }
        seq.push(e.agent.clone());
        prev = Some((&e.tool, &e.session, e.ts));
    }
    flush(&mut seq, &mut patterns);
    let mut agents: Vec<AgentStat> = per
        .into_values()
        .map(|(mut a, sessions, daily, tools)| {
            a.sessions = sessions.len() as u32;
            a.daily = daily.into_iter().collect();
            a.tools = tools.into_iter().collect();
            a
        })
        .collect();
    agents.sort_by(|a, b| b.invocations.cmp(&a.invocations));
    let mut pats: Vec<Pattern> = patterns
        .into_iter()
        .map(|(pattern, count)| Pattern { pattern, count })
        .collect();
    pats.sort_by(|a, b| b.count.cmp(&a.count).then(a.pattern.cmp(&b.pattern)));
    pats.truncate(10);
    let total: u32 = agents.iter().map(|a| a.invocations).sum();
    let types = agents.len() as u32;
    let multi = agents.iter().filter(|a| a.invocations > 1).count() as f64;
    let mut timeline: Vec<AgentEvent> = evs
        .iter()
        .map(|e| AgentEvent {
            ts: util::ms_to_rfc3339(e.ts),
            tool: e.tool.clone(),
            session_id: e.session.clone(),
            agent: e.agent.clone(),
        })
        .collect();
    timeline.sort_by(|a, b| b.ts.cmp(&a.ts));
    timeline.truncate(1000);
    let mut subs = Vec::new();
    let (ws, ps) = subagent_where(f);
    if let Ok(mut st) = idx.conn().prepare(&format!(
        "SELECT s.tool, COUNT(*) FROM sessions s WHERE s.parent_session IS NOT NULL AND s.parent_session <> '' {ws} GROUP BY s.tool"
    )) {
        if let Ok(rows) = st.query_map(params_from_iter(ps.iter()), |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)? as u32))) {
            subs = rows.flatten().collect();
        }
    }
    AgentsReport {
        top_agent: agents.first().map(|a| a.name.clone()),
        avg_invocations_per_type: if types > 0 {
            total as f64 / types as f64
        } else {
            0.0
        },
        adoption_rate: if types > 0 {
            multi * 100.0 / types as f64
        } else {
            0.0
        },
        agents,
        patterns: pats,
        timeline,
        popular_hours: popular,
        by_day: by_day.into_iter().collect(),
        by_tool: by_tool.into_iter().collect(),
        subagent_sessions: subs,
        total_invocations: total,
        agent_types: types,
    }
}

fn subagent_where(f: &ListFilter) -> (String, Vec<String>) {
    let mut w = String::new();
    let mut p = Vec::new();
    if let Some(t) = f.tool.as_ref().filter(|s| !s.is_empty()) {
        w.push_str(" AND s.tool = ?");
        p.push(t.clone());
    }
    if let Some(from) = f.from.as_deref().and_then(index::day_start_ms) {
        w.push_str(" AND s.ended_ms >= ?");
        p.push(from.to_string());
    }
    if let Some(to) = f.to.as_deref().and_then(index::day_start_ms) {
        w.push_str(" AND s.started_ms < ?");
        p.push((to + 86_400_000).to_string());
    }
    (w, p)
}

// --- time (Agent Teams + subagentes) ----------------------------------------

#[derive(Debug, Clone, Default, Serialize)]
pub struct TeamNode {
    pub id: String,
    pub name: String,
    /// `lead` | `subagent` | `teammate`.
    pub kind: String,
    pub session_id: Option<String>,
    pub spawned_by: Option<String>,
    pub spawn_call_id: Option<String>,
    pub agent_type: Option<String>,
    pub description: Option<String>,
    pub started: Option<String>,
    pub ended: Option<String>,
    pub turns: u32,
    pub tool_calls: u32,
    pub messages_sent: u32,
    pub messages_received: u32,
    pub tools_used: Vec<(String, u32)>,
    pub usage: TokenUsage,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct TeamEdge {
    pub from: String,
    pub to: String,
    /// `spawn` | `message`.
    pub kind: String,
    pub count: u32,
    pub samples: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct TeamTask {
    pub id: String,
    pub subject: Option<String>,
    pub description: Option<String>,
    pub status: Option<String>,
    pub owner: Option<String>,
    pub created_by: Option<String>,
    pub history: Vec<(String, String)>,
}

#[derive(Debug, Clone, Serialize)]
pub struct TeamMessage {
    pub ts: String,
    pub from: String,
    pub to: String,
    pub text: String,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct TeamGraph {
    pub tool: String,
    pub session_id: String,
    /// Houve `TeamCreate`/`SendMessage`/`<teammate-message>` (Agent Teams).
    pub is_team: bool,
    pub nodes: Vec<TeamNode>,
    pub edges: Vec<TeamEdge>,
    pub tasks: Vec<TeamTask>,
    pub messages: Vec<TeamMessage>,
}

/// Filho do líder: a sessão e, quando a ferramenta grava, o tool call que o
/// criou (`toolUseId` do `.meta.json` do Claude).
pub struct Child {
    pub session: Session,
    pub spawn_call_id: Option<String>,
    pub agent_type: Option<String>,
    pub description: Option<String>,
}

fn attr(text: &str, name: &str) -> Option<String> {
    let key = format!("{name}=\"");
    let i = text.find(&key)? + key.len();
    let j = text[i..].find('"')? + i;
    Some(text[i..j].to_string())
}

/// `<teammate-message teammate_id="x" ...>corpo</teammate-message>`.
fn teammate_messages(text: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut rest = text;
    while let Some(i) = rest.find("<teammate-message") {
        let after = &rest[i..];
        let Some(close) = after.find('>') else { break };
        let head = &after[..close];
        let from = attr(head, "teammate_id").unwrap_or_else(|| "?".into());
        let body_start = close + 1;
        let end = after.find("</teammate-message>").unwrap_or(after.len());
        let body = after.get(body_start..end).unwrap_or("").trim().to_string();
        out.push((from, body));
        rest = &after[end.min(after.len())..];
        if end == after.len() {
            break;
        }
        rest = &rest["</teammate-message>".len().min(rest.len())..];
    }
    out
}

fn task_id_from(result: Option<&str>) -> Option<String> {
    let r = result?;
    let i = r.find('#')?;
    let digits: String = r[i + 1..]
        .chars()
        .take_while(|c| c.is_ascii_digit())
        .collect();
    (!digits.is_empty()).then_some(digits)
}

pub fn team(lead: &Session, children: Vec<Child>) -> TeamGraph {
    let mut g = TeamGraph {
        tool: lead.meta.tool.clone(),
        session_id: lead.meta.id.clone(),
        ..Default::default()
    };
    let lead_name = if lead.meta.tool == "codex" {
        "/root".to_string()
    } else {
        "lead".to_string()
    };
    let mut nodes: Vec<TeamNode> = vec![node_of(&lead.meta.id, &lead_name, "lead", Some(lead))];
    // Chamadas de Agent do líder, para casar com os filhos.
    struct Spawn {
        call_id: String,
        ts: i64,
        name: Option<String>,
        subagent: Option<String>,
        description: Option<String>,
        used: bool,
    }
    let mut spawns: Vec<Spawn> = Vec::new();
    for t in &lead.turns {
        for c in &t.tool_calls {
            if c.name_canonical == "Agent" {
                spawns.push(Spawn {
                    call_id: c.id.clone(),
                    ts: util::turn_ms(&t.ts),
                    name: util::str_of(&c.input, "name")
                        .or_else(|| util::str_of(&c.input, "task_name"))
                        .map(|s| s.to_string()),
                    subagent: c.subagent.clone(),
                    description: util::str_of(&c.input, "description").map(|s| s.to_string()),
                    used: false,
                });
            }
        }
    }
    let mut edges: BTreeMap<(String, String, String), TeamEdge> = BTreeMap::new();
    let mut add_edge = |from: &str, to: &str, kind: &str, sample: Option<String>| {
        let e = edges
            .entry((from.to_string(), to.to_string(), kind.to_string()))
            .or_insert_with(|| TeamEdge {
                from: from.to_string(),
                to: to.to_string(),
                kind: kind.to_string(),
                ..Default::default()
            });
        e.count += 1;
        if let Some(s) = sample {
            if e.samples.len() < 3 {
                e.samples.push(util::truncate_chars(&s, 200));
            }
        }
    };
    let mut sessions: Vec<(&Session, String)> = vec![(lead, lead.meta.id.clone())];
    for ch in &children {
        let start = ch
            .session
            .turns
            .iter()
            .map(|t| util::turn_ms(&t.ts))
            .find(|t| *t > 0)
            .unwrap_or(0);
        // Casamento: pelo id do tool call; senão pela janela −2 s..+120 s.
        let mut matched: Option<usize> = ch
            .spawn_call_id
            .as_ref()
            .and_then(|id| spawns.iter().position(|s| &s.call_id == id));
        if matched.is_none() && start > 0 {
            matched = spawns
                .iter()
                .enumerate()
                .filter(|(_, s)| !s.used && start - s.ts >= -2000 && start - s.ts <= 120_000)
                .min_by_key(|(_, s)| (start - s.ts).abs())
                .map(|(i, _)| i);
        }
        let mut name = ch
            .session
            .meta
            .title
            .clone()
            .unwrap_or_else(|| ch.session.meta.id.clone());
        let mut spawn_call = ch.spawn_call_id.clone();
        if let Some(i) = matched {
            spawns[i].used = true;
            if let Some(n) = spawns[i]
                .name
                .clone()
                .or_else(|| ch.agent_type.clone())
                .or_else(|| spawns[i].subagent.clone())
            {
                name = n;
            }
            spawn_call = Some(spawns[i].call_id.clone());
        } else if let Some(t) = &ch.agent_type {
            name = t.clone();
        }
        // Nome do teammate (Agent Teams) quando a mensagem traz `teammate_id`.
        if let Some(first_user) = ch.session.turns.iter().find(|t| t.role == Role::User) {
            if let Some(tid) = attr(&first_user.text, "teammate_id") {
                if matched.is_none() {
                    name = tid;
                }
            }
        }
        let mut n = node_of(&ch.session.meta.id, &name, "subagent", Some(&ch.session));
        n.spawned_by = Some(lead.meta.id.clone());
        n.spawn_call_id = spawn_call;
        n.agent_type = ch
            .agent_type
            .clone()
            .or_else(|| matched.and_then(|i| spawns[i].subagent.clone()));
        n.description = ch
            .description
            .clone()
            .or_else(|| matched.and_then(|i| spawns[i].description.clone()));
        add_edge(
            &lead.meta.id,
            &ch.session.meta.id,
            "spawn",
            n.description.clone(),
        );
        nodes.push(n);
        sessions.push((&ch.session, ch.session.meta.id.clone()));
    }
    // Chamadas de Agent sem arquivo de filho: nó sem sessão.
    for s in spawns.iter().filter(|s| !s.used) {
        let id = format!("call:{}", s.call_id);
        let mut n = node_of(
            &id,
            &s.name
                .clone()
                .or_else(|| s.subagent.clone())
                .unwrap_or_else(|| "agent".into()),
            "subagent",
            None,
        );
        n.spawned_by = Some(lead.meta.id.clone());
        n.spawn_call_id = Some(s.call_id.clone());
        n.agent_type = s.subagent.clone();
        n.description = s.description.clone();
        n.started = Some(util::ms_to_rfc3339(s.ts));
        add_edge(&lead.meta.id, &id, "spawn", s.description.clone());
        nodes.push(n);
    }
    // Mensagens e tarefas.
    let mut by_name: HashMap<String, String> = nodes
        .iter()
        .map(|n| (n.name.clone(), n.id.clone()))
        .collect();
    by_name.insert("team-lead".into(), lead.meta.id.clone());
    by_name.insert("lead".into(), lead.meta.id.clone());
    let mut extra_nodes: Vec<TeamNode> = Vec::new();
    let mut resolve = |name: &str, extra: &mut Vec<TeamNode>| -> String {
        if let Some(id) = by_name.get(name) {
            return id.clone();
        }
        let id = format!("name:{name}");
        by_name.insert(name.to_string(), id.clone());
        extra.push(node_of(&id, name, "teammate", None));
        id
    };
    let mut tasks: BTreeMap<String, TeamTask> = BTreeMap::new();
    let mut created = 0u32;
    let mut messages: Vec<TeamMessage> = Vec::new();
    for (s, node_id) in &sessions {
        for t in &s.turns {
            if t.role == Role::User {
                for (from, body) in teammate_messages(&t.text) {
                    g.is_team = true;
                    let fid = resolve(&from, &mut extra_nodes);
                    add_edge(&fid, node_id, "message", Some(body.clone()));
                    messages.push(TeamMessage {
                        ts: t.ts.clone(),
                        from: fid,
                        to: node_id.clone(),
                        text: util::truncate_chars(&body, 1000),
                    });
                }
            }
            if t.role == Role::System && t.text.starts_with("<agent-message") {
                let from = attr(&t.text, "from").unwrap_or_default();
                let to = attr(&t.text, "to").unwrap_or_default();
                let body = t
                    .text
                    .split_once('>')
                    .map(|x| x.1.trim_end_matches("</agent-message>").to_string())
                    .unwrap_or_default();
                let fid = resolve(&from, &mut extra_nodes);
                let tid = resolve(&to, &mut extra_nodes);
                add_edge(&fid, &tid, "message", Some(body.clone()));
                messages.push(TeamMessage {
                    ts: t.ts.clone(),
                    from: fid,
                    to: tid,
                    text: util::truncate_chars(&body, 1000),
                });
            }
            for c in &t.tool_calls {
                match c.name_raw.as_str() {
                    "TeamCreate" | "team_create" => g.is_team = true,
                    "SendMessage" | "send_message" | "followup_task" => {
                        if c.name_raw == "SendMessage" {
                            g.is_team = true;
                        }
                        let to = util::str_of(&c.input, "recipient")
                            .or_else(|| util::str_of(&c.input, "to"))
                            .or_else(|| util::str_of(&c.input, "target"))
                            .unwrap_or("?")
                            .to_string();
                        let body = util::str_of(&c.input, "summary")
                            .or_else(|| util::str_of(&c.input, "content"))
                            .or_else(|| util::str_of(&c.input, "message"))
                            .unwrap_or("")
                            .to_string();
                        let tid = resolve(&to, &mut extra_nodes);
                        add_edge(node_id, &tid, "message", Some(body.clone()));
                        messages.push(TeamMessage {
                            ts: t.ts.clone(),
                            from: node_id.clone(),
                            to: tid,
                            text: util::truncate_chars(&body, 1000),
                        });
                    }
                    "TaskCreate" | "task_create" => {
                        created += 1;
                        let id = task_id_from(c.result.as_deref())
                            .unwrap_or_else(|| created.to_string());
                        let task = tasks.entry(id.clone()).or_insert_with(|| TeamTask {
                            id: id.clone(),
                            ..Default::default()
                        });
                        task.subject = util::str_of(&c.input, "subject")
                            .map(|s| s.to_string())
                            .or(task.subject.take());
                        task.description = util::str_of(&c.input, "description")
                            .map(|s| util::truncate_chars(s, 500));
                        task.created_by = Some(node_id.clone());
                        task.status.get_or_insert_with(|| "pending".into());
                        task.history.push((t.ts.clone(), "created".into()));
                    }
                    "TaskUpdate" | "task_update" => {
                        let id = c
                            .input
                            .get("taskId")
                            .or_else(|| c.input.get("task_id"))
                            .or_else(|| c.input.get("id"))
                            .map(|v| {
                                v.as_str()
                                    .map(|s| s.to_string())
                                    .unwrap_or_else(|| v.to_string())
                            })
                            .unwrap_or_else(|| "?".into());
                        let task = tasks.entry(id.clone()).or_insert_with(|| TeamTask {
                            id: id.clone(),
                            ..Default::default()
                        });
                        if let Some(st) = util::str_of(&c.input, "status") {
                            task.status = Some(st.to_string());
                            task.history.push((t.ts.clone(), st.to_string()));
                        }
                        if let Some(o) = util::str_of(&c.input, "owner") {
                            task.owner = Some(o.to_string());
                        }
                        if let Some(sj) = util::str_of(&c.input, "subject") {
                            task.subject = Some(sj.to_string());
                        }
                    }
                    _ => {}
                }
            }
        }
    }
    nodes.extend(extra_nodes);
    let edges: Vec<TeamEdge> = edges.into_values().collect();
    for n in &mut nodes {
        n.messages_sent = edges
            .iter()
            .filter(|e| e.kind == "message" && e.from == n.id)
            .map(|e| e.count)
            .sum();
        n.messages_received = edges
            .iter()
            .filter(|e| e.kind == "message" && e.to == n.id)
            .map(|e| e.count)
            .sum();
    }
    messages.sort_by(|a, b| a.ts.cmp(&b.ts));
    messages.truncate(2000);
    g.nodes = nodes;
    g.edges = edges;
    g.tasks = tasks.into_values().collect();
    g.messages = messages;
    g
}

fn node_of(id: &str, name: &str, kind: &str, s: Option<&Session>) -> TeamNode {
    let mut n = TeamNode {
        id: id.to_string(),
        name: name.to_string(),
        kind: kind.to_string(),
        ..Default::default()
    };
    if let Some(s) = s {
        n.session_id = Some(s.meta.id.clone());
        n.started = s.meta.started.clone();
        n.ended = s.meta.ended.clone();
        n.turns = s.turns.len() as u32;
        let mut tools: HashMap<String, u32> = HashMap::new();
        for t in &s.turns {
            n.usage.add(&t.usage);
            for c in &t.tool_calls {
                n.tool_calls += 1;
                *tools.entry(c.name_raw.clone()).or_default() += 1;
            }
        }
        n.tools_used = sorted_counts(tools);
    }
    n
}

// --- retrospectiva ---------------------------------------------------------

#[derive(Debug, Clone, Default, Serialize)]
pub struct Ranked {
    pub name: String,
    pub count: u64,
    pub tokens: u64,
    pub pct: f64,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct Milestone {
    pub date: String,
    /// `first_session` | `sessions` | `tokens` | `new_model` | `new_tool` |
    /// `biggest_session` | `longest_session` | `longest_streak`.
    pub kind: String,
    pub label: String,
    pub value: f64,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct RetroTotals {
    pub sessions: u64,
    pub messages: u64,
    pub user_messages: u64,
    pub tool_calls: u64,
    pub usage: TokenUsage,
    pub total_tokens: u64,
    pub cost_usd: f64,
    pub active_days: u32,
    pub projects: u32,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct Retro {
    pub from: String,
    pub to: String,
    pub totals: RetroTotals,
    pub top_models: Vec<Ranked>,
    pub top_tools: Vec<Ranked>,
    pub top_projects: Vec<Ranked>,
    pub top_agents: Vec<Ranked>,
    pub top_slash_commands: Vec<Ranked>,
    pub top_skills: Vec<Ranked>,
    pub top_mcp: Vec<Ranked>,
    /// Ferramentas (claude, codex…) por sessões.
    pub by_tool: Vec<Ranked>,
    pub most_productive_day: Option<HeatDay>,
    pub peak_hour: Option<u32>,
    pub busiest_weekday: Option<u32>,
    pub current_streak: u32,
    pub longest_streak: u32,
    pub heatmap: Vec<HeatDay>,
    pub milestones: Vec<Milestone>,
}

fn ranked(idx: &SessionIndex, sql: &str, p: &[String], limit: usize) -> Vec<Ranked> {
    let mut out: Vec<Ranked> = Vec::new();
    if let Ok(mut st) = idx.conn().prepare(sql) {
        if let Ok(rows) = st.query_map(params_from_iter(p.iter()), |r| {
            Ok(Ranked {
                name: r.get::<_, Option<String>>(0)?.unwrap_or_default(),
                count: r.get::<_, i64>(1)?.max(0) as u64,
                tokens: r.get::<_, Option<i64>>(2)?.unwrap_or(0).max(0) as u64,
                pct: 0.0,
            })
        }) {
            out = rows.flatten().filter(|r| !r.name.is_empty()).collect();
        }
    }
    let total: u64 = out.iter().map(|r| r.count).sum();
    for r in &mut out {
        r.pct = if total > 0 {
            r.count as f64 * 100.0 / total as f64
        } else {
            0.0
        };
    }
    out.truncate(limit);
    out
}

pub fn retro(idx: &SessionIndex, f: &ListFilter, book: &PriceBook) -> Retro {
    let (from, to) = fill_range(f, 365);
    let mut f = f.clone();
    f.from = Some(from.format("%Y-%m-%d").to_string());
    f.to = Some(to.format("%Y-%m-%d").to_string());
    let hm = heatmap(idx, &f);
    let u = usage(idx, &f, "model", book);
    let (wu, pu) = index::usage_where(&f, "u");
    let (wc, pc) = index::usage_where(&f, "c");
    let (wm, pm) = index::usage_where(&f, "m");
    let top_models = ranked(
        idx,
        &format!(
            "SELECT u.model, COUNT(*), SUM(u.input+u.output+u.cache_read+u.cache_write+u.reasoning) AS t
             FROM usage u JOIN sessions s ON s.tool=u.tool AND s.id=u.session_id
             WHERE {wu} AND u.model <> '' AND u.model <> '<synthetic>' GROUP BY u.model ORDER BY t DESC"
        ),
        &pu,
        5,
    );
    let call_rank = |filter: &str, col: &str| {
        ranked(
            idx,
            &format!(
                "SELECT {col}, COUNT(*), 0 FROM calls c JOIN sessions s ON s.tool=c.tool AND s.id=c.session_id
                 WHERE {wc} AND {filter} GROUP BY {col} ORDER BY COUNT(*) DESC"
            ),
            &pc,
            10,
        )
    };
    let top_tools = call_rank("c.name <> 'SlashCommand'", "c.raw");
    let top_agents = call_rank("c.name = 'Agent'", "COALESCE(c.subagent, c.raw)");
    let top_slash = call_rank("c.name = 'SlashCommand'", "c.raw");
    let top_skills = call_rank("c.name = 'Skill'", "COALESCE(c.detail, c.raw)");
    let top_mcp = call_rank("c.name = 'Mcp'", "COALESCE(c.detail, c.raw)");
    let top_projects = ranked(
        idx,
        &format!(
            "SELECT s.project_path, COUNT(DISTINCT m.tool||'/'||m.session_id), 0
             FROM msgs m JOIN sessions s ON s.tool=m.tool AND s.id=m.session_id
             WHERE {wm} GROUP BY s.project_path ORDER BY 2 DESC"
        ),
        &pm,
        5,
    );
    let by_tool = ranked(
        idx,
        &format!(
            "SELECT m.tool, COUNT(DISTINCT m.session_id), 0 FROM msgs m JOIN sessions s ON s.tool=m.tool AND s.id=m.session_id
             WHERE {wm} GROUP BY m.tool ORDER BY 2 DESC"
        ),
        &pm,
        50,
    );
    let tool_calls: u64 = idx
        .conn()
        .query_row(
            &format!(
                "SELECT COUNT(*) FROM calls c JOIN sessions s ON s.tool=c.tool AND s.id=c.session_id WHERE {wc} AND c.name <> 'SlashCommand'"
            ),
            params_from_iter(pc.iter()),
            |r| r.get::<_, i64>(0),
        )
        .unwrap_or(0) as u64;
    let projects: u32 = idx
        .conn()
        .query_row(
            &format!(
                "SELECT COUNT(DISTINCT s.project_path) FROM msgs m JOIN sessions s ON s.tool=m.tool AND s.id=m.session_id WHERE {wm}"
            ),
            params_from_iter(pm.iter()),
            |r| r.get::<_, i64>(0),
        )
        .unwrap_or(0) as u32;
    let totals = RetroTotals {
        sessions: hm.stats.total_sessions as u64,
        messages: hm.days.iter().map(|d| d.messages as u64).sum(),
        user_messages: hm.days.iter().map(|d| d.user_messages as u64).sum(),
        tool_calls,
        usage: u.totals.usage.clone(),
        total_tokens: u.totals.total_tokens,
        cost_usd: u.totals.cost_usd,
        active_days: hm.stats.active_days,
        projects,
    };
    let milestones = milestones(idx, &f, &hm);
    Retro {
        from: f.from.clone().unwrap_or_default(),
        to: f.to.clone().unwrap_or_default(),
        totals,
        top_models,
        top_tools,
        top_projects,
        top_agents,
        top_slash_commands: top_slash,
        top_skills,
        top_mcp,
        by_tool,
        most_productive_day: hm
            .days
            .iter()
            .max_by_key(|d| d.messages)
            .filter(|d| d.messages > 0)
            .cloned(),
        peak_hour: hm.stats.peak_hour,
        busiest_weekday: hm.stats.peak_weekday,
        current_streak: hm.stats.current_streak,
        longest_streak: hm.stats.longest_streak,
        heatmap: hm.days,
        milestones,
    }
}

fn milestones(idx: &SessionIndex, f: &ListFilter, hm: &Heatmap) -> Vec<Milestone> {
    let mut out = Vec::new();
    let from_ms = f.from.as_deref().and_then(index::day_start_ms).unwrap_or(0);
    let to_ms =
        f.to.as_deref()
            .and_then(index::day_start_ms)
            .map(|t| t + 86_400_000)
            .unwrap_or(i64::MAX);
    let tool_clause = f.tool.as_ref().filter(|t| !t.is_empty());
    let tw = if tool_clause.is_some() {
        " AND s.tool = ?1"
    } else {
        ""
    };
    let tp: Vec<String> = tool_clause.cloned().into_iter().collect();
    // Sessões principais em ordem (todas as épocas, para os marcos cumulativos).
    let mut sess: Vec<(i64, i64, String, Option<String>, String)> = Vec::new();
    if let Ok(mut st) = idx.conn().prepare(&format!(
        "SELECT s.started_ms, COALESCE(s.ended_ms, s.started_ms), s.id, s.title, s.tool FROM sessions s
         WHERE s.started_ms IS NOT NULL AND (s.parent_session IS NULL OR s.parent_session = ''){tw} ORDER BY s.started_ms"
    )) {
        if let Ok(rows) = st.query_map(params_from_iter(tp.iter()), |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))
        }) {
            sess = rows.flatten().collect();
        }
    }
    let day = |ms: i64| util::local_day_hour(ms).0;
    if let Some((s, _, id, title, _)) = sess.iter().find(|(s, ..)| *s >= from_ms && *s < to_ms) {
        out.push(Milestone {
            date: day(*s),
            kind: "first_session".into(),
            label: title.clone().unwrap_or_else(|| id.clone()),
            value: 1.0,
        });
    }
    for (i, (s, ..)) in sess.iter().enumerate() {
        let n = (i + 1) as u64;
        if matches!(n, 10 | 50 | 100 | 250 | 500 | 1000 | 2500 | 5000 | 10000)
            && *s >= from_ms
            && *s < to_ms
        {
            out.push(Milestone {
                date: day(*s),
                kind: "sessions".into(),
                label: format!("{n}"),
                value: n as f64,
            });
        }
    }
    let longest = sess
        .iter()
        .filter(|(s, ..)| *s >= from_ms && *s < to_ms)
        .max_by_key(|(s, e, ..)| e - s);
    if let Some((s, e, id, title, _)) = longest {
        out.push(Milestone {
            date: day(*s),
            kind: "longest_session".into(),
            label: title.clone().unwrap_or_else(|| id.clone()),
            value: (e - s) as f64,
        });
    }
    // Tokens cumulativos por dia (todas as épocas).
    let mut cum = 0u64;
    let thresholds = [
        1_000_000u64,
        10_000_000,
        100_000_000,
        1_000_000_000,
        10_000_000_000,
    ];
    let (fromd, tod) = (
        f.from.clone().unwrap_or_default(),
        f.to.clone().unwrap_or_default(),
    );
    if let Ok(mut st) = idx.conn().prepare(&format!(
        "SELECT u.day, SUM(u.input+u.output+u.cache_read+u.cache_write+u.reasoning) FROM usage u
         JOIN sessions s ON s.tool=u.tool AND s.id=u.session_id WHERE u.day IS NOT NULL{tw} GROUP BY u.day ORDER BY u.day"
    )) {
        if let Ok(rows) = st.query_map(params_from_iter(tp.iter()), |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)? as u64))) {
            for (d, t) in rows.flatten() {
                let before = cum;
                cum += t;
                for th in thresholds {
                    if before < th && cum >= th && d >= fromd && d <= tod {
                        out.push(Milestone {
                            date: d.clone(),
                            kind: "tokens".into(),
                            label: format!("{th}"),
                            value: th as f64,
                        });
                    }
                }
            }
        }
    }
    // Primeiro uso de modelo e de ferramenta dentro do período.
    for (col, kind) in [("u.model", "new_model"), ("u.tool", "new_tool")] {
        if let Ok(mut st) = idx.conn().prepare(&format!(
            "SELECT {col}, MIN(u.day) FROM usage u JOIN sessions s ON s.tool=u.tool AND s.id=u.session_id
             WHERE {col} <> '' AND {col} <> '<synthetic>'{tw} GROUP BY {col}"
        )) {
            if let Ok(rows) = st.query_map(params_from_iter(tp.iter()), |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?.unwrap_or_default()))
            }) {
                for (name, first) in rows.flatten() {
                    if first >= fromd && first <= tod {
                        out.push(Milestone {
                            date: first,
                            kind: kind.into(),
                            label: name,
                            value: 1.0,
                        });
                    }
                }
            }
        }
    }
    // Maior sessão (tokens) do período.
    let (wu, pu) = index::usage_where(f, "u");
    if let Ok(mut st) = idx.conn().prepare(&format!(
        "SELECT u.tool, u.session_id, SUM(u.input+u.output+u.cache_read+u.cache_write+u.reasoning) AS t, MIN(u.day), s.title
         FROM usage u JOIN sessions s ON s.tool=u.tool AND s.id=u.session_id WHERE {wu}
         GROUP BY u.tool, u.session_id ORDER BY t DESC LIMIT 1"
    )) {
        if let Ok(Some(row)) = st
            .query_row(params_from_iter(pu.iter()), |r| {
                Ok((
                    r.get::<_, String>(1)?,
                    r.get::<_, i64>(2)?,
                    r.get::<_, Option<String>>(3)?.unwrap_or_default(),
                    r.get::<_, Option<String>>(4)?,
                ))
            })
            .map(Some)
        {
            out.push(Milestone {
                date: row.2,
                kind: "biggest_session".into(),
                label: row.3.unwrap_or(row.0),
                value: row.1 as f64,
            });
        }
    }
    if hm.stats.longest_streak > 1 {
        if let Some(start) = &hm.stats.longest_streak_start {
            out.push(Milestone {
                date: start.clone(),
                kind: "longest_streak".into(),
                label: hm.stats.longest_streak_end.clone().unwrap_or_default(),
                value: hm.stats.longest_streak as f64,
            });
        }
    }
    out.sort_by(|a, b| a.date.cmp(&b.date).then(a.kind.cmp(&b.kind)));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::sessions::model::ToolCall;
    use std::path::PathBuf;

    fn t(role: Role, ts: &str, text: &str) -> crate::core::sessions::model::Turn {
        crate::core::sessions::model::Turn {
            role,
            ts: ts.into(),
            text: text.into(),
            tool_calls: Vec::new(),
            usage: TokenUsage::default(),
            cost_usd: None,
            model: None,
            message_id: None,
        }
    }

    fn meta() -> SessionMeta {
        SessionMeta {
            tool: "claude".into(),
            account: None,
            id: "s".into(),
            title: None,
            project_path: None,
            git_branch: None,
            started: None,
            ended: None,
            models: Vec::new(),
            parent_session: None,
            turn_count: 0,
            usage: TokenUsage::default(),
            cost_usd: 0.0,
            source: PathBuf::new(),
            mtime_ms: 0,
        }
    }

    #[test]
    fn wait_and_think_time_split_by_exchanges() {
        let mut a1 = t(Role::Assistant, "2026-09-20T10:00:10Z", "");
        a1.usage.cache_read = 10;
        a1.usage.cache_write = 90;
        a1.model = Some("m1".into());
        a1.tool_calls.push(ToolCall {
            id: "x".into(),
            name_canonical: "Agent".into(),
            name_raw: "Task".into(),
            input: serde_json::json!({"subagent_type": "Explore"}),
            result: None,
            status: ToolStatus::Error,
            ms: Some(100),
            subagent: Some("Explore".into()),
        });
        let mut a2 = t(Role::Assistant, "2026-09-20T10:01:00Z", "");
        a2.model = Some("m2".into());
        let s = Session {
            meta: meta(),
            turns: vec![
                t(
                    Role::User,
                    "2026-09-20T10:00:00Z",
                    "<command-name>/review</command-name>",
                ),
                a1,
                t(Role::User, "2026-09-20T10:00:40Z", "e agora?"),
                a2,
                t(Role::User, "2026-09-20T12:00:00Z", "voltei"),
            ],
        };
        let a = analyze(&s, &PriceBook::empty(), Some(1.5));
        assert_eq!(a.time.agent_ms, 30_000);
        assert_eq!(a.time.think_ms, 30_000);
        assert!(a.time.away_ms > 3_600_000);
        assert_eq!(a.components.agents, vec![("Explore".to_string(), 1)]);
        assert_eq!(
            a.components.slash_commands,
            vec![("/review".to_string(), 1)]
        );
        assert!((a.cost.cost_usd - 1.5).abs() < 1e-9);
        assert_eq!(a.cost.source, "recorded");
        assert!(a.tips.iter().any(|t| t.code == "low_cache"));
        assert!(a.tips.iter().any(|t| t.code == "multiple_models"));
    }

    #[test]
    fn streaks_and_teammate_messages() {
        let d = |date: &str, m: u32| HeatDay {
            date: date.into(),
            messages: m,
            ..Default::default()
        };
        let days = vec![
            d("2026-01-01", 1),
            d("2026-01-02", 3),
            d("2026-01-03", 0),
            d("2026-01-04", 2),
        ];
        let (cur, longest, s, e) = streaks(&days);
        assert_eq!(longest, 2);
        assert_eq!(cur, 1);
        assert_eq!(s.as_deref(), Some("2026-01-01"));
        assert_eq!(e.as_deref(), Some("2026-01-02"));
        let msgs = teammate_messages(
            r#"<teammate-message teammate_id="researcher" color="blue">{"x":1}</teammate-message> e <teammate-message teammate_id="lead">oi</teammate-message>"#,
        );
        assert_eq!(
            msgs,
            vec![
                ("researcher".into(), r#"{"x":1}"#.into()),
                ("lead".into(), "oi".into())
            ]
        );
        assert_eq!(task_id_from(Some("Task #12 created")), Some("12".into()));
    }
}

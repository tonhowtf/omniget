use std::sync::Arc;

use futures::StreamExt;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use tokio_util::sync::CancellationToken;

use super::error::{LlmError, ERR_LLM_BUDGET, ERR_LLM_PARSE};
use super::providers::Provider;
use super::types::{GenParams, Message, ModelRef, ProviderId, Role, TurnEvent, TurnRequest};

pub const MAX_PROBE_REQUESTS: usize = 8;
pub const MAX_OUTPUT_TOKENS: u32 = 200;
pub const PROBE_SEED: u64 = 7;

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ProbeCase {
    pub param: &'static str,
    pub a: Value,
    pub b: Value,
    pub prompt: &'static str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    Proven,
    Sent,
    Ignored,
    Unsupported,
}

impl Verdict {
    pub fn as_str(&self) -> &'static str {
        match self {
            Verdict::Proven => "proven",
            Verdict::Sent => "sent",
            Verdict::Ignored => "ignored",
            Verdict::Unsupported => "unsupported",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProbeResult {
    pub provider: String,
    pub model: String,
    pub param: String,
    pub sent_a: Value,
    pub sent_b: Value,
    pub reply_a: String,
    pub reply_b: String,
    pub differs: bool,
    pub cost_usd: f64,
    pub verdict: Verdict,
    pub detail: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ProbeEstimate {
    pub cases: usize,
    pub requests: usize,
    pub input_tokens: u32,
    pub output_tokens: u32,
    pub cost_usd: Option<f64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct TokenPrice {
    pub input_usd_per_mtok: f64,
    pub output_usd_per_mtok: f64,
}

pub fn default_cases() -> Vec<ProbeCase> {
    vec![
        ProbeCase {
            param: "temperature",
            a: Value::from(0.0),
            b: Value::from(1.5),
            prompt: "Write one short sentence about the sea.",
        },
        ProbeCase {
            param: "top_p",
            a: Value::from(1.0),
            b: Value::from(0.05),
            prompt: "Write one short sentence about the sea.",
        },
        ProbeCase {
            param: "max_tokens",
            a: Value::from(200),
            b: Value::from(8),
            prompt: "List the numbers from 1 to 40, separated by commas.",
        },
        ProbeCase {
            param: "stop",
            a: Value::Array(vec![]),
            b: Value::Array(vec![Value::from("5")]),
            prompt: "Count from 1 to 9. Only the digits, one per line, nothing else.",
        },
    ]
}

pub fn reasoning_case() -> ProbeCase {
    ProbeCase {
        param: "reasoning_effort",
        a: Value::from("low"),
        b: Value::from("high"),
        prompt: "How many r's are in strawberry? Answer with the number only.",
    }
}

pub fn estimate(cases: &[ProbeCase], price: Option<TokenPrice>) -> ProbeEstimate {
    let runs = cases.len().min(MAX_PROBE_REQUESTS / 2);
    let requests = runs * 2;
    let input_tokens: u32 = cases
        .iter()
        .take(runs)
        .map(|c| (c.prompt.len() as u32).div_ceil(4) + 8)
        .sum::<u32>()
        * 2;
    let output_tokens = MAX_OUTPUT_TOKENS * requests as u32;
    let cost_usd = price.map(|p| {
        (input_tokens as f64 * p.input_usd_per_mtok + output_tokens as f64 * p.output_usd_per_mtok)
            / 1_000_000.0
    });
    ProbeEstimate {
        cases: runs,
        requests,
        input_tokens,
        output_tokens,
        cost_usd,
    }
}

fn accepts_seed(provider: &ProviderId) -> bool {
    matches!(
        provider.as_str(),
        "openai" | "openrouter" | "ollama" | "lmstudio" | "llamacpp" | "groq" | "fake"
    )
}

fn aliases(param: &str) -> &'static [&'static str] {
    match param {
        "temperature" => &["temperature"],
        "top_p" => &["top_p", "topP"],
        "max_tokens" => &[
            "max_tokens",
            "max_completion_tokens",
            "max_output_tokens",
            "num_predict",
        ],
        "stop" => &["stop", "stop_sequences", "stop_words"],
        "reasoning_effort" => &["reasoning_effort", "reasoning", "thinking"],
        _ => &[],
    }
}

pub fn apply_param(params: &mut GenParams, param: &str, value: &Value) -> Result<(), LlmError> {
    let bad = |want: &str| {
        Err(LlmError::new(
            ERR_LLM_PARSE,
            format!("probe case `{param}` needs {want}, got {value}"),
        ))
    };
    match param {
        "temperature" => match value.as_f64() {
            Some(v) => params.temperature = Some(v as f32),
            None => return bad("a number"),
        },
        "top_p" => match value.as_f64() {
            Some(v) => params.top_p = Some(v as f32),
            None => return bad("a number"),
        },
        "max_tokens" => match value.as_u64() {
            Some(v) => params.max_tokens = Some(v.min(MAX_OUTPUT_TOKENS as u64) as u32),
            None => return bad("a whole number"),
        },
        "reasoning_effort" => match value.as_str() {
            Some(v) => params.reasoning_effort = Some(v.to_string()),
            None => return bad("a string"),
        },
        "stop" => match value.as_array() {
            Some(items) => {
                let mut out = Vec::with_capacity(items.len());
                for it in items {
                    match it.as_str() {
                        Some(s) => out.push(s.to_string()),
                        None => return bad("an array of strings"),
                    }
                }
                params.stop = out;
            }
            None => return bad("an array of strings"),
        },
        other => {
            params.extra.insert(other.to_string(), value.clone());
        }
    }
    Ok(())
}

fn find_key<'a>(
    body: &'a Value,
    keys: &[&str],
    path: &str,
    depth: u8,
) -> Option<(String, &'a Value)> {
    if depth > 6 {
        return None;
    }
    match body {
        Value::Object(map) => {
            for k in keys {
                if let Some(v) = map.get(*k) {
                    if !v.is_null() {
                        return Some((join_path(path, k), v));
                    }
                }
            }
            for (k, v) in map {
                if let Some(found) = find_key(v, keys, &join_path(path, k), depth + 1) {
                    return Some(found);
                }
            }
            None
        }
        Value::Array(items) => {
            for (i, v) in items.iter().enumerate() {
                if let Some(found) = find_key(v, keys, &format!("{path}[{i}]"), depth + 1) {
                    return Some(found);
                }
            }
            None
        }
        _ => None,
    }
}

fn join_path(path: &str, key: &str) -> String {
    if path.is_empty() {
        key.to_string()
    } else {
        format!("{path}.{key}")
    }
}

pub fn values_match(sent: &Value, asked: &Value) -> bool {
    match (sent, asked) {
        (Value::Number(a), Value::Number(b)) => match (a.as_f64(), b.as_f64()) {
            (Some(x), Some(y)) => (x - y).abs() <= 1e-6 * y.abs().max(1.0),
            _ => a == b,
        },
        (Value::Array(a), Value::Array(b)) if a.len() == b.len() => {
            a.iter().zip(b).all(|(x, y)| values_match(x, y))
        }
        (Value::String(_), Value::Array(b)) if b.len() == 1 => values_match(sent, &b[0]),
        (Value::Array(a), _) if !asked.is_array() => a.iter().any(|x| values_match(x, asked)),
        (Value::Object(map), _) if !asked.is_object() => {
            map.values().any(|v| values_match(v, asked))
        }
        _ => sent == asked,
    }
}

fn requires_presence(v: &Value) -> bool {
    match v {
        Value::Null => false,
        Value::Array(a) => !a.is_empty(),
        Value::String(s) => !s.is_empty(),
        _ => true,
    }
}

fn normalize_reply(s: &str) -> String {
    s.lines()
        .map(|l| l.trim_end())
        .collect::<Vec<_>>()
        .join("\n")
        .trim()
        .to_string()
}

struct TurnOutcome {
    sent: Value,
    reply: String,
    cost_usd: f64,
    error: Option<LlmError>,
}

async fn one_turn(
    provider: &Arc<dyn Provider>,
    model: &ModelRef,
    case: &ProbeCase,
    value: &Value,
) -> TurnOutcome {
    let mut params = GenParams {
        max_tokens: Some(MAX_OUTPUT_TOKENS),
        ..GenParams::default()
    };
    if accepts_seed(&model.provider) {
        params
            .extra
            .insert("seed".to_string(), Value::from(PROBE_SEED));
    }
    if let Err(e) = apply_param(&mut params, case.param, value) {
        return TurnOutcome {
            sent: Value::Null,
            reply: String::new(),
            cost_usd: 0.0,
            error: Some(e),
        };
    }

    let req = TurnRequest {
        model: model.clone(),
        messages: vec![Message::text(Role::User, case.prompt)],
        tools: vec![],
        params,
        cancel: CancellationToken::new(),
        agent_id: None,
    };

    let mut reply = String::new();
    let mut cost_usd = 0.0;
    let mut error = None;
    match provider.turn(req).await {
        Ok(mut stream) => {
            while let Some(ev) = stream.next().await {
                match ev {
                    TurnEvent::TextDelta { text } => reply.push_str(&text),
                    TurnEvent::Usage { usage } => cost_usd += usage.cost_usd.unwrap_or(0.0),
                    TurnEvent::Error { error: e } => error = Some(e),
                    _ => {}
                }
            }
        }
        Err(e) => error = Some(e),
    }

    let sent = provider
        .wire_capture()
        .and_then(|c| c.snapshot().sent_body)
        .unwrap_or(Value::Null);

    TurnOutcome {
        sent,
        reply,
        cost_usd,
        error,
    }
}

fn judge(case: &ProbeCase, a: &TurnOutcome, b: &TurnOutcome, differs: bool) -> (Verdict, String) {
    if let Some(e) = a.error.as_ref().or(b.error.as_ref()) {
        return (Verdict::Unsupported, format!("{}: {}", e.code, e.message));
    }
    if a.sent.is_null() || b.sent.is_null() {
        return (
            Verdict::Unsupported,
            "provider exposes no wire capture".to_string(),
        );
    }

    let keys: Vec<&str> = {
        let named = aliases(case.param);
        if named.is_empty() {
            vec![case.param]
        } else {
            named.to_vec()
        }
    };

    let mut where_found = String::new();
    for (outcome, asked) in [(a, &case.a), (b, &case.b)] {
        match find_key(&outcome.sent, &keys, "", 0) {
            Some((path, got)) => {
                if !values_match(got, asked) {
                    if requires_presence(asked) {
                        return (
                            Verdict::Ignored,
                            format!("`{path}` carried {got}, not {asked}"),
                        );
                    }
                } else if where_found.is_empty() {
                    where_found = path;
                }
            }
            None => {
                if requires_presence(asked) {
                    return (
                        Verdict::Ignored,
                        format!("`{}` is not in the body sent", case.param),
                    );
                }
            }
        }
    }

    if where_found.is_empty() {
        where_found = case.param.to_string();
    }
    if differs {
        (Verdict::Proven, where_found)
    } else {
        (Verdict::Sent, format!("{where_found}; reply unchanged"))
    }
}

pub async fn run(
    provider: Arc<dyn Provider>,
    model: &ModelRef,
    cases: &[ProbeCase],
) -> Vec<ProbeResult> {
    let allowed = MAX_PROBE_REQUESTS / 2;
    let mut out = Vec::with_capacity(cases.len());
    for (idx, case) in cases.iter().enumerate() {
        if idx >= allowed {
            out.push(ProbeResult {
                provider: model.provider.as_str().to_string(),
                model: model.model.clone(),
                param: case.param.to_string(),
                sent_a: Value::Null,
                sent_b: Value::Null,
                reply_a: String::new(),
                reply_b: String::new(),
                differs: false,
                cost_usd: 0.0,
                verdict: Verdict::Unsupported,
                detail: format!("{ERR_LLM_BUDGET}: over {MAX_PROBE_REQUESTS} requests per round"),
            });
            continue;
        }

        let a = one_turn(&provider, model, case, &case.a).await;
        let b = one_turn(&provider, model, case, &case.b).await;
        let differs = normalize_reply(&a.reply) != normalize_reply(&b.reply);
        let (verdict, detail) = judge(case, &a, &b, differs);
        out.push(ProbeResult {
            provider: model.provider.as_str().to_string(),
            model: model.model.clone(),
            param: case.param.to_string(),
            sent_a: a.sent,
            sent_b: b.sent,
            reply_a: a.reply,
            reply_b: b.reply,
            differs,
            cost_usd: a.cost_usd + b.cost_usd,
            verdict,
            detail,
        });
    }
    out
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProbeRound {
    pub provider: String,
    pub model: String,
    pub started_at_ms: u64,
    pub results: Vec<ProbeResult>,
    pub cost_usd: f64,
}

impl ProbeRound {
    pub fn new(model: &ModelRef, started_at_ms: u64, results: Vec<ProbeResult>) -> Self {
        let cost_usd = results.iter().map(|r| r.cost_usd).sum();
        Self {
            provider: model.provider.as_str().to_string(),
            model: model.model.clone(),
            started_at_ms,
            results,
            cost_usd,
        }
    }

    pub fn count(&self, verdict: Verdict) -> usize {
        self.results.iter().filter(|r| r.verdict == verdict).count()
    }
}

pub fn redact(body: &Value) -> Value {
    const SECRET_KEYS: &[&str] = &[
        "authorization",
        "api_key",
        "api-key",
        "x-api-key",
        "key",
        "token",
        "access_token",
        "password",
        "cookie",
    ];
    match body {
        Value::Object(map) => {
            let mut out = Map::with_capacity(map.len());
            for (k, v) in map {
                if SECRET_KEYS.contains(&k.to_ascii_lowercase().as_str()) {
                    out.insert(k.clone(), Value::from("[redacted]"));
                } else {
                    out.insert(k.clone(), redact(v));
                }
            }
            Value::Object(out)
        }
        Value::Array(items) => Value::Array(items.iter().map(redact).collect()),
        _ => body.clone(),
    }
}

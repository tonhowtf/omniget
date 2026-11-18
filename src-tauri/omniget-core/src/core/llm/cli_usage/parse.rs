use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CliKind {
    #[default]
    Claude,
    Codex,
}

impl CliKind {
    pub fn as_str(self) -> &'static str {
        match self {
            CliKind::Claude => "claude",
            CliKind::Codex => "codex",
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct CliUsageEntry {
    pub cli: CliKind,
    pub account: String,
    pub session_id: String,
    pub project: String,
    pub ts_ms: i64,
    pub model: String,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_write_tokens: u64,
    pub reasoning_tokens: u64,
    pub cost_usd: Option<f64>,
    pub dedupe_key: Option<String>,
    pub sidechain: bool,
    pub tools: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RateLimitSample {
    pub window_minutes: u32,
    pub used_percent: f32,
    pub resets_at: Option<i64>,
    pub observed_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Parsed {
    Usage(Box<CliUsageEntry>),
    Limits(Vec<RateLimitSample>),
    Ignored,
}

#[derive(Debug, Clone)]
pub struct FileParser {
    cli: CliKind,
    account: String,
    project: String,
    session_id: String,
    model: String,
}

impl FileParser {
    pub fn new(
        cli: CliKind,
        account: impl Into<String>,
        project: impl Into<String>,
        session_id: impl Into<String>,
    ) -> Self {
        Self {
            cli,
            account: account.into(),
            project: project.into(),
            session_id: session_id.into(),
            model: String::new(),
        }
    }

    pub fn feed(&mut self, line: &str) -> Parsed {
        let line = line.trim();
        if line.len() < 2 || !may_carry_usage(self.cli, line.as_bytes()) {
            return Parsed::Ignored;
        }
        let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else {
            return Parsed::Ignored;
        };
        match self.cli {
            CliKind::Claude => self.feed_claude(&v),
            CliKind::Codex => self.feed_codex(&v),
        }
    }

    fn feed_claude(&mut self, v: &serde_json::Value) -> Parsed {
        match v.get("type").and_then(|t| t.as_str()) {
            Some("assistant") => {
                if v.get("isApiErrorMessage").and_then(|b| b.as_bool()) == Some(true) {
                    return Parsed::Ignored;
                }
                let msg = match v.get("message") {
                    Some(m) => m,
                    None => return Parsed::Ignored,
                };
                let usage = match msg.get("usage") {
                    Some(u) => u,
                    None => return Parsed::Ignored,
                };
                let model = msg.get("model").and_then(|m| m.as_str()).unwrap_or("");
                if model.is_empty() || model == "<synthetic>" {
                    return Parsed::Ignored;
                }
                let fresh = num(usage, "input_tokens");
                let cache_write = num(usage, "cache_creation_input_tokens");
                let cache_read = num(usage, "cache_read_input_tokens");
                let output = num(usage, "output_tokens");
                if fresh + cache_write + cache_read + output == 0 {
                    return Parsed::Ignored;
                }
                if let Some(sid) = v.get("sessionId").and_then(|s| s.as_str()) {
                    if !sid.is_empty() {
                        self.session_id = sid.to_string();
                    }
                }
                let reasoning = usage
                    .get("output_tokens_details")
                    .map(|d| num(d, "thinking_tokens"))
                    .unwrap_or(0);
                let dedupe = match (
                    msg.get("id").and_then(|i| i.as_str()),
                    v.get("requestId").and_then(|i| i.as_str()),
                ) {
                    (Some(id), Some(req)) => Some(format!("{id}:{req}")),
                    (Some(id), None) => Some(id.to_string()),
                    _ => None,
                };
                let mut tools = Vec::new();
                if let Some(blocks) = msg.get("content").and_then(|c| c.as_array()) {
                    for b in blocks {
                        if b.get("type").and_then(|t| t.as_str()) == Some("tool_use") {
                            if let Some(name) = b.get("name").and_then(|n| n.as_str()) {
                                tools.push(name.to_string());
                            }
                        }
                    }
                }
                Parsed::Usage(Box::new(CliUsageEntry {
                    cli: CliKind::Claude,
                    account: self.account.clone(),
                    session_id: self.session_id.clone(),
                    project: self.project.clone(),
                    ts_ms: ts_ms(v.get("timestamp")).unwrap_or(0),
                    model: model.to_string(),
                    input_tokens: fresh + cache_write + cache_read,
                    output_tokens: output,
                    cache_read_tokens: cache_read,
                    cache_write_tokens: cache_write,
                    reasoning_tokens: reasoning,
                    cost_usd: None,
                    dedupe_key: dedupe,
                    sidechain: v
                        .get("isSidechain")
                        .and_then(|b| b.as_bool())
                        .unwrap_or(false),
                    tools,
                }))
            }
            Some("rate_limit_event") => {
                let obs = ts_ms(v.get("timestamp")).unwrap_or(0);
                let windows = v
                    .get("unifiedWindows")
                    .or_else(|| v.get("unified_windows"))
                    .or_else(|| v.get("rate_limits"));
                let Some(windows) = windows else {
                    return Parsed::Ignored;
                };
                let mut out = Vec::new();
                for (key, minutes) in [("five_hour", 300u32), ("seven_day", 10080u32)] {
                    if let Some(w) = windows.get(key) {
                        if let Some(used) = window_percent(w) {
                            out.push(RateLimitSample {
                                window_minutes: minutes,
                                used_percent: used,
                                resets_at: w
                                    .get("resetsAt")
                                    .or_else(|| w.get("resets_at"))
                                    .and_then(epoch_secs),
                                observed_at_ms: obs,
                            });
                        }
                    }
                }
                if out.is_empty() {
                    Parsed::Ignored
                } else {
                    Parsed::Limits(out)
                }
            }
            _ => Parsed::Ignored,
        }
    }

    fn feed_codex(&mut self, v: &serde_json::Value) -> Parsed {
        let kind = v.get("type").and_then(|t| t.as_str()).unwrap_or("");
        let payload = v.get("payload");
        match kind {
            "session_meta" => {
                if let Some(p) = payload {
                    if let Some(sid) = p.get("session_id").and_then(|s| s.as_str()) {
                        self.session_id = sid.to_string();
                    }
                }
                Parsed::Ignored
            }
            "turn_context" => {
                if let Some(m) = payload
                    .and_then(|p| p.get("model"))
                    .and_then(|m| m.as_str())
                {
                    self.model = m.to_string();
                }
                Parsed::Ignored
            }
            "token_usage_record" => {
                let Some(p) = payload else {
                    return Parsed::Ignored;
                };
                let Some(u) = p.get("usage") else {
                    return Parsed::Ignored;
                };
                let input = num(u, "input_tokens");
                let output = num(u, "output_tokens");
                if input + output == 0 {
                    return Parsed::Ignored;
                }
                if let Some(sid) = p.get("session_id").and_then(|s| s.as_str()) {
                    if !sid.is_empty() {
                        self.session_id = sid.to_string();
                    }
                }
                Parsed::Usage(Box::new(CliUsageEntry {
                    cli: CliKind::Codex,
                    account: self.account.clone(),
                    session_id: self.session_id.clone(),
                    project: self.project.clone(),
                    ts_ms: ts_ms(v.get("timestamp")).unwrap_or(0),
                    model: self.model.clone(),
                    input_tokens: input,
                    output_tokens: output,
                    cache_read_tokens: num(u, "cached_input_tokens"),
                    cache_write_tokens: num(u, "cache_write_input_tokens"),
                    reasoning_tokens: num(u, "reasoning_output_tokens"),
                    cost_usd: None,
                    dedupe_key: p
                        .get("response_id")
                        .and_then(|s| s.as_str())
                        .map(|s| s.to_string()),
                    sidechain: false,
                    tools: Vec::new(),
                }))
            }
            "event_msg" => {
                let Some(p) = payload else {
                    return Parsed::Ignored;
                };
                if p.get("type").and_then(|t| t.as_str()) != Some("token_count") {
                    return Parsed::Ignored;
                }
                let Some(rl) = p.get("rate_limits") else {
                    return Parsed::Ignored;
                };
                let obs = ts_ms(v.get("timestamp")).unwrap_or(0);
                let mut out = Vec::new();
                for key in ["primary", "secondary"] {
                    let Some(w) = rl.get(key) else { continue };
                    let Some(used) = window_percent(w) else {
                        continue;
                    };
                    let minutes = w
                        .get("window_minutes")
                        .and_then(|m| m.as_u64())
                        .unwrap_or(if key == "primary" { 300 } else { 10080 })
                        as u32;
                    out.push(RateLimitSample {
                        window_minutes: minutes,
                        used_percent: used,
                        resets_at: w.get("resets_at").and_then(epoch_secs),
                        observed_at_ms: obs,
                    });
                }
                if out.is_empty() {
                    Parsed::Ignored
                } else {
                    Parsed::Limits(out)
                }
            }
            _ => Parsed::Ignored,
        }
    }
}

pub fn may_carry_usage(cli: CliKind, line: &[u8]) -> bool {
    match cli {
        CliKind::Claude => contains(line, b"\"usage\"") || contains(line, b"\"rate_limit"),
        CliKind::Codex => {
            contains(line, b"\"usage\"")
                || contains(line, b"\"rate_limits\"")
                || contains(line, b"\"turn_context\"")
                || contains(line, b"\"session_meta\"")
        }
    }
}

pub fn may_be_tool_call(line: &[u8]) -> bool {
    contains(line, b"_tool_call\"") || contains(line, b"function_call\"")
}

fn contains(hay: &[u8], needle: &[u8]) -> bool {
    if needle.is_empty() || needle.len() > hay.len() {
        return false;
    }
    let first = needle[0];
    let last = hay.len() - needle.len() + 1;
    let mut i = 0;
    while i < last {
        match hay[i..last].iter().position(|&b| b == first) {
            Some(p) => {
                let s = i + p;
                if &hay[s..s + needle.len()] == needle {
                    return true;
                }
                i = s + 1;
            }
            None => return false,
        }
    }
    false
}

pub fn codex_tool_name(line: &str) -> Option<String> {
    if !may_be_tool_call(line.as_bytes()) {
        return None;
    }
    let v: serde_json::Value = serde_json::from_str(line.trim()).ok()?;
    let p = v.get("payload")?;
    match p.get("type").and_then(|t| t.as_str()) {
        Some("custom_tool_call") | Some("function_call") | Some("local_shell_call") => Some(
            p.get("name")
                .and_then(|n| n.as_str())
                .unwrap_or("exec")
                .to_string(),
        ),
        _ => None,
    }
}

fn num(v: &serde_json::Value, key: &str) -> u64 {
    v.get(key).and_then(|x| x.as_u64()).unwrap_or(0)
}

fn window_percent(w: &serde_json::Value) -> Option<f32> {
    for key in ["used_percent", "used_percentage", "usedPercent"] {
        if let Some(x) = w.get(key).and_then(|x| x.as_f64()) {
            return Some(x.clamp(0.0, 100.0) as f32);
        }
    }
    for key in ["utilization", "used_fraction"] {
        if let Some(x) = w.get(key).and_then(|x| x.as_f64()) {
            return Some((x.clamp(0.0, 1.0) * 100.0) as f32);
        }
    }
    None
}

fn epoch_secs(v: &serde_json::Value) -> Option<i64> {
    if let Some(n) = v.as_i64() {
        return Some(if n > 1_000_000_000_000 { n / 1000 } else { n });
    }
    if let Some(f) = v.as_f64() {
        return Some(f as i64);
    }
    let s = v.as_str()?;
    chrono::DateTime::parse_from_rfc3339(s)
        .ok()
        .map(|t| t.timestamp())
}

pub fn ts_ms(v: Option<&serde_json::Value>) -> Option<i64> {
    let v = v?;
    if let Some(n) = v.as_i64() {
        return Some(if n < 100_000_000_000 { n * 1000 } else { n });
    }
    let s = v.as_str()?;
    chrono::DateTime::parse_from_rfc3339(s)
        .ok()
        .map(|t| t.timestamp_millis())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn claude() -> FileParser {
        FileParser::new(CliKind::Claude, "default", "proj", "")
    }

    const ASSISTANT: &str = r#"{"type":"assistant","isSidechain":false,"sessionId":"s1","timestamp":"2026-03-04T12:00:09.000Z","requestId":"req_1","message":{"id":"msg_1","role":"assistant","model":"claude-opus-5","content":[{"type":"text","text":"hi"},{"type":"tool_use","id":"t1","name":"Bash","input":{}},{"type":"tool_use","id":"t2","name":"Read","input":{}}],"usage":{"input_tokens":12,"cache_creation_input_tokens":20000,"cache_read_input_tokens":30000,"output_tokens":300,"output_tokens_details":{"thinking_tokens":120}}}}"#;

    #[test]
    fn claude_assistant_line_becomes_an_entry() {
        let Parsed::Usage(e) = claude().feed(ASSISTANT) else {
            panic!("expected usage");
        };
        assert_eq!(e.model, "claude-opus-5");
        assert_eq!(e.input_tokens, 50_012);
        assert_eq!(e.cache_read_tokens, 30_000);
        assert_eq!(e.cache_write_tokens, 20_000);
        assert_eq!(e.output_tokens, 300);
        assert_eq!(e.reasoning_tokens, 120);
        assert_eq!(e.session_id, "s1");
        assert_eq!(e.tools, vec!["Bash".to_string(), "Read".to_string()]);
        assert_eq!(e.dedupe_key.as_deref(), Some("msg_1:req_1"));
        assert!(!e.sidechain);
    }

    #[test]
    fn unknown_and_malformed_lines_are_skipped_not_fatal() {
        let mut p = claude();
        assert_eq!(
            p.feed("{\"type\":\"some-future-record\",\"usage\":1}"),
            Parsed::Ignored
        );
        assert_eq!(p.feed("{not json at all"), Parsed::Ignored);
        assert_eq!(p.feed(""), Parsed::Ignored);
        assert_eq!(
            p.feed("{\"type\":\"mode\",\"mode\":\"normal\"}"),
            Parsed::Ignored
        );
    }

    #[test]
    fn api_error_and_synthetic_model_lines_are_not_billed() {
        let mut p = claude();
        let err = r#"{"type":"assistant","isApiErrorMessage":true,"timestamp":"2026-03-04T12:00:00.000Z","message":{"id":"m","model":"<synthetic>","usage":{"input_tokens":0,"output_tokens":0}}}"#;
        assert_eq!(p.feed(err), Parsed::Ignored);
        let syn = r#"{"type":"assistant","timestamp":"2026-03-04T12:00:00.000Z","message":{"id":"m","model":"<synthetic>","usage":{"input_tokens":5,"output_tokens":5}}}"#;
        assert_eq!(p.feed(syn), Parsed::Ignored);
    }

    #[test]
    fn sidechain_turns_are_flagged() {
        let line = ASSISTANT.replace("\"isSidechain\":false", "\"isSidechain\":true");
        let Parsed::Usage(e) = claude().feed(&line) else {
            panic!("expected usage");
        };
        assert!(e.sidechain);
    }

    #[test]
    fn claude_rate_limit_event_is_normalised_to_percent() {
        let line = r#"{"type":"rate_limit_event","timestamp":"2026-03-04T12:00:00.000Z","unifiedWindows":{"five_hour":{"utilization":0.25,"resetsAt":1772629200},"seven_day":{"utilization":0.5,"resetsAt":1773061200}}}"#;
        let Parsed::Limits(l) = claude().feed(line) else {
            panic!("expected limits");
        };
        assert_eq!(l.len(), 2);
        assert_eq!(l[0].window_minutes, 300);
        assert!((l[0].used_percent - 25.0).abs() < 1e-3);
        assert_eq!(l[1].resets_at, Some(1_773_061_200));
    }

    #[test]
    fn codex_usage_record_and_model_from_turn_context() {
        let mut p = FileParser::new(CliKind::Codex, "default", "proj", "");
        assert_eq!(
            p.feed(r#"{"type":"turn_context","payload":{"model":"gpt-6-astra"}}"#),
            Parsed::Ignored
        );
        let line = r#"{"timestamp":"2026-03-04T12:00:21.000Z","type":"token_usage_record","payload":{"session_id":"cs1","response_id":"resp_1","usage":{"input_tokens":25207,"cached_input_tokens":18560,"cache_write_input_tokens":0,"output_tokens":171,"reasoning_output_tokens":64}}}"#;
        let Parsed::Usage(e) = p.feed(line) else {
            panic!("expected usage");
        };
        assert_eq!(e.model, "gpt-6-astra");
        assert_eq!(e.cli, CliKind::Codex);
        assert_eq!(e.input_tokens, 25_207);
        assert_eq!(e.cache_read_tokens, 18_560);
        assert_eq!(e.reasoning_tokens, 64);
        assert_eq!(e.session_id, "cs1");
        assert_eq!(e.dedupe_key.as_deref(), Some("resp_1"));
    }

    #[test]
    fn codex_token_count_carries_both_official_windows() {
        let mut p = FileParser::new(CliKind::Codex, "default", "proj", "");
        let line = r#"{"timestamp":"2026-03-04T12:00:22.000Z","type":"event_msg","payload":{"type":"token_count","rate_limits":{"primary":{"used_percent":12.5,"window_minutes":300,"resets_at":1772629200},"secondary":{"used_percent":41.0,"window_minutes":10080,"resets_at":1773061200}}}}"#;
        let Parsed::Limits(l) = p.feed(line) else {
            panic!("expected limits");
        };
        assert_eq!(l.len(), 2);
        assert_eq!(l[0].window_minutes, 300);
        assert!((l[0].used_percent - 12.5).abs() < 1e-3);
        assert_eq!(l[1].window_minutes, 10080);
        assert_eq!(l[1].observed_at_ms, 1_772_625_622_000);
    }

    #[test]
    fn codex_tool_calls_are_read_from_their_own_line() {
        let line = r#"{"type":"response_item","payload":{"type":"custom_tool_call","name":"exec","input":"x"}}"#;
        assert_eq!(codex_tool_name(line).as_deref(), Some("exec"));
        assert_eq!(
            codex_tool_name(r#"{"type":"world_state","payload":{}}"#),
            None
        );
    }

    #[test]
    fn timestamps_accept_rfc3339_and_epochs() {
        assert_eq!(
            ts_ms(Some(&serde_json::json!("2026-03-04T12:00:00.000Z"))),
            Some(1_772_625_600_000)
        );
        assert_eq!(
            ts_ms(Some(&serde_json::json!(1_772_625_600i64))),
            Some(1_772_625_600_000)
        );
        assert_eq!(
            ts_ms(Some(&serde_json::json!(1_772_625_600_000i64))),
            Some(1_772_625_600_000)
        );
        assert_eq!(ts_ms(Some(&serde_json::json!("not a date"))), None);
    }
}

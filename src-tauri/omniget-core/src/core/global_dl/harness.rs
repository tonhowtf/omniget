use std::collections::HashMap;

use serde::Deserialize;
use serde_json::{json, Value};

pub const RESULT_CAP_CHARS: usize = 8000;

#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct Probe {
    pub url: String,
    pub title: String,
    pub doc: String,
    pub muts: u64,
    pub focus: String,
    pub val: i64,
    #[serde(rename = "scrollY")]
    pub scroll_y: i64,
    pub frames: Vec<String>,
    pub password: bool,
    pub head: String,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct SnapOpts {
    pub max_tokens: Option<u64>,
    pub role: Option<String>,
    pub text: Option<String>,
    pub diff: bool,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct TreeDiff {
    pub added: Vec<String>,
    pub removed: Vec<String>,
    pub changed: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum AttachTarget {
    Ws(String),
    Http(String),
}

pub fn estimate_tokens(s: &str) -> u64 {
    s.chars().count().div_ceil(4) as u64
}

fn role_of(line: &str) -> &str {
    line.trim_start()
        .trim_start_matches("- ")
        .split([' ', ':'])
        .next()
        .unwrap_or("")
}

pub fn filter_lines(tree: &str, role: Option<&str>, text: Option<&str>) -> Vec<String> {
    let text = text.map(str::to_lowercase);
    let filtering = role.is_some() || text.is_some();
    tree.lines()
        .filter(|l| !l.trim().is_empty())
        .filter(|l| role.is_none_or(|r| role_of(l).eq_ignore_ascii_case(r)))
        .filter(|l| {
            text.as_ref()
                .is_none_or(|t| l.to_lowercase().contains(t.as_str()))
        })
        .map(|l| if filtering { l.trim_start() } else { l }.to_string())
        .collect()
}

pub fn budget_lines(lines: &[String], max_tokens: u64) -> usize {
    let mut used = 0;
    lines
        .iter()
        .take_while(|l| {
            used += estimate_tokens(l);
            used <= max_tokens
        })
        .count()
}

fn without_ref(line: &str) -> &str {
    let l = line.trim();
    match l.rfind(" [ref=e") {
        Some(i) if l.ends_with(']') => &l[..i],
        _ => l,
    }
}

fn identity(key: &str) -> &str {
    if key.starts_with("- text:") {
        return key;
    }
    let Some(open) = key.find('"') else {
        return key;
    };
    let rest = &key[open + 1..];
    let mut escaped = false;
    for (i, c) in rest.char_indices() {
        match c {
            '\\' if !escaped => escaped = true,
            '"' if !escaped => return &key[..open + 1 + i + 1],
            _ => escaped = false,
        }
    }
    key
}

pub fn diff_lines(prev: &[String], cur: &[String]) -> TreeDiff {
    let mut left: HashMap<&str, usize> = HashMap::new();
    for l in prev {
        *left.entry(without_ref(l)).or_default() += 1;
    }
    let mut fresh: Vec<&str> = Vec::new();
    for l in cur {
        match left.get_mut(without_ref(l)) {
            Some(n) if *n > 0 => *n -= 1,
            _ => fresh.push(l.trim()),
        }
    }
    let mut gone: Vec<&str> = Vec::new();
    for l in prev {
        let k = without_ref(l);
        if let Some(n) = left.get_mut(k).filter(|n| **n > 0) {
            *n -= 1;
            gone.push(k);
        }
    }
    let mut d = TreeDiff::default();
    for l in fresh {
        let id = identity(without_ref(l));
        match gone
            .iter()
            .position(|g| id.contains('"') && identity(g) == id)
        {
            Some(i) => d.changed.push(format!("{} => {l}", gone.remove(i))),
            None => d.added.push(l.to_string()),
        }
    }
    d.removed = gone.into_iter().map(String::from).collect();
    d
}

const NARROW_HINT: &str =
    "raise maxTokens, or narrow the result with role, text or scope to read the rest";

pub fn shape_snapshot(mut v: Value, o: &SnapOpts, prev: Option<&str>) -> Value {
    if !v.is_object() {
        return v;
    }
    let signals = Probe {
        url: v["url"].as_str().unwrap_or_default().to_string(),
        title: v["title"].as_str().unwrap_or_default().to_string(),
        head: v["text"]
            .as_str()
            .unwrap_or_default()
            .chars()
            .take(1500)
            .collect(),
        frames: serde_json::from_value(v["frames"].take()).unwrap_or_default(),
        password: v["password"].as_bool().unwrap_or(false),
        ..Default::default()
    };
    if let Some(m) = v.as_object_mut() {
        m.remove("frames");
        m.remove("password");
    }
    if let Some(b) = detect_blocker(&signals, false) {
        v["needs_human"] = b;
    }
    let filtering = o.role.is_some() || o.text.is_some();
    if !filtering && !o.diff && o.max_tokens.is_none() {
        return sized(v);
    }
    let tree = v["tree"].as_str().unwrap_or_default().to_string();
    let lines = filter_lines(&tree, o.role.as_deref(), o.text.as_deref());
    let budget = o.max_tokens.unwrap_or(u64::MAX);
    let mut truncated = v["truncated"].as_bool().unwrap_or(false);

    if let (true, Some(prev)) = (o.diff, prev) {
        let before = filter_lines(prev, o.role.as_deref(), o.text.as_deref());
        let d = diff_lines(&before, &lines);
        let total = d.added.len() + d.changed.len() + d.removed.len();
        let mut all = d.added.clone();
        all.extend(d.changed.iter().cloned());
        all.extend(d.removed.iter().cloned());
        let kept = budget_lines(&all, budget);
        let take = |from: usize, len: usize| -> Vec<String> {
            all.iter()
                .skip(from)
                .take(len.min(kept.saturating_sub(from)))
                .cloned()
                .collect()
        };
        if let Some(m) = v.as_object_mut() {
            m.remove("tree");
            m.remove("text");
        }
        v["diff"] = json!({
            "added": take(0, d.added.len()),
            "changed": take(d.added.len(), d.changed.len()),
            "removed": take(d.added.len() + d.changed.len(), d.removed.len()),
        });
        if total == 0 {
            v["unchanged"] = json!(true);
        }
        truncated |= kept < total;
        v["total"] = json!(total);
        v["returned"] = json!(kept);
    } else {
        if o.diff {
            v["baseline"] = json!(true);
        }
        let kept = budget_lines(&lines, budget);
        let out = lines[..kept].join("\n");
        truncated |= kept < lines.len();
        if filtering {
            if let Some(m) = v.as_object_mut() {
                m.remove("text");
            }
            if lines.is_empty() {
                v["reason"] = json!("no_match");
            }
        } else if o.max_tokens.is_some() {
            let room = budget
                .saturating_sub(estimate_tokens(&out))
                .saturating_mul(4) as usize;
            let text = v["text"].as_str().unwrap_or_default();
            if kept < lines.len() {
                v["text"] = json!("");
            } else if text.chars().count() > room {
                truncated = true;
                v["text"] = json!(text.chars().take(room).collect::<String>());
            }
        }
        v["tree"] = json!(out);
        v["total"] = json!(lines.len());
        v["returned"] = json!(kept);
    }
    v["truncated"] = json!(truncated);
    if truncated {
        v["hint"] = json!(NARROW_HINT);
    }
    sized(v)
}

fn sized(mut v: Value) -> Value {
    let size = |key: &str| estimate_tokens(v[key].as_str().unwrap_or_default());
    v["tokens"] = json!(size("tree") + size("text"));
    v
}

pub fn tool_for_op(op: &str) -> Option<&'static str> {
    Some(match op {
        "click" => "gd_click",
        "type" => "gd_type",
        "press" => "gd_press",
        "scroll" => "gd_scroll",
        "navigate" => "gd_navigate",
        "wait" => "gd_wait",
        "eval" => "gd_page_eval",
        _ => return None,
    })
}

pub fn changes(before: &Probe, after: &Probe) -> Vec<&'static str> {
    let mut c = Vec::new();
    if before.url != after.url {
        c.push("url");
    }
    if before.doc != after.doc || before.muts != after.muts {
        c.push("dom");
    }
    if before.focus != after.focus {
        c.push("focus");
    }
    if before.val != after.val {
        c.push("value");
    }
    if before.scroll_y != after.scroll_y {
        c.push("scroll");
    }
    c
}

pub fn derive_outcome(
    op: &str,
    before: Option<&Probe>,
    after: Option<&Probe>,
    result: &Result<Value, String>,
) -> &'static str {
    let (Some(before), Some(after)) = (before, after) else {
        return "unknown";
    };
    let moved = changes(before, after);
    let changed = !moved.is_empty();
    match result {
        Ok(_) if op == "click" && moved == ["focus"] => "unknown",
        Err(_) if changed => "unknown",
        Err(_) => "no_effect",
        Ok(r) if op == "wait" => {
            if r["found"] == true {
                "confirmed"
            } else {
                "no_effect"
            }
        }
        Ok(_) if changed => "confirmed",
        Ok(_) if op == "eval" => "unknown",
        Ok(_) => "no_effect",
    }
}

fn blocker(kind: &str, hint: &str) -> Option<Value> {
    Some(json!({ "kind": kind, "hint": hint }))
}

pub fn detect_blocker(p: &Probe, redirected: bool) -> Option<Value> {
    let title = p.title.to_lowercase();
    let head = p.head.to_lowercase();
    let url = p.url.to_lowercase();
    let frame = |needle: &str| {
        p.frames.iter().any(|f| {
            let f = f.to_lowercase();
            f.contains(needle) && !f.contains("size=invisible")
        })
    };
    if title.contains("just a moment")
        || title.contains("attention required")
        || head.contains("verify you are human")
        || head.contains("checking your browser")
        || frame("challenges.cloudflare.com")
    {
        return blocker(
            "bot_challenge",
            "The site is checking for a bot. Ask the person to pass the check in a visible window (gd_browser_open with window=true, or gd_browser_attach to their own Chrome), then take a new gd_snapshot.",
        );
    }
    if frame("/recaptcha/") || frame("hcaptcha.com") || head.contains("i'm not a robot") {
        return blocker(
            "captcha",
            "The page shows a captcha. Ask the person to solve it in a visible window (gd_browser_open with window=true), then continue.",
        );
    }
    if head.contains("browser or app may not be secure") || head.contains("couldn't sign you in") {
        return blocker(
            "automation_refused",
            "The site refuses a browser driven by automation. Use the person's own Chrome with gd_browser_attach, or ask them to sign in by hand in a visible window (gd_browser_open with window=true).",
        );
    }
    let login_url = [
        "login",
        "signin",
        "sign-in",
        "sign_in",
        "/auth",
        "/sso",
        "accounts.",
    ]
    .iter()
    .any(|n| url.contains(n));
    if p.password && (redirected || login_url) {
        return blocker(
            "login_wall",
            "The site asks for a login. The person logs in by hand in a visible window (gd_browser_open with window=true), or use their logged-in Chrome with gd_browser_attach or gd_browser_import.",
        );
    }
    None
}

pub fn redirected(op: &str, requested: &str, before: &Probe, after: &Probe) -> bool {
    if before.url == after.url {
        return false;
    }
    op != "navigate" || requested.trim_end_matches('/') != after.url.trim_end_matches('/')
}

const CDP_DENIED: &[&str] = &[
    "browser.close",
    "browser.crash",
    "browser.crashgpuprocess",
    "target.disposebrowsercontext",
    "storage.clearcookies",
    "network.clearbrowsercookies",
];

pub fn cdp_method_allowed(method: &str) -> bool {
    let Some((domain, name)) = method.split_once('.') else {
        return false;
    };
    let word = |s: &str| !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric());
    if !word(domain) || !word(name) {
        return false;
    }
    let m = method.to_ascii_lowercase();
    !m.starts_with("systeminfo.") && !CDP_DENIED.contains(&m.as_str())
}

pub fn cdp_browser_level(method: &str) -> bool {
    method.starts_with("Browser.") || method.starts_with("Target.")
}

pub fn cap_result(v: Value, max_chars: usize) -> (Value, Option<String>) {
    let full = v.to_string();
    let total = full.chars().count();
    if total <= max_chars {
        return (v, None);
    }
    let preview: String = full.chars().take(max_chars).collect();
    (
        json!({ "truncated": true, "total": total, "preview": preview }),
        Some(full),
    )
}

pub fn attach_target(args: &Value) -> Result<AttachTarget, String> {
    let endpoint = args["endpoint"].as_str().unwrap_or_default().trim();
    if endpoint.starts_with("ws://") || endpoint.starts_with("wss://") {
        return Ok(AttachTarget::Ws(endpoint.to_string()));
    }
    if endpoint.starts_with("http://") || endpoint.starts_with("https://") {
        return Ok(AttachTarget::Http(
            endpoint.trim_end_matches('/').to_string(),
        ));
    }
    if !endpoint.is_empty() {
        return Err("endpoint must start with ws://, wss://, http:// or https://".into());
    }
    match args.get("port") {
        None | Some(Value::Null) => Ok(AttachTarget::Http("http://127.0.0.1:9222".into())),
        Some(p) => match p.as_u64().filter(|p| (1..=65535).contains(p)) {
            Some(p) => Ok(AttachTarget::Http(format!("http://127.0.0.1:{p}"))),
            None => Err("port must be between 1 and 65535".into()),
        },
    }
}

pub fn parse_version(body: &str) -> Option<String> {
    let v: Value = serde_json::from_str(body).ok()?;
    let ws = v["webSocketDebuggerUrl"].as_str()?;
    (ws.starts_with("ws://") || ws.starts_with("wss://")).then(|| ws.to_string())
}

pub fn not_debuggable(target: &str, detail: &str) -> Value {
    json!({
        "code": "CHROME_NOT_DEBUGGABLE",
        "message": format!("no Chrome with remote debugging answered at {target}: {detail}"),
        "retryable": true,
        "side_effect": "none",
        "next_step": "Ask the person to turn remote debugging on and call gd_browser_attach again. In recent Chrome: open chrome://inspect/#remote-debugging, tick \"Allow remote debugging for this browser instance\" and accept the prompt. Otherwise quit Chrome and start it with --remote-debugging-port=9222 (Chrome 136 and later also need --user-data-dir pointing at a non-default folder). To use the OmniGet browser instead, call gd_browser_open.",
    })
}

pub fn connect_failed(target: &str, detail: &str) -> Value {
    if !detail.contains("timed out") {
        return not_debuggable(target, detail);
    }
    json!({
        "code": "CHROME_WAITING_APPROVAL",
        "message": format!("Chrome at {target} has remote debugging on but did not accept the connection"),
        "retryable": true,
        "side_effect": "none",
        "next_step": "Chrome is showing an \"Allow remote debugging?\" prompt in its window. Ask the person to click Allow, then call gd_browser_attach again.",
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const TREE: &str = "- banner [ref=e1]\n  - link \"Home\" href=\"/\" [ref=e2]\n  - button \"Sign in\" [ref=e3]\n- main [ref=e4]\n  - heading \"Orders\" [ref=e5]\n  - text: \"3 orders found\"\n  - textbox \"Search\" value=\"abc\" [ref=e6]\n  - button \"Search orders\" [ref=e7]";

    fn lines(t: &str) -> Vec<String> {
        filter_lines(t, None, None)
    }

    fn probe(url: &str) -> Probe {
        Probe {
            url: url.into(),
            doc: "d1".into(),
            ..Default::default()
        }
    }

    #[test]
    fn tokens_are_a_quarter_of_the_chars_rounded_up() {
        assert_eq!(estimate_tokens(""), 0);
        assert_eq!(estimate_tokens("abcd"), 1);
        assert_eq!(estimate_tokens("abcde"), 2);
        assert_eq!(estimate_tokens("ááááá"), 2);
    }

    #[test]
    fn no_filter_keeps_every_line_with_its_indent() {
        let l = lines(TREE);
        assert_eq!(l.len(), 8);
        assert_eq!(l[1], "  - link \"Home\" href=\"/\" [ref=e2]");
    }

    #[test]
    fn role_filter_keeps_only_that_role_and_drops_the_indent() {
        let l = filter_lines(TREE, Some("Button"), None);
        assert_eq!(
            l,
            vec![
                "- button \"Sign in\" [ref=e3]",
                "- button \"Search orders\" [ref=e7]"
            ]
        );
    }

    #[test]
    fn text_filter_is_case_insensitive_and_combines_with_role() {
        let l = filter_lines(TREE, None, Some("ORDERS"));
        assert_eq!(l.len(), 3);
        let l = filter_lines(TREE, Some("button"), Some("orders"));
        assert_eq!(l, vec!["- button \"Search orders\" [ref=e7]"]);
        assert!(filter_lines(TREE, Some("text"), None)[0].starts_with("- text:"));
    }

    #[test]
    fn budget_keeps_whole_lines_that_fit() {
        let l = lines(TREE);
        assert_eq!(budget_lines(&l, 100_000), 8);
        assert_eq!(budget_lines(&l, 0), 0);
        let two = estimate_tokens(&l[0]) + estimate_tokens(&l[1]);
        assert_eq!(budget_lines(&l, two), 2);
        assert_eq!(budget_lines(&l, two - 1), 1);
    }

    #[test]
    fn diff_ignores_renumbered_refs_and_depth() {
        let prev = lines("- main [ref=e1]\n  - button \"Save\" [ref=e2]");
        let cur = lines("- banner [ref=e1]\n- main [ref=e2]\n    - button \"Save\" [ref=e3]");
        let d = diff_lines(&prev, &cur);
        assert_eq!(d.added, vec!["- banner [ref=e1]"]);
        assert!(d.removed.is_empty() && d.changed.is_empty());
    }

    #[test]
    fn diff_reports_added_removed_and_changed() {
        let prev =
            lines("- textbox \"Search\" [ref=e1]\n- button \"Go\" [ref=e2]\n- text: \"old\"");
        let cur = lines(
            "- textbox \"Search\" value=\"abc\" [ref=e1]\n- text: \"new\"\n- link \"Next\" [ref=e2]",
        );
        let d = diff_lines(&prev, &cur);
        assert_eq!(
            d.changed,
            vec!["- textbox \"Search\" => - textbox \"Search\" value=\"abc\" [ref=e1]"]
        );
        assert_eq!(d.added, vec!["- text: \"new\"", "- link \"Next\" [ref=e2]"]);
        assert_eq!(d.removed, vec!["- button \"Go\"", "- text: \"old\""]);
    }

    #[test]
    fn diff_counts_duplicates() {
        let prev = lines("- listitem [ref=e1]");
        let cur = lines("- listitem [ref=e1]\n- listitem [ref=e2]");
        let d = diff_lines(&prev, &cur);
        assert_eq!(d.added, vec!["- listitem [ref=e2]"]);
        assert!(d.removed.is_empty());
    }

    fn raw() -> Value {
        json!({ "url": "https://shop.test/orders", "title": "Orders", "tree": TREE, "nodes": 8, "truncated": false, "text": "Orders 3 orders found", "frames": [], "password": false })
    }

    #[test]
    fn snapshot_reports_its_size_in_tokens() {
        let v = shape_snapshot(raw(), &SnapOpts::default(), None);
        let expected = estimate_tokens(TREE) + estimate_tokens("Orders 3 orders found");
        assert_eq!(v["tokens"], expected);
    }

    #[test]
    fn snapshot_without_options_is_unchanged_but_for_the_signals() {
        let v = shape_snapshot(raw(), &SnapOpts::default(), None);
        assert_eq!(v["tree"], TREE);
        assert_eq!(v["text"], "Orders 3 orders found");
        assert_eq!(v["truncated"], false);
        assert!(v.get("frames").is_none() && v.get("password").is_none());
        assert!(v.get("total").is_none() && v.get("needs_human").is_none());
    }

    #[test]
    fn snapshot_budget_truncates_and_reports_the_total() {
        let o = SnapOpts {
            max_tokens: Some(20),
            ..Default::default()
        };
        let v = shape_snapshot(raw(), &o, None);
        assert_eq!(v["truncated"], true);
        assert_eq!(v["total"], 8);
        let returned = v["returned"].as_u64().unwrap();
        assert!(returned > 0 && returned < 8);
        assert_eq!(v["tree"].as_str().unwrap().lines().count() as u64, returned);
        assert!(v["hint"].as_str().unwrap().contains("maxTokens"));
        assert!(estimate_tokens(v["tree"].as_str().unwrap()) <= 20);
        assert_eq!(v["text"], "");
    }

    #[test]
    fn snapshot_budget_gives_the_rest_to_the_page_text() {
        let o = SnapOpts {
            max_tokens: Some(1000),
            ..Default::default()
        };
        let v = shape_snapshot(raw(), &o, None);
        assert_eq!(v["truncated"], false);
        assert_eq!(v["total"], 8);
        assert_eq!(v["text"], "Orders 3 orders found");
    }

    #[test]
    fn snapshot_filter_drops_the_page_text() {
        let o = SnapOpts {
            role: Some("button".into()),
            ..Default::default()
        };
        let v = shape_snapshot(raw(), &o, None);
        assert_eq!(v["total"], 2);
        assert_eq!(v["returned"], 2);
        assert!(v.get("text").is_none());
        assert_eq!(v["truncated"], false);
    }

    #[test]
    fn snapshot_filter_with_no_match_says_why() {
        let o = SnapOpts {
            role: Some("slider".into()),
            ..Default::default()
        };
        let v = shape_snapshot(raw(), &o, None);
        assert_eq!(v["total"], 0);
        assert_eq!(v["reason"], "no_match");
    }

    #[test]
    fn snapshot_diff_without_a_previous_one_is_a_baseline() {
        let o = SnapOpts {
            diff: true,
            ..Default::default()
        };
        let v = shape_snapshot(raw(), &o, None);
        assert_eq!(v["baseline"], true);
        assert_eq!(v["tree"], TREE);
    }

    #[test]
    fn snapshot_diff_returns_only_what_changed() {
        let o = SnapOpts {
            diff: true,
            ..Default::default()
        };
        let prev = TREE.replace("3 orders found", "2 orders found");
        let v = shape_snapshot(raw(), &o, Some(&prev));
        assert!(v.get("tree").is_none() && v.get("text").is_none());
        assert_eq!(v["diff"]["added"], json!(["- text: \"3 orders found\""]));
        assert_eq!(v["diff"]["removed"], json!(["- text: \"2 orders found\""]));
        assert_eq!(v["diff"]["changed"], json!([]));
        assert_eq!(v["total"], 2);
        assert_eq!(v["truncated"], false);

        let same = shape_snapshot(raw(), &o, Some(TREE));
        assert_eq!(same["unchanged"], true);
        assert_eq!(same["total"], 0);
    }

    #[test]
    fn snapshot_diff_respects_the_budget() {
        let o = SnapOpts {
            diff: true,
            max_tokens: Some(8),
            ..Default::default()
        };
        let v = shape_snapshot(raw(), &o, Some("- main [ref=e1]"));
        assert_eq!(v["truncated"], true);
        assert!(v["returned"].as_u64().unwrap() < v["total"].as_u64().unwrap());
    }

    #[test]
    fn snapshot_reports_a_blocker() {
        let mut r = raw();
        r["title"] = json!("Just a moment...");
        let v = shape_snapshot(r, &SnapOpts::default(), None);
        assert_eq!(v["needs_human"]["kind"], "bot_challenge");
    }

    #[test]
    fn ops_map_to_the_single_step_tools() {
        assert_eq!(tool_for_op("click"), Some("gd_click"));
        assert_eq!(tool_for_op("type"), Some("gd_type"));
        assert_eq!(tool_for_op("press"), Some("gd_press"));
        assert_eq!(tool_for_op("scroll"), Some("gd_scroll"));
        assert_eq!(tool_for_op("navigate"), Some("gd_navigate"));
        assert_eq!(tool_for_op("wait"), Some("gd_wait"));
        assert_eq!(tool_for_op("eval"), Some("gd_page_eval"));
        assert_eq!(tool_for_op("act"), None);
        assert_eq!(tool_for_op(""), None);
    }

    #[test]
    fn changes_name_what_moved() {
        let a = probe("https://a.test/");
        assert!(changes(&a, &a).is_empty());
        let mut b = a.clone();
        b.url = "https://a.test/next".into();
        b.doc = "d2".into();
        assert_eq!(changes(&a, &b), vec!["url", "dom"]);
        let mut c = a.clone();
        c.muts = 4;
        c.focus = "INPUT#q".into();
        c.val = 9;
        c.scroll_y = 300;
        assert_eq!(changes(&a, &c), vec!["dom", "focus", "value", "scroll"]);
    }

    #[test]
    fn outcome_is_confirmed_when_something_changed() {
        let a = probe("https://a.test/");
        let mut b = a.clone();
        b.muts = 2;
        let ok = Ok(json!({}));
        assert_eq!(
            derive_outcome("click", Some(&a), Some(&b), &ok),
            "confirmed"
        );
        assert_eq!(derive_outcome("eval", Some(&a), Some(&b), &ok), "confirmed");
    }

    #[test]
    fn outcome_is_no_effect_when_an_action_changed_nothing() {
        let a = probe("https://a.test/");
        let ok = Ok(json!({}));
        for op in ["click", "type", "press", "scroll", "navigate"] {
            assert_eq!(derive_outcome(op, Some(&a), Some(&a), &ok), "no_effect");
        }
    }

    #[test]
    fn a_click_that_only_moved_focus_is_unknown() {
        let a = probe("https://a.test/");
        let mut b = a.clone();
        b.focus = "button#buy".into();
        let ok = Ok(json!({}));
        assert_eq!(derive_outcome("click", Some(&a), Some(&b), &ok), "unknown");
        assert_eq!(
            derive_outcome("press", Some(&a), Some(&b), &ok),
            "confirmed"
        );
    }

    #[test]
    fn a_page_that_refuses_automated_browsers_is_its_own_blocker() {
        let mut p = probe("https://accounts.google.com/v3/signin/rejected");
        p.head = "Couldn't sign you in. This browser or app may not be secure.".into();
        let b = detect_blocker(&p, false).unwrap();
        assert_eq!(b["kind"], "automation_refused");
        assert!(b["hint"].as_str().unwrap().contains("gd_browser_attach"));
    }

    #[test]
    fn outcome_of_wait_follows_found() {
        let a = probe("https://a.test/");
        let found = Ok(json!({ "found": true }));
        let missing = Ok(json!({ "found": false }));
        assert_eq!(
            derive_outcome("wait", Some(&a), Some(&a), &found),
            "confirmed"
        );
        assert_eq!(
            derive_outcome("wait", Some(&a), Some(&a), &missing),
            "no_effect"
        );
    }

    #[test]
    fn outcome_is_unknown_without_evidence() {
        let a = probe("https://a.test/");
        let ok = Ok(json!({}));
        assert_eq!(derive_outcome("click", None, Some(&a), &ok), "unknown");
        assert_eq!(derive_outcome("click", Some(&a), None, &ok), "unknown");
        assert_eq!(derive_outcome("eval", Some(&a), Some(&a), &ok), "unknown");
    }

    #[test]
    fn outcome_of_a_failed_step() {
        let a = probe("https://a.test/");
        let mut b = a.clone();
        b.muts = 1;
        let err = Err("No element matches #x".to_string());
        assert_eq!(
            derive_outcome("click", Some(&a), Some(&a), &err),
            "no_effect"
        );
        assert_eq!(derive_outcome("click", Some(&a), Some(&b), &err), "unknown");
    }

    #[test]
    fn detects_cloudflare_and_human_checks() {
        let mut p = probe("https://site.test/");
        p.title = "Just a moment...".into();
        assert_eq!(detect_blocker(&p, false).unwrap()["kind"], "bot_challenge");
        let mut p = probe("https://site.test/");
        p.head = "Please Verify you are human to continue".into();
        assert_eq!(detect_blocker(&p, false).unwrap()["kind"], "bot_challenge");
        let mut p = probe("https://site.test/");
        p.frames = vec![
            "https://challenges.cloudflare.com/cdn-cgi/challenge-platform/h/b/turnstile".into(),
        ];
        let b = detect_blocker(&p, false).unwrap();
        assert_eq!(b["kind"], "bot_challenge");
        assert!(!b["hint"].as_str().unwrap().is_empty());
    }

    #[test]
    fn detects_captcha_frames_but_not_the_invisible_badge() {
        let mut p = probe("https://site.test/form");
        p.frames = vec!["https://www.google.com/recaptcha/api2/anchor?k=abc&size=normal".into()];
        assert_eq!(detect_blocker(&p, false).unwrap()["kind"], "captcha");
        p.frames = vec!["https://newassets.hcaptcha.com/captcha/v1/x/static/hcaptcha.html".into()];
        assert_eq!(detect_blocker(&p, false).unwrap()["kind"], "captcha");
        p.frames = vec!["https://www.google.com/recaptcha/api2/anchor?k=abc&size=invisible".into()];
        assert_eq!(detect_blocker(&p, false), None);
    }

    #[test]
    fn detects_a_login_wall_only_on_a_redirect_or_a_login_url() {
        let mut p = probe("https://site.test/settings");
        p.password = true;
        assert_eq!(detect_blocker(&p, false), None);
        assert_eq!(detect_blocker(&p, true).unwrap()["kind"], "login_wall");
        let mut p = probe("https://accounts.site.test/signin?next=/orders");
        p.password = true;
        assert_eq!(detect_blocker(&p, false).unwrap()["kind"], "login_wall");
        p.password = false;
        assert_eq!(detect_blocker(&p, true), None);
    }

    #[test]
    fn a_plain_page_has_no_blocker() {
        let mut p = probe("https://site.test/");
        p.title = "Orders".into();
        p.head = "3 orders found".into();
        p.frames = vec!["https://www.youtube.com/embed/x".into()];
        assert_eq!(detect_blocker(&p, false), None);
    }

    #[test]
    fn redirect_means_landing_somewhere_else_than_asked() {
        let a = probe("https://site.test/");
        let b = probe("https://site.test/login");
        assert!(redirected("click", "", &a, &b));
        assert!(!redirected("click", "", &a, &a));
        assert!(redirected("navigate", "https://site.test/orders", &a, &b));
        assert!(!redirected("navigate", "https://site.test/login/", &a, &b));
    }

    #[test]
    fn cdp_denylist() {
        for m in [
            "Browser.close",
            "Browser.crash",
            "Browser.crashGpuProcess",
            "Target.disposeBrowserContext",
            "Storage.clearCookies",
            "Network.clearBrowserCookies",
            "SystemInfo.getInfo",
            "SystemInfo.getProcessInfo",
            "systeminfo.getInfo",
            "browser.close",
            "",
            "Page",
            "Page.",
            ".enable",
            "Page.navigate extra",
        ] {
            assert!(!cdp_method_allowed(m), "{m} must be denied");
        }
        for m in [
            "Page.navigate",
            "IO.read",
            "Runtime.evaluate",
            "Network.deleteCookies",
            "Target.closeTarget",
            "Browser.getVersion",
            "Accessibility.getFullAXTree",
        ] {
            assert!(cdp_method_allowed(m), "{m} must be allowed");
        }
    }

    #[test]
    fn browser_and_target_methods_need_no_tab() {
        assert!(cdp_browser_level("Browser.getVersion"));
        assert!(cdp_browser_level("Target.getTargets"));
        assert!(!cdp_browser_level("Page.navigate"));
    }

    #[test]
    fn small_results_pass_through() {
        let v = json!({ "a": 1 });
        assert_eq!(cap_result(v.clone(), 100), (v, None));
    }

    #[test]
    fn large_results_become_a_preview_and_the_full_text() {
        let v = json!({ "data": "é".repeat(500) });
        let (out, full) = cap_result(v.clone(), 100);
        assert_eq!(out["truncated"], true);
        assert_eq!(out["total"], v.to_string().chars().count());
        assert_eq!(out["preview"].as_str().unwrap().chars().count(), 100);
        assert_eq!(full, Some(v.to_string()));
    }

    #[test]
    fn attach_target_defaults_to_port_9222() {
        assert_eq!(
            attach_target(&json!({})),
            Ok(AttachTarget::Http("http://127.0.0.1:9222".into()))
        );
        assert_eq!(
            attach_target(&json!({ "port": 9333 })),
            Ok(AttachTarget::Http("http://127.0.0.1:9333".into()))
        );
    }

    #[test]
    fn attach_target_takes_an_endpoint() {
        assert_eq!(
            attach_target(&json!({ "endpoint": "ws://127.0.0.1:9222/devtools/browser/abc" })),
            Ok(AttachTarget::Ws(
                "ws://127.0.0.1:9222/devtools/browser/abc".into()
            ))
        );
        assert_eq!(
            attach_target(&json!({ "endpoint": "http://localhost:9222/" })),
            Ok(AttachTarget::Http("http://localhost:9222".into()))
        );
        assert!(attach_target(&json!({ "endpoint": "ftp://x" })).is_err());
        assert!(attach_target(&json!({ "port": 0 })).is_err());
        assert!(attach_target(&json!({ "port": 70000 })).is_err());
    }

    #[test]
    fn version_json_gives_the_websocket() {
        let body = r#"{"Browser":"Chrome/141.0.7390.55","Protocol-Version":"1.3","webSocketDebuggerUrl":"ws://127.0.0.1:9222/devtools/browser/0a1b"}"#;
        assert_eq!(
            parse_version(body).as_deref(),
            Some("ws://127.0.0.1:9222/devtools/browser/0a1b")
        );
        assert_eq!(parse_version("<html>404</html>"), None);
        assert_eq!(parse_version(r#"{"Browser":"x"}"#), None);
        assert_eq!(
            parse_version(r#"{"webSocketDebuggerUrl":"http://x"}"#),
            None
        );
    }

    #[test]
    fn a_connection_that_never_opens_means_chrome_waits_for_the_person() {
        let e = connect_failed(
            "http://127.0.0.1:9222",
            "timed out connecting to the browser",
        );
        assert_eq!(e["code"], "CHROME_WAITING_APPROVAL");
        assert_eq!(e["retryable"], true);
        assert!(e["next_step"].as_str().unwrap().contains("Allow"));
        let refused = connect_failed("http://127.0.0.1:9222", "connection refused");
        assert_eq!(refused["code"], "CHROME_NOT_DEBUGGABLE");
    }

    #[test]
    fn not_debuggable_error_says_what_to_do() {
        let e = not_debuggable("http://127.0.0.1:9222", "connection refused");
        assert_eq!(e["code"], "CHROME_NOT_DEBUGGABLE");
        assert_eq!(e["retryable"], true);
        assert_eq!(e["side_effect"], "none");
        assert!(e["message"].as_str().unwrap().contains("127.0.0.1:9222"));
        assert!(e["message"]
            .as_str()
            .unwrap()
            .contains("connection refused"));
        let next = e["next_step"].as_str().unwrap();
        assert!(next.contains("chrome://inspect/#remote-debugging"));
        assert!(next.contains("--remote-debugging-port=9222"));
    }
}

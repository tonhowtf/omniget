use std::io::{IsTerminal, Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::path::PathBuf;
use std::time::{Duration, Instant};

use serde_json::{json, Map, Value};

use super::adapters::{codex, gemini, pty};
use crate::core::agentkit::convert::hook_shim;

pub const EXIT_OK: i32 = 0;
pub const EXIT_ERROR: i32 = 1;
pub const EXIT_USAGE: i32 = 2;
pub const EXIT_NOT_FINISHED: i32 = 3;
pub const EXIT_RELEASED: i32 = 4;

pub const TOKEN_ENV: &str = "OMNIGET_CANVAS_TOKEN";
pub const URL_ENV: &str = "OMNIGET_BRIDGE_URL";
pub const NODE_ENV: &str = "OMNIGET_CANVAS_NODE";

pub const DEFAULT_ASK_TIMEOUT_S: u64 = 540;
const WAIT_SLICE_MS: u64 = 100_000;

#[derive(Debug, Clone)]
pub struct Conn {
    pub host: String,
    pub port: u16,
    pub token: String,
}

pub fn parse_url(url: &str) -> Option<(String, u16)> {
    let rest = url.trim().strip_prefix("http://")?;
    let hostport = rest.split('/').next()?;
    let (host, port) = hostport.rsplit_once(':')?;
    Some((host.to_string(), port.parse().ok()?))
}

impl Conn {
    pub fn from_env() -> Result<Conn, String> {
        let token = std::env::var(TOKEN_ENV).unwrap_or_default();
        if token.trim().is_empty() {
            return Err(format!(
                "not on an OmniGet canvas: {TOKEN_ENV} is not set (run this inside a terminal of the canvas)"
            ));
        }
        let (host, port) = std::env::var(URL_ENV)
            .ok()
            .and_then(|u| parse_url(&u))
            .or_else(|| {
                crate::core::paths::app_data_dir()
                    .and_then(|d| hook_shim::bridge_port(&d))
                    .map(|p| ("127.0.0.1".to_string(), p))
            })
            .ok_or_else(|| {
                format!("no bridge address: {URL_ENV} is not set and the app's bridge is off")
            })?;
        Ok(Conn {
            host,
            port,
            token: token.trim().to_string(),
        })
    }

    pub fn url(&self) -> String {
        format!("http://{}:{}", self.host, self.port)
    }

    pub fn request(
        &self,
        method: &str,
        path: &str,
        body: Option<&Value>,
        read_timeout: Duration,
    ) -> Result<(u16, Value), String> {
        let addr = (self.host.as_str(), self.port)
            .to_socket_addrs()
            .map_err(|e| format!("bad bridge address: {e}"))?
            .next()
            .ok_or("bad bridge address")?;
        let mut s = TcpStream::connect_timeout(&addr, Duration::from_millis(1500))
            .map_err(|e| format!("the OmniGet app does not answer on {}: {e}", self.url()))?;
        s.set_read_timeout(Some(read_timeout)).ok();
        s.set_write_timeout(Some(Duration::from_secs(10))).ok();
        let payload = body
            .map(|b| serde_json::to_vec(b).unwrap_or_default())
            .unwrap_or_default();
        let mut head = format!(
            "{method} {path} HTTP/1.1\r\nHost: {}:{}\r\nAuthorization: Bearer {}\r\nConnection: close\r\n",
            self.host, self.port, self.token
        );
        if body.is_some() {
            head.push_str(&format!(
                "Content-Type: application/json\r\nContent-Length: {}\r\n",
                payload.len()
            ));
        }
        head.push_str("\r\n");
        s.write_all(head.as_bytes()).map_err(|e| e.to_string())?;
        if !payload.is_empty() {
            s.write_all(&payload).map_err(|e| e.to_string())?;
        }
        let mut buf = Vec::new();
        s.read_to_end(&mut buf).map_err(|e| {
            let timed_out = matches!(
                e.kind(),
                std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
            );
            match (timed_out, method) {
                (true, "GET") => format!(
                    "the app did not answer within {}s (it may be busy or paused). The request reached it and may have run anyway; this command only reads, so it is safe to run again.",
                    read_timeout.as_secs()
                ),
                (true, _) => format!(
                    "the app did not answer within {}s. The command reached it and MAY HAVE RUN anyway: check (e.g. `omniget canvas list`) before running it again.",
                    read_timeout.as_secs()
                ),
                _ => format!(
                    "reading the answer: {e}. The request may have reached the app and run anyway: check before running it again."
                ),
            }
        })?;
        parse_response(&buf).ok_or_else(|| "malformed answer from the app".into())
    }

    pub fn get(&self, path: &str, read_timeout: Duration) -> Result<(u16, Value), String> {
        self.request("GET", path, None, read_timeout)
    }

    pub fn post(
        &self,
        path: &str,
        body: &Value,
        read_timeout: Duration,
    ) -> Result<(u16, Value), String> {
        self.request("POST", path, Some(body), read_timeout)
    }
}

fn parse_response(buf: &[u8]) -> Option<(u16, Value)> {
    let split = buf.windows(4).position(|w| w == b"\r\n\r\n")?;
    let headers = String::from_utf8_lossy(&buf[..split]).to_ascii_lowercase();
    let status: u16 = headers
        .lines()
        .next()?
        .split_whitespace()
        .nth(1)?
        .parse()
        .ok()?;
    let mut payload = buf[split + 4..].to_vec();
    if headers.contains("transfer-encoding: chunked") {
        payload = dechunk(&payload);
    }
    let v = if payload.iter().all(|b| b.is_ascii_whitespace()) {
        Value::Null
    } else {
        serde_json::from_slice(&payload)
            .unwrap_or_else(|_| Value::String(String::from_utf8_lossy(&payload).into_owned()))
    };
    Some((status, v))
}

fn dechunk(b: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        let Some(nl) = b[i..].windows(2).position(|w| w == b"\r\n") else {
            break;
        };
        let size_s = String::from_utf8_lossy(&b[i..i + nl]).to_string();
        let size =
            usize::from_str_radix(size_s.split(';').next().unwrap_or("0").trim(), 16).unwrap_or(0);
        i += nl + 2;
        if size == 0 || i + size > b.len() {
            break;
        }
        out.extend_from_slice(&b[i..i + size]);
        i += size + 2;
    }
    out
}

fn err_text(v: &Value) -> String {
    v.get("error")
        .and_then(|e| e.as_str())
        .map(str::to_string)
        .unwrap_or_else(|| v.to_string())
}

fn exit_of(status: u16) -> i32 {
    match status {
        400 | 401 | 403 | 404 | 409 | 422 | 429 => EXIT_USAGE,
        _ => EXIT_ERROR,
    }
}

struct Args {
    pos: Vec<String>,
    flags: Map<String, Value>,
    json: bool,
}

const VALUED: &[&str] = &[
    "timeout",
    "lines",
    "file",
    "text-from",
    "batch",
    "raw",
    "name",
    "turns",
    "last",
    "from",
    "to",
    "attach",
    "tool",
    "size",
    "url",
    "offset",
    "limit",
    "agent",
    "preset",
    "command",
    "role",
    "floor",
    "workspace",
    "dir",
    "replace",
    "every",
    "daily",
    "at",
    "prompt",
    "target",
    "into",
    "message",
    "branch",
    "kind",
    "id",
    "group",
    "folder",
    "query",
    "pre-run",
];

const SWITCHES: &[&str] = &[
    "no-wait",
    "progress",
    "force",
    "none",
    "all",
    "peek",
    "clear",
    "existing-branch",
    "no-git",
    "copy-ground",
    "delete-branch",
    "keep-floor",
    "keep-branch",
    "skip-if-busy",
    "notify",
    "submit",
    "double",
    "full",
    "include-startup-commands",
    "blank-notes",
    "include-chat",
    "help",
];

fn parse(args: &[String]) -> Args {
    let mut a = Args {
        pos: Vec::new(),
        flags: Map::new(),
        json: false,
    };
    let mut i = 0;
    let mut only_pos = false;
    while i < args.len() {
        let s = &args[i];
        if only_pos || !s.starts_with("--") || s == "--" {
            if s == "--" && !only_pos {
                only_pos = true;
            } else {
                a.pos.push(s.clone());
            }
            i += 1;
            continue;
        }
        let (k, inline) = match s[2..].split_once('=') {
            Some((k, v)) => (k.to_string(), Some(v.to_string())),
            None => (s[2..].to_string(), None),
        };
        if k == "json" {
            a.json = true;
            i += 1;
            continue;
        }
        if VALUED.contains(&k.as_str()) || !SWITCHES.contains(&k.as_str()) {
            let v = inline.or_else(|| {
                let next = args.get(i + 1).cloned();
                if next.as_deref().is_some_and(|n| !n.starts_with("--")) {
                    i += 1;
                    next
                } else {
                    None
                }
            });
            match (a.flags.get_mut(&k), v) {
                (Some(Value::Array(arr)), Some(v)) => arr.push(Value::String(v)),
                (Some(prev), Some(v)) if k == "attach" => {
                    let first = prev.clone();
                    *prev = json!([first, v]);
                }
                (_, Some(v)) => {
                    a.flags.insert(k, Value::String(v));
                }
                (_, None) => {
                    a.flags.insert(k, Value::Bool(true));
                }
            }
        } else {
            a.flags
                .insert(k, inline.map(Value::String).unwrap_or(Value::Bool(true)));
        }
        i += 1;
    }
    a
}

impl Args {
    fn flag(&self, k: &str) -> Option<&str> {
        self.flags.get(k).and_then(|v| v.as_str())
    }
    fn has(&self, k: &str) -> bool {
        self.flags.contains_key(k)
    }
    fn num(&self, k: &str) -> Option<u64> {
        self.flag(k).and_then(|v| v.parse().ok())
    }
}

fn text_arg(v: Option<&String>) -> Option<String> {
    match v.map(String::as_str) {
        Some("-") => read_stdin(),
        Some(s) => Some(pty::decode_text(s)),
        None if !std::io::stdin().is_terminal() => read_stdin().filter(|s| !s.trim().is_empty()),
        None => None,
    }
}

fn read_stdin() -> Option<String> {
    let mut s = String::new();
    std::io::stdin().read_to_string(&mut s).ok()?;
    Some(s)
}

fn usage(msg: &str) -> i32 {
    eprintln!("omniget canvas: {msg}\n\n{HELP}");
    EXIT_USAGE
}

pub const HELP: &str = "Usage: omniget canvas <command> [--json]

  list                                   who you are and what is connected to you
  ask \"Name\" \"text\" [--timeout 540] [--no-wait]
  ask --batch '{\"A\":\"...\",\"B\":\"...\"}'  ask several peers in parallel
  ask \"Name\" --raw \"\\e[A\\r\"              send raw keys (needs the raw permission)
  check \"Name\" [--lines 60]              read a peer's screen
  reply [ask_id] \"text\" | --file f       answer whoever asked you
  say \"text\" [--progress] [--file f] [--text-from f]  chat with the user
  recall [--turns N]                     the chat thread so far
  note create|read|write|edit|stack|unstack|delete ...
  portal <verb> ...                      drive a connected browser portal
  recruit|connect|dismiss|notify ...     Maestro only (--name, --agent, --from, --to)
  floor|routine|role|partitura <verb>    e.g. routine create --every 30m --prompt ...
  debug                                  transport, node and token check

Exit codes: 0 ok, 2 usage or not allowed, 3 not finished yet, 4 released (deadlock), 1 error.";

fn print_json(v: &Value) {
    println!("{}", serde_json::to_string_pretty(v).unwrap_or_default());
}

pub fn run_cli(args: &[String]) -> i32 {
    let Some(cmd) = args.first().map(String::as_str) else {
        println!("{HELP}");
        return EXIT_OK;
    };
    if matches!(cmd, "help" | "--help" | "-h") {
        println!("{HELP}");
        return EXIT_OK;
    }
    if cmd == "hook" {
        return hook_main(&args[1..]);
    }
    if wants_help(&args[1..]) {
        let verb = args[1..]
            .iter()
            .find(|s| !s.starts_with('-'))
            .map(String::as_str);
        println!("{}", command_help(cmd, verb));
        return EXIT_OK;
    }
    let a = parse(&args[1..]);
    let conn = match Conn::from_env() {
        Ok(c) => c,
        Err(e) => {
            if cmd == "debug" {
                return debug(None, &a, Some(e));
            }
            eprintln!("omniget canvas: {e}");
            return EXIT_USAGE;
        }
    };
    match cmd {
        "list" | "ls" | "peers" => list(&conn, &a),
        "me" | "whoami" => simple_get(&conn, "/v1/canvas/me", &a),
        "ask" => ask(&conn, &a),
        "check" => check(&conn, &a),
        "reply" => reply(&conn, &a),
        "say" => say(&conn, &a),
        "recall" => rpc_print(&conn, "chat.recall", recall_args(&a), &a),
        "note" => note(&conn, &a),
        "portal" => {
            let Some(verb) = a.pos.first() else {
                return usage("portal needs a verb (snapshot, click, fill, navigate, ...)");
            };
            let mut words = a.pos[1..].to_vec();
            let named = !matches!(verb.as_str(), "list" | "create" | "open")
                && a.flag("portal").is_none()
                && words.first().is_some_and(|w| !looks_like_target(w));
            if named {
                if let Ok(v) = rpc(&conn, "portal.list", json!({})) {
                    let names: Vec<String> = v["portals"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(|p| p["name"].as_str().map(str::to_string))
                        .collect();
                    if names.len() == 1 && !names[0].eq_ignore_ascii_case(words[0].trim()) {
                        words.insert(0, names[0].clone());
                    }
                }
            }
            let (verb, args) = portal_args(verb, &words, &a);
            if verb == "snapshot" && !a.json {
                return portal_snapshot_print(&conn, args);
            }
            rpc_print(&conn, &format!("portal.{verb}"), args, &a)
        }
        "rpc" => {
            let Some(method) = a.pos.first() else {
                return usage("rpc needs a method name");
            };
            let args = a
                .pos
                .get(1)
                .and_then(|s| serde_json::from_str(s).ok())
                .unwrap_or_else(|| json!({}));
            rpc_print(&conn, method, args, &a)
        }
        "debug" => debug(Some(&conn), &a, None),
        "recruit" | "dismiss" | "connect" | "disconnect" | "notify" | "preset" => {
            let method = match (cmd, a.pos.first().map(String::as_str)) {
                ("preset", Some(v)) => format!("maestro.preset_{v}"),
                _ => format!("maestro.{cmd}"),
            };
            let rest = if cmd == "preset" {
                &a.pos[a.pos.len().min(1)..]
            } else {
                &a.pos[..]
            };
            rpc_print(&conn, &method, forward_args(&method, rest, &a), &a)
        }
        "role" | "routine" | "floor" | "workspace" | "partitura" | "companion" | "remote"
        | "chat" | "env" => {
            let Some(verb) = a.pos.first() else {
                return usage(&format!("{cmd} needs a verb"));
            };
            let method = method_of(cmd, verb);
            rpc_print(&conn, &method, forward_args(&method, &a.pos[1..], &a), &a)
        }
        other => usage(&format!("unknown command `{other}`")),
    }
}

fn camel(k: &str) -> String {
    let mut out = String::with_capacity(k.len());
    let mut up = false;
    for c in k.chars() {
        if c == '-' || c == '_' {
            up = true;
        } else if up {
            out.extend(c.to_uppercase());
            up = false;
        } else {
            out.push(c);
        }
    }
    out
}

fn argv_args(pos: &[String], a: &Args) -> Value {
    let mut m = Map::new();
    for (k, v) in &a.flags {
        m.insert(camel(k), v.clone());
    }
    m.insert("argv".into(), json!(pos));
    Value::Object(m)
}

fn portal_args(verb: &str, pos: &[String], a: &Args) -> (String, Value) {
    let mut v = argv_args(pos, a);
    let (verb, action) = match verb {
        "key" => ("press", None),
        "forward" | "reload" => ("back", Some(verb)),
        "create" => ("open", None),
        "logs" | "logs-start" => ("console", None),
        other => (other, None),
    };
    if let Some(act) = action {
        put(&mut v, "action", json!(act));
    }
    let mut rest = pos.iter();
    if verb == "open" {
        if let Some(u) = rest.next() {
            put(&mut v, "url", json!(u));
        }
        if let Some(n) = rest.next() {
            put(&mut v, "name", json!(n));
        }
        if let Some((w, h)) = a.flag("size").and_then(|s| s.split_once(['x', 'X'])) {
            if let (Ok(w), Ok(h)) = (w.trim().parse::<f64>(), h.trim().parse::<f64>()) {
                put(&mut v, "width", json!(w));
                put(&mut v, "height", json!(h));
            }
        }
        return (verb.to_string(), v);
    }
    let first_is_target = pos.first().is_some_and(|w| looks_like_target(w));
    if !first_is_target {
        if let Some(p) = rest.next() {
            put(&mut v, "portal", json!(p));
        }
    }
    let words: Vec<&String> = rest.collect();
    let joined = |from: usize| {
        words[from.min(words.len())..]
            .iter()
            .map(|s| s.as_str())
            .collect::<Vec<_>>()
            .join(" ")
    };
    match verb {
        "click" | "hover" | "check" | "uncheck" | "text" | "html" => {
            if let Some(sel) = words.first() {
                put(&mut v, "selector", json!(sel));
            }
        }
        "select" => {
            if let Some(sel) = words.first() {
                put(&mut v, "selector", json!(sel));
            }
            if words.len() >= 2 {
                put(&mut v, "value", json!(joined(1)));
            }
        }
        "scroll" => {
            for w in &words {
                match w.as_str() {
                    "up" | "down" | "top" | "bottom" => put(&mut v, "direction", json!(w)),
                    n if n.parse::<f64>().is_ok() => {
                        put(&mut v, "dy", json!(n.parse::<f64>().unwrap_or(0.0)))
                    }
                    sel => put(&mut v, "selector", json!(sel)),
                }
            }
        }
        "wait" => {
            if let Some(w) = words.first() {
                if let Ok(ms) = w.parse::<u64>() {
                    put(&mut v, "timeout", json!(ms));
                } else if looks_like_target(w) {
                    put(&mut v, "selector", json!(w));
                } else {
                    put(&mut v, "text", json!(joined(0)));
                }
            }
        }
        "fill" | "type" => {
            if words.len() >= 2 {
                put(&mut v, "selector", json!(words[0]));
                put(
                    &mut v,
                    "text",
                    json!(words[1..]
                        .iter()
                        .map(|s| s.as_str())
                        .collect::<Vec<_>>()
                        .join(" ")),
                );
            } else if let Some(t) = words.first() {
                put(&mut v, "text", json!(t));
            }
        }
        "press" => {
            if words.len() >= 2 {
                put(&mut v, "selector", json!(words[0]));
                put(&mut v, "key", json!(words[1]));
            } else if let Some(k) = words.first() {
                put(&mut v, "key", json!(k));
            }
        }
        "evaluate" => {
            if !words.is_empty() {
                put(
                    &mut v,
                    "code",
                    json!(words
                        .iter()
                        .map(|s| s.as_str())
                        .collect::<Vec<_>>()
                        .join(" ")),
                );
            }
        }
        "navigate" => {
            if let Some(u) = words.first() {
                match u.as_str() {
                    "back" | "forward" | "reload" => {
                        put(&mut v, "action", json!(u));
                        return ("back".to_string(), v);
                    }
                    _ => put(&mut v, "url", json!(u)),
                }
            }
        }
        "resize" => {
            if let (Some(w), Some(h)) = (words.first(), words.get(1)) {
                if let (Ok(w), Ok(h)) = (w.parse::<f64>(), h.parse::<f64>()) {
                    put(&mut v, "width", json!(w));
                    put(&mut v, "height", json!(h));
                }
            }
        }
        "ua" => {
            if let Some(u) = words.first() {
                put(&mut v, "ua", json!(u));
            }
        }
        _ => {}
    }
    (verb.to_string(), v)
}

fn put(v: &mut Value, k: &str, val: Value) {
    if let Some(m) = v.as_object_mut() {
        m.entry(k.to_string()).or_insert(val);
    }
}

fn looks_like_target(w: &str) -> bool {
    let w = w.trim();
    w.starts_with('@')
        || w.starts_with("ref=")
        || w.starts_with("text=")
        || w.starts_with('#')
        || w.starts_with('.')
        || w.starts_with('[')
        || w.split_once(',').is_some_and(|(a, b)| {
            a.trim().parse::<f64>().is_ok() && b.trim().parse::<f64>().is_ok()
        })
}

fn wants_help(args: &[String]) -> bool {
    args.iter()
        .take_while(|s| s.as_str() != "--")
        .any(|s| s == "--help" || s == "-h")
}

fn method_of(cmd: &str, verb: &str) -> String {
    match (cmd, verb) {
        ("workspace", "create") => "maestro.workspace.create".into(),
        (c, v) => format!("{c}.{v}"),
    }
}

fn positional_names(method: &str) -> &'static [&'static str] {
    match method {
        "maestro.recruit" | "maestro.dismiss" => &["name"],
        "maestro.connect" | "maestro.disconnect" => &["from", "to"],
        "maestro.swap_role" => &["name", "role"],
        "maestro.swap_agent" => &["name", "agent"],
        "maestro.workspace.create" => &["name"],
        "role.get" | "role.show" | "role.delete" => &["name"],
        "role.create" | "role.write" => &["name", "prompt"],
        "role.edit" => &["name", "old", "new"],
        "role.assign" => &["target", "role"],
        "routine.show" | "routine.edit" | "routine.enable" | "routine.disable" | "routine.run"
        | "routine.delete" | "routine.create" | "routine.history" => &["name"],
        "floor.create" | "floor.land" | "floor.delete" | "floor.run" => &["name"],
        "partitura.stamp" => &["name"],
        "partitura.list" => &["query"],
        _ => &[],
    }
}

fn forward_args(method: &str, pos: &[String], a: &Args) -> Value {
    let mut v = argv_args(pos, a);
    let names = positional_names(method);
    let Some(m) = v.as_object_mut() else {
        return v;
    };
    if method == "maestro.notify" {
        if !m.contains_key("message") && !pos.is_empty() {
            m.insert("message".into(), json!(pos.join(" ")));
        }
        return v;
    }
    for (name, word) in names.iter().zip(pos) {
        let key = camel(name);
        if !m.contains_key(&key) {
            m.insert(key, json!(word));
        }
    }
    if method == "role.assign"
        && m.get("role")
            .and_then(Value::as_str)
            .is_some_and(|r| r.eq_ignore_ascii_case("none") || r == "--none")
    {
        m.remove("role");
        m.insert("none".into(), json!(true));
    }
    v
}

fn command_help(cmd: &str, verb: Option<&str>) -> String {
    let body = match (cmd, verb) {
        ("list" | "ls" | "peers", _) => "omniget canvas list [--json]\n  Who you are (You:) and every agent, note and portal connected to you.",
        ("me" | "whoami", _) => "omniget canvas me\n  Your own node as JSON.",
        ("ask", _) => "omniget canvas ask \"Name\" \"text\" [--timeout 540] [--no-wait] [--json]\nomniget canvas ask --batch '{\"A\":\"...\",\"B\":\"...\"}'\nomniget canvas ask \"Name\" --raw \"\\e[A\\r\"\n  Types into a connected agent and waits until it is idle, then prints its answer.\n  Exit 3 = still working (never resend; use check). --raw needs the raw permission.\n  \"Name @ Workspace\" reaches an agent of another workspace through a connector.",
        ("check", _) => "omniget canvas check \"Name\" [--lines 60] [--json]\n  Reads a connected agent's screen and state without sending anything.",
        ("reply", _) => "omniget canvas reply [ask_id] \"text\" | --file answer.md\n  Answers whoever asked you (the pending ask resolves with this text).",
        ("say", _) => "omniget canvas say \"text\" [--progress] [--file f] [--text-from f] [--attach path]...\n  A chat bubble for the user; --progress does not close the turn.\n  --file and --attach attach files to the bubble; --text-from uses a text file's content as the message.",
        ("recall", _) => "omniget canvas recall [thread] [--turns N | --last day|week|month | --from DATE [--to DATE] | --all] [--json]\n  The chat thread so far.",
        ("note", _) => "omniget canvas note create [\"text\"] [--name \"Name\"] [--stack [\"Fichario\"]]\nomniget canvas note read \"Note\" [offset] [limit]\nomniget canvas note write \"Note\" \"text\"\nomniget canvas note edit \"Note\" \"old\" \"new\"\nomniget canvas note stack \"Note\" [\"Fichario\"] | unstack \"Note\" | delete \"Note\"",
        ("portal", _) => "omniget canvas portal <verb> \"Portal\" [args]\n  verbs: list, create URL [Name] [--size WxH], snapshot, click @eN|x,y, fill @eN text, type text, key K, scroll [@eN] [up|down|top|bottom|PX], hover @eN, select @eN value, check @eN, uncheck @eN, text [@eN], html [@eN], evaluate JS, logs-start, logs, screenshot, navigate URL|back|forward|reload, wait @eN|MS|text, resize W H, ua [UA]\n  The portal name can be left out when only one portal is connected to you",
        ("recruit", _) => "omniget canvas recruit \"Name\" [--agent claude|codex|gemini|opencode|ollama|shell|<preset>] [--role Role] [--command CMD] [--dir PATH] [--floor Floor] [--workspace Workspace] [--replace \"Old\"]\n  Maestro only. Creates a connected teammate under you; --role applies a role (id or name) at spawn;\n  --replace swaps an existing teammate keeping its cords and position.\n  Example: omniget canvas recruit \"Rev\" --agent claude --role reviewer",
        ("dismiss", _) => "omniget canvas dismiss \"Name\"\n  Maestro only. Stops a teammate and removes its node.",
        ("connect", _) | ("disconnect", _) => "omniget canvas connect|disconnect \"From\" \"To\"\n  Maestro only, inside your team tree.",
        ("notify", _) => "omniget canvas notify \"message\"\n  Maestro only. A system notification (up to 500 characters).",
        ("role", Some("assign")) => "omniget canvas role assign \"Agent\" \"Role\" | --none\n  Maestro only. Role by id or name (case-insensitive); the agent restarts with it, cords and position stay.",
        ("role", Some("create")) => "omniget canvas role create \"Name\" \"Prompt\" [--scope current|global] [--icon I] [--color #RRGGBB] [--agent A]",
        ("role", Some("edit")) => "omniget canvas role edit \"Name\" \"old text\" \"new text\"",
        ("role", Some("write")) => "omniget canvas role write \"Name\" \"Prompt\"",
        ("role", Some("delete")) => "omniget canvas role delete \"Name\" [--force]",
        ("role", _) => "omniget canvas role list | show \"Name\" | create \"Name\" \"Prompt\" [--scope current|global] | write \"Name\" \"Prompt\" | edit \"Name\" \"old\" \"new\" | assign \"Agent\" \"Role\"|--none | delete \"Name\" [--force]",
        ("routine", _) => "omniget canvas routine list | show \"Name\" | create \"Name\" --prompt P [--target \"Agent\"] [--every 30m | --daily 09:00 | --at RFC3339] [--pre-run CMD] | edit \"Name\" ... | enable|disable|run|delete \"Name\"",
        ("floor", _) => "omniget canvas floor list | create \"Name\" [--branch B] [--existing-branch] [--no-git] [--copy-ground] | land \"Name\" [--into B] [--delete-branch] [--keep-floor] | delete \"Name\" [--keep-branch] [--force] | run \"Name\"",
        ("workspace", _) => "omniget canvas workspace create \"Name\" --dir PATH [--from Origin] [--group G] [--folder F]\n  Maestro only.",
        ("partitura", _) => "omniget canvas partitura list [query] | stamp \"Name\"",
        ("env", _) => "omniget canvas env get | test | providers | set docker:NAME|ssh:HOST|local|workspace [--dir D] [--terminal \"Name\"|self]\n  Where your workspace's terminals run. set is Maestro only; --terminal sets one terminal's override (it restarts).",
        ("debug", _) => "omniget canvas debug\n  Transport, node id, token and whether the app answers.",
        _ => return HELP.to_string(),
    };
    body.to_string()
}

fn simple_get(conn: &Conn, path: &str, a: &Args) -> i32 {
    match conn.get(path, Duration::from_secs(15)) {
        Ok((200, v)) => {
            let _ = a.json;
            print_json(&v);
            EXIT_OK
        }
        Ok((st, v)) => {
            eprintln!("omniget canvas: {}", err_text(&v));
            exit_of(st)
        }
        Err(e) => {
            eprintln!("omniget canvas: {e}");
            EXIT_ERROR
        }
    }
}

fn list(conn: &Conn, a: &Args) -> i32 {
    let v = match conn.get("/v1/canvas/peers", Duration::from_secs(15)) {
        Ok((200, v)) => v,
        Ok((st, v)) => {
            eprintln!("omniget canvas: {}", err_text(&v));
            return exit_of(st);
        }
        Err(e) => {
            eprintln!("omniget canvas: {e}");
            return EXIT_ERROR;
        }
    };
    if a.json {
        print_json(&v);
        return EXIT_OK;
    }
    let you = &v["you"];
    let mut line = format!("You: {}", you["name"].as_str().unwrap_or("?"));
    if let Some(r) = you["role"].as_str() {
        line.push_str(&format!(" (role: {r})"));
    }
    println!("{line}");
    let peers = v["peers"].as_array().cloned().unwrap_or_default();
    if peers.is_empty() {
        println!("\nNo connected agents, notes, or portals. Ask the user to connect you to another node on the canvas.");
        return EXIT_OK;
    }
    println!();
    for p in &peers {
        print_peer(p, 0);
    }
    EXIT_OK
}

fn print_peer(p: &Value, depth: usize) {
    let pad = "  ".repeat(depth);
    let name = p["name"].as_str().unwrap_or("?");
    let kind = p["kind"].as_str().unwrap_or("?");
    let mut parts = vec![kind.to_string()];
    if let Some(ag) = p["agent"].as_str() {
        parts.push(ag.to_string());
    }
    if let Some(r) = p["role"].as_str() {
        parts.push(format!("role: {r}"));
    }
    if let Some(s) = p["state"].as_str() {
        parts.push(format!("status: {s}"));
    }
    if p["locked"] == Value::Bool(true) {
        parts.push("locked: true".into());
    }
    if let Some(u) = p["url"].as_str() {
        parts.push(u.to_string());
    }
    if let Some(perms) = p["perms"].as_array() {
        let ps: Vec<&str> = perms.iter().filter_map(|x| x.as_str()).collect();
        if !ps.is_empty() {
            parts.push(format!("can: {}", ps.join(", ")));
        }
    }
    if p["canAskYou"] == Value::Bool(true) {
        parts.push(if p["direction"] == "in" {
            "cord drawn to you: can ask you".into()
        } else {
            "can ask you".into()
        });
    }
    println!("{pad}- {name} ({})", parts.join(", "));
    if let Some(children) = p["children"].as_array() {
        for c in children {
            print_peer(c, depth + 1);
        }
    }
}

fn new_request_id() -> String {
    uuid::Uuid::new_v4().simple().to_string()
}

fn ask(conn: &Conn, a: &Args) -> i32 {
    let timeout = Duration::from_secs(a.num("timeout").unwrap_or(DEFAULT_ASK_TIMEOUT_S).max(1));
    let no_wait = a.has("no-wait");
    let request_id = new_request_id();
    let body = if let Some(batch) = a.flag("batch") {
        let Ok(Value::Object(m)) = serde_json::from_str::<Value>(batch) else {
            return usage("--batch takes a JSON object {\"Name\": \"text\", ...}");
        };
        if m.is_empty() {
            return usage("--batch is empty");
        }
        json!({ "batch": m, "request_id": request_id })
    } else {
        let Some(to) = a.pos.first() else {
            return usage("ask needs a name (see `omniget canvas list`)");
        };
        if let Some(raw) = a.flag("raw") {
            let bytes = pty::decode_escapes(raw);
            json!({ "to": to, "raw": String::from_utf8_lossy(&bytes), "request_id": request_id })
        } else {
            let Some(text) = text_arg(a.pos.get(1)).filter(|t| !t.trim().is_empty()) else {
                return usage("ask needs the text to send");
            };
            json!({ "to": to, "text": text, "request_id": request_id, "wait": !no_wait })
        }
    };
    let mut sent = None;
    for attempt in 0..3 {
        match conn.post("/v1/canvas/asks", &body, Duration::from_secs(30)) {
            Ok(r) => {
                sent = Some(r);
                break;
            }
            Err(e) if attempt == 2 => {
                eprintln!("omniget canvas: {e}");
                return EXIT_ERROR;
            }
            Err(_) => std::thread::sleep(Duration::from_millis(500)),
        }
    }
    let (status, v) = sent.expect("sent or returned");
    if !(200..300).contains(&status) {
        eprintln!("omniget canvas: {}", err_text(&v));
        return exit_of(status);
    }
    let asks: Vec<(String, String)> = v["asks"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .iter()
        .filter_map(|x| {
            Some((
                x["id"].as_str()?.to_string(),
                x["to"].as_str().unwrap_or("?").to_string(),
            ))
        })
        .collect();
    if asks.is_empty() {
        eprintln!("omniget canvas: the app admitted nothing");
        return EXIT_ERROR;
    }
    if no_wait {
        if a.json {
            print_json(&v);
        } else {
            for (id, to) in &asks {
                println!("Sent to {to} ({id}). The answer arrives as a new message.");
            }
        }
        return EXIT_OK;
    }
    let deadline = Instant::now() + timeout;
    if a.has("batch") {
        let handles: Vec<_> = asks
            .into_iter()
            .map(|(id, to)| {
                let c = conn.clone();
                std::thread::spawn(move || (to, wait_ask(&c, &id, deadline)))
            })
            .collect();
        let mut out = Vec::new();
        let mut code = EXIT_OK;
        for h in handles {
            let Ok((to, w)) = h.join() else { continue };
            match w {
                Waited::Answered(ans) => {
                    out.push(json!({ "name": to, "answer": ans["text"], "source": ans["source"] }))
                }
                Waited::Released(msg) => {
                    code = code.max(EXIT_RELEASED);
                    out.push(json!({ "name": to, "error": "released", "message": msg }))
                }
                Waited::Pending => {
                    code = code.max(EXIT_NOT_FINISHED);
                    out.push(json!({ "name": to, "error": "not_finished" }))
                }
                Waited::Failed(e, c) => {
                    code = code.max(c);
                    out.push(json!({ "name": to, "error": e }))
                }
            }
        }
        print_json(&Value::Array(out));
        return code;
    }
    let (id, to) = asks.into_iter().next().expect("one ask");
    match wait_ask(conn, &id, deadline) {
        Waited::Answered(ans) => {
            if a.json {
                print_json(&ans);
            } else {
                println!("{}", ans["text"].as_str().unwrap_or(""));
            }
            EXIT_OK
        }
        Waited::Released(msg) => {
            println!("{msg}");
            EXIT_RELEASED
        }
        Waited::Pending => {
            println!(
                "'{to}' has NOT finished yet: it received your message and keeps working. The answer arrives later as a new message, or run `omniget canvas check \"{to}\"` to see its progress. Do not send the question again."
            );
            EXIT_NOT_FINISHED
        }
        Waited::Failed(e, code) => {
            eprintln!("omniget canvas: {e}");
            code
        }
    }
}

enum Waited {
    Answered(Value),
    Released(String),
    Pending,
    Failed(String, i32),
}

fn wait_ask(conn: &Conn, id: &str, deadline: Instant) -> Waited {
    let mut errors = 0;
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return Waited::Pending;
        }
        let slice = (left.as_millis() as u64).clamp(1_000, WAIT_SLICE_MS);
        let path = format!("/v1/canvas/asks/{id}/wait?timeout_ms={slice}");
        match conn.get(&path, Duration::from_millis(slice + 15_000)) {
            Ok((200, v)) => match v["state"].as_str() {
                Some("answered") => return Waited::Answered(v["answer"].clone()),
                Some("released") => {
                    let by = v["by"].as_str().unwrap_or("another agent");
                    return Waited::Released(format!(
                        "OK: releasing this ask to prevent a deadlock: '{by}' is messaging you, and that message was blocked behind this command by a cycle of pending asks. Your question stays alive; its answer arrives later as a new message."
                    ));
                }
                Some("failed") => {
                    let e = v["error"].as_str().unwrap_or("failed").to_string();
                    let code = if matches!(e.as_str(), "denied" | "target_is_shell")
                        || e.starts_with("limit_")
                    {
                        EXIT_USAGE
                    } else {
                        EXIT_ERROR
                    };
                    let msg = v["message"]
                        .as_str()
                        .map(|m| format!("{e}: {m}"))
                        .unwrap_or(e);
                    return Waited::Failed(msg, code);
                }
                _ => {}
            },
            Ok((408, _)) => {}
            Ok((st, v)) => return Waited::Failed(err_text(&v), exit_of(st)),
            Err(e) => {
                errors += 1;
                if errors > 5 {
                    return Waited::Failed(e, EXIT_ERROR);
                }
                std::thread::sleep(Duration::from_secs(1));
            }
        }
    }
}

fn check(conn: &Conn, a: &Args) -> i32 {
    let Some(name) = a.pos.first() else {
        return usage("check needs a name");
    };
    let lines = a.num("lines").unwrap_or(60);
    let path = format!("/v1/canvas/screen?name={}&lines={lines}", urlencode(name));
    match conn.get(&path, Duration::from_secs(15)) {
        Ok((200, v)) => {
            if a.json {
                print_json(&v);
            } else {
                println!(
                    "[{name}: {}]\n{}",
                    v["state"].as_str().unwrap_or("?"),
                    v["text"].as_str().unwrap_or("").trim_end()
                );
            }
            EXIT_OK
        }
        Ok((st, v)) => {
            eprintln!("omniget canvas: {}", err_text(&v));
            exit_of(st)
        }
        Err(e) => {
            eprintln!("omniget canvas: {e}");
            EXIT_ERROR
        }
    }
}

fn reply(conn: &Conn, a: &Args) -> i32 {
    let (ask_id, text_pos) = match a.pos.first() {
        Some(p) if p.starts_with("ask_") => (Some(p.clone()), a.pos.get(1)),
        _ => (None, a.pos.first()),
    };
    let text = if let Some(f) = a.flag("file") {
        match std::fs::read_to_string(f) {
            Ok(t) => t,
            Err(e) => return usage(&format!("cannot read {f}: {e}")),
        }
    } else {
        match text_arg(text_pos) {
            Some(t) if !t.trim().is_empty() => t,
            _ => return usage("reply needs the text (or --file)"),
        }
    };
    let body = json!({ "ask_id": ask_id, "text": text, "file": a.flag("file") });
    match conn.post("/v1/canvas/reply", &body, Duration::from_secs(30)) {
        Ok((200, v)) => {
            if a.json {
                print_json(&v);
            } else {
                println!("Replied ({}).", v["resolved"].as_str().unwrap_or("?"));
            }
            EXIT_OK
        }
        Ok((st, v)) => {
            eprintln!("omniget canvas: {}", err_text(&v));
            exit_of(st)
        }
        Err(e) => {
            eprintln!("omniget canvas: {e}");
            EXIT_ERROR
        }
    }
}

fn say(conn: &Conn, a: &Args) -> i32 {
    let text = text_arg(a.pos.first()).unwrap_or_default();
    if text.trim().is_empty() && !a.has("file") && !a.has("text-from") && !a.has("attach") {
        return usage("say needs the text (or --file / --text-from)");
    }
    let mut args = a.flags.clone();
    args.insert("text".into(), json!(text));
    args.insert("progress".into(), json!(a.has("progress")));
    match args.get("attach").cloned() {
        Some(Value::String(one)) => {
            args.insert("attach".into(), json!([one]));
        }
        Some(Value::Bool(_)) => {
            return usage("--attach needs a path");
        }
        _ => {}
    }
    if !args.contains_key("cwd") {
        if let Ok(dir) = std::env::current_dir() {
            args.insert("cwd".into(), json!(dir.to_string_lossy()));
        }
    }
    rpc_print(conn, "chat.say", Value::Object(args), a)
}

fn portal_snapshot_print(conn: &Conn, args: Value) -> i32 {
    match rpc(conn, "portal.snapshot", args) {
        Ok(v) => {
            print!("{}", snapshot_text(&v));
            EXIT_OK
        }
        Err((e, code)) => {
            eprintln!("omniget canvas: {e}");
            code
        }
    }
}

fn snapshot_text(v: &Value) -> String {
    let mut out = String::new();
    let head: Vec<&str> = [v["title"].as_str(), v["url"].as_str()]
        .into_iter()
        .flatten()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect();
    if !head.is_empty() {
        out.push_str(&format!("# {}\n", head.join(" ")));
    }
    if let Some(note) = v["note"].as_str().filter(|n| !n.trim().is_empty()) {
        out.push_str(&format!("({})\n", note.trim()));
    }
    let body = v["tree"]
        .as_str()
        .filter(|t| !t.trim().is_empty())
        .or_else(|| v["text"].as_str())
        .unwrap_or("");
    out.push_str(body.trim_end());
    out.push('\n');
    out
}

fn recall_args(a: &Args) -> Value {
    let mut m = a.flags.clone();
    match a.pos.first().map(String::as_str) {
        Some("list") => {
            m.insert("list".into(), json!(true));
        }
        Some("attachment") => {
            if let Some(id) = a.pos.get(1) {
                m.insert("attachment".into(), json!(id));
            }
        }
        Some(t) => {
            m.insert("thread".into(), json!(t));
        }
        None => {}
    }
    if let Some(n) = a.num("turns") {
        m.insert("turns".into(), json!(n));
    }
    m.insert("argv".into(), json!(a.pos));
    Value::Object(m)
}

fn note(conn: &Conn, a: &Args) -> i32 {
    let Some(verb) = a.pos.first().map(String::as_str) else {
        return usage("note needs a verb: create, read, write, edit, stack, unstack, delete");
    };
    let p = &a.pos[1..];
    let name = p.first().cloned();
    let args = match verb {
        "create" => {
            let stack = a.flags.get("stack").map(|v| match v {
                Value::String(s) => json!(s),
                _ => json!(true),
            });
            json!({
                "text": text_arg(p.first()).unwrap_or_default(),
                "name": a.flag("name"),
                "stack": stack,
            })
        }
        "read" => json!({
            "name": name,
            "offset": p.get(1).and_then(|s| s.parse::<u64>().ok()).or(a.num("offset")),
            "limit": p.get(2).and_then(|s| s.parse::<u64>().ok()).or(a.num("limit")),
        }),
        "write" => {
            let text = if let Some(f) = a.flag("file") {
                std::fs::read_to_string(f).ok()
            } else {
                text_arg(p.get(1))
            };
            let Some(text) = text else {
                return usage("note write needs the content");
            };
            json!({ "name": name, "text": text })
        }
        "edit" => {
            let (Some(old), Some(new)) = (p.get(1), p.get(2)) else {
                return usage("note edit \"Note\" \"old text\" \"new text\"");
            };
            json!({ "name": name, "old": pty::decode_text(old), "new": pty::decode_text(new) })
        }
        "stack" => json!({ "name": name, "stack": p.get(1) }),
        "unstack" | "delete" => json!({ "name": name }),
        "list" => json!({}),
        other => return usage(&format!("unknown note verb `{other}`")),
    };
    if verb != "create" && verb != "list" && name.is_none() {
        return usage(&format!("note {verb} needs the note's name"));
    }
    rpc_print(conn, &format!("note.{verb}"), args, a)
}

pub fn rpc(conn: &Conn, method: &str, args: Value) -> Result<Value, (String, i32)> {
    let body = json!({ "method": method, "args": args });
    match conn.post("/v1/canvas/rpc", &body, Duration::from_secs(130)) {
        Ok((200, v)) => Ok(v.get("result").cloned().unwrap_or(v)),
        Ok((st, v)) => Err((err_text(&v), exit_of(st))),
        Err(e) => Err((e, EXIT_ERROR)),
    }
}

fn rpc_print(conn: &Conn, method: &str, args: Value, a: &Args) -> i32 {
    match rpc(conn, method, args) {
        Ok(v) => {
            if a.json {
                print_json(&v);
            } else if let Some(s) = v.as_str() {
                println!("{s}");
            } else if let Some(s) = v.get("text").and_then(|t| t.as_str()) {
                println!("{s}");
            } else if let Some(s) = v.get("message").and_then(|t| t.as_str()) {
                println!("{s}");
            } else if !v.is_null() {
                print_json(&v);
            }
            EXIT_OK
        }
        Err((e, code)) => {
            eprintln!("omniget canvas: {e}");
            code
        }
    }
}

fn debug(conn: Option<&Conn>, a: &Args, env_err: Option<String>) -> i32 {
    let node = std::env::var(NODE_ENV).unwrap_or_default();
    let mut report = json!({
        "node": node,
        "bridge_url": std::env::var(URL_ENV).ok(),
        "cli": std::env::var("OMNIGET_CLI").ok(),
        "terminal": std::env::var("OMNIGET_TERMINAL_ID").ok(),
        "token": std::env::var(TOKEN_ENV).ok().map(|t| t.chars().take(16).collect::<String>() + "…"),
    });
    let mut code = EXIT_OK;
    if let Some(e) = env_err {
        report["error"] = json!(e);
        code = EXIT_USAGE;
    }
    if let Some(conn) = conn {
        report["bridge"] = json!(conn.url());
        match conn.get("/v1/canvas/me", Duration::from_secs(5)) {
            Ok((200, v)) => {
                report["app"] = json!("answering");
                report["token_valid"] = json!(true);
                report["me"] = v;
            }
            Ok((st, v)) => {
                report["app"] = json!("answering");
                report["token_valid"] = json!(false);
                report["error"] = json!(format!("{st}: {}", err_text(&v)));
                code = EXIT_USAGE;
            }
            Err(e) => {
                report["app"] = json!("not answering");
                report["error"] = json!(e);
                code = EXIT_ERROR;
            }
        }
    }
    if a.json {
        print_json(&report);
    } else {
        for (k, v) in report.as_object().into_iter().flatten() {
            let s = match v {
                Value::String(s) => s.clone(),
                Value::Null => "-".into(),
                other => other.to_string(),
            };
            println!("{k}: {s}");
        }
    }
    code
}

fn urlencode(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

pub fn hook_main(args: &[String]) -> i32 {
    let a = parse(args);
    let tool = a.flag("tool").unwrap_or("claude").to_string();
    let Ok(conn) = Conn::from_env() else {
        return EXIT_OK;
    };
    let (event, mut payload) = match tool.as_str() {
        "codex" => {
            let raw = a
                .pos
                .last()
                .and_then(|s| serde_json::from_str::<Value>(s).ok());
            match raw.as_ref().and_then(codex::notify_to_hook) {
                Some(x) => x,
                None => return EXIT_OK,
            }
        }
        _ => {
            let mut stdin = Vec::new();
            let _ = std::io::stdin().read_to_end(&mut stdin);
            let raw: Value = serde_json::from_slice(&stdin).unwrap_or_else(|_| json!({}));
            let native = raw
                .get("hook_event_name")
                .or_else(|| raw.get("hookEventName"))
                .and_then(|e| e.as_str())
                .unwrap_or("")
                .to_string();
            let event = if tool == "gemini" {
                gemini::claude_event(&native).unwrap_or("").to_string()
            } else {
                native
            };
            if event.is_empty() {
                return EXIT_OK;
            }
            let mut v = raw;
            if let Some(m) = v.as_object_mut() {
                m.insert("hook_event_name".into(), json!(event));
            }
            (event, v)
        }
    };
    let hold = event == "PermissionRequest";
    if let Some(m) = payload.as_object_mut() {
        m.insert(
            "omniget".into(),
            json!({ "tool": tool, "hold": hold, "wait_ms": hook_shim::DEFAULT_WAIT_SECS * 1000 }),
        );
    }
    let timeout = if hold {
        Duration::from_secs(hook_shim::MAX_WAIT_SECS + 10)
    } else {
        Duration::from_secs(2)
    };
    let resp = conn.post("/v1/canvas/hook", &payload, timeout);
    if hold && tool == "claude" {
        let decision = match resp
            .as_ref()
            .ok()
            .and_then(|(_, v)| v["decision"].as_str().map(str::to_string))
            .as_deref()
        {
            Some("allow") => hook_shim::Decision::Allow,
            Some("deny") => hook_shim::Decision::Deny,
            _ => hook_shim::Decision::None,
        };
        let verdict = hook_shim::Verdict {
            decision,
            reason: if decision == hook_shim::Decision::Deny {
                "Denied in OmniGet".into()
            } else {
                String::new()
            },
            ..Default::default()
        };
        let out = hook_shim::render_claude(&event, &verdict);
        if !out.stdout.is_empty() {
            println!("{}", out.stdout);
        }
    }
    EXIT_OK
}

pub fn multicall_main() -> i32 {
    let argv: Vec<String> = std::env::args().collect();
    if argv.get(1).map(String::as_str) == Some("canvas") {
        return run_cli(&argv[2..]);
    }
    let me = std::env::current_exe()
        .ok()
        .and_then(|p| p.canonicalize().ok());
    if let Some(next) = next_on_path("omniget", me.as_deref()) {
        let status = std::process::Command::new(next).args(&argv[1..]).status();
        return match status {
            Ok(s) => s.code().unwrap_or(EXIT_ERROR),
            Err(e) => {
                eprintln!("omniget: {e}");
                EXIT_ERROR
            }
        };
    }
    eprintln!(
        "omniget: this is the canvas helper of the OmniGet app; it only knows `omniget canvas …` (the full CLI is not installed).\n\n{HELP}"
    );
    EXIT_USAGE
}

fn next_on_path(name: &str, skip: Option<&std::path::Path>) -> Option<PathBuf> {
    let file = if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.to_string()
    };
    let path = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path) {
        let cand = dir.join(&file);
        if !cand.is_file() {
            continue;
        }
        let canon = cand.canonicalize().unwrap_or_else(|_| cand.clone());
        if Some(canon.as_path()) == skip {
            continue;
        }
        return Some(cand);
    }
    None
}

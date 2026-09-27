//! Dev-server discovery for the thread's Browser tab: which processes whose
//! working directory is inside the thread folder are listening on a local TCP
//! port, confirmed by an HTTP probe that answers HTML.
//!
//! macOS/Linux: `lsof -nP -iTCP -sTCP:LISTEN -F pcn` for the listeners and
//! `lsof -a -d cwd -Fn -p <pids>` (Linux: `/proc/<pid>/cwd` first) for the
//! folder; `ss -ltnpH` when lsof is missing on Linux. Windows: `netstat -ano`
//! plus the process command line (no cwd there), matched by path prefix.
//! Runs only when the tab asks: nothing polls.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::Serialize;

use omniget_core::core::process;

/// One listening socket as the OS reports it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Listener {
    pub pid: u32,
    pub command: String,
    /// The bound host as printed (`127.0.0.1`, `*`, `::1`, `0.0.0.0`).
    pub host: String,
    pub port: u16,
}

/// What the tab shows for one port.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DevServer {
    pub port: u16,
    pub pid: u32,
    pub command: String,
    pub cwd: Option<String>,
    /// The URL the Browser tab opens.
    pub url: String,
    /// The process folder is inside the thread folder (or its command line
    /// names it, on Windows).
    pub in_workspace: bool,
    /// The probe answered HTML.
    pub html: bool,
    pub status: Option<u16>,
    pub title: Option<String>,
}

// ── Parsers (pure) ────────────────────────────────────────────────────────

/// Splits `127.0.0.1:5173`, `*:8000`, `[::1]:3000`, `::1.3000` style names.
pub fn split_host_port(name: &str) -> Option<(String, u16)> {
    let name = name.trim();
    // lsof may append " (LISTEN)" when -F is not used; be tolerant.
    let name = name.split_whitespace().next()?;
    let name = name.split("->").next()?;
    let (host, port) = if let Some(rest) = name.strip_prefix('[') {
        let (h, p) = rest.split_once("]:")?;
        (h.to_string(), p)
    } else {
        let idx = name.rfind(':')?;
        (name[..idx].to_string(), &name[idx + 1..])
    };
    let port: u16 = port.parse().ok()?;
    Some((host, port))
}

/// `lsof -F pcn` output: `p<pid>`, `c<command>`, `f<fd>`, `n<name>` lines.
pub fn parse_lsof_listen(out: &str) -> Vec<Listener> {
    let mut res = Vec::new();
    let mut pid = 0u32;
    let mut command = String::new();
    for line in out.lines() {
        let Some(tag) = line.chars().next() else {
            continue;
        };
        let val = &line[tag.len_utf8()..];
        match tag {
            'p' => {
                pid = val.trim().parse().unwrap_or(0);
                command.clear();
            }
            'c' => command = val.to_string(),
            'n' => {
                if let Some((host, port)) = split_host_port(val) {
                    if pid != 0 {
                        res.push(Listener {
                            pid,
                            command: command.clone(),
                            host,
                            port,
                        });
                    }
                }
            }
            _ => {}
        }
    }
    dedupe(res)
}

/// `lsof -a -d cwd -Fn -p ...` output: `p<pid>`, `fcwd`, `n<path>`.
pub fn parse_lsof_cwd(out: &str) -> HashMap<u32, String> {
    let mut map = HashMap::new();
    let mut pid = 0u32;
    for line in out.lines() {
        if let Some(v) = line.strip_prefix('p') {
            pid = v.trim().parse().unwrap_or(0);
        } else if let Some(v) = line.strip_prefix('n') {
            if pid != 0 && !v.is_empty() {
                map.insert(pid, v.to_string());
            }
        }
    }
    map
}

/// `ss -ltnpH`: `LISTEN 0 511 127.0.0.1:5173 0.0.0.0:* users:(("node",pid=12,fd=20))`.
pub fn parse_ss(out: &str) -> Vec<Listener> {
    let mut res = Vec::new();
    for line in out.lines() {
        let cols: Vec<&str> = line.split_whitespace().collect();
        if cols.len() < 5 {
            continue;
        }
        // With -H the first column is the state; tolerate the netid column.
        let local = if cols[0] == "tcp" {
            cols.get(4)
        } else {
            cols.get(3)
        };
        let Some((host, port)) = local.and_then(|l| split_host_port(l)) else {
            continue;
        };
        let users = line.find("users:((").map(|i| &line[i..]).unwrap_or("");
        let mut rest = users;
        let mut any = false;
        while let Some(start) = rest.find("(\"") {
            let chunk = &rest[start + 2..];
            let Some(qend) = chunk.find('"') else { break };
            let command = chunk[..qend].to_string();
            let after = &chunk[qend..];
            let pid = after
                .find("pid=")
                .map(|i| &after[i + 4..])
                .and_then(|s| s.split(|c: char| !c.is_ascii_digit()).next())
                .and_then(|s| s.parse::<u32>().ok());
            if let Some(pid) = pid {
                res.push(Listener {
                    pid,
                    command,
                    host: host.clone(),
                    port,
                });
                any = true;
            }
            rest = &chunk[qend..];
            let Some(close) = rest.find(')') else { break };
            rest = &rest[close..];
        }
        if !any {
            res.push(Listener {
                pid: 0,
                command: String::new(),
                host,
                port,
            });
        }
    }
    dedupe(res)
}

/// `netstat -ano`: `  TCP    127.0.0.1:5173   0.0.0.0:0   LISTENING   1234`.
pub fn parse_netstat(out: &str) -> Vec<Listener> {
    let mut res = Vec::new();
    for line in out.lines() {
        let cols: Vec<&str> = line.split_whitespace().collect();
        if cols.len() < 5 || !cols[0].eq_ignore_ascii_case("TCP") {
            continue;
        }
        if !cols[3].eq_ignore_ascii_case("LISTENING") {
            continue;
        }
        let Some((host, port)) = split_host_port(cols[1]) else {
            continue;
        };
        let pid: u32 = cols[4].parse().unwrap_or(0);
        res.push(Listener {
            pid,
            command: String::new(),
            host,
            port,
        });
    }
    dedupe(res)
}

fn dedupe(v: Vec<Listener>) -> Vec<Listener> {
    let mut seen = HashSet::new();
    v.into_iter()
        .filter(|l| seen.insert((l.pid, l.port)))
        .collect()
}

/// A listener reachable from this machine's loopback.
pub fn is_local_bind(host: &str) -> bool {
    matches!(
        host,
        "*" | "0.0.0.0" | "::" | "127.0.0.1" | "::1" | "localhost" | "[::]" | "::ffff:127.0.0.1"
    ) || host.starts_with("127.")
}

/// The URL the probe and the tab use for a bound host.
pub fn url_for(host: &str, port: u16) -> String {
    if host == "::1" {
        format!("http://[::1]:{port}/")
    } else if host.starts_with("127.") && host != "127.0.0.1" {
        format!("http://{host}:{port}/")
    } else {
        format!("http://localhost:{port}/")
    }
}

/// `path` is `root` or below it (component-wise, case-insensitive on Windows).
pub fn is_inside(path: &Path, root: &Path) -> bool {
    if cfg!(windows) {
        let p = path.to_string_lossy().replace('/', "\\").to_lowercase();
        let r = root.to_string_lossy().replace('/', "\\").to_lowercase();
        let r = r.trim_end_matches('\\');
        return p == r || p.starts_with(&format!("{r}\\"));
    }
    path.starts_with(root)
}

/// `<title>` of an HTML body, whitespace collapsed.
pub fn html_title(body: &str) -> Option<String> {
    let lower = body.to_ascii_lowercase();
    let start = lower.find("<title")?;
    let open_end = lower[start..].find('>')? + start + 1;
    let close = lower[open_end..].find("</title")? + open_end;
    let t = body
        .get(open_end..close)?
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    (!t.is_empty()).then(|| t.chars().take(160).collect())
}

pub fn looks_like_html(content_type: &str, body: &str) -> bool {
    if content_type.to_ascii_lowercase().contains("text/html") {
        return true;
    }
    let head = body
        .trim_start()
        .get(..64)
        .unwrap_or(body.trim_start())
        .to_ascii_lowercase();
    head.starts_with("<!doctype html") || head.starts_with("<html")
}

// ── OS calls ──────────────────────────────────────────────────────────────

async fn run(program: &str, args: &[&str]) -> Option<String> {
    let mut cmd = process::command(program);
    cmd.args(args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true);
    let child = process::spawn_retrying_busy(|| cmd.spawn()).ok()?;
    let out = tokio::time::timeout(Duration::from_secs(8), child.wait_with_output())
        .await
        .ok()?
        .ok()?;
    // lsof exits 1 when a filter matches nothing but still prints the rest.
    Some(String::from_utf8_lossy(&out.stdout).into_owned())
}

async fn listeners() -> Result<Vec<Listener>, String> {
    if cfg!(windows) {
        let out = run("netstat", &["-ano"])
            .await
            .ok_or("PREVIEW_PORTS: netstat failed")?;
        return Ok(parse_netstat(&out));
    }
    if let Some(out) = run("lsof", &["-nP", "-iTCP", "-sTCP:LISTEN", "-F", "pcn"]).await {
        let v = parse_lsof_listen(&out);
        if !v.is_empty() || !cfg!(target_os = "linux") {
            return Ok(v);
        }
    }
    if cfg!(target_os = "linux") {
        if let Some(out) = run("ss", &["-ltnpH"]).await {
            return Ok(parse_ss(&out));
        }
    }
    Err("PREVIEW_PORTS: neither lsof nor ss is available".into())
}

async fn cwds(pids: &[u32]) -> HashMap<u32, String> {
    let mut map = HashMap::new();
    if pids.is_empty() || cfg!(windows) {
        return map;
    }
    #[cfg(target_os = "linux")]
    for pid in pids {
        if let Ok(p) = std::fs::read_link(PathBuf::from("/proc").join(pid.to_string()).join("cwd"))
        {
            map.insert(*pid, p.to_string_lossy().into_owned());
        }
    }
    let missing: Vec<String> = pids
        .iter()
        .filter(|p| !map.contains_key(p))
        .map(|p| p.to_string())
        .collect();
    if missing.is_empty() {
        return map;
    }
    let list = missing.join(",");
    if let Some(out) = run("lsof", &["-a", "-d", "cwd", "-Fn", "-p", &list]).await {
        map.extend(parse_lsof_cwd(&out));
    }
    map
}

/// Windows: name and command line per pid (there is no cwd to read).
async fn windows_processes(pids: &[u32]) -> HashMap<u32, (String, String)> {
    let mut map = HashMap::new();
    if pids.is_empty() {
        return map;
    }
    let filter = pids
        .iter()
        .map(|p| format!("ProcessId={p}"))
        .collect::<Vec<_>>()
        .join(" OR ");
    let script = format!(
        "Get-CimInstance Win32_Process -Filter \"{filter}\" | Select-Object ProcessId,Name,CommandLine | ConvertTo-Json -Compress"
    );
    let Some(out) = run(
        "powershell",
        &["-NoProfile", "-NonInteractive", "-Command", &script],
    )
    .await
    else {
        return map;
    };
    let v: serde_json::Value = serde_json::from_str(out.trim()).unwrap_or(serde_json::Value::Null);
    let items = match v {
        serde_json::Value::Array(a) => a,
        serde_json::Value::Object(_) => vec![v],
        _ => vec![],
    };
    for it in items {
        let pid = it.get("ProcessId").and_then(|x| x.as_u64()).unwrap_or(0) as u32;
        let name = it
            .get("Name")
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .to_string();
        let cmd = it
            .get("CommandLine")
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .to_string();
        map.insert(pid, (name, cmd));
    }
    map
}

/// GETs the URL briefly; `(status, html, title)`.
pub async fn probe(url: &str) -> (Option<u16>, bool, Option<String>) {
    let client = match reqwest::Client::builder()
        .timeout(Duration::from_millis(1500))
        .connect_timeout(Duration::from_millis(600))
        .redirect(reqwest::redirect::Policy::limited(3))
        .no_proxy()
        .build()
    {
        Ok(c) => c,
        Err(_) => return (None, false, None),
    };
    let Ok(resp) = client
        .get(url)
        .header("Accept", "text/html,*/*;q=0.8")
        .send()
        .await
    else {
        return (None, false, None);
    };
    let status = resp.status().as_u16();
    let ct = resp
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    let mut body = Vec::new();
    let mut resp = resp;
    while let Ok(Some(chunk)) = resp.chunk().await {
        body.extend_from_slice(&chunk);
        if body.len() > 64 * 1024 {
            break;
        }
    }
    let text = String::from_utf8_lossy(&body);
    let html = looks_like_html(&ct, &text);
    (
        Some(status),
        html,
        if html { html_title(&text) } else { None },
    )
}

/// Listening local ports whose process runs inside `root`. With `all`, the
/// other local listeners come too (flagged `in_workspace: false`), so the tab
/// can offer a server started elsewhere. Our own process is left out.
pub async fn discover(root: Option<&Path>, all: bool) -> Result<Vec<DevServer>, String> {
    let own = std::process::id();
    let listeners: Vec<Listener> = listeners()
        .await?
        .into_iter()
        .filter(|l| l.pid != own && is_local_bind(&l.host))
        .collect();
    let pids: Vec<u32> = listeners
        .iter()
        .map(|l| l.pid)
        .filter(|p| *p != 0)
        .collect::<HashSet<_>>()
        .into_iter()
        .collect();

    let root = root.map(|r| std::fs::canonicalize(r).unwrap_or_else(|_| r.to_path_buf()));
    let cwd_map = cwds(&pids).await;
    let win = if cfg!(windows) {
        windows_processes(&pids).await
    } else {
        HashMap::new()
    };

    // One entry per port (IPv4 and IPv6 binds of the same server collapse).
    let mut by_port: BTreeMap<u16, DevServer> = BTreeMap::new();
    for l in listeners {
        let cwd = cwd_map.get(&l.pid).cloned();
        let (command, cmdline) = match win.get(&l.pid) {
            Some((n, c)) => (n.clone(), c.clone()),
            None => (l.command.clone(), String::new()),
        };
        let in_workspace = match &root {
            None => false,
            Some(r) => {
                cwd.as_deref()
                    .map(|c| {
                        let p = PathBuf::from(c);
                        is_inside(&std::fs::canonicalize(&p).unwrap_or(p), r)
                    })
                    .unwrap_or(false)
                    || (!cmdline.is_empty()
                        && cmdline
                            .to_lowercase()
                            .contains(&r.to_string_lossy().to_lowercase()))
            }
        };
        if !in_workspace && !all {
            continue;
        }
        let entry = DevServer {
            port: l.port,
            pid: l.pid,
            command,
            cwd,
            url: url_for(&l.host, l.port),
            in_workspace,
            html: false,
            status: None,
            title: None,
        };
        match by_port.get(&l.port) {
            Some(prev) if prev.in_workspace || !entry.in_workspace => {}
            _ => {
                by_port.insert(l.port, entry);
            }
        }
    }

    // Probe at most 48 ports, concurrently.
    let mut list: Vec<DevServer> = by_port.into_values().collect();
    list.sort_by_key(|d| (!d.in_workspace, d.port));
    list.truncate(48);
    let probes = futures::future::join_all(list.iter().map(|d| {
        let url = d.url.clone();
        async move { probe(&url).await }
    }))
    .await;
    for (d, (status, html, title)) in list.iter_mut().zip(probes) {
        d.status = status;
        d.html = html;
        d.title = title;
    }
    // Workspace HTML servers first, then other HTML, then the rest.
    list.sort_by_key(|d| (!d.in_workspace, !d.html, d.port));
    Ok(list)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lsof_listen_parses_pid_command_and_ports() {
        let out = "p815\ncSpotify\nf100\nn127.0.0.1:7768\nf207\nn*:53822\np906\ncnode\nf20\nn[::1]:5173\n";
        let v = parse_lsof_listen(out);
        assert_eq!(v.len(), 3);
        assert_eq!(
            v[0],
            Listener {
                pid: 815,
                command: "Spotify".into(),
                host: "127.0.0.1".into(),
                port: 7768
            }
        );
        assert_eq!(v[1].host, "*");
        assert_eq!(
            v[2],
            Listener {
                pid: 906,
                command: "node".into(),
                host: "::1".into(),
                port: 5173
            }
        );
    }

    #[test]
    fn lsof_cwd_maps_pid_to_path() {
        let m = parse_lsof_cwd("p12\nfcwd\nn/Users/x/app\np13\nfcwd\nn/tmp\n");
        assert_eq!(m.get(&12).unwrap(), "/Users/x/app");
        assert_eq!(m.get(&13).unwrap(), "/tmp");
    }

    #[test]
    fn ss_and_netstat_parse() {
        let ss = "LISTEN 0      511        127.0.0.1:5173      0.0.0.0:*    users:((\"node\",pid=4242,fd=20))\nLISTEN 0 4096 [::]:22 [::]:*\n";
        let v = parse_ss(ss);
        assert_eq!(
            v[0],
            Listener {
                pid: 4242,
                command: "node".into(),
                host: "127.0.0.1".into(),
                port: 5173
            }
        );
        assert_eq!(v[1].port, 22);
        let ns = "  Proto  Local Address          Foreign Address        State           PID\n  TCP    0.0.0.0:135            0.0.0.0:0              LISTENING       1100\n  TCP    127.0.0.1:3000         0.0.0.0:0              LISTENING       9\n  TCP    127.0.0.1:3000         127.0.0.1:5000         ESTABLISHED     9\n  TCP    [::1]:5173             [::]:0                 LISTENING       77\n";
        let v = parse_netstat(ns);
        assert_eq!(v.len(), 3);
        assert_eq!(
            v[2],
            Listener {
                pid: 77,
                command: String::new(),
                host: "::1".into(),
                port: 5173
            }
        );
    }

    #[test]
    fn html_helpers() {
        assert!(looks_like_html("text/html; charset=utf-8", ""));
        assert!(looks_like_html("", "  <!DOCTYPE html><html>"));
        assert!(!looks_like_html("application/json", "{}"));
        assert_eq!(
            html_title("<html><head><TITLE>\n Vite  App </title>").as_deref(),
            Some("Vite App")
        );
        assert_eq!(url_for("::1", 5173), "http://[::1]:5173/");
        assert_eq!(url_for("*", 8000), "http://localhost:8000/");
        assert!(is_local_bind("*") && is_local_bind("127.0.0.1") && !is_local_bind("192.168.0.4"));
    }

    /// Live: `PREVIEW_TEST_DIR=<dir with a server running> cargo test -p omniget --lib live_discover -- --ignored --nocapture`.
    #[tokio::test]
    #[ignore]
    async fn live_discover() {
        let dir = std::env::var("PREVIEW_TEST_DIR").expect("PREVIEW_TEST_DIR");
        let list = discover(Some(Path::new(&dir)), false).await.unwrap();
        println!("{}", serde_json::to_string_pretty(&list).unwrap());
        assert!(list.iter().any(|d| d.in_workspace && d.html));
    }
}

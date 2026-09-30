//! Connectors: a folder with `manifest.json` and a script. The script defines
//! `resolve(url, ctx)` and returns a tree (`{ title, children: [...] }`). It
//! runs in QuickJS with no file system and no sockets: `ctx.fetch` reaches
//! only the manifest's `domains`, going through a browser tab of the same
//! origin when one is open.
//!
//! Installed connectors live in `<data>/global-downloader/connectors/<id>`;
//! a developer folder is linked in place (`connectors.json` → `dev`).

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::{browser, data_dir, host_matches, host_of, sniffer, Node, Source};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Manifest {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub author: String,
    #[serde(default)]
    pub license: String,
    #[serde(default)]
    pub homepage: String,
    #[serde(default)]
    pub description: String,
    pub domains: Vec<String>,
    #[serde(rename = "match")]
    pub matches: Vec<String>,
    #[serde(default)]
    pub login: Option<Value>,
    #[serde(default = "default_entry")]
    pub entry: String,
    #[serde(default)]
    pub min_omniget: String,
    /// Settings the person can change, shown by the app and passed to the
    /// script as `ctx.options`: `[{ id, label, type: boolean|string|select,
    /// default, choices? }]`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub options: Vec<Value>,
}

fn default_entry() -> String {
    "connector.js".into()
}

#[derive(Debug, Clone, Serialize)]
pub struct Connector {
    pub manifest: Manifest,
    pub dir: PathBuf,
    pub dev: bool,
}

fn installed_dir() -> PathBuf {
    let d = data_dir().join("connectors");
    let _ = std::fs::create_dir_all(&d);
    d
}

fn dev_list_file() -> PathBuf {
    data_dir().join("connectors.json")
}

fn dev_dirs() -> Vec<PathBuf> {
    std::fs::read_to_string(dev_list_file())
        .ok()
        .and_then(|t| serde_json::from_str::<Value>(&t).ok())
        .and_then(|v| v["dev"].as_array().cloned())
        .unwrap_or_default()
        .into_iter()
        .filter_map(|v| v.as_str().map(PathBuf::from))
        .collect()
}

fn save_dev_dirs(dirs: &[PathBuf]) -> Result<(), String> {
    let v = json!({ "dev": dirs.iter().map(|d| d.display().to_string()).collect::<Vec<_>>() });
    std::fs::write(dev_list_file(), v.to_string()).map_err(|e| e.to_string())
}

/// Reads and checks a connector folder.
pub fn load(dir: &Path) -> Result<Manifest, String> {
    let text = std::fs::read_to_string(dir.join("manifest.json"))
        .map_err(|e| format!("{}: no manifest.json ({e})", dir.display()))?;
    let m: Manifest = serde_json::from_str(&text).map_err(|e| format!("manifest.json: {e}"))?;
    validate_manifest(&m)?;
    if !dir.join(&m.entry).is_file() {
        return Err(format!("entry file {} is missing", m.entry));
    }
    Ok(m)
}

pub fn validate_manifest(m: &Manifest) -> Result<(), String> {
    if m.id.trim().is_empty()
        || !m
            .id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.')
    {
        return Err("id must be letters, digits, '-', '_' or '.'".into());
    }
    if m.domains.is_empty() {
        return Err("domains must list at least one host".into());
    }
    if m.matches.is_empty() {
        return Err("match must list at least one URL pattern".into());
    }
    if m.entry.contains("..") || m.entry.starts_with('/') {
        return Err("entry must be a file inside the connector folder".into());
    }
    for d in &m.domains {
        if crate::core::platform_optout::is_opted_out_host(d.trim_start_matches("*.")) {
            return Err(format!(
                "{d}: {}",
                crate::core::platform_optout::OPTED_OUT_ERROR
            ));
        }
    }
    for p in &m.matches {
        pattern_regex(p)?;
    }
    Ok(())
}

pub fn list() -> Vec<Connector> {
    let mut out = Vec::new();
    for d in dev_dirs() {
        if let Ok(m) = load(&d) {
            out.push(Connector {
                manifest: m,
                dir: d,
                dev: true,
            });
        }
    }
    if let Ok(rd) = std::fs::read_dir(installed_dir()) {
        for e in rd.flatten() {
            let d = e.path();
            if let Ok(m) = load(&d) {
                if !out.iter().any(|c| c.manifest.id == m.id) {
                    out.push(Connector {
                        manifest: m,
                        dir: d,
                        dev: false,
                    });
                }
            }
        }
    }
    out
}

/// Links a local folder in developer mode: edits take effect on the next run.
pub fn install_dev(dir: &Path) -> Result<Manifest, String> {
    let dir = dir.canonicalize().map_err(|e| e.to_string())?;
    let m = load(&dir)?;
    let mut dirs = dev_dirs();
    dirs.retain(|d| d != &dir);
    dirs.push(dir);
    save_dev_dirs(&dirs)?;
    Ok(m)
}

pub fn remove(id: &str) -> Result<(), String> {
    let mut dirs = dev_dirs();
    dirs.retain(|d| load(d).map(|m| m.id != id).unwrap_or(true));
    save_dev_dirs(&dirs)?;
    let p = installed_dir().join(id);
    if p.is_dir() {
        std::fs::remove_dir_all(p).map_err(|e| e.to_string())?;
    }
    Ok(())
}

// ── Options ───────────────────────────────────────────────────────────

fn options_file(id: &str) -> PathBuf {
    let d = data_dir().join("connector-options");
    let _ = std::fs::create_dir_all(&d);
    d.join(format!("{id}.json"))
}

/// Defaults from the manifest, then what the person saved, then `overrides`.
pub fn option_values(m: &Manifest, overrides: Option<&Value>) -> Value {
    let mut out = serde_json::Map::new();
    for o in &m.options {
        if let Some(id) = o["id"].as_str() {
            out.insert(id.into(), o.get("default").cloned().unwrap_or(Value::Null));
        }
    }
    let saved = std::fs::read_to_string(options_file(&m.id))
        .ok()
        .and_then(|t| serde_json::from_str::<Value>(&t).ok());
    for src in [saved.as_ref(), overrides].into_iter().flatten() {
        if let Some(o) = src.as_object() {
            for (k, v) in o {
                out.insert(k.clone(), v.clone());
            }
        }
    }
    Value::Object(out)
}

pub fn find_by_id(id: &str) -> Option<Connector> {
    list().into_iter().find(|c| c.manifest.id == id)
}

pub fn get_options(id: &str) -> Result<Value, String> {
    let c = find_by_id(id).ok_or_else(|| format!("no connector {id}"))?;
    Ok(json!({ "defs": c.manifest.options, "values": option_values(&c.manifest, None) }))
}

pub fn set_options(id: &str, values: &Value) -> Result<Value, String> {
    let c = find_by_id(id).ok_or_else(|| format!("no connector {id}"))?;
    let known: Vec<&str> = c
        .manifest
        .options
        .iter()
        .filter_map(|o| o["id"].as_str())
        .collect();
    let mut keep = serde_json::Map::new();
    for (k, v) in values.as_object().ok_or("options must be an object")? {
        if known.contains(&k.as_str()) {
            keep.insert(k.clone(), v.clone());
        }
    }
    std::fs::write(options_file(id), Value::Object(keep).to_string()).map_err(|e| e.to_string())?;
    get_options(id)
}

fn pattern_regex(p: &str) -> Result<regex::Regex, String> {
    let src = if p.starts_with('^') {
        p.to_string()
    } else {
        let mut s = String::from("^");
        for ch in p.chars() {
            if ch == '*' {
                s.push_str(".*");
            } else {
                s.push_str(&regex::escape(&ch.to_string()));
            }
        }
        s.push('$');
        s
    };
    regex::Regex::new(&src).map_err(|e| format!("match pattern {p}: {e}"))
}

pub fn find_for(url: &str) -> Option<Connector> {
    list().into_iter().find(|c| {
        c.manifest
            .matches
            .iter()
            .any(|p| pattern_regex(p).map(|r| r.is_match(url)).unwrap_or(false))
    })
}

pub fn scaffold(
    dir: &Path,
    id: &str,
    name: &str,
    domains: &[String],
    matches: &[String],
) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let m = Manifest {
        id: id.into(),
        name: name.into(),
        version: "0.1.0".into(),
        author: String::new(),
        license: "MIT".into(),
        homepage: String::new(),
        description: String::new(),
        domains: domains.to_vec(),
        matches: matches.to_vec(),
        login: None,
        entry: "connector.js".into(),
        min_omniget: env!("CARGO_PKG_VERSION").into(),
        options: Vec::new(),
    };
    validate_manifest(&m)?;
    let write = |f: &str, body: String| -> Result<(), String> {
        let p = dir.join(f);
        if p.exists() {
            return Ok(());
        }
        std::fs::write(p, body).map_err(|e| e.to_string())
    };
    write(
        "manifest.json",
        serde_json::to_string_pretty(&m).map_err(|e| e.to_string())?,
    )?;
    write("connector.js", TEMPLATE.to_string())?;
    write(
        "GUIDE.md",
        format!("# {name}\n\nWhat it downloads, what needs a login, known limits.\n"),
    )?;
    Ok(())
}

const TEMPLATE: &str = r#"// resolve(url, ctx) returns the tree OmniGet downloads.
// ctx.fetch(url, {method, headers, body}) -> {status, ok, headers, text(), json()}
// ctx.cookies(domain) -> {name: value}   ctx.page.eval(js) -> value (the open tab)
// ctx.sniffed() -> media the tab already loaded   ctx.log(...)
function resolve(url, ctx) {
  const page = ctx.fetch(url).text();
  const title = (page.match(/<title>([^<]*)<\/title>/i) || [, "Untitled"])[1].trim();
  return {
    title,
    children: ctx.sniffed().map((m) => ({
      title: m.page_title || title,
      kind: m.kind === "hls" || m.kind === "dash" ? "video" : m.kind,
      source: { format: m.kind === "hls" ? "hls" : m.kind === "dash" ? "dash" : "direct", url: m.url, headers: m.headers },
    })),
  };
}
"#;

/// The contract, served to agents by `gd_connector_spec`.
pub const SPEC: &str = include_str!("SPEC.md");

// ── Running a connector ───────────────────────────────────────────────

const PRELUDE: &str = r#"
var ctx = (function () {
  function wrap(r) {
    if (r.error) throw new Error(r.error);
    return {
      status: r.status, ok: r.status >= 200 && r.status < 300, url: r.url, headers: r.headers,
      text: function () { return r.body; },
      json: function () { return JSON.parse(r.body); },
    };
  }
  return {
    url: __target_url,
    tabId: __tab_id,
    options: JSON.parse(__options),
    fetch: function (url, init) {
      init = init || {};
      return wrap(JSON.parse(__host_fetch(JSON.stringify({
        url: String(url), method: init.method || "GET", headers: init.headers || {},
        body: init.body === undefined ? null : (typeof init.body === "string" ? init.body : JSON.stringify(init.body)),
      }))));
    },
    fetchJson: function (url, init) {
      var r = this.fetch(url, init);
      if (!r.ok) throw new Error("HTTP " + r.status + " for " + url);
      return r.json();
    },
    cookies: function (domain) { return JSON.parse(__host_cookies(String(domain || ""))); },
    sniffed: function () { return JSON.parse(__host_sniffed()); },
    page: { eval: function (js) { var r = JSON.parse(__host_eval(String(js))); if (r.error) throw new Error(r.error); return r.value; } },
    log: function () { __host_log(Array.prototype.map.call(arguments, function (a) { return typeof a === "string" ? a : JSON.stringify(a); }).join(" ")); },
    query: function (o) { return Object.keys(o).map(function (k) { return encodeURIComponent(k) + "=" + encodeURIComponent(o[k]); }).join("&"); },
  };
})();
"#;

#[derive(Debug, Clone, Serialize)]
pub struct ResolveResult {
    pub connector: Option<String>,
    pub tree: Node,
    pub logs: Vec<String>,
    pub items: usize,
    pub drm: usize,
    pub ms: u128,
}

/// Resolves `url` with the matching connector (or `dir`, a connector folder
/// under development), or with the built-in sniffer when none matches.
pub async fn resolve(
    url: &str,
    dir: Option<PathBuf>,
    tab_id: Option<String>,
    options: Option<Value>,
) -> Result<ResolveResult, String> {
    crate::core::platform_optout::ensure_allowed(url)?;
    let started = Instant::now();
    let conn = match dir {
        Some(d) => Some(Connector {
            manifest: load(&d)?,
            dir: d,
            dev: true,
        }),
        None => find_for(url),
    };
    let tab_id = tab_id.or_else(|| {
        browser::current().and_then(|b| {
            b.tabs()
                .into_iter()
                .rev()
                .find(|t| t.url == url)
                .or_else(|| b.tab_for_origin(url))
                .map(|t| t.id)
        })
    });
    let (mut tree, logs, id) = match conn {
        Some(c) => {
            let opts = option_values(&c.manifest, options.as_ref());
            let (out, logs) =
                run_script(c.clone(), url.to_string(), tab_id, Call::Resolve, opts).await?;
            let mut tree: Node = serde_json::from_str(&out).map_err(|e| format!("tree: {e}"))?;
            if tree.kind.is_empty() {
                tree.kind = "folder".into();
            }
            tree.connector = Some(c.manifest.id.clone());
            tree.connector_dir = Some(c.dir.display().to_string());
            tree.origin = Some(url.to_string());
            (tree, logs, Some(c.manifest.id))
        }
        None => (generic_tree(url, tab_id.as_deref()), Vec::new(), None),
    };
    tree.assign_ids("0");
    let (items, drm, _) = tree.counts();
    Ok(ResolveResult {
        connector: id,
        tree,
        logs,
        items,
        drm,
        ms: started.elapsed().as_millis(),
    })
}

/// No connector: what the tab played, plus the page itself for yt-dlp.
fn generic_tree(url: &str, tab_id: Option<&str>) -> Node {
    let page_title = tab_id
        .and_then(|t| browser::current().and_then(|b| b.tab(t)))
        .map(|t| t.title)
        .filter(|t| !t.is_empty())
        .unwrap_or_else(|| url.to_string());
    let mut children = vec![Node {
        title: format!("{page_title} (yt-dlp)"),
        kind: "video".into(),
        source: Some(Source {
            format: "ytdlp".into(),
            url: Some(url.to_string()),
            ..Default::default()
        }),
        note: Some("sent to the Downloads queue".into()),
        ..Default::default()
    }];
    if let Some(t) = tab_id {
        for m in sniffer::sniffed(Some(t)) {
            let format = match m.kind.as_str() {
                "hls" => "hls",
                "dash" => "dash",
                _ => "direct",
            };
            let name = m
                .url
                .split(['?', '#'])
                .next()
                .and_then(|p| p.rsplit('/').next())
                .unwrap_or("media")
                .to_string();
            children.push(Node {
                title: name,
                kind: match m.kind.as_str() {
                    "hls" | "dash" => "video".into(),
                    "document" => "file".into(),
                    k => k.into(),
                },
                source: Some(Source {
                    format: format.into(),
                    url: Some(m.url),
                    headers: m.headers,
                    ..Default::default()
                }),
                ..Default::default()
            });
        }
    }
    Node {
        title: page_title,
        kind: "folder".into(),
        children,
        ..Default::default()
    }
}

fn domain_allowed(domains: &[String], url: &str) -> Result<(), String> {
    let host = host_of(url).ok_or_else(|| format!("bad url: {url}"))?;
    if crate::core::platform_optout::is_opted_out_host(&host) {
        return Err(crate::core::platform_optout::OPTED_OUT_ERROR.into());
    }
    if domains.iter().any(|d| host_matches(&host, d)) {
        Ok(())
    } else {
        Err(format!("{host} is not in this connector's domains"))
    }
}

/// Which function of the script to call.
#[derive(Clone)]
enum Call {
    Resolve,
    Item(String),
}

/// Resolves one lazy item of a tree the connector made: calls
/// `resolveItem(key, ctx)`, which returns a source (`{ format, url, ... }`) or
/// a node (`{ kind, drm, source }`).
pub async fn resolve_item(
    connector_id: &str,
    connector_dir: Option<&str>,
    origin: &str,
    key: &str,
) -> Result<Node, String> {
    let c = match connector_dir.map(PathBuf::from).filter(|d| d.is_dir()) {
        Some(d) => Connector {
            manifest: load(&d)?,
            dir: d,
            dev: true,
        },
        None => find_by_id(connector_id)
            .ok_or_else(|| format!("connector {connector_id} is not installed"))?,
    };
    let tab_id = browser::current()
        .and_then(|b| b.tab_for_origin(origin))
        .map(|t| t.id);
    let opts = option_values(&c.manifest, None);
    let (out, _) = run_script(
        c,
        origin.to_string(),
        tab_id,
        Call::Item(key.to_string()),
        opts,
    )
    .await?;
    let v: Value = serde_json::from_str(&out).map_err(|e| format!("resolveItem: {e}"))?;
    let node = if v.get("source").is_some() || v.get("drm").is_some() {
        serde_json::from_value::<Node>(v).map_err(|e| format!("resolveItem: {e}"))?
    } else {
        Node {
            source: Some(
                serde_json::from_value::<Source>(v).map_err(|e| format!("resolveItem: {e}"))?,
            ),
            ..Default::default()
        }
    };
    Ok(node)
}

async fn run_script(
    c: Connector,
    url: String,
    tab_id: Option<String>,
    call: Call,
    options: Value,
) -> Result<(String, Vec<String>), String> {
    let source =
        std::fs::read_to_string(c.dir.join(&c.manifest.entry)).map_err(|e| e.to_string())?;
    let source = source
        .replace("export async function", "async function")
        .replace("export function", "function")
        .replace("export default ", "");
    let handle = tokio::runtime::Handle::current();
    tokio::task::spawn_blocking(move || {
        run_blocking(handle, c.manifest, source, url, tab_id, call, options)
    })
    .await
    .map_err(|e| e.to_string())?
}

fn run_blocking(
    handle: tokio::runtime::Handle,
    manifest: Manifest,
    source: String,
    url: String,
    tab_id: Option<String>,
    call: Call,
    options: Value,
) -> Result<(String, Vec<String>), String> {
    use rquickjs::{CatchResultExt, Context, Function, Runtime};
    use std::sync::{Arc, Mutex};

    let rt = Runtime::new().map_err(|e| e.to_string())?;
    rt.set_memory_limit(512 << 20);
    rt.set_max_stack_size(4 << 20);
    let deadline = Instant::now() + Duration::from_secs(15 * 60);
    rt.set_interrupt_handler(Some(Box::new(move || Instant::now() > deadline)));
    let ctx = Context::full(&rt).map_err(|e| e.to_string())?;
    let logs: Arc<Mutex<Vec<String>>> = Arc::default();
    let domains = Arc::new(manifest.domains.clone());

    let result: Result<String, String> = ctx.with(|ctx| {
        let g = ctx.globals();
        let js_err = |e: rquickjs::Error| e.to_string();
        g.set("__target_url", url.clone()).map_err(js_err)?;
        g.set("__tab_id", tab_id.clone().unwrap_or_default())
            .map_err(js_err)?;
        g.set("__options", options.to_string()).map_err(js_err)?;

        let (h, d) = (handle.clone(), domains.clone());
        g.set(
            "__host_fetch",
            Function::new(ctx.clone(), move |req: String| -> String {
                let out = (|| -> Result<Value, String> {
                    let r: Value = serde_json::from_str(&req).map_err(|e| e.to_string())?;
                    let u = r["url"].as_str().unwrap_or_default().to_string();
                    domain_allowed(&d, &u)?;
                    let headers = r["headers"].as_object().cloned().unwrap_or_default();
                    let method = r["method"].as_str().unwrap_or("GET").to_string();
                    let body = r["body"].as_str().map(String::from);
                    h.block_on(browser::fetch(&u, &method, &headers, body.as_deref()))
                })();
                out.unwrap_or_else(|e| json!({ "error": e })).to_string()
            })
            .map_err(js_err)?,
        )
        .map_err(js_err)?;

        let (h, d) = (handle.clone(), domains.clone());
        g.set(
            "__host_cookies",
            Function::new(ctx.clone(), move |domain: String| -> String {
                let host = domain.trim_start_matches('.').to_ascii_lowercase();
                if !d.iter().any(|x| host_matches(&host, x)) {
                    return "{}".into();
                }
                let Some(b) = browser::current() else {
                    return "{}".into();
                };
                let cookies = h.block_on(b.cookies_for(&host)).unwrap_or_default();
                let mut o = serde_json::Map::new();
                for c in cookies {
                    if let (Some(n), Some(v)) = (c["name"].as_str(), c["value"].as_str()) {
                        o.insert(n.into(), Value::String(v.into()));
                    }
                }
                Value::Object(o).to_string()
            })
            .map_err(js_err)?,
        )
        .map_err(js_err)?;

        let tab = tab_id.clone();
        g.set(
            "__host_sniffed",
            Function::new(ctx.clone(), move || -> String {
                serde_json::to_string(&sniffer::sniffed(tab.as_deref()))
                    .unwrap_or_else(|_| "[]".into())
            })
            .map_err(js_err)?,
        )
        .map_err(js_err)?;

        let (h, d, tab) = (handle.clone(), domains.clone(), tab_id.clone());
        g.set(
            "__host_eval",
            Function::new(ctx.clone(), move |js: String| -> String {
                let out = (|| -> Result<Value, String> {
                    let b = browser::require()?;
                    let t = tab
                        .as_deref()
                        .and_then(|t| b.tab(t))
                        .ok_or("no tab is open for this page")?;
                    domain_allowed(&d, &t.url)?;
                    h.block_on(b.eval(&t.id, &js, Duration::from_secs(60)))
                })();
                match out {
                    Ok(v) => json!({ "value": v }).to_string(),
                    Err(e) => json!({ "error": e }).to_string(),
                }
            })
            .map_err(js_err)?,
        )
        .map_err(js_err)?;

        let l = logs.clone();
        g.set(
            "__host_log",
            Function::new(ctx.clone(), move |msg: String| {
                tracing::info!("[global-dl] connector: {msg}");
                if let Ok(mut l) = l.lock() {
                    if l.len() < 500 {
                        l.push(msg);
                    }
                }
            })
            .map_err(js_err)?,
        )
        .map_err(js_err)?;

        ctx.eval::<(), _>(PRELUDE)
            .catch(&ctx)
            .map_err(|e| format!("prelude: {e}"))?;
        ctx.eval::<(), _>(source)
            .catch(&ctx)
            .map_err(|e| format!("connector: {e}"))?;
        let (fname, arg) = match &call {
            Call::Resolve => ("resolve", url.clone()),
            Call::Item(k) => ("resolveItem", k.clone()),
        };
        let f: Function = g
            .get(fname)
            .map_err(|_| format!("the connector defines no {fname}(…, ctx)"))?;
        let c: rquickjs::Value = g.get("ctx").map_err(js_err)?;
        let mut out: rquickjs::Value = f
            .call((arg, c))
            .catch(&ctx)
            .map_err(|e| format!("{fname}: {e}"))?;
        if let Some(p) = out.as_promise().cloned() {
            out = p
                .finish::<rquickjs::Value>()
                .catch(&ctx)
                .map_err(|e| format!("{fname}: {e}"))?;
        }
        ctx.json_stringify(out)
            .catch(&ctx)
            .map_err(|e| format!("{fname} returned something that is not JSON: {e}"))?
            .map(|s| s.to_string().unwrap_or_default())
            .ok_or_else(|| format!("{fname} returned nothing"))
    });
    let text = result?;
    let logs = logs.lock().map(|l| l.clone()).unwrap_or_default();
    Ok((text, logs))
}

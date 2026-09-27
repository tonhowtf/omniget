//! Official MCP Registry (`registry.modelcontextprotocol.io/v0.1`) → itens `mcp`.
//!
//! Busca paginada por cursor (`metadata.nextCursor`), cache em disco de 1 h por
//! consulta (servido velho se a rede cair) e mapa em memória dos últimos itens
//! vistos, para `catalog_item(id)` achar um servidor sem nova busca.
//!
//! Conversão: `packages[]` npm/pypi/oci/nuget/cargo → servidor stdio
//! (`npx`/`uvx`/`docker`/`dnx`/`cargo`), `remotes[]` → `{type: http|sse, url, headers}`.
//! O formato é o `.mcp.json` (`{"mcpServers": {...}}`), igual aos MCPs do
//! claude-code-templates, então o `agentkit` usa o mesmo caminho de conversão.

use std::collections::{BTreeMap, HashMap};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, SystemTime};

use serde::Serialize;
use serde_json::{json, Map, Value};

use super::model::{CatalogItem, ItemSource};
use super::normalize::short_description;
use super::{catalog_dir, get_bytes, sha256_hex, write_atomic, CatalogError, Result, ERR_PARSE};

pub const SOURCE_ID: &str = "mcp-registry";
pub const BASE_URL: &str = "https://registry.modelcontextprotocol.io/v0.1";
const CACHE_TTL: Duration = Duration::from_secs(3600);
const MEMORY_MAX: usize = 5000;

#[derive(Debug, Clone, Serialize)]
pub struct RegistryPage {
    pub items: Vec<CatalogItem>,
    pub next_cursor: Option<String>,
    pub count: usize,
    /// `network`, `cache` ou `stale-cache`.
    pub from: &'static str,
}

fn memory() -> &'static Mutex<HashMap<String, (CatalogItem, Value)>> {
    static M: OnceLock<Mutex<HashMap<String, (CatalogItem, Value)>>> = OnceLock::new();
    M.get_or_init(|| Mutex::new(HashMap::new()))
}

fn remember(item: &CatalogItem, raw: &Value) {
    if let Ok(mut m) = memory().lock() {
        if m.len() >= MEMORY_MAX {
            m.clear();
        }
        m.insert(item.id.clone(), (item.clone(), raw.clone()));
    }
}

/// Item visto numa busca recente.
pub fn cached_item(id: &str) -> Option<CatalogItem> {
    memory().lock().ok()?.get(id).map(|(i, _)| i.clone())
}

fn cached_raw(id: &str) -> Option<Value> {
    memory().lock().ok()?.get(id).map(|(_, r)| r.clone())
}

/// Id do item para um nome do registry (`io.github.user/server`).
pub fn item_id(name: &str) -> String {
    let (ns, rest) = name.split_once('/').unwrap_or(("registry", name));
    format!("{SOURCE_ID}:mcps/{ns}/{rest}")
}

/// Nome curto de servidor para a chave do `mcpServers`.
pub fn short_name(name: &str) -> String {
    let base = name.rsplit('/').next().unwrap_or(name);
    let s: String = base
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    let s = s.trim_matches('-').to_string();
    if s.is_empty() {
        "server".into()
    } else {
        s
    }
}

fn str_of<'a>(v: &'a Value, k: &str) -> Option<&'a str> {
    v.get(k).and_then(Value::as_str).filter(|s| !s.is_empty())
}

/// Valor de um input: `value` > `default` > placeholder `${NAME}`.
fn input_value(inp: &Value, name: &str) -> String {
    if let Some(v) = str_of(inp, "value") {
        return v.to_string();
    }
    if let Some(d) = inp.get("default").filter(|d| !d.is_null()) {
        return d
            .as_str()
            .map(str::to_string)
            .unwrap_or_else(|| d.to_string());
    }
    format!("${{{name}}}")
}

fn input_entry(inp: &Value, name: &str, kind: &str) -> Value {
    json!({
        "name": name,
        "kind": kind,
        "description": inp.get("description").cloned().unwrap_or(Value::Null),
        "required": inp.get("isRequired").and_then(Value::as_bool).unwrap_or(false),
        "secret": inp.get("isSecret").and_then(Value::as_bool).unwrap_or(false),
        "default": inp.get("default").cloned().unwrap_or(Value::Null),
        "choices": inp.get("choices").cloned().unwrap_or(Value::Null),
    })
}

fn args_from(list: Option<&Value>, inputs: &mut Vec<Value>) -> Vec<String> {
    let mut out = Vec::new();
    for a in list.and_then(Value::as_array).into_iter().flatten() {
        let named = str_of(a, "type") == Some("named");
        let has_value = a.get("value").is_some() || a.get("default").is_some();
        let hint = str_of(a, "valueHint")
            .or_else(|| str_of(a, "name"))
            .unwrap_or("value")
            .trim_start_matches('-')
            .to_string();
        let placeholder = || format!("${{{}}}", hint.to_ascii_uppercase().replace('-', "_"));
        if named {
            if let Some(n) = str_of(a, "name") {
                out.push(n.to_string());
            }
            if has_value {
                out.push(input_value(a, &hint));
            } else if str_of(a, "valueHint").is_some() {
                inputs.push(input_entry(a, &hint, "argument"));
                out.push(placeholder());
            }
            // sem valor nem dica: é uma flag booleana
        } else if has_value {
            out.push(input_value(a, &hint));
        } else {
            inputs.push(input_entry(a, &hint, "argument"));
            out.push(placeholder());
        }
    }
    out
}

fn headers_from(list: Option<&Value>, inputs: &mut Vec<Value>) -> Map<String, Value> {
    let mut m = Map::new();
    for h in list.and_then(Value::as_array).into_iter().flatten() {
        let Some(name) = str_of(h, "name") else {
            continue;
        };
        let v = input_value(h, &name.to_ascii_uppercase().replace('-', "_"));
        if h.get("isSecret").and_then(Value::as_bool).unwrap_or(false) || v.contains('{') {
            inputs.push(input_entry(h, name, "header"));
        }
        m.insert(name.to_string(), Value::String(v));
    }
    m
}

/// Um `packages[]` → servidor stdio (ou http local), com os inputs necessários.
fn package_config(pkg: &Value) -> Option<(String, Value, Vec<Value>)> {
    let reg = str_of(pkg, "registryType").unwrap_or("");
    let ident = str_of(pkg, "identifier")?;
    let version = str_of(pkg, "version");
    let hint = str_of(pkg, "runtimeHint");
    let mut inputs = Vec::new();
    let mut env = Map::new();
    for e in pkg
        .get("environmentVariables")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let Some(name) = str_of(e, "name") else {
            continue;
        };
        if e.get("value").is_none() && e.get("default").is_none() {
            inputs.push(input_entry(e, name, "env"));
        }
        env.insert(name.to_string(), Value::String(input_value(e, name)));
    }
    let runtime_args = args_from(pkg.get("runtimeArguments"), &mut inputs);
    let package_args = args_from(pkg.get("packageArguments"), &mut inputs);
    let transport = pkg
        .get("transport")
        .cloned()
        .unwrap_or(json!({"type":"stdio"}));
    let ttype = str_of(&transport, "type").unwrap_or("stdio");
    let (command, mut args): (String, Vec<String>) = match reg {
        "npm" => {
            let spec = match version {
                Some(v) => format!("{ident}@{v}"),
                None => ident.to_string(),
            };
            (hint.unwrap_or("npx").to_string(), {
                let mut a = runtime_args.clone();
                if !a.iter().any(|x| x == "-y" || x == "--yes") {
                    a.insert(0, "-y".into());
                }
                a.push(spec);
                a
            })
        }
        "pypi" => {
            let spec = match version {
                Some(v) => format!("{ident}=={v}"),
                None => ident.to_string(),
            };
            let mut a = runtime_args.clone();
            a.push(spec);
            (hint.unwrap_or("uvx").to_string(), a)
        }
        "oci" => {
            let image = match version {
                Some(v) if !ident.rsplit('/').next().unwrap_or("").contains(':') => {
                    format!("{ident}:{v}")
                }
                _ => ident.to_string(),
            };
            let mut a = vec!["run".to_string(), "-i".into(), "--rm".into()];
            for k in env.keys() {
                a.push("-e".into());
                a.push(k.clone());
            }
            a.extend(runtime_args.clone());
            a.push(image);
            (hint.unwrap_or("docker").to_string(), a)
        }
        "nuget" => {
            let spec = match version {
                Some(v) => format!("{ident}@{v}"),
                None => ident.to_string(),
            };
            let mut a = runtime_args.clone();
            a.push(spec);
            a.push("--yes".into());
            (hint.unwrap_or("dnx").to_string(), a)
        }
        "cargo" => {
            // não há runner efêmero oficial: instala e roda o binário
            (hint.unwrap_or(ident).to_string(), runtime_args.clone())
        }
        _ => return None,
    };
    args.extend(package_args);
    let label = format!("{reg}:{ident}");
    if ttype == "streamable-http" || ttype == "sse" {
        let url = str_of(&transport, "url")?.to_string();
        let headers = headers_from(transport.get("headers"), &mut inputs);
        let mut cfg = json!({
            "type": if ttype == "sse" { "sse" } else { "http" },
            "url": url,
            "launch": { "command": command, "args": args, "env": env },
        });
        if !headers.is_empty() {
            cfg["headers"] = Value::Object(headers);
        }
        return Some((label, cfg, inputs));
    }
    let mut cfg = json!({ "command": command, "args": args });
    if !env.is_empty() {
        cfg["env"] = Value::Object(env);
    }
    Some((label, cfg, inputs))
}

fn remote_config(r: &Value) -> Option<(String, Value, Vec<Value>)> {
    let ty = str_of(r, "type").unwrap_or("streamable-http");
    let url = str_of(r, "url")?;
    let mut inputs = Vec::new();
    let headers = headers_from(r.get("headers"), &mut inputs);
    for (k, v) in r
        .get("variables")
        .and_then(Value::as_object)
        .into_iter()
        .flatten()
    {
        inputs.push(input_entry(v, k, "url"));
    }
    let mut cfg = json!({ "type": if ty == "sse" { "sse" } else { "http" }, "url": url });
    if !headers.is_empty() {
        cfg["headers"] = Value::Object(headers);
    }
    Some((format!("remote:{ty}"), cfg, inputs))
}

fn github_repo_of(url: &str) -> Option<String> {
    let rest = url
        .trim_end_matches(".git")
        .split("github.com/")
        .nth(1)?
        .trim_matches('/');
    let mut p = rest.split('/');
    let (o, r) = (p.next()?, p.next()?);
    let repo = format!("{o}/{r}");
    super::valid_repo(&repo).then_some(repo)
}

/// Converte uma entrada `{server, _meta}` do registry num item `mcp`.
pub fn convert(entry: &Value) -> Option<CatalogItem> {
    let server = entry.get("server").unwrap_or(entry);
    let name = str_of(server, "name")?;
    let official = entry
        .get("_meta")
        .and_then(|m| m.get("io.modelcontextprotocol.registry/official"))
        .cloned()
        .unwrap_or(Value::Null);
    let short = short_name(name);
    let mut variants: Vec<Value> = Vec::new();
    let mut tags: Vec<String> = Vec::new();
    for r in server
        .get("remotes")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        if let Some((label, cfg, inputs)) = remote_config(r) {
            variants.push(json!({"label": label, "config": cfg, "inputs": inputs}));
            tags.push("remote".into());
        }
    }
    for p in server
        .get("packages")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        if let Some(t) = str_of(p, "registryType") {
            tags.push(t.to_string());
        }
        if let Some((label, cfg, inputs)) = package_config(p) {
            variants.push(json!({"label": label, "config": cfg, "inputs": inputs}));
        }
    }
    tags.sort();
    tags.dedup();
    let primary = variants.first().cloned();
    let mut servers = Map::new();
    if let Some(p) = &primary {
        let mut cfg = p["config"].clone();
        if let Some(d) = str_of(server, "description") {
            cfg["description"] = Value::String(d.to_string());
        }
        servers.insert(short.clone(), cfg);
    }
    let mut norm = Vec::new();
    if primary.is_none() {
        norm.push("no_installable_transport");
    }
    let (desc, marks) = short_description(str_of(server, "description").unwrap_or(name));
    norm.extend(marks);
    let repo_url = server
        .get("repository")
        .and_then(|r| str_of(r, "url"))
        .map(str::to_string);
    let repo = repo_url.as_deref().and_then(github_repo_of);
    let (ns, _) = name.split_once('/').unwrap_or(("registry", name));
    let updated = str_of(&official, "updatedAt")
        .or_else(|| str_of(&official, "publishedAt"))
        .map(|s| s.chars().take(10).collect());
    let frontmatter = json!({
        "mcpServers": servers,
        "registry": {
            "name": name,
            "title": server.get("title").cloned().unwrap_or(Value::Null),
            "version": server.get("version").cloned().unwrap_or(Value::Null),
            "websiteUrl": server.get("websiteUrl").cloned().unwrap_or(Value::Null),
            "repository": server.get("repository").cloned().unwrap_or(Value::Null),
            "status": official.get("status").cloned().unwrap_or(Value::Null),
            "publishedAt": official.get("publishedAt").cloned().unwrap_or(Value::Null),
            "updatedAt": official.get("updatedAt").cloned().unwrap_or(Value::Null),
            "isLatest": official.get("isLatest").cloned().unwrap_or(Value::Null),
        },
        "inputs": primary.as_ref().map(|p| p["inputs"].clone()).unwrap_or(json!([])),
        "variants": variants,
    });
    Some(CatalogItem {
        id: item_id(name),
        kind: "mcp".into(),
        name: short.clone(),
        category: ns.to_string(),
        description: desc,
        source: ItemSource {
            id: SOURCE_ID.into(),
            repo,
            commit: None,
            git_ref: None,
            path: String::new(),
            dir: String::new(),
            url: str_of(server, "websiteUrl")
                .map(str::to_string)
                .or(repo_url)
                .or_else(|| {
                    Some(format!(
                        "{BASE_URL}/servers/{}/versions/latest",
                        urlencoding::encode(name)
                    ))
                }),
            upstream: Some(name.to_string()),
            ..Default::default()
        },
        license: None,
        author: Some(ns.to_string()),
        tags,
        origin_tool: "claude".into(),
        files: Vec::new(),
        entry: format!("{short}.json"),
        frontmatter,
        references: Vec::new(),
        stars: None,
        updated,
        normalization: norm.into_iter().map(str::to_string).collect(),
        security: None,
        collides_with: Vec::new(),
        install_name: None,
    })
}

/// Converte uma resposta `/v0.1/servers` inteira.
pub fn convert_page(body: &Value) -> (Vec<CatalogItem>, Option<String>) {
    let mut items = Vec::new();
    for e in body["servers"].as_array().into_iter().flatten() {
        if let Some(it) = convert(e) {
            remember(&it, e);
            items.push(it);
        }
    }
    let next = body["metadata"]["nextCursor"]
        .as_str()
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    (items, next)
}

fn cache_path(key: &str) -> Result<std::path::PathBuf> {
    Ok(catalog_dir()?
        .join("mcp-registry")
        .join(format!("{}.json", &sha256_hex(key.as_bytes())[..32])))
}

fn fresh(p: &std::path::Path) -> bool {
    std::fs::metadata(p)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| SystemTime::now().duration_since(t).ok())
        .is_some_and(|age| age < CACHE_TTL)
}

/// Busca no registry (substring no nome; vazio lista tudo), paginada por cursor.
pub async fn search(
    query: Option<&str>,
    cursor: Option<&str>,
    limit: usize,
) -> Result<RegistryPage> {
    let limit = limit.clamp(1, 100);
    let mut url = format!("{BASE_URL}/servers?version=latest&limit={limit}");
    if let Some(q) = query.map(str::trim).filter(|q| !q.is_empty()) {
        url.push_str(&format!("&search={}", urlencoding::encode(q)));
    }
    if let Some(c) = cursor.filter(|c| !c.is_empty()) {
        url.push_str(&format!("&cursor={}", urlencoding::encode(c)));
    }
    let cache = cache_path(&url)?;
    if fresh(&cache) {
        if let Some(body) = std::fs::read(&cache)
            .ok()
            .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
        {
            let (items, next_cursor) = convert_page(&body);
            return Ok(RegistryPage {
                count: items.len(),
                items,
                next_cursor,
                from: "cache",
            });
        }
    }
    match get_bytes(&url, Some("application/json")).await {
        Ok(bytes) => {
            let body: Value = serde_json::from_slice(&bytes)
                .map_err(|e| CatalogError::new(ERR_PARSE, format!("registry: {e}")))?;
            let _ = write_atomic(&cache, &bytes);
            let (items, next_cursor) = convert_page(&body);
            Ok(RegistryPage {
                count: items.len(),
                items,
                next_cursor,
                from: "network",
            })
        }
        Err(e) => {
            if let Some(body) = std::fs::read(&cache)
                .ok()
                .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
            {
                let (items, next_cursor) = convert_page(&body);
                return Ok(RegistryPage {
                    count: items.len(),
                    items,
                    next_cursor,
                    from: "stale-cache",
                });
            }
            Err(e)
        }
    }
}

/// Um servidor pelo nome (`io.github.user/server`), versão mais recente.
pub async fn fetch_server(name: &str) -> Result<CatalogItem> {
    let url = format!(
        "{BASE_URL}/servers/{}/versions/latest",
        urlencoding::encode(name)
    );
    let bytes = get_bytes(&url, Some("application/json")).await?;
    let body: Value = serde_json::from_slice(&bytes)
        .map_err(|e| CatalogError::new(ERR_PARSE, format!("registry: {e}")))?;
    let item = convert(&body)
        .ok_or_else(|| CatalogError::new(ERR_PARSE, format!("registry: `{name}` unreadable")))?;
    remember(&item, &body);
    Ok(item)
}

/// Item do registry por id (memória ou rede).
pub async fn item(id: &str) -> Result<CatalogItem> {
    if let Some(i) = cached_item(id) {
        return Ok(i);
    }
    let name = id
        .strip_prefix(&format!("{SOURCE_ID}:mcps/"))
        .ok_or_else(|| CatalogError::new(super::ERR_NOT_FOUND, format!("item `{id}`")))?;
    fetch_server(name).await
}

/// Arquivos do item: `<nome>.json` (`.mcp.json` pronto) + `server.json` cru.
pub fn generated_files(item: &CatalogItem) -> Result<BTreeMap<String, Vec<u8>>> {
    let mut out = BTreeMap::new();
    let mcp =
        json!({ "mcpServers": item.frontmatter.get("mcpServers").cloned().unwrap_or(json!({})) });
    out.insert(
        item.entry.clone(),
        serde_json::to_vec_pretty(&mcp).unwrap_or_default(),
    );
    if let Some(raw) = cached_raw(&item.id) {
        out.insert(
            "server.json".into(),
            serde_json::to_vec_pretty(raw.get("server").unwrap_or(&raw)).unwrap_or_default(),
        );
    }
    Ok(out)
}

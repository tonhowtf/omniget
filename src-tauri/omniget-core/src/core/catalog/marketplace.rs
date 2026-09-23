//! Marketplaces de plugin do Claude: `.claude-plugin/marketplace.json` de um
//! repo `owner/repo` (curado ou adicionado pelo usuário) → itens `plugin`.
//!
//! O commit é resolvido na hora (API do GitHub, `Accept: application/vnd.github.sha`)
//! e a listagem fica em `<app_data>/agentkit/catalog/marketplaces/<owner>__<repo>.json`;
//! o índice em memória troca os plugins daquele marketplace pelos buscados.

use serde_json::{json, Value};

use super::model::{CatalogItem, ItemSource, MarketplaceInfo};
use super::normalize::{humanize, normalize_license, short_description, slim_json, to_list};
use super::{
    catalog_dir, get_bytes, github_api, github_commit, raw_url, valid_repo, write_atomic,
    CatalogError, Result, ERR_INVALID, ERR_NOT_FOUND,
};

/// Origem resolvida de um plugin.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PluginSource {
    pub repo: Option<String>,
    pub commit: Option<String>,
    pub git_ref: Option<String>,
    pub path: String,
    pub url: Option<String>,
}

fn clean(p: &str) -> String {
    p.trim_start_matches("./")
        .trim_start_matches('/')
        .trim_end_matches('/')
        .to_string()
}

fn github_url_source(url: &str) -> PluginSource {
    let u = url.trim_end_matches('/');
    if let Some(rest) = u.split("github.com").nth(1) {
        let rest = rest.trim_start_matches([':', '/']);
        let mut parts = rest.splitn(3, '/');
        let owner = parts.next().unwrap_or_default();
        let repo = parts.next().unwrap_or_default().trim_end_matches(".git");
        let tail = parts.next().unwrap_or_default();
        let full = format!("{owner}/{repo}");
        if valid_repo(&full) {
            let (git_ref, path) = match tail.strip_prefix("tree/") {
                Some(t) => {
                    let mut tp = t.splitn(2, '/');
                    (
                        tp.next().map(str::to_string),
                        tp.next().unwrap_or_default().to_string(),
                    )
                }
                None => (None, String::new()),
            };
            return PluginSource {
                repo: Some(full),
                commit: None,
                git_ref,
                path,
                url: None,
            };
        }
    }
    PluginSource {
        url: Some(url.to_string()),
        ..Default::default()
    }
}

/// `source` de uma entrada de marketplace → repo/commit/pasta.
pub fn plugin_source(
    src: &Value,
    market_repo: &str,
    market_commit: Option<&str>,
    plugin_root: Option<&str>,
) -> PluginSource {
    match src {
        Value::String(s) => {
            if s.starts_with("http://") || s.starts_with("https://") {
                return github_url_source(s);
            }
            let mut p = clean(s);
            if let Some(root) = plugin_root {
                if !s.starts_with("./") && !p.contains('/') {
                    let r = clean(root);
                    if !r.is_empty() {
                        p = format!("{r}/{p}");
                    }
                }
            }
            PluginSource {
                repo: Some(market_repo.to_string()),
                commit: market_commit.map(str::to_string),
                git_ref: None,
                path: p,
                url: None,
            }
        }
        Value::Object(o) => {
            let get = |k: &str| o.get(k).and_then(Value::as_str);
            let kind = get("source").unwrap_or("");
            let mut out = match kind {
                "github" if get("repo").is_some() => PluginSource {
                    repo: get("repo").map(str::to_string),
                    ..Default::default()
                },
                "url" | "git" | "git-subdir" if get("url").is_some() => {
                    github_url_source(get("url").unwrap_or_default())
                }
                "npm" | "pip" => PluginSource {
                    url: Some(format!(
                        "{kind}:{}",
                        get("package").or(get("name")).unwrap_or_default()
                    )),
                    git_ref: get("version").map(str::to_string),
                    ..Default::default()
                },
                _ => PluginSource {
                    repo: Some(market_repo.to_string()),
                    commit: market_commit.map(str::to_string),
                    ..Default::default()
                },
            };
            if let Some(sha) = get("sha") {
                out.commit = Some(sha.to_string());
            }
            if let Some(r) = get("ref") {
                out.git_ref = Some(r.to_string());
            }
            if let Some(p) = get("path").or(get("subdir")) {
                out.path = clean(p);
            }
            out
        }
        _ => PluginSource {
            repo: Some(market_repo.to_string()),
            commit: market_commit.map(str::to_string),
            ..Default::default()
        },
    }
}

fn parse_json_loose(bytes: &[u8]) -> Option<Value> {
    if let Ok(v) = serde_json::from_slice(bytes) {
        return Some(v);
    }
    let text = String::from_utf8_lossy(bytes);
    let no_comments: String = text
        .lines()
        .map(|l| match l.find("//") {
            Some(i) if !l[..i].contains('"') || l[..i].matches('"').count() % 2 == 0 => &l[..i],
            _ => l,
        })
        .collect::<Vec<_>>()
        .join("\n");
    let re = regex::Regex::new(r",(\s*[}\]])").ok()?;
    serde_json::from_str(&re.replace_all(&no_comments, "$1")).ok()
}

/// Itens `plugin` de um `marketplace.json` (ou `plugin.json` único).
#[allow(clippy::too_many_arguments)]
pub fn plugins_from_manifest(
    repo: &str,
    commit: Option<&str>,
    market: Option<&Value>,
    single: Option<&Value>,
    stars: Option<u64>,
    repo_license: Option<String>,
    updated: Option<String>,
    repo_description: Option<&str>,
) -> (String, Vec<CatalogItem>) {
    let mut entries: Vec<(Value, PluginSource)> = Vec::new();
    if let Some(m) = market {
        let root = m
            .get("metadata")
            .and_then(|x| x.get("pluginRoot"))
            .and_then(Value::as_str);
        for p in m
            .get("plugins")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            if p.get("name").and_then(Value::as_str).is_some() {
                let src =
                    plugin_source(p.get("source").unwrap_or(&Value::Null), repo, commit, root);
                entries.push((p.clone(), src));
            }
        }
    } else if let Some(s) = single.filter(|s| s.get("name").is_some()) {
        entries.push((
            s.clone(),
            PluginSource {
                repo: Some(repo.to_string()),
                commit: commit.map(str::to_string),
                ..Default::default()
            },
        ));
    }
    let m_name = market
        .and_then(|m| m.get("name"))
        .or_else(|| single.and_then(|s| s.get("name")))
        .and_then(Value::as_str)
        .map(str::to_string)
        .unwrap_or_else(|| repo.rsplit('/').next().unwrap_or(repo).to_string());
    let owner_name = market
        .and_then(|m| m.get("owner"))
        .and_then(|o| o.get("name"))
        .and_then(Value::as_str);
    let mut items = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for (p, src) in entries {
        let name = p["name"].as_str().unwrap_or_default().to_string();
        let mut norm: Vec<String> = Vec::new();
        if src.repo.is_some() && src.commit.is_none() {
            norm.push("unpinned_source".into());
        }
        let category = p
            .get("category")
            .and_then(Value::as_str)
            .map(|c| c.trim().to_lowercase().replace(char::is_whitespace, "-"))
            .filter(|c| !c.is_empty())
            .unwrap_or_else(|| "general".into());
        let raw_desc = p
            .get("description")
            .and_then(Value::as_str)
            .or_else(|| {
                market
                    .and_then(|m| m.get("metadata"))
                    .and_then(|x| x.get("description"))
                    .and_then(Value::as_str)
            })
            .or(repo_description)
            .map(str::to_string)
            .unwrap_or_else(|| humanize(&name));
        let (desc, marks) = short_description(&raw_desc);
        norm.extend(marks.into_iter().map(str::to_string));
        let author = match p.get("author") {
            Some(Value::String(s)) => Some(s.clone()),
            Some(Value::Object(o)) => o.get("name").and_then(Value::as_str).map(str::to_string),
            _ => None,
        }
        .or_else(|| owner_name.map(str::to_string))
        .or_else(|| repo.split('/').next().map(str::to_string));
        let mut tags = to_list(p.get("keywords"));
        tags.extend(to_list(p.get("tags")));
        tags.sort();
        tags.dedup();
        let same_repo = src.repo.as_deref() == Some(repo);
        let mut id = format!("{repo}:plugins/{category}/{name}");
        let mut n = 2;
        while !seen.insert(id.clone()) {
            id = format!("{repo}:plugins/{category}/{name}~{n}");
            n += 1;
            norm.push("renamed_collision".into());
        }
        let dir = src.path.clone();
        let url = match &src.repo {
            Some(r) => Some(
                format!(
                    "https://github.com/{r}/tree/{}/{dir}",
                    src.commit
                        .as_deref()
                        .or(src.git_ref.as_deref())
                        .unwrap_or("HEAD")
                )
                .trim_end_matches('/')
                .to_string(),
            ),
            None => src.url.clone(),
        };
        norm.sort();
        norm.dedup();
        items.push(CatalogItem {
            id,
            kind: "plugin".into(),
            name,
            category,
            description: desc,
            source: ItemSource {
                id: repo.to_string(),
                repo: src.repo.clone(),
                commit: src.commit.clone(),
                git_ref: src.git_ref.clone(),
                path: if dir.is_empty() {
                    ".claude-plugin/plugin.json".into()
                } else {
                    format!("{dir}/.claude-plugin/plugin.json")
                },
                dir,
                url,
                marketplace: Some(repo.to_string()),
                marketplace_name: Some(m_name.clone()),
                upstream: src.url.clone(),
                local: None,
            },
            license: normalize_license(p.get("license").unwrap_or(&Value::Null)).or_else(|| {
                if same_repo {
                    repo_license.clone()
                } else {
                    None
                }
            }),
            author,
            tags,
            origin_tool: "claude".into(),
            files: Vec::new(),
            entry: ".claude-plugin/plugin.json".into(),
            frontmatter: slim_json(&p),
            references: Vec::new(),
            stars: if same_repo { stars } else { None },
            updated: updated.clone(),
            normalization: norm,
            security: None,
            collides_with: Vec::new(),
            install_name: None,
        });
    }
    (m_name, items)
}

fn cache_file(repo: &str) -> Result<std::path::PathBuf> {
    Ok(catalog_dir()?
        .join("marketplaces")
        .join(format!("{}.json", repo.replace('/', "__"))))
}

/// Listagens de marketplace já buscadas (para o índice em memória).
pub fn cached_listings() -> Vec<(MarketplaceInfo, Vec<CatalogItem>)> {
    let Ok(dir) = catalog_dir().map(|d| d.join("marketplaces")) else {
        return Vec::new();
    };
    let Ok(rd) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for e in rd.flatten() {
        let Ok(bytes) = std::fs::read(e.path()) else {
            continue;
        };
        let Ok(v) = serde_json::from_slice::<Value>(&bytes) else {
            continue;
        };
        let info = serde_json::from_value::<MarketplaceInfo>(v["info"].clone()).ok();
        let items = serde_json::from_value::<Vec<CatalogItem>>(v["items"].clone()).ok();
        if let (Some(i), Some(it)) = (info, items) {
            out.push((i, it));
        }
    }
    out.sort_by(|a, b| a.0.repo.cmp(&b.0.repo));
    out
}

/// Remove a listagem guardada de um marketplace.
pub fn forget(repo: &str) -> Result<()> {
    let p = cache_file(repo)?;
    if p.exists() {
        std::fs::remove_file(&p).map_err(|e| super::io_err("remove marketplace cache", e))?;
    }
    super::index::invalidate();
    Ok(())
}

/// Normaliza `owner/repo`, URL do GitHub ou `github:owner/repo`.
pub fn parse_repo(spec: &str) -> Result<String> {
    let s = spec.trim().trim_start_matches("github:");
    let s = s
        .split("github.com/")
        .nth(1)
        .unwrap_or(s)
        .trim_end_matches('/')
        .trim_end_matches(".git");
    let mut parts = s.split('/');
    let repo = match (parts.next(), parts.next()) {
        (Some(o), Some(r)) => format!("{o}/{r}"),
        _ => String::new(),
    };
    if valid_repo(&repo) {
        Ok(repo)
    } else {
        Err(CatalogError::new(
            ERR_INVALID,
            format!("`{spec}` is not owner/repo"),
        ))
    }
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct MarketplaceListing {
    pub info: MarketplaceInfo,
    pub items: Vec<CatalogItem>,
}

/// Busca o marketplace de um repo, guarda e devolve os plugins.
pub async fn load(spec: &str) -> Result<MarketplaceListing> {
    let repo = parse_repo(spec)?;
    let info_api = github_api(&format!("repos/{repo}")).await.ok();
    let branch = info_api
        .as_ref()
        .and_then(|i| i["default_branch"].as_str())
        .unwrap_or("HEAD")
        .to_string();
    let commit = github_commit(&repo, &branch).await.ok();
    let at = commit.clone().unwrap_or_else(|| branch.clone());
    let market_bytes = get_bytes(
        &raw_url(&repo, &at, ".claude-plugin/marketplace.json"),
        None,
    )
    .await;
    let market = market_bytes.ok().and_then(|b| parse_json_loose(&b));
    let single = if market.is_none() {
        get_bytes(&raw_url(&repo, &at, ".claude-plugin/plugin.json"), None)
            .await
            .ok()
            .and_then(|b| parse_json_loose(&b))
    } else {
        None
    };
    if market.is_none() && single.is_none() {
        return Err(CatalogError::new(
            ERR_NOT_FOUND,
            format!("{repo}: no .claude-plugin/marketplace.json or plugin.json"),
        ));
    }
    let stars = info_api
        .as_ref()
        .and_then(|i| i["stargazers_count"].as_u64());
    let license = info_api
        .as_ref()
        .and_then(|i| i["license"]["spdx_id"].as_str())
        .filter(|s| *s != "NOASSERTION")
        .and_then(super::normalize::normalize_license_str);
    let updated = info_api
        .as_ref()
        .and_then(|i| i["pushed_at"].as_str())
        .map(|s| s.chars().take(10).collect::<String>());
    let repo_desc = info_api
        .as_ref()
        .and_then(|i| i["description"].as_str())
        .map(str::to_string);
    let (name, items) = plugins_from_manifest(
        &repo,
        commit.as_deref(),
        market.as_ref(),
        single.as_ref(),
        stars,
        license.clone(),
        updated.clone(),
        repo_desc.as_deref(),
    );
    let desc = market
        .as_ref()
        .and_then(|m| m["metadata"]["description"].as_str())
        .or_else(|| single.as_ref().and_then(|s| s["description"].as_str()))
        .map(str::to_string)
        .or(repo_desc);
    let info = MarketplaceInfo {
        repo: repo.clone(),
        name,
        kind: if market.is_some() {
            "marketplace"
        } else {
            "plugin"
        }
        .into(),
        description: desc.map(|d| short_description(&d).0),
        stars,
        license,
        updated,
        commit,
        website: info_api
            .as_ref()
            .and_then(|i| i["homepage"].as_str())
            .filter(|s| !s.is_empty())
            .map(str::to_string),
        plugins: items.len() as u64,
        url: Some(format!("https://github.com/{repo}")),
    };
    let blob = json!({ "info": info, "items": items });
    write_atomic(
        &cache_file(&repo)?,
        &serde_json::to_vec(&blob).unwrap_or_default(),
    )?;
    super::index::invalidate();
    Ok(MarketplaceListing { info, items })
}

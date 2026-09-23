//! Fontes extras do usuário, persistidas em `<app_data>/agentkit/sources.json`:
//!
//! - `git`: `owner/repo[/path][@ref]` ou URL do GitHub. O commit é resolvido,
//!   o tarball imutável (`codeload.github.com/<repo>/tar.gz/<sha>`) é extraído
//!   em `<app_data>/agentkit/sources/<id>/` e varrido por [`super::scan`]; os
//!   itens apontam para o repo+commit (conteúdo remoto verificado) e para a
//!   cópia local (leitura direta);
//! - `local`: uma pasta; varrida no lugar;
//! - `marketplace`: `owner/repo` com `.claude-plugin/marketplace.json`
//!   (ver [`super::marketplace`]).
//!
//! Os itens varridos ficam em `<app_data>/agentkit/catalog/sources/<id>.json`
//! e entram no índice em memória.

use std::io::Read;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::model::CatalogItem;
use super::scan::{scan, ScanSource};
use super::{
    agentkit_dir, catalog_dir, get_bytes, github_api, github_commit, io_err, valid_repo,
    write_atomic, CatalogError, Result, ERR_INVALID, ERR_LIMIT, ERR_NOT_FOUND, ERR_PARSE,
};

const MAX_TARBALL: usize = 200 * 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Source {
    pub id: String,
    /// `git`, `local` ou `marketplace`.
    pub kind: String,
    /// O que o usuário digitou (normalizado).
    pub spec: String,
    #[serde(default)]
    pub repo: Option<String>,
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default, rename = "ref")]
    pub git_ref: Option<String>,
    #[serde(default)]
    pub commit: Option<String>,
    pub added: String,
    #[serde(default)]
    pub scanned: Option<String>,
    #[serde(default)]
    pub items: usize,
    #[serde(default)]
    pub excluded: usize,
    #[serde(default)]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct SourcesFile {
    #[serde(default)]
    version: u32,
    #[serde(default)]
    sources: Vec<Source>,
}

fn sources_path() -> Result<PathBuf> {
    Ok(agentkit_dir()?.join("sources.json"))
}

fn items_path(id: &str) -> Result<PathBuf> {
    Ok(catalog_dir()?.join("sources").join(format!("{id}.json")))
}

fn load_file() -> SourcesFile {
    sources_path()
        .ok()
        .and_then(|p| std::fs::read(p).ok())
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

fn save_file(f: &SourcesFile) -> Result<()> {
    let bytes =
        serde_json::to_vec_pretty(f).map_err(|e| CatalogError::new(ERR_PARSE, e.to_string()))?;
    write_atomic(&sources_path()?, &bytes)
}

/// Fontes cadastradas.
pub fn list() -> Vec<Source> {
    load_file().sources
}

/// Itens em cache de cada fonte `git`/`local` (para o índice em memória).
pub fn cached_items() -> Vec<(String, Vec<CatalogItem>)> {
    list()
        .into_iter()
        .filter(|s| s.kind != "marketplace")
        .filter_map(|s| {
            let bytes = std::fs::read(items_path(&s.id).ok()?).ok()?;
            let items: Vec<CatalogItem> = serde_json::from_slice(&bytes).ok()?;
            Some((s.id, items))
        })
        .collect()
}

fn slug(s: &str) -> String {
    let mut out = String::new();
    for c in s.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if !out.ends_with('-') {
            out.push('-');
        }
    }
    let t = out.trim_matches('-');
    t.chars().take(60).collect()
}

/// Spec interpretado.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Spec {
    Local(PathBuf),
    Git {
        repo: String,
        path: Option<String>,
        git_ref: Option<String>,
    },
    Marketplace(String),
}

/// `owner/repo[/path][@ref]`, URL do GitHub (`/tree/<ref>/<path>`), pasta
/// existente, ou `marketplace:owner/repo`. `kind` força o tipo.
pub fn parse_spec(spec: &str, kind: Option<&str>) -> Result<Spec> {
    let s = spec.trim();
    if s.is_empty() {
        return Err(CatalogError::new(ERR_INVALID, "empty source"));
    }
    if kind == Some("local")
        || (kind.is_none() && (Path::new(s).is_absolute() || s.starts_with('~')))
    {
        let p = if let Some(rest) = s.strip_prefix('~') {
            dirs::home_dir()
                .ok_or_else(|| CatalogError::new(ERR_INVALID, "no home dir"))?
                .join(rest.trim_start_matches(['/', '\\']))
        } else {
            PathBuf::from(s)
        };
        if !p.is_dir() {
            return Err(CatalogError::new(ERR_NOT_FOUND, format!("folder `{s}`")));
        }
        return Ok(Spec::Local(p));
    }
    if let Some(r) = s.strip_prefix("marketplace:") {
        return Ok(Spec::Marketplace(super::marketplace::parse_repo(r)?));
    }
    if kind == Some("marketplace") {
        return Ok(Spec::Marketplace(super::marketplace::parse_repo(s)?));
    }
    let s = s.trim_start_matches("github:");
    let (body, url_ref) = if let Some(rest) = s.split("github.com/").nth(1) {
        let rest = rest.trim_end_matches('/').trim_end_matches(".git");
        let mut parts = rest.splitn(3, '/');
        let (o, r) = (parts.next().unwrap_or(""), parts.next().unwrap_or(""));
        let tail = parts.next().unwrap_or("");
        match tail
            .strip_prefix("tree/")
            .or_else(|| tail.strip_prefix("blob/"))
        {
            Some(t) => {
                let mut tp = t.splitn(2, '/');
                let gref = tp.next().map(str::to_string);
                let path = tp.next().unwrap_or("");
                (
                    if path.is_empty() {
                        format!("{o}/{r}")
                    } else {
                        format!("{o}/{r}/{path}")
                    },
                    gref,
                )
            }
            None => (format!("{o}/{r}"), None),
        }
    } else {
        (s.to_string(), None)
    };
    let (body, at_ref) = match body.rsplit_once('@') {
        Some((b, r)) if !r.is_empty() && !r.contains('/') => (b.to_string(), Some(r.to_string())),
        _ => (body, None),
    };
    let mut parts = body.splitn(3, '/');
    let (o, r) = (parts.next().unwrap_or(""), parts.next().unwrap_or(""));
    let repo = format!("{o}/{r}");
    if !valid_repo(&repo) {
        return Err(CatalogError::new(
            ERR_INVALID,
            format!("`{spec}` is not owner/repo[/path][@ref] nor a folder"),
        ));
    }
    let path = parts
        .next()
        .map(|p| p.trim_matches('/').to_string())
        .filter(|p| !p.is_empty());
    if let Some(p) = &path {
        super::fetch::safe_rel(p)?;
    }
    Ok(Spec::Git {
        repo,
        path,
        git_ref: at_ref.or(url_ref),
    })
}

fn now() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

/// Extrai um tar.gz do GitHub (primeiro componente de caminho removido).
fn extract_tarball(bytes: &[u8], dest: &Path) -> Result<()> {
    if dest.exists() {
        std::fs::remove_dir_all(dest).map_err(|e| io_err("clean source dir", e))?;
    }
    std::fs::create_dir_all(dest).map_err(|e| io_err("create source dir", e))?;
    let mut ar = tar::Archive::new(flate2::read::GzDecoder::new(bytes));
    let entries = ar.entries().map_err(|e| io_err("tarball", e))?;
    for entry in entries {
        let mut entry = entry.map_err(|e| io_err("tarball entry", e))?;
        let ty = entry.header().entry_type();
        if !(ty.is_file() || ty.is_dir()) {
            continue; // sem symlink/hardlink
        }
        let path = entry
            .path()
            .map_err(|e| io_err("tarball path", e))?
            .into_owned();
        let rel: PathBuf = path.components().skip(1).collect();
        if rel.as_os_str().is_empty() {
            continue;
        }
        let rel_str = rel.to_string_lossy().replace('\\', "/");
        let Ok(safe) = super::fetch::safe_rel(&rel_str) else {
            continue;
        };
        let out = dest.join(safe);
        if ty.is_dir() {
            std::fs::create_dir_all(&out).map_err(|e| io_err("mkdir", e))?;
            continue;
        }
        if let Some(parent) = out.parent() {
            std::fs::create_dir_all(parent).map_err(|e| io_err("mkdir", e))?;
        }
        let mut buf = Vec::new();
        entry
            .read_to_end(&mut buf)
            .map_err(|e| io_err("tarball read", e))?;
        std::fs::write(&out, buf).map_err(|e| io_err("write", e))?;
    }
    Ok(())
}

/// Busca/varre uma fonte e grava os itens. Atualiza os campos da fonte.
async fn refresh(src: &mut Source) -> Result<()> {
    match src.kind.as_str() {
        "marketplace" => {
            let l = super::marketplace::load(&src.spec).await?;
            src.commit = l.info.commit.clone();
            src.items = l.items.len();
            src.excluded = 0;
        }
        "local" => {
            let root = PathBuf::from(&src.spec);
            let id = src.id.clone();
            let report = tokio::task::spawn_blocking(move || {
                scan(
                    &root,
                    &ScanSource {
                        id,
                        local_root: Some(root.clone()),
                        updated: Some(chrono::Utc::now().format("%Y-%m-%d").to_string()),
                        ..Default::default()
                    },
                )
            })
            .await
            .map_err(|e| io_err("scan", e))?;
            src.items = report.items.len();
            src.excluded = report.excluded.len();
            write_items(&src.id, &report.items)?;
        }
        "git" => {
            let repo = src
                .repo
                .clone()
                .ok_or_else(|| CatalogError::new(ERR_INVALID, "git source without repo"))?;
            let info = github_api(&format!("repos/{repo}")).await.ok();
            let gref = src
                .git_ref
                .clone()
                .or_else(|| {
                    info.as_ref()
                        .and_then(|i| i["default_branch"].as_str().map(str::to_string))
                })
                .unwrap_or_else(|| "HEAD".into());
            let commit = github_commit(&repo, &gref).await?;
            let url = format!("https://codeload.github.com/{repo}/tar.gz/{commit}");
            let bytes = get_bytes(&url, None).await?;
            if bytes.len() > MAX_TARBALL {
                return Err(CatalogError::new(
                    ERR_LIMIT,
                    format!("{repo}: tarball over 200 MB"),
                ));
            }
            let dest = agentkit_dir()?.join("sources").join(&src.id);
            let dest2 = dest.clone();
            tokio::task::spawn_blocking(move || extract_tarball(&bytes, &dest2))
                .await
                .map_err(|e| io_err("extract", e))??;
            let prefix = src.path.clone().unwrap_or_default();
            let root = if prefix.is_empty() {
                dest.clone()
            } else {
                dest.join(&prefix)
            };
            if !root.is_dir() {
                return Err(CatalogError::new(
                    ERR_NOT_FOUND,
                    format!("{repo}: path `{prefix}` not found"),
                ));
            }
            let license = info
                .as_ref()
                .and_then(|i| i["license"]["spdx_id"].as_str())
                .filter(|s| *s != "NOASSERTION")
                .and_then(super::normalize::normalize_license_str);
            let updated = info
                .as_ref()
                .and_then(|i| i["pushed_at"].as_str())
                .map(|s| s.chars().take(10).collect::<String>());
            let stars = info.as_ref().and_then(|i| i["stargazers_count"].as_u64());
            let scan_src = ScanSource {
                id: src.id.clone(),
                repo: Some(repo.clone()),
                commit: Some(commit.clone()),
                repo_prefix: prefix,
                local_root: Some(dest.clone()),
                updated,
                default_license: license,
            };
            let mut report = tokio::task::spawn_blocking(move || scan(&root, &scan_src))
                .await
                .map_err(|e| io_err("scan", e))?;
            // a cópia local é a raiz do repo; `source.dir` já é relativo a ela
            for it in &mut report.items {
                it.stars = stars;
            }
            src.commit = Some(commit);
            src.items = report.items.len();
            src.excluded = report.excluded.len();
            write_items(&src.id, &report.items)?;
        }
        other => {
            return Err(CatalogError::new(
                ERR_INVALID,
                format!("unknown source kind `{other}`"),
            ))
        }
    }
    src.scanned = Some(now());
    src.error = None;
    Ok(())
}

fn write_items(id: &str, items: &[CatalogItem]) -> Result<()> {
    let bytes =
        serde_json::to_vec(items).map_err(|e| CatalogError::new(ERR_PARSE, e.to_string()))?;
    write_atomic(&items_path(id)?, &bytes)
}

/// Cadastra e varre uma fonte (`kind`: `git`/`local`/`marketplace`, ou deduzido).
pub async fn add(spec: &str, kind: Option<&str>) -> Result<Source> {
    let parsed = parse_spec(spec, kind)?;
    let mut src = match &parsed {
        Spec::Local(p) => Source {
            id: format!("user-local-{}", slug(&p.to_string_lossy())),
            kind: "local".into(),
            spec: p.to_string_lossy().into_owned(),
            repo: None,
            path: None,
            git_ref: None,
            commit: None,
            added: now(),
            scanned: None,
            items: 0,
            excluded: 0,
            error: None,
        },
        Spec::Git {
            repo,
            path,
            git_ref,
        } => Source {
            id: format!(
                "user-{}",
                slug(&format!("{repo}-{}", path.clone().unwrap_or_default()))
            ),
            kind: "git".into(),
            spec: format!(
                "{repo}{}{}",
                path.as_ref().map(|p| format!("/{p}")).unwrap_or_default(),
                git_ref
                    .as_ref()
                    .map(|r| format!("@{r}"))
                    .unwrap_or_default()
            ),
            repo: Some(repo.clone()),
            path: path.clone(),
            git_ref: git_ref.clone(),
            commit: None,
            added: now(),
            scanned: None,
            items: 0,
            excluded: 0,
            error: None,
        },
        Spec::Marketplace(repo) => Source {
            id: format!("marketplace-{}", slug(repo)),
            kind: "marketplace".into(),
            spec: repo.clone(),
            repo: Some(repo.clone()),
            path: None,
            git_ref: None,
            commit: None,
            added: now(),
            scanned: None,
            items: 0,
            excluded: 0,
            error: None,
        },
    };
    if let Some(existing) = load_file().sources.iter().find(|s| s.id == src.id) {
        src.added = existing.added.clone();
    }
    refresh(&mut src).await?;
    // relê: outra chamada pode ter mexido na lista enquanto baixava
    let mut file = load_file();
    file.version = 1;
    file.sources.retain(|s| s.id != src.id);
    file.sources.push(src.clone());
    save_file(&file)?;
    super::index::invalidate();
    Ok(src)
}

/// Varre de novo uma fonte cadastrada.
pub async fn rescan(id: &str) -> Result<Source> {
    let mut file = load_file();
    let mut src = file
        .sources
        .iter()
        .find(|s| s.id == id)
        .cloned()
        .ok_or_else(|| CatalogError::new(ERR_NOT_FOUND, format!("source `{id}`")))?;
    let res = refresh(&mut src).await;
    if let Err(e) = &res {
        src.error = Some(e.to_string());
    }
    file.sources.retain(|s| s.id != id);
    file.sources.push(src.clone());
    save_file(&file)?;
    super::index::invalidate();
    res.map(|_| src)
}

/// Remove uma fonte, seu cache de itens e a cópia extraída.
pub fn remove(id: &str) -> Result<()> {
    let mut file = load_file();
    let Some(src) = file.sources.iter().find(|s| s.id == id).cloned() else {
        return Err(CatalogError::new(ERR_NOT_FOUND, format!("source `{id}`")));
    };
    file.sources.retain(|s| s.id != id);
    save_file(&file)?;
    let _ = std::fs::remove_file(items_path(id)?);
    let extracted = agentkit_dir()?.join("sources").join(id);
    if extracted.is_dir() {
        let _ = std::fs::remove_dir_all(extracted);
    }
    if src.kind == "marketplace" {
        if let Some(r) = src.repo.as_deref() {
            super::marketplace::forget(r)?;
        }
    }
    super::index::invalidate();
    Ok(())
}

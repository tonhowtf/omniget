//! Conteúdo de um item: baixado no commit pinado, conferido e guardado em cache
//! endereçado por hash (`<app_data>/agentkit/catalog/content/<sha256>`).
//!
//! - item com `files` (índice/fonte do usuário): cada arquivo de
//!   `raw.githubusercontent.com/<repo>/<commit>/<dir>/<path>` (ou da raiz local),
//!   com o sha256 do índice conferido; skill/mod/template trazem a pasta inteira;
//! - plugin de marketplace (sem lista de arquivos): árvore do GitHub no commit,
//!   cada blob conferido pelo sha1 de blob do git;
//! - MCP Registry: arquivos gerados a partir do `server.json`.

use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};

use base64::Engine;
use futures::{StreamExt, TryStreamExt};
use serde::Serialize;

use super::model::{CatalogItem, ItemFile};
use super::{
    catalog_dir, get_bytes, github_api, github_commit, raw_url, sha256_hex, write_atomic,
    CatalogError, Result, ERR_HASH, ERR_INVALID, ERR_LIMIT,
};

pub const MAX_FILES: usize = 2000;
pub const MAX_BYTES: u64 = 64 * 1024 * 1024;
const PARALLEL: usize = 8;

#[derive(Debug, Clone, Serialize)]
pub struct FileContent {
    /// `utf8` ou `base64`.
    pub encoding: &'static str,
    pub content: String,
    pub sha256: String,
    pub size: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct ItemFiles {
    pub id: String,
    pub entry: String,
    /// `sha256` (índice), `git-blob` (árvore do GitHub) ou `generated` (registry).
    pub verified: &'static str,
    pub total_bytes: u64,
    pub files: BTreeMap<String, FileContent>,
}

/// Caminho relativo seguro (sem `..`, sem raiz, sem prefixo de drive).
pub fn safe_rel(p: &str) -> Result<PathBuf> {
    let path = Path::new(p);
    if p.is_empty() || p.contains('\\') || p.contains('\0') {
        return Err(CatalogError::new(ERR_INVALID, format!("unsafe path `{p}`")));
    }
    let mut out = PathBuf::new();
    for c in path.components() {
        match c {
            Component::Normal(s) => out.push(s),
            Component::CurDir => {}
            _ => return Err(CatalogError::new(ERR_INVALID, format!("unsafe path `{p}`"))),
        }
    }
    if out.as_os_str().is_empty() {
        return Err(CatalogError::new(ERR_INVALID, format!("unsafe path `{p}`")));
    }
    Ok(out)
}

fn join_repo(dir: &str, rel: &str) -> String {
    let d = dir.trim_matches('/');
    if d.is_empty() {
        rel.to_string()
    } else {
        format!("{d}/{rel}")
    }
}

fn content_dir() -> Result<PathBuf> {
    Ok(catalog_dir()?.join("content"))
}

/// Lê do cache por hash (conferindo de novo).
pub fn cached(sha: &str) -> Option<Vec<u8>> {
    if sha.len() != 64 || !sha.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let p = content_dir().ok()?.join(sha);
    let bytes = std::fs::read(p).ok()?;
    (sha256_hex(&bytes) == sha).then_some(bytes)
}

fn store(bytes: &[u8]) -> String {
    let sha = sha256_hex(bytes);
    if let Ok(dir) = content_dir() {
        let p = dir.join(&sha);
        if !p.exists() {
            let _ = write_atomic(&p, bytes);
        }
    }
    sha
}

fn verify(expected: &str, bytes: &[u8], what: &str) -> Result<()> {
    let got = sha256_hex(bytes);
    if got != expected {
        return Err(CatalogError::new(
            ERR_HASH,
            format!("{what}: expected {expected}, got {got}"),
        ));
    }
    Ok(())
}

async fn fetch_indexed(item: &CatalogItem, f: &ItemFile) -> Result<Vec<u8>> {
    safe_rel(&f.path)?;
    if let Some(b) = cached(&f.sha256) {
        return Ok(b);
    }
    let what = format!("{}:{}", item.id, f.path);
    if let Some(root) = item.source.local.as_deref() {
        let mut p = PathBuf::from(root);
        if !item.source.dir.is_empty() {
            p.push(safe_rel(&item.source.dir)?);
        }
        p.push(safe_rel(&f.path)?);
        if let Ok(bytes) = tokio::fs::read(&p).await {
            verify(&f.sha256, &bytes, &what)?;
            store(&bytes);
            return Ok(bytes);
        }
    }
    let (Some(repo), Some(commit)) = (item.source.repo.as_deref(), item.source.commit.as_deref())
    else {
        return Err(CatalogError::new(
            ERR_INVALID,
            format!("{what}: no repo/commit and no local copy"),
        ));
    };
    let url = raw_url(repo, commit, &join_repo(&item.source.dir, &f.path));
    let bytes = get_bytes(&url, None).await?;
    verify(&f.sha256, &bytes, &what)?;
    store(&bytes);
    Ok(bytes)
}

/// sha1 de blob do git: `sha1("blob <len>\0" + bytes)`.
pub fn git_blob_sha1(bytes: &[u8]) -> String {
    use sha1::{Digest, Sha1};
    let mut h = Sha1::new();
    h.update(format!("blob {}\0", bytes.len()).as_bytes());
    h.update(bytes);
    hex::encode(h.finalize())
}

async fn fetch_plugin_tree(item: &CatalogItem) -> Result<BTreeMap<String, Vec<u8>>> {
    let repo = item
        .source
        .repo
        .as_deref()
        .filter(|r| super::valid_repo(r))
        .ok_or_else(|| {
            CatalogError::new(
                ERR_INVALID,
                format!(
                    "{}: plugin source is not a GitHub repo ({})",
                    item.id,
                    item.source.upstream.as_deref().unwrap_or("?")
                ),
            )
        })?;
    let commit = match item.source.commit.as_deref() {
        Some(c) => c.to_string(),
        None => github_commit(repo, item.source.git_ref.as_deref().unwrap_or("HEAD")).await?,
    };
    let tree = github_api(&format!("repos/{repo}/git/trees/{commit}?recursive=1")).await?;
    let prefix = item.source.dir.trim_matches('/').to_string();
    let mut blobs: Vec<(String, String, u64)> = Vec::new();
    for e in tree["tree"].as_array().into_iter().flatten() {
        if e["type"] != "blob" {
            continue;
        }
        let path = e["path"].as_str().unwrap_or_default();
        let rel = if prefix.is_empty() {
            path.to_string()
        } else if let Some(r) = path.strip_prefix(&format!("{prefix}/")) {
            r.to_string()
        } else {
            continue;
        };
        if rel.split('/').any(|s| s == ".git") {
            continue;
        }
        blobs.push((
            rel,
            e["sha"].as_str().unwrap_or_default().to_string(),
            e["size"].as_u64().unwrap_or(0),
        ));
    }
    if blobs.is_empty() {
        return Err(CatalogError::new(
            super::ERR_NOT_FOUND,
            format!("{}: no files under `{prefix}` at {commit}", item.id),
        ));
    }
    let total: u64 = blobs.iter().map(|b| b.2).sum();
    if blobs.len() > MAX_FILES || total > MAX_BYTES {
        return Err(CatalogError::new(
            ERR_LIMIT,
            format!(
                "{}: {} files / {total} bytes is over the limit",
                item.id,
                blobs.len()
            ),
        ));
    }
    let results: Vec<(String, Vec<u8>)> =
        futures::stream::iter(blobs.into_iter().map(|(rel, sha1, _)| {
            let url = raw_url(repo, &commit, &join_repo(&prefix, &rel));
            let id = item.id.clone();
            async move {
                safe_rel(&rel)?;
                let bytes = get_bytes(&url, None).await?;
                let got = git_blob_sha1(&bytes);
                if got != sha1 {
                    return Err(CatalogError::new(
                        ERR_HASH,
                        format!("{id}:{rel}: git blob expected {sha1}, got {got}"),
                    ));
                }
                store(&bytes);
                Ok::<_, CatalogError>((rel, bytes))
            }
        }))
        .buffer_unordered(PARALLEL)
        .try_collect()
        .await?;
    Ok(results.into_iter().collect())
}

/// Bytes crus de todos os arquivos do item (o que o `agentkit::parse_raw` consome).
pub async fn item_bytes(item: &CatalogItem) -> Result<(BTreeMap<String, Vec<u8>>, &'static str)> {
    if item.source.id == super::registry::SOURCE_ID {
        return Ok((super::registry::generated_files(item)?, "generated"));
    }
    if item.files.is_empty() {
        if item.kind == "plugin" {
            return Ok((fetch_plugin_tree(item).await?, "git-blob"));
        }
        return Err(CatalogError::new(
            ERR_INVALID,
            format!("{}: item has no files", item.id),
        ));
    }
    if item.files.len() > MAX_FILES || item.files.iter().map(|f| f.size).sum::<u64>() > MAX_BYTES {
        return Err(CatalogError::new(
            ERR_LIMIT,
            format!("{}: too large", item.id),
        ));
    }
    // Owned files: a closure over `&ItemFile` makes the future not Send-general
    // enough for a Tauri async command (`run_wizard_detect`).
    let files: Vec<_> = item.files.to_vec();
    let results: Vec<(String, Vec<u8>)> =
        futures::stream::iter(files.into_iter().map(|f| async move {
            let b = fetch_indexed(item, &f).await?;
            Ok::<_, CatalogError>((f.path, b))
        }))
        .buffer_unordered(PARALLEL)
        .try_collect()
        .await?;
    Ok((results.into_iter().collect(), "sha256"))
}

/// Codifica bytes: UTF-8 válido vai como texto; o resto como base64.
pub fn encode(bytes: &[u8]) -> FileContent {
    let sha256 = sha256_hex(bytes);
    let size = bytes.len() as u64;
    match std::str::from_utf8(bytes) {
        Ok(s) if !s.contains('\0') => FileContent {
            encoding: "utf8",
            content: s.to_string(),
            sha256,
            size,
        },
        _ => FileContent {
            encoding: "base64",
            content: base64::engine::general_purpose::STANDARD.encode(bytes),
            sha256,
            size,
        },
    }
}

/// Baixa, confere e devolve o mapa caminho → conteúdo.
pub async fn item_files(item: &CatalogItem) -> Result<ItemFiles> {
    let (bytes, verified) = item_bytes(item).await?;
    let total_bytes = bytes.values().map(|b| b.len() as u64).sum();
    Ok(ItemFiles {
        id: item.id.clone(),
        entry: item.entry.clone(),
        verified,
        total_bytes,
        files: bytes.iter().map(|(k, v)| (k.clone(), encode(v))).collect(),
    })
}

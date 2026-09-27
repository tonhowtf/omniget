//! `catalog`: o Catálogo da Central de agentes.
//!
//! Guarda itens **crus** (arquivos como estão na fonte) + metadados; quem
//! converte é o `agentkit` (`parse_raw`). Partes:
//!
//! - [`index`]: índice embutido (`static/agentkit/catalog-index.json.gz`,
//!   gerado por `scripts/agentkit-catalog/build.mjs` a partir do
//!   claude-code-templates pinado) ou atualizado em `<app_data>/agentkit/catalog/`,
//!   mesclado com as fontes do usuário;
//! - [`search`]: busca por texto, filtros, facetas com contagem e ordenação;
//! - [`fetch`]: conteúdo de um item, baixado de `raw.githubusercontent.com` no
//!   commit pinado, conferido pelo sha256 do índice e guardado em cache por hash;
//! - [`registry`]: Official MCP Registry convertido em itens `mcp`;
//! - [`marketplace`]: `.claude-plugin/marketplace.json` de um repo → itens `plugin`;
//! - [`scan`]: o mesmo gerador portado para Rust (descoberta de tipo por
//!   caminho/arquivo), usado pelas fontes locais/Git do usuário ([`sources`]);
//! - [`collections`]: coleções/stacks locais em SQLite + arquivo `.omnistack`.
//!
//! Nada roda em repouso: rede só quando um comando pede.

pub mod collections;
pub mod fetch;
pub mod frontmatter;
pub mod index;
pub mod marketplace;
pub mod model;
pub mod normalize;
pub mod registry;
pub mod scan;
pub mod search;
pub mod sources;

use std::path::PathBuf;
use std::sync::OnceLock;
use std::time::Duration;

/// Erro com código estável; vai para o front como `"CODE: mensagem"`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CatalogError {
    pub code: &'static str,
    pub message: String,
}

impl CatalogError {
    pub fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

impl std::fmt::Display for CatalogError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for CatalogError {}

impl From<CatalogError> for String {
    fn from(e: CatalogError) -> Self {
        e.to_string()
    }
}

pub type Result<T> = std::result::Result<T, CatalogError>;

pub const ERR_IO: &str = "CATALOG_IO";
pub const ERR_NETWORK: &str = "CATALOG_NETWORK";
pub const ERR_PARSE: &str = "CATALOG_PARSE";
pub const ERR_NOT_FOUND: &str = "CATALOG_NOT_FOUND";
pub const ERR_HASH: &str = "CATALOG_HASH_MISMATCH";
pub const ERR_INVALID: &str = "CATALOG_INVALID";
pub const ERR_DB: &str = "CATALOG_DB";
pub const ERR_LIMIT: &str = "CATALOG_LIMIT";

pub(crate) fn io_err(what: &str, e: impl std::fmt::Display) -> CatalogError {
    CatalogError::new(ERR_IO, format!("{what}: {e}"))
}

/// `<app_data>/agentkit`.
pub fn agentkit_dir() -> Result<PathBuf> {
    let base = crate::core::paths::app_data_dir()
        .ok_or_else(|| CatalogError::new(ERR_IO, "app data dir unavailable"))?;
    Ok(base.join("agentkit"))
}

/// `<app_data>/agentkit/catalog` (índice atualizado, caches).
pub fn catalog_dir() -> Result<PathBuf> {
    Ok(agentkit_dir()?.join("catalog"))
}

/// sha256 em hex minúsculo.
pub fn sha256_hex(data: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(data);
    hex::encode(h.finalize())
}

/// Escrita atômica (arquivo temporário + rename) criando a pasta.
pub(crate) fn write_atomic(path: &std::path::Path, data: &[u8]) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| io_err("create dir", e))?;
    }
    let tmp = path.with_extension(format!(
        "tmp-{}",
        uuid::Uuid::new_v4()
            .simple()
            .to_string()
            .get(..8)
            .unwrap_or("x")
    ));
    std::fs::write(&tmp, data).map_err(|e| io_err("write", e))?;
    std::fs::rename(&tmp, path).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        io_err("rename", e)
    })
}

/// Cliente HTTP do catálogo (proxy global do app, UA próprio, timeout).
pub(crate) fn http() -> reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT
        .get_or_init(|| {
            let b = reqwest::Client::builder()
                .user_agent(concat!("OmniGet-Catalog/", env!("CARGO_PKG_VERSION")))
                .connect_timeout(Duration::from_secs(15))
                .timeout(Duration::from_secs(60));
            crate::core::http_client::apply_global_proxy(b)
                .build()
                .unwrap_or_else(|_| reqwest::Client::new())
        })
        .clone()
}

/// GET que devolve bytes; erro de rede/status vira `CATALOG_NETWORK`.
pub(crate) async fn get_bytes(url: &str, accept: Option<&str>) -> Result<Vec<u8>> {
    let mut req = http().get(url);
    if let Some(a) = accept {
        req = req.header(reqwest::header::ACCEPT, a);
    }
    let resp = req
        .send()
        .await
        .map_err(|e| CatalogError::new(ERR_NETWORK, format!("{url}: {e}")))?;
    let status = resp.status();
    if status == reqwest::StatusCode::NOT_FOUND {
        return Err(CatalogError::new(ERR_NOT_FOUND, format!("{url}: 404")));
    }
    if !status.is_success() {
        let limited = status == reqwest::StatusCode::FORBIDDEN
            || status == reqwest::StatusCode::TOO_MANY_REQUESTS;
        return Err(CatalogError::new(
            if limited { ERR_LIMIT } else { ERR_NETWORK },
            format!("{url}: HTTP {status}"),
        ));
    }
    resp.bytes()
        .await
        .map(|b| b.to_vec())
        .map_err(|e| CatalogError::new(ERR_NETWORK, format!("{url}: {e}")))
}

/// GET na API do GitHub (sem token; o limite anônimo é tolerado por quem chama).
pub(crate) async fn github_api(path: &str) -> Result<serde_json::Value> {
    let url = format!("https://api.github.com/{}", path.trim_start_matches('/'));
    let bytes = get_bytes(&url, Some("application/vnd.github+json")).await?;
    serde_json::from_slice(&bytes).map_err(|e| CatalogError::new(ERR_PARSE, format!("{url}: {e}")))
}

/// Resolve `ref` (branch/tag/`HEAD`) de um repo para o sha do commit.
pub(crate) async fn github_commit(repo: &str, git_ref: &str) -> Result<String> {
    let url = format!(
        "https://api.github.com/repos/{repo}/commits/{}",
        urlencoding::encode(git_ref)
    );
    let bytes = get_bytes(&url, Some("application/vnd.github.sha")).await?;
    let sha = String::from_utf8_lossy(&bytes).trim().to_string();
    if sha.len() == 40 && sha.chars().all(|c| c.is_ascii_hexdigit()) {
        Ok(sha)
    } else {
        Err(CatalogError::new(
            ERR_PARSE,
            format!("{repo}@{git_ref}: unexpected sha"),
        ))
    }
}

/// URL crua de um arquivo num commit, com cada segmento codificado.
pub(crate) fn raw_url(repo: &str, commit: &str, path: &str) -> String {
    let enc: Vec<String> = path
        .split('/')
        .filter(|s| !s.is_empty())
        .map(|s| urlencoding::encode(s).into_owned())
        .collect();
    format!(
        "https://raw.githubusercontent.com/{repo}/{commit}/{}",
        enc.join("/")
    )
}

/// `owner/repo` válido (sem `..`, sem espaços).
pub(crate) fn valid_repo(repo: &str) -> bool {
    let mut parts = repo.split('/');
    let (Some(o), Some(r), None) = (parts.next(), parts.next(), parts.next()) else {
        return false;
    };
    let ok = |s: &str| {
        !s.is_empty()
            && s != "."
            && s != ".."
            && s.chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.')
    };
    ok(o) && ok(r)
}

#[cfg(test)]
mod tests;

//! Instagram pela sessão web (estudos 67-68): tudo aqui usa os cookies que a
//! extensão captura (`sessionid`, `csrftoken`, `ds_user_id`) e os mesmos
//! endpoints `/api/v1/...` e `/graphql/query` que o instagram.com chama no
//! navegador. Sem senha, sem API mobile, sem token de app.
//!
//! Regras que valem para todos os submódulos:
//! * GET leva `X-IG-App-ID`, `X-ASBD-ID`, `X-IG-WWW-Claim` e `X-Requested-With`;
//!   POST leva só `X-IG-App-ID` + `X-CSRFToken` (com `X-Requested-With` o
//!   Instagram responde 403, verificado em 2026-09-05).
//! * Redirect para `/accounts/login/` = sessão caiu; 429 = esperar; corpo com
//!   `checkpoint_required`/`challenge_required` = abrir o app e resolver.
//! * Paginação com pausa aleatória entre páginas; ações (unfollow, remover
//!   seguidor) com pausa longa, lote pequeno e teto diário.

pub mod analytics;
pub mod follow;
pub mod media;
pub mod profile;
pub mod publish;
pub mod social;

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use anyhow::{anyhow, bail};
use reqwest::header::{HeaderMap, HeaderValue};
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const BASE: &str = "https://www.instagram.com";
pub const APP_ID: &str = "936619743392459";
pub const ASBD_ID: &str = "129477";
const UA: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36";
const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";

// ── Erros ────────────────────────────────────────────────────────────────

#[derive(Debug, thiserror::Error)]
pub enum IgError {
    #[error("sessao do Instagram nao encontrada: capture os cookies pela extensao do OmniGet com o instagram.com aberto e logado")]
    NoSession,
    #[error("a sessao do Instagram expirou ou foi desconectada; capture os cookies de novo pela extensao")]
    LoginRequired,
    #[error("o Instagram pediu para esperar (429). Tente de novo em alguns minutos")]
    RateLimited,
    #[error("o Instagram pediu uma verificacao (checkpoint). Abra o app ou o site, resolva o aviso e tente de novo")]
    Checkpoint,
    #[error("o Instagram bloqueou esta acao por enquanto (feedback_required). Espere algumas horas antes de continuar")]
    ActionBlocked,
    #[error("nao encontrado: {0}")]
    NotFound(String),
    #[error("conteudo privado: voce precisa seguir esta conta para ver isso")]
    Private,
    #[error("{0}")]
    Other(String),
}

// ── Sessão ───────────────────────────────────────────────────────────────

/// Um cookie no formato Netscape que o gerenciador de cookies do app grava.
#[derive(Debug, Clone)]
pub struct NetscapeCookie {
    pub domain: String,
    pub path: String,
    pub secure: bool,
    pub name: String,
    pub value: String,
}

pub fn parse_netscape(content: &str) -> Vec<NetscapeCookie> {
    let mut out = Vec::new();
    for raw in content.lines() {
        let line = raw.trim();
        if line.is_empty() {
            continue;
        }
        let line = line.strip_prefix("#HttpOnly_").unwrap_or(line);
        if line.starts_with('#') {
            continue;
        }
        let cols: Vec<&str> = line.split('\t').collect();
        if cols.len() < 7 {
            continue;
        }
        out.push(NetscapeCookie {
            domain: cols[0].trim_start_matches('.').to_lowercase(),
            path: cols[2].to_string(),
            secure: cols[3].eq_ignore_ascii_case("TRUE"),
            name: cols[5].to_string(),
            value: cols[6..].join("\t"),
        });
    }
    out
}

#[derive(Debug, Clone, Serialize)]
pub struct SessionInfo {
    pub user_id: String,
    pub has_session: bool,
    pub cookie_count: usize,
}

/// Cookies de uma conta + o que o cliente precisa saber deles.
#[derive(Debug, Clone)]
pub struct Session {
    pub cookies: Vec<NetscapeCookie>,
    pub user_id: String,
    pub csrf: String,
}

impl Session {
    pub fn from_netscape(content: &str) -> Result<Self, IgError> {
        let cookies: Vec<NetscapeCookie> = parse_netscape(content)
            .into_iter()
            .filter(|c| c.domain.ends_with("instagram.com"))
            .collect();
        let get = |n: &str| {
            cookies
                .iter()
                .find(|c| c.name == n)
                .map(|c| c.value.clone())
        };
        let session_id = get("sessionid").filter(|v| !v.is_empty());
        let user_id = get("ds_user_id").filter(|v| !v.is_empty());
        match (session_id, user_id) {
            (Some(_), Some(user_id)) => Ok(Session {
                csrf: get("csrftoken").unwrap_or_default(),
                user_id,
                cookies,
            }),
            _ => Err(IgError::NoSession),
        }
    }

    pub fn info(&self) -> SessionInfo {
        SessionInfo {
            user_id: self.user_id.clone(),
            has_session: true,
            cookie_count: self.cookies.len(),
        }
    }
}

// ── Cliente ──────────────────────────────────────────────────────────────

pub struct IgClient {
    http: reqwest::Client,
    pub session: Session,
    www_claim: Mutex<String>,
    csrf: Mutex<String>,
    /// Pausa entre requisições de paginação (min, max) em ms.
    pub page_delay_ms: (u64, u64),
}

impl IgClient {
    pub fn new(session: Session) -> anyhow::Result<Arc<Self>> {
        let jar = reqwest::cookie::Jar::default();
        for c in &session.cookies {
            let scheme = if c.secure { "https" } else { "http" };
            let url: reqwest::Url = format!(
                "{}://{}{}",
                scheme,
                c.domain,
                if c.path.is_empty() { "/" } else { &c.path }
            )
            .parse()?;
            jar.add_cookie_str(
                &format!(
                    "{}={}; Domain={}; Path={}",
                    c.name,
                    c.value,
                    c.domain,
                    if c.path.is_empty() { "/" } else { &c.path }
                ),
                &url,
            );
        }
        let mut headers = HeaderMap::new();
        headers.insert(reqwest::header::USER_AGENT, HeaderValue::from_static(UA));
        headers.insert(
            reqwest::header::ACCEPT_LANGUAGE,
            HeaderValue::from_static("en-US,en;q=0.9,pt-BR;q=0.8"),
        );
        let http = crate::core::http_client::apply_global_proxy(reqwest::Client::builder())
            .default_headers(headers)
            .cookie_provider(Arc::new(jar))
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(120))
            .build()?;
        let csrf = session.csrf.clone();
        Ok(Arc::new(Self {
            http,
            session,
            www_claim: Mutex::new("0".into()),
            csrf: Mutex::new(csrf),
            page_delay_ms: (1200, 2600),
        }))
    }

    fn api_headers(&self) -> HeaderMap {
        let mut h = HeaderMap::new();
        h.insert("X-IG-App-ID", HeaderValue::from_static(APP_ID));
        h.insert("X-ASBD-ID", HeaderValue::from_static(ASBD_ID));
        h.insert(
            "X-IG-WWW-Claim",
            HeaderValue::from_str(&self.www_claim.lock().unwrap())
                .unwrap_or(HeaderValue::from_static("0")),
        );
        h.insert(
            "X-Requested-With",
            HeaderValue::from_static("XMLHttpRequest"),
        );
        h.insert("Accept", HeaderValue::from_static("*/*"));
        h.insert(
            "Referer",
            HeaderValue::from_static("https://www.instagram.com/"),
        );
        h.insert("Sec-Fetch-Dest", HeaderValue::from_static("empty"));
        h.insert("Sec-Fetch-Mode", HeaderValue::from_static("cors"));
        h.insert("Sec-Fetch-Site", HeaderValue::from_static("same-origin"));
        if let Ok(v) = HeaderValue::from_str(&self.csrf.lock().unwrap()) {
            h.insert("X-CSRFToken", v);
        }
        h
    }

    fn post_headers(&self) -> HeaderMap {
        let mut h = HeaderMap::new();
        h.insert("X-IG-App-ID", HeaderValue::from_static(APP_ID));
        h.insert("Accept", HeaderValue::from_static("*/*"));
        h.insert(
            "Origin",
            HeaderValue::from_static("https://www.instagram.com"),
        );
        h.insert(
            "Referer",
            HeaderValue::from_static("https://www.instagram.com/"),
        );
        if let Ok(v) = HeaderValue::from_str(&self.csrf.lock().unwrap()) {
            h.insert("X-CSRFToken", v);
        }
        h
    }

    fn absorb(&self, resp: &reqwest::Response) {
        if let Some(c) = resp
            .headers()
            .get("x-ig-set-www-claim")
            .and_then(|v| v.to_str().ok())
        {
            *self.www_claim.lock().unwrap() = c.to_string();
        }
        for sc in resp.headers().get_all(reqwest::header::SET_COOKIE) {
            if let Ok(s) = sc.to_str() {
                if let Some(v) = s.strip_prefix("csrftoken=") {
                    let v = v.split(';').next().unwrap_or("").trim();
                    if !v.is_empty() {
                        *self.csrf.lock().unwrap() = v.to_string();
                    }
                }
            }
        }
    }

    async fn finish(&self, resp: reqwest::Response, what: &str) -> Result<Value, IgError> {
        self.absorb(&resp);
        let status = resp.status();
        if status.is_redirection() {
            let loc = resp
                .headers()
                .get(reqwest::header::LOCATION)
                .and_then(|v| v.to_str().ok())
                .unwrap_or("");
            if loc.contains("/accounts/login") || loc.contains("/challenge") {
                return Err(if loc.contains("/challenge") {
                    IgError::Checkpoint
                } else {
                    IgError::LoginRequired
                });
            }
            return Err(IgError::Other(format!(
                "{}: redirecionado para {}",
                what, loc
            )));
        }
        let text = resp
            .text()
            .await
            .map_err(|e| IgError::Other(e.to_string()))?;
        if status.as_u16() == 429 {
            return Err(IgError::RateLimited);
        }
        let json: Value = match serde_json::from_str(&text) {
            Ok(v) => v,
            Err(_) => {
                if text.trim_start().starts_with('<') {
                    if text.contains("/accounts/login") && !text.contains("logged-in") {
                        return Err(IgError::LoginRequired);
                    }
                    return Err(IgError::Other(format!("{}: o Instagram respondeu HTML em vez de JSON (endpoint mudou ou sessao invalida, HTTP {})", what, status.as_u16())));
                }
                return Err(IgError::Other(format!(
                    "{}: resposta invalida (HTTP {})",
                    what,
                    status.as_u16()
                )));
            }
        };
        let message = json
            .get("message")
            .and_then(|m| m.as_str())
            .unwrap_or("")
            .to_string();
        let error_type = json
            .get("error_type")
            .and_then(|m| m.as_str())
            .unwrap_or("");
        if message == "login_required" || error_type == "login_required" {
            return Err(IgError::LoginRequired);
        }
        if message == "checkpoint_required"
            || message == "challenge_required"
            || json.get("checkpoint_url").is_some()
        {
            return Err(IgError::Checkpoint);
        }
        if message == "feedback_required" || json.get("feedback_title").is_some() {
            return Err(IgError::ActionBlocked);
        }
        if status.as_u16() == 404
            || message.contains("not found")
            || message == "Media not found or unavailable"
        {
            return Err(IgError::NotFound(what.to_string()));
        }
        if !status.is_success() {
            let m = if message.is_empty() {
                format!("HTTP {}", status.as_u16())
            } else {
                message
            };
            return Err(IgError::Other(format!("{}: {}", what, m)));
        }
        Ok(json)
    }

    /// GET `/api/v1/...` (ou URL absoluta) com os cabeçalhos da web.
    pub async fn get_json(&self, path: &str, query: &[(&str, String)]) -> Result<Value, IgError> {
        let url = if path.starts_with("http") {
            path.to_string()
        } else {
            format!("{}{}", BASE, path)
        };
        let resp = self
            .http
            .get(&url)
            .headers(self.api_headers())
            .query(query)
            .send()
            .await
            .map_err(|e| IgError::Other(format!("GET {}: {}", path, e)))?;
        self.finish(resp, path).await
    }

    /// POST de formulário para `/api/v1/...`.
    pub async fn post_form(&self, path: &str, form: &[(&str, String)]) -> Result<Value, IgError> {
        let url = if path.starts_with("http") {
            path.to_string()
        } else {
            format!("{}{}", BASE, path)
        };
        let resp = self
            .http
            .post(&url)
            .headers(self.post_headers())
            .form(form)
            .send()
            .await
            .map_err(|e| IgError::Other(format!("POST {}: {}", path, e)))?;
        self.finish(resp, path).await
    }

    /// POST cru (upload) com cabeçalhos extras.
    pub async fn post_raw(
        &self,
        url: &str,
        extra: HeaderMap,
        body: Vec<u8>,
    ) -> Result<Value, IgError> {
        let mut h = self.post_headers();
        h.extend(extra);
        let resp = self
            .http
            .post(url)
            .headers(h)
            .body(body)
            .send()
            .await
            .map_err(|e| IgError::Other(format!("upload: {}", e)))?;
        self.finish(resp, "upload").await
    }

    /// GET de uma página HTML (perfil, bundles) com cookies.
    pub async fn get_text(&self, url: &str) -> Result<String, IgError> {
        let resp = self
            .http
            .get(url)
            .header("Accept", "text/html,application/xhtml+xml,*/*;q=0.8")
            .header("Sec-Fetch-Dest", "document")
            .header("Sec-Fetch-Mode", "navigate")
            .send()
            .await
            .map_err(|e| IgError::Other(e.to_string()))?;
        self.absorb(&resp);
        if resp.status().is_redirection() {
            return Err(IgError::LoginRequired);
        }
        resp.text().await.map_err(|e| IgError::Other(e.to_string()))
    }

    /// Baixa um arquivo de CDN (sem cabeçalhos de API).
    pub async fn download(&self, url: &str, dest: &std::path::Path) -> anyhow::Result<u64> {
        use futures::StreamExt;
        use tokio::io::AsyncWriteExt;
        let resp = self
            .http
            .get(url)
            .header("Referer", "https://www.instagram.com/")
            .send()
            .await?;
        if !resp.status().is_success() {
            bail!("download falhou: HTTP {}", resp.status());
        }
        if let Some(p) = dest.parent() {
            std::fs::create_dir_all(p)?;
        }
        let part = dest.with_extension("part");
        let mut file = tokio::fs::File::create(&part).await?;
        let mut stream = resp.bytes_stream();
        let mut n = 0u64;
        while let Some(chunk) = stream.next().await {
            let chunk = chunk?;
            file.write_all(&chunk).await?;
            n += chunk.len() as u64;
        }
        file.flush().await?;
        drop(file);
        tokio::fs::rename(&part, dest).await?;
        Ok(n)
    }

    /// GraphQL por `doc_id` (POST `/graphql/query`), com as variáveis
    /// "provider" que a query exige; sem elas o servidor devolve
    /// "execution error".
    pub async fn graphql(
        &self,
        friendly_name: &str,
        mut variables: Value,
    ) -> Result<Value, IgError> {
        let ids = doc_ids(self).await;
        let doc_id = ids.ids.get(friendly_name).cloned().ok_or_else(|| {
            IgError::Other(format!(
                "nao achei o doc_id de {} nos bundles do Instagram",
                friendly_name
            ))
        })?;
        if let Some(obj) = variables.as_object_mut() {
            for p in ids
                .providers
                .get(friendly_name)
                .cloned()
                .unwrap_or_default()
            {
                obj.entry(format!("__relay_internal__pv__{}", p.replace('.', "")))
                    .or_insert(Value::Bool(false));
            }
        }
        let form = vec![
            ("fb_api_caller_class", "RelayModern".to_string()),
            ("fb_api_req_friendly_name", friendly_name.to_string()),
            ("variables", variables.to_string()),
            ("server_timestamps", "true".to_string()),
            ("doc_id", doc_id),
        ];
        let json = self.post_form("/graphql/query", &form).await?;
        if let Some(err) = json
            .get("errors")
            .and_then(|e| e.as_array())
            .and_then(|a| a.first())
        {
            let msg = err
                .get("message")
                .and_then(|m| m.as_str())
                .unwrap_or("erro");
            if msg.contains("execution error") {
                invalidate_doc_ids();
            }
            return Err(IgError::Other(format!(
                "GraphQL {}: {}",
                friendly_name, msg
            )));
        }
        Ok(json)
    }

    pub async fn pause(&self) {
        let (lo, hi) = self.page_delay_ms;
        sleep_jitter(lo, hi).await;
    }
}

pub async fn sleep_jitter(min_ms: u64, max_ms: u64) {
    let span = max_ms.saturating_sub(min_ms) + 1;
    let ms = min_ms + rand::random::<u64>() % span;
    tokio::time::sleep(Duration::from_millis(ms)).await;
}

// ── doc_ids do GraphQL ───────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DocIds {
    pub fetched_at: i64,
    pub ids: HashMap<String, String>,
    pub providers: HashMap<String, Vec<String>>,
}

fn doc_ids_path() -> Option<std::path::PathBuf> {
    super::tools_dir().map(|d| d.join("instagram").join("doc_ids.json"))
}

/// Valores vistos em 2026-09-05; servem de fallback quando a varredura dos
/// bundles não acha o módulo (ele pode estar num chunk carregado depois).
fn default_doc_ids() -> DocIds {
    let mut ids = HashMap::new();
    ids.insert(
        "PolarisProfilePostsQuery".into(),
        "38154989454116081".into(),
    );
    ids.insert(
        "PolarisProfilePostsTabContentQuery_connection".into(),
        "39535953862670189".into(),
    );
    ids.insert(
        "PolarisPostActionLoadPostQueryQuery".into(),
        "8845758582119845".into(),
    );
    let provs = vec![
        "PolarisMultiCaptionCarouselEnabled.relayprovider".to_string(),
        "PolarisShortDramaEnabled.relayprovider".to_string(),
        "PolarisReelsRecoDebugOverlayEnabled.relayprovider".to_string(),
    ];
    let mut providers = HashMap::new();
    providers.insert("PolarisProfilePostsQuery".into(), provs.clone());
    providers.insert(
        "PolarisProfilePostsTabContentQuery_connection".into(),
        provs,
    );
    DocIds {
        fetched_at: 0,
        ids,
        providers,
    }
}

static DOC_IDS: OnceLock<Mutex<Option<DocIds>>> = OnceLock::new();

fn invalidate_doc_ids() {
    *DOC_IDS.get_or_init(|| Mutex::new(None)).lock().unwrap() = None;
    if let Some(p) = doc_ids_path() {
        let _ = std::fs::remove_file(p);
    }
}

/// Lê o cache (12 h) ou varre os bundles JS da página de perfil procurando
/// `__d("<Nome>_instagramRelayOperation",[],(function(...){e.exports="<id>"`
/// e a lista de providers em `__d("<Nome>.graphql",[deps])`.
pub async fn doc_ids(client: &IgClient) -> DocIds {
    let cell = DOC_IDS.get_or_init(|| Mutex::new(None));
    if let Some(d) = cell.lock().unwrap().clone() {
        return d;
    }
    let now = chrono::Utc::now().timestamp();
    if let Some(p) = doc_ids_path() {
        if let Ok(text) = std::fs::read_to_string(&p) {
            if let Ok(d) = serde_json::from_str::<DocIds>(&text) {
                if now - d.fetched_at < 12 * 3600 && !d.ids.is_empty() {
                    *cell.lock().unwrap() = Some(d.clone());
                    return d;
                }
            }
        }
    }
    let mut found = default_doc_ids();
    found.fetched_at = now;
    if let Ok(html) = client.get_text(&format!("{}/instagram/", BASE)).await {
        let script_re = regex::Regex::new(
            r#"https://static\.cdninstagram\.com/rsrc\.php/[A-Za-z0-9_/.\-]+\.js"#,
        )
        .unwrap();
        let mut urls: Vec<String> = script_re
            .find_iter(&html)
            .map(|m| m.as_str().to_string())
            .collect();
        urls.dedup();
        let id_re = regex::Regex::new(r#"__d\("([A-Za-z0-9_]*Query[A-Za-z0-9_]*)_instagramRelayOperation"\s*,\s*\[\]\s*,\s*\(function\([^)]*\)\{[^}]{0,80}?exports\s*=\s*"?(\d{10,20})"?"#).unwrap();
        let dep_re = regex::Regex::new(
            r#"__d\("([A-Za-z0-9_]*Query[A-Za-z0-9_]*)\.graphql"\s*,\s*\[([^\]]*)\]"#,
        )
        .unwrap();
        let mut hits = 0;
        for u in urls.into_iter().take(40) {
            let Ok(text) = client.get_text(&u).await else {
                continue;
            };
            for c in id_re.captures_iter(&text) {
                if c[1].starts_with("Polaris") {
                    found.ids.insert(c[1].to_string(), c[2].to_string());
                    hits += 1;
                }
            }
            for c in dep_re.captures_iter(&text) {
                let provs: Vec<String> = c[2]
                    .split(',')
                    .map(|s| s.trim().trim_matches('"').to_string())
                    .filter(|s| s.to_lowercase().ends_with("relayprovider"))
                    .collect();
                if !provs.is_empty() {
                    let mut dedup = Vec::new();
                    for p in provs {
                        if !dedup.contains(&p) {
                            dedup.push(p);
                        }
                    }
                    found.providers.insert(c[1].to_string(), dedup);
                }
            }
        }
        tracing::debug!(
            "[instagram] doc_ids: {} operacoes encontradas nos bundles",
            hits
        );
    }
    if let Some(p) = doc_ids_path() {
        if let Some(parent) = p.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let _ = std::fs::write(&p, serde_json::to_string_pretty(&found).unwrap_or_default());
    }
    *cell.lock().unwrap() = Some(found.clone());
    found
}

// ── Shortcode ↔ id ───────────────────────────────────────────────────────

pub fn shortcode_to_pk(code: &str) -> Option<u64> {
    let code = if code.len() > 28 {
        &code[..code.len() - 28]
    } else {
        code
    };
    let mut n: u128 = 0;
    for b in code.bytes() {
        let i = ALPHABET.iter().position(|&a| a == b)? as u128;
        n = n * 64 + i;
    }
    u64::try_from(n).ok()
}

pub fn pk_to_shortcode(mut pk: u64) -> String {
    if pk == 0 {
        return "A".into();
    }
    let mut out = Vec::new();
    while pk > 0 {
        out.push(ALPHABET[(pk % 64) as usize]);
        pk /= 64;
    }
    out.reverse();
    String::from_utf8(out).unwrap_or_default()
}

// ── URLs ─────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum IgTarget {
    /// `/p/<code>`, `/reel/<code>`, `/tv/<code>`, `/<user>/p/<code>`
    Post {
        shortcode: String,
    },
    /// `/stories/<user>/<id?>`
    Story {
        username: String,
        media_id: Option<String>,
    },
    /// `/stories/highlights/<id>` ou `/s/<base64>`
    Highlight {
        id: String,
    },
    /// `/<user>/`, `/<user>/reels/`, `/<user>/tagged/`
    Profile {
        username: String,
        tab: String,
    },
    /// `/explore/tags/<tag>`
    Tag {
        name: String,
    },
    Unknown,
}

pub fn parse_target(input: &str) -> IgTarget {
    let s = input.trim();
    let s = s.strip_prefix('@').unwrap_or(s);
    if let Some(tag) = s.strip_prefix('#') {
        return IgTarget::Tag {
            name: tag.to_string(),
        };
    }
    let candidate: String = if s.contains("://") {
        s.to_string()
    } else if s.contains("instagram.com") {
        format!("https://{}", s)
    } else {
        s.to_string()
    };
    let Ok(url) = url::Url::parse(&candidate) else {
        // Só um nome de usuário ou um shortcode?
        if s.chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '_')
            && !s.is_empty()
        {
            return IgTarget::Profile {
                username: s.to_lowercase(),
                tab: "posts".into(),
            };
        }
        return IgTarget::Unknown;
    };
    if !url
        .host_str()
        .map(|h| h.ends_with("instagram.com"))
        .unwrap_or(false)
    {
        return IgTarget::Unknown;
    }
    let segs: Vec<&str> = url
        .path_segments()
        .map(|p| p.filter(|s| !s.is_empty()).collect())
        .unwrap_or_default();
    match segs.as_slice() {
        ["p" | "reel" | "reels" | "tv", code, ..] => IgTarget::Post {
            shortcode: code.to_string(),
        },
        [_, "p" | "reel" | "reels" | "tv", code, ..] => IgTarget::Post {
            shortcode: code.to_string(),
        },
        ["stories", "highlights", id, ..] => IgTarget::Highlight { id: id.to_string() },
        ["s", b64, ..] => {
            use base64::Engine;
            let decoded = base64::engine::general_purpose::STANDARD
                .decode(b64)
                .ok()
                .and_then(|b| String::from_utf8(b).ok())
                .unwrap_or_default();
            match decoded.strip_prefix("highlight:") {
                Some(id) => IgTarget::Highlight { id: id.to_string() },
                None => IgTarget::Unknown,
            }
        }
        ["stories", user, rest @ ..] => IgTarget::Story {
            username: user.to_lowercase(),
            media_id: rest.first().map(|s| s.to_string()),
        },
        ["explore", "tags", tag, ..] => IgTarget::Tag {
            name: tag.to_string(),
        },
        [user] => IgTarget::Profile {
            username: user.to_lowercase(),
            tab: "posts".into(),
        },
        [user, tab @ ("reels" | "tagged" | "saved" | "highlights" | "followers" | "following"), ..] => {
            IgTarget::Profile {
                username: user.to_lowercase(),
                tab: tab.to_string(),
            }
        }
        _ => IgTarget::Unknown,
    }
}

// ── Jobs canceláveis ─────────────────────────────────────────────────────

static JOBS: OnceLock<Mutex<HashMap<String, Arc<AtomicBool>>>> = OnceLock::new();

pub fn job_start(id: &str) -> Arc<AtomicBool> {
    let flag = Arc::new(AtomicBool::new(false));
    JOBS.get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .unwrap()
        .insert(id.to_string(), flag.clone());
    flag
}

pub fn job_finish(id: &str) {
    if let Some(m) = JOBS.get() {
        m.lock().unwrap().remove(id);
    }
}

pub fn job_cancel(id: &str) -> bool {
    if let Some(m) = JOBS.get() {
        if let Some(f) = m.lock().unwrap().get(id) {
            f.store(true, Ordering::Relaxed);
            return true;
        }
    }
    false
}

pub fn cancelled(flag: &AtomicBool) -> bool {
    flag.load(Ordering::Relaxed)
}

// ── Pasta de dados ───────────────────────────────────────────────────────

pub fn data_dir() -> std::path::PathBuf {
    let d = super::tools_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("instagram");
    let _ = std::fs::create_dir_all(&d);
    d
}

pub fn read_json<T: serde::de::DeserializeOwned + Default>(path: &std::path::Path) -> T {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

pub fn write_json<T: Serialize>(path: &std::path::Path, value: &T) -> anyhow::Result<()> {
    if let Some(p) = path.parent() {
        std::fs::create_dir_all(p)?;
    }
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_string_pretty(value)?)?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

// ── Utilidades de JSON ───────────────────────────────────────────────────

pub(crate) fn s(v: &Value, key: &str) -> String {
    match v.get(key) {
        Some(Value::String(x)) => x.clone(),
        Some(Value::Number(n)) => n.to_string(),
        _ => String::new(),
    }
}

pub(crate) fn u(v: &Value, key: &str) -> u64 {
    match v.get(key) {
        Some(Value::Number(n)) => n.as_u64().unwrap_or(0),
        Some(Value::String(x)) => x.parse().unwrap_or(0),
        _ => 0,
    }
}

pub(crate) fn b(v: &Value, key: &str) -> bool {
    v.get(key).and_then(|x| x.as_bool()).unwrap_or(false)
}

pub fn map_err(e: IgError) -> anyhow::Error {
    anyhow!(e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shortcode_roundtrip() {
        assert_eq!(shortcode_to_pk("B89prebFBcw"), Some(2251138696266127152));
        assert_eq!(pk_to_shortcode(2251138696266127152), "B89prebFBcw");
        assert_eq!(shortcode_to_pk("Dc30nJeRKKz"), Some(3978880184454783667));
    }

    #[test]
    fn parses_targets() {
        assert_eq!(
            parse_target("https://www.instagram.com/p/B89prebFBcw/?igsh=abc"),
            IgTarget::Post {
                shortcode: "B89prebFBcw".into()
            }
        );
        assert_eq!(
            parse_target("https://www.instagram.com/reel/Dc30nJeRKKz/"),
            IgTarget::Post {
                shortcode: "Dc30nJeRKKz".into()
            }
        );
        assert_eq!(
            parse_target("https://www.instagram.com/instagram/reel/Dc30nJeRKKz/"),
            IgTarget::Post {
                shortcode: "Dc30nJeRKKz".into()
            }
        );
        assert_eq!(
            parse_target("https://www.instagram.com/stories/highlights/18142207969557132/"),
            IgTarget::Highlight {
                id: "18142207969557132".into()
            }
        );
        assert_eq!(
            parse_target("https://www.instagram.com/stories/instagram/3811480328699137079/"),
            IgTarget::Story {
                username: "instagram".into(),
                media_id: Some("3811480328699137079".into())
            }
        );
        assert_eq!(
            parse_target("instagram.com/instagram/reels/"),
            IgTarget::Profile {
                username: "instagram".into(),
                tab: "reels".into()
            }
        );
        assert_eq!(
            parse_target("@Instagram"),
            IgTarget::Profile {
                username: "instagram".into(),
                tab: "posts".into()
            }
        );
        assert_eq!(
            parse_target("#sunset"),
            IgTarget::Tag {
                name: "sunset".into()
            }
        );
        assert_eq!(parse_target("https://youtube.com/x"), IgTarget::Unknown);
    }

    #[test]
    fn netscape_session() {
        let txt = "# Netscape HTTP Cookie File\n.instagram.com\tTRUE\t/\tTRUE\t0\tsessionid\tabc%3A1\n#HttpOnly_.instagram.com\tTRUE\t/\tTRUE\t0\tds_user_id\t123\n.instagram.com\tTRUE\t/\tTRUE\t0\tcsrftoken\tzzz\n";
        let s = Session::from_netscape(txt).unwrap();
        assert_eq!(s.user_id, "123");
        assert_eq!(s.csrf, "zzz");
        assert!(Session::from_netscape("").is_err());
    }
}

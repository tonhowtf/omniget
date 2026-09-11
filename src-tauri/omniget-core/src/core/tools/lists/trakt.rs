//! Trakt — API oficial, plano gratuito, sem raspar nada.
//!
//! O Trakt publica uma API REST de verdade e o plano gratuito já entrega
//! histórico de visualização, notas, watchlist e listas do usuário; VIP só
//! muda limite e estatística. A autenticação é o **device flow**: o app pede
//! um código, o usuário digita esse código em trakt.tv/activate e o app fica
//! consultando até o Trakt liberar o token.
//!
//! O `client_id`/`client_secret` são **do usuário**, criados por ele em
//! <https://trakt.tv/oauth/applications>. Nada de segredo embutido no
//! binário: as credenciais e o token ficam no mesmo cofre local que as
//! chaves de IA usam (`<app_data>/tools/`), e a UI só recebe o começo e o
//! fim de cada uma.

use std::sync::Mutex;
use std::time::{Duration, Instant};

use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{iso_date, Entry};
use crate::core::tools::{report, ProgressFn};

const TOOL_ID: &str = "trakt-export";
pub const API: &str = "https://api.trakt.tv";
/// O Trakt separou os hosts: tudo que é `/oauth/*` vai para o de
/// autenticação, e só o resto continua no da API.
pub const AUTH: &str = "https://auth.trakt.tv";
/// Onde o usuário cria a app dele e pega client_id/client_secret.
pub const APP_URL: &str = "https://app.trakt.tv/settings/apps/api/new";

// ── Cofre local ─────────────────────────────────────────────────────────

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Creds {
    #[serde(default)]
    pub client_id: String,
    #[serde(default)]
    pub client_secret: String,
    #[serde(default)]
    pub access_token: String,
    #[serde(default)]
    pub refresh_token: String,
    /// Unix em segundos; 0 = desconhecido.
    #[serde(default)]
    pub expires_at: i64,
    #[serde(default)]
    pub username: String,
}

/// O que a UI vê: nunca a credencial inteira.
#[derive(Debug, Clone, Serialize)]
pub struct CredsView {
    pub client_id_hint: String,
    pub has_client_id: bool,
    pub has_client_secret: bool,
    pub connected: bool,
    pub username: String,
    pub expires_at: i64,
    pub app_url: &'static str,
}

impl Creds {
    pub fn view(&self) -> CredsView {
        CredsView {
            client_id_hint: crate::core::tools::ai_keys::hint(&self.client_id),
            has_client_id: !self.client_id.is_empty(),
            has_client_secret: !self.client_secret.is_empty(),
            connected: !self.access_token.is_empty(),
            username: self.username.clone(),
            expires_at: self.expires_at,
            app_url: APP_URL,
        }
    }
}

static LOCK: Mutex<()> = Mutex::new(());

fn file() -> Option<std::path::PathBuf> {
    crate::core::tools::tools_dir().map(|d| d.join("lists-trakt.json"))
}

fn load() -> Creds {
    file()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

fn store(c: &Creds) -> Result<()> {
    let p = file().ok_or_else(|| anyhow!("sem pasta de dados"))?;
    if let Some(parent) = p.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = p.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_string_pretty(c)?)?;
    std::fs::rename(&tmp, &p)?;
    Ok(())
}

pub fn creds_view() -> CredsView {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    load().view()
}

/// Grava as credenciais da app do usuário. Campo em branco mantém o que já
/// estava guardado (a UI nunca recebe o segredo de volta para reenviar).
pub fn save_app(client_id: &str, client_secret: &str) -> Result<CredsView> {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut c = load();
    let id = client_id.trim();
    let secret = client_secret.trim();
    if !id.is_empty() && id != c.client_id {
        // Trocou de app: o token antigo não vale mais.
        c.access_token.clear();
        c.refresh_token.clear();
        c.expires_at = 0;
        c.username.clear();
    }
    if !id.is_empty() {
        c.client_id = id.to_string();
    }
    if !secret.is_empty() {
        c.client_secret = secret.to_string();
    }
    if c.client_id.is_empty() {
        return Err(anyhow!("informe o client_id da sua app do Trakt"));
    }
    store(&c)?;
    Ok(c.view())
}

pub fn disconnect() -> Result<CredsView> {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut c = load();
    c.access_token.clear();
    c.refresh_token.clear();
    c.expires_at = 0;
    c.username.clear();
    store(&c)?;
    Ok(c.view())
}

/// Apaga tudo, inclusive as credenciais da app.
pub fn forget() -> Result<CredsView> {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let c = Creds::default();
    store(&c)?;
    Ok(c.view())
}

// ── Device flow ─────────────────────────────────────────────────────────

/// O que a UI mostra enquanto o usuário libera o acesso. Volta para o app no
/// passo 2, então precisa saber ir e voltar.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceCode {
    pub device_code: String,
    /// O código curto que o usuário digita no site.
    pub user_code: String,
    pub verification_url: String,
    /// Segundos até o código morrer.
    pub expires_in: u64,
    /// Segundos entre uma consulta e outra, como o Trakt pede.
    pub interval: u64,
}

/// Cada resposta possível do polling do device flow. É uma máquina de
/// estados pequena, mas errar um código aqui significa martelar o servidor
/// ou desistir cedo demais — por isso ela é uma função pura, testada.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Poll {
    /// 200: token liberado.
    Done,
    /// 400: o usuário ainda não digitou o código. Continue esperando.
    Pending,
    /// 429: consultando rápido demais; aumente o intervalo.
    SlowDown,
    /// 404: `device_code` inválido — recomeçar.
    NotFound,
    /// 409: esse código já virou token.
    Used,
    /// 410: expirou antes de o usuário aprovar.
    Expired,
    /// 418: o usuário negou.
    Denied,
    Http(u16),
}

pub fn poll_state(status: u16) -> Poll {
    match status {
        200 => Poll::Done,
        400 => Poll::Pending,
        404 => Poll::NotFound,
        409 => Poll::Used,
        410 => Poll::Expired,
        418 => Poll::Denied,
        429 => Poll::SlowDown,
        other => Poll::Http(other),
    }
}

impl Poll {
    /// Se vale a pena consultar de novo.
    pub fn keep_waiting(&self) -> bool {
        matches!(self, Poll::Pending | Poll::SlowDown)
    }

    pub fn message(&self) -> &'static str {
        match self {
            Poll::Done => "conectado",
            Poll::Pending => "esperando você liberar o acesso no site",
            Poll::SlowDown => "o Trakt pediu para consultar mais devagar",
            Poll::NotFound => "código inválido: comece de novo",
            Poll::Used => "esse código já foi usado",
            Poll::Expired => "o código expirou antes de você liberar",
            Poll::Denied => "acesso negado no site do Trakt",
            Poll::Http(_) => "o Trakt respondeu algo inesperado",
        }
    }
}

fn client() -> Result<reqwest::Client> {
    crate::core::tools::client()
}

/// Passo 1: pede o código que o usuário vai digitar em trakt.tv/activate.
pub async fn device_code() -> Result<DeviceCode> {
    let c = {
        let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        load()
    };
    if c.client_id.is_empty() {
        return Err(anyhow!(
            "informe primeiro o client_id da sua app do Trakt (crie em {})",
            APP_URL
        ));
    }
    let r = client()?
        .post(format!("{}/oauth/device/code", AUTH))
        .json(&serde_json::json!({ "client_id": c.client_id }))
        .send()
        .await?;
    if !r.status().is_success() {
        return Err(anyhow!(
            "o Trakt recusou o pedido de código (HTTP {}) — confira o client_id",
            r.status()
        ));
    }
    let v: Value = r.json().await?;
    parse_device_code(&v)
}

pub fn parse_device_code(v: &Value) -> Result<DeviceCode> {
    let s = |k: &str| {
        v.get(k)
            .and_then(|x| x.as_str())
            .unwrap_or_default()
            .to_string()
    };
    let n = |k: &str, d: u64| v.get(k).and_then(|x| x.as_u64()).unwrap_or(d);
    let device = s("device_code");
    let user = s("user_code");
    if device.is_empty() || user.is_empty() {
        return Err(anyhow!("o Trakt não devolveu o código do dispositivo"));
    }
    let mut url = s("verification_url");
    if url.is_empty() {
        url = "https://trakt.tv/activate".to_string();
    }
    Ok(DeviceCode {
        device_code: device,
        user_code: user,
        verification_url: url,
        expires_in: n("expires_in", 600),
        // O Trakt pede 5 s; menos que isso leva 429.
        interval: n("interval", 5).clamp(1, 60),
    })
}

#[derive(Debug, Clone, Serialize)]
pub struct ConnectResult {
    pub connected: bool,
    pub username: String,
    pub message: String,
}

/// Passo 2: consulta até o usuário liberar (ou o código morrer). Respeita o
/// intervalo pedido e recua quando leva 429.
pub async fn wait_for_token(dc: &DeviceCode, p: ProgressFn) -> Result<ConnectResult> {
    let c = {
        let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        load()
    };
    if c.client_secret.is_empty() {
        return Err(anyhow!(
            "o device flow precisa do client_secret da sua app do Trakt"
        ));
    }
    let http = client()?;
    let started = Instant::now();
    let deadline = Duration::from_secs(dc.expires_in.clamp(60, 1800));
    let mut interval = Duration::from_secs(dc.interval);
    loop {
        if started.elapsed() >= deadline {
            return Err(anyhow!("{}", Poll::Expired.message()));
        }
        tokio::time::sleep(interval).await;
        let left = deadline.saturating_sub(started.elapsed()).as_secs();
        report(
            &p,
            TOOL_ID,
            "progress",
            started.elapsed().as_secs(),
            Some(deadline.as_secs()),
            Some(format!("{} ({}s)", Poll::Pending.message(), left)),
        );
        let r = http
            .post(format!("{}/oauth/device/token", AUTH))
            .json(&serde_json::json!({
                "code": dc.device_code,
                "client_id": c.client_id,
                "client_secret": c.client_secret,
            }))
            .send()
            .await?;
        let status = r.status().as_u16();
        match poll_state(status) {
            Poll::Done => {
                let v: Value = r.json().await?;
                let username = save_token(&v)?;
                report(&p, TOOL_ID, "done", 1, Some(1), None);
                return Ok(ConnectResult {
                    connected: true,
                    username: username.clone(),
                    message: format!("conectado como {}", username),
                });
            }
            Poll::SlowDown => interval += Duration::from_secs(dc.interval.max(1)),
            Poll::Pending => {}
            other => return Err(anyhow!("{}", other.message())),
        }
    }
}

fn save_token(v: &Value) -> Result<String> {
    let access = v
        .get("access_token")
        .and_then(|x| x.as_str())
        .unwrap_or_default()
        .to_string();
    if access.is_empty() {
        return Err(anyhow!("o Trakt não devolveu o token"));
    }
    let refresh = v
        .get("refresh_token")
        .and_then(|x| x.as_str())
        .unwrap_or_default()
        .to_string();
    let created = v.get("created_at").and_then(|x| x.as_i64()).unwrap_or(0);
    let expires = v.get("expires_in").and_then(|x| x.as_i64()).unwrap_or(0);
    let base = if created > 0 {
        created
    } else {
        chrono::Utc::now().timestamp()
    };
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut c = load();
    c.access_token = access;
    c.refresh_token = refresh;
    c.expires_at = base + expires;
    store(&c)?;
    Ok(c.username)
}

/// Renova o token quando falta menos de um dia, sem incomodar o usuário. O
/// token do Trakt dura de 1 a 7 dias conforme a app, então um dia de
/// antecedência cobre os dois casos.
pub fn needs_refresh(expires_at: i64, now: i64) -> bool {
    expires_at > 0 && expires_at - now < 86_400
}

async fn refresh_if_needed(c: &mut Creds) -> Result<()> {
    if !needs_refresh(c.expires_at, chrono::Utc::now().timestamp()) || c.refresh_token.is_empty() {
        return Ok(());
    }
    let r = client()?
        .post(format!("{}/oauth/token", AUTH))
        .json(&serde_json::json!({
            "refresh_token": c.refresh_token,
            "client_id": c.client_id,
            "client_secret": c.client_secret,
            "redirect_uri": "urn:ietf:wg:oauth:2.0:oob",
            "grant_type": "refresh_token",
        }))
        .send()
        .await?;
    if !r.status().is_success() {
        let status = r.status().as_u16();
        let body = r.text().await.unwrap_or_default();
        // O Trakt passou a invalidar o refresh token a cada uso, e a sessão
        // antiga morre de vez: quando ele diz isso, não adianta insistir.
        if status == 400 && body.contains("invalid_grant") {
            let _ = disconnect();
            return Err(anyhow!(
                "a autorização do Trakt expirou de vez. Conecte a conta de novo (o código aparece aqui e você libera em trakt.tv/activate)"
            ));
        }
        // Fora isso não é fatal: o token atual ainda pode valer.
        tracing::debug!("trakt: refresh recusado (HTTP {})", status);
        return Ok(());
    }
    let v: Value = r.json().await?;
    save_token(&v)?;
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    *c = load();
    Ok(())
}

// ── Leitura da API ──────────────────────────────────────────────────────

/// Cliente autenticado com freio, no mesmo desenho do Reddit.
struct Api {
    http: reqwest::Client,
    client_id: String,
    token: String,
    delay: Duration,
    last: tokio::sync::Mutex<Option<Instant>>,
    requests: std::sync::atomic::AtomicU32,
}

impl Api {
    fn new(c: &Creds, delay_ms: u64) -> Result<Self> {
        Ok(Self {
            http: client()?,
            client_id: c.client_id.clone(),
            token: c.access_token.clone(),
            delay: Duration::from_millis(delay_ms.clamp(200, 10_000)),
            last: tokio::sync::Mutex::new(None),
            requests: std::sync::atomic::AtomicU32::new(0),
        })
    }

    async fn pace(&self) {
        let mut last = self.last.lock().await;
        if let Some(t) = *last {
            let since = t.elapsed();
            if since < self.delay {
                tokio::time::sleep(self.delay - since).await;
            }
        }
        *last = Some(Instant::now());
    }

    /// GET de uma página. Devolve o corpo e quantas páginas existem.
    async fn get_page(&self, path: &str, page: u32, limit: u32) -> Result<(Value, u32)> {
        let url = format!("{}{}", API, path);
        let sep = if path.contains('?') { '&' } else { '?' };
        let url = format!("{}{}page={}&limit={}", url, sep, page, limit);
        let mut wait = Duration::from_secs(3);
        for attempt in 1..=4u32 {
            self.pace().await;
            self.requests
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let r = self
                .http
                .get(&url)
                .header("Content-Type", "application/json")
                .header("trakt-api-version", "2")
                .header("trakt-api-key", &self.client_id)
                .header(
                    reqwest::header::AUTHORIZATION,
                    format!("Bearer {}", self.token),
                )
                .send()
                .await?;
            let status = r.status();
            if status.as_u16() == 401 {
                return Err(anyhow!(
                    "o Trakt recusou o token (401). Reconecte a conta no botão acima"
                ));
            }
            if status.as_u16() == 426 {
                return Err(anyhow!("esse recurso é só para contas VIP do Trakt"));
            }
            if status.as_u16() == 420 {
                return Err(anyhow!(
                    "a conta do Trakt bateu no limite do plano gratuito (HTTP 420)"
                ));
            }
            if status.as_u16() == 423 {
                return Err(anyhow!(
                    "a conta do Trakt está bloqueada; fale com o suporte deles"
                ));
            }
            if status.as_u16() == 429 || status.is_server_error() {
                if attempt == 4 {
                    return Err(anyhow!("o Trakt está limitando o acesso (HTTP {})", status));
                }
                let retry = r
                    .headers()
                    .get(reqwest::header::RETRY_AFTER)
                    .and_then(|v| v.to_str().ok())
                    .and_then(|v| v.trim().parse::<u64>().ok())
                    .map(Duration::from_secs);
                tokio::time::sleep(retry.unwrap_or(wait)).await;
                wait *= 2;
                continue;
            }
            if !status.is_success() {
                return Err(anyhow!("HTTP {} em {}", status, url));
            }
            let pages = r
                .headers()
                .get("x-pagination-page-count")
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.trim().parse::<u32>().ok())
                .unwrap_or(1);
            let v: Value = r.json().await?;
            return Ok((v, pages.max(1)));
        }
        Err(anyhow!("não consegui ler {}", url))
    }

    /// Percorre todas as páginas até o teto pedido.
    async fn get_all(
        &self,
        path: &str,
        max_pages: u32,
        p: &ProgressFn,
        what: &str,
    ) -> Result<Vec<Value>> {
        let mut out = Vec::new();
        let mut page = 1u32;
        loop {
            let (v, pages) = self.get_page(path, page, 100).await?;
            let total = pages.min(max_pages.max(1));
            report(
                p,
                TOOL_ID,
                "progress",
                page as u64,
                Some(total as u64),
                Some(format!("{} ({}/{})", what, page, total)),
            );
            match v {
                Value::Array(items) => {
                    let n = items.len();
                    out.extend(items);
                    if n == 0 {
                        break;
                    }
                }
                other => {
                    out.push(other);
                    break;
                }
            }
            if page >= pages || page >= max_pages {
                break;
            }
            page += 1;
        }
        Ok(out)
    }
}

fn ids_of(v: &Value) -> (Option<u64>, String, String) {
    let ids = v.get("ids");
    let tmdb = ids.and_then(|i| i.get("tmdb")).and_then(|x| x.as_u64());
    let imdb = ids
        .and_then(|i| i.get("imdb"))
        .and_then(|x| x.as_str())
        .unwrap_or_default()
        .to_string();
    let slug = ids
        .and_then(|i| i.get("slug"))
        .and_then(|x| x.as_str())
        .unwrap_or_default()
        .to_string();
    (tmdb, imdb, slug)
}

/// Um item de qualquer endpoint de sincronismo do Trakt (`/sync/history`,
/// `/sync/ratings`, `/sync/watchlist`, itens de lista) vira uma linha. O
/// `type` diz onde procurar o título: `movie`, `show`, `episode`, `season`.
pub fn parse_item(v: &Value, list: &str) -> Option<Entry> {
    let kind = v.get("type").and_then(|x| x.as_str()).unwrap_or("movie");
    let date = ["watched_at", "rated_at", "listed_at", "last_watched_at"]
        .iter()
        .find_map(|k| v.get(*k).and_then(|x| x.as_str()))
        .map(iso_date)
        .unwrap_or_default();
    let rating = v.get("rating").and_then(|x| x.as_f64()).map(|r| r as f32);

    let (media, title, kind_pt, url) = match kind {
        "movie" => {
            let m = v.get("movie")?;
            let t = m.get("title").and_then(|x| x.as_str())?.to_string();
            let (_, _, slug) = ids_of(m);
            (m, t, "filme", format!("https://trakt.tv/movies/{}", slug))
        }
        "episode" => {
            let show = v.get("show")?;
            let ep = v.get("episode")?;
            let st = show.get("title").and_then(|x| x.as_str())?.to_string();
            let s = ep.get("season").and_then(|x| x.as_u64()).unwrap_or(0);
            let n = ep.get("number").and_then(|x| x.as_u64()).unwrap_or(0);
            let et = ep.get("title").and_then(|x| x.as_str()).unwrap_or_default();
            let (_, _, slug) = ids_of(show);
            let title = if et.is_empty() {
                format!("{} S{:02}E{:02}", st, s, n)
            } else {
                format!("{} S{:02}E{:02} — {}", st, s, n, et)
            };
            (
                show,
                title,
                "serie",
                format!(
                    "https://trakt.tv/shows/{}/seasons/{}/episodes/{}",
                    slug, s, n
                ),
            )
        }
        "show" | "season" => {
            let show = v.get("show")?;
            let t = show.get("title").and_then(|x| x.as_str())?.to_string();
            let (_, _, slug) = ids_of(show);
            (show, t, "serie", format!("https://trakt.tv/shows/{}", slug))
        }
        _ => return None,
    };

    let (tmdb, imdb, _) = ids_of(media);
    let mut e = Entry::new(kind_pt, "trakt", &title);
    e.date = date;
    e.year = media
        .get("year")
        .and_then(|x| x.as_i64())
        .map(|y| y as i32)
        .filter(|y| *y > 1200);
    e.rating = rating.filter(|r| *r > 0.0);
    e.url = url;
    e.list = list.to_string();
    e.tmdb_id = tmdb;
    e.imdb_id = imdb;
    Some(e)
}

pub fn parse_items(v: &[Value], list: &str) -> Vec<Entry> {
    v.iter().filter_map(|i| parse_item(i, list)).collect()
}

/// As listas pessoais: `/users/me/lists` devolve nome e id de cada uma.
pub fn parse_lists(v: &[Value]) -> Vec<(u64, String)> {
    v.iter()
        .filter_map(|l| {
            let name = l.get("name").and_then(|x| x.as_str())?.to_string();
            let id = l
                .get("ids")
                .and_then(|i| i.get("trakt"))
                .and_then(|x| x.as_u64())?;
            Some((id, name))
        })
        .collect()
}

// ── Execução ────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Deserialize)]
pub struct Options {
    pub dest: String,
    /// `history`, `ratings`, `watchlist`, `lists`. Vazio = tudo.
    #[serde(default)]
    pub parts: Vec<String>,
    #[serde(default)]
    pub formats: Vec<String>,
    /// Teto de páginas por endpoint (100 itens por página).
    #[serde(default = "default_pages")]
    pub max_pages: u32,
    #[serde(default = "default_delay")]
    pub delay_ms: u64,
}

fn default_pages() -> u32 {
    50
}
/// O teto do Trakt é 500 GET a cada 5 minutos (1,67/s). 800 ms deixa folga.
fn default_delay() -> u64 {
    800
}

#[derive(Debug, Clone, Serialize)]
pub struct ExportResult {
    pub used_session: bool,
    pub username: String,
    pub entries: usize,
    pub by_part: Vec<super::letterboxd::PartCount>,
    pub requests: u32,
    pub files: Vec<String>,
    pub dest: String,
    pub sample: Vec<Entry>,
}

pub async fn run(opts: &Options, p: ProgressFn) -> Result<ExportResult> {
    if opts.dest.trim().is_empty() {
        return Err(anyhow!("escolha a pasta de destino"));
    }
    let mut c = {
        let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        load()
    };
    if c.access_token.is_empty() {
        return Err(anyhow!(
            "conecte a conta do Trakt primeiro (o código aparece aqui e você libera em trakt.tv/activate)"
        ));
    }
    refresh_if_needed(&mut c).await?;
    let api = Api::new(&c, opts.delay_ms)?;
    let want =
        |x: &str| opts.parts.is_empty() || opts.parts.iter().any(|s| s.eq_ignore_ascii_case(x));

    // Quem é o usuário — serve de teste de token e nomeia o arquivo.
    let mut username = c.username.clone();
    if let Ok((v, _)) = api.get_page("/users/settings", 1, 1).await {
        if let Some(u) = v
            .get("user")
            .and_then(|u| u.get("username"))
            .and_then(|x| x.as_str())
        {
            username = u.to_string();
            let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
            let mut saved = load();
            saved.username = username.clone();
            let _ = store(&saved);
        }
    }

    let mut by_part = Vec::new();
    let mut all: Vec<Entry> = Vec::new();
    let push = |label: &str,
                entries: Vec<Entry>,
                by: &mut Vec<super::letterboxd::PartCount>,
                all: &mut Vec<Entry>| {
        by.push(super::letterboxd::PartCount {
            part: label.to_string(),
            entries: entries.len(),
        });
        all.extend(entries);
    };

    if want("history") {
        let v = api
            .get_all("/sync/history", opts.max_pages, &p, "histórico")
            .await?;
        push(
            "historico",
            parse_items(&v, "historico"),
            &mut by_part,
            &mut all,
        );
    }
    if want("ratings") {
        let v = api
            .get_all("/sync/ratings", opts.max_pages, &p, "notas")
            .await?;
        push("notas", parse_items(&v, "notas"), &mut by_part, &mut all);
    }
    if want("watchlist") {
        let v = api
            .get_all("/sync/watchlist", opts.max_pages, &p, "watchlist")
            .await?;
        push(
            "watchlist",
            parse_items(&v, "watchlist"),
            &mut by_part,
            &mut all,
        );
    }
    if want("lists") {
        let raw = api.get_all("/users/me/lists", 1, &p, "listas").await?;
        for (id, name) in parse_lists(&raw) {
            let path = format!("/users/me/lists/{}/items", id);
            let items = api.get_all(&path, opts.max_pages, &p, &name).await?;
            let label = format!("lista: {}", name);
            push(&label, parse_items(&items, &label), &mut by_part, &mut all);
        }
    }
    if all.is_empty() {
        return Err(anyhow!("a conta não devolveu nenhuma linha"));
    }
    all.sort_by(|a, b| b.date.cmp(&a.date).then(a.title.cmp(&b.title)));

    let dest = std::path::PathBuf::from(opts.dest.trim());
    let formats = if opts.formats.is_empty() {
        vec!["json".to_string(), "csv".to_string()]
    } else {
        opts.formats.clone()
    };
    let files = super::write_exports(&dest, "trakt", &all, &formats, |es| {
        super::merge::timeline_markdown("Trakt", es)
    })?;
    report(&p, TOOL_ID, "done", 1, Some(1), None);
    Ok(ExportResult {
        used_session: true,
        username,
        entries: all.len(),
        by_part,
        requests: api.requests.load(std::sync::atomic::Ordering::Relaxed),
        files,
        dest: dest.to_string_lossy().to_string(),
        sample: all.iter().take(40).cloned().collect(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn maquina_de_estados_do_device_flow() {
        assert_eq!(poll_state(200), Poll::Done);
        assert_eq!(poll_state(400), Poll::Pending);
        assert_eq!(poll_state(404), Poll::NotFound);
        assert_eq!(poll_state(409), Poll::Used);
        assert_eq!(poll_state(410), Poll::Expired);
        assert_eq!(poll_state(418), Poll::Denied);
        assert_eq!(poll_state(429), Poll::SlowDown);
        assert_eq!(poll_state(500), Poll::Http(500));
    }

    #[test]
    fn so_pendente_e_devagar_continuam_esperando() {
        assert!(poll_state(400).keep_waiting());
        assert!(poll_state(429).keep_waiting());
        for s in [200u16, 404, 409, 410, 418, 500] {
            assert!(!poll_state(s).keep_waiting(), "status {}", s);
        }
        assert!(!Poll::Denied.message().is_empty());
    }

    #[test]
    fn codigo_do_dispositivo_tem_padrao_quando_falta_campo() {
        let v = json!({
            "device_code": "d4c2f8...", "user_code": "5055CC",
            "verification_url": "https://trakt.tv/activate",
            "expires_in": 600, "interval": 5
        });
        let dc = parse_device_code(&v).unwrap_or_else(|_| unreachable!());
        assert_eq!(dc.user_code, "5055CC");
        assert_eq!(dc.interval, 5);
        let magro = json!({ "device_code": "x", "user_code": "ABC" });
        let dc = parse_device_code(&magro).unwrap_or_else(|_| unreachable!());
        assert_eq!(dc.verification_url, "https://trakt.tv/activate");
        assert_eq!(dc.expires_in, 600);
        assert_eq!(dc.interval, 5);
        assert!(parse_device_code(&json!({})).is_err());
    }

    #[test]
    fn filme_do_historico_vira_linha() {
        let v = json!({
            "id": 1982, "watched_at": "2014-03-31T09:28:53.000Z",
            "action": "scrobble", "type": "movie",
            "movie": { "title": "The Dark Knight", "year": 2008,
                "ids": { "trakt": 4, "slug": "the-dark-knight-2008", "imdb": "tt0468569", "tmdb": 155 } }
        });
        let e = parse_item(&v, "historico").unwrap_or_else(|| unreachable!());
        assert_eq!(e.kind, "filme");
        assert_eq!(e.title, "The Dark Knight");
        assert_eq!(e.year, Some(2008));
        assert_eq!(e.date, "2014-03-31");
        assert_eq!(e.tmdb_id, Some(155));
        assert_eq!(e.imdb_id, "tt0468569");
        assert_eq!(e.url, "https://trakt.tv/movies/the-dark-knight-2008");
        assert_eq!(e.source, "trakt");
    }

    #[test]
    fn episodio_junta_serie_temporada_e_numero() {
        let v = json!({
            "watched_at": "2021-05-13T14:33:00.000Z", "type": "episode",
            "episode": { "season": 1, "number": 2, "title": "Cat's in the Bag...",
                "ids": { "trakt": 74, "tmdb": 62086 } },
            "show": { "title": "Breaking Bad", "year": 2008,
                "ids": { "trakt": 1, "slug": "breaking-bad", "imdb": "tt0903747", "tmdb": 1396 } }
        });
        let e = parse_item(&v, "historico").unwrap_or_else(|| unreachable!());
        assert_eq!(e.kind, "serie");
        assert_eq!(e.title, "Breaking Bad S01E02 — Cat's in the Bag...");
        // O id é o da série, não o do episódio: é assim que casa com o resto.
        assert_eq!(e.tmdb_id, Some(1396));
        assert_eq!(e.year, Some(2008));
        assert!(e.url.contains("/shows/breaking-bad/seasons/1/episodes/2"));
    }

    #[test]
    fn nota_e_watchlist_leem_a_data_do_campo_certo() {
        let nota = json!({
            "rated_at": "2014-09-01T09:10:11.000Z", "rating": 9, "type": "movie",
            "movie": { "title": "TRON: Legacy", "year": 2010, "ids": { "slug": "tron-legacy-2010" } }
        });
        let e = parse_item(&nota, "notas").unwrap_or_else(|| unreachable!());
        assert_eq!(e.date, "2014-09-01");
        assert_eq!(e.rating, Some(9.0));
        let wl = json!({
            "rank": 1, "listed_at": "2014-09-01T09:10:11.000Z", "type": "show",
            "show": { "title": "Utopia", "year": 2013, "ids": { "slug": "utopia" } }
        });
        let e = parse_item(&wl, "watchlist").unwrap_or_else(|| unreachable!());
        assert_eq!(e.kind, "serie");
        assert_eq!(e.list, "watchlist");
        assert_eq!(e.rating, None);
    }

    #[test]
    fn item_sem_titulo_ou_de_tipo_estranho_e_ignorado() {
        assert!(parse_item(&json!({ "type": "movie" }), "x").is_none());
        assert!(parse_item(&json!({ "type": "person", "person": {} }), "x").is_none());
        let itens = vec![
            json!({ "type": "movie" }),
            json!({
                "type": "movie", "movie": { "title": "Duna", "year": 2021, "ids": { "slug": "dune-2021" } }
            }),
        ];
        assert_eq!(parse_items(&itens, "x").len(), 1);
    }

    #[test]
    fn listas_pessoais_saem_com_id_e_nome() {
        let v = vec![
            json!({ "name": "Star Wars in machete order", "ids": { "trakt": 55, "slug": "swmo" } }),
            json!({ "name": "sem id" }),
        ];
        let l = parse_lists(&v);
        assert_eq!(l.len(), 1);
        assert_eq!(l[0], (55, "Star Wars in machete order".to_string()));
    }

    #[test]
    fn token_so_renova_perto_do_vencimento() {
        let agora = 1_700_000_000i64;
        assert!(needs_refresh(agora + 3_600, agora));
        assert!(!needs_refresh(agora + 90_000, agora));
        assert!(!needs_refresh(0, agora));
    }

    #[test]
    fn credencial_nunca_sai_inteira_para_a_ui() {
        let c = Creds {
            client_id: "abcdef0123456789".into(),
            client_secret: "segredo-muito-secreto".into(),
            access_token: "tok".into(),
            username: "fulano".into(),
            ..Default::default()
        };
        let v = c.view();
        assert!(v.has_client_id && v.has_client_secret && v.connected);
        assert!(!v.client_id_hint.contains("0123456"));
        let json = serde_json::to_string(&v).unwrap_or_default();
        assert!(!json.contains("segredo-muito-secreto"));
        assert!(!json.contains("abcdef0123456789"));
    }

    #[test]
    #[ignore = "rede + app do usuário: roda o device flow inteiro contra api.trakt.tv"]
    fn device_flow_de_verdade() {
        // Precisa de client_id/client_secret reais e de alguém digitando o
        // código em trakt.tv/activate; o CI não tem nem um nem outro.
    }
}

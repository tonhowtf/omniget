//! Listas e diários de consumo (categoria `lists`). A regra da casa aqui é
//! uma só: quando a plataforma já tem um export oficial, a ferramenta aperta
//! o botão que já existe em vez de raspar página. Sobra scraping só para o
//! que o export não cobre (ID do TMDB, texto de review por prateleira).
//!
//! - `letterboxd.rs`: baixa o ZIP de `/data/export/` com a sessão do usuário
//!   e normaliza diário, notas, reviews, watchlist e listas.
//! - `trakt.rs`: API oficial com device flow de OAuth; histórico, notas,
//!   watchlist e listas no plano gratuito.
//! - `goodreads.rs`: dispara o "Export Library" oficial, espera o CSV ficar
//!   pronto e enriquece por prateleira.
//! - `merge.rs`: junta os três (mais o export do Spotify) numa linha do
//!   tempo única de leu/assistiu/ouviu.

pub mod goodreads;
pub mod letterboxd;
pub mod merge;
pub mod trakt;

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};

/// Leitor de CSV do export do Reddit: mesmo dialeto (RFC 4180 com aspas
/// duplicadas), então não vale reescrever.
pub use crate::core::tools::reddit::gdpr::parse_csv;

// ── Esquema comum ───────────────────────────────────────────────────────

/// Uma linha da linha do tempo, venha de que serviço vier. É o formato que
/// `merge.rs` consome e que cada módulo produz.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Entry {
    /// `YYYY-MM-DD` quando o serviço dá a data; vazio quando não dá.
    #[serde(default)]
    pub date: String,
    /// `filme` | `serie` | `livro` | `musica`.
    pub kind: String,
    pub title: String,
    #[serde(default)]
    pub year: Option<i32>,
    /// Diretor, autor ou artista — o que a origem souber dizer.
    #[serde(default)]
    pub creator: String,
    /// Já na escala de 0 a 10 (Letterboxd 0–5 vira 0–10, Goodreads 0–5 idem).
    #[serde(default)]
    pub rating: Option<f32>,
    #[serde(default)]
    pub review: String,
    /// `letterboxd` | `trakt` | `goodreads` | `spotify`.
    pub source: String,
    #[serde(default)]
    pub url: String,
    /// Que lista/prateleira/atividade gerou a linha (`diary`, `watchlist`,
    /// `read`, `lista: Favoritos`…).
    #[serde(default)]
    pub list: String,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub tmdb_id: Option<u64>,
    #[serde(default)]
    pub imdb_id: String,
    #[serde(default)]
    pub isbn: String,
    /// Quantas vezes a mesma coisa apareceu depois da deduplicação.
    #[serde(default)]
    pub times: u32,
}

impl Entry {
    pub fn new(kind: &str, source: &str, title: &str) -> Self {
        Self {
            kind: kind.to_string(),
            source: source.to_string(),
            title: title.to_string(),
            times: 1,
            ..Default::default()
        }
    }

    /// Chave de deduplicação: tipo + título normalizado + ano.
    pub fn dedup_key(&self) -> String {
        format!(
            "{}|{}|{}",
            self.kind,
            normalize_title(&self.title),
            self.year.map(|y| y.to_string()).unwrap_or_default()
        )
    }

    /// `YYYY-MM` da linha, ou vazio se não tem data utilizável.
    pub fn month(&self) -> String {
        if self.date.len() >= 7 {
            self.date[..7].to_string()
        } else {
            String::new()
        }
    }

    pub fn year_of_date(&self) -> String {
        if self.date.len() >= 4 {
            self.date[..4].to_string()
        } else {
            String::new()
        }
    }
}

// ── Normalização de título ──────────────────────────────────────────────

/// Artigos iniciais que só atrapalham o casamento entre serviços: o mesmo
/// filme é "The Thing" no Trakt e "Thing, The" em export antigo, e o mesmo
/// livro é "O Nome do Vento" e "Nome do Vento, O".
const ARTICLES: &[&str] = &[
    "the ", "a ", "an ", "o ", "os ", "as ", "um ", "uma ", "el ", "la ", "los ", "las ", "le ",
    "les ", "il ", "lo ", "der ", "die ", "das ",
];

/// Título comparável: sem acento, sem caixa, sem artigo inicial, sem
/// subtítulo depois de `:` ou ` - `, sem pontuação e sem espaço sobrando.
///
/// O subtítulo cai fora porque é onde os serviços mais divergem ("Duna" vs
/// "Duna: Parte Um" vs "Dune - Part One"), e o parêntese junto com ele — é
/// nele que vêm o ano ("Duna (2021)") e a série ("Duna (Duna, #1)"), que são
/// campo próprio no esquema. O que sobra ainda separa obras diferentes
/// porque o ano entra na chave de deduplicação junto.
pub fn normalize_title(input: &str) -> String {
    let s = crate::core::tools::music::norm::strip_accents(input).to_lowercase();
    // Subtítulo: só corta se sobra alguma coisa antes.
    let mut base = s.as_str();
    for sep in [": ", " - ", " — ", " – ", " ("] {
        if let Some(pos) = base.find(sep) {
            if pos > 0 {
                base = &base[..pos];
            }
        }
    }
    let base = base.strip_suffix(':').unwrap_or(base);
    // Sufixo "..., The" que alguns exports antigos ainda usam.
    let mut s = base.to_string();
    for art in ARTICLES {
        let tail = format!(", {}", art.trim());
        if let Some(head) = s.strip_suffix(&tail) {
            s = format!("{} {}", art.trim(), head);
            break;
        }
    }
    // Pontuação vira espaço; dígito e letra ficam.
    let cleaned: String = s
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { ' ' })
        .collect();
    let mut out = cleaned.split_whitespace().collect::<Vec<_>>().join(" ");
    for art in ARTICLES {
        if let Some(rest) = out.strip_prefix(art) {
            out = rest.to_string();
            break;
        }
    }
    out.trim().to_string()
}

/// Ano dentro do título ("Duna (2021)") ou de um campo solto. Aceita de 1870
/// (primeiros registros de cinema) até o ano que vem.
pub fn parse_year(s: &str) -> Option<i32> {
    let digits: String = s.chars().filter(|c| c.is_ascii_digit()).collect();
    if digits.len() == 4 {
        return digits
            .parse::<i32>()
            .ok()
            .filter(|y| (1400..=2200).contains(y));
    }
    let bytes: Vec<char> = s.chars().collect();
    for w in bytes.windows(4) {
        if w.iter().all(|c| c.is_ascii_digit()) {
            let y: String = w.iter().collect();
            if let Ok(y) = y.parse::<i32>() {
                if (1870..=2100).contains(&y) {
                    return Some(y);
                }
            }
        }
    }
    None
}

/// `2021-05-13T14:33:00Z`, `2021-05-13 14:33`, `2021/05/13` e `13/05/2021`
/// viram todos `2021-05-13`. Devolve vazio quando não dá para ler.
pub fn iso_date(input: &str) -> String {
    let s = input.trim();
    if s.is_empty() {
        return String::new();
    }
    let head: String = s.chars().take(10).collect();
    let parts: Vec<&str> = head.split(['-', '/', '.']).collect();
    if parts.len() == 3 {
        let (a, b, c) = (parts[0], parts[1], parts[2]);
        if a.len() == 4 {
            if let (Ok(y), Ok(m), Ok(d)) = (a.parse::<i32>(), b.parse::<u32>(), c.parse::<u32>()) {
                if (1..=12).contains(&m) && (1..=31).contains(&d) {
                    return format!("{:04}-{:02}-{:02}", y, m, d);
                }
            }
        } else if c.len() == 4 {
            // dd/mm/yyyy — o Goodreads escreve mm/dd/yyyy, tratado lá.
            if let (Ok(d), Ok(m), Ok(y)) = (a.parse::<u32>(), b.parse::<u32>(), c.parse::<i32>()) {
                if (1..=12).contains(&m) && (1..=31).contains(&d) {
                    return format!("{:04}-{:02}-{:02}", y, m, d);
                }
            }
        }
    }
    // Data por extenso do Goodreads: "May 13, 2021" / "Sep 2021".
    if let Some(d) = parse_long_date(s) {
        return d;
    }
    String::new()
}

const MONTHS: &[&str] = &[
    "jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec",
];

fn parse_long_date(s: &str) -> Option<String> {
    let low = s.to_lowercase();
    let clean: String = low
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { ' ' })
        .collect();
    let toks: Vec<&str> = clean.split_whitespace().collect();
    let mut month = None;
    let mut day: Option<u32> = None;
    let mut year: Option<i32> = None;
    for tok in &toks {
        if month.is_none() && tok.len() >= 3 {
            if let Some(i) = MONTHS.iter().position(|m| tok.starts_with(m)) {
                month = Some(i as u32 + 1);
                continue;
            }
        }
        if tok.len() == 4 {
            if let Ok(y) = tok.parse::<i32>() {
                if (1400..=2200).contains(&y) {
                    year = Some(y);
                    continue;
                }
            }
        }
        if day.is_none() && tok.len() <= 2 {
            if let Ok(d) = tok.parse::<u32>() {
                if (1..=31).contains(&d) {
                    day = Some(d);
                }
            }
        }
    }
    let (m, y) = (month?, year?);
    Some(format!("{:04}-{:02}-{:02}", y, m, day.unwrap_or(1)))
}

// ── CSV de saída ────────────────────────────────────────────────────────

pub fn csv_cell(s: &str) -> String {
    if s.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}

pub fn csv_line(cells: &[String]) -> String {
    let row: Vec<String> = cells.iter().map(|c| csv_cell(c)).collect();
    format!("{}\n", row.join(","))
}

/// Índice de cada coluna pelo nome do cabeçalho, sem depender de ordem nem
/// de caixa — o Letterboxd já mudou a ordem das colunas mais de uma vez.
pub fn header_index(header: &[String]) -> std::collections::HashMap<String, usize> {
    header
        .iter()
        .enumerate()
        .map(|(i, h)| (h.trim().to_lowercase(), i))
        .collect()
}

/// Célula pelo nome da coluna, aceitando apelidos (o mesmo campo mudou de
/// nome entre versões do export).
pub fn cell<'a>(
    row: &'a [String],
    idx: &std::collections::HashMap<String, usize>,
    names: &[&str],
) -> &'a str {
    for name in names {
        if let Some(i) = idx.get(&name.to_lowercase()) {
            if let Some(v) = row.get(*i) {
                let v = v.trim();
                if !v.is_empty() {
                    return v;
                }
            }
        }
    }
    ""
}

/// Escreve as linhas em CSV com o cabeçalho do esquema comum.
pub fn entries_csv(entries: &[Entry]) -> String {
    let mut out = String::from(
        "date,kind,title,year,creator,rating,review,source,list,tags,tmdb_id,imdb_id,isbn,times\n",
    );
    for e in entries {
        out.push_str(&csv_line(&[
            e.date.clone(),
            e.kind.clone(),
            e.title.clone(),
            e.year.map(|y| y.to_string()).unwrap_or_default(),
            e.creator.clone(),
            e.rating.map(|r| format!("{:.1}", r)).unwrap_or_default(),
            e.review.replace('\n', " ").trim().to_string(),
            e.source.clone(),
            e.list.clone(),
            e.tags.join("; "),
            e.tmdb_id.map(|v| v.to_string()).unwrap_or_default(),
            e.imdb_id.clone(),
            e.isbn.clone(),
            e.times.to_string(),
        ]));
    }
    out
}

/// Grava os formatos pedidos (`json`, `csv`, `md`) e devolve os caminhos.
pub fn write_exports(
    dir: &std::path::Path,
    stem: &str,
    entries: &[Entry],
    formats: &[String],
    markdown: impl FnOnce(&[Entry]) -> String,
) -> Result<Vec<String>> {
    std::fs::create_dir_all(dir)?;
    let want = |f: &str| formats.iter().any(|x| x.eq_ignore_ascii_case(f));
    let mut files = Vec::new();
    if want("json") {
        let p = dir.join(format!("{}.json", stem));
        std::fs::write(&p, serde_json::to_string_pretty(entries)?)?;
        files.push(p.to_string_lossy().to_string());
    }
    if want("csv") {
        let p = dir.join(format!("{}.csv", stem));
        std::fs::write(&p, entries_csv(entries))?;
        files.push(p.to_string_lossy().to_string());
    }
    if want("md") {
        let p = dir.join(format!("{}.md", stem));
        std::fs::write(&p, markdown(entries))?;
        files.push(p.to_string_lossy().to_string());
    }
    Ok(files)
}

// ── Cliente com sessão ──────────────────────────────────────────────────

/// Filtra os cookies de um domínio do arquivo Netscape e monta o pote. O
/// gerenciador grava um arquivo por (domínio, conta) e um mesmo arquivo pode
/// carregar cookie de vizinho, então o filtro não é decoração.
pub fn jar_from_netscape(content: &str, domain: &str) -> (Arc<reqwest::cookie::Jar>, usize) {
    let jar = reqwest::cookie::Jar::default();
    let suffix = format!(".{}", domain);
    let mut n = 0;
    for c in crate::core::tools::instagram::parse_netscape(content) {
        let d = c.domain.trim_start_matches('.');
        if d != domain && !d.ends_with(&suffix) {
            continue;
        }
        let path = if c.path.is_empty() { "/" } else { &c.path };
        let scheme = if c.secure { "https" } else { "http" };
        let Ok(url) = format!("{}://{}{}", scheme, d, path).parse::<reqwest::Url>() else {
            continue;
        };
        jar.add_cookie_str(
            &format!("{}={}; Domain={}; Path={}", c.name, c.value, c.domain, path),
            &url,
        );
        n += 1;
    }
    (Arc::new(jar), n)
}

const UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0.0.0 Safari/537.36";

fn cookie_client(session: Option<&str>, domain: &str) -> Result<(reqwest::Client, bool)> {
    use reqwest::header::{HeaderMap, HeaderValue, ACCEPT_LANGUAGE, USER_AGENT};
    let mut headers = HeaderMap::new();
    headers.insert(USER_AGENT, HeaderValue::from_static(UA));
    headers.insert(
        ACCEPT_LANGUAGE,
        HeaderValue::from_static("en-US,en;q=0.9,pt-BR;q=0.8"),
    );
    let (jar, seeded) = match session {
        Some(content) => jar_from_netscape(content, domain),
        None => (Arc::new(reqwest::cookie::Jar::default()), 0),
    };
    let client = crate::core::http_client::apply_global_proxy(reqwest::Client::builder())
        .default_headers(headers)
        .cookie_provider(jar)
        .timeout(Duration::from_secs(180))
        .build()?;
    Ok((client, seeded > 0))
}

/// Cliente com freio, no mesmo desenho do `Fetcher` do Reddit: uma
/// requisição por vez, espera configurável entre elas e recuo crescente
/// quando o servidor reclama. Letterboxd e Goodreads não têm API pública e
/// respondem mal a rajada — o freio é o que mantém a conta do usuário fora
/// de encrenca.
pub struct Fetcher {
    client: reqwest::Client,
    delay: Duration,
    last: tokio::sync::Mutex<Option<Instant>>,
    requests: AtomicU32,
    has_session: bool,
}

impl Fetcher {
    pub fn new(delay_ms: u64, session: Option<&str>, domain: &str) -> Result<Self> {
        let (client, has_session) = cookie_client(session, domain)?;
        Ok(Self {
            client,
            delay: Duration::from_millis(delay_ms.clamp(200, 30_000)),
            last: tokio::sync::Mutex::new(None),
            requests: AtomicU32::new(0),
            has_session,
        })
    }

    pub fn has_session(&self) -> bool {
        self.has_session
    }

    pub fn requests(&self) -> u32 {
        self.requests.load(Ordering::Relaxed)
    }

    pub fn client(&self) -> &reqwest::Client {
        &self.client
    }

    pub async fn pace(&self) {
        let mut last = self.last.lock().await;
        if let Some(t) = *last {
            let since = t.elapsed();
            if since < self.delay {
                tokio::time::sleep(self.delay - since).await;
            }
        }
        *last = Some(Instant::now());
    }

    /// GET com o corpo em texto. Repete em 429 e em erro de servidor.
    pub async fn get_text(&self, url: &str) -> Result<String> {
        const TRIES: u32 = 4;
        let mut wait = Duration::from_secs(3);
        for attempt in 1..=TRIES {
            self.pace().await;
            self.requests.fetch_add(1, Ordering::Relaxed);
            let resp = self.client.get(url).send().await;
            match resp {
                Ok(r) if r.status().is_success() => return Ok(r.text().await?),
                Ok(r) if r.status().as_u16() == 429 || r.status().is_server_error() => {
                    if attempt == TRIES {
                        return Err(anyhow!(
                            "o servidor está limitando o acesso (HTTP {}). Tente de novo daqui a pouco",
                            r.status()
                        ));
                    }
                    let retry = r
                        .headers()
                        .get(reqwest::header::RETRY_AFTER)
                        .and_then(|v| v.to_str().ok())
                        .and_then(|v| v.trim().parse::<u64>().ok())
                        .map(Duration::from_secs);
                    tokio::time::sleep(retry.unwrap_or(wait)).await;
                    wait *= 2;
                }
                Ok(r) if r.status().as_u16() == 401 || r.status().as_u16() == 403 => {
                    return Err(anyhow!(
                        "o site respondeu {} — a sessão salva no gerenciador de cookies expirou ou não tem acesso a isso",
                        r.status()
                    ));
                }
                Ok(r) if r.status().as_u16() == 404 => {
                    return Err(anyhow!("não encontrado ({})", url));
                }
                Ok(r) => return Err(anyhow!("HTTP {} em {}", r.status(), url)),
                Err(e) if attempt < TRIES => {
                    tokio::time::sleep(wait).await;
                    wait *= 2;
                    tracing::debug!("lists: tentativa {} falhou: {}", attempt, e);
                }
                Err(e) => return Err(e.into()),
            }
        }
        Err(anyhow!("não consegui buscar {}", url))
    }

    /// HEAD só para saber se um arquivo já existe. É como o Goodreads avisa
    /// que terminou de gerar o CSV: enquanto não está pronto, responde 404.
    pub async fn head_status(&self, url: &str) -> Result<u16> {
        self.pace().await;
        self.requests.fetch_add(1, Ordering::Relaxed);
        Ok(self.client.head(url).send().await?.status().as_u16())
    }

    /// GET que devolve os bytes crus (o ZIP do Letterboxd, o CSV do Goodreads).
    pub async fn get_bytes(&self, url: &str) -> Result<(Vec<u8>, String)> {
        self.pace().await;
        self.requests.fetch_add(1, Ordering::Relaxed);
        let r = self.client.get(url).send().await?;
        if !r.status().is_success() {
            return Err(anyhow!("HTTP {} em {}", r.status(), url));
        }
        let ctype = r
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default()
            .to_string();
        Ok((r.bytes().await?.to_vec(), ctype))
    }
}

// ── HTML mínimo ─────────────────────────────────────────────────────────

/// Valor de um atributo dentro de uma tag, sem trazer um parser de HTML
/// inteiro para achar dois números. Procura `attr="valor"` a partir de
/// `anchor` e devolve o primeiro que aparecer.
pub fn attr_after(html: &str, anchor: &str, attr: &str) -> Option<String> {
    let start = html.find(anchor)?;
    let rest = &html[start..];
    let pat = format!("{}=\"", attr);
    let at = rest.find(&pat)? + pat.len();
    let end = rest[at..].find('"')? + at;
    Some(rest[at..end].to_string())
}

/// Tira as tags de um pedaço de HTML e resolve as entidades mais comuns.
pub fn strip_html(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut inside = false;
    for c in html.chars() {
        match c {
            '<' => inside = true,
            // A tag vira espaço: sem isso um `<br />` entre duas frases as
            // gruda numa palavra só.
            '>' => {
                inside = false;
                out.push(' ');
            }
            _ if !inside => out.push(c),
            _ => {}
        }
    }
    out.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&#x27;", "'")
        .replace("&nbsp;", " ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normaliza_acento_artigo_e_subtitulo() {
        assert_eq!(
            normalize_title("O Auto da Compadecida"),
            "auto da compadecida"
        );
        assert_eq!(normalize_title("The Thing"), "thing");
        assert_eq!(normalize_title("Thing, The"), "thing");
        assert_eq!(normalize_title("Duna: Parte Dois"), "duna");
        assert_eq!(normalize_title("Dune - Part Two"), "dune");
        assert_eq!(normalize_title("Amélie"), "amelie");
        assert_eq!(
            normalize_title("  Blade   Runner 2049 "),
            "blade runner 2049"
        );
        assert_eq!(normalize_title("WALL·E"), "wall e");
        // Ano e série moram em campo próprio; no título são ruído.
        assert_eq!(normalize_title("Duna (2021)"), "duna");
        assert_eq!(normalize_title("Duna (Duna, #1)"), "duna");
        assert_eq!(normalize_title("Blade Runner 2049"), "blade runner 2049");
    }

    #[test]
    fn normalizacao_casa_titulos_de_servicos_diferentes() {
        assert_eq!(
            normalize_title("The Lord of the Rings: The Fellowship of the Ring"),
            normalize_title("Lord of the Rings, The")
        );
        assert_eq!(
            normalize_title("Cidade de Deus"),
            normalize_title("Cidade de Deus")
        );
        assert_ne!(normalize_title("Duna"), normalize_title("Dune"));
    }

    #[test]
    fn chave_de_dedup_usa_tipo_titulo_e_ano() {
        let mut a = Entry::new("filme", "letterboxd", "Duna: Parte Dois");
        a.year = Some(2024);
        let mut b = Entry::new("filme", "trakt", "Duna");
        b.year = Some(2024);
        let mut c = Entry::new("livro", "goodreads", "Duna");
        c.year = Some(2024);
        assert_eq!(a.dedup_key(), b.dedup_key());
        assert_ne!(a.dedup_key(), c.dedup_key());
    }

    #[test]
    fn datas_em_varios_formatos_viram_iso() {
        assert_eq!(iso_date("2021-05-13"), "2021-05-13");
        assert_eq!(iso_date("2021-05-13T14:33:00.000Z"), "2021-05-13");
        assert_eq!(iso_date("2021/05/13"), "2021-05-13");
        assert_eq!(iso_date("13/05/2021"), "2021-05-13");
        assert_eq!(iso_date("May 13, 2021"), "2021-05-13");
        assert_eq!(iso_date("Sep 2021"), "2021-09-01");
        assert_eq!(iso_date(""), "");
        assert_eq!(iso_date("not a date"), "");
    }

    #[test]
    fn ano_sai_do_titulo_ou_do_campo() {
        assert_eq!(parse_year("2021"), Some(2021));
        assert_eq!(parse_year("Duna (2021)"), Some(2021));
        assert_eq!(parse_year("sem ano"), None);
        assert_eq!(parse_year("12"), None);
    }

    #[test]
    fn csv_escapa_virgula_aspas_e_quebra() {
        assert_eq!(csv_cell("simples"), "simples");
        assert_eq!(csv_cell("a,b"), "\"a,b\"");
        assert_eq!(csv_cell("diz \"oi\""), "\"diz \"\"oi\"\"\"");
        assert_eq!(csv_line(&["a".into(), "b,c".into()]), "a,\"b,c\"\n");
    }

    #[test]
    fn cabecalho_indexa_sem_caixa_e_aceita_apelido() {
        let rows = parse_csv("Date,Name,Letterboxd URI\n2021-05-13,Duna,https://x\n");
        let idx = header_index(&rows[0]);
        assert_eq!(cell(&rows[1], &idx, &["name"]), "Duna");
        assert_eq!(cell(&rows[1], &idx, &["Título", "Name"]), "Duna");
        assert_eq!(cell(&rows[1], &idx, &["nada"]), "");
    }

    #[test]
    fn pote_de_cookie_so_aceita_o_dominio_certo() {
        let content = "# Netscape HTTP Cookie File\n\
.letterboxd.com\tTRUE\t/\tTRUE\t2000000000\tletterboxd.signed.in.as\tfulano\n\
letterboxd.com\tFALSE\t/\tFALSE\t2000000000\tcom.xk72.webparts.csrf\tabc\n\
.goodreads.com\tTRUE\t/\tTRUE\t2000000000\t_session_id2\tdeadbeef\n\
.evil.com\tTRUE\t/\tTRUE\t2000000000\troubado\t1\n";
        let (_, n) = jar_from_netscape(content, "letterboxd.com");
        assert_eq!(n, 2);
        let (_, g) = jar_from_netscape(content, "goodreads.com");
        assert_eq!(g, 1);
        let (_, vazio) = jar_from_netscape("# Netscape HTTP Cookie File\n", "letterboxd.com");
        assert_eq!(vazio, 0);
    }

    #[test]
    fn dominio_parecido_nao_entra_no_pote() {
        let content = "# Netscape HTTP Cookie File\n\
.naoletterboxd.com\tTRUE\t/\tTRUE\t2000000000\tx\t1\n\
.letterboxd.com.br.golpe.net\tTRUE\t/\tTRUE\t2000000000\ty\t2\n";
        let (_, n) = jar_from_netscape(content, "letterboxd.com");
        assert_eq!(n, 0);
    }

    #[test]
    fn html_minimo_le_atributo_e_tira_tag() {
        let html = r#"<body class="film" data-tmdb-id="438631" data-tmdb-type="movie">"#;
        assert_eq!(
            attr_after(html, "data-tmdb-id", "data-tmdb-id"),
            Some("438631".into())
        );
        assert_eq!(
            strip_html("<p>oi <b>mundo</b> &amp; cia</p>"),
            "oi mundo & cia"
        );
    }

    #[test]
    fn mes_e_ano_saem_da_data() {
        let mut e = Entry::new("filme", "letterboxd", "Duna");
        e.date = "2024-03-07".into();
        assert_eq!(e.month(), "2024-03");
        assert_eq!(e.year_of_date(), "2024");
        e.date = String::new();
        assert_eq!(e.month(), "");
    }
}

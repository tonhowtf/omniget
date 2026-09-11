//! Reddit (categoria `reddit`). O site publica qualquer permalink em JSON só
//! acrescentando `.json` ao caminho, e é disso que estas ferramentas vivem:
//! nada de login, nada de chave de API, só conteúdo público.
//!
//! - `download.rs`: post de vídeo (v.redd.it, vídeo **com** áudio pelo yt-dlp),
//!   galeria de imagens (gallery-dl), imagem única (i.redd.it) e link externo.
//! - `thread.rs`: backup de uma thread inteira (post + árvore de comentários)
//!   em Markdown, HTML navegável e JSON cru normalizado.
//! - `gdpr.rs`: leitura do export oficial de dados (pilha de CSV), sem rede.

pub mod download;
pub mod gdpr;
pub mod thread;

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{anyhow, Result};

/// O que um link do Reddit aponta.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    /// Post: `/r/<sub>/comments/<id>/<slug>/[<comentário>]`.
    Post {
        subreddit: Option<String>,
        id: String,
        comment: Option<String>,
    },
    /// `redd.it/<id>` e `/r/<sub>/s/<código>`: só um GET diz para onde vai.
    Short {
        url: String,
    },
    /// Mídia direta: `i.redd.it`, `preview.redd.it` (imagem) ou `v.redd.it`.
    Media {
        url: String,
        video: bool,
    },
    Subreddit {
        name: String,
    },
    User {
        name: String,
    },
    /// Não é do Reddit — quem resolve é o yt-dlp.
    External {
        url: String,
    },
}

fn is_reddit_host(host: &str) -> bool {
    host == "reddit.com" || host.ends_with(".reddit.com")
}

/// Base36 de post e de comentário: curto e só alfanumérico.
fn is_thing_id(s: &str) -> bool {
    let s = s.trim_start_matches("t3_").trim_start_matches("t1_");
    !s.is_empty()
        && s.len() <= 13
        && s.chars().all(|c| c.is_ascii_alphanumeric())
        && s.chars()
            .any(|c| c.is_ascii_digit() || c.is_ascii_lowercase())
}

fn clean_id(s: &str) -> String {
    s.trim_start_matches("t3_")
        .trim_start_matches("t1_")
        .to_ascii_lowercase()
}

/// Deixa a entrada com esquema para o `url::Url` conseguir ler.
fn with_scheme(input: &str) -> String {
    let s = input.trim();
    if s.starts_with("http://") || s.starts_with("https://") {
        return s.to_string();
    }
    if let Some(rest) = s.strip_prefix("//") {
        return format!("https://{}", rest);
    }
    if s.starts_with('/') {
        return format!("https://www.reddit.com{}", s);
    }
    format!("https://{}", s)
}

/// Lê qualquer forma de link do Reddit: permalink, `redd.it/<id>`, link de
/// compartilhamento `/s/<código>`, mídia direta, sub, perfil — ou só o id.
pub fn parse_target(input: &str) -> Option<Target> {
    let raw = input.trim();
    if raw.is_empty() {
        return None;
    }
    // Só o id do post, colado da barra de endereço.
    if !raw.contains('/') && !raw.contains('.') && is_thing_id(raw) && raw.len() >= 5 {
        return Some(Target::Post {
            subreddit: None,
            id: clean_id(raw),
            comment: None,
        });
    }
    let url = url::Url::parse(&with_scheme(raw)).ok()?;
    let host = url
        .host_str()?
        .trim_start_matches("www.")
        .to_ascii_lowercase();
    let segs: Vec<String> = url
        .path_segments()
        .map(|s| s.filter(|p| !p.is_empty()).map(|p| p.to_string()).collect())
        .unwrap_or_default();

    match host.as_str() {
        "redd.it" => {
            return segs.first().map(|_| Target::Short {
                url: url.to_string(),
            })
        }
        "v.redd.it" => {
            return Some(Target::Media {
                url: url.to_string(),
                video: true,
            })
        }
        "i.redd.it" | "preview.redd.it" | "i.redditmedia.com" => {
            return Some(Target::Media {
                url: url.to_string(),
                video: false,
            })
        }
        _ => {}
    }
    if !is_reddit_host(&host) {
        return Some(Target::External {
            url: url.to_string(),
        });
    }

    let at = |i: usize| segs.get(i).map(|s| s.as_str()).unwrap_or_default();
    match at(0) {
        "r" if at(2) == "comments" && is_thing_id(at(3)) => Some(Target::Post {
            subreddit: Some(at(1).to_ascii_lowercase()),
            id: clean_id(at(3)),
            comment: comment_of(&segs, 4),
        }),
        "r" if at(2) == "s" && !at(3).is_empty() => Some(Target::Short {
            url: url.to_string(),
        }),
        "r" if !at(1).is_empty() => Some(Target::Subreddit {
            name: at(1).to_ascii_lowercase(),
        }),
        "comments" if is_thing_id(at(1)) => Some(Target::Post {
            subreddit: None,
            id: clean_id(at(1)),
            comment: comment_of(&segs, 2),
        }),
        "gallery" if is_thing_id(at(1)) => Some(Target::Post {
            subreddit: None,
            id: clean_id(at(1)),
            comment: None,
        }),
        "user" | "u" if at(2) == "comments" && is_thing_id(at(3)) => Some(Target::Post {
            subreddit: None,
            id: clean_id(at(3)),
            comment: comment_of(&segs, 4),
        }),
        "user" | "u" if !at(1).is_empty() => Some(Target::User {
            name: at(1).to_string(),
        }),
        _ => None,
    }
}

/// Depois do slug pode vir o id do comentário focado (`/<slug>/<id>/` ou
/// `/<slug>/comment/<id>/`).
fn comment_of(segs: &[String], from: usize) -> Option<String> {
    let tail: Vec<&str> = segs
        .iter()
        .skip(from)
        .map(|s| s.as_str())
        .filter(|s| *s != "comment")
        .collect();
    // O primeiro item é o slug do título; o id vem depois.
    tail.iter()
        .skip(1)
        .find(|s| is_thing_id(s) && s.len() >= 5)
        .map(|s| clean_id(s))
}

/// JSON do post inteiro. `/comments/<id>.json` dispensa saber o sub.
pub fn post_json_url(id: &str, sort: &str, limit: u32) -> String {
    format!(
        "https://www.reddit.com/comments/{}.json?limit={}&raw_json=1&sort={}",
        id, limit, sort
    )
}

/// JSON de um galho: o permalink do comentário com `context=0`.
pub fn branch_json_url(post_id: &str, comment_id: &str, sort: &str, limit: u32) -> String {
    format!(
        "https://www.reddit.com/comments/{}/_/{}.json?limit={}&raw_json=1&sort={}&context=0",
        post_id, comment_id, limit, sort
    )
}

/// `api/morechildren`: os comentários que a página escondeu atrás de
/// "carregar mais". Público, sem token.
pub fn more_children_url(post_id: &str, children: &[String], sort: &str) -> String {
    format!(
        "https://www.reddit.com/api/morechildren.json?api_type=json&link_id=t3_{}&children={}&limit_children=false&sort={}&raw_json=1",
        post_id,
        children.join(","),
        sort
    )
}

/// O desafio de JavaScript que o Reddit serve para quem chega sem cookie:
/// uma página com um `jsc_token` e um script que dobra uma constante. Sem
/// passar por ele, todo `.json` volta 403 ("blocked by network security").
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JsChallenge {
    pub action: String,
    pub token: String,
    pub solution: String,
}

/// Lê o desafio de dentro do HTML. `None` quando a página é normal.
pub fn parse_js_challenge(html: &str) -> Option<JsChallenge> {
    if !html.contains("jsc_token") {
        return None;
    }
    let token = between(html, "name=\"jsc_token\" value=\"", "\"")?;
    let action = between(html, "<form hidden method=\"GET\" action=\"", "\"")
        .unwrap_or_else(|| "/".to_string());
    // `await(async e=>e+e)("2e40…")`: a resposta é a constante repetida uma
    // vez por termo da soma.
    let re = regex::Regex::new(
        r#"\(\s*async\s+(\w+)\s*=>\s*((?:\w+\s*\+\s*)*\w+)\s*\)\s*\(\s*"([0-9A-Za-z]+)"\s*\)"#,
    )
    .ok()?;
    let caps = re.captures(html)?;
    let var = caps.get(1)?.as_str();
    let expr = caps.get(2)?.as_str();
    let literal = caps.get(3)?.as_str();
    let terms: Vec<&str> = expr.split('+').map(|t| t.trim()).collect();
    if terms.is_empty() || terms.len() > 8 || terms.iter().any(|t| *t != var) {
        return None;
    }
    Some(JsChallenge {
        action,
        token,
        solution: literal.repeat(terms.len()),
    })
}

fn between(hay: &str, open: &str, close: &str) -> Option<String> {
    let start = hay.find(open)? + open.len();
    let rest = &hay[start..];
    let end = rest.find(close)?;
    Some(rest[..end].to_string())
}

/// Cliente com freio: o Reddit devolve 429 com facilidade em conteúdo
/// público, então uma requisição por vez, com espera entre elas e recuo
/// crescente quando o servidor reclama. Guarda cookies porque o desafio de
/// JavaScript da porta de entrada só vale enquanto eles durarem.
pub struct Fetcher {
    client: reqwest::Client,
    delay: Duration,
    last: tokio::sync::Mutex<Option<Instant>>,
    requests: AtomicU32,
    unlocked: tokio::sync::Mutex<bool>,
    has_session: bool,
}

/// Filtra os cookies do bucket `reddit.com` de um arquivo Netscape e monta o
/// pote do cliente. Reusa o parser que a sessão do Instagram já usa — é o
/// mesmo formato que o gerenciador de cookies do app grava para todo mundo.
fn jar_from_netscape(content: &str) -> (Arc<reqwest::cookie::Jar>, usize) {
    let jar = reqwest::cookie::Jar::default();
    let mut n = 0;
    for c in crate::core::tools::instagram::parse_netscape(content) {
        let domain = c.domain.trim_start_matches('.');
        if domain != "reddit.com" && !domain.ends_with(".reddit.com") {
            continue;
        }
        let path = if c.path.is_empty() { "/" } else { &c.path };
        let scheme = if c.secure { "https" } else { "http" };
        let Ok(url) = format!("{}://{}{}", scheme, domain, path).parse::<reqwest::Url>() else {
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

/// Mesmo UA do `tools::client()`, mas com loja de cookies: sem guardar o que
/// o desafio devolve, a próxima requisição volta a ser barrada.
fn cookie_client(session: Option<&str>) -> Result<(reqwest::Client, bool)> {
    use reqwest::header::{HeaderMap, HeaderValue, ACCEPT_LANGUAGE, USER_AGENT};
    let mut headers = HeaderMap::new();
    headers.insert(
        USER_AGENT,
        HeaderValue::from_static(
            "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0.0.0 Safari/537.36",
        ),
    );
    headers.insert(
        ACCEPT_LANGUAGE,
        HeaderValue::from_static("en-US,en;q=0.9,pt-BR;q=0.8"),
    );
    // O pote sempre existe: além de carregar a sessão do usuário (quando ela
    // vem), é ele que guarda o que o desafio de JavaScript devolve.
    let (jar, seeded) = match session {
        Some(content) => jar_from_netscape(content),
        None => (Arc::new(reqwest::cookie::Jar::default()), 0),
    };
    let client = crate::core::http_client::apply_global_proxy(reqwest::Client::builder())
        .default_headers(headers)
        .cookie_provider(jar)
        .timeout(Duration::from_secs(120))
        .build()?;
    Ok((client, seeded > 0))
}

impl Fetcher {
    /// `session` é o conteúdo Netscape do bucket `reddit.com` capturado pela
    /// extensão, quando existe. Com ele o Reddit responde como responde para o
    /// navegador do usuário e o desafio de JavaScript nem aparece; sem ele, o
    /// desafio continua sendo o caminho (é o que sustenta o modo anônimo).
    pub fn new(delay_ms: u64, session: Option<&str>) -> Result<Self> {
        let (client, has_session) = cookie_client(session)?;
        Ok(Self {
            client,
            delay: Duration::from_millis(delay_ms.clamp(200, 10_000)),
            last: tokio::sync::Mutex::new(None),
            requests: AtomicU32::new(0),
            unlocked: tokio::sync::Mutex::new(false),
            has_session,
        })
    }

    /// Se a requisição está saindo com a sessão do usuário ou anônima.
    pub fn has_session(&self) -> bool {
        self.has_session
    }

    pub fn requests(&self) -> u32 {
        self.requests.load(Ordering::Relaxed)
    }

    pub fn client(&self) -> &reqwest::Client {
        &self.client
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

    /// Passa pela porta de entrada: abre uma página normal, resolve o desafio
    /// de JavaScript se ele aparecer e guarda os cookies. Sem isso o Reddit
    /// responde 403 em todo `.json`.
    pub async fn unlock(&self) -> Result<()> {
        let mut done = self.unlocked.lock().await;
        if *done {
            return Ok(());
        }
        self.pace().await;
        self.requests.fetch_add(1, Ordering::Relaxed);
        let html = self
            .client
            .get("https://www.reddit.com/r/popular/")
            .send()
            .await?
            .text()
            .await?;
        if let Some(ch) = parse_js_challenge(&html) {
            let action = if ch.action.starts_with("http") {
                ch.action.clone()
            } else {
                format!("https://www.reddit.com{}", ch.action)
            };
            self.pace().await;
            self.requests.fetch_add(1, Ordering::Relaxed);
            let _ = self
                .client
                .get(&action)
                .query(&[
                    ("solution", ch.solution.as_str()),
                    ("js_challenge", "1"),
                    ("jsc_token", ch.token.as_str()),
                    ("jsc_orig_r", ""),
                ])
                .send()
                .await?
                .text()
                .await?;
        }
        *done = true;
        Ok(())
    }

    /// GET com JSON de volta. Repete em 429 e em erro de servidor, e passa
    /// pelo desafio de JavaScript quando leva 403.
    pub async fn get_json(&self, url: &str) -> Result<serde_json::Value> {
        const TRIES: u32 = 4;
        let mut wait = Duration::from_secs(3);
        for attempt in 1..=TRIES {
            self.pace().await;
            self.requests.fetch_add(1, Ordering::Relaxed);
            let resp = self.client.get(url).send().await;
            match resp {
                Ok(r) if r.status().as_u16() == 403 && attempt < TRIES => {
                    let _ = r.text().await;
                    self.unlock().await?;
                }
                Ok(r) if r.status().is_success() => {
                    let text = r.text().await?;
                    return serde_json::from_str(&text).map_err(|e| {
                        anyhow!("o Reddit respondeu algo que não é JSON ({}): {}", e, url)
                    });
                }
                Ok(r) if r.status().as_u16() == 429 || r.status().is_server_error() => {
                    if attempt == TRIES {
                        return Err(anyhow!(
                            "o Reddit está limitando o acesso (HTTP {}). Tente de novo daqui a pouco",
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
                Ok(r) if r.status().as_u16() == 403 => {
                    return Err(anyhow!(
                        "o Reddit barrou o acesso público (403). Costuma ser bloqueio de rede ou conteúdo restrito: tente de outra conexão"
                    ));
                }
                Ok(r) if r.status().as_u16() == 404 => {
                    return Err(anyhow!(
                        "post não encontrado (apagado, privado ou id errado)"
                    ));
                }
                Ok(r) => {
                    return Err(anyhow!("HTTP {} em {}", r.status(), url));
                }
                Err(e) if attempt < TRIES => {
                    tokio::time::sleep(wait).await;
                    wait *= 2;
                    let _ = e;
                }
                Err(e) => return Err(e.into()),
            }
        }
        Err(anyhow!("não foi possível ler {}", url))
    }

    /// Segue os redirecionamentos de um link curto e devolve a URL final.
    pub async fn resolve(&self, url: &str) -> Result<String> {
        self.pace().await;
        self.requests.fetch_add(1, Ordering::Relaxed);
        let r = self.client.get(url).send().await?;
        Ok(r.url().to_string())
    }
}

/// Data legível a partir do `created_utc` do Reddit.
pub fn fmt_utc(ts: f64) -> String {
    chrono::DateTime::from_timestamp(ts as i64, 0)
        .map(|d| d.format("%Y-%m-%d %H:%M UTC").to_string())
        .unwrap_or_default()
}

/// Escapa o que vai para dentro de um HTML gerado por nós.
pub fn esc(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn post(t: Option<Target>) -> (Option<String>, String, Option<String>) {
        match t {
            Some(Target::Post {
                subreddit,
                id,
                comment,
            }) => (subreddit, id, comment),
            other => panic!("esperava um post, veio {:?}", other),
        }
    }

    #[test]
    fn o_pote_so_aceita_cookie_do_reddit() {
        // Formato Netscape: dominio, flag de subdominio, path, secure, expira,
        // nome, valor. A terceira linha e de outro site e nao pode entrar.
        let content = "\
# Netscape HTTP Cookie File
.reddit.com\tTRUE\t/\tTRUE\t0\treddit_session\tabc123
reddit.com\tFALSE\t/\tTRUE\t0\ttoken_v2\tdef456
.instagram.com\tTRUE\t/\tTRUE\t0\tsessionid\tnao-e-daqui
";
        let (_, n) = jar_from_netscape(content);
        assert_eq!(n, 2, "so os dois cookies de reddit.com entram no pote");
        let (_, vazio) = jar_from_netscape("# Netscape HTTP Cookie File\n");
        assert_eq!(vazio, 0);
    }

    #[test]
    fn le_permalink_completo() {
        let (sub, id, c) = post(parse_target(
            "https://www.reddit.com/r/rust/comments/1abcdef/um_titulo_qualquer/",
        ));
        assert_eq!(sub.as_deref(), Some("rust"));
        assert_eq!(id, "1abcdef");
        assert_eq!(c, None);
    }

    #[test]
    fn le_comentario_focado() {
        let (sub, id, c) = post(parse_target(
            "https://old.reddit.com/r/rust/comments/1abcdef/um_titulo/kx1y2z3/",
        ));
        assert_eq!(sub.as_deref(), Some("rust"));
        assert_eq!(id, "1abcdef");
        assert_eq!(c.as_deref(), Some("kx1y2z3"));
        let (_, _, c2) = post(parse_target(
            "https://www.reddit.com/r/rust/comments/1abcdef/um_titulo/comment/kx1y2z3/?utm_source=share",
        ));
        assert_eq!(c2.as_deref(), Some("kx1y2z3"));
    }

    #[test]
    fn le_permalink_sem_sub_e_relativo() {
        let (sub, id, _) = post(parse_target("/r/rust/comments/1abcdef/titulo/"));
        assert_eq!(sub.as_deref(), Some("rust"));
        assert_eq!(id, "1abcdef");
        let (sub2, id2, _) = post(parse_target("https://reddit.com/comments/1abcdef"));
        assert_eq!(sub2, None);
        assert_eq!(id2, "1abcdef");
    }

    #[test]
    fn le_id_solto_e_com_prefixo() {
        assert_eq!(post(parse_target("1abcdef")).1, "1abcdef");
        assert_eq!(post(parse_target("t3_1abcdef")).1, "1abcdef");
    }

    #[test]
    fn le_perfil_e_post_de_perfil() {
        assert_eq!(
            parse_target("https://www.reddit.com/user/spez/"),
            Some(Target::User {
                name: "spez".to_string()
            })
        );
        assert_eq!(
            post(parse_target(
                "https://www.reddit.com/user/spez/comments/1abcdef/titulo/"
            ))
            .1,
            "1abcdef"
        );
    }

    #[test]
    fn le_curtos_e_compartilhados() {
        assert!(matches!(
            parse_target("https://redd.it/1abcdef"),
            Some(Target::Short { .. })
        ));
        assert!(matches!(
            parse_target("https://www.reddit.com/r/rust/s/aBcDeF12"),
            Some(Target::Short { .. })
        ));
    }

    #[test]
    fn le_midia_direta() {
        assert!(matches!(
            parse_target("https://v.redd.it/abcd1234"),
            Some(Target::Media { video: true, .. })
        ));
        assert!(matches!(
            parse_target("https://i.redd.it/abcd1234.jpg"),
            Some(Target::Media { video: false, .. })
        ));
    }

    #[test]
    fn le_sub_galeria_e_externo() {
        assert_eq!(
            parse_target("https://www.reddit.com/r/AskReddit/"),
            Some(Target::Subreddit {
                name: "askreddit".to_string()
            })
        );
        assert_eq!(
            post(parse_target("https://www.reddit.com/gallery/1abcdef")).1,
            "1abcdef"
        );
        assert!(matches!(
            parse_target("https://www.redgifs.com/watch/algumacoisa"),
            Some(Target::External { .. })
        ));
        assert_eq!(parse_target("   "), None);
    }

    #[test]
    fn monta_as_urls_de_api() {
        assert_eq!(
            post_json_url("1abcdef", "top", 500),
            "https://www.reddit.com/comments/1abcdef.json?limit=500&raw_json=1&sort=top"
        );
        assert!(
            branch_json_url("1abcdef", "kx1y2z3", "top", 500).contains("/1abcdef/_/kx1y2z3.json")
        );
        let more = more_children_url("1abcdef", &["a".to_string(), "b".to_string()], "top");
        assert!(more.contains("link_id=t3_1abcdef"));
        assert!(more.contains("children=a,b"));
    }

    #[test]
    fn le_o_desafio_de_javascript() {
        let html = r#"<html><head><script nonce="x">
          document.addEventListener("DOMContentLoaded",async function(){var e=document.forms[0],n=(e.onsubmit=function(t){return new URLSearchParams(document.location.search).forEach((e,n)=>t.target.appendChild(Object.assign(document.createElement("input"),{name:n,type:"hidden",value:e}))),!0},await(async e=>e+e)("2e403072eb7ea220"));e.elements.namedItem("solution").value=n,e.requestSubmit()},{once:!0});
        </script></head><body>
          <form hidden method="GET" action="/r/rust/">
            <input type="hidden" name="solution" />
            <input type="hidden" name="js_challenge" value="1"/>
            <input type="hidden" name="jsc_token" value="7afd7253fec22262ff1c52b1703fe9ec"/>
          </form></body></html>"#;
        let ch = parse_js_challenge(html).expect("desafio");
        assert_eq!(ch.action, "/r/rust/");
        assert_eq!(ch.token, "7afd7253fec22262ff1c52b1703fe9ec");
        assert_eq!(ch.solution, "2e403072eb7ea2202e403072eb7ea220");
        assert_eq!(
            parse_js_challenge("<html><body>pagina normal</body></html>"),
            None
        );
    }

    #[test]
    fn escapa_html_e_formata_data() {
        assert_eq!(
            esc("<a href=\"x\">&</a>"),
            "&lt;a href=&quot;x&quot;&gt;&amp;&lt;/a&gt;"
        );
        assert_eq!(fmt_utc(1_700_000_000.0), "2023-11-14 22:13 UTC");
    }
}

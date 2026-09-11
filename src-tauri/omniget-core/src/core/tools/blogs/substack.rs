//! Arquivo pessoal das newsletters do Substack.
//!
//! O que a tool faz: para cada publicação que o usuário assina, percorre o
//! arquivo dela e grava cada post em Markdown (com front-matter), HTML e JSON.
//! É backup do que a **assinatura dele** entrega — nada além disso.
//!
//! ## Endpoints (verificados em 2026-09-09)
//!
//! - `GET https://<pub>.substack.com/api/v1/archive?sort=new&search=&offset=N&limit=M`
//!   — **200 verificado**, devolve um array de resumos de post (`id`, `slug`,
//!   `title`, `post_date`, `audience`, `canonical_url`, `type`). O `offset`
//!   pagina de verdade (conferido com offset 0 e 2 devolvendo posts
//!   diferentes). Quando `limit` volta menos itens do que o pedido, acabou.
//! - `GET https://<pub>.substack.com/api/v1/posts/<slug>` — **200 verificado**,
//!   devolve o post inteiro com `body_html`, `subtitle`, `publishedBylines`,
//!   `postTags`, `wordcount`.
//! - `GET https://substack.com/api/v1/subscriptions` — **401 "Please sign in"
//!   verificado sem cookie**; com a sessão do usuário é ela que lista as
//!   assinaturas. Não deu para exercitar o caminho logado, então a tool sai
//!   como `beta` e o modo manual (o usuário digita as publicações) é o
//!   caminho garantido.
//!
//! ## O limite
//!
//! Post que a conta não pode ler chega sem corpo (`body_html` vazio) ou
//! truncado. A tool **conta como pulado** e segue adiante. Não existe aqui
//! nenhuma tentativa de contornar paywall: o que a assinatura abre, sai; o
//! resto fica de fora, e o relatório diz quantos foram.

use std::path::PathBuf;

use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{
    file_stem, front_matter, html_to_markdown, localize_images, write_post, Fetcher, PostDoc,
    PostMeta,
};
use crate::core::tools::ProgressFn;

const ID: &str = "blog-substack";
/// Domínios cujos cookies este cliente aceita. O `substack.com` cobre o
/// `<pub>.substack.com` de todo mundo; domínio próprio só funciona se o
/// usuário tiver capturado os cookies dele também.
const COOKIE_DOMAINS: &[&str] = &["substack.com"];

// ── Opções e resultado ─────────────────────────────────────────────────

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct Options {
    /// Publicações a exportar: `foo`, `foo.substack.com` ou a URL inteira.
    /// Vazio = descobrir pelas assinaturas da conta logada.
    pub publications: Vec<String>,
    pub dest: String,
    /// Posts por publicação, do mais novo para o mais velho. 0 = tudo.
    pub limit: usize,
    /// `AAAA-MM-DD`: para de descer quando o post é mais velho que isso.
    pub since: String,
    pub markdown: bool,
    pub html: bool,
    pub json: bool,
    /// Baixar as imagens para uma pasta ao lado e reescrever os links.
    pub images: bool,
    pub delay_ms: u64,
    /// Teto de requisições, para a exportação nunca virar uma varredura sem
    /// fim num arquivo de dez anos.
    pub max_requests: u32,
    pub account_slug: Option<String>,
    pub session_netscape: Option<String>,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            publications: Vec::new(),
            dest: String::new(),
            limit: 50,
            since: String::new(),
            markdown: true,
            html: false,
            json: false,
            images: false,
            delay_ms: 1200,
            max_requests: 400,
            account_slug: None,
            session_netscape: None,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct PublicationReport {
    pub host: String,
    pub name: String,
    pub dir: String,
    pub posts: usize,
    /// Posts que a conta não abre (pago de terceiro, arquivado): pulados.
    pub locked: usize,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ArchiveResult {
    pub used_session: bool,
    pub dest: String,
    pub publications: Vec<PublicationReport>,
    pub posts: usize,
    pub locked: usize,
    pub images: usize,
    pub requests: u32,
    pub files: Vec<String>,
}

// ── Peças puras ────────────────────────────────────────────────────────

/// Uma assinatura da conta: publicação e se é paga.
#[derive(Debug, Clone, PartialEq)]
pub struct Subscription {
    pub host: String,
    pub name: String,
    pub paid: bool,
}

/// `foo`, `foo.substack.com`, `https://foo.substack.com/p/x` — tudo vira o
/// host da publicação. Nome com ponto é tratado como domínio próprio.
pub fn normalize_host(input: &str) -> Option<String> {
    let s = input.trim().trim_matches('/');
    if s.is_empty() {
        return None;
    }
    let s = s
        .strip_prefix("https://")
        .or_else(|| s.strip_prefix("http://"))
        .unwrap_or(s);
    let s = s.split('/').next().unwrap_or(s);
    let s = s.split('?').next().unwrap_or(s).trim().to_lowercase();
    if s.is_empty() {
        return None;
    }
    if s.contains('.') {
        Some(s)
    } else if s
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        Some(format!("{}.substack.com", s))
    } else {
        None
    }
}

fn as_str(v: &Value, key: &str) -> String {
    v.get(key).and_then(Value::as_str).unwrap_or("").to_string()
}

/// Host de uma publicação vinda do JSON: domínio próprio quando existe,
/// senão `<subdomain>.substack.com`.
fn publication_host(p: &Value) -> Option<String> {
    let custom = as_str(p, "custom_domain");
    let custom_ok = p
        .get("custom_domain_optional")
        .and_then(Value::as_bool)
        .map(|opt| !opt)
        .unwrap_or(true);
    if !custom.is_empty() && custom_ok {
        return normalize_host(&custom);
    }
    let sub = as_str(p, "subdomain");
    if !sub.is_empty() {
        return normalize_host(&sub);
    }
    normalize_host(&custom)
}

/// Lê `/api/v1/subscriptions`. O JSON traz `publications` (o catálogo) e
/// `subscriptions` (o vínculo da conta, com `membership_state`). Só entra o
/// que está de fato assinado; `expired`/`none` fica de fora.
pub fn parse_subscriptions(v: &Value) -> Vec<Subscription> {
    let pubs = v.get("publications").and_then(Value::as_array);
    let subs = v.get("subscriptions").and_then(Value::as_array);
    let Some(pubs) = pubs else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for p in pubs {
        let Some(host) = publication_host(p) else {
            continue;
        };
        let id = p.get("id").and_then(Value::as_i64);
        let mut active = subs.is_none();
        let mut paid = false;
        if let (Some(subs), Some(id)) = (subs, id) {
            for s in subs {
                if s.get("publication_id").and_then(Value::as_i64) != Some(id) {
                    continue;
                }
                let state = as_str(s, "membership_state");
                if state == "expired" || state == "none" || state.is_empty() {
                    continue;
                }
                active = true;
                paid = state != "free_signup";
            }
        }
        if !active {
            continue;
        }
        let name = {
            let n = as_str(p, "name");
            if n.is_empty() {
                host.clone()
            } else {
                n
            }
        };
        if out.iter().any(|s: &Subscription| s.host == host) {
            continue;
        }
        out.push(Subscription { host, name, paid });
    }
    out
}

/// Um item do arquivo da publicação.
#[derive(Debug, Clone, PartialEq)]
pub struct ArchiveItem {
    pub slug: String,
    pub title: String,
    pub date: String,
    pub audience: String,
    pub url: String,
}

pub fn archive_url(host: &str, offset: usize, limit: usize) -> String {
    format!(
        "https://{}/api/v1/archive?sort=new&search=&offset={}&limit={}",
        host, offset, limit
    )
}

pub fn post_url(host: &str, slug: &str) -> String {
    format!("https://{}/api/v1/posts/{}", host, slug)
}

/// Lê uma página do arquivo. Post sem `slug` não serve para nada e cai fora.
pub fn parse_archive(v: &Value) -> Vec<ArchiveItem> {
    let items = match v.as_array() {
        Some(a) => a.clone(),
        None => v
            .get("posts")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default(),
    };
    items
        .iter()
        .filter_map(|p| {
            let slug = as_str(p, "slug");
            if slug.is_empty() {
                return None;
            }
            Some(ArchiveItem {
                title: as_str(p, "title"),
                date: as_str(p, "post_date"),
                audience: as_str(p, "audience"),
                url: as_str(p, "canonical_url"),
                slug,
            })
        })
        .collect()
}

/// Offsets de uma varredura do arquivo: páginas de `page` itens até `limit`
/// (0 = sem teto, aí o chamador para quando a página vier curta).
pub fn offsets(limit: usize, page: usize) -> Vec<usize> {
    let page = page.max(1);
    if limit == 0 {
        return (0..200).map(|i| i * page).collect();
    }
    let pages = limit.div_ceil(page);
    (0..pages).map(|i| i * page).collect()
}

/// `AAAA-MM-DD` de uma data ISO. Serve para comparar com o `since` sem
/// arrastar fuso para dentro da conta.
pub fn day_of(date: &str) -> String {
    date.chars().take(10).collect()
}

/// Monta o documento a partir do JSON de `/api/v1/posts/<slug>`.
pub fn parse_post(v: &Value, publication: &str) -> PostDoc {
    let body = as_str(v, "body_html");
    let title = as_str(v, "title");
    let subtitle = as_str(v, "subtitle");
    let author = v
        .get("publishedBylines")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .map(|b| as_str(b, "name"))
                .filter(|n| !n.is_empty())
                .collect::<Vec<_>>()
                .join(", ")
        })
        .unwrap_or_default();
    let tags = v
        .get("postTags")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .map(|t| {
                    let n = as_str(t, "name");
                    if n.is_empty() {
                        as_str(t, "slug")
                    } else {
                        n
                    }
                })
                .filter(|n| !n.is_empty())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let audience = as_str(v, "audience");
    let meta = PostMeta {
        title: title.clone(),
        subtitle: if subtitle.trim().is_empty() {
            None
        } else {
            Some(subtitle)
        },
        author,
        publication: publication.to_string(),
        date: as_str(v, "post_date"),
        url: as_str(v, "canonical_url"),
        tags,
        audience: if audience.is_empty() {
            None
        } else {
            Some(audience)
        },
        kind: "post".into(),
        words: v.get("wordcount").and_then(Value::as_u64),
    };
    let locked = body.trim().is_empty();
    PostDoc {
        slug: as_str(v, "slug"),
        markdown: if locked {
            String::new()
        } else {
            html_to_markdown(&body)
        },
        html: if locked { None } else { Some(body) },
        meta,
        locked,
    }
}

// ── Execução ───────────────────────────────────────────────────────────

const PAGE: usize = 12;

/// Descobre as assinaturas da conta logada. Sem sessão não há o que
/// descobrir: o endpoint responde 401.
pub async fn discover(fetcher: &Fetcher) -> Result<Vec<Subscription>> {
    if !fetcher.has_session() {
        return Err(anyhow!(
            "sem a sessão do Substack não dá para listar as suas assinaturas. Capture os cookies de substack.com no gerenciador, ou digite as publicações à mão"
        ));
    }
    let v = fetcher
        .get_json("https://substack.com/api/v1/subscriptions")
        .await?;
    let subs = parse_subscriptions(&v);
    if subs.is_empty() {
        return Err(anyhow!(
            "a conta logada não tem nenhuma assinatura ativa (ou a sessão expirou)"
        ));
    }
    Ok(subs)
}

pub async fn run(opts: &Options, progress: ProgressFn) -> Result<ArchiveResult> {
    let dest = opts.dest.trim();
    if dest.is_empty() {
        return Err(anyhow!("escolha a pasta de destino"));
    }
    if !opts.markdown && !opts.html && !opts.json {
        return Err(anyhow!("escolha ao menos um formato"));
    }
    let root = PathBuf::from(dest);
    std::fs::create_dir_all(&root)?;

    let domains: Vec<String> = COOKIE_DOMAINS.iter().map(|d| d.to_string()).collect();
    let fetcher = Fetcher::new(opts.delay_ms, opts.session_netscape.as_deref(), &domains)?;

    // Lista de publicações: o que o usuário digitou tem precedência; vazio
    // significa "as minhas assinaturas".
    let targets: Vec<Subscription> = if opts.publications.is_empty() {
        crate::core::tools::report(
            &progress,
            ID,
            "discover",
            0,
            None,
            Some("assinaturas".into()),
        );
        discover(&fetcher).await?
    } else {
        opts.publications
            .iter()
            .filter_map(|p| normalize_host(p))
            .map(|host| Subscription {
                name: host.clone(),
                host,
                paid: false,
            })
            .collect()
    };
    if targets.is_empty() {
        return Err(anyhow!("nenhuma publicação válida na lista"));
    }

    let since = opts.since.trim().to_string();
    let mut out = ArchiveResult {
        used_session: fetcher.has_session(),
        dest: root.to_string_lossy().to_string(),
        publications: Vec::new(),
        posts: 0,
        locked: 0,
        images: 0,
        requests: 0,
        files: Vec::new(),
    };

    let total = targets.len() as u64;
    for (i, sub) in targets.iter().enumerate() {
        crate::core::tools::report(
            &progress,
            ID,
            "publication",
            i as u64,
            Some(total),
            Some(sub.name.clone()),
        );
        let dir = root.join(crate::core::tools::sanitize_name(
            &super::slugify(&sub.host).replace("-substack-com", ""),
        ));
        let mut report = PublicationReport {
            host: sub.host.clone(),
            name: sub.name.clone(),
            dir: dir.to_string_lossy().to_string(),
            posts: 0,
            locked: 0,
            error: None,
        };
        match export_publication(&fetcher, opts, sub, &dir, &since, &progress, &mut out).await {
            Ok((posts, locked)) => {
                report.posts = posts;
                report.locked = locked;
            }
            Err(e) => report.error = Some(e.to_string()),
        }
        out.publications.push(report);
        if fetcher.requests() >= opts.max_requests {
            break;
        }
    }

    out.requests = fetcher.requests();
    crate::core::tools::report(
        &progress,
        ID,
        "done",
        out.posts as u64,
        Some(out.posts as u64),
        None,
    );
    Ok(out)
}

async fn export_publication(
    fetcher: &Fetcher,
    opts: &Options,
    sub: &Subscription,
    dir: &std::path::Path,
    since: &str,
    progress: &ProgressFn,
    out: &mut ArchiveResult,
) -> Result<(usize, usize)> {
    let mut items: Vec<ArchiveItem> = Vec::new();
    for offset in offsets(opts.limit, PAGE) {
        if fetcher.requests() >= opts.max_requests {
            break;
        }
        let v = fetcher
            .get_json(&archive_url(&sub.host, offset, PAGE))
            .await?;
        let page = parse_archive(&v);
        let short = page.len() < PAGE;
        let mut stop = false;
        for it in page {
            if !since.is_empty() && day_of(&it.date).as_str() < since {
                stop = true;
                break;
            }
            items.push(it);
            if opts.limit > 0 && items.len() >= opts.limit {
                stop = true;
                break;
            }
        }
        if stop || short {
            break;
        }
    }
    if items.is_empty() {
        return Ok((0, 0));
    }

    let mut posts = 0usize;
    let mut locked = 0usize;
    let total = items.len() as u64;
    for (i, it) in items.iter().enumerate() {
        if fetcher.requests() >= opts.max_requests {
            break;
        }
        crate::core::tools::report(
            progress,
            ID,
            "progress",
            i as u64,
            Some(total),
            Some(it.title.clone()),
        );
        let v = match fetcher.get_json(&post_url(&sub.host, &it.slug)).await {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!("substack: post {} falhou: {}", it.slug, e);
                continue;
            }
        };
        let mut doc = parse_post(&v, &sub.name);
        if doc.meta.title.trim().is_empty() {
            doc.meta.title = it.title.clone();
        }
        if doc.meta.date.trim().is_empty() {
            doc.meta.date.clone_from(&it.date);
        }
        if doc.meta.url.trim().is_empty() {
            doc.meta.url = format!("https://{}/p/{}", sub.host, it.slug);
        }
        if doc.locked {
            locked += 1;
            continue;
        }
        let stem = file_stem(&doc.meta.date, &doc.meta.title, &it.slug);
        if opts.images {
            let rel = format!("{}-imagens", stem);
            match localize_images(&doc.markdown, fetcher, dir, &rel, progress, ID).await {
                Ok((md, n)) => {
                    doc.markdown = md;
                    out.images += n;
                }
                Err(e) => tracing::warn!("substack: imagens de {} falharam: {}", it.slug, e),
            }
        }
        let files = write_post(dir, &stem, &doc, opts.markdown, opts.html, opts.json)?;
        for f in files {
            out.files.push(f.to_string_lossy().to_string());
        }
        posts += 1;
        out.posts += 1;
    }
    out.locked += locked;
    Ok((posts, locked))
}

/// Só para a UI mostrar o cabeçalho de um post sem gravar nada.
pub fn preview(doc: &PostDoc) -> String {
    format!("{}{}", front_matter(&doc.meta), doc.markdown)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn json(s: &str) -> Value {
        serde_json::from_str(s).expect("json de amostra")
    }

    #[test]
    fn host_da_publicacao_aceita_as_tres_formas() {
        assert_eq!(
            normalize_host("astralcodexten"),
            Some("astralcodexten.substack.com".into())
        );
        assert_eq!(
            normalize_host("https://astralcodexten.substack.com/p/algo?utm_source=x"),
            Some("astralcodexten.substack.com".into())
        );
        assert_eq!(
            normalize_host("www.thefp.com/"),
            Some("www.thefp.com".into())
        );
        assert_eq!(normalize_host("   "), None);
        assert_eq!(normalize_host("nome com espaço"), None);
    }

    #[test]
    fn assinaturas_expiradas_ficam_de_fora() {
        let v = json(
            r#"{
              "publications": [
                {"id": 1, "subdomain": "um", "custom_domain": null, "name": "Um"},
                {"id": 2, "subdomain": "dois", "custom_domain": "dois.com.br", "custom_domain_optional": false, "name": "Dois"},
                {"id": 3, "subdomain": "tres", "name": "Tres"},
                {"id": 4, "subdomain": "quatro", "custom_domain": "op.com", "custom_domain_optional": true, "name": "Quatro"}
              ],
              "subscriptions": [
                {"publication_id": 1, "membership_state": "free_signup"},
                {"publication_id": 2, "membership_state": "subscribed"},
                {"publication_id": 3, "membership_state": "expired"},
                {"publication_id": 4, "membership_state": "subscribed"}
              ]
            }"#,
        );
        let subs = parse_subscriptions(&v);
        assert_eq!(subs.len(), 3, "{:?}", subs);
        assert_eq!(subs[0].host, "um.substack.com");
        assert!(!subs[0].paid, "free_signup é assinatura grátis");
        assert_eq!(subs[1].host, "dois.com.br");
        assert!(subs[1].paid);
        // Domínio próprio opcional: o canônico continua sendo o substack.com.
        assert_eq!(subs[2].host, "quatro.substack.com");
        assert!(!subs.iter().any(|s| s.host.starts_with("tres")));
    }

    #[test]
    fn subscriptions_sem_publicacoes_nao_quebra() {
        assert!(parse_subscriptions(&json("{}")).is_empty());
        assert!(parse_subscriptions(&json("[]")).is_empty());
    }

    #[test]
    fn arquivo_vira_lista_de_posts() {
        let v = json(
            r#"[
              {"id": 1, "slug": "um", "title": "Um", "post_date": "2026-09-08T12:04:21.658Z",
               "audience": "everyone", "canonical_url": "https://x.com/p/um"},
              {"id": 2, "title": "Sem slug", "post_date": "2026-09-07T00:00:00.000Z"},
              {"id": 3, "slug": "dois", "title": "Dois", "post_date": "2026-09-01T00:00:00.000Z",
               "audience": "only_paid", "canonical_url": "https://x.com/p/dois"}
            ]"#,
        );
        let items = parse_archive(&v);
        assert_eq!(items.len(), 2, "post sem slug é descartado");
        assert_eq!(items[0].slug, "um");
        assert_eq!(items[1].audience, "only_paid");
        assert_eq!(day_of(&items[0].date), "2026-09-08");
    }

    #[test]
    fn paginacao_gera_os_offsets_certos() {
        assert_eq!(offsets(12, 12), vec![0]);
        assert_eq!(offsets(30, 12), vec![0, 12, 24]);
        assert_eq!(offsets(1, 12), vec![0]);
        let tudo = offsets(0, 12);
        assert_eq!(&tudo[..3], &[0, 12, 24]);
        assert!(tudo.len() > 100, "sem teto, o chamador é quem para");
        assert_eq!(
            archive_url("x.substack.com", 24, 12),
            "https://x.substack.com/api/v1/archive?sort=new&search=&offset=24&limit=12"
        );
        assert_eq!(
            post_url("x.substack.com", "um"),
            "https://x.substack.com/api/v1/posts/um"
        );
    }

    #[test]
    fn post_vira_documento_com_front_matter() {
        let v = json(
            r#"{
              "slug": "um", "title": "Um post", "subtitle": "O subtítulo",
              "post_date": "2026-09-08T12:04:21.658Z", "audience": "everyone",
              "canonical_url": "https://x.substack.com/p/um", "wordcount": 4695,
              "publishedBylines": [{"name": "Scott Alexander"}, {"name": "Outro"}],
              "postTags": [{"name": "IA", "slug": "ia"}, {"slug": "so-slug"}],
              "body_html": "<p>Olá <strong>mundo</strong>.</p><div class=\"subscription-widget-wrap\"><p>Assine</p></div>"
            }"#,
        );
        let doc = parse_post(&v, "Astral Codex Ten");
        assert!(!doc.locked);
        assert_eq!(doc.meta.author, "Scott Alexander, Outro");
        assert_eq!(doc.meta.tags, vec!["IA", "so-slug"]);
        assert_eq!(doc.meta.words, Some(4695));
        assert_eq!(doc.meta.subtitle.as_deref(), Some("O subtítulo"));
        assert!(doc.markdown.contains("**mundo**"), "{}", doc.markdown);
        assert!(!doc.markdown.contains("Assine"), "widget não entra");
        let full = preview(&doc);
        assert!(full.starts_with("---\ntitle: \"Um post\""), "{}", full);
        assert!(full.contains("publication: \"Astral Codex Ten\""));
        assert_eq!(
            file_stem(&doc.meta.date, &doc.meta.title, &doc.slug),
            "2026-09-08-um"
        );
    }

    #[test]
    fn post_pago_sem_corpo_e_marcado_como_bloqueado() {
        let v = json(
            r#"{"slug":"pago","title":"Pago","audience":"only_paid",
                "post_date":"2026-09-08T00:00:00.000Z","body_html":"",
                "truncated_body_text":"Comece a ler..."}"#,
        );
        let doc = parse_post(&v, "Boletim");
        assert!(doc.locked, "sem body_html a conta não tem acesso");
        assert!(doc.markdown.is_empty());
        assert!(doc.html.is_none());
        assert_eq!(doc.meta.audience.as_deref(), Some("only_paid"));
    }

    #[tokio::test]
    #[ignore = "rede: le o arquivo publico de uma publicacao real do Substack"]
    async fn rede_arquivo_publico_responde() {
        let f = Fetcher::new(1200, None, &["substack.com".to_string()]).expect("cliente");
        let v = f
            .get_json(&archive_url("astralcodexten.substack.com", 0, 2))
            .await
            .expect("arquivo");
        let items = parse_archive(&v);
        assert_eq!(items.len(), 2);
        let post = f
            .get_json(&post_url("astralcodexten.substack.com", &items[0].slug))
            .await
            .expect("post");
        let doc = parse_post(&post, "Astral Codex Ten");
        assert!(!doc.locked && doc.markdown.len() > 500);
    }

    #[tokio::test]
    #[ignore = "rede: exporta de ponta a ponta 2 posts publicos para uma pasta temporaria"]
    async fn rede_exporta_de_ponta_a_ponta() {
        let dir = crate::core::tools::temp_dir().join("teste-substack");
        let _ = std::fs::remove_dir_all(&dir);
        let opts = Options {
            publications: vec!["astralcodexten".into()],
            dest: dir.to_string_lossy().to_string(),
            limit: 2,
            markdown: true,
            html: true,
            json: true,
            ..Default::default()
        };
        let out = run(&opts, crate::core::tools::noop_progress())
            .await
            .expect("exportacao");
        assert_eq!(out.posts, 2, "{:?}", out.publications);
        assert_eq!(out.files.len(), 6, "md + html + json por post");
        let md = std::fs::read_to_string(
            out.files
                .iter()
                .find(|f| f.ends_with(".md"))
                .expect("um .md"),
        )
        .expect("le o markdown");
        assert!(md.starts_with("---\ntitle: \""), "{}", &md[..80]);
        assert!(md.contains("publication: "), "front-matter completo");
        assert!(md.len() > 800, "corpo de verdade");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    #[ignore = "sessao: lista as assinaturas da conta logada (precisa dos cookies de substack.com)"]
    async fn sessao_lista_assinaturas() {
        let path = std::env::var("OMNIGET_SUBSTACK_COOKIES").expect("OMNIGET_SUBSTACK_COOKIES");
        let content = std::fs::read_to_string(path).expect("arquivo netscape");
        let f = Fetcher::new(1200, Some(&content), &["substack.com".to_string()]).expect("cliente");
        assert!(f.has_session(), "nenhum cookie de substack.com no arquivo");
        let subs = discover(&f).await.expect("assinaturas");
        assert!(!subs.is_empty());
    }
}

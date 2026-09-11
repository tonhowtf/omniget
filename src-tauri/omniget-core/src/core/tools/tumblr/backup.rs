//! `tumblr-blog-backup`: espelho offline de um blog do Tumblr.
//!
//! Modelo de dados na linha do TumblThreeApp/TumblThree (MIT): o post tem um
//! `trail`, que é a cadeia de reblog do mais antigo (raiz) para o mais novo.
//! Renderizar essa cadeia como citação aninhada é o que faz o backup parecer
//! o Tumblr e não uma lista de imagens soltas.
//!
//! A saída é HTML local navegável: índice paginado, uma página por post e uma
//! por tag, com os links de mídia reescritos para os arquivos baixados.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::Result;
use once_cell::sync::Lazy;
use regex::Regex;
use serde::{Deserialize, Serialize};

use super::gdl::{self, Entry};
use super::likes::{index_dir, LocalIndex};
use super::{esc, safe_slug};

pub const TOOL_ID: &str = "tumblr-blog-backup";

#[derive(Debug, Clone, Deserialize)]
pub struct Options {
    /// Nome do blog (`meublog`) ou a URL dele.
    pub blog: String,
    /// Pasta raiz do espelho (uma subpasta por blog é criada dentro).
    pub out_dir: String,
    #[serde(default = "yes")]
    pub download_media: bool,
    #[serde(default)]
    pub limit: Option<u64>,
    #[serde(default = "fifty")]
    pub per_page: usize,
    #[serde(default)]
    pub session_netscape: Option<String>,
    #[serde(default)]
    pub account_slug: Option<String>,
}

fn yes() -> bool {
    true
}
fn fifty() -> usize {
    50
}

#[derive(Debug, Clone, PartialEq)]
pub struct TrailItem {
    pub blog: String,
    pub post_id: String,
    pub content: String,
    pub is_root: bool,
    pub is_current: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Post {
    pub id: String,
    pub blog: String,
    pub date: String,
    pub timestamp: i64,
    pub kind: String,
    pub title: String,
    pub summary: String,
    pub tags: Vec<String>,
    pub url: String,
    pub body: String,
    pub trail: Vec<TrailItem>,
    pub media: Vec<String>,
    pub note_count: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct BackupResult {
    pub blog: String,
    pub posts: u64,
    pub pages: u64,
    pub tags: u64,
    pub media_files: u64,
    pub used_session: bool,
    pub dir: String,
    pub index_path: String,
}

/// `meublog` → URL do blog. URL completa passa direto.
pub fn blog_url(input: &str) -> String {
    let s = input.trim().trim_end_matches('/');
    if s.is_empty() {
        return String::new();
    }
    if s.contains("://") {
        return s.to_string();
    }
    if s.contains(".tumblr.com") {
        return format!("https://{}", s);
    }
    format!("https://{}.tumblr.com/", s.trim_start_matches('@'))
}

// ───────────────────────── sanitização ─────────────────────────

/// Tags que executam ou puxam recurso de fora; nenhuma delas tem razão de
/// existir num espelho local.
const DANGER: &[&str] = &[
    "script", "style", "iframe", "object", "embed", "form", "link", "meta",
];

static BLOCKS: Lazy<Regex> = Lazy::new(|| {
    // A crate `regex` não tem retrovisor, então a alternância é montada com o
    // par de tags escrito por extenso.
    let alt = DANGER
        .iter()
        .map(|t| format!(r"<\s*{0}\b[^>]*>.*?<\s*/\s*{0}\s*>", t))
        .collect::<Vec<_>>()
        .join("|");
    Regex::new(&format!("(?is){}", alt)).expect("regex de blocos perigosos")
});
static VOIDS: Lazy<Regex> = Lazy::new(|| {
    Regex::new(&format!(r"(?is)<\s*/?\s*(?:{})\b[^>]*>", DANGER.join("|")))
        .expect("regex de tags perigosas soltas")
});
static HANDLERS: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r#"(?is)\s+on[a-z]+\s*=\s*(?:"[^"]*"|'[^']*'|[^\s>]+)"#)
        .expect("regex de handler inline")
});
static JS_URL: Lazy<Regex> = Lazy::new(|| {
    Regex::new(
        r#"(?is)(href|src|action)\s*=\s*(?:"|')?\s*(?:javascript|data|vbscript):[^"'>\s]*(?:"|')?"#,
    )
    .expect("regex de URL de script")
});

/// O corpo do post é HTML que veio do Tumblr; escapar tudo mataria o espelho.
/// Então some com o que executa (script, iframe, `onerror=`, `javascript:`) e
/// deixa o resto passar. Texto curto (título, tag, blog) usa `esc`, não isto.
pub fn sanitize_html(input: &str) -> String {
    let step = BLOCKS.replace_all(input, "");
    let step = VOIDS.replace_all(&step, "");
    let step = HANDLERS.replace_all(&step, "");
    JS_URL.replace_all(&step, r#"${1}="""#).to_string()
}

// ───────────────────────── leitura do dump ─────────────────────────

fn trail_of(meta: &serde_json::Value) -> Vec<TrailItem> {
    let Some(items) = gdl::dig(meta, "trail").and_then(|v| v.as_array()) else {
        return Vec::new();
    };
    items
        .iter()
        .map(|item| TrailItem {
            blog: gdl::dig(item, "blog.name")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
            post_id: match gdl::dig(item, "post.id") {
                Some(serde_json::Value::String(s)) => s.clone(),
                Some(serde_json::Value::Number(n)) => n.to_string(),
                _ => String::new(),
            },
            content: gdl::dig(item, "content_raw")
                .or_else(|| gdl::dig(item, "content"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
            is_root: gdl::dig(item, "is_root_item")
                .and_then(|v| v.as_bool())
                .unwrap_or(false),
            is_current: gdl::dig(item, "is_current_item")
                .and_then(|v| v.as_bool())
                .unwrap_or(false),
        })
        .collect()
}

/// O corpo depende do tipo do post: foto tem `caption`, texto tem `body`,
/// citação tem `text` + `source`, pergunta tem `question` + `answer`.
fn body_of(e: &Entry) -> String {
    let mut parts: Vec<String> = Vec::new();
    let mut push = |path: &str, wrap: Option<&str>| {
        let v = e.text(path);
        if v.trim().is_empty() {
            return;
        }
        match wrap {
            Some(tag) => parts.push(format!("<{0}>{1}</{0}>", tag, v)),
            None => parts.push(v),
        }
    };
    push("question", Some("blockquote"));
    push("answer", None);
    push("text", Some("blockquote"));
    push("source", None);
    push("body", None);
    push("caption", None);
    push("description", None);
    parts.join("\n")
}

/// Uma linha por post: as várias mídias de um mesmo post entram juntas, e a
/// ordem do dump (o Tumblr entrega do mais novo para o mais velho) é mantida.
pub fn parse_posts(entries: &[Entry]) -> Vec<Post> {
    let mut order: Vec<String> = Vec::new();
    let mut posts: HashMap<String, Post> = HashMap::new();
    for e in entries.iter().filter(|e| e.is_url()) {
        let id = e.first_text(&["id", "post.id", "post_id"]);
        let key = if id.is_empty() {
            e.url.clone().unwrap_or_default()
        } else {
            id.clone()
        };
        if key.is_empty() {
            continue;
        }
        let post = posts.entry(key.clone()).or_insert_with(|| {
            order.push(key.clone());
            Post {
                id: id.clone(),
                blog: e.first_text(&["blog_name", "blog.name"]),
                date: e.first_text(&["date", "timestamp"]),
                timestamp: e.number("timestamp").unwrap_or(0),
                kind: e.first_text(&["type", "post_type"]),
                title: e.first_text(&["title"]),
                summary: e.first_text(&["summary"]),
                tags: e.tags("tags"),
                url: e.first_text(&["post_url", "short_url", "url"]),
                body: body_of(e),
                trail: trail_of(&e.meta),
                media: Vec::new(),
                note_count: e.number("note_count").unwrap_or(0),
            }
        });
        if let Some(url) = e.url.as_ref().filter(|u| !u.is_empty()) {
            if !post.media.iter().any(|m| m == url) {
                post.media.push(url.clone());
            }
        }
    }
    order.into_iter().filter_map(|k| posts.remove(&k)).collect()
}

// ───────────────────────── render ─────────────────────────

/// Cadeia de reblog como citação aninhada: a raiz fica no nível mais fundo e
/// o item atual, do lado de fora — igual ao Tumblr.
pub fn render_trail(items: &[TrailItem], depth: &str, media: &HashMap<String, String>) -> String {
    let mut inner = String::new();
    for item in items {
        let name = if item.blog.is_empty() {
            "anônimo".to_string()
        } else {
            item.blog.clone()
        };
        let link = if item.blog.is_empty() {
            esc(&name)
        } else {
            format!(
                r#"<a href="https://{0}.tumblr.com/" rel="noopener">{1}</a>"#,
                esc(&item.blog),
                esc(&name)
            )
        };
        let content = rewrite_media(&sanitize_html(&item.content), media, depth);
        inner = format!(
            r#"<blockquote class="tr"{root}>{inner}<div class="who">{link}</div><div class="c">{content}</div></blockquote>"#,
            root = if item.is_root {
                r#" data-root="1""#
            } else {
                ""
            },
            inner = inner,
            link = link,
            content = content,
        );
    }
    inner
}

/// Troca as URLs remotas pelos arquivos locais já baixados. `map` guarda o
/// caminho relativo à raiz do espelho; `depth` é o prefixo da página atual.
pub fn rewrite_media(html: &str, map: &HashMap<String, String>, depth: &str) -> String {
    let mut out = html.to_string();
    for (url, local) in map {
        if url.is_empty() || !out.contains(url.as_str()) {
            continue;
        }
        out = out.replace(url.as_str(), &format!("{}{}", depth, local));
    }
    out
}

const CSS: &str = "\
:root{color-scheme:light dark;--bg:#fafafa;--fg:#111;--dim:#6b6b70;--card:#fff;--line:#e5e5e7;--accent:#00b8ff}\
@media(prefers-color-scheme:dark){:root{--bg:#0f0f10;--fg:#ececf0;--dim:#98989f;--card:#1c1c1e;--line:#2c2c2e}}\
*{box-sizing:border-box}body{margin:0;background:var(--bg);color:var(--fg);font:15px/1.55 -apple-system,BlinkMacSystemFont,\"Segoe UI\",Roboto,sans-serif}\
a{color:var(--accent);text-decoration:none}a:hover{text-decoration:underline}\
header{position:sticky;top:0;z-index:2;padding:14px 20px;border-bottom:1px solid var(--line);background:var(--bg);display:flex;gap:12px;align-items:baseline;flex-wrap:wrap}\
header h1{font-size:19px;margin:0}header .sub{color:var(--dim);font-size:13px;margin-right:auto}\
main{max-width:720px;margin:0 auto;padding:20px}\
article{background:var(--card);border:1px solid var(--line);border-radius:16px;padding:16px 18px;margin:0 0 16px}\
article h2{font-size:16px;margin:0 0 4px}.meta{color:var(--dim);font-size:12px;margin-bottom:8px}\
img,video{max-width:100%;height:auto;border-radius:10px;display:block;margin:8px 0}\
.tags{display:flex;flex-wrap:wrap;gap:6px;margin-top:10px}\
.tags a{font-size:12px;padding:2px 9px;border-radius:999px;background:var(--line);color:var(--fg)}\
blockquote.tr{margin:10px 0;padding:10px 12px;border-left:3px solid var(--line);border-radius:8px;background:color-mix(in srgb,var(--line) 30%,transparent)}\
blockquote.tr .who{font-size:12px;font-weight:600;color:var(--dim);margin-bottom:4px}\
nav.pg{display:flex;gap:10px;justify-content:center;padding:10px 0 30px}\
nav.pg a,nav.pg span{padding:6px 12px;border:1px solid var(--line);border-radius:999px;font-size:13px}\
nav.pg span{color:var(--dim)}\
.empty{color:var(--dim);font-style:italic}";

fn shell(title: &str, subtitle: &str, depth: &str, body: &str) -> String {
    format!(
        "<!doctype html>\n<html lang=\"pt\"><head><meta charset=\"utf-8\">\
<meta name=\"viewport\" content=\"width=device-width,initial-scale=1\">\
<title>{title}</title><style>{css}</style></head><body>\
<header><h1><a href=\"{depth}index.html\">{title}</a></h1><span class=\"sub\">{sub}</span>\
<a href=\"{depth}tags/index.html\">tags</a></header>\n<main>\n{body}</main></body></html>\n",
        title = esc(title),
        sub = esc(subtitle),
        css = CSS,
        depth = depth,
        body = body,
    )
}

fn media_html(post: &Post, map: &HashMap<String, String>, depth: &str) -> String {
    let mut out = String::new();
    for url in &post.media {
        let src = match map.get(url) {
            Some(local) => format!("{}{}", depth, local),
            None => url.clone(),
        };
        let lower = src.to_lowercase();
        if lower.ends_with(".mp4") || lower.ends_with(".webm") || lower.ends_with(".mov") {
            out.push_str(&format!(
                r#"<video src="{}" controls playsinline preload="metadata"></video>"#,
                esc(&src)
            ));
        } else if lower.ends_with(".mp3") || lower.ends_with(".m4a") || lower.ends_with(".ogg") {
            out.push_str(&format!(r#"<audio src="{}" controls></audio>"#, esc(&src)));
        } else {
            out.push_str(&format!(
                r#"<img src="{}" alt="{}" loading="lazy">"#,
                esc(&src),
                esc(&post.summary)
            ));
        }
    }
    out
}

fn tag_links(post: &Post, slugs: &HashMap<String, String>, depth: &str) -> String {
    if post.tags.is_empty() {
        return String::new();
    }
    let items: String = post
        .tags
        .iter()
        .map(|t| {
            let slug = slugs.get(t).cloned().unwrap_or_else(|| safe_slug(t));
            format!(
                r#"<a href="{}tags/{}.html">#{}</a>"#,
                depth,
                esc(&slug),
                esc(t)
            )
        })
        .collect();
    format!(r#"<div class="tags">{}</div>"#, items)
}

fn article(
    post: &Post,
    map: &HashMap<String, String>,
    slugs: &HashMap<String, String>,
    depth: &str,
    full: bool,
) -> String {
    let head = if post.title.trim().is_empty() {
        format!("{} · {}", esc(&post.kind), esc(&post.blog))
    } else {
        esc(&post.title)
    };
    let body = if full {
        let mut b = rewrite_media(&sanitize_html(&post.body), map, depth);
        if !post.trail.is_empty() {
            b.push_str(&render_trail(&post.trail, depth, map));
        }
        b
    } else {
        let s = post.summary.trim();
        if s.is_empty() {
            r#"<span class="empty">sem resumo</span>"#.to_string()
        } else {
            format!("<p>{}</p>", esc(s))
        }
    };
    format!(
        "<article id=\"p{id}\"><h2><a href=\"{depth}posts/{id}.html\">{head}</a></h2>\
<div class=\"meta\">{date} · {notes} notas · <a href=\"{url}\" rel=\"noopener\">original</a></div>\
{media}{body}{tags}</article>\n",
        id = esc(&post.id),
        depth = depth,
        head = head,
        date = esc(&post.date),
        notes = post.note_count,
        url = esc(&post.url),
        media = media_html(post, map, depth),
        body = body,
        tags = tag_links(post, slugs, depth),
    )
}

/// Nome de arquivo por página: a 1 é o `index.html`.
pub fn page_name(page: usize) -> String {
    if page <= 1 {
        "index.html".to_string()
    } else {
        format!("page-{}.html", page)
    }
}

pub fn page_count(total: usize, per_page: usize) -> usize {
    let per = per_page.max(1);
    if total == 0 {
        1
    } else {
        total.div_ceil(per)
    }
}

fn pager(page: usize, pages: usize, depth: &str) -> String {
    if pages <= 1 {
        return String::new();
    }
    let prev = if page > 1 {
        format!(
            r#"<a href="{}{}">← anteriores</a>"#,
            depth,
            page_name(page - 1)
        )
    } else {
        String::new()
    };
    let next = if page < pages {
        format!(
            r#"<a href="{}{}">próximas →</a>"#,
            depth,
            page_name(page + 1)
        )
    } else {
        String::new()
    };
    format!(
        r#"<nav class="pg">{}<span>{} / {}</span>{}</nav>"#,
        prev, page, pages, next
    )
}

/// Tag → nome de arquivo, resolvendo colisão (`arte` e `Arte!` viram slugs
/// iguais) com sufixo numérico.
pub fn slug_map(tags: &[String]) -> HashMap<String, String> {
    let mut used: HashMap<String, usize> = HashMap::new();
    let mut out: HashMap<String, String> = HashMap::new();
    let mut ordered: Vec<&String> = tags.iter().collect();
    ordered.sort();
    ordered.dedup();
    for tag in ordered {
        let base = safe_slug(tag);
        let n = used.entry(base.clone()).or_insert(0);
        *n += 1;
        let slug = if *n == 1 {
            base
        } else {
            format!("{}-{}", base, n)
        };
        out.insert(tag.clone(), slug);
    }
    out
}

// ───────────────────────── escrita ─────────────────────────

fn rel_to_root(root: &Path, file: &str) -> String {
    let root_str = root.to_string_lossy().to_string();
    let trimmed = root_str.trim_end_matches(['/', '\\']);
    match file.strip_prefix(trimmed) {
        Some(rest) => rest.trim_start_matches(['/', '\\']).replace('\\', "/"),
        None => file.to_string(),
    }
}

fn media_map(posts: &[Post], index: &LocalIndex, root: &Path) -> HashMap<String, String> {
    let mut map = HashMap::new();
    if index.is_empty() {
        return map;
    }
    for post in posts {
        for url in &post.media {
            if let Some(local) = index.file_for_url(url) {
                map.insert(url.clone(), rel_to_root(root, &local));
            }
        }
    }
    map
}

pub async fn run(opts: &Options, progress: &super::super::ProgressFn) -> Result<BackupResult> {
    let url = blog_url(&opts.blog);
    if url.is_empty() {
        anyhow::bail!("informe o blog");
    }
    let cookies = match opts.session_netscape.as_deref() {
        Some(s) => gdl::write_cookies(s, &["tumblr.com"])?,
        None => None,
    };
    let used_session = cookies.is_some();
    let cookie_path = cookies.as_ref().map(|c| c.path.clone());

    let extra = vec![
        "-o".to_string(),
        "extractor.tumblr.posts=all".to_string(),
        "-o".to_string(),
        "extractor.tumblr.reblogs=true".to_string(),
        "-o".to_string(),
        "extractor.tumblr.original=true".to_string(),
    ];

    super::super::report(progress, TOOL_ID, "started", 0, None, Some(url.clone()));
    let entries = gdl::dump(
        &url,
        cookie_path.as_deref(),
        opts.limit,
        &extra,
        progress,
        TOOL_ID,
    )
    .await?;
    let posts = parse_posts(&entries);
    if posts.is_empty() {
        anyhow::bail!("nenhum post foi lido desse blog");
    }

    let blog_name = posts
        .iter()
        .find(|p| !p.blog.is_empty())
        .map(|p| p.blog.clone())
        .unwrap_or_else(|| safe_slug(&opts.blog));
    let root = PathBuf::from(&opts.out_dir).join(safe_slug(&blog_name));
    let media_dir = root.join("media");
    std::fs::create_dir_all(root.join("posts"))?;
    std::fs::create_dir_all(root.join("tags"))?;

    let mut media_files = 0u64;
    if opts.download_media {
        let out = gdl::download(
            &url,
            &media_dir,
            cookie_path.as_deref(),
            opts.limit,
            &extra,
            progress,
            TOOL_ID,
        )
        .await?;
        media_files = out.files.len() as u64;
    }
    let index = if media_dir.exists() {
        index_dir(&media_dir)
    } else {
        LocalIndex::new()
    };
    let map = media_map(&posts, &index, &root);

    let all_tags: Vec<String> = posts.iter().flat_map(|p| p.tags.clone()).collect();
    let slugs = slug_map(&all_tags);

    // Índice paginado.
    let per_page = opts.per_page.max(1);
    let pages = page_count(posts.len(), per_page);
    for page in 1..=pages {
        let slice = &posts[(page - 1) * per_page..(page * per_page).min(posts.len())];
        let mut body = String::new();
        for post in slice {
            body.push_str(&article(post, &map, &slugs, "", false));
        }
        body.push_str(&pager(page, pages, ""));
        let html = shell(
            &blog_name,
            &format!("{} posts · espelho local", posts.len()),
            "",
            &body,
        );
        std::fs::write(root.join(page_name(page)), html)?;
    }

    // Uma página por post.
    for (i, post) in posts.iter().enumerate() {
        if i % 25 == 0 {
            super::super::report(
                progress,
                TOOL_ID,
                "progress",
                i as u64,
                Some(posts.len() as u64),
                Some(post.id.clone()),
            );
        }
        let body = article(post, &map, &slugs, "../", true);
        let html = shell(&blog_name, &post.date, "../", &body);
        let name = format!("{}.html", safe_slug(&post.id));
        std::fs::write(root.join("posts").join(name), html)?;
    }

    // Uma página por tag, mais o índice de tags.
    let mut by_tag: HashMap<String, Vec<&Post>> = HashMap::new();
    for post in &posts {
        for tag in &post.tags {
            by_tag.entry(tag.clone()).or_default().push(post);
        }
    }
    for (tag, list) in &by_tag {
        let slug = slugs.get(tag).cloned().unwrap_or_else(|| safe_slug(tag));
        let mut body = format!("<h2>#{}</h2>\n", esc(tag));
        for post in list {
            body.push_str(&article(post, &map, &slugs, "../", false));
        }
        let html = shell(&blog_name, &format!("#{}", tag), "../", &body);
        std::fs::write(root.join("tags").join(format!("{}.html", slug)), html)?;
    }
    let mut tag_names: Vec<&String> = by_tag.keys().collect();
    tag_names.sort();
    let list: String = tag_names
        .iter()
        .map(|t| {
            let slug = slugs.get(*t).cloned().unwrap_or_else(|| safe_slug(t));
            let n = by_tag.get(*t).map(|v| v.len()).unwrap_or(0);
            format!(r#"<a href="{}.html">#{} ({})</a>"#, esc(&slug), esc(t), n)
        })
        .collect();
    let tags_index = shell(
        &blog_name,
        &format!("{} tags", tag_names.len()),
        "../",
        &format!(r#"<div class="tags">{}</div>"#, list),
    );
    std::fs::write(root.join("tags").join("index.html"), tags_index)?;

    let index_path = root.join("index.html");
    super::super::report(
        progress,
        TOOL_ID,
        "done",
        posts.len() as u64,
        Some(posts.len() as u64),
        None,
    );
    Ok(BackupResult {
        blog: blog_name,
        posts: posts.len() as u64,
        pages: pages as u64,
        tags: by_tag.len() as u64,
        media_files,
        used_session,
        dir: root.to_string_lossy().to_string(),
        index_path: index_path.to_string_lossy().to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const DUMP: &str = r#"[
      [3, "https://64.media.tumblr.com/aa/foto_1280.jpg",
        {"id": 111, "blog_name": "meu", "type": "photo", "date": "2024-01-01 09:00:00",
         "timestamp": 1704099600, "note_count": 9, "tags": ["arte", "Arte!"],
         "summary": "resumo", "caption": "<p>legenda</p>",
         "post_url": "https://meu.tumblr.com/post/111",
         "trail": [
           {"blog": {"name": "raiz"}, "post": {"id": "9"}, "content_raw": "<p>começou aqui</p>",
            "is_root_item": true, "is_current_item": false},
           {"blog": {"name": "meio"}, "post": {"id": "10"}, "content_raw": "<p>passou por aqui</p>",
            "is_root_item": false, "is_current_item": false},
           {"blog": {"name": "meu"}, "post": {"id": "111"}, "content_raw": "<p>e cheguei</p>",
            "is_root_item": false, "is_current_item": true}
         ]}],
      [3, "https://64.media.tumblr.com/bb/foto2_1280.jpg",
        {"id": 111, "blog_name": "meu", "type": "photo", "date": "2024-01-01 09:00:00",
         "timestamp": 1704099600, "note_count": 9, "tags": ["arte", "Arte!"],
         "summary": "resumo", "post_url": "https://meu.tumblr.com/post/111"}],
      [3, "https://meu.tumblr.com/post/222",
        {"id": 222, "blog_name": "meu", "type": "text", "date": "2023-12-31 20:00:00",
         "timestamp": 1704063600, "note_count": 0, "tags": [],
         "title": "Ano novo", "body": "<p>texto</p>",
         "post_url": "https://meu.tumblr.com/post/222"}]
    ]"#;

    fn posts() -> Vec<Post> {
        let entries = gdl::parse_dump(DUMP).expect("dump válido");
        parse_posts(&entries)
    }

    #[test]
    fn posts_agrupam_midia_e_mantem_a_ordem() {
        let posts = posts();
        assert_eq!(posts.len(), 2);
        assert_eq!(posts[0].id, "111");
        assert_eq!(posts[0].media.len(), 2);
        assert_eq!(posts[0].trail.len(), 3);
        assert_eq!(posts[0].body, "<p>legenda</p>");
        assert_eq!(posts[1].title, "Ano novo");
        assert_eq!(posts[1].body, "<p>texto</p>");
    }

    #[test]
    fn a_cadeia_de_reblog_fica_aninhada_com_a_raiz_no_fundo() {
        let posts = posts();
        let html = render_trail(&posts[0].trail, "", &HashMap::new());
        assert_eq!(
            html.matches("<blockquote").count(),
            3,
            "um nível por elo da cadeia"
        );
        let raiz = html.find("começou aqui").expect("raiz presente");
        let meio = html.find("passou por aqui").expect("meio presente");
        let fim = html.find("e cheguei").expect("atual presente");
        assert!(raiz < meio && meio < fim, "raiz é o nível mais interno");
        assert!(html.contains(r#"data-root="1""#));
        // O elo mais externo é o item atual.
        assert!(html.starts_with("<blockquote"));
        assert!(html.ends_with("</blockquote>"));
    }

    #[test]
    fn trail_vazio_nao_gera_citacao() {
        assert_eq!(render_trail(&[], "", &HashMap::new()), "");
    }

    #[test]
    fn sanitize_tira_script_iframe_e_handler_mas_mantem_o_post() {
        let sujo = r#"<p>oi</p><script>alert(1)</script><img src="x.jpg" onerror="roubar()">
<iframe src="https://mal.com"></iframe><a href="javascript:alert(2)">clique</a>"#;
        let limpo = sanitize_html(sujo);
        assert!(limpo.contains("<p>oi</p>"), "o post continua lá");
        assert!(limpo.contains(r#"<img src="x.jpg""#));
        assert!(!limpo.contains("alert(1)"));
        assert!(!limpo.to_lowercase().contains("<script"));
        assert!(!limpo.to_lowercase().contains("<iframe"));
        assert!(!limpo.to_lowercase().contains("onerror"));
        assert!(!limpo.to_lowercase().contains("javascript:"));
    }

    #[test]
    fn midia_e_reescrita_para_o_arquivo_local_com_o_prefixo_da_pagina() {
        let mut map = HashMap::new();
        map.insert(
            "https://64.media.tumblr.com/aa/foto_1280.jpg".to_string(),
            "media/meu/foto_1280.jpg".to_string(),
        );
        let html = r#"<img src="https://64.media.tumblr.com/aa/foto_1280.jpg">"#;
        assert_eq!(
            rewrite_media(html, &map, "../"),
            r#"<img src="../media/meu/foto_1280.jpg">"#
        );
        assert_eq!(
            rewrite_media(html, &map, ""),
            r#"<img src="media/meu/foto_1280.jpg">"#
        );
        assert_eq!(
            rewrite_media(r#"<img src="https://outro/x.jpg">"#, &map, ""),
            r#"<img src="https://outro/x.jpg">"#,
            "o que não baixou continua apontando para a web"
        );
    }

    #[test]
    fn tags_com_o_mesmo_slug_nao_se_sobrescrevem() {
        let map = slug_map(&["arte".into(), "Arte!".into(), "arte".into()]);
        assert_eq!(map.len(), 2);
        let mut slugs: Vec<&String> = map.values().collect();
        slugs.sort();
        assert_eq!(slugs, vec!["arte", "arte-2"]);
    }

    #[test]
    fn paginacao_conta_certo_e_a_primeira_pagina_e_o_index() {
        assert_eq!(page_count(0, 50), 1);
        assert_eq!(page_count(50, 50), 1);
        assert_eq!(page_count(51, 50), 2);
        assert_eq!(page_count(10, 0), 10, "per_page zero não divide por zero");
        assert_eq!(page_name(1), "index.html");
        assert_eq!(page_name(3), "page-3.html");
    }

    #[test]
    fn a_pagina_do_post_escapa_titulo_e_tag() {
        let mut posts = posts();
        posts[1].title = "<b>oi</b>".into();
        posts[1].tags = vec!["<script>".into()];
        let slugs = slug_map(&posts[1].tags);
        let html = article(&posts[1], &HashMap::new(), &slugs, "../", true);
        assert!(html.contains("&lt;b&gt;oi&lt;/b&gt;"));
        assert!(!html.contains("<b>oi</b>"));
        assert!(html.contains("#&lt;script&gt;"));
    }

    #[test]
    fn url_do_blog_aceita_nome_e_url() {
        assert_eq!(blog_url("meu"), "https://meu.tumblr.com/");
        assert_eq!(blog_url("@meu"), "https://meu.tumblr.com/");
        assert_eq!(blog_url("meu.tumblr.com"), "https://meu.tumblr.com");
        assert_eq!(
            blog_url("https://www.tumblr.com/meu"),
            "https://www.tumblr.com/meu"
        );
        assert_eq!(blog_url(" "), "");
    }

    #[test]
    fn caminho_relativo_a_raiz_do_espelho() {
        let root = Path::new("/Users/a/Espelho/meu");
        assert_eq!(
            rel_to_root(root, "/Users/a/Espelho/meu/media/tumblr/x.jpg"),
            "media/tumblr/x.jpg"
        );
        assert_eq!(rel_to_root(root, "/fora/x.jpg"), "/fora/x.jpg");
    }

    #[test]
    fn nome_de_arquivo_do_post_e_seguro() {
        assert_eq!(safe_slug("../../evil"), "evil");
        assert_eq!(safe_slug("700111222"), "700111222");
    }

    #[test]
    #[ignore = "rede: exige gallery-dl instalado e baixa um blog inteiro do Tumblr"]
    fn espelho_real_de_um_blog_pequeno() {
        let opts = Options {
            blog: "staff".into(),
            out_dir: std::env::temp_dir().to_string_lossy().to_string(),
            download_media: false,
            limit: Some(5),
            per_page: 5,
            session_netscape: None,
            account_slug: None,
        };
        let rt = tokio::runtime::Runtime::new().expect("runtime");
        let out = rt
            .block_on(run(&opts, &super::super::super::noop_progress()))
            .expect("espelho");
        assert!(out.posts > 0);
        assert!(Path::new(&out.index_path).exists());
    }
}

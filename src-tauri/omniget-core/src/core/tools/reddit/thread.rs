//! Backup de uma thread do Reddit: post + árvore de comentários, offline.
//!
//! Tudo sai do JSON público (`<permalink>.json`). Os galhos que a página
//! esconde atrás de "carregar mais" viram uma chamada a
//! `api/morechildren.json`; os "continuar esta conversa" viram um novo GET no
//! permalink do próprio nó. A saída é Markdown, HTML de arquivo único
//! (colapsa, expande e reordena sem servidor) e o JSON normalizado.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{branch_json_url, esc, fmt_utc, more_children_url, post_json_url, Fetcher, Target};
use crate::core::tools::{report, ProgressFn};

const ID: &str = "rd-thread";

// ───────────────────────── modelo ─────────────────────────

#[derive(Debug, Clone, Default, Serialize)]
pub struct Post {
    pub id: String,
    pub title: String,
    pub author: String,
    pub subreddit: String,
    pub created_utc: f64,
    pub created: String,
    pub score: i64,
    pub upvote_ratio: f64,
    pub num_comments: i64,
    pub permalink: String,
    pub url: String,
    pub selftext: String,
    pub flair: String,
    pub over_18: bool,
    pub locked: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct Comment {
    pub id: String,
    pub parent_id: String,
    pub author: String,
    pub body: String,
    pub score: Option<i64>,
    pub created_utc: f64,
    pub created: String,
    pub permalink: String,
    pub is_op: bool,
    pub distinguished: String,
    pub stickied: bool,
    pub replies: Vec<Comment>,
}

/// Um galho que o Reddit não mandou: ou uma lista de ids, ou o marcador de
/// "continuar esta conversa" (`id == "_"`, sem ids).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MoreRef {
    pub parent_id: String,
    pub children: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Thread {
    pub post: Post,
    pub comments: Vec<Comment>,
    pub total: usize,
    pub missing: usize,
    pub sorted_by: String,
}

// ───────────────────────── leitura do JSON ─────────────────────────

fn s(v: &Value, key: &str) -> String {
    v.get(key)
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .to_string()
}

fn i(v: &Value, key: &str) -> i64 {
    v.get(key).and_then(|x| x.as_i64()).unwrap_or(0)
}

fn f(v: &Value, key: &str) -> f64 {
    v.get(key).and_then(|x| x.as_f64()).unwrap_or(0.0)
}

fn b(v: &Value, key: &str) -> bool {
    v.get(key).and_then(|x| x.as_bool()).unwrap_or(false)
}

pub fn parse_post(data: &Value) -> Post {
    let created_utc = f(data, "created_utc");
    Post {
        id: s(data, "id"),
        title: s(data, "title"),
        author: s(data, "author"),
        subreddit: s(data, "subreddit"),
        created_utc,
        created: fmt_utc(created_utc),
        score: i(data, "score"),
        upvote_ratio: f(data, "upvote_ratio"),
        num_comments: i(data, "num_comments"),
        permalink: format!("https://www.reddit.com{}", s(data, "permalink")),
        url: s(data, "url"),
        selftext: s(data, "selftext"),
        flair: s(data, "link_flair_text"),
        over_18: b(data, "over_18"),
        locked: b(data, "locked"),
    }
}

fn parse_comment(data: &Value) -> Comment {
    let created_utc = f(data, "created_utc");
    let permalink = s(data, "permalink");
    Comment {
        id: s(data, "id"),
        parent_id: s(data, "parent_id"),
        author: s(data, "author"),
        body: s(data, "body"),
        score: if b(data, "score_hidden") {
            None
        } else {
            data.get("score").and_then(|x| x.as_i64())
        },
        created_utc,
        created: fmt_utc(created_utc),
        permalink: if permalink.is_empty() {
            String::new()
        } else {
            format!("https://www.reddit.com{}", permalink)
        },
        is_op: b(data, "is_submitter"),
        distinguished: s(data, "distinguished"),
        stickied: b(data, "stickied"),
        replies: Vec::new(),
    }
}

/// Achata `[{kind, data}, …]` — serve tanto para o `children` de um Listing
/// quanto para o `things` do morechildren.
pub fn parse_things(children: &Value, out: &mut Vec<Comment>, mores: &mut Vec<MoreRef>) {
    let Some(list) = children.as_array() else {
        return;
    };
    for child in list {
        let kind = child.get("kind").and_then(|k| k.as_str()).unwrap_or("");
        let Some(data) = child.get("data") else {
            continue;
        };
        match kind {
            "t1" => {
                out.push(parse_comment(data));
                if let Some(replies) = data.get("replies") {
                    if let Some(inner) = replies.pointer("/data/children") {
                        parse_things(inner, out, mores);
                    }
                }
            }
            "more" => {
                let children: Vec<String> = data
                    .get("children")
                    .and_then(|c| c.as_array())
                    .map(|a| {
                        a.iter()
                            .filter_map(|x| x.as_str())
                            .filter(|x| *x != "_")
                            .map(|x| x.to_string())
                            .collect()
                    })
                    .unwrap_or_default();
                let parent_id = s(data, "parent_id");
                if !parent_id.is_empty() {
                    mores.push(MoreRef {
                        parent_id,
                        children,
                    });
                }
            }
            _ => {}
        }
    }
}

/// A resposta de um permalink é `[listing do post, listing dos comentários]`.
pub fn parse_thread_json(root: &Value) -> Result<(Post, Vec<Comment>, Vec<MoreRef>)> {
    let arr = root
        .as_array()
        .ok_or_else(|| anyhow!("resposta inesperada do Reddit (não é a lista de dois listings)"))?;
    let post_data = arr
        .first()
        .and_then(|l| l.pointer("/data/children/0/data"))
        .ok_or_else(|| anyhow!("o post não veio na resposta (removido, privado ou apagado?)"))?;
    let post = parse_post(post_data);
    let mut flat = Vec::new();
    let mut mores = Vec::new();
    if let Some(children) = arr.get(1).and_then(|l| l.pointer("/data/children")) {
        parse_things(children, &mut flat, &mut mores);
    }
    Ok((post, flat, mores))
}

/// O `things` de `api/morechildren.json`.
pub fn parse_more_json(root: &Value) -> (Vec<Comment>, Vec<MoreRef>) {
    let mut flat = Vec::new();
    let mut mores = Vec::new();
    if let Some(things) = root
        .pointer("/json/data/things")
        .or_else(|| root.pointer("/data/things"))
    {
        parse_things(things, &mut flat, &mut mores);
    }
    (flat, mores)
}

// ───────────────────────── árvore ─────────────────────────

/// Monta a árvore a partir da lista achatada. Nós órfãos (o pai não veio)
/// entram na raiz para nada se perder.
pub fn build_tree(flat: Vec<Comment>, post_id: &str) -> Vec<Comment> {
    let root = format!("t3_{}", post_id);
    let mut map: HashMap<String, Comment> = HashMap::new();
    let mut order: Vec<String> = Vec::new();
    for c in flat {
        if c.id.is_empty() {
            continue;
        }
        let key = format!("t1_{}", c.id);
        if map.contains_key(&key) {
            continue;
        }
        order.push(key.clone());
        map.insert(key, c);
    }
    let mut kids: HashMap<String, Vec<String>> = HashMap::new();
    for key in &order {
        let parent = map
            .get(key)
            .map(|c| c.parent_id.clone())
            .unwrap_or_default();
        let parent = if parent == root || map.contains_key(&parent) {
            parent
        } else {
            root.clone()
        };
        kids.entry(parent).or_default().push(key.clone());
    }
    let mut built: HashMap<String, Comment> = HashMap::new();
    // De baixo para cima: a ordem inversa de descoberta garante que os filhos
    // já estão prontos quando o pai é montado.
    for key in order.iter().rev() {
        let child_keys = kids.remove(key).unwrap_or_default();
        let replies: Vec<Comment> = child_keys.iter().filter_map(|k| built.remove(k)).collect();
        if let Some(mut node) = map.remove(key) {
            node.replies = replies;
            built.insert(key.clone(), node);
        }
    }
    kids.remove(&root)
        .unwrap_or_default()
        .iter()
        .filter_map(|k| built.remove(k))
        .collect()
}

pub fn count_tree(list: &[Comment]) -> usize {
    list.iter().map(|c| 1 + count_tree(&c.replies)).sum()
}

/// `score` (mais votados primeiro), `new` (recentes primeiro) ou `old`.
pub fn sort_tree(list: &mut [Comment], by: &str) {
    match by {
        "new" => list.sort_by(|a, b| {
            b.created_utc
                .partial_cmp(&a.created_utc)
                .unwrap_or(std::cmp::Ordering::Equal)
        }),
        "old" => list.sort_by(|a, b| {
            a.created_utc
                .partial_cmp(&b.created_utc)
                .unwrap_or(std::cmp::Ordering::Equal)
        }),
        _ => list.sort_by(|a, b| {
            b.score
                .unwrap_or(i64::MIN)
                .cmp(&a.score.unwrap_or(i64::MIN))
                .then_with(|| {
                    a.created_utc
                        .partial_cmp(&b.created_utc)
                        .unwrap_or(std::cmp::Ordering::Equal)
                })
        }),
    }
    for c in list.iter_mut() {
        sort_tree(&mut c.replies, by);
    }
}

// ───────────────────────── Markdown ─────────────────────────

fn score_label(score: Option<i64>) -> String {
    match score {
        Some(n) => format!("{} pts", n),
        None => "pts ocultos".to_string(),
    }
}

fn md_comment(c: &Comment, depth: usize, out: &mut String) {
    let pad = "  ".repeat(depth);
    let mut tags = String::new();
    if c.is_op {
        tags.push_str(" **[OP]**");
    }
    if c.stickied {
        tags.push_str(" **[fixado]**");
    }
    if !c.distinguished.is_empty() {
        tags.push_str(&format!(" **[{}]**", c.distinguished));
    }
    out.push_str(&format!(
        "{}- **u/{}** · {} · {}{}\n",
        pad,
        c.author,
        score_label(c.score),
        c.created,
        tags
    ));
    let body_pad = format!("{}  ", pad);
    let body = if c.body.trim().is_empty() {
        "*(sem texto)*"
    } else {
        c.body.trim()
    };
    for line in body.lines() {
        out.push_str(&body_pad);
        out.push_str(line);
        out.push('\n');
    }
    out.push('\n');
    for r in &c.replies {
        md_comment(r, depth + 1, out);
    }
}

pub fn render_markdown(t: &Thread) -> String {
    let p = &t.post;
    let mut out = String::new();
    out.push_str(&format!("# {}\n\n", p.title));
    out.push_str(&format!(
        "r/{} · u/{} · {} pts · {} comentários · {}\n\n",
        p.subreddit, p.author, p.score, p.num_comments, p.created
    ));
    out.push_str(&format!("<{}>\n\n", p.permalink));
    if !p.flair.is_empty() {
        out.push_str(&format!("Flair: {}\n\n", p.flair));
    }
    if !p.url.is_empty() && !p.url.contains("/comments/") {
        out.push_str(&format!("Link: <{}>\n\n", p.url));
    }
    if !p.selftext.trim().is_empty() {
        out.push_str(p.selftext.trim());
        out.push_str("\n\n");
    }
    out.push_str("---\n\n");
    out.push_str(&format!("## Comentários ({})\n\n", t.total));
    for c in &t.comments {
        md_comment(c, 0, &mut out);
    }
    if t.missing > 0 {
        out.push_str(&format!(
            "\n> {} comentários não vieram (o Reddit cortou o galho).\n",
            t.missing
        ));
    }
    out
}

// ───────────────────────── HTML ─────────────────────────

/// Markdown do Reddit em HTML, no essencial: parágrafo, citação, bloco de
/// código, código curto, negrito, itálico, link e URL solta. Tudo escapado
/// antes — nada do que veio do Reddit vira marcação de verdade.
pub fn md_to_html(body: &str) -> String {
    let mut out = String::new();
    let mut in_code = false;
    let mut in_quote = false;
    let mut paragraph = false;
    let close = |out: &mut String, paragraph: &mut bool| {
        if *paragraph {
            out.push_str("</p>");
            *paragraph = false;
        }
    };
    for raw in body.replace("\r\n", "\n").lines() {
        let line = raw.trim_end();
        if line.trim_start().starts_with("```") {
            close(&mut out, &mut paragraph);
            if in_code {
                out.push_str("</pre>");
            } else {
                out.push_str("<pre>");
            }
            in_code = !in_code;
            continue;
        }
        if in_code {
            out.push_str(&esc(line));
            out.push('\n');
            continue;
        }
        let quoted = line.trim_start().starts_with('>');
        if quoted && !in_quote {
            close(&mut out, &mut paragraph);
            out.push_str("<blockquote>");
            in_quote = true;
        }
        if !quoted && in_quote {
            close(&mut out, &mut paragraph);
            out.push_str("</blockquote>");
            in_quote = false;
        }
        let text = if quoted {
            line.trim_start().trim_start_matches('>').trim_start()
        } else {
            line
        };
        if text.trim().is_empty() {
            close(&mut out, &mut paragraph);
            continue;
        }
        if !paragraph {
            out.push_str("<p>");
            paragraph = true;
        } else {
            out.push_str("<br>");
        }
        out.push_str(&inline_md(text));
    }
    close(&mut out, &mut paragraph);
    if in_quote {
        out.push_str("</blockquote>");
    }
    if in_code {
        out.push_str("</pre>");
    }
    out
}

fn inline_md(text: &str) -> String {
    let escaped = esc(text);
    let with_links = link_md(&escaped);
    let with_code = wrap_pairs(&with_links, '`', "<code>", "</code>");
    let with_bold = wrap_double(&with_code, "**", "<strong>", "</strong>");
    wrap_pairs(&with_bold, '*', "<em>", "</em>")
}

/// `[texto](url)` e URL solta viram âncora.
fn link_md(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let chars: Vec<char> = s.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '[' {
            if let Some((label, href, next)) = read_link(&chars, i) {
                out.push_str(&format!(
                    "<a href=\"{}\" rel=\"noreferrer\">{}</a>",
                    href, label
                ));
                i = next;
                continue;
            }
        }
        if chars[i] == 'h' && s[byte_at(&chars, i)..].starts_with("http") {
            let rest: String = chars[i..]
                .iter()
                .take_while(|c| !c.is_whitespace() && **c != ')' && **c != '<')
                .collect();
            if rest.starts_with("http://") || rest.starts_with("https://") {
                let trimmed = rest.trim_end_matches(['.', ',', ';', ':']);
                out.push_str(&format!(
                    "<a href=\"{}\" rel=\"noreferrer\">{}</a>",
                    trimmed, trimmed
                ));
                i += trimmed.chars().count();
                continue;
            }
        }
        out.push(chars[i]);
        i += 1;
    }
    out
}

fn byte_at(chars: &[char], idx: usize) -> usize {
    chars.iter().take(idx).map(|c| c.len_utf8()).sum()
}

fn read_link(chars: &[char], start: usize) -> Option<(String, String, usize)> {
    let close = chars.iter().skip(start).position(|c| *c == ']')? + start;
    if chars.get(close + 1) != Some(&'(') {
        return None;
    }
    let end = chars.iter().skip(close + 2).position(|c| *c == ')')? + close + 2;
    let label: String = chars[start + 1..close].iter().collect();
    let href: String = chars[close + 2..end].iter().collect();
    if href.is_empty() || href.contains(' ') {
        return None;
    }
    Some((label, href, end + 1))
}

/// Troca pares de um mesmo caractere pelo par de tags.
fn wrap_pairs(s: &str, marker: char, open: &str, close: &str) -> String {
    let count = s.chars().filter(|c| *c == marker).count();
    if count < 2 {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let mut open_now = true;
    let mut left = count - count % 2;
    for c in s.chars() {
        if c == marker && left > 0 {
            out.push_str(if open_now { open } else { close });
            open_now = !open_now;
            left -= 1;
        } else {
            out.push(c);
        }
    }
    out
}

fn wrap_double(s: &str, marker: &str, open: &str, close: &str) -> String {
    let parts: Vec<&str> = s.split(marker).collect();
    if parts.len() < 3 {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let usable = parts.len() - 1 - (parts.len() - 1) % 2;
    for (idx, part) in parts.iter().enumerate() {
        out.push_str(part);
        if idx < usable {
            out.push_str(if idx % 2 == 0 { open } else { close });
        } else if idx < parts.len() - 1 {
            out.push_str(marker);
        }
    }
    out
}

fn html_comment(c: &Comment, out: &mut String) {
    let score = c.score.unwrap_or(0);
    out.push_str(&format!(
        "<div class=\"c\" data-score=\"{}\" data-created=\"{}\">",
        score, c.created_utc as i64
    ));
    out.push_str("<div class=\"h\"><button class=\"tg\" type=\"button\">−</button>");
    out.push_str(&format!("<span class=\"a\">u/{}</span>", esc(&c.author)));
    if c.is_op {
        out.push_str("<span class=\"b op\">OP</span>");
    }
    if !c.distinguished.is_empty() {
        out.push_str(&format!(
            "<span class=\"b\">{}</span>",
            esc(&c.distinguished)
        ));
    }
    if c.stickied {
        out.push_str("<span class=\"b\">fixado</span>");
    }
    out.push_str(&format!(
        "<span class=\"m\">{} · {}</span>",
        esc(&score_label(c.score)),
        esc(&c.created)
    ));
    if !c.permalink.is_empty() {
        out.push_str(&format!(
            "<a class=\"m lk\" href=\"{}\" rel=\"noreferrer\">link</a>",
            esc(&c.permalink)
        ));
    }
    out.push_str("</div>");
    out.push_str(&format!(
        "<div class=\"body\">{}</div>",
        md_to_html(&c.body)
    ));
    if !c.replies.is_empty() {
        out.push_str("<div class=\"kids\">");
        for r in &c.replies {
            html_comment(r, out);
        }
        out.push_str("</div>");
    }
    out.push_str("</div>");
}

const CSS: &str = r#"
:root { color-scheme: light dark; --bg:#fff; --fg:#16181c; --dim:#6b7280; --line:#e5e7eb; --accent:#ff4500; --card:#f7f8f9; }
@media (prefers-color-scheme: dark) { :root { --bg:#0f1113; --fg:#e8eaed; --dim:#9aa0a6; --line:#2a2e33; --card:#16191d; } }
* { box-sizing: border-box; }
body { margin:0; background:var(--bg); color:var(--fg); font:15px/1.55 -apple-system,BlinkMacSystemFont,"Segoe UI",Roboto,Helvetica,Arial,sans-serif; }
.wrap { max-width: 900px; margin: 0 auto; padding: 24px 16px 80px; }
h1 { font-size: 24px; line-height:1.25; margin: 0 0 8px; }
.meta { color: var(--dim); font-size: 13px; margin-bottom: 16px; }
.meta a { color: var(--accent); }
.self { background: var(--card); border:1px solid var(--line); border-radius:12px; padding: 12px 16px; margin-bottom: 20px; }
.bar { position: sticky; top:0; background:var(--bg); border-bottom:1px solid var(--line); padding:10px 0; margin-bottom:16px; display:flex; gap:8px; align-items:center; flex-wrap:wrap; z-index:2; }
.bar button { font: inherit; font-size:13px; padding:5px 12px; border-radius:999px; border:1px solid var(--line); background:var(--card); color:var(--fg); cursor:pointer; }
.bar button.on { background: var(--accent); border-color: var(--accent); color:#fff; }
.bar .sp { flex:1; }
.c { border-left: 2px solid var(--line); padding-left: 12px; margin: 12px 0; }
.c:hover { border-left-color: var(--accent); }
.h { display:flex; gap:8px; align-items:baseline; flex-wrap:wrap; font-size:13px; }
.h .a { font-weight:600; }
.h .m { color: var(--dim); }
.h .lk { color: var(--dim); text-decoration:none; }
.h .lk:hover { color: var(--accent); }
.b { font-size:11px; padding:1px 6px; border-radius:6px; background:var(--card); border:1px solid var(--line); color:var(--dim); }
.b.op { color:#fff; background:var(--accent); border-color:var(--accent); }
.tg { font: inherit; width:20px; height:20px; line-height:1; padding:0; border-radius:6px; border:1px solid var(--line); background:var(--card); color:var(--dim); cursor:pointer; }
.body { overflow-wrap:anywhere; }
.body p { margin: 6px 0; }
.body pre { background: var(--card); border:1px solid var(--line); border-radius:8px; padding:10px; overflow-x:auto; font-size:13px; }
.body code { background: var(--card); border-radius:4px; padding:1px 4px; font-size:13px; }
.body blockquote { margin: 6px 0; padding-left:10px; border-left:3px solid var(--line); color:var(--dim); }
.body a { color: var(--accent); }
.c.collapsed > .body, .c.collapsed > .kids { display:none; }
.foot { color:var(--dim); font-size:12px; margin-top:40px; border-top:1px solid var(--line); padding-top:12px; }
"#;

const JS: &str = r#"
document.addEventListener('click', function (e) {
  var t = e.target;
  if (t && t.classList && t.classList.contains('tg')) {
    var c = t.closest('.c');
    c.classList.toggle('collapsed');
    t.textContent = c.classList.contains('collapsed') ? '+' : '−';
  }
});
function sortBy(key, dir) {
  var boxes = [document.getElementById('roots')].concat(Array.prototype.slice.call(document.querySelectorAll('.kids')));
  boxes.forEach(function (box) {
    var kids = Array.prototype.slice.call(box.children);
    kids.sort(function (a, b) {
      var x = parseInt(a.dataset[key] || '0', 10), y = parseInt(b.dataset[key] || '0', 10);
      return dir * (y - x);
    });
    kids.forEach(function (k) { box.appendChild(k); });
  });
}
function press(id) {
  ['s-score', 's-new', 's-old'].forEach(function (b) {
    var el = document.getElementById(b);
    if (el) el.classList.toggle('on', b === id);
  });
}
document.getElementById('s-score').onclick = function () { sortBy('score', 1); press('s-score'); };
document.getElementById('s-new').onclick = function () { sortBy('created', 1); press('s-new'); };
document.getElementById('s-old').onclick = function () { sortBy('created', -1); press('s-old'); };
document.getElementById('s-fold').onclick = function () {
  var all = document.querySelectorAll('.c');
  var open = document.querySelectorAll('.c:not(.collapsed)').length > 0;
  all.forEach(function (c) {
    c.classList.toggle('collapsed', open);
    var t = c.querySelector(':scope > .h > .tg');
    if (t) t.textContent = open ? '+' : '−';
  });
};
"#;

pub fn render_html(t: &Thread) -> String {
    let p = &t.post;
    let mut out = String::new();
    out.push_str("<!doctype html><html lang=\"pt-BR\"><head><meta charset=\"utf-8\">");
    out.push_str("<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">");
    out.push_str(&format!("<title>{}</title>", esc(&p.title)));
    out.push_str(&format!(
        "<style>{}</style></head><body><div class=\"wrap\">",
        CSS
    ));
    out.push_str(&format!("<h1>{}</h1>", esc(&p.title)));
    out.push_str(&format!(
        "<div class=\"meta\">r/{} · u/{} · {} pts · {} comentários · {}<br><a href=\"{}\" rel=\"noreferrer\">{}</a></div>",
        esc(&p.subreddit),
        esc(&p.author),
        p.score,
        p.num_comments,
        esc(&p.created),
        esc(&p.permalink),
        esc(&p.permalink)
    ));
    if !p.selftext.trim().is_empty() {
        out.push_str(&format!(
            "<div class=\"self\">{}</div>",
            md_to_html(&p.selftext)
        ));
    } else if !p.url.is_empty() && !p.url.contains("/comments/") {
        out.push_str(&format!(
            "<div class=\"self\"><a href=\"{}\" rel=\"noreferrer\">{}</a></div>",
            esc(&p.url),
            esc(&p.url)
        ));
    }
    out.push_str("<div class=\"bar\">");
    out.push_str("<button id=\"s-score\" type=\"button\" class=\"on\">Mais votados</button>");
    out.push_str("<button id=\"s-new\" type=\"button\">Recentes</button>");
    out.push_str("<button id=\"s-old\" type=\"button\">Antigos</button>");
    out.push_str("<span class=\"sp\"></span>");
    out.push_str("<button id=\"s-fold\" type=\"button\">Colapsar tudo</button></div>");
    out.push_str("<div id=\"roots\">");
    for c in &t.comments {
        html_comment(c, &mut out);
    }
    out.push_str("</div>");
    out.push_str(&format!(
        "<div class=\"foot\">{} comentários salvos{} · arquivo gerado pelo OmniGet</div>",
        t.total,
        if t.missing > 0 {
            format!(" · {} não vieram", t.missing)
        } else {
            String::new()
        }
    ));
    out.push_str(&format!("</div><script>{}</script></body></html>", JS));
    out
}

// ───────────────────────── execução ─────────────────────────

fn def_true() -> bool {
    true
}
fn def_sort() -> String {
    "top".to_string()
}
fn def_order() -> String {
    "score".to_string()
}
fn def_requests() -> u32 {
    40
}
fn def_delay() -> u64 {
    1100
}

#[derive(Debug, Clone, Deserialize)]
pub struct Options {
    pub url: String,
    pub dest: String,
    /// Ordem que o Reddit usa para escolher o que mandar: top, confidence,
    /// new, old, controversial, qa.
    #[serde(default = "def_sort")]
    pub sort: String,
    /// Ordem da saída: score, new, old.
    #[serde(default = "def_order")]
    pub order: String,
    #[serde(default = "def_true")]
    pub markdown: bool,
    #[serde(default = "def_true")]
    pub html: bool,
    #[serde(default)]
    pub json: bool,
    /// Teto de requisições extras para abrir os "carregar mais".
    #[serde(default = "def_requests")]
    pub max_requests: u32,
    #[serde(default = "def_delay")]
    pub delay_ms: u64,
    /// Conta do gerenciador de cookies a usar (None = `_default`). Com a
    /// sessão da extensão o Reddit responde como responde ao navegador do
    /// usuário; sem ela, cai no modo anônimo com o desafio de JavaScript.
    #[serde(default)]
    pub account_slug: Option<String>,
    /// Conteúdo Netscape do bucket `reddit.com`. Quem preenche é o comando
    /// Tauri, nunca o front — daí o `skip`.
    #[serde(skip)]
    pub session_netscape: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ThreadResult {
    pub title: String,
    pub subreddit: String,
    pub author: String,
    pub permalink: String,
    pub comments: usize,
    pub missing: usize,
    pub requests: u32,
    pub files: Vec<String>,
    pub dest: String,
    /// Se o arquivamento saiu com a sessão do usuário ou anônimo.
    pub used_session: bool,
}

async fn resolve_post(fetcher: &Fetcher, url: &str) -> Result<(String, Option<String>)> {
    let mut target = super::parse_target(url)
        .ok_or_else(|| anyhow!("cole o link de um post do Reddit (ou o id dele)"))?;
    if let Target::Short { url } = &target {
        let final_url = fetcher.resolve(url).await?;
        target = super::parse_target(&final_url)
            .ok_or_else(|| anyhow!("o link curto não levou a um post ({})", final_url))?;
    }
    match target {
        Target::Post { id, subreddit, .. } => Ok((id, subreddit)),
        _ => Err(anyhow!(
            "isto não é um post: cole o link de uma thread (com /comments/ no meio)"
        )),
    }
}

/// Abre os "carregar mais" até o teto de requisições. Devolve quantos
/// comentários ficaram de fora.
async fn expand(
    fetcher: &Fetcher,
    post_id: &str,
    sort: &str,
    mut mores: Vec<MoreRef>,
    flat: &mut Vec<Comment>,
    budget: u32,
    progress: &ProgressFn,
) -> usize {
    let mut missing = 0usize;
    let mut used = 0u32;
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    while let Some(more) = mores.pop() {
        if used >= budget {
            missing += more.children.len().max(1);
            continue;
        }
        let key = format!("{}|{}", more.parent_id, more.children.join(","));
        if !seen.insert(key) {
            continue;
        }
        let (new_flat, new_mores) = if more.children.is_empty() {
            // "continuar esta conversa": o galho inteiro vem do permalink do nó.
            let parent = more.parent_id.trim_start_matches("t1_");
            if parent.is_empty() || more.parent_id.starts_with("t3_") {
                continue;
            }
            used += 1;
            match fetcher
                .get_json(&branch_json_url(post_id, parent, sort, 500))
                .await
            {
                Ok(v) => match parse_thread_json(&v) {
                    Ok((_, f, m)) => (f, m),
                    Err(_) => (Vec::new(), Vec::new()),
                },
                Err(_) => {
                    missing += 1;
                    (Vec::new(), Vec::new())
                }
            }
        } else {
            let batch: Vec<String> = more.children.iter().take(100).cloned().collect();
            let rest: Vec<String> = more.children.iter().skip(100).cloned().collect();
            if !rest.is_empty() {
                mores.push(MoreRef {
                    parent_id: more.parent_id.clone(),
                    children: rest,
                });
            }
            used += 1;
            match fetcher
                .get_json(&more_children_url(post_id, &batch, sort))
                .await
            {
                Ok(v) => {
                    let parsed = parse_more_json(&v);
                    if parsed.0.is_empty() {
                        missing += batch.len();
                    }
                    parsed
                }
                Err(_) => {
                    missing += batch.len();
                    (Vec::new(), Vec::new())
                }
            }
        };
        flat.extend(new_flat);
        mores.extend(new_mores);
        report(
            progress,
            ID,
            "progress",
            flat.len() as u64,
            None,
            Some(format!("{} comentários", flat.len())),
        );
    }
    missing
}

fn out_path(dest: &Path, base: &str, ext: &str) -> PathBuf {
    dest.join(format!("{}.{}", base, ext))
}

pub async fn run(opts: &Options, progress: ProgressFn) -> Result<ThreadResult> {
    report(&progress, ID, "started", 0, None, None);
    let fetcher = Fetcher::new(opts.delay_ms, opts.session_netscape.as_deref())?;
    let (post_id, _sub) = resolve_post(&fetcher, &opts.url).await?;
    let root = fetcher
        .get_json(&post_json_url(&post_id, &opts.sort, 500))
        .await?;
    let (post, mut flat, mores) = parse_thread_json(&root)?;
    report(
        &progress,
        ID,
        "progress",
        flat.len() as u64,
        Some(post.num_comments.max(0) as u64),
        Some(post.title.clone()),
    );
    let missing = expand(
        &fetcher,
        &post_id,
        &opts.sort,
        mores,
        &mut flat,
        opts.max_requests,
        &progress,
    )
    .await;
    let mut comments = build_tree(flat, &post.id);
    sort_tree(&mut comments, &opts.order);
    let total = count_tree(&comments);
    let thread = Thread {
        post,
        comments,
        total,
        missing,
        sorted_by: opts.order.clone(),
    };

    let dest = PathBuf::from(&opts.dest);
    std::fs::create_dir_all(&dest)?;
    let base: String = crate::core::tools::sanitize_name(&format!(
        "r-{} {} {}",
        thread.post.subreddit,
        thread.post.id,
        thread.post.title.trim()
    ))
    .chars()
    .take(90)
    .collect();
    let base = base.trim().to_string();
    let mut files = Vec::new();
    if opts.markdown {
        let path = out_path(&dest, &base, "md");
        std::fs::write(&path, render_markdown(&thread))?;
        files.push(path.to_string_lossy().to_string());
    }
    if opts.html {
        let path = out_path(&dest, &base, "html");
        std::fs::write(&path, render_html(&thread))?;
        files.push(path.to_string_lossy().to_string());
    }
    if opts.json {
        let path = out_path(&dest, &base, "json");
        std::fs::write(&path, serde_json::to_string_pretty(&thread)?)?;
        files.push(path.to_string_lossy().to_string());
    }
    if files.is_empty() {
        return Err(anyhow!("escolha pelo menos um formato de saída"));
    }
    report(
        &progress,
        ID,
        "done",
        total as u64,
        Some(total as u64),
        None,
    );
    Ok(ThreadResult {
        title: thread.post.title,
        subreddit: thread.post.subreddit,
        author: thread.post.author,
        permalink: thread.post.permalink,
        comments: total,
        missing,
        requests: fetcher.requests(),
        files,
        dest: dest.to_string_lossy().to_string(),
        used_session: fetcher.has_session(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Uma thread de mentira com a mesma forma da de verdade: dois listings,
    /// resposta aninhada, um `more` com ids e um "continuar esta conversa".
    const FIXTURE: &str = r#"[
      {"kind":"Listing","data":{"children":[{"kind":"t3","data":{
        "id":"1abcdef","title":"Como o Reddit serve video e audio separados",
        "author":"fulano","subreddit":"rust","created_utc":1700000000,
        "score":1234,"upvote_ratio":0.97,"num_comments":5,
        "permalink":"/r/rust/comments/1abcdef/como_o_reddit/",
        "url":"https://v.redd.it/xyz123","selftext":"Texto do **post** com <script>alert(1)</script>",
        "link_flair_text":"Discussao","over_18":false,"locked":false}}]}},
      {"kind":"Listing","data":{"children":[
        {"kind":"t1","data":{"id":"c1","parent_id":"t3_1abcdef","author":"ana",
          "body":"Primeiro comentario","score":50,"created_utc":1700000100,
          "permalink":"/r/rust/comments/1abcdef/como_o_reddit/c1/","is_submitter":false,
          "distinguished":null,"stickied":false,
          "replies":{"kind":"Listing","data":{"children":[
            {"kind":"t1","data":{"id":"c2","parent_id":"t1_c1","author":"bruno",
              "body":"Resposta com [link](https://exemplo.com) e `codigo`","score":10,
              "created_utc":1700000200,"permalink":"/r/rust/comments/1abcdef/x/c2/",
              "is_submitter":true,"stickied":false,"replies":""}},
            {"kind":"more","data":{"id":"_","parent_id":"t1_c1","children":[],"count":3}}
          ]}}}},
        {"kind":"t1","data":{"id":"c3","parent_id":"t3_1abcdef","author":"carla",
          "body":"Comentario mais novo e mais fraco","score":2,"created_utc":1700000900,
          "permalink":"/r/rust/comments/1abcdef/x/c3/","score_hidden":false,"replies":""}},
        {"kind":"more","data":{"id":"m1","parent_id":"t3_1abcdef","children":["c9","ca"],"count":2}}
      ]}}
    ]"#;

    fn thread_from_fixture() -> Thread {
        let v: Value = serde_json::from_str(FIXTURE).expect("fixture valida");
        let (post, flat, mores) = parse_thread_json(&v).expect("parse");
        assert_eq!(mores.len(), 2);
        let mut comments = build_tree(flat, &post.id);
        sort_tree(&mut comments, "score");
        let total = count_tree(&comments);
        Thread {
            post,
            comments,
            total,
            missing: 2,
            sorted_by: "score".to_string(),
        }
    }

    #[test]
    fn normaliza_post_e_arvore() {
        let t = thread_from_fixture();
        assert_eq!(t.post.id, "1abcdef");
        assert_eq!(t.post.subreddit, "rust");
        assert_eq!(t.post.score, 1234);
        assert_eq!(
            t.post.permalink,
            "https://www.reddit.com/r/rust/comments/1abcdef/como_o_reddit/"
        );
        assert_eq!(t.total, 3);
        assert_eq!(t.comments.len(), 2);
        // Ordenado por score: ana (50) antes de carla (2).
        assert_eq!(t.comments[0].author, "ana");
        assert_eq!(t.comments[0].replies.len(), 1);
        assert_eq!(t.comments[0].replies[0].author, "bruno");
        assert!(t.comments[0].replies[0].is_op);
    }

    #[test]
    fn separa_os_dois_tipos_de_more() {
        let v: Value = serde_json::from_str(FIXTURE).expect("fixture valida");
        let (_, _, mores) = parse_thread_json(&v).expect("parse");
        let continua = mores
            .iter()
            .find(|m| m.children.is_empty())
            .expect("continuar");
        assert_eq!(continua.parent_id, "t1_c1");
        let lote = mores.iter().find(|m| !m.children.is_empty()).expect("lote");
        assert_eq!(lote.children, vec!["c9".to_string(), "ca".to_string()]);
    }

    #[test]
    fn ordena_por_data() {
        let mut t = thread_from_fixture();
        sort_tree(&mut t.comments, "new");
        assert_eq!(t.comments[0].author, "carla");
        sort_tree(&mut t.comments, "old");
        assert_eq!(t.comments[0].author, "ana");
    }

    #[test]
    fn orfao_nao_se_perde() {
        let flat = vec![Comment {
            id: "z1".to_string(),
            parent_id: "t1_sumiu".to_string(),
            author: "eco".to_string(),
            body: "sem pai".to_string(),
            score: Some(1),
            created_utc: 1.0,
            created: String::new(),
            permalink: String::new(),
            is_op: false,
            distinguished: String::new(),
            stickied: false,
            replies: Vec::new(),
        }];
        let tree = build_tree(flat, "1abcdef");
        assert_eq!(tree.len(), 1);
        assert_eq!(tree[0].author, "eco");
    }

    #[test]
    fn le_o_things_do_morechildren() {
        let raw = r#"{"json":{"errors":[],"data":{"things":[
          {"kind":"t1","data":{"id":"c9","parent_id":"t3_1abcdef","author":"davi","body":"veio do more","score":7,"created_utc":1700000300}},
          {"kind":"more","data":{"id":"m2","parent_id":"t1_c9","children":["cb"],"count":1}}
        ]}}}"#;
        let v: Value = serde_json::from_str(raw).expect("json");
        let (flat, mores) = parse_more_json(&v);
        assert_eq!(flat.len(), 1);
        assert_eq!(flat[0].author, "davi");
        assert_eq!(mores.len(), 1);
        assert_eq!(mores[0].children, vec!["cb".to_string()]);
    }

    #[test]
    fn markdown_sai_indentado() {
        let md = render_markdown(&thread_from_fixture());
        assert!(md.starts_with("# Como o Reddit serve video e audio separados"));
        assert!(md.contains("r/rust · u/fulano · 1234 pts"));
        assert!(md.contains("- **u/ana** · 50 pts"));
        assert!(md.contains("  - **u/bruno** · 10 pts"));
        assert!(md.contains("    Resposta com [link](https://exemplo.com)"));
        assert!(md.contains("2 comentários não vieram"));
    }

    #[test]
    fn html_e_arquivo_unico_e_escapa_o_que_veio_de_fora() {
        let html = render_html(&thread_from_fixture());
        assert!(html.starts_with("<!doctype html>"));
        assert!(html.contains("<style>"));
        assert!(html.contains("<script>"));
        assert!(!html.contains("<script>alert(1)</script>"));
        assert!(html.contains("&lt;script&gt;alert(1)&lt;/script&gt;"));
        assert!(html.contains("data-score=\"50\""));
        assert!(html.contains("data-created=\"1700000100\""));
        assert!(html.contains("id=\"s-score\""));
        assert!(html.contains("class=\"kids\""));
        assert!(html.contains("<span class=\"b op\">OP</span>"));
    }

    #[test]
    fn markdown_do_corpo_vira_html() {
        assert_eq!(md_to_html("oi"), "<p>oi</p>");
        assert_eq!(md_to_html("a\nb"), "<p>a<br>b</p>");
        assert_eq!(md_to_html("a\n\nb"), "<p>a</p><p>b</p>");
        assert_eq!(
            md_to_html("> citado"),
            "<blockquote><p>citado</p></blockquote>"
        );
        assert_eq!(md_to_html("```\nx < y\n```"), "<pre>x &lt; y\n</pre>");
        assert_eq!(
            md_to_html("um **negrito** aqui"),
            "<p>um <strong>negrito</strong> aqui</p>"
        );
        assert_eq!(
            md_to_html("um *italico* aqui"),
            "<p>um <em>italico</em> aqui</p>"
        );
        assert_eq!(
            md_to_html("use `x` agora"),
            "<p>use <code>x</code> agora</p>"
        );
        assert!(
            md_to_html("veja https://exemplo.com/a").contains("<a href=\"https://exemplo.com/a\"")
        );
        assert!(md_to_html("[texto](https://exemplo.com)").contains(">texto</a>"));
        // Asterisco solto não pode virar tag aberta.
        assert_eq!(md_to_html("2 * 3"), "<p>2 * 3</p>");
    }

    #[test]
    fn escreve_os_tres_arquivos() {
        let dir = std::env::temp_dir().join(format!("omniget-rdthread-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).expect("tempdir");
        let t = thread_from_fixture();
        let base = "teste";
        std::fs::write(out_path(&dir, base, "md"), render_markdown(&t)).expect("md");
        std::fs::write(out_path(&dir, base, "html"), render_html(&t)).expect("html");
        std::fs::write(
            out_path(&dir, base, "json"),
            serde_json::to_string_pretty(&t).expect("json"),
        )
        .expect("escreve json");
        let de_volta: Value = serde_json::from_str(
            &std::fs::read_to_string(out_path(&dir, base, "json")).expect("le"),
        )
        .expect("json valido");
        assert_eq!(de_volta["post"]["subreddit"], "rust");
        assert_eq!(de_volta["comments"][0]["author"], "ana");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    #[ignore = "rede: baixa uma thread publica de verdade do Reddit"]
    async fn baixa_uma_thread_publica_de_verdade() {
        let dir = std::env::temp_dir().join("omniget-rdthread-live");
        let _ = std::fs::create_dir_all(&dir);
        let opts = Options {
            url: "https://www.reddit.com/r/rust/comments/1uwmef6/so_long_rrust_and_thanks_for_all_the_slop/".to_string(),
            dest: dir.to_string_lossy().to_string(),
            sort: "top".to_string(),
            order: "score".to_string(),
            markdown: true,
            html: true,
            json: true,
            max_requests: 6,
            delay_ms: 1200,
            account_slug: None,
            session_netscape: None,
        };
        let r = run(&opts, crate::core::tools::noop_progress())
            .await
            .expect("thread real");
        println!(
            "r/{} · {} · {} comentários · {} requisições · {:?}",
            r.subreddit, r.title, r.comments, r.requests, r.files
        );
        assert!(r.comments > 0);
        assert_eq!(r.files.len(), 3);
    }
}

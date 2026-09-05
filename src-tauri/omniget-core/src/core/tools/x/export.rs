//! Exportacao de posts e usuarios (estudo 67): JSON, CSV, Markdown, HTML e
//! texto. Os mesmos formatos do twitter-web-exporter, escritos a mao para
//! nao trazer crate de CSV.

use super::{XPost, XUser};

pub fn csv_escape(s: &str) -> String {
    if s.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

pub fn posts_csv(posts: &[XPost]) -> String {
    let mut out = String::from("id,url,created_at,author,name,text,likes,reposts,replies,quotes,views,bookmarks,lang,media_urls,reply_to,quote_url\n");
    for p in posts {
        let media: Vec<&str> = p.media.iter().map(|m| m.url.as_str()).collect();
        let row = [
            p.id.clone(),
            p.url.clone(),
            p.created_at.clone(),
            p.author.handle.clone(),
            p.author.name.clone(),
            p.text.clone(),
            p.likes.to_string(),
            p.reposts.to_string(),
            p.replies.to_string(),
            p.quotes.to_string(),
            p.views.to_string(),
            p.bookmarks.to_string(),
            p.lang.clone(),
            media.join(" "),
            p.reply_to_id.clone().unwrap_or_default(),
            p.quote.as_ref().map(|q| q.url.clone()).unwrap_or_default(),
        ];
        out.push_str(&row.iter().map(|c| csv_escape(c)).collect::<Vec<_>>().join(","));
        out.push('\n');
    }
    out
}

pub fn users_csv(users: &[XUser]) -> String {
    let mut out = String::from("id,handle,name,url,followers,following,posts,bio,location,joined,verified,protected,follows_me,followed_by_me\n");
    for u in users {
        let row = [
            u.id.clone(),
            u.handle.clone(),
            u.name.clone(),
            u.url(),
            u.followers.to_string(),
            u.following.to_string(),
            u.posts.to_string(),
            u.bio.clone(),
            u.location.clone(),
            u.joined.clone(),
            u.verified.to_string(),
            u.protected.to_string(),
            u.follows_me.map(|b| b.to_string()).unwrap_or_default(),
            u.followed_by_me.map(|b| b.to_string()).unwrap_or_default(),
        ];
        out.push_str(&row.iter().map(|c| csv_escape(c)).collect::<Vec<_>>().join(","));
        out.push('\n');
    }
    out
}

fn date_short(iso: &str) -> String {
    iso.get(..16).map(|s| s.replace('T', " ")).unwrap_or_else(|| iso.to_string())
}

pub fn posts_markdown(title: &str, posts: &[XPost]) -> String {
    let mut out = format!("# {}\n\n", title);
    if let Some(first) = posts.first() {
        out.push_str(&format!("**{}** (@{}) · {}\n\n", first.author.name, first.author.handle, first.url));
    }
    for (i, p) in posts.iter().enumerate() {
        if posts.len() > 1 {
            out.push_str(&format!("### {}/{} · {}\n\n", i + 1, posts.len(), date_short(&p.created_at)));
        }
        if posts.len() > 1 && posts.first().map(|f| f.author.handle != p.author.handle).unwrap_or(false) {
            out.push_str(&format!("_@{}_\n\n", p.author.handle));
        }
        out.push_str(p.text.trim());
        out.push_str("\n\n");
        for m in &p.media {
            match m.kind.as_str() {
                "photo" => out.push_str(&format!("![{}]({})\n\n", if m.alt.is_empty() { "imagem" } else { &m.alt }, m.url)),
                _ => out.push_str(&format!("[{}]({})\n\n", m.kind, m.url)),
            }
        }
        if let Some(q) = &p.quote {
            out.push_str(&format!("> **@{}**: {}\n> {}\n\n", q.author.handle, q.text.replace('\n', "\n> "), q.url));
        }
        out.push_str(&format!("♥ {} · ↻ {} · 💬 {}{} · [{}]({})\n\n", p.likes, p.reposts, p.replies, if p.views > 0 { format!(" · 👁 {}", p.views) } else { String::new() }, p.id, p.url));
    }
    out.trim_end().to_string() + "\n"
}

pub fn posts_text(posts: &[XPost]) -> String {
    posts
        .iter()
        .map(|p| format!("@{} · {}\n{}\n{}", p.author.handle, date_short(&p.created_at), p.text.trim(), p.url))
        .collect::<Vec<_>>()
        .join("\n\n---\n\n")
        + "\n"
}

pub fn posts_html(title: &str, posts: &[XPost]) -> String {
    let mut body = String::new();
    for p in posts {
        let mut media = String::new();
        for m in &p.media {
            match m.kind.as_str() {
                "photo" => media.push_str(&format!("<a href=\"{0}\"><img src=\"{0}\" alt=\"{1}\" loading=\"lazy\"></a>", html_escape(&m.url), html_escape(&m.alt))),
                _ => media.push_str(&format!("<video src=\"{}\" poster=\"{}\" controls preload=\"none\"></video>", html_escape(&m.url), html_escape(&m.thumb))),
            }
        }
        let quote = p
            .quote
            .as_ref()
            .map(|q| format!("<blockquote><b>@{}</b> {}<br><a href=\"{}\">{}</a></blockquote>", html_escape(&q.author.handle), html_escape(&q.text).replace('\n', "<br>"), html_escape(&q.url), html_escape(&q.url)))
            .unwrap_or_default();
        body.push_str(&format!(
            "<article><header><img class=\"avatar\" src=\"{}\" alt=\"\"><div><b>{}</b> <span class=\"h\">@{}</span><br><time>{}</time></div></header><p>{}</p><div class=\"media\">{}</div>{}<footer>♥ {} · ↻ {} · 💬 {} · 👁 {} · <a href=\"{}\">abrir no X</a></footer></article>\n",
            html_escape(&p.author.avatar),
            html_escape(&p.author.name),
            html_escape(&p.author.handle),
            date_short(&p.created_at),
            html_escape(&p.text).replace('\n', "<br>"),
            media,
            quote,
            p.likes,
            p.reposts,
            p.replies,
            p.views,
            html_escape(&p.url)
        ));
    }
    format!(
        "<!doctype html><html><head><meta charset=\"utf-8\"><title>{t}</title><style>body{{font:16px/1.5 -apple-system,Segoe UI,Helvetica,Arial,sans-serif;max-width:680px;margin:32px auto;padding:0 16px;color:#111;background:#fafafa}}article{{background:#fff;border:1px solid #e5e5e5;border-radius:16px;padding:16px;margin:0 0 16px}}header{{display:flex;gap:12px;align-items:center;margin-bottom:8px}}.avatar{{width:44px;height:44px;border-radius:50%}}.h,time,footer{{color:#666;font-size:14px}}.media img,.media video{{max-width:100%;border-radius:12px;margin-top:8px;display:block}}blockquote{{border-left:3px solid #ddd;margin:8px 0;padding:4px 12px;color:#333}}p{{white-space:pre-wrap;margin:0}}</style></head><body><h1>{t}</h1>{b}</body></html>",
        t = html_escape(title),
        b = body
    )
}

/// Escreve `posts` em `dest` no formato pedido e devolve o caminho.
pub fn write_posts(posts: &[XPost], format: &str, dest: &std::path::Path, title: &str) -> anyhow::Result<String> {
    let content = match format {
        "json" => serde_json::to_string_pretty(posts)?,
        "csv" => posts_csv(posts),
        "md" | "markdown" => posts_markdown(title, posts),
        "html" => posts_html(title, posts),
        "txt" | "text" => posts_text(posts),
        other => anyhow::bail!("formato desconhecido: {}", other),
    };
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(dest, content)?;
    Ok(dest.to_string_lossy().to_string())
}

pub fn write_users(users: &[XUser], format: &str, dest: &std::path::Path) -> anyhow::Result<String> {
    let content = match format {
        "json" => serde_json::to_string_pretty(users)?,
        "csv" => users_csv(users),
        "md" | "markdown" | "txt" => users.iter().map(|u| format!("- @{} · {} · {}", u.handle, u.name, u.url())).collect::<Vec<_>>().join("\n") + "\n",
        other => anyhow::bail!("formato desconhecido: {}", other),
    };
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(dest, content)?;
    Ok(dest.to_string_lossy().to_string())
}

pub fn ext_for(format: &str) -> &'static str {
    match format {
        "json" => "json",
        "csv" => "csv",
        "html" => "html",
        "txt" | "text" => "txt",
        _ => "md",
    }
}

use std::collections::HashMap;

use chrono::{DateTime, FixedOffset};
use serde_json::{json, Value};

use super::model::{Channel, Message};

pub fn local_time(ts: &str, offset: &FixedOffset) -> String {
    DateTime::parse_from_rfc3339(ts)
        .map(|t| t.with_timezone(offset).format("%Y-%m-%d %H:%M").to_string())
        .unwrap_or_else(|_| ts.to_string())
}

pub fn escape_html(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 16);
    for ch in s.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            c => out.push(c),
        }
    }
    out
}

fn author_line(m: &Message) -> String {
    let display = m.author.display();
    if display == m.author.username || m.author.username.is_empty() {
        display.to_string()
    } else {
        format!("{} ({})", display, m.author.username)
    }
}

pub fn readable_content(m: &Message) -> String {
    let mut text = m.content.clone();
    for u in &m.mentions {
        let name = format!("@{}", u.display());
        text = text
            .replace(&format!("<@{}>", u.id), &name)
            .replace(&format!("<@!{}>", u.id), &name);
    }
    text
}

pub struct Meta<'a> {
    pub place: &'a str,
    pub channel: &'a str,
    pub exported_at: &'a str,
}

pub fn to_txt(
    meta: &Meta,
    msgs: &[Message],
    offset: &FixedOffset,
    local: &HashMap<String, String>,
) -> String {
    let mut out = String::new();
    out.push_str(&format!("# {} / {}\n", meta.place, meta.channel));
    out.push_str(&format!(
        "# {} · {} messages\n\n",
        meta.exported_at,
        msgs.len()
    ));
    for m in msgs {
        out.push_str(&format!(
            "[{}] {}",
            local_time(&m.timestamp, offset),
            author_line(m)
        ));
        if m.edited_timestamp.is_some() {
            out.push_str(" (edited)");
        }
        out.push(':');
        if let Some(r) = &m.referenced_message {
            out.push_str(&format!(
                "\n    > {}: {}",
                r.author.display(),
                first_line(&r.content)
            ));
        }
        let content = readable_content(m);
        if !content.is_empty() {
            for (i, line) in content.lines().enumerate() {
                out.push_str(if i == 0 { " " } else { "\n    " });
                out.push_str(line);
            }
        }
        for a in &m.attachments {
            let link = local.get(&a.url).cloned().unwrap_or_else(|| a.url.clone());
            out.push_str(&format!("\n    [file] {} {}", a.filename, link));
        }
        for e in &m.embeds {
            let title = e.title.as_deref().or(e.url.as_deref()).unwrap_or("");
            if !title.is_empty() {
                out.push_str(&format!("\n    [embed] {}", title));
            }
        }
        for s in &m.sticker_items {
            out.push_str(&format!("\n    [sticker] {}", s.name));
        }
        out.push('\n');
    }
    out
}

fn first_line(s: &str) -> String {
    let line = s.lines().next().unwrap_or("");
    if line.chars().count() > 120 {
        format!("{}…", line.chars().take(120).collect::<String>())
    } else {
        line.to_string()
    }
}

fn linkify(escaped: &str) -> String {
    let mut out = String::with_capacity(escaped.len());
    for (i, word) in escaped.split(' ').enumerate() {
        if i > 0 {
            out.push(' ');
        }
        if (word.starts_with("https://") || word.starts_with("http://")) && !word.contains('"') {
            out.push_str(&format!(
                "<a href=\"{w}\" rel=\"noreferrer\">{w}</a>",
                w = word
            ));
        } else {
            out.push_str(word);
        }
    }
    out
}

fn is_image(name: &str, ct: Option<&str>) -> bool {
    super::model::kind_of(ct, name) == "image"
}

pub fn to_html(
    meta: &Meta,
    msgs: &[Message],
    offset: &FixedOffset,
    local: &HashMap<String, String>,
) -> String {
    let mut body = String::new();
    let mut last_author = String::new();
    for m in msgs {
        let grouped = last_author == m.author.id && m.referenced_message.is_none();
        last_author = m.author.id.clone();
        body.push_str(&format!(
            "<article class=\"m{}\" id=\"m{}\">",
            if grouped { " g" } else { "" },
            escape_html(&m.id)
        ));
        if let Some(r) = &m.referenced_message {
            body.push_str(&format!(
                "<div class=\"r\">↪ <b>{}</b> {}</div>",
                escape_html(r.author.display()),
                escape_html(&first_line(&r.content))
            ));
        }
        if !grouped {
            body.push_str(&format!(
                "<header><b>{}</b> <time>{}</time>{}</header>",
                escape_html(&author_line(m)),
                escape_html(&local_time(&m.timestamp, offset)),
                if m.edited_timestamp.is_some() {
                    " <i>(edited)</i>"
                } else {
                    ""
                }
            ));
        }
        let content = readable_content(m);
        if !content.is_empty() {
            let html: Vec<String> = content.lines().map(|l| linkify(&escape_html(l))).collect();
            body.push_str(&format!("<p>{}</p>", html.join("<br>")));
        }
        for a in &m.attachments {
            let link = escape_html(local.get(&a.url).map(String::as_str).unwrap_or(&a.url));
            if is_image(&a.filename, a.content_type.as_deref()) {
                body.push_str(&format!(
                    "<a href=\"{link}\"><img loading=\"lazy\" src=\"{link}\" alt=\"{}\"></a>",
                    escape_html(&a.filename)
                ));
            } else {
                body.push_str(&format!(
                    "<div class=\"f\"><a href=\"{link}\">{}</a></div>",
                    escape_html(&a.filename)
                ));
            }
        }
        for e in &m.embeds {
            let title = e.title.as_deref().unwrap_or("");
            let desc = e.description.as_deref().unwrap_or("");
            if title.is_empty() && desc.is_empty() {
                continue;
            }
            body.push_str("<div class=\"e\">");
            if !title.is_empty() {
                match &e.url {
                    Some(u) if u.starts_with("http") => body.push_str(&format!(
                        "<a href=\"{}\"><b>{}</b></a>",
                        escape_html(u),
                        escape_html(title)
                    )),
                    _ => body.push_str(&format!("<b>{}</b>", escape_html(title))),
                }
            }
            if !desc.is_empty() {
                body.push_str(&format!(
                    "<div>{}</div>",
                    escape_html(desc).replace('\n', "<br>")
                ));
            }
            body.push_str("</div>");
        }
        body.push_str("</article>\n");
    }
    format!(
        "<!DOCTYPE html>\n<html><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width\"><title>{title}</title>\n<style>{css}</style></head>\n<body><h1>{place} / {channel}</h1><p class=\"meta\">{at} · {n}</p>\n{body}</body></html>\n",
        title = escape_html(&format!("{} / {}", meta.place, meta.channel)),
        css = CSS,
        place = escape_html(meta.place),
        channel = escape_html(meta.channel),
        at = escape_html(meta.exported_at),
        n = msgs.len(),
        body = body
    )
}

const CSS: &str = "body{font:14px/1.45 system-ui,sans-serif;max-width:860px;margin:2em auto;padding:0 1em;background:#1e1f22;color:#dbdee1}h1{font-size:20px}.meta{color:#949ba4}article{padding:4px 0}article:not(.g){margin-top:12px}header b{color:#f2f3f5}time{color:#949ba4;font-size:12px;margin-left:6px}p{margin:2px 0;white-space:pre-wrap;overflow-wrap:anywhere}a{color:#00a8fc}img{max-width:420px;max-height:320px;border-radius:6px;display:block;margin:4px 0}.r{color:#949ba4;font-size:12px}.e{border-left:3px solid #4e5058;background:#2b2d31;padding:6px 10px;margin:4px 0;border-radius:4px;max-width:520px}.f{margin:4px 0}";

pub fn to_json(meta: &Meta, channel: Option<&Channel>, msgs: &[Message]) -> Value {
    json!({
        "format": "omniget-discord-export",
        "version": 1,
        "exported_at": meta.exported_at,
        "place": meta.place,
        "channel": channel.map(|c| json!({"id": c.id, "name": c.name, "guild_id": c.guild_id, "type": c.kind})),
        "message_count": msgs.len(),
        "messages": msgs,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::discord::model::{Attachment, User};

    fn msg(id: &str, author: &str, content: &str) -> Message {
        Message {
            id: id.into(),
            channel_id: "5".into(),
            author: User {
                id: author.into(),
                username: author.into(),
                ..Default::default()
            },
            content: content.into(),
            timestamp: "2026-11-06T12:30:05.000000+00:00".into(),
            ..Default::default()
        }
    }

    fn meta() -> Meta<'static> {
        Meta {
            place: "Guild",
            channel: "general",
            exported_at: "2026-11-06 13:00",
        }
    }

    #[test]
    fn txt_lists_messages_with_files_and_mentions() {
        let mut a = msg("1", "ana", "hello <@2>\nsecond line");
        a.mentions.push(User {
            id: "2".into(),
            username: "bob".into(),
            global_name: Some("Bob".into()),
            ..Default::default()
        });
        a.attachments.push(Attachment {
            filename: "x.png".into(),
            url: "https://cdn/x.png".into(),
            ..Default::default()
        });
        let mut b = msg("2", "bob", "hi");
        b.edited_timestamp = Some("x".into());
        b.referenced_message = Some(Box::new(a.clone()));
        let tz = FixedOffset::west_opt(3 * 3600).unwrap();
        let mut local = HashMap::new();
        local.insert("https://cdn/x.png".to_string(), "files/x.png".to_string());
        let txt = to_txt(&meta(), &[a, b], &tz, &local);
        assert!(txt.starts_with("# Guild / general\n"));
        assert!(txt.contains("[2026-11-06 09:30] ana: hello @Bob\n    second line"));
        assert!(txt.contains("[file] x.png files/x.png"));
        assert!(txt.contains("bob (edited):\n    > ana: hello <@2>"));
    }

    #[test]
    fn html_escapes_everything_and_groups_authors() {
        let a = msg(
            "1",
            "ana",
            "<script>alert(1)</script> https://example.com/a?b=1&c=2",
        );
        let b = msg("2", "ana", "again");
        let html = to_html(
            &meta(),
            &[a, b],
            &FixedOffset::east_opt(0).unwrap(),
            &HashMap::new(),
        );
        assert!(!html.contains("<script>alert"));
        assert!(html.contains("&lt;script&gt;"));
        assert!(html.contains("<a href=\"https://example.com/a?b=1&amp;c=2\""));
        assert_eq!(html.matches("<header>").count(), 1);
        assert!(html.contains("class=\"m g\""));
    }

    #[test]
    fn json_keeps_the_messages() {
        let v = to_json(&meta(), None, &[msg("1", "ana", "x")]);
        assert_eq!(v["message_count"], 1);
        assert_eq!(v["messages"][0]["content"], "x");
        assert_eq!(v["format"], "omniget-discord-export");
    }

    #[test]
    fn bad_timestamps_pass_through() {
        assert_eq!(
            local_time("nope", &FixedOffset::east_opt(0).unwrap()),
            "nope"
        );
    }
}

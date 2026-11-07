use chrono::{FixedOffset, TimeZone};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::model::MessageKind;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExportFormat {
    Json,
    Txt,
    Html,
}

impl ExportFormat {
    pub fn extension(self) -> &'static str {
        match self {
            Self::Json => "json",
            Self::Txt => "txt",
            Self::Html => "html",
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct ExportLabels {
    pub you: String,
    pub media_omitted: String,
    pub attached: String,
    pub exported_from: String,
    pub messages: String,
    pub deleted: String,
    pub edited: String,
}

impl Default for ExportLabels {
    fn default() -> Self {
        Self {
            you: "You".into(),
            media_omitted: "<Media omitted>".into(),
            attached: "(file attached)".into(),
            exported_from: "Exported from OmniGet".into(),
            messages: "messages".into(),
            deleted: "This message was deleted".into(),
            edited: "<This message was edited>".into(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ExportMessage {
    pub id: String,
    pub ts: i64,
    pub from_me: bool,
    pub sender: String,
    pub kind: MessageKind,
    pub text: Option<String>,
    pub file_name: Option<String>,
    pub mime: Option<String>,
    pub size: Option<u64>,
    pub saved_path: Option<String>,
    pub edited: bool,
    pub deleted: bool,
}

fn stamp(ts: i64, offset: FixedOffset, fmt: &str) -> String {
    offset
        .timestamp_opt(ts, 0)
        .single()
        .map(|d| d.format(fmt).to_string())
        .unwrap_or_default()
}

fn who<'a>(m: &'a ExportMessage, labels: &'a ExportLabels) -> &'a str {
    if m.from_me {
        &labels.you
    } else {
        &m.sender
    }
}

fn attachment_name(m: &ExportMessage) -> Option<String> {
    m.saved_path
        .as_deref()
        .and_then(|p| std::path::Path::new(p).file_name())
        .map(|n| n.to_string_lossy().into_owned())
}

fn body_line(m: &ExportMessage, labels: &ExportLabels) -> String {
    if m.deleted {
        return labels.deleted.clone();
    }
    let line = body_parts(m, labels);
    if m.edited {
        format!("{line} {}", labels.edited)
    } else {
        line
    }
}

fn body_parts(m: &ExportMessage, labels: &ExportLabels) -> String {
    let mut parts: Vec<String> = Vec::new();
    if m.kind.media().is_some() {
        match attachment_name(m) {
            Some(name) => parts.push(format!("{name} {}", labels.attached)),
            None => parts.push(labels.media_omitted.clone()),
        }
    } else if matches!(
        m.kind,
        MessageKind::Location | MessageKind::Contact | MessageKind::Poll
    ) {
        if let Some(t) = &m.text {
            parts.push(format!("[{}] {t}", m.kind.as_str()));
            return parts.join("\n");
        }
    }
    if let Some(t) = &m.text {
        parts.push(t.clone());
    }
    parts.join("\n")
}

pub fn to_txt(messages: &[ExportMessage], labels: &ExportLabels, offset: FixedOffset) -> String {
    let mut out = String::new();
    for m in messages {
        out.push_str(&format!(
            "{} - {}: {}\n",
            stamp(m.ts, offset, "%d/%m/%Y %H:%M"),
            who(m, labels),
            body_line(m, labels)
        ));
    }
    out
}

pub fn to_json(
    chat_id: &str,
    chat_name: &str,
    messages: &[ExportMessage],
    offset: FixedOffset,
) -> String {
    let items: Vec<serde_json::Value> = messages
        .iter()
        .map(|m| {
            json!({
                "id": m.id,
                "timestamp": m.ts,
                "date": stamp(m.ts, offset, "%Y-%m-%dT%H:%M:%S%:z"),
                "from_me": m.from_me,
                "sender": m.sender,
                "kind": m.kind,
                "text": m.text,
                "file_name": m.file_name,
                "mime": m.mime,
                "size": m.size,
                "saved_path": m.saved_path,
                "edited": m.edited,
                "deleted": m.deleted,
            })
        })
        .collect();
    let doc = json!({
        "chat": { "id": chat_id, "name": chat_name },
        "exported_by": "OmniGet",
        "count": messages.len(),
        "messages": items,
    });
    serde_json::to_string_pretty(&doc).unwrap_or_else(|_| "{}".into())
}

pub fn escape_html(s: &str) -> String {
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

fn file_href(path: &str) -> String {
    let p = path.replace('\\', "/");
    let mut out = String::from("file://");
    if !p.starts_with('/') {
        out.push('/');
    }
    for b in p.bytes() {
        if b.is_ascii_alphanumeric() || b"/-_.~:".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

pub fn to_html(
    chat_name: &str,
    messages: &[ExportMessage],
    labels: &ExportLabels,
    offset: FixedOffset,
) -> String {
    let mut body = String::new();
    let mut last_day = String::new();
    for m in messages {
        let day = stamp(m.ts, offset, "%d/%m/%Y");
        if day != last_day {
            body.push_str(&format!("<div class=\"day\">{}</div>\n", escape_html(&day)));
            last_day = day;
        }
        let mut content = String::new();
        if m.deleted {
            content.push_str(&format!(
                "<span class=\"omitted\">{}</span>",
                escape_html(&labels.deleted)
            ));
        } else if m.kind.media().is_some() {
            match (&m.saved_path, attachment_name(m)) {
                (Some(path), Some(name)) => {
                    let href = file_href(path);
                    if m.kind == MessageKind::Image || m.kind == MessageKind::Sticker {
                        content.push_str(&format!(
                            "<a href=\"{href}\"><img src=\"{href}\" alt=\"{}\"></a>",
                            escape_html(&name)
                        ));
                    } else {
                        content.push_str(&format!(
                            "<a class=\"file\" href=\"{href}\">{}</a>",
                            escape_html(&name)
                        ));
                    }
                }
                _ => content.push_str(&format!(
                    "<span class=\"omitted\">{}</span>",
                    escape_html(&labels.media_omitted)
                )),
            }
        }
        if let Some(t) = &m.text {
            if !content.is_empty() {
                content.push_str("<br>");
            }
            content.push_str(&escape_html(t).replace('\n', "<br>"));
        }
        if m.edited && !m.deleted {
            content.push_str(&format!(
                " <span class=\"edited\">{}</span>",
                escape_html(&labels.edited)
            ));
        }
        body.push_str(&format!(
            "<div class=\"msg{}\"><div class=\"who\">{}</div><div class=\"text\">{}</div><div class=\"time\">{}</div></div>\n",
            if m.from_me { " me" } else { "" },
            escape_html(who(m, labels)),
            content,
            escape_html(&stamp(m.ts, offset, "%H:%M"))
        ));
    }
    format!(
        "<!DOCTYPE html>\n<html><head><meta charset=\"utf-8\"><title>{title}</title><style>\
body{{margin:0;background:#efeae2;font:14px/1.45 -apple-system,Segoe UI,Roboto,sans-serif;color:#111}}\
header{{position:sticky;top:0;background:#075e54;color:#fff;padding:12px 20px}}header h1{{margin:0;font-size:17px}}header p{{margin:2px 0 0;font-size:12px;opacity:.8}}\
main{{max-width:820px;margin:0 auto;padding:16px}}.day{{text-align:center;margin:14px 0 8px;font-size:12px;color:#555}}\
.msg{{max-width:75%;margin:4px 0;padding:6px 10px 4px;background:#fff;border-radius:8px;box-shadow:0 1px 1px rgba(0,0,0,.1)}}\
.msg.me{{margin-left:auto;background:#d9fdd3}}.who{{font-size:12px;font-weight:600;color:#075e54}}.time{{text-align:right;font-size:11px;color:#777}}\
.text{{white-space:normal;word-wrap:break-word}}.text img{{max-width:100%;border-radius:6px;display:block}}.omitted{{color:#777;font-style:italic}}.edited{{color:#777;font-size:11px}}\
</style></head><body><header><h1>{title}</h1><p>{count} {msgs} · {from}</p></header><main>\n{body}</main></body></html>\n",
        title = escape_html(chat_name),
        count = messages.len(),
        msgs = escape_html(&labels.messages),
        from = escape_html(&labels.exported_from),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Vec<ExportMessage> {
        vec![
            ExportMessage {
                id: "1".into(),
                ts: 1_700_000_000,
                from_me: false,
                sender: "Ana".into(),
                kind: MessageKind::Text,
                text: Some("Oi <b>tudo</b> bem?".into()),
                file_name: None,
                mime: None,
                size: None,
                saved_path: None,
                edited: false,
                deleted: false,
            },
            ExportMessage {
                id: "2".into(),
                ts: 1_700_000_060,
                from_me: true,
                sender: "me".into(),
                kind: MessageKind::Image,
                text: Some("foto".into()),
                file_name: None,
                mime: Some("image/jpeg".into()),
                size: Some(10),
                saved_path: Some("/tmp/x/2023 image.jpg".into()),
                edited: false,
                deleted: false,
            },
            ExportMessage {
                id: "3".into(),
                ts: 1_700_090_000,
                from_me: false,
                sender: "Ana".into(),
                kind: MessageKind::Voice,
                text: None,
                file_name: None,
                mime: None,
                size: None,
                saved_path: None,
                edited: false,
                deleted: false,
            },
        ]
    }

    fn brt() -> FixedOffset {
        FixedOffset::west_opt(3 * 3600).unwrap()
    }

    #[test]
    fn txt_matches_whatsapp_style() {
        let txt = to_txt(&sample(), &ExportLabels::default(), brt());
        let lines: Vec<&str> = txt.lines().collect();
        assert_eq!(lines[0], "14/11/2023 19:13 - Ana: Oi <b>tudo</b> bem?");
        assert_eq!(
            lines[1],
            "14/11/2023 19:14 - You: 2023 image.jpg (file attached)"
        );
        assert_eq!(lines[2], "foto");
        assert_eq!(lines[3], "15/11/2023 20:13 - Ana: <Media omitted>");
    }

    #[test]
    fn json_is_valid_and_complete() {
        let s = to_json("123@s.whatsapp.net", "Ana", &sample(), brt());
        let v: serde_json::Value = serde_json::from_str(&s).unwrap();
        assert_eq!(v["count"], 3);
        assert_eq!(v["messages"][1]["kind"], "image");
        assert_eq!(v["messages"][0]["date"], "2023-11-14T19:13:20-03:00");
    }

    #[test]
    fn html_escapes_and_groups_days() {
        let html = to_html("Ana & Bia", &sample(), &ExportLabels::default(), brt());
        assert!(html.contains("<title>Ana &amp; Bia</title>"));
        assert!(html.contains("Oi &lt;b&gt;tudo&lt;/b&gt; bem?"));
        assert!(!html.contains("<b>tudo"));
        assert_eq!(html.matches("class=\"day\"").count(), 2);
        assert!(html.contains("file:///tmp/x/2023%20image.jpg"));
        assert!(html.contains("&lt;Media omitted&gt;"));
    }

    #[test]
    fn edited_and_deleted_render_like_whatsapp() {
        let mut msgs = sample();
        msgs[0].edited = true;
        msgs[1].deleted = true;
        msgs[1].text = None;
        msgs[1].saved_path = None;
        let labels = ExportLabels::default();
        let txt = to_txt(&msgs, &labels, brt());
        let lines: Vec<&str> = txt.lines().collect();
        assert_eq!(
            lines[0],
            "14/11/2023 19:13 - Ana: Oi <b>tudo</b> bem? <This message was edited>"
        );
        assert_eq!(lines[1], "14/11/2023 19:14 - You: This message was deleted");
        let html = to_html("Ana", &msgs, &labels, brt());
        assert!(html.contains("<span class=\"edited\">&lt;This message was edited&gt;</span>"));
        assert!(html.contains("<span class=\"omitted\">This message was deleted</span>"));
        let v: serde_json::Value =
            serde_json::from_str(&to_json("c", "Ana", &msgs, brt())).unwrap();
        assert_eq!(v["messages"][0]["edited"], true);
        assert_eq!(v["messages"][1]["deleted"], true);
    }
}

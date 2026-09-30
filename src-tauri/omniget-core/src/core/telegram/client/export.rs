//! Chat export in the shape of Telegram Desktop's `result.json` (and a
//! plain HTML page), optionally with the media files next to it.

use std::collections::HashMap;
use std::path::PathBuf;

use grammers_client::grammers_tl_types as tl;
use serde_json::{json, Value};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use super::api;
use super::auth::TelegramSessionHandle;
use super::events;

pub struct ExportRequest {
    pub chat_id: i64,
    pub chat_type: String,
    pub chat_title: String,
    pub html: bool,
    pub include_media: bool,
    pub output_dir: PathBuf,
    /// 0 = everything.
    pub limit: usize,
}

fn iso(ts: i32) -> String {
    chrono::DateTime::from_timestamp(ts as i64, 0)
        .map(|d| d.naive_utc().format("%Y-%m-%dT%H:%M:%S").to_string())
        .unwrap_or_default()
}

fn td_type(chat_type: &str) -> &'static str {
    match chat_type {
        "private" => "personal_chat",
        "group" => "private_supergroup",
        _ => "public_channel",
    }
}

fn peer_key(p: &tl::enums::Peer) -> (String, i64) {
    match p {
        tl::enums::Peer::User(u) => ("user".into(), u.user_id),
        tl::enums::Peer::Chat(c) => ("chat".into(), c.chat_id),
        tl::enums::Peer::Channel(c) => ("channel".into(), c.channel_id),
    }
}

fn remember_names(
    names: &mut HashMap<(String, i64), String>,
    users: &[tl::enums::User],
    chats: &[tl::enums::Chat],
) {
    for u in users {
        if let tl::enums::User::User(u) = u {
            let n = [u.first_name.clone(), u.last_name.clone()]
                .into_iter()
                .flatten()
                .collect::<Vec<_>>()
                .join(" ");
            names.insert(("user".into(), u.id), n);
        }
    }
    for c in chats {
        match c {
            tl::enums::Chat::Chat(c) => {
                names.insert(("chat".into(), c.id), c.title.clone());
            }
            tl::enums::Chat::Channel(c) => {
                names.insert(("channel".into(), c.id), c.title.clone());
            }
            _ => {}
        }
    }
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// Runs the export; progress as `telegram-export-progress`
/// `{export_id, stage, messages, media_done, media_total}`.
pub async fn run(
    handle: &TelegramSessionHandle,
    export_id: u64,
    req: ExportRequest,
    cancel: &CancellationToken,
) -> anyhow::Result<(PathBuf, usize)> {
    let (client, access_hash) = {
        let g = handle.lock().await;
        let c = g
            .client
            .as_ref()
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("Not authenticated"))?;
        (c, g.peer_hashes.get(&req.chat_id).copied().unwrap_or(0))
    };
    let input_peer = api::make_input_peer(req.chat_id, &req.chat_type, access_hash);
    let dir_name = sanitize_filename::sanitize(format!(
        "ChatExport_{}_{}",
        if req.chat_title.trim().is_empty() {
            req.chat_id.to_string()
        } else {
            req.chat_title.trim().to_string()
        },
        chrono::Local::now().format("%Y-%m-%d")
    ));
    let root = req.output_dir.join(dir_name);
    std::fs::create_dir_all(&root)?;

    let progress = |stage: &str, messages: usize, media_done: usize, media_total: usize| {
        events::emit(
            "telegram-export-progress",
            json!({ "export_id": export_id, "stage": stage, "messages": messages, "media_done": media_done, "media_total": media_total }),
        );
    };

    let mut names: HashMap<(String, i64), String> = HashMap::new();
    let mut raw: Vec<tl::types::Message> = Vec::new();
    let mut offset_id = 0;
    loop {
        if cancel.is_cancelled() {
            anyhow::bail!("Export cancelled");
        }
        let hreq = tl::functions::messages::GetHistory {
            peer: input_peer.clone(),
            offset_id,
            offset_date: 0,
            add_offset: 0,
            limit: 100,
            max_id: 0,
            min_id: 0,
            hash: 0,
        };
        let res = api::invoke_with_flood_wait(&client, &hreq)
            .await
            .map_err(|e| anyhow::anyhow!("messages.getHistory: {}", e))?;
        let (msgs, users, chats) = match res {
            tl::enums::messages::Messages::Messages(m) => (m.messages, m.users, m.chats),
            tl::enums::messages::Messages::Slice(m) => (m.messages, m.users, m.chats),
            tl::enums::messages::Messages::ChannelMessages(m) => (m.messages, m.users, m.chats),
            tl::enums::messages::Messages::NotModified(_) => (vec![], vec![], vec![]),
        };
        remember_names(&mut names, &users, &chats);
        if msgs.is_empty() {
            break;
        }
        let mut min_id = i32::MAX;
        for m in msgs {
            if let tl::enums::Message::Message(m) = m {
                min_id = min_id.min(m.id);
                raw.push(m);
            } else if let tl::enums::Message::Service(s) = m {
                min_id = min_id.min(s.id);
            }
        }
        progress("messages", raw.len(), 0, 0);
        if min_id == i32::MAX || min_id <= 1 || (req.limit > 0 && raw.len() >= req.limit) {
            break;
        }
        offset_id = min_id;
    }
    if req.limit > 0 {
        raw.truncate(req.limit);
    }
    raw.sort_by_key(|m| m.id);

    let fix_ext = super::fix_file_extensions();
    let media_total = if req.include_media {
        raw.iter().filter(|m| m.media.is_some()).count()
    } else {
        0
    };
    let mut media_done = 0usize;
    let mut out_msgs: Vec<Value> = Vec::with_capacity(raw.len());
    for m in &raw {
        if cancel.is_cancelled() {
            anyhow::bail!("Export cancelled");
        }
        let (from, from_id) = match m.from_id.as_ref().or(Some(&m.peer_id)) {
            Some(p) => {
                let k = peer_key(p);
                (names.get(&k).cloned(), format!("{}{}", k.0, k.1))
            }
            None => (None, String::new()),
        };
        let mut v = json!({
            "id": m.id,
            "type": "message",
            "date": iso(m.date),
            "date_unixtime": m.date.to_string(),
            "from": from,
            "from_id": from_id,
            "text": m.message,
        });
        if let Some(ed) = m.edit_date {
            v["edited"] = json!(iso(ed));
            v["edited_unixtime"] = json!(ed.to_string());
        }
        if let Some(tl::enums::MessageReplyHeader::Header(h)) = m.reply_to.as_ref() {
            if let Some(r) = h.reply_to_msg_id {
                v["reply_to_message_id"] = json!(r);
            }
        }
        if let Some(media) = m.media.as_ref() {
            if let Some((name, size, mt)) = api::extract_raw_media_info(media, fix_ext) {
                let key = if mt == "photo" { "photo" } else { "file" };
                v["media_type"] = json!(mt);
                v["file_name"] = json!(name);
                v["file_size"] = json!(size);
                if req.include_media {
                    let folder = if mt == "photo" { "photos" } else { "files" };
                    let rel = format!("{}/{}_{}", folder, m.id, sanitize_filename::sanitize(&name));
                    let dest = root.join(&rel);
                    let (tx, mut rx) = mpsc::channel::<f64>(16);
                    let drain = tokio::spawn(async move { while rx.recv().await.is_some() {} });
                    let already = std::fs::metadata(&dest)
                        .map(|md| md.len() > 0)
                        .unwrap_or(false);
                    let res = if already {
                        Ok(0)
                    } else {
                        api::download_media_with_retry(
                            handle,
                            req.chat_id,
                            &req.chat_type,
                            m.id,
                            &dest,
                            tx,
                            cancel,
                        )
                        .await
                    };
                    let _ = drain.await;
                    match res {
                        Ok(_) => v[key] = json!(rel),
                        Err(e) => {
                            if cancel.is_cancelled() {
                                anyhow::bail!("Export cancelled");
                            }
                            v[key] = json!("(File not included. Download failed)");
                            v["download_error"] = json!(e.to_string());
                        }
                    }
                    media_done += 1;
                    progress("media", raw.len(), media_done, media_total);
                } else {
                    v[key] =
                        json!("(File not included. Change data exporting settings to download.)");
                }
            } else if let Some(wp) = api::extract_webpage_info_raw(media) {
                v["webpage"] = serde_json::to_value(wp).unwrap_or(Value::Null);
            }
        }
        out_msgs.push(v);
    }

    let doc = json!({
        "name": req.chat_title,
        "type": td_type(&req.chat_type),
        "id": req.chat_id,
        "messages": out_msgs,
    });
    let count = raw.len();
    let path = if req.html {
        let mut body = String::new();
        for m in doc["messages"].as_array().into_iter().flatten() {
            let from = m["from"].as_str().unwrap_or("");
            let text = html_escape(m["text"].as_str().unwrap_or("")).replace('\n', "<br>");
            let mut media = String::new();
            if let Some(p) = m["photo"].as_str().filter(|p| p.starts_with("photos/")) {
                media = format!(
                    "<div><img src=\"{0}\" loading=\"lazy\"></div>",
                    html_escape(p)
                );
            } else if let Some(p) = m["file"].as_str().filter(|p| p.starts_with("files/")) {
                media = format!(
                    "<div><a href=\"{0}\">{1}</a></div>",
                    html_escape(p),
                    html_escape(m["file_name"].as_str().unwrap_or(p))
                );
            } else if let Some(n) = m["file_name"].as_str() {
                media = format!("<div class=\"f\">[{}]</div>", html_escape(n));
            }
            body.push_str(&format!(
                "<div class=\"m\" id=\"message{id}\"><div class=\"h\"><b>{from}</b> <span>{date}</span></div>{media}<div>{text}</div></div>\n",
                id = m["id"],
                from = html_escape(from),
                date = html_escape(m["date"].as_str().unwrap_or("")),
            ));
        }
        let html = format!(
            "<!doctype html><html><head><meta charset=\"utf-8\"><title>{t}</title><style>body{{font:15px system-ui;max-width:760px;margin:24px auto;padding:0 16px;background:#fff;color:#111}}@media(prefers-color-scheme:dark){{body{{background:#17212b;color:#e6e6e6}}}}.m{{padding:10px 0;border-bottom:1px solid #8883}}.h span{{opacity:.6;font-size:12px}}img{{max-width:100%;border-radius:8px}}.f{{opacity:.7}}</style></head><body><h1>{t}</h1>\n{body}</body></html>",
            t = html_escape(&req.chat_title),
        );
        let p = root.join("messages.html");
        std::fs::write(&p, html)?;
        // The JSON goes along too: it is the machine-readable copy.
        std::fs::write(
            root.join("result.json"),
            serde_json::to_string_pretty(&doc)?,
        )?;
        p
    } else {
        let p = root.join("result.json");
        std::fs::write(&p, serde_json::to_string_pretty(&doc)?)?;
        p
    };
    progress("done", count, media_done, media_total);
    Ok((path, count))
}

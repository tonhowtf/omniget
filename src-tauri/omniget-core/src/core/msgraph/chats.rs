use std::path::{Path, PathBuf};

use anyhow::anyhow;
use serde::Serialize;
use serde_json::Value;

use super::html::to_text;

#[derive(Debug, Clone, Serialize)]
pub struct ChatInfo {
    pub id: String,
    pub kind: String,
    pub name: String,
    pub members: Vec<String>,
    pub last_updated: Option<String>,
    pub web_url: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Message {
    pub id: String,
    pub created: String,
    pub author: String,
    pub text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reply_to: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub files: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub reactions: Vec<String>,
    pub system: bool,
    pub edited: bool,
}

fn me_id() -> String {
    super::load_config()
        .account
        .map(|a| a.id)
        .unwrap_or_default()
}

fn chat_info(v: &Value, me: &str) -> ChatInfo {
    let members: Vec<String> = v["members"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|m| m["userId"].as_str() != Some(me))
        .filter_map(|m| m["displayName"].as_str().map(str::to_string))
        .collect();
    let kind = v["chatType"].as_str().unwrap_or("").to_string();
    let topic = v["topic"].as_str().map(str::trim).filter(|t| !t.is_empty());
    let name = topic.map(str::to_string).unwrap_or_else(|| {
        if members.is_empty() {
            "(chat)".to_string()
        } else {
            members.join(", ")
        }
    });
    ChatInfo {
        id: v["id"].as_str().unwrap_or("").to_string(),
        kind,
        name,
        members,
        last_updated: v["lastUpdatedDateTime"].as_str().map(str::to_string),
        web_url: v["webUrl"].as_str().map(str::to_string),
    }
}

pub async fn list_chats(limit: usize, query: Option<&str>) -> anyhow::Result<Vec<ChatInfo>> {
    let me = me_id();
    let raw = super::get_paged(
        "/me/chats?$expand=members&$top=50&$orderby=lastMessagePreview/createdDateTime desc",
        0,
        |_| false,
    )
    .await?;
    let q = query
        .map(|q| q.to_lowercase())
        .filter(|q| !q.trim().is_empty());
    let mut out: Vec<ChatInfo> = raw
        .iter()
        .map(|v| chat_info(v, &me))
        .filter(|c| match &q {
            Some(q) => {
                c.name.to_lowercase().contains(q)
                    || c.members.iter().any(|m| m.to_lowercase().contains(q))
            }
            None => true,
        })
        .collect();
    out.sort_by(|a, b| b.last_updated.cmp(&a.last_updated));
    out.truncate(limit.max(1));
    Ok(out)
}

pub async fn chat(chat_id: &str) -> anyhow::Result<ChatInfo> {
    let v = super::get(&format!(
        "/me/chats/{}?$expand=members",
        urlencoding::encode(chat_id)
    ))
    .await?;
    Ok(chat_info(&v, &me_id()))
}

fn message(v: &Value) -> Option<Message> {
    if v["deletedDateTime"].is_string() {
        return None;
    }
    let system = v["messageType"].as_str().is_some_and(|t| t != "message");
    let author = v["from"]["user"]["displayName"]
        .as_str()
        .or_else(|| v["from"]["application"]["displayName"].as_str())
        .unwrap_or(if system { "system" } else { "unknown" })
        .to_string();
    let body = v["body"]["content"].as_str().unwrap_or("");
    let mut text = if v["body"]["contentType"].as_str() == Some("html") {
        to_text(body)
    } else {
        body.trim().to_string()
    };
    if system && text.is_empty() {
        text = v["eventDetail"]["@odata.type"]
            .as_str()
            .map(|t| {
                t.trim_start_matches("#microsoft.graph.")
                    .replace("EventMessageDetail", "")
            })
            .unwrap_or_default();
    }
    let files: Vec<String> = v["attachments"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|a| a["contentType"].as_str() == Some("reference"))
        .map(|a| {
            let name = a["name"].as_str().unwrap_or("file");
            match a["contentUrl"].as_str() {
                Some(u) => format!("{name} ({u})"),
                None => name.to_string(),
            }
        })
        .collect();
    let reply_to = v["attachments"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|a| a["contentType"].as_str() == Some("messageReference"))
        .and_then(|a| a["id"].as_str().map(str::to_string))
        .or_else(|| v["replyToId"].as_str().map(str::to_string));
    let reactions: Vec<String> = v["reactions"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|r| {
            let kind = r["displayName"]
                .as_str()
                .or_else(|| r["reactionType"].as_str())?;
            let who = r["user"]["user"]["displayName"].as_str().unwrap_or("");
            Some(if who.is_empty() {
                kind.to_string()
            } else {
                format!("{kind} ({who})")
            })
        })
        .collect();
    if text.is_empty() && files.is_empty() {
        return None;
    }
    Some(Message {
        id: v["id"].as_str().unwrap_or("").to_string(),
        created: v["createdDateTime"].as_str().unwrap_or("").to_string(),
        author,
        text,
        reply_to,
        files,
        reactions,
        system,
        edited: v["lastEditedDateTime"].is_string(),
    })
}

fn older_than(v: &Value, since: Option<&chrono::DateTime<chrono::Utc>>) -> bool {
    let Some(since) = since else { return false };
    v["createdDateTime"]
        .as_str()
        .and_then(|t| chrono::DateTime::parse_from_rfc3339(t).ok())
        .is_some_and(|t| t.with_timezone(&chrono::Utc) < *since)
}

pub async fn chat_messages(
    chat_id: &str,
    since: Option<chrono::DateTime<chrono::Utc>>,
    max: usize,
) -> anyhow::Result<Vec<Message>> {
    let path = format!(
        "/me/chats/{}/messages?$top=50&$orderby=createdDateTime desc",
        urlencoding::encode(chat_id)
    );
    let raw = super::get_paged(&path, max, |v| older_than(v, since.as_ref())).await?;
    let mut out: Vec<Message> = raw.iter().filter_map(message).collect();
    out.reverse();
    Ok(out)
}

#[derive(Debug, Clone, Serialize)]
pub struct ChannelInfo {
    pub team_id: String,
    pub team: String,
    pub channel_id: String,
    pub channel: String,
}

pub async fn list_channels() -> anyhow::Result<Vec<ChannelInfo>> {
    let teams = super::get_paged("/me/joinedTeams", 0, |_| false).await?;
    let mut out = Vec::new();
    for t in teams {
        let team_id = t["id"].as_str().unwrap_or("").to_string();
        let team = t["displayName"].as_str().unwrap_or("").to_string();
        let chans = super::get_paged(&format!("/teams/{team_id}/channels"), 0, |_| false).await?;
        for c in chans {
            out.push(ChannelInfo {
                team_id: team_id.clone(),
                team: team.clone(),
                channel_id: c["id"].as_str().unwrap_or("").to_string(),
                channel: c["displayName"].as_str().unwrap_or("").to_string(),
            });
        }
    }
    Ok(out)
}

pub async fn channel_messages(
    team_id: &str,
    channel_id: &str,
    since: Option<chrono::DateTime<chrono::Utc>>,
    max: usize,
) -> anyhow::Result<Vec<Message>> {
    let base = format!(
        "/teams/{}/channels/{}/messages",
        urlencoding::encode(team_id),
        urlencoding::encode(channel_id)
    );
    let posts = super::get_paged(&format!("{base}?$top=50&$expand=replies"), max, |v| {
        older_than(v, since.as_ref())
    })
    .await?;
    let mut out = Vec::new();
    for p in posts.iter().rev() {
        let Some(root) = message(p) else { continue };
        let root_id = root.id.clone();
        out.push(root);
        let mut replies: Vec<Message> = p["replies"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(message)
            .map(|mut r| {
                r.reply_to = Some(root_id.clone());
                r
            })
            .collect();
        replies.sort_by(|a, b| a.created.cmp(&b.created));
        out.extend(replies);
    }
    Ok(out)
}

fn local_time(iso: &str) -> String {
    chrono::DateTime::parse_from_rfc3339(iso)
        .map(|d| {
            d.with_timezone(&chrono::Local)
                .format("%Y-%m-%d %H:%M")
                .to_string()
        })
        .unwrap_or_else(|_| iso.to_string())
}

fn yaml(s: &str) -> String {
    format!(
        "\"{}\"",
        s.replace('\\', "\\\\")
            .replace('"', "\\\"")
            .replace('\n', " ")
    )
}

pub fn to_markdown(title: &str, kind: &str, members: &[String], messages: &[Message]) -> String {
    let mut md = String::from("---\n");
    md.push_str(&format!("title: {}\n", yaml(title)));
    md.push_str(&format!("type: teams-{kind}\n"));
    if !members.is_empty() {
        md.push_str("members:\n");
        for m in members {
            md.push_str(&format!("  - {}\n", yaml(m)));
        }
    }
    md.push_str(&format!("messages: {}\n", messages.len()));
    if let (Some(first), Some(last)) = (messages.first(), messages.last()) {
        md.push_str(&format!("from: {}\nto: {}\n", first.created, last.created));
    }
    md.push_str(&format!("exported: {}\n", chrono::Utc::now().to_rfc3339()));
    md.push_str("---\n\n");
    md.push_str(&format!("# {title}\n\n"));
    let ids: std::collections::HashSet<&str> = messages.iter().map(|m| m.id.as_str()).collect();
    let mut last_day = String::new();
    for m in messages {
        let t = local_time(&m.created);
        let day = t.get(..10).unwrap_or("").to_string();
        if day != last_day {
            md.push_str(&format!("## {day}\n\n"));
            last_day = day;
        }
        let indent = if m.reply_to.as_deref().is_some_and(|r| ids.contains(r)) {
            "> "
        } else {
            ""
        };
        let time = t.get(11..).unwrap_or(&t);
        if m.system {
            md.push_str(&format!("{indent}_[{time}] {}_\n\n", m.text));
            continue;
        }
        let mut body = m.text.replace('\n', &format!("\n{indent}"));
        if m.edited {
            body.push_str(" _(edited)_");
        }
        md.push_str(&format!("{indent}**[{time}] {}:** {body}\n", m.author));
        for f in &m.files {
            md.push_str(&format!("{indent}- 📎 {f}\n"));
        }
        if !m.reactions.is_empty() {
            md.push_str(&format!(
                "{indent}_reactions: {}_\n",
                m.reactions.join(", ")
            ));
        }
        md.push('\n');
    }
    md
}

pub fn export_dir() -> anyhow::Result<PathBuf> {
    Ok(crate::core::paths::app_data_dir()
        .ok_or_else(|| anyhow!("Could not determine data directory"))?
        .join("teams-export"))
}

fn slug(s: &str) -> String {
    let mut out: String = s
        .chars()
        .map(|c| {
            if c.is_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    while out.contains("--") {
        out = out.replace("--", "-");
    }
    let out = out.trim_matches('-');
    let out: String = out.chars().take(60).collect();
    if out.is_empty() {
        "chat".into()
    } else {
        out
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Exported {
    pub name: String,
    pub path: String,
    pub messages: usize,
}

pub async fn export_chat(
    chat_id: &str,
    since: Option<chrono::DateTime<chrono::Utc>>,
    json: bool,
    dir: &Path,
) -> anyhow::Result<Exported> {
    let info = chat(chat_id).await?;
    let msgs = chat_messages(chat_id, since, 0).await?;
    std::fs::create_dir_all(dir)?;
    let tag: String = slug(chat_id.trim_start_matches("19:"))
        .chars()
        .take(8)
        .collect();
    let base = format!("{}-{tag}", slug(&info.name));
    let path = if json {
        let p = dir.join(format!("{base}.json"));
        std::fs::write(
            &p,
            serde_json::to_vec_pretty(&serde_json::json!({ "chat": info, "messages": msgs }))?,
        )?;
        p
    } else {
        let p = dir.join(format!("{base}.md"));
        std::fs::write(
            &p,
            to_markdown(&info.name, &info.kind, &info.members, &msgs),
        )?;
        p
    };
    Ok(Exported {
        name: info.name,
        path: path.to_string_lossy().to_string(),
        messages: msgs.len(),
    })
}

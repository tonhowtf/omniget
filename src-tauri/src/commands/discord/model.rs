use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const DISCORD_EPOCH_MS: u64 = 1_420_070_400_000;

pub const VIEW_CHANNEL: u64 = 1 << 10;
pub const SEND_MESSAGES: u64 = 1 << 11;
pub const READ_HISTORY: u64 = 1 << 16;
pub const ADMINISTRATOR: u64 = 1 << 3;
pub const SEND_IN_THREADS: u64 = 1 << 38;

pub fn snowflake_ms(id: &str) -> Option<u64> {
    id.parse::<u64>().ok().map(|n| (n >> 22) + DISCORD_EPOCH_MS)
}

pub fn snowflake_at(ms: u64) -> u64 {
    ms.saturating_sub(DISCORD_EPOCH_MS) << 22
}

pub fn id_of(v: &Value) -> Option<String> {
    match v {
        Value::String(s) if !s.is_empty() => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

fn str_of(v: &Value, k: &str) -> Option<String> {
    v.get(k).and_then(|x| x.as_str()).map(str::to_string)
}

fn u64_of(v: &Value) -> u64 {
    match v {
        Value::String(s) => s.parse().unwrap_or(0),
        Value::Number(n) => n.as_u64().unwrap_or(0),
        _ => 0,
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct User {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub username: String,
    #[serde(default)]
    pub global_name: Option<String>,
    #[serde(default)]
    pub avatar: Option<String>,
    #[serde(default)]
    pub bot: bool,
}

impl User {
    pub fn display(&self) -> &str {
        self.global_name
            .as_deref()
            .filter(|s| !s.is_empty())
            .unwrap_or(&self.username)
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct Attachment {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub filename: String,
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub proxy_url: Option<String>,
    #[serde(default)]
    pub size: u64,
    #[serde(default)]
    pub content_type: Option<String>,
    #[serde(default)]
    pub width: Option<u32>,
    #[serde(default)]
    pub height: Option<u32>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct EmbedMedia {
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub proxy_url: Option<String>,
    #[serde(default)]
    pub width: Option<u32>,
    #[serde(default)]
    pub height: Option<u32>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct EmbedAuthor {
    #[serde(default)]
    pub name: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct Embed {
    #[serde(default, rename = "type")]
    pub kind: Option<String>,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub color: Option<u32>,
    #[serde(default)]
    pub author: Option<EmbedAuthor>,
    #[serde(default)]
    pub image: Option<EmbedMedia>,
    #[serde(default)]
    pub thumbnail: Option<EmbedMedia>,
    #[serde(default)]
    pub video: Option<EmbedMedia>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct Sticker {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub name: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct Message {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub channel_id: String,
    #[serde(default)]
    pub guild_id: Option<String>,
    #[serde(default)]
    pub author: User,
    #[serde(default)]
    pub content: String,
    #[serde(default)]
    pub timestamp: String,
    #[serde(default)]
    pub edited_timestamp: Option<String>,
    #[serde(default)]
    pub attachments: Vec<Attachment>,
    #[serde(default)]
    pub embeds: Vec<Embed>,
    #[serde(default)]
    pub mentions: Vec<User>,
    #[serde(default)]
    pub mention_roles: Vec<String>,
    #[serde(default)]
    pub mention_everyone: bool,
    #[serde(default, rename = "type")]
    pub kind: u32,
    #[serde(default)]
    pub sticker_items: Vec<Sticker>,
    #[serde(default)]
    pub referenced_message: Option<Box<Message>>,
}

impl Message {
    pub fn from_value(v: &Value) -> Option<Self> {
        let mut m: Message = serde_json::from_value(v.clone()).ok()?;
        if m.id.is_empty() {
            return None;
        }
        if let Some(r) = m.referenced_message.as_mut() {
            r.referenced_message = None;
            r.embeds.clear();
            if r.content.chars().count() > 200 {
                r.content = r.content.chars().take(200).collect();
            }
        }
        Some(m)
    }

    pub fn list_from(v: &Value) -> Vec<Message> {
        v.as_array()
            .map(|a| a.iter().filter_map(Message::from_value).collect())
            .unwrap_or_default()
    }
}

#[derive(Serialize, Clone, Debug, Default, PartialEq)]
pub struct Channel {
    pub id: String,
    pub guild_id: Option<String>,
    pub kind: u32,
    pub name: String,
    pub position: i64,
    pub parent_id: Option<String>,
    pub last_message_id: Option<String>,
    pub topic: Option<String>,
    pub can_send: bool,
    pub icon: Option<String>,
    pub recipients: Vec<User>,
}

#[derive(Serialize, Clone, Debug, Default, PartialEq)]
pub struct Guild {
    pub id: String,
    pub name: String,
    pub icon: Option<String>,
    pub channels: Vec<Channel>,
}

#[derive(Serialize, Clone, Debug, Default, PartialEq)]
pub struct ReadState {
    pub last_message_id: Option<String>,
    pub mention_count: u32,
}

#[derive(Serialize, Clone, Debug, Default)]
pub struct Snapshot {
    pub me: User,
    pub guilds: Vec<Guild>,
    pub dms: Vec<Channel>,
    pub read_states: HashMap<String, ReadState>,
}

impl Snapshot {
    pub fn channel(&self, id: &str) -> Option<&Channel> {
        self.dms.iter().find(|c| c.id == id).or_else(|| {
            self.guilds
                .iter()
                .flat_map(|g| g.channels.iter())
                .find(|c| c.id == id)
        })
    }

    pub fn channel_mut(&mut self, id: &str) -> Option<&mut Channel> {
        if let Some(i) = self.dms.iter().position(|c| c.id == id) {
            return self.dms.get_mut(i);
        }
        self.guilds
            .iter_mut()
            .flat_map(|g| g.channels.iter_mut())
            .find(|c| c.id == id)
    }

    pub fn guild_name(&self, id: &str) -> Option<&str> {
        self.guilds
            .iter()
            .find(|g| g.id == id)
            .map(|g| g.name.as_str())
    }

    pub fn upsert_guild(&mut self, g: Guild) {
        match self.guilds.iter_mut().find(|x| x.id == g.id) {
            Some(slot) => *slot = g,
            None => self.guilds.push(g),
        }
    }

    pub fn channel_title(&self, id: &str) -> (String, String) {
        let Some(c) = self.channel(id) else {
            return ("Discord".into(), id.to_string());
        };
        let place = match &c.guild_id {
            Some(g) => self.guild_name(g).unwrap_or("Server").to_string(),
            None => "Direct Messages".into(),
        };
        (place, c.name.clone())
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Overwrite {
    pub id: String,
    pub kind: u8,
    pub allow: u64,
    pub deny: u64,
}

pub fn overwrites_of(v: &Value) -> Vec<Overwrite> {
    v.get("permission_overwrites")
        .and_then(|x| x.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|o| {
                    Some(Overwrite {
                        id: id_of(o.get("id")?)?,
                        kind: match o.get("type") {
                            Some(Value::String(s)) if s == "member" => 1,
                            Some(t) => u64_of(t) as u8,
                            None => 0,
                        },
                        allow: o.get("allow").map(u64_of).unwrap_or(0),
                        deny: o.get("deny").map(u64_of).unwrap_or(0),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

pub struct Me<'a> {
    pub user_id: &'a str,
    pub roles: &'a [String],
    pub is_owner: bool,
}

pub fn base_permissions(guild_id: &str, roles: &HashMap<String, u64>, me: &Me) -> u64 {
    if me.is_owner {
        return u64::MAX;
    }
    let mut perms = roles.get(guild_id).copied().unwrap_or(0);
    for r in me.roles {
        perms |= roles.get(r).copied().unwrap_or(0);
    }
    if perms & ADMINISTRATOR != 0 {
        return u64::MAX;
    }
    perms
}

pub fn channel_permissions(base: u64, guild_id: &str, overwrites: &[Overwrite], me: &Me) -> u64 {
    if base == u64::MAX {
        return base;
    }
    let mut perms = base;
    if let Some(o) = overwrites.iter().find(|o| o.id == guild_id) {
        perms = (perms & !o.deny) | o.allow;
    }
    let (mut allow, mut deny) = (0u64, 0u64);
    for o in overwrites
        .iter()
        .filter(|o| o.kind == 0 && me.roles.contains(&o.id))
    {
        allow |= o.allow;
        deny |= o.deny;
    }
    perms = (perms & !deny) | allow;
    if let Some(o) = overwrites
        .iter()
        .find(|o| o.kind == 1 && o.id == me.user_id)
    {
        perms = (perms & !o.deny) | o.allow;
    }
    perms
}

fn is_text_kind(kind: u32) -> bool {
    matches!(kind, 0 | 5)
}

fn is_thread_kind(kind: u32) -> bool {
    matches!(kind, 10..=12)
}

pub fn parse_guild(v: &Value, me_id: &str, my_roles: Option<&[String]>) -> Option<Guild> {
    if v.get("unavailable").and_then(|x| x.as_bool()) == Some(true) {
        return None;
    }
    let id = id_of(v.get("id")?)?;
    let props = v.get("properties").unwrap_or(v);
    let name = str_of(props, "name").or_else(|| str_of(v, "name"))?;
    let icon = str_of(props, "icon").or_else(|| str_of(v, "icon"));
    let owner = props
        .get("owner_id")
        .or_else(|| v.get("owner_id"))
        .and_then(id_of);
    let roles: HashMap<String, u64> = v
        .get("roles")
        .or_else(|| props.get("roles"))
        .and_then(|x| x.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|r| {
                    Some((
                        id_of(r.get("id")?)?,
                        r.get("permissions").map(u64_of).unwrap_or(0),
                    ))
                })
                .collect()
        })
        .unwrap_or_default();
    let member_roles = my_roles.map(<[String]>::to_vec).or_else(|| {
        v.get("members").and_then(|x| x.as_array()).and_then(|a| {
            a.iter()
                .find(|m| {
                    m.get("user")
                        .and_then(|u| u.get("id"))
                        .or_else(|| m.get("user_id"))
                        .and_then(id_of)
                        .as_deref()
                        == Some(me_id)
                })
                .map(roles_of_member)
        })
    });
    let role_list = member_roles.clone().unwrap_or_default();
    let me = Me {
        user_id: me_id,
        roles: &role_list,
        is_owner: owner.as_deref() == Some(me_id),
    };
    let base = if member_roles.is_some() || me.is_owner {
        base_permissions(&id, &roles, &me)
    } else {
        u64::MAX
    };

    let raw_channels: Vec<&Value> = v
        .get("channels")
        .and_then(|x| x.as_array())
        .map(|a| a.iter().collect())
        .unwrap_or_default();
    let mut categories: Vec<(String, String, i64)> = Vec::new();
    let mut text: Vec<Channel> = Vec::new();
    let mut perms_of: HashMap<String, u64> = HashMap::new();
    for c in &raw_channels {
        let Some(cid) = c.get("id").and_then(id_of) else {
            continue;
        };
        let kind = c.get("type").map(u64_of).unwrap_or(0) as u32;
        let p = channel_permissions(base, &id, &overwrites_of(c), &me);
        perms_of.insert(cid.clone(), p);
        let name = str_of(c, "name").unwrap_or_default();
        let position = c.get("position").and_then(|x| x.as_i64()).unwrap_or(0);
        if kind == 4 {
            categories.push((cid, name, position));
            continue;
        }
        if !is_text_kind(kind) || p & VIEW_CHANNEL == 0 || p & READ_HISTORY == 0 {
            continue;
        }
        text.push(Channel {
            id: cid,
            guild_id: Some(id.clone()),
            kind,
            name,
            position,
            parent_id: c.get("parent_id").and_then(id_of),
            last_message_id: c.get("last_message_id").and_then(id_of),
            topic: str_of(c, "topic"),
            can_send: p & SEND_MESSAGES != 0,
            icon: None,
            recipients: Vec::new(),
        });
    }
    let mut threads: Vec<Channel> = v
        .get("threads")
        .and_then(|x| x.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|t| {
                    let kind = t.get("type").map(u64_of).unwrap_or(0) as u32;
                    if !is_thread_kind(kind) {
                        return None;
                    }
                    let parent = t.get("parent_id").and_then(id_of)?;
                    if !text.iter().any(|c| c.id == parent) {
                        return None;
                    }
                    let p = perms_of.get(&parent).copied().unwrap_or(0);
                    Some(Channel {
                        id: id_of(t.get("id")?)?,
                        guild_id: Some(id.clone()),
                        kind,
                        name: str_of(t, "name").unwrap_or_default(),
                        position: 0,
                        parent_id: Some(parent),
                        last_message_id: t.get("last_message_id").and_then(id_of),
                        topic: None,
                        can_send: p & SEND_IN_THREADS != 0 || p == u64::MAX,
                        icon: None,
                        recipients: Vec::new(),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    threads.sort_by(|a, b| snowflake_cmp(&b.last_message_id, &a.last_message_id));

    categories.sort_by_key(|(cid, _, pos)| (*pos, cid.parse::<u64>().unwrap_or(0)));
    let order = |list: &mut Vec<Channel>| {
        list.sort_by_key(|c| (c.position, c.id.parse::<u64>().unwrap_or(0)));
    };
    let mut out: Vec<Channel> = Vec::new();
    let push_with_threads = |out: &mut Vec<Channel>, c: Channel, threads: &[Channel]| {
        let cid = c.id.clone();
        out.push(c);
        out.extend(
            threads
                .iter()
                .filter(|t| t.parent_id.as_deref() == Some(&cid))
                .cloned(),
        );
    };
    let mut loose: Vec<Channel> = text
        .iter()
        .filter(|c| {
            c.parent_id
                .as_ref()
                .is_none_or(|p| !categories.iter().any(|(cid, _, _)| cid == p))
        })
        .cloned()
        .collect();
    order(&mut loose);
    for c in loose {
        push_with_threads(&mut out, c, &threads);
    }
    for (cid, cname, pos) in &categories {
        let mut kids: Vec<Channel> = text
            .iter()
            .filter(|c| c.parent_id.as_deref() == Some(cid))
            .cloned()
            .collect();
        if kids.is_empty() {
            continue;
        }
        order(&mut kids);
        out.push(Channel {
            id: cid.clone(),
            guild_id: Some(id.clone()),
            kind: 4,
            name: cname.clone(),
            position: *pos,
            ..Default::default()
        });
        for c in kids {
            push_with_threads(&mut out, c, &threads);
        }
    }
    Some(Guild {
        id,
        name,
        icon,
        channels: out,
    })
}

fn roles_of_member(m: &Value) -> Vec<String> {
    m.get("roles")
        .and_then(|r| r.as_array())
        .map(|a| a.iter().filter_map(id_of).collect())
        .unwrap_or_default()
}

pub fn snowflake_cmp(a: &Option<String>, b: &Option<String>) -> std::cmp::Ordering {
    let n = |x: &Option<String>| {
        x.as_deref()
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(0)
    };
    n(a).cmp(&n(b))
}

pub fn parse_user(v: &Value) -> Option<User> {
    serde_json::from_value::<User>(v.clone())
        .ok()
        .filter(|u| !u.id.is_empty())
}

pub fn parse_dm(v: &Value, users: &HashMap<String, User>) -> Option<Channel> {
    let kind = v.get("type").map(u64_of).unwrap_or(1) as u32;
    if kind != 1 && kind != 3 {
        return None;
    }
    let mut recipients: Vec<User> = v
        .get("recipients")
        .and_then(|x| x.as_array())
        .map(|a| a.iter().filter_map(parse_user).collect())
        .unwrap_or_default();
    if recipients.is_empty() {
        recipients = v
            .get("recipient_ids")
            .and_then(|x| x.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(id_of)
                    .map(|id| {
                        users.get(&id).cloned().unwrap_or(User {
                            id,
                            ..Default::default()
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();
    }
    let name = str_of(v, "name")
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| {
            let names: Vec<&str> = recipients
                .iter()
                .map(User::display)
                .filter(|n| !n.is_empty())
                .collect();
            if names.is_empty() {
                "Unnamed".into()
            } else {
                names.join(", ")
            }
        });
    Some(Channel {
        id: id_of(v.get("id")?)?,
        guild_id: None,
        kind,
        name,
        position: 0,
        parent_id: None,
        last_message_id: v.get("last_message_id").and_then(id_of),
        topic: None,
        can_send: true,
        icon: str_of(v, "icon"),
        recipients,
    })
}

pub fn parse_read_states(v: Option<&Value>) -> HashMap<String, ReadState> {
    let Some(v) = v else {
        return HashMap::new();
    };
    let list = v
        .as_array()
        .or_else(|| v.get("entries").and_then(|e| e.as_array()));
    let mut out = HashMap::new();
    for e in list.into_iter().flatten() {
        if e.get("read_state_type").map(u64_of).unwrap_or(0) != 0 {
            continue;
        }
        let Some(id) = e.get("id").and_then(id_of) else {
            continue;
        };
        out.insert(
            id,
            ReadState {
                last_message_id: e
                    .get("last_message_id")
                    .and_then(id_of)
                    .filter(|s| s != "0"),
                mention_count: e.get("mention_count").map(u64_of).unwrap_or(0) as u32,
            },
        );
    }
    out
}

pub fn parse_ready(d: &Value) -> Snapshot {
    let me = d.get("user").and_then(parse_user).unwrap_or_default();
    let users: HashMap<String, User> = d
        .get("users")
        .and_then(|x| x.as_array())
        .map(|a| {
            a.iter()
                .filter_map(parse_user)
                .map(|u| (u.id.clone(), u))
                .collect()
        })
        .unwrap_or_default();
    let merged = d.get("merged_members").and_then(|x| x.as_array());
    let guilds = d
        .get("guilds")
        .and_then(|x| x.as_array())
        .map(|a| {
            a.iter()
                .enumerate()
                .filter_map(|(i, g)| {
                    let roles = merged.and_then(|m| m.get(i)).and_then(|list| {
                        list.as_array()?
                            .iter()
                            .find(|m| {
                                m.get("user_id").and_then(id_of).as_deref() == Some(me.id.as_str())
                            })
                            .map(roles_of_member)
                    });
                    parse_guild(g, &me.id, roles.as_deref())
                })
                .collect()
        })
        .unwrap_or_default();
    let mut dms: Vec<Channel> = d
        .get("private_channels")
        .and_then(|x| x.as_array())
        .map(|a| a.iter().filter_map(|c| parse_dm(c, &users)).collect())
        .unwrap_or_default();
    dms.sort_by(|a, b| snowflake_cmp(&b.last_message_id, &a.last_message_id));
    Snapshot {
        me,
        guilds,
        dms,
        read_states: parse_read_states(d.get("read_state")),
    }
}

#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct MediaItem {
    pub url: String,
    pub filename: String,
    pub kind: &'static str,
    pub size: u64,
    pub message_id: String,
    pub timestamp_ms: u64,
}

pub fn kind_of(content_type: Option<&str>, filename: &str) -> &'static str {
    if let Some(ct) = content_type {
        if ct.starts_with("image/") {
            return "image";
        }
        if ct.starts_with("video/") {
            return "video";
        }
        if ct.starts_with("audio/") {
            return "audio";
        }
    }
    let ext = filename
        .rsplit('.')
        .next()
        .unwrap_or("")
        .to_ascii_lowercase();
    match ext.as_str() {
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "avif" | "bmp" | "heic" => "image",
        "mp4" | "webm" | "mov" | "mkv" | "m4v" | "avi" => "video",
        "mp3" | "ogg" | "wav" | "flac" | "m4a" | "opus" | "aac" => "audio",
        _ => "file",
    }
}

fn name_from_url(url: &str) -> String {
    url.split(['?', '#'])
        .next()
        .unwrap_or(url)
        .rsplit('/')
        .next()
        .filter(|s| !s.is_empty())
        .unwrap_or("file")
        .to_string()
}

pub fn media_of(m: &Message, include_embeds: bool) -> Vec<MediaItem> {
    let ts = snowflake_ms(&m.id).unwrap_or(0);
    let mut out: Vec<MediaItem> = m
        .attachments
        .iter()
        .filter(|a| !a.url.is_empty())
        .map(|a| MediaItem {
            url: a.url.clone(),
            filename: if a.filename.is_empty() {
                name_from_url(&a.url)
            } else {
                a.filename.clone()
            },
            kind: kind_of(a.content_type.as_deref(), &a.filename),
            size: a.size,
            message_id: m.id.clone(),
            timestamp_ms: ts,
        })
        .collect();
    if include_embeds {
        for e in &m.embeds {
            let media = match e.kind.as_deref() {
                Some("gifv") | Some("video") => e.video.as_ref().or(e.thumbnail.as_ref()),
                Some("image") => e.image.as_ref().or(e.thumbnail.as_ref()),
                _ => e.image.as_ref(),
            };
            let Some(media) = media else {
                continue;
            };
            let url = media
                .proxy_url
                .clone()
                .filter(|u| u.starts_with("https://"))
                .unwrap_or_else(|| media.url.clone());
            if !url.starts_with("https://") || out.iter().any(|o| o.url == url) {
                continue;
            }
            let filename = name_from_url(&url);
            out.push(MediaItem {
                kind: kind_of(None, &filename),
                filename,
                url,
                size: 0,
                message_id: m.id.clone(),
                timestamp_ms: ts,
            });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn snowflake_round_trip() {
        let id = "175928847299117063";
        assert_eq!(snowflake_ms(id), Some(1_462_015_105_796));
        let back = snowflake_at(1_462_015_105_796);
        assert_eq!(snowflake_ms(&back.to_string()), Some(1_462_015_105_796));
        assert_eq!(snowflake_at(0), 0);
    }

    #[test]
    fn permissions_follow_overwrite_order() {
        let mut roles = HashMap::new();
        roles.insert("g".to_string(), VIEW_CHANNEL | SEND_MESSAGES | READ_HISTORY);
        roles.insert("mod".to_string(), 0);
        let my_roles = vec!["mod".to_string()];
        let me = Me {
            user_id: "u",
            roles: &my_roles,
            is_owner: false,
        };
        let base = base_permissions("g", &roles, &me);
        assert_eq!(base, VIEW_CHANNEL | SEND_MESSAGES | READ_HISTORY);
        let hidden = vec![Overwrite {
            id: "g".into(),
            kind: 0,
            allow: 0,
            deny: VIEW_CHANNEL,
        }];
        assert_eq!(
            channel_permissions(base, "g", &hidden, &me) & VIEW_CHANNEL,
            0
        );
        let role_allows = vec![
            Overwrite {
                id: "g".into(),
                kind: 0,
                allow: 0,
                deny: VIEW_CHANNEL,
            },
            Overwrite {
                id: "mod".into(),
                kind: 0,
                allow: VIEW_CHANNEL,
                deny: SEND_MESSAGES,
            },
        ];
        let p = channel_permissions(base, "g", &role_allows, &me);
        assert_ne!(p & VIEW_CHANNEL, 0);
        assert_eq!(p & SEND_MESSAGES, 0);
        let member_denies = vec![Overwrite {
            id: "u".into(),
            kind: 1,
            allow: 0,
            deny: VIEW_CHANNEL,
        }];
        assert_eq!(
            channel_permissions(base, "g", &member_denies, &me) & VIEW_CHANNEL,
            0
        );
        roles.insert("mod".to_string(), ADMINISTRATOR);
        let admin = base_permissions("g", &roles, &me);
        assert_eq!(channel_permissions(admin, "g", &hidden, &me), u64::MAX);
    }

    #[test]
    fn ready_parses_guilds_dms_and_read_state() {
        let d = json!({
            "user": {"id": "1", "username": "me"},
            "users": [{"id": "2", "username": "friend", "global_name": "Friend"}],
            "guilds": [{
                "id": "10",
                "properties": {"name": "Guild", "icon": "abc", "owner_id": "9"},
                "roles": [{"id": "10", "permissions": (VIEW_CHANNEL | READ_HISTORY | SEND_MESSAGES).to_string()}],
                "channels": [
                    {"id": "11", "type": 4, "name": "Info", "position": 0},
                    {"id": "12", "type": 0, "name": "rules", "position": 1, "parent_id": "11",
                     "permission_overwrites": [{"id": "10", "type": 0, "allow": "0", "deny": SEND_MESSAGES.to_string()}]},
                    {"id": "13", "type": 0, "name": "secret", "position": 2, "parent_id": "11",
                     "permission_overwrites": [{"id": "10", "type": 0, "allow": "0", "deny": VIEW_CHANNEL.to_string()}]},
                    {"id": "14", "type": 2, "name": "voice", "position": 3},
                    {"id": "15", "type": 0, "name": "general", "position": 0, "last_message_id": "500"},
                    {"id": "16", "type": 4, "name": "Empty", "position": 5}
                ],
                "threads": [{"id": "17", "type": 11, "name": "a thread", "parent_id": "15"}]
            }, {"id": "20", "unavailable": true}],
            "merged_members": [[{"user_id": "1", "roles": []}], []],
            "private_channels": [
                {"id": "30", "type": 1, "recipient_ids": ["2"], "last_message_id": "100"},
                {"id": "31", "type": 3, "name": "", "recipient_ids": ["2", "3"], "last_message_id": "200"}
            ],
            "read_state": {"version": 1, "partial": false, "entries": [
                {"id": "15", "last_message_id": "400", "mention_count": 2},
                {"id": "99", "read_state_type": 1, "last_acked_id": "5"}
            ]}
        });
        let s = parse_ready(&d);
        assert_eq!(s.me.id, "1");
        assert_eq!(s.guilds.len(), 1);
        let g = &s.guilds[0];
        assert_eq!(g.name, "Guild");
        let names: Vec<&str> = g.channels.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, vec!["general", "a thread", "Info", "rules"]);
        let rules = g.channels.iter().find(|c| c.name == "rules").unwrap();
        assert!(!rules.can_send);
        assert!(
            g.channels
                .iter()
                .find(|c| c.name == "general")
                .unwrap()
                .can_send
        );
        assert_eq!(s.dms[0].id, "31");
        assert_eq!(s.dms[0].name, "Friend");
        assert_eq!(s.dms[1].name, "Friend");
        assert_eq!(s.read_states.len(), 1);
        assert_eq!(s.read_states["15"].mention_count, 2);
        assert_eq!(s.channel_title("12"), ("Guild".into(), "rules".into()));
    }

    #[test]
    fn missing_member_info_does_not_hide_channels() {
        let g = json!({
            "id": "10", "name": "Legacy", "roles": [{"id": "10", "permissions": "0"}],
            "channels": [{"id": "12", "type": 0, "name": "chat", "position": 0}]
        });
        let parsed = parse_guild(&g, "1", None).unwrap();
        assert_eq!(parsed.channels.len(), 1);
    }

    #[test]
    fn message_parsing_and_media() {
        let v = json!({
            "id": "175928847299117063", "channel_id": "5", "content": "hi",
            "author": {"id": "2", "username": "a"},
            "timestamp": "2016-04-30T11:18:25.796000+00:00",
            "attachments": [
                {"id": "1", "filename": "cat.PNG", "url": "https://cdn.discordapp.com/a/cat.PNG?ex=1", "size": 10},
                {"id": "2", "filename": "clip", "url": "https://cdn.discordapp.com/a/clip", "content_type": "video/mp4", "size": 20}
            ],
            "embeds": [
                {"type": "gifv", "url": "https://tenor.com/x", "video": {"url": "https://media.tenor.com/x.mp4"}},
                {"type": "rich", "title": "t"}
            ],
            "referenced_message": {"id": "9", "content": "x".repeat(500), "author": {"id": "3", "username": "b"},
                "referenced_message": {"id": "8"}},
            "unknown_field": {"deep": true}
        });
        let m = Message::from_value(&v).unwrap();
        let r = m.referenced_message.as_ref().unwrap();
        assert_eq!(r.content.len(), 200);
        assert!(r.referenced_message.is_none());
        let media = media_of(&m, true);
        assert_eq!(media.len(), 3);
        assert_eq!(media[0].kind, "image");
        assert_eq!(media[1].kind, "video");
        assert_eq!(media[2].filename, "x.mp4");
        assert_eq!(media[2].kind, "video");
        assert_eq!(media_of(&m, false).len(), 2);
        assert!(Message::from_value(&json!({"content": "no id"})).is_none());
    }
}

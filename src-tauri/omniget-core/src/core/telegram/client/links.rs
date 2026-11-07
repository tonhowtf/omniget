use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

use grammers_client::grammers_tl_types as tl;
use serde::Serialize;
use tokio::sync::{mpsc, Semaphore};
use tokio_util::sync::CancellationToken;

use super::api;
use super::auth::TelegramSessionHandle;
use super::events::{self, DownloadReporter};

const MAX_RANGE: i32 = 5000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkPeer {
    Username(String),
    Channel(i64),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedLink {
    pub url: String,
    pub peer: LinkPeer,
    pub ids: Vec<i32>,
    pub single: bool,
}

const RESERVED: &[&str] = &[
    "joinchat",
    "addstickers",
    "addemoji",
    "login",
    "share",
    "proxy",
    "socks",
    "iv",
    "setlanguage",
    "addtheme",
    "bg",
    "invoice",
    "confirmphone",
    "boost",
    "giftcode",
    "m",
];

fn parse_ids(seg: &str) -> Option<Vec<i32>> {
    if let Some((a, b)) = seg.split_once('-') {
        let a: i32 = a.trim().parse().ok()?;
        let b: i32 = b.trim().parse().ok()?;
        let (lo, hi) = if a <= b { (a, b) } else { (b, a) };
        if lo <= 0 {
            return None;
        }
        let hi = hi.min(lo.saturating_add(MAX_RANGE - 1));
        return Some((lo..=hi).collect());
    }
    let id: i32 = seg.trim().parse().ok()?;
    (id > 0).then(|| vec![id])
}

pub fn parse_links(text: &str) -> Vec<ParsedLink> {
    let re = regex::Regex::new(
        r"(?i)(?:https?://)?(?:www\.)?(?:t\.me|telegram\.me|telegram\.dog)/([A-Za-z0-9_/\-]+)(\?[^\s<>]*)?",
    )
    .expect("valid regex");
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    for cap in re.captures_iter(text) {
        let url = cap
            .get(0)
            .map(|m| m.as_str().to_string())
            .unwrap_or_default();
        let path = cap.get(1).map(|m| m.as_str()).unwrap_or("");
        let query = cap.get(2).map(|m| m.as_str()).unwrap_or("");
        let single = query.trim_start_matches('?').split('&').any(|kv| {
            kv.eq_ignore_ascii_case("single") || kv.to_ascii_lowercase().starts_with("single=")
        });
        let mut segs: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
        if segs
            .first()
            .map(|s| s.eq_ignore_ascii_case("s"))
            .unwrap_or(false)
        {
            segs.remove(0);
        }
        let parsed = if segs
            .first()
            .map(|s| s.eq_ignore_ascii_case("c"))
            .unwrap_or(false)
        {
            if segs.len() < 3 {
                continue;
            }
            let Ok(chan) = segs[1].parse::<i64>() else {
                continue;
            };
            let Some(ids) = parse_ids(segs[segs.len() - 1]) else {
                continue;
            };
            ParsedLink {
                url,
                peer: LinkPeer::Channel(chan),
                ids,
                single,
            }
        } else {
            if segs.len() < 2 {
                continue;
            }
            let user = segs[0];
            if user.starts_with('+') || RESERVED.iter().any(|r| r.eq_ignore_ascii_case(user)) {
                continue;
            }
            let Some(ids) = parse_ids(segs[segs.len() - 1]) else {
                continue;
            };
            ParsedLink {
                url,
                peer: LinkPeer::Username(user.to_string()),
                ids,
                single,
            }
        };
        let key = format!("{:?}{:?}{}", parsed.peer, parsed.ids, parsed.single);
        if seen.insert(key) {
            out.push(parsed);
        }
    }
    out
}

#[derive(Debug, Clone, Serialize)]
pub struct LinkItem {
    pub url: String,
    pub chat_id: i64,
    pub chat_type: String,
    pub chat_title: String,
    pub message_id: i32,
    pub file_name: String,
    pub file_size: u64,
    pub media_type: String,
    pub date: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub caption: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub grouped_id: Option<i64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct LinkError {
    pub url: String,
    pub error: String,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct Resolved {
    pub items: Vec<LinkItem>,
    pub errors: Vec<LinkError>,
    pub skipped: u32,
}

struct PeerInfo {
    chat_id: i64,
    chat_type: String,
    title: String,
    access_hash: i64,
}

async fn client_of(handle: &TelegramSessionHandle) -> anyhow::Result<grammers_client::Client> {
    let g = handle.lock().await;
    g.client
        .as_ref()
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("Not authenticated"))
}

fn channel_info(ch: &tl::types::Channel) -> PeerInfo {
    PeerInfo {
        chat_id: ch.id,
        chat_type: if ch.broadcast {
            "channel".into()
        } else {
            "group".into()
        },
        title: ch.title.clone(),
        access_hash: ch.access_hash.unwrap_or(0),
    }
}

async fn resolve_peer(handle: &TelegramSessionHandle, peer: &LinkPeer) -> anyhow::Result<PeerInfo> {
    let client = client_of(handle).await?;
    let info = match peer {
        LinkPeer::Username(name) => {
            let req = tl::functions::contacts::ResolveUsername {
                username: name.clone(),
                referer: None,
            };
            let tl::enums::contacts::ResolvedPeer::Peer(res) =
                api::invoke_with_flood_wait(&client, &req)
                    .await
                    .map_err(|e| anyhow::anyhow!("Cannot resolve @{}: {}", name, e))?;
            match res.peer {
                tl::enums::Peer::Channel(p) => res
                    .chats
                    .iter()
                    .find_map(|c| match c {
                        tl::enums::Chat::Channel(ch) if ch.id == p.channel_id => {
                            Some(channel_info(ch))
                        }
                        _ => None,
                    })
                    .ok_or_else(|| anyhow::anyhow!("@{} did not resolve to a channel", name))?,
                tl::enums::Peer::User(p) => res
                    .users
                    .iter()
                    .find_map(|u| match u {
                        tl::enums::User::User(u) if u.id == p.user_id => Some(PeerInfo {
                            chat_id: u.id,
                            chat_type: "private".into(),
                            title: [u.first_name.clone(), u.last_name.clone()]
                                .into_iter()
                                .flatten()
                                .collect::<Vec<_>>()
                                .join(" "),
                            access_hash: u.access_hash.unwrap_or(0),
                        }),
                        _ => None,
                    })
                    .ok_or_else(|| anyhow::anyhow!("@{} did not resolve to a user", name))?,
                tl::enums::Peer::Chat(p) => PeerInfo {
                    chat_id: p.chat_id,
                    chat_type: "group".into(),
                    title: name.clone(),
                    access_hash: 0,
                },
            }
        }
        LinkPeer::Channel(id) => {
            let known = handle.lock().await.peer_hashes.get(id).copied();
            let hash = match known {
                Some(h) => Some(h),
                None => api::refresh_channel_hash(handle, *id).await.ok().flatten(),
            };
            let hash = hash.ok_or_else(|| {
                anyhow::anyhow!("Private channel {} is not in your chats: join it first", id)
            })?;
            let req = tl::functions::channels::GetChannels {
                id: vec![tl::enums::InputChannel::Channel(tl::types::InputChannel {
                    channel_id: *id,
                    access_hash: hash,
                })],
            };
            let chats = match api::invoke_with_flood_wait(&client, &req).await {
                Ok(tl::enums::messages::Chats::Chats(c)) => c.chats,
                Ok(tl::enums::messages::Chats::Slice(c)) => c.chats,
                Err(_) => Vec::new(),
            };
            chats
                .iter()
                .find_map(|c| match c {
                    tl::enums::Chat::Channel(ch) if ch.id == *id => Some(channel_info(ch)),
                    _ => None,
                })
                .map(|mut p| {
                    if p.access_hash == 0 {
                        p.access_hash = hash;
                    }
                    p
                })
                .unwrap_or(PeerInfo {
                    chat_id: *id,
                    chat_type: "channel".into(),
                    title: format!("Channel {}", id),
                    access_hash: hash,
                })
        }
    };
    if info.access_hash != 0 {
        handle
            .lock()
            .await
            .peer_hashes
            .insert(info.chat_id, info.access_hash);
    }
    Ok(info)
}

fn messages_of(res: tl::enums::messages::Messages) -> Vec<tl::types::Message> {
    let list = match res {
        tl::enums::messages::Messages::Messages(m) => m.messages,
        tl::enums::messages::Messages::Slice(m) => m.messages,
        tl::enums::messages::Messages::ChannelMessages(m) => m.messages,
        tl::enums::messages::Messages::NotModified(_) => vec![],
    };
    list.into_iter()
        .filter_map(|m| match m {
            tl::enums::Message::Message(m) => Some(m),
            _ => None,
        })
        .collect()
}

pub(super) async fn fetch_messages(
    client: &grammers_client::Client,
    peer: &PeerInfoRef,
    ids: &[i32],
) -> anyhow::Result<Vec<tl::types::Message>> {
    let mut out = Vec::new();
    for chunk in ids.chunks(100) {
        let id: Vec<tl::enums::InputMessage> = chunk
            .iter()
            .map(|i| tl::enums::InputMessage::Id(tl::types::InputMessageId { id: *i }))
            .collect();
        let res = if peer.is_channel {
            let req = tl::functions::channels::GetMessages {
                channel: tl::enums::InputChannel::Channel(tl::types::InputChannel {
                    channel_id: peer.chat_id,
                    access_hash: peer.access_hash,
                }),
                id,
            };
            api::invoke_with_flood_wait(client, &req).await
        } else {
            api::invoke_with_flood_wait(client, &tl::functions::messages::GetMessages { id }).await
        };
        out.extend(messages_of(
            res.map_err(|e| anyhow::anyhow!("getMessages: {}", e))?,
        ));
    }
    Ok(out)
}

pub(super) struct PeerInfoRef {
    pub chat_id: i64,
    pub access_hash: i64,
    pub is_channel: bool,
}

pub async fn resolve_links(
    handle: &TelegramSessionHandle,
    text: &str,
    fix_ext: bool,
) -> anyhow::Result<Resolved> {
    let links = parse_links(text);
    if links.is_empty() {
        anyhow::bail!("No Telegram message links found (expected t.me/<channel>/<id>)");
    }
    let client = client_of(handle).await?;
    let mut out = Resolved::default();
    let mut seen: HashSet<(i64, i32)> = HashSet::new();
    let mut peers: HashMap<String, Arc<PeerInfo>> = HashMap::new();

    for link in links {
        let key = format!("{:?}", link.peer);
        let peer = match peers.get(&key) {
            Some(p) => p.clone(),
            None => match resolve_peer(handle, &link.peer).await {
                Ok(p) => {
                    let p = Arc::new(p);
                    peers.insert(key, p.clone());
                    p
                }
                Err(e) => {
                    out.errors.push(LinkError {
                        url: link.url.clone(),
                        error: e.to_string(),
                    });
                    continue;
                }
            },
        };
        let pref = PeerInfoRef {
            chat_id: peer.chat_id,
            access_hash: peer.access_hash,
            is_channel: peer.chat_type == "channel"
                || (peer.chat_type == "group" && peer.access_hash != 0),
        };
        let mut msgs = match fetch_messages(&client, &pref, &link.ids).await {
            Ok(m) => m,
            Err(e) => {
                out.errors.push(LinkError {
                    url: link.url.clone(),
                    error: e.to_string(),
                });
                continue;
            }
        };
        if !link.single && link.ids.len() == 1 {
            if let Some(gid) = msgs.first().and_then(|m| m.grouped_id) {
                let base = link.ids[0];
                let around: Vec<i32> = ((base - 10).max(1)..=base + 10)
                    .filter(|i| *i != base)
                    .collect();
                if let Ok(extra) = fetch_messages(&client, &pref, &around).await {
                    msgs.extend(extra.into_iter().filter(|m| m.grouped_id == Some(gid)));
                }
            }
        }
        msgs.sort_by_key(|m| m.id);
        let found = msgs.len();
        if found == 0 {
            out.errors.push(LinkError {
                url: link.url.clone(),
                error: "Message not found or deleted".into(),
            });
            continue;
        }
        for m in msgs {
            if !seen.insert((peer.chat_id, m.id)) {
                continue;
            }
            let Some(media) = m.media.as_ref() else {
                out.skipped += 1;
                continue;
            };
            let Some((file_name, file_size, media_type)) =
                api::extract_raw_media_info(media, fix_ext)
            else {
                out.skipped += 1;
                continue;
            };
            out.items.push(LinkItem {
                url: link.url.clone(),
                chat_id: peer.chat_id,
                chat_type: peer.chat_type.clone(),
                chat_title: peer.title.clone(),
                message_id: m.id,
                file_name,
                file_size,
                media_type,
                date: m.date as i64,
                caption: Some(m.message.clone()).filter(|c| !c.is_empty()),
                grouped_id: m.grouped_id,
            });
        }
    }
    Ok(out)
}

fn safe_name(s: &str) -> String {
    let cleaned = sanitize_filename::sanitize(s.trim());
    if cleaned.is_empty() {
        "telegram".to_string()
    } else {
        cleaned
    }
}

pub fn item_path(output_dir: &Path, item: &LinkItem) -> PathBuf {
    output_dir.join(safe_name(&item.chat_title)).join(format!(
        "{}_{}",
        item.message_id,
        safe_name(&item.file_name)
    ))
}

pub async fn download_items(
    handle: TelegramSessionHandle,
    active: super::ActiveDownloads,
    items: Vec<LinkItem>,
    output_dir: PathBuf,
    concurrent: usize,
    title: String,
) -> u64 {
    let batch_id = events::next_id();
    let cancel = CancellationToken::new();
    active
        .lock()
        .await
        .insert(batch_id, (format!("tg-batch:{}", batch_id), cancel.clone()));
    let total_files = items.len().max(1) as u32;
    let total_bytes: u64 = items.iter().map(|i| i.file_size).sum();

    tokio::spawn(async move {
        let mut agg = DownloadReporter::new(
            batch_id,
            &title,
            "batch",
            None,
            Some(total_bytes).filter(|b| *b > 0),
            Some(output_dir.to_string_lossy().into()),
        );
        agg.progress(0.0);
        let agg = Arc::new(tokio::sync::Mutex::new(agg));
        let sem = Arc::new(Semaphore::new(concurrent.max(1)));
        let done = Arc::new(AtomicU32::new(0));
        let ok = Arc::new(AtomicU32::new(0));
        let failed = Arc::new(AtomicU32::new(0));
        let deleted = Arc::new(AtomicU32::new(0));
        let mut handles = Vec::new();

        for item in items {
            let handle = handle.clone();
            let sem = sem.clone();
            let cancel = cancel.clone();
            let agg = agg.clone();
            let (done, ok, failed, deleted) =
                (done.clone(), ok.clone(), failed.clone(), deleted.clone());
            let output_dir = output_dir.clone();
            handles.push(tokio::spawn(async move {
                let path = item_path(&output_dir, &item);
                let finish = |agg: Arc<tokio::sync::Mutex<DownloadReporter>>| async move {
                    let n = done.fetch_add(1, Ordering::Relaxed) + 1;
                    agg.lock()
                        .await
                        .progress(n as f64 / total_files as f64 * 100.0);
                };
                if let Ok(meta) = std::fs::metadata(&path) {
                    if meta.len() > 0 && (item.file_size == 0 || meta.len() == item.file_size) {
                        ok.fetch_add(1, Ordering::Relaxed);
                        events::batch_file_status(
                            batch_id,
                            item.message_id,
                            "skipped",
                            100.0,
                            None,
                        );
                        finish(agg).await;
                        return;
                    }
                }
                let _permit = tokio::select! {
                    p = sem.acquire_owned() => match p { Ok(p) => p, Err(_) => return },
                    _ = cancel.cancelled() => return,
                };
                if cancel.is_cancelled() {
                    return;
                }
                let claim = super::template::claim_unique_path(&path);
                let path = claim.path().to_path_buf();
                let item_id = events::next_id();
                let mut rep = DownloadReporter::new(
                    item_id,
                    &item.file_name,
                    "item",
                    Some(batch_id),
                    Some(item.file_size).filter(|s| *s > 0),
                    Some(path.to_string_lossy().into()),
                );
                events::batch_file_status(batch_id, item.message_id, "downloading", 0.0, None);
                let (tx, mut rx) = mpsc::channel::<f64>(32);
                let mid = item.message_id;
                let pf = tokio::spawn(async move {
                    while let Some(p) = rx.recv().await {
                        rep.progress(p);
                        events::batch_file_status(batch_id, mid, "downloading", p, None);
                    }
                    rep
                });
                let result = api::download_media_with_retry(
                    &handle,
                    item.chat_id,
                    &item.chat_type,
                    item.message_id,
                    &path,
                    tx,
                    &cancel,
                )
                .await;
                let rep = pf.await.ok();
                match result {
                    Ok(size) => {
                        ok.fetch_add(1, Ordering::Relaxed);
                        super::bandwidth::record_bytes(
                            &crate::core::telegram::host::data_dir(),
                            size,
                        );
                        events::batch_file_status(batch_id, item.message_id, "done", 100.0, None);
                        if let Some(r) = rep {
                            r.complete(
                                true,
                                None,
                                Some(path.to_string_lossy().into()),
                                Some(size),
                                Some(1),
                                None,
                            );
                        }
                    }
                    Err(e) => {
                        let msg = e.to_string();
                        let upper = msg.to_uppercase();
                        let status = if msg.contains("cancelled") {
                            "cancelled"
                        } else if upper.contains("MESSAGE_ID_INVALID")
                            || upper.contains("MESSAGE_DELETED")
                            || msg.contains("not found")
                        {
                            deleted.fetch_add(1, Ordering::Relaxed);
                            "skipped_deleted"
                        } else {
                            failed.fetch_add(1, Ordering::Relaxed);
                            "error"
                        };
                        events::batch_file_status(
                            batch_id,
                            item.message_id,
                            status,
                            0.0,
                            Some(msg.clone()),
                        );
                        if let Some(r) = rep {
                            r.complete(false, Some(msg), None, None, None, None);
                        }
                    }
                }
                finish(agg).await;
            }));
        }
        for h in handles {
            let _ = h.await;
        }
        active.lock().await.remove(&batch_id);
        let (okn, failn, deln) = (
            ok.load(Ordering::Relaxed),
            failed.load(Ordering::Relaxed),
            deleted.load(Ordering::Relaxed),
        );
        let mut errs = Vec::new();
        if cancel.is_cancelled() {
            errs.push("Batch cancelled".to_string());
        }
        if failn > 0 {
            errs.push(format!("{} files failed", failn));
        }
        if deln > 0 {
            errs.push(format!("{} messages deleted/invalid (skipped)", deln));
        }
        agg.lock().await.complete(
            failn == 0 && !cancel.is_cancelled(),
            (!errs.is_empty()).then(|| errs.join("; ")),
            Some(output_dir.to_string_lossy().into()),
            None,
            Some(okn),
            (deln > 0).then_some(deln),
        );
    });
    batch_id
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_public_private_single_and_ranges() {
        let l = parse_links("see https://t.me/durov/123 and t.me/c/1234567/45?single x");
        assert_eq!(l.len(), 2);
        assert_eq!(l[0].peer, LinkPeer::Username("durov".into()));
        assert_eq!(l[0].ids, vec![123]);
        assert!(!l[0].single);
        assert_eq!(l[1].peer, LinkPeer::Channel(1234567));
        assert!(l[1].single);

        let r = parse_links("https://t.me/somechan/10-12");
        assert_eq!(r[0].ids, vec![10, 11, 12]);

        let t = parse_links("https://t.me/c/99/5/77");
        assert_eq!(t[0].ids, vec![77]);

        assert!(
            parse_links("https://t.me/joinchat/abc https://t.me/+xyz https://t.me/durov")
                .is_empty()
        );
    }
}

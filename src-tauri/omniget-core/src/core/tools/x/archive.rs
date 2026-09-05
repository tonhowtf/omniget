//! Arquivo do X (estudo 67): le o zip de "Baixar seus dados" (ou a pasta
//! extraida) no formato `window.YTD.<x>.part0 = [...]` mapeado pelo
//! twitter-archive-reader (MIT), e faz offline o que o
//! twitter-x-unfollow-tool faz: seguindo × seguidores sem tocar na rede.

use std::collections::HashMap;
use std::io::Read;

use anyhow::anyhow;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{XMedia, XPost, XUser};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ArchiveAccount {
    pub account_id: String,
    pub user_link: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct YearCount {
    pub year: i32,
    pub tweets: u64,
    pub likes_received: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ArchiveSummary {
    pub path: String,
    pub username: String,
    pub display_name: String,
    pub account_id: String,
    pub created_at: String,
    pub tweets: usize,
    pub replies: usize,
    pub reposts: usize,
    pub likes: usize,
    pub followers: usize,
    pub following: usize,
    pub blocked: usize,
    pub muted: usize,
    pub dm_messages: usize,
    pub first_tweet: String,
    pub last_tweet: String,
    pub likes_received: u64,
    pub reposts_received: u64,
    pub by_year: Vec<YearCount>,
    pub by_weekday: Vec<u64>,
    pub by_hour: Vec<u64>,
    pub top_tweets: Vec<XPost>,
    pub not_following_back: Vec<ArchiveAccount>,
    pub fans: Vec<ArchiveAccount>,
    pub files: Vec<String>,
}

struct Source {
    files: HashMap<String, String>,
}

impl Source {
    fn open(path: &str) -> anyhow::Result<Self> {
        let p = std::path::Path::new(path);
        let mut files = HashMap::new();
        if p.is_dir() {
            let data = if p.join("data").is_dir() { p.join("data") } else { p.to_path_buf() };
            for entry in std::fs::read_dir(&data)? {
                let entry = entry?;
                let name = entry.file_name().to_string_lossy().to_string();
                if name.ends_with(".js") && wanted(&name) {
                    if let Ok(s) = std::fs::read_to_string(entry.path()) {
                        files.insert(name, s);
                    }
                }
            }
        } else {
            let f = std::fs::File::open(p)?;
            let mut zip = zip::ZipArchive::new(f)?;
            for i in 0..zip.len() {
                let mut entry = zip.by_index(i)?;
                let full = entry.name().to_string();
                let name = full.rsplit('/').next().unwrap_or(&full).to_string();
                if full.contains("data/") && name.ends_with(".js") && wanted(&name) {
                    let mut s = String::new();
                    if entry.read_to_string(&mut s).is_ok() {
                        files.insert(name, s);
                    }
                }
            }
        }
        if files.is_empty() {
            return Err(anyhow!("nao achei os arquivos data/*.js do arquivo do X"));
        }
        Ok(Self { files })
    }

    /// `tweets.js`, `tweets-part1.js`… concatenados em ordem.
    fn json(&self, base: &str) -> Vec<Value> {
        let mut names: Vec<&String> = self.files.keys().filter(|n| *n == &format!("{}.js", base) || n.starts_with(&format!("{}-part", base))).collect();
        names.sort();
        let mut out = Vec::new();
        for n in names {
            let s = &self.files[n];
            let Some(i) = s.find('=') else { continue };
            if let Ok(Value::Array(a)) = serde_json::from_str::<Value>(s[i + 1..].trim().trim_end_matches(';')) {
                out.extend(a);
            }
        }
        out
    }
}

fn wanted(name: &str) -> bool {
    ["account", "profile", "tweets", "tweet", "like", "follower", "following", "block", "mute", "direct-messages", "direct-message"]
        .iter()
        .any(|b| name == format!("{}.js", b) || name.starts_with(&format!("{}-part", b)))
}

fn s(v: &Value, k: &str) -> String {
    v.get(k).and_then(|x| x.as_str()).unwrap_or("").to_string()
}

fn n(v: &Value, k: &str) -> u64 {
    match v.get(k) {
        Some(Value::String(x)) => x.parse().unwrap_or(0),
        Some(Value::Number(x)) => x.as_u64().unwrap_or(0),
        _ => 0,
    }
}

fn account_list(items: &[Value], key: &str) -> Vec<ArchiveAccount> {
    items
        .iter()
        .filter_map(|v| v.get(key))
        .map(|a| ArchiveAccount { account_id: s(a, "accountId"), user_link: s(a, "userLink") })
        .filter(|a| !a.account_id.is_empty())
        .collect()
}

fn tweet_from(v: &Value, author: &XUser) -> Option<XPost> {
    let t = v.get("tweet").or_else(|| v.get("tweetEdit")).unwrap_or(v);
    let id = s(t, "id_str");
    if id.is_empty() {
        return None;
    }
    let mut text = s(t, "full_text");
    let mut links = Vec::new();
    for u in t.pointer("/entities/urls").and_then(|a| a.as_array()).into_iter().flatten() {
        let short = s(u, "url");
        let long = s(u, "expanded_url");
        if !short.is_empty() && !long.is_empty() {
            text = text.replace(&short, &long);
            links.push(long);
        }
    }
    let mut media = Vec::new();
    for m in t.pointer("/extended_entities/media").and_then(|a| a.as_array()).into_iter().flatten() {
        let short = s(m, "url");
        if !short.is_empty() {
            text = text.replace(&short, "").trim_end().to_string();
        }
        let kind = s(m, "type");
        let thumb = s(m, "media_url_https");
        let mut url = super::photo_orig_url(&thumb);
        if kind != "photo" {
            let mut best = 0u64;
            for var in m.pointer("/video_info/variants").and_then(|a| a.as_array()).into_iter().flatten() {
                if s(var, "content_type") == "video/mp4" && n(var, "bitrate") >= best {
                    best = n(var, "bitrate");
                    url = s(var, "url");
                }
            }
        }
        media.push(XMedia { kind: if kind == "photo" { "photo".into() } else if kind == "video" { "video".into() } else { "gif".into() }, url, thumb, ..Default::default() });
    }
    let (created_at, ts) = super::parse_twitter_date(&s(t, "created_at")).unwrap_or_default();
    let re_tag = regex::Regex::new(r"#([\p{L}\p{N}_]+)").unwrap();
    let re_at = regex::Regex::new(r"@([A-Za-z0-9_]{1,15})").unwrap();
    let is_rt = text.starts_with("RT @");
    Some(XPost {
        url: format!("https://x.com/{}/status/{}", if author.handle.is_empty() { "i" } else { &author.handle }, id),
        id,
        hashtags: re_tag.captures_iter(&text).map(|c| c[1].to_string()).collect(),
        mentions: re_at.captures_iter(&text).map(|c| c[1].to_string()).collect(),
        text,
        created_at,
        timestamp: ts,
        author: author.clone(),
        likes: n(t, "favorite_count"),
        reposts: n(t, "retweet_count"),
        lang: s(t, "lang"),
        media,
        reply_to_id: Some(s(t, "in_reply_to_status_id_str")).filter(|x| !x.is_empty()),
        reply_to_handle: Some(s(t, "in_reply_to_screen_name")).filter(|x| !x.is_empty()),
        reposted_by: if is_rt { Some(author.handle.clone()) } else { None },
        source: regex::Regex::new(r"<[^>]+>").unwrap().replace_all(&s(t, "source"), "").to_string(),
        links,
        ..Default::default()
    })
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LikeItem {
    pub tweet_id: String,
    pub text: String,
    pub url: String,
}

pub struct Loaded {
    pub summary: ArchiveSummary,
    pub posts: Vec<XPost>,
    pub likes: Vec<LikeItem>,
    pub followers: Vec<ArchiveAccount>,
    pub following: Vec<ArchiveAccount>,
    pub blocked: Vec<ArchiveAccount>,
    pub muted: Vec<ArchiveAccount>,
}

pub fn load(path: &str) -> anyhow::Result<Loaded> {
    use chrono::{Datelike, TimeZone, Timelike};
    let src = Source::open(path)?;
    let account = src.json("account").into_iter().next().and_then(|v| v.get("account").cloned()).unwrap_or(Value::Null);
    let profile = src.json("profile").into_iter().next().and_then(|v| v.get("profile").cloned()).unwrap_or(Value::Null);
    let author = XUser {
        id: s(&account, "accountId"),
        handle: s(&account, "username"),
        name: s(&account, "accountDisplayName"),
        bio: profile.pointer("/description/bio").and_then(|b| b.as_str()).unwrap_or("").to_string(),
        avatar: s(&profile, "avatarMediaUrl"),
        ..Default::default()
    };
    let posts: Vec<XPost> = {
        let mut t = src.json("tweets");
        if t.is_empty() {
            t = src.json("tweet");
        }
        t.iter().filter_map(|v| tweet_from(v, &author)).collect()
    };
    let likes: Vec<LikeItem> = src
        .json("like")
        .iter()
        .filter_map(|v| v.get("like"))
        .map(|l| LikeItem { tweet_id: s(l, "tweetId"), text: s(l, "fullText"), url: if s(l, "expandedUrl").is_empty() { format!("https://x.com/i/status/{}", s(l, "tweetId")) } else { s(l, "expandedUrl") } })
        .collect();
    let followers = account_list(&src.json("follower"), "follower");
    let following = account_list(&src.json("following"), "following");
    let blocked = account_list(&src.json("block"), "blocking");
    let muted = account_list(&src.json("mute"), "muting");
    let dm_messages: usize = src
        .json("direct-messages")
        .iter()
        .chain(src.json("direct-message").iter())
        .filter_map(|v| v.get("dmConversation"))
        .map(|c| c.get("messages").and_then(|m| m.as_array()).map(|a| a.len()).unwrap_or(0))
        .sum();
    let follower_ids: std::collections::HashSet<&str> = followers.iter().map(|a| a.account_id.as_str()).collect();
    let following_ids: std::collections::HashSet<&str> = following.iter().map(|a| a.account_id.as_str()).collect();
    let not_following_back: Vec<ArchiveAccount> = following.iter().filter(|a| !follower_ids.contains(a.account_id.as_str())).cloned().collect();
    let fans: Vec<ArchiveAccount> = followers.iter().filter(|a| !following_ids.contains(a.account_id.as_str())).cloned().collect();
    let mut by_year: HashMap<i32, YearCount> = HashMap::new();
    let mut by_weekday = vec![0u64; 7];
    let mut by_hour = vec![0u64; 24];
    let (mut min_ts, mut max_ts) = (i64::MAX, i64::MIN);
    for p in &posts {
        if p.timestamp <= 0 {
            continue;
        }
        min_ts = min_ts.min(p.timestamp);
        max_ts = max_ts.max(p.timestamp);
        if let Some(dt) = chrono::Local.timestamp_opt(p.timestamp, 0).single() {
            let e = by_year.entry(dt.year()).or_insert(YearCount { year: dt.year(), ..Default::default() });
            e.tweets += 1;
            e.likes_received += p.likes;
            by_weekday[dt.weekday().num_days_from_sunday() as usize] += 1;
            by_hour[dt.hour() as usize] += 1;
        }
    }
    let mut by_year: Vec<YearCount> = by_year.into_values().collect();
    by_year.sort_by_key(|y| y.year);
    let mut top: Vec<XPost> = posts.iter().filter(|p| p.reposted_by.is_none()).cloned().collect();
    top.sort_by_key(|b| std::cmp::Reverse(b.likes + b.reposts * 2));
    top.truncate(10);
    let summary = ArchiveSummary {
        path: path.to_string(),
        username: author.handle.clone(),
        display_name: author.name.clone(),
        account_id: author.id.clone(),
        created_at: s(&account, "createdAt"),
        tweets: posts.len(),
        replies: posts.iter().filter(|p| p.is_reply()).count(),
        reposts: posts.iter().filter(|p| p.reposted_by.is_some()).count(),
        likes: likes.len(),
        followers: followers.len(),
        following: following.len(),
        blocked: blocked.len(),
        muted: muted.len(),
        dm_messages,
        first_tweet: if min_ts < i64::MAX { super::iso_from_timestamp(min_ts) } else { String::new() },
        last_tweet: if max_ts > i64::MIN { super::iso_from_timestamp(max_ts) } else { String::new() },
        likes_received: posts.iter().map(|p| p.likes).sum(),
        reposts_received: posts.iter().map(|p| p.reposts).sum(),
        by_year,
        by_weekday,
        by_hour,
        top_tweets: top,
        not_following_back,
        fans,
        files: {
            let mut f: Vec<String> = src.files.keys().cloned().collect();
            f.sort();
            f
        },
    };
    Ok(Loaded { summary, posts, likes, followers, following, blocked, muted })
}

pub fn open(path: &str) -> anyhow::Result<ArchiveSummary> {
    load(path).map(|l| l.summary)
}

/// `what`: tweets | likes | followers | following | not_following_back | fans | blocked | muted.
pub fn export(path: &str, dest_dir: &str, what: &str, format: &str) -> anyhow::Result<String> {
    let l = load(path)?;
    let dir = std::path::Path::new(dest_dir);
    std::fs::create_dir_all(dir)?;
    let file = dir.join(format!("x-archive-{}-{}.{}", l.summary.username, what, super::export::ext_for(format)));
    let accounts = |list: &[ArchiveAccount]| -> anyhow::Result<String> {
        let content = match format {
            "json" => serde_json::to_string_pretty(list)?,
            "csv" => {
                let mut o = String::from("account_id,url\n");
                for a in list {
                    o.push_str(&format!("{},{}\n", a.account_id, if a.user_link.is_empty() { format!("https://x.com/i/user/{}", a.account_id) } else { a.user_link.clone() }));
                }
                o
            }
            _ => list.iter().map(|a| format!("- https://x.com/i/user/{}", a.account_id)).collect::<Vec<_>>().join("\n") + "\n",
        };
        std::fs::write(&file, content)?;
        Ok(file.to_string_lossy().to_string())
    };
    match what {
        "tweets" => super::export::write_posts(&l.posts, format, &file, &format!("Posts de @{}", l.summary.username)),
        "likes" => {
            let content = match format {
                "json" => serde_json::to_string_pretty(&l.likes)?,
                "csv" => {
                    let mut o = String::from("tweet_id,url,text\n");
                    for k in &l.likes {
                        o.push_str(&format!("{},{},{}\n", k.tweet_id, k.url, super::export::csv_escape(&k.text)));
                    }
                    o
                }
                _ => l.likes.iter().map(|k| format!("- {}\n  {}", k.text.replace('\n', " "), k.url)).collect::<Vec<_>>().join("\n") + "\n",
            };
            std::fs::write(&file, content)?;
            Ok(file.to_string_lossy().to_string())
        }
        "followers" => accounts(&l.followers),
        "following" => accounts(&l.following),
        "not_following_back" => accounts(&l.summary.not_following_back),
        "fans" => accounts(&l.summary.fans),
        "blocked" => accounts(&l.blocked),
        "muted" => accounts(&l.muted),
        other => Err(anyhow!("nao sei exportar {}", other)),
    }
}

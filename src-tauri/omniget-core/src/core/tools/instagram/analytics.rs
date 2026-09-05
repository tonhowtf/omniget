//! Métricas de perfil (engajamento, ritmo, horários, hashtags), comparação
//! entre perfis, seguidores fantasmas e leitura do export oficial ("Baixe
//! suas informações", formato JSON).

use std::collections::{HashMap, HashSet};
use std::io::Read;
use std::path::Path;
use std::sync::atomic::AtomicBool;

use anyhow::anyhow;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::follow::MiniUser;
use super::media::MediaItem;
use super::profile::UserInfo;
use super::IgClient;

#[derive(Debug, Clone, Serialize, Default)]
pub struct PostBrief {
    pub code: String,
    pub url: String,
    pub thumbnail: String,
    pub taken_at: i64,
    pub likes: u64,
    pub comments: u64,
    pub plays: u64,
    pub kind: String,
    pub caption: String,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct ProfileStats {
    pub user: UserInfo,
    pub posts_analyzed: usize,
    pub span_days: f64,
    pub posts_per_week: f64,
    pub avg_likes: f64,
    pub avg_comments: f64,
    pub avg_plays: f64,
    pub median_likes: u64,
    /// (curtidas + comentários) médios ÷ seguidores, em %.
    pub engagement_rate: f64,
    pub comment_ratio: f64,
    pub follow_ratio: f64,
    pub share_photo: f64,
    pub share_video: f64,
    pub share_carousel: f64,
    pub avg_caption_len: f64,
    pub avg_hashtags: f64,
    pub top_hashtags: Vec<(String, u32)>,
    pub top_mentions: Vec<(String, u32)>,
    /// 0 = domingo … 6 = sábado (hora local), contagem de posts.
    pub weekday_counts: [u32; 7],
    pub hour_counts: [u32; 24],
    /// Engajamento médio por dia da semana (curtidas + comentários).
    pub weekday_engagement: [f64; 7],
    pub best_weekday: u8,
    pub best_hour: u8,
    pub top_posts: Vec<PostBrief>,
    pub paid_partnerships: u32,
    pub first_post_at: i64,
    pub last_post_at: i64,
}

fn top_counts(items: impl Iterator<Item = String>, n: usize) -> Vec<(String, u32)> {
    let mut m: HashMap<String, u32> = HashMap::new();
    for i in items {
        *m.entry(i).or_default() += 1;
    }
    let mut v: Vec<(String, u32)> = m.into_iter().collect();
    v.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    v.truncate(n);
    v
}

pub fn compute(user: UserInfo, posts: &[MediaItem]) -> ProfileStats {
    let n = posts.len();
    let mut st = ProfileStats {
        user,
        posts_analyzed: n,
        ..Default::default()
    };
    if n == 0 {
        return st;
    }
    let f = |x: u64| x as f64;
    let total_likes: u64 = posts.iter().map(|p| p.like_count).sum();
    let total_comments: u64 = posts.iter().map(|p| p.comment_count).sum();
    let videos: Vec<&MediaItem> = posts.iter().filter(|p| p.play_count > 0).collect();
    st.avg_likes = f(total_likes) / n as f64;
    st.avg_comments = f(total_comments) / n as f64;
    st.avg_plays = if videos.is_empty() {
        0.0
    } else {
        videos.iter().map(|p| f(p.play_count)).sum::<f64>() / videos.len() as f64
    };
    let mut likes: Vec<u64> = posts.iter().map(|p| p.like_count).collect();
    likes.sort_unstable();
    st.median_likes = likes[n / 2];
    if st.user.follower_count > 0 {
        st.engagement_rate = (st.avg_likes + st.avg_comments) / f(st.user.follower_count) * 100.0;
    }
    if st.avg_likes > 0.0 {
        st.comment_ratio = st.avg_comments / st.avg_likes * 100.0;
    }
    if st.user.following_count > 0 {
        st.follow_ratio = f(st.user.follower_count) / f(st.user.following_count);
    }
    let count = |pred: &dyn Fn(&MediaItem) -> bool| {
        posts.iter().filter(|p| pred(p)).count() as f64 / n as f64 * 100.0
    };
    st.share_carousel = count(&|p| p.media_type == 8);
    st.share_video = count(&|p| p.media_type == 2);
    st.share_photo = count(&|p| p.media_type == 1);
    st.avg_caption_len = posts
        .iter()
        .map(|p| p.caption.chars().count() as f64)
        .sum::<f64>()
        / n as f64;
    st.avg_hashtags = posts.iter().map(|p| p.hashtags.len() as f64).sum::<f64>() / n as f64;
    st.top_hashtags = top_counts(posts.iter().flat_map(|p| p.hashtags.iter().cloned()), 15);
    st.top_mentions = top_counts(posts.iter().flat_map(|p| p.mentions.iter().cloned()), 10);
    st.paid_partnerships = posts.iter().filter(|p| p.is_paid_partnership).count() as u32;
    let mut eng_sum = [0f64; 7];
    for p in posts {
        if let Some(dt) = chrono::DateTime::from_timestamp(p.taken_at, 0) {
            use chrono::{Datelike, Timelike};
            let local = dt.with_timezone(&chrono::Local);
            let wd = local.weekday().num_days_from_sunday() as usize;
            st.weekday_counts[wd] += 1;
            st.hour_counts[local.hour() as usize] += 1;
            eng_sum[wd] += f(p.like_count + p.comment_count);
        }
    }
    #[allow(clippy::needless_range_loop)]
    for i in 0..7 {
        st.weekday_engagement[i] = if st.weekday_counts[i] > 0 {
            eng_sum[i] / st.weekday_counts[i] as f64
        } else {
            0.0
        };
    }
    st.best_weekday = (0..7)
        .max_by(|a, b| {
            st.weekday_engagement[*a]
                .partial_cmp(&st.weekday_engagement[*b])
                .unwrap()
        })
        .unwrap_or(0) as u8;
    st.best_hour = (0..24).max_by_key(|h| st.hour_counts[*h]).unwrap_or(0) as u8;
    let (min_t, max_t) = posts.iter().fold((i64::MAX, 0i64), |(lo, hi), p| {
        (lo.min(p.taken_at), hi.max(p.taken_at))
    });
    st.first_post_at = min_t;
    st.last_post_at = max_t;
    st.span_days = ((max_t - min_t).max(0) as f64) / 86400.0;
    st.posts_per_week = if st.span_days >= 1.0 {
        n as f64 / (st.span_days / 7.0)
    } else {
        n as f64
    };
    let mut ranked: Vec<&MediaItem> = posts.iter().collect();
    ranked.sort_by_key(|p| std::cmp::Reverse(p.like_count + p.comment_count));
    st.top_posts = ranked
        .into_iter()
        .take(6)
        .map(|p| PostBrief {
            code: p.code.clone(),
            url: p.url.clone(),
            thumbnail: p.thumbnail.clone(),
            taken_at: p.taken_at,
            likes: p.like_count,
            comments: p.comment_count,
            plays: p.play_count,
            kind: p.product_type.clone(),
            caption: p.caption.chars().take(120).collect(),
        })
        .collect();
    st
}

// ── Seguidores fantasmas ─────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
pub struct GhostReport {
    pub posts_checked: usize,
    pub followers_total: usize,
    pub engaged: usize,
    pub ghosts: Vec<MiniUser>,
    /// Seguidores que curtiram/comentaram e quantas vezes.
    pub top_fans: Vec<(MiniUser, u32)>,
}

/// Seguidores que não curtiram nem comentaram nenhum dos últimos N posts.
pub async fn ghosts(
    client: &IgClient,
    followers: Vec<MiniUser>,
    posts: &[MediaItem],
    comment_pages: usize,
    flag: &AtomicBool,
    progress: &super::super::ProgressFn,
    job: &str,
) -> Result<GhostReport, super::IgError> {
    let id = format!("ig:{}", job);
    let mut engagement: HashMap<String, u32> = HashMap::new();
    let total = posts.len() as u64;
    for (i, post) in posts.iter().enumerate() {
        if super::cancelled(flag) {
            break;
        }
        super::super::report(
            progress,
            &id,
            "likers",
            i as u64,
            Some(total),
            Some(post.code.clone()),
        );
        if let Ok((_, users)) = super::social::likers(client, &post.pk).await {
            for u in users {
                *engagement.entry(u.pk).or_default() += 1;
            }
        }
        client.pause().await;
        if comment_pages > 0 {
            let limit = comment_pages * 20;
            if let Ok(cs) =
                super::social::comments(client, &post.pk, limit, flag, progress, job).await
            {
                for c in cs {
                    *engagement.entry(c.user.pk).or_default() += 1;
                }
            }
            client.pause().await;
        }
    }
    let engaged_set: HashSet<&String> = engagement.keys().collect();
    let ghosts: Vec<MiniUser> = followers
        .iter()
        .filter(|f| !engaged_set.contains(&f.pk))
        .cloned()
        .collect();
    let mut top_fans: Vec<(MiniUser, u32)> = followers
        .iter()
        .filter_map(|f| engagement.get(&f.pk).map(|n| (f.clone(), *n)))
        .collect();
    top_fans.sort_by_key(|b| std::cmp::Reverse(b.1));
    top_fans.truncate(30);
    super::super::report(progress, &id, "done", total, Some(total), None);
    Ok(GhostReport {
        posts_checked: posts.len(),
        followers_total: followers.len(),
        engaged: followers.len() - ghosts.len(),
        ghosts,
        top_fans,
    })
}

// ── Export oficial ───────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct ExportUser {
    pub username: String,
    pub href: String,
    pub timestamp: i64,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct ExportReport {
    pub source: String,
    pub files_found: Vec<String>,
    pub followers: Vec<ExportUser>,
    pub following: Vec<ExportUser>,
    pub not_following_back: Vec<ExportUser>,
    pub fans: Vec<ExportUser>,
    pub mutuals: usize,
    pub pending_sent: Vec<ExportUser>,
    pub close_friends: Vec<ExportUser>,
    pub blocked: Vec<ExportUser>,
    pub recently_unfollowed: Vec<ExportUser>,
    pub received_requests: Vec<ExportUser>,
    pub restricted: Vec<ExportUser>,
    pub hide_story_from: Vec<ExportUser>,
    pub removed_suggestions: Vec<ExportUser>,
    /// Seguidores por mês (AAAA-MM → n), a partir do timestamp do export.
    pub followers_by_month: Vec<(String, u32)>,
}

/// Todo item do export tem `string_list_data: [{href, value, timestamp}]`;
/// alguns arquivos são um array na raiz, outros `{"relationships_x": [...]}`.
fn collect_users(v: &Value, out: &mut Vec<ExportUser>) {
    match v {
        Value::Array(a) => a.iter().for_each(|x| collect_users(x, out)),
        Value::Object(o) => {
            if let Some(list) = o.get("string_list_data").and_then(|l| l.as_array()) {
                let title = o.get("title").and_then(|t| t.as_str()).unwrap_or("");
                for e in list {
                    let value = e.get("value").and_then(|x| x.as_str()).unwrap_or("");
                    let href = e.get("href").and_then(|x| x.as_str()).unwrap_or("");
                    let username = if !value.is_empty() {
                        value
                    } else if !title.is_empty() {
                        title
                    } else {
                        href.trim_end_matches('/').rsplit('/').next().unwrap_or("")
                    };
                    if username.is_empty() {
                        continue;
                    }
                    out.push(ExportUser {
                        username: username.to_lowercase(),
                        href: href.to_string(),
                        timestamp: e.get("timestamp").and_then(|x| x.as_i64()).unwrap_or(0),
                    });
                }
            } else {
                o.values().for_each(|x| collect_users(x, out));
            }
        }
        _ => {}
    }
}

fn read_export_files(path: &Path) -> anyhow::Result<Vec<(String, Vec<u8>)>> {
    let mut files = Vec::new();
    let wanted = |name: &str| {
        let n = name.to_lowercase();
        n.ends_with(".json")
            && (n.contains("followers_and_following")
                || n.contains("connections/")
                || n.starts_with("followers")
                || n.contains("following")
                || n.contains("close_friends")
                || n.contains("pending_follow")
                || n.contains("blocked")
                || n.contains("unfollowed")
                || n.contains("follow_requests")
                || n.contains("restricted")
                || n.contains("hide_story")
                || n.contains("removed_suggestions"))
    };
    if path.is_dir() {
        fn walk(dir: &Path, out: &mut Vec<(String, Vec<u8>)>, wanted: &dyn Fn(&str) -> bool) {
            if let Ok(rd) = std::fs::read_dir(dir) {
                for e in rd.flatten() {
                    let p = e.path();
                    if p.is_dir() {
                        walk(&p, out, wanted);
                    } else if let Some(name) = p.to_str() {
                        if wanted(&name.replace('\\', "/")) {
                            if let Ok(bytes) = std::fs::read(&p) {
                                out.push((name.to_string(), bytes));
                            }
                        }
                    }
                }
            }
        }
        walk(path, &mut files, &wanted);
    } else {
        let file = std::fs::File::open(path)?;
        let mut zip = zip::ZipArchive::new(file)?;
        for i in 0..zip.len() {
            let mut entry = zip.by_index(i)?;
            let name = entry.name().to_string();
            if wanted(&name) {
                let mut bytes = Vec::new();
                entry.read_to_end(&mut bytes)?;
                files.push((name, bytes));
            }
        }
    }
    if files.is_empty() {
        return Err(anyhow!("nao achei os arquivos JSON de seguidores no export. No Instagram, peça o export em formato JSON (Configurações → Central de contas → Suas informações e permissões → Baixar suas informações)"));
    }
    Ok(files)
}

pub fn analyze_export(path: &str) -> anyhow::Result<ExportReport> {
    let files = read_export_files(Path::new(path))?;
    let mut rep = ExportReport {
        source: path.to_string(),
        ..Default::default()
    };
    for (name, bytes) in &files {
        let Ok(v) = serde_json::from_slice::<Value>(bytes) else {
            continue;
        };
        let base = name
            .replace('\\', "/")
            .rsplit('/')
            .next()
            .unwrap_or("")
            .to_lowercase();
        let mut users = Vec::new();
        collect_users(&v, &mut users);
        rep.files_found.push(base.clone());
        let target = if base.starts_with("followers") && !base.contains("request") {
            &mut rep.followers
        } else if base == "following.json" {
            &mut rep.following
        } else if base.contains("pending_follow_requests") {
            &mut rep.pending_sent
        } else if base.contains("close_friends") {
            &mut rep.close_friends
        } else if base.contains("blocked") {
            &mut rep.blocked
        } else if base.contains("recently_unfollowed") {
            &mut rep.recently_unfollowed
        } else if base.contains("follow_requests_you") || base.contains("recent_follow_requests") {
            &mut rep.received_requests
        } else if base.contains("restricted") {
            &mut rep.restricted
        } else if base.contains("hide_story") {
            &mut rep.hide_story_from
        } else if base.contains("removed_suggestions") {
            &mut rep.removed_suggestions
        } else {
            continue;
        };
        for u in users {
            if !target.contains(&u) {
                target.push(u);
            }
        }
    }
    let fset: HashSet<&str> = rep.followers.iter().map(|u| u.username.as_str()).collect();
    let gset: HashSet<&str> = rep.following.iter().map(|u| u.username.as_str()).collect();
    rep.not_following_back = rep
        .following
        .iter()
        .filter(|u| !fset.contains(u.username.as_str()))
        .cloned()
        .collect();
    rep.fans = rep
        .followers
        .iter()
        .filter(|u| !gset.contains(u.username.as_str()))
        .cloned()
        .collect();
    rep.mutuals = rep
        .following
        .iter()
        .filter(|u| fset.contains(u.username.as_str()))
        .count();
    let mut by_month: HashMap<String, u32> = HashMap::new();
    for u in &rep.followers {
        if let Some(dt) = chrono::DateTime::from_timestamp(u.timestamp, 0) {
            *by_month.entry(dt.format("%Y-%m").to_string()).or_default() += 1;
        }
    }
    let mut months: Vec<(String, u32)> = by_month.into_iter().collect();
    months.sort();
    rep.followers_by_month = months;
    rep.followers
        .sort_by_key(|u| std::cmp::Reverse(u.timestamp));
    rep.following
        .sort_by_key(|u| std::cmp::Reverse(u.timestamp));
    Ok(rep)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn export_users() {
        let followers = serde_json::json!([{"title": "", "media_list_data": [], "string_list_data": [{"href": "https://www.instagram.com/alice", "value": "alice", "timestamp": 1700000000}]}]);
        let following = serde_json::json!({"relationships_following": [{"title": "bob", "string_list_data": [{"href": "https://www.instagram.com/bob", "timestamp": 1}]}, {"string_list_data": [{"value": "alice", "href": "", "timestamp": 2}]}]});
        let mut f = Vec::new();
        collect_users(&followers, &mut f);
        let mut g = Vec::new();
        collect_users(&following, &mut g);
        assert_eq!(f.len(), 1);
        assert_eq!(
            g.iter().map(|u| u.username.as_str()).collect::<Vec<_>>(),
            vec!["bob", "alice"]
        );
    }

    #[test]
    fn stats_basics() {
        let mk = |likes: u64, ts: i64, mt: u8| MediaItem {
            pk: String::new(),
            code: String::new(),
            media_type: mt,
            product_type: String::new(),
            taken_at: ts,
            expiring_at: None,
            caption: "#a #b".into(),
            like_count: likes,
            comment_count: 1,
            play_count: 0,
            owner_id: String::new(),
            username: String::new(),
            full_name: String::new(),
            thumbnail: String::new(),
            files: vec![],
            duration: 0.0,
            location: None,
            url: String::new(),
            width: 0,
            height: 0,
            hashtags: vec!["a".into(), "b".into()],
            mentions: vec![],
            is_paid_partnership: false,
            coauthors: vec![],
            title: None,
        };
        let user = UserInfo {
            follower_count: 1000,
            following_count: 100,
            ..Default::default()
        };
        let st = compute(user, &[mk(100, 0, 1), mk(200, 7 * 86400, 8)]);
        assert_eq!(st.avg_likes, 150.0);
        assert!((st.engagement_rate - 15.1).abs() < 0.01);
        assert_eq!(st.posts_per_week, 2.0);
        assert_eq!(st.top_hashtags[0].1, 2);
        assert_eq!(st.share_carousel, 50.0);
    }
}

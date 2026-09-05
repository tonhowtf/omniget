//! Raio-X de perfil (estudo 67): o que Black Magic / Tweet Hunter mostram,
//! calculado a partir dos ultimos N posts publicos (FxTwitter).

use anyhow::anyhow;
use serde::{Deserialize, Serialize};

use super::{XPost, XUser};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Slot {
    pub key: u32,
    pub posts: u64,
    pub avg_likes: f64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TagCount {
    pub tag: String,
    pub count: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ProfileReport {
    pub user: XUser,
    pub sampled: usize,
    pub since: String,
    pub until: String,
    pub days_spanned: f64,
    pub posts_per_day: f64,
    pub avg_likes: f64,
    pub median_likes: f64,
    pub avg_reposts: f64,
    pub avg_replies: f64,
    pub avg_views: f64,
    /// (likes + reposts + replies) medios / seguidores × 100
    pub engagement_rate: f64,
    pub reply_share: f64,
    pub repost_share: f64,
    pub media_share: f64,
    pub link_share: f64,
    /// Hora local (0–23).
    pub by_hour: Vec<Slot>,
    /// 0 = domingo.
    pub by_weekday: Vec<Slot>,
    pub best_hour: Option<u32>,
    pub best_weekday: Option<u32>,
    pub top_posts: Vec<XPost>,
    pub top_hashtags: Vec<TagCount>,
    pub top_mentions: Vec<TagCount>,
    pub utc_offset_minutes: i32,
}

pub async fn analyze(
    input: &str,
    limit: usize,
    with_replies: bool,
) -> anyhow::Result<ProfileReport> {
    let handle = super::handle_from(input)
        .ok_or_else(|| anyhow!("nao reconheci um perfil do X em: {}", input))?;
    let user = super::fx::profile(&handle).await?;
    let mut posts: Vec<XPost> = Vec::new();
    let mut cursor: Option<String> = None;
    let limit = limit.clamp(20, 1000);
    for _ in 0..40 {
        let page = super::fx::profile_statuses(&handle, cursor.as_deref(), with_replies).await?;
        if page.items.is_empty() {
            break;
        }
        posts.extend(page.items);
        if posts.len() >= limit || page.cursor.is_none() {
            break;
        }
        cursor = page.cursor;
        tokio::time::sleep(std::time::Duration::from_millis(250)).await;
    }
    posts.truncate(limit);
    let posts = super::dedup_posts(posts);
    Ok(report(user, posts))
}

pub fn report(user: XUser, posts: Vec<XPost>) -> ProfileReport {
    use chrono::{Datelike, Local, TimeZone, Timelike};
    let own: Vec<&XPost> = posts.iter().filter(|p| p.reposted_by.is_none()).collect();
    let n = own.len().max(1) as f64;
    let sum = |f: &dyn Fn(&XPost) -> u64| own.iter().map(|p| f(p) as f64).sum::<f64>();
    let avg_likes = sum(&|p| p.likes) / n;
    let avg_reposts = sum(&|p| p.reposts) / n;
    let avg_replies = sum(&|p| p.replies) / n;
    let avg_views = sum(&|p| p.views) / n;
    let mut likes_sorted: Vec<u64> = own.iter().map(|p| p.likes).collect();
    likes_sorted.sort_unstable();
    let median_likes = if likes_sorted.is_empty() {
        0.0
    } else if likes_sorted.len().is_multiple_of(2) {
        (likes_sorted[likes_sorted.len() / 2 - 1] + likes_sorted[likes_sorted.len() / 2]) as f64
            / 2.0
    } else {
        likes_sorted[likes_sorted.len() / 2] as f64
    };
    let mut by_hour: Vec<(u64, f64)> = vec![(0, 0.0); 24];
    let mut by_wd: Vec<(u64, f64)> = vec![(0, 0.0); 7];
    let offset = Local::now().offset().local_minus_utc() / 60;
    let (mut min_ts, mut max_ts) = (i64::MAX, i64::MIN);
    for p in &own {
        if p.timestamp <= 0 {
            continue;
        }
        min_ts = min_ts.min(p.timestamp);
        max_ts = max_ts.max(p.timestamp);
        if let Some(dt) = Local.timestamp_opt(p.timestamp, 0).single() {
            let h = dt.hour() as usize;
            by_hour[h].0 += 1;
            by_hour[h].1 += p.likes as f64;
            let w = dt.weekday().num_days_from_sunday() as usize;
            by_wd[w].0 += 1;
            by_wd[w].1 += p.likes as f64;
        }
    }
    let slots = |v: &[(u64, f64)]| -> Vec<Slot> {
        v.iter()
            .enumerate()
            .map(|(i, (c, l))| Slot {
                key: i as u32,
                posts: *c,
                avg_likes: if *c > 0 { l / *c as f64 } else { 0.0 },
            })
            .collect()
    };
    let best = |v: &[Slot]| -> Option<u32> {
        v.iter()
            .filter(|s| s.posts >= 2)
            .max_by(|a, b| {
                a.avg_likes
                    .partial_cmp(&b.avg_likes)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .map(|s| s.key)
    };
    let by_hour = slots(&by_hour);
    let by_weekday = slots(&by_wd);
    let days = if min_ts < max_ts {
        (max_ts - min_ts) as f64 / 86400.0
    } else {
        0.0
    };
    let mut tags: std::collections::HashMap<String, u64> = std::collections::HashMap::new();
    let mut ats: std::collections::HashMap<String, u64> = std::collections::HashMap::new();
    for p in &own {
        for t in &p.hashtags {
            *tags.entry(t.to_ascii_lowercase()).or_default() += 1;
        }
        for m in &p.mentions {
            if !m.eq_ignore_ascii_case(&user.handle) {
                *ats.entry(m.to_ascii_lowercase()).or_default() += 1;
            }
        }
    }
    let top = |m: std::collections::HashMap<String, u64>| -> Vec<TagCount> {
        let mut v: Vec<TagCount> = m
            .into_iter()
            .map(|(tag, count)| TagCount { tag, count })
            .collect();
        v.sort_by(|a, b| b.count.cmp(&a.count).then(a.tag.cmp(&b.tag)));
        v.truncate(10);
        v
    };
    let mut top_posts: Vec<XPost> = own.iter().map(|p| (*p).clone()).collect();
    top_posts.sort_by_key(|b| std::cmp::Reverse(b.likes + b.reposts * 2));
    top_posts.truncate(5);
    let share = |f: &dyn Fn(&XPost) -> bool| own.iter().filter(|p| f(p)).count() as f64 / n * 100.0;
    ProfileReport {
        sampled: own.len(),
        since: if min_ts < i64::MAX {
            super::iso_from_timestamp(min_ts)
        } else {
            String::new()
        },
        until: if max_ts > i64::MIN {
            super::iso_from_timestamp(max_ts)
        } else {
            String::new()
        },
        days_spanned: days,
        posts_per_day: if days > 0.0 {
            own.len() as f64 / days
        } else {
            0.0
        },
        avg_likes,
        median_likes,
        avg_reposts,
        avg_replies,
        avg_views,
        engagement_rate: if user.followers > 0 {
            (avg_likes + avg_reposts + avg_replies) / user.followers as f64 * 100.0
        } else {
            0.0
        },
        reply_share: share(&|p| p.is_reply()),
        repost_share: posts.iter().filter(|p| p.reposted_by.is_some()).count() as f64
            / posts.len().max(1) as f64
            * 100.0,
        media_share: share(&|p| !p.media.is_empty()),
        link_share: share(&|p| !p.links.is_empty()),
        best_hour: best(&by_hour),
        best_weekday: best(&by_weekday),
        by_hour,
        by_weekday,
        top_posts,
        top_hashtags: top(tags),
        top_mentions: top(ats),
        utc_offset_minutes: offset,
        user,
    }
}

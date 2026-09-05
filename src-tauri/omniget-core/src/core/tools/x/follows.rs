//! Quem nao me segue de volta (estudo 67): `Following` × `Followers` da
//! sessao, whitelist, unfollow com jitter (15–40 s como o
//! Twitter-X-Mass-Unfollow), limite diario e botao de parar. Nunca deixa
//! de seguir sem o usuario ver a lista antes.

use anyhow::anyhow;
use serde::{Deserialize, Serialize};
use serde_json::json;

use super::{client::XClient, ProgressFn, XUser};

pub const JOB: &str = "x-follows";
pub const UNFOLLOW_JOB: &str = "x-unfollow";

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Audit {
    pub me: XUser,
    pub following: usize,
    pub followers: usize,
    pub mutuals: usize,
    pub not_following_back: Vec<XUser>,
    pub fans: Vec<XUser>,
    pub whitelist: Vec<String>,
    pub cancelled: bool,
    pub unfollowed_today: usize,
}

fn whitelist_path() -> std::path::PathBuf {
    super::x_dir().join("whitelist.json")
}

pub fn whitelist() -> Vec<String> {
    std::fs::read_to_string(whitelist_path()).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
}

pub fn set_whitelist(handles: &[String]) -> anyhow::Result<Vec<String>> {
    let mut clean: Vec<String> = handles.iter().map(|h| h.trim().trim_start_matches('@').to_ascii_lowercase()).filter(|h| !h.is_empty()).collect();
    clean.sort();
    clean.dedup();
    std::fs::write(whitelist_path(), serde_json::to_string_pretty(&clean)?)?;
    Ok(clean)
}

fn log_path() -> std::path::PathBuf {
    super::x_dir().join("unfollow-log.json")
}

fn today() -> String {
    chrono::Local::now().format("%Y-%m-%d").to_string()
}

pub fn unfollowed_today() -> usize {
    let log: std::collections::HashMap<String, usize> = std::fs::read_to_string(log_path()).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default();
    log.get(&today()).copied().unwrap_or(0)
}

fn bump_today() {
    let mut log: std::collections::HashMap<String, usize> = std::fs::read_to_string(log_path()).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default();
    *log.entry(today()).or_default() += 1;
    let _ = std::fs::write(log_path(), serde_json::to_string(&log).unwrap_or_default());
}

pub async fn me(client: &XClient) -> anyhow::Result<XUser> {
    let uid = client.user_id().ok_or_else(|| anyhow!("cookie twid ausente: entre de novo no X"))?;
    let v = client
        .gql_get(
            "UserByRestId",
            json!({ "userId": uid, "withSafetyModeUserFields": true }),
            json!({ "hidden_profile_likes_enabled": true, "subscriptions_verification_info_is_identity_verified_enabled": false, "responsive_web_twitter_article_notes_tab_enabled": false, "subscriptions_feature_can_gift_premium": false, "profile_label_improvements_pcf_label_in_post_enabled": false }),
            None,
        )
        .await?;
    v.pointer("/data/user/result").and_then(super::parse::parse_user).ok_or_else(|| anyhow!("nao consegui ler o perfil da sessao"))
}

async fn list(client: &XClient, op: &str, uid: &str, limit: usize, progress: &ProgressFn, stage: &str) -> anyhow::Result<Vec<XUser>> {
    let mut users: Vec<XUser> = Vec::new();
    let extra = if op == "Followers" { json!({ "responsive_web_twitter_article_notes_tab_enabled": false }) } else { json!({}) };
    let p2 = progress.clone();
    client
        .paginate(op, json!({ "userId": uid, "count": 100, "includePromotedContent": false }), extra, limit, JOB, |page| {
            let got = super::parse::users_from(page);
            let n = got.len();
            users.extend(got);
            super::report(&p2, JOB, stage, users.len() as u64, None, None);
            n
        })
        .await?;
    let mut seen = std::collections::HashSet::new();
    Ok(users.into_iter().filter(|u| seen.insert(u.id.clone())).collect())
}

pub async fn audit(limit: usize, progress: ProgressFn) -> anyhow::Result<Audit> {
    let client = XClient::new()?;
    client.require_login()?;
    super::clear_cancel(JOB);
    let me = me(&client).await?;
    let limit = if limit == 0 { 10_000 } else { limit };
    let following = list(&client, "Following", &me.id, limit, &progress, "following").await?;
    let followers = if super::cancelled(JOB) { Vec::new() } else { list(&client, "Followers", &me.id, limit, &progress, "followers").await? };
    let follower_ids: std::collections::HashSet<&str> = followers.iter().map(|u| u.id.as_str()).collect();
    let following_ids: std::collections::HashSet<&str> = following.iter().map(|u| u.id.as_str()).collect();
    let wl = whitelist();
    let not_following_back: Vec<XUser> = following
        .iter()
        .filter(|u| !follower_ids.contains(u.id.as_str()) && u.follows_me != Some(true))
        .filter(|u| !wl.contains(&u.handle.to_ascii_lowercase()))
        .cloned()
        .collect();
    let fans: Vec<XUser> = followers.iter().filter(|u| !following_ids.contains(u.id.as_str()) && u.followed_by_me != Some(true)).cloned().collect();
    let mutuals = following.iter().filter(|u| follower_ids.contains(u.id.as_str()) || u.follows_me == Some(true)).count();
    super::report(&progress, JOB, "done", following.len() as u64, Some(following.len() as u64), None);
    Ok(Audit {
        me,
        following: following.len(),
        followers: followers.len(),
        mutuals,
        not_following_back,
        fans,
        whitelist: wl,
        cancelled: super::cancelled(JOB),
        unfollowed_today: unfollowed_today(),
    })
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct UnfollowResult {
    pub done: Vec<String>,
    pub failed: Vec<(String, String)>,
    pub stopped: bool,
    pub reason: String,
}

pub async fn unfollow(ids: &[String], min_delay: u64, max_delay: u64, daily_cap: usize, progress: ProgressFn) -> anyhow::Result<UnfollowResult> {
    let client = XClient::new()?;
    client.require_login()?;
    super::clear_cancel(UNFOLLOW_JOB);
    let (lo, hi) = (min_delay.max(3), max_delay.max(min_delay.max(3)));
    let mut r = UnfollowResult::default();
    let total = ids.len() as u64;
    for (i, id) in ids.iter().enumerate() {
        if super::cancelled(UNFOLLOW_JOB) {
            r.stopped = true;
            r.reason = "cancelled".into();
            break;
        }
        if daily_cap > 0 && unfollowed_today() >= daily_cap {
            r.stopped = true;
            r.reason = "daily_cap".into();
            break;
        }
        super::report(&progress, UNFOLLOW_JOB, "unfollow", i as u64, Some(total), Some(id.clone()));
        let form = [
            ("include_profile_interstitial_type", "1"),
            ("include_blocking", "1"),
            ("include_blocked_by", "1"),
            ("include_followed_by", "1"),
            ("include_want_retweets", "1"),
            ("include_mute_edge", "1"),
            ("include_can_dm", "1"),
            ("include_can_media_tag", "1"),
            ("include_ext_is_blue_verified", "1"),
            ("include_ext_verified_type", "1"),
            ("include_ext_profile_image_shape", "1"),
            ("skip_status", "1"),
            ("user_id", id.as_str()),
        ];
        match client.rest_post_form("friendships/destroy.json", &form).await {
            Ok(_) => {
                r.done.push(id.clone());
                bump_today();
            }
            Err(e) => {
                let msg = e.to_string();
                r.failed.push((id.clone(), msg.clone()));
                if msg.contains("X_RATE_LIMIT") {
                    r.stopped = true;
                    r.reason = msg;
                    break;
                }
            }
        }
        if i + 1 < ids.len() {
            let wait = lo + (rand::random::<u64>() % (hi - lo + 1));
            for s in 0..wait {
                if super::cancelled(UNFOLLOW_JOB) {
                    break;
                }
                super::report(&progress, UNFOLLOW_JOB, "waiting", (i + 1) as u64, Some(total), Some((wait - s).to_string()));
                tokio::time::sleep(std::time::Duration::from_secs(1)).await;
            }
        }
    }
    super::report(&progress, UNFOLLOW_JOB, "done", r.done.len() as u64, Some(total), None);
    Ok(r)
}

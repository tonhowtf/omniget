//! Seguidores e seguindo: listas paginadas, quem não segue de volta, fãs,
//! mútuos, lista branca, deixar de seguir / remover seguidor com ritmo
//! seguro (referência: InstagramUnfollowers, MIT — 4 s entre ações e 5 min
//! a cada cinco; aqui o padrão é mais conservador e há teto diário), e
//! snapshots locais para saber quem deixou de seguir.

use std::collections::{HashMap, HashSet};
use std::sync::atomic::AtomicBool;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{b, s, IgClient, IgError};

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct MiniUser {
    pub pk: String,
    pub username: String,
    pub full_name: String,
    pub is_private: bool,
    pub is_verified: bool,
    pub profile_pic_url: String,
}

impl MiniUser {
    pub fn from_value(v: &Value) -> MiniUser {
        MiniUser {
            pk: {
                let p = s(v, "pk");
                if p.is_empty() { s(v, "id") } else { p }
            },
            username: s(v, "username"),
            full_name: s(v, "full_name"),
            is_private: b(v, "is_private"),
            is_verified: b(v, "is_verified"),
            profile_pic_url: s(v, "profile_pic_url"),
        }
    }
}

async fn friendship_list(client: &IgClient, user_id: &str, which: &str, limit: usize, flag: &AtomicBool, progress: &super::super::ProgressFn, job: &str) -> Result<Vec<MiniUser>, IgError> {
    let mut out = Vec::new();
    let mut max_id: Option<String> = None;
    let id = format!("ig:{}", job);
    loop {
        let mut q = vec![("count", "50".to_string()), ("search_surface", "follow_list_page".to_string())];
        if let Some(m) = &max_id {
            q.push(("max_id", m.clone()));
        }
        let json = client.get_json(&format!("/api/v1/friendships/{}/{}/", user_id, which), &q).await?;
        let users = json.get("users").and_then(|u| u.as_array()).cloned().unwrap_or_default();
        for us in &users {
            out.push(MiniUser::from_value(us));
        }
        super::super::report(progress, &id, which, out.len() as u64, if limit > 0 { Some(limit as u64) } else { None }, None);
        max_id = json.get("next_max_id").and_then(|m| match m { Value::String(x) => Some(x.clone()), Value::Number(n) => Some(n.to_string()), _ => None });
        if users.is_empty() || max_id.is_none() || (limit > 0 && out.len() >= limit) || super::cancelled(flag) {
            break;
        }
        client.pause().await;
    }
    Ok(out)
}

pub async fn followers(client: &IgClient, user_id: &str, limit: usize, flag: &AtomicBool, progress: &super::super::ProgressFn, job: &str) -> Result<Vec<MiniUser>, IgError> {
    friendship_list(client, user_id, "followers", limit, flag, progress, job).await
}

pub async fn following(client: &IgClient, user_id: &str, limit: usize, flag: &AtomicBool, progress: &super::super::ProgressFn, job: &str) -> Result<Vec<MiniUser>, IgError> {
    friendship_list(client, user_id, "following", limit, flag, progress, job).await
}

#[derive(Debug, Clone, Serialize)]
pub struct FollowAnalysis {
    pub followers_count: usize,
    pub following_count: usize,
    /// Eu sigo, não me segue.
    pub not_following_back: Vec<MiniUser>,
    /// Me segue, eu não sigo.
    pub fans: Vec<MiniUser>,
    pub mutuals: Vec<MiniUser>,
    pub whitelisted: usize,
    pub followers: Vec<MiniUser>,
    pub following: Vec<MiniUser>,
}

pub fn analyze(followers: Vec<MiniUser>, following: Vec<MiniUser>, whitelist: &HashSet<String>) -> FollowAnalysis {
    let fset: HashSet<&str> = followers.iter().map(|u| u.pk.as_str()).collect();
    let gset: HashSet<&str> = following.iter().map(|u| u.pk.as_str()).collect();
    let mut whitelisted = 0;
    let not_following_back: Vec<MiniUser> = following
        .iter()
        .filter(|u| !fset.contains(u.pk.as_str()))
        .filter(|u| {
            let wl = whitelist.contains(&u.pk) || whitelist.contains(&u.username);
            if wl {
                whitelisted += 1;
            }
            !wl
        })
        .cloned()
        .collect();
    let fans = followers.iter().filter(|u| !gset.contains(u.pk.as_str())).cloned().collect();
    let mutuals = following.iter().filter(|u| fset.contains(u.pk.as_str())).cloned().collect();
    FollowAnalysis { followers_count: followers.len(), following_count: following.len(), not_following_back, fans, mutuals, whitelisted, followers, following }
}

// ── Ações ────────────────────────────────────────────────────────────────

pub async fn unfollow(client: &IgClient, user_id: &str) -> Result<(), IgError> {
    let r = client.post_form(&format!("/api/v1/web/friendships/{}/unfollow/", user_id), &[]).await;
    match r {
        Ok(_) => Ok(()),
        Err(IgError::NotFound(_)) | Err(IgError::Other(_)) => client.post_form(&format!("/web/friendships/{}/unfollow/", user_id), &[]).await.map(|_| ()),
        Err(e) => Err(e),
    }
}

pub async fn remove_follower(client: &IgClient, user_id: &str) -> Result<(), IgError> {
    let r = client.post_form(&format!("/api/v1/web/friendships/{}/remove_follower/", user_id), &[]).await;
    match r {
        Ok(_) => Ok(()),
        Err(IgError::NotFound(_)) | Err(IgError::Other(_)) => client.post_form(&format!("/web/friendships/{}/remove_follower/", user_id), &[]).await.map(|_| ()),
        Err(e) => Err(e),
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Pacing {
    pub delay_min_ms: u64,
    pub delay_max_ms: u64,
    /// Pausa longa a cada N ações.
    pub pause_every: u32,
    pub pause_ms: u64,
    /// Teto de ações por dia (todas as ações de escrita somadas).
    pub daily_cap: u32,
}

impl Default for Pacing {
    fn default() -> Self {
        Pacing { delay_min_ms: 6000, delay_max_ms: 14000, pause_every: 5, pause_ms: 300_000, daily_cap: 100 }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct DailyCounter {
    day: String,
    count: u32,
}

fn counter_path(owner: &str) -> std::path::PathBuf {
    super::data_dir().join(format!("actions-{}.json", owner))
}

pub fn actions_today(owner: &str) -> u32 {
    let c: DailyCounter = super::read_json(&counter_path(owner));
    if c.day == chrono::Local::now().format("%Y-%m-%d").to_string() { c.count } else { 0 }
}

fn bump_today(owner: &str) -> u32 {
    let today = chrono::Local::now().format("%Y-%m-%d").to_string();
    let mut c: DailyCounter = super::read_json(&counter_path(owner));
    if c.day != today {
        c = DailyCounter { day: today, count: 0 };
    }
    c.count += 1;
    let _ = super::write_json(&counter_path(owner), &c);
    c.count
}

#[derive(Debug, Clone, Serialize)]
pub struct ActionReport {
    pub done: Vec<MiniUser>,
    pub failed: Vec<(MiniUser, String)>,
    pub remaining: Vec<MiniUser>,
    /// "finished" | "cancelled" | "daily_cap" | "blocked" | "rate_limited" | "login"
    pub stopped: String,
    pub actions_today: u32,
}

/// Executa `unfollow` ou `remove_follower` numa lista com ritmo humano.
/// Para em 429 (espera 10 min e tenta mais uma vez), em bloqueio, em
/// sessão caída, no teto diário e quando o job é cancelado.
pub async fn run_actions(client: &IgClient, action: &str, users: Vec<MiniUser>, pacing: &Pacing, flag: &AtomicBool, progress: &super::super::ProgressFn, job: &str) -> ActionReport {
    let owner = client.session.user_id.clone();
    let id = format!("ig:{}", job);
    let total = users.len() as u64;
    let mut done = Vec::new();
    let mut failed = Vec::new();
    let mut stopped = "finished".to_string();
    let mut iter = users.into_iter();
    let mut n = 0u32;
    let mut waited_429 = false;
    while let Some(user) = iter.next() {
        if super::cancelled(flag) {
            stopped = "cancelled".into();
            let mut rest = vec![user];
            rest.extend(iter);
            return ActionReport { actions_today: actions_today(&owner), done, failed, remaining: rest, stopped };
        }
        if actions_today(&owner) >= pacing.daily_cap {
            stopped = "daily_cap".into();
            let mut rest = vec![user];
            rest.extend(iter);
            return ActionReport { actions_today: actions_today(&owner), done, failed, remaining: rest, stopped };
        }
        super::super::report(progress, &id, action, done.len() as u64, Some(total), Some(format!("@{}", user.username)));
        let result = match action {
            "remove_follower" => remove_follower(client, &user.pk).await,
            _ => unfollow(client, &user.pk).await,
        };
        match result {
            Ok(()) => {
                bump_today(&owner);
                done.push(user);
                n += 1;
            }
            Err(IgError::RateLimited) if !waited_429 => {
                waited_429 = true;
                super::super::report(progress, &id, "waiting", done.len() as u64, Some(total), Some("429".into()));
                tokio::time::sleep(std::time::Duration::from_secs(600)).await;
                match if action == "remove_follower" { remove_follower(client, &user.pk).await } else { unfollow(client, &user.pk).await } {
                    Ok(()) => {
                        bump_today(&owner);
                        done.push(user);
                    }
                    Err(e) => {
                        stopped = "rate_limited".into();
                        failed.push((user, e.to_string()));
                        return ActionReport { actions_today: actions_today(&owner), done, failed, remaining: iter.collect(), stopped };
                    }
                }
            }
            Err(e @ (IgError::RateLimited | IgError::ActionBlocked | IgError::Checkpoint | IgError::LoginRequired)) => {
                stopped = match e {
                    IgError::RateLimited => "rate_limited",
                    IgError::ActionBlocked => "blocked",
                    IgError::Checkpoint => "checkpoint",
                    _ => "login",
                }
                .into();
                failed.push((user, e.to_string()));
                return ActionReport { actions_today: actions_today(&owner), done, failed, remaining: iter.collect(), stopped };
            }
            Err(e) => failed.push((user, e.to_string())),
        }
        super::super::report(progress, &id, action, done.len() as u64, Some(total), None);
        if pacing.pause_every > 0 && n > 0 && n.is_multiple_of(pacing.pause_every) {
            super::super::report(progress, &id, "pause", done.len() as u64, Some(total), Some(format!("{}s", pacing.pause_ms / 1000)));
            tokio::time::sleep(std::time::Duration::from_millis(pacing.pause_ms)).await;
        } else {
            super::sleep_jitter(pacing.delay_min_ms, pacing.delay_max_ms).await;
        }
    }
    ActionReport { actions_today: actions_today(&owner), done, failed, remaining: Vec::new(), stopped }
}

// ── Lista branca ─────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Whitelist {
    pub users: Vec<MiniUser>,
}

fn whitelist_path(owner: &str) -> std::path::PathBuf {
    super::data_dir().join(format!("whitelist-{}.json", owner))
}

pub fn whitelist_get(owner: &str) -> Whitelist {
    super::read_json(&whitelist_path(owner))
}

pub fn whitelist_set(owner: &str, wl: &Whitelist) -> anyhow::Result<()> {
    super::write_json(&whitelist_path(owner), wl)
}

pub fn whitelist_keys(wl: &Whitelist) -> HashSet<String> {
    wl.users.iter().flat_map(|u| [u.pk.clone(), u.username.to_lowercase()]).filter(|k| !k.is_empty()).collect()
}

// ── Snapshots ────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Snapshot {
    pub taken_at: i64,
    pub owner: String,
    pub followers: Vec<MiniUser>,
    pub following: Vec<MiniUser>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SnapshotMeta {
    pub file: String,
    pub taken_at: i64,
    pub followers: usize,
    pub following: usize,
}

fn snapshots_dir(owner: &str) -> std::path::PathBuf {
    super::data_dir().join("snapshots").join(owner)
}

pub fn snapshot_save(snap: &Snapshot) -> anyhow::Result<String> {
    let dir = snapshots_dir(&snap.owner);
    let path = dir.join(format!("{}.json", snap.taken_at));
    super::write_json(&path, snap)?;
    Ok(path.to_string_lossy().to_string())
}

pub fn snapshots_list(owner: &str) -> Vec<SnapshotMeta> {
    let mut out = Vec::new();
    if let Ok(rd) = std::fs::read_dir(snapshots_dir(owner)) {
        for e in rd.flatten() {
            let p = e.path();
            if p.extension().map(|x| x == "json").unwrap_or(false) {
                let snap: Snapshot = super::read_json(&p);
                if snap.taken_at > 0 {
                    out.push(SnapshotMeta { file: p.to_string_lossy().to_string(), taken_at: snap.taken_at, followers: snap.followers.len(), following: snap.following.len() });
                }
            }
        }
    }
    out.sort_by_key(|m| std::cmp::Reverse(m.taken_at));
    out
}

pub fn snapshot_load(file: &str) -> Snapshot {
    super::read_json(std::path::Path::new(file))
}

#[derive(Debug, Clone, Serialize)]
pub struct SnapshotDiff {
    pub from: i64,
    pub to: i64,
    pub new_followers: Vec<MiniUser>,
    pub lost_followers: Vec<MiniUser>,
    pub new_following: Vec<MiniUser>,
    pub lost_following: Vec<MiniUser>,
}

pub fn snapshot_diff(a: &Snapshot, b: &Snapshot) -> SnapshotDiff {
    let by_pk = |v: &[MiniUser]| -> HashMap<String, MiniUser> { v.iter().map(|u| (u.pk.clone(), u.clone())).collect() };
    let (fa, fb) = (by_pk(&a.followers), by_pk(&b.followers));
    let (ga, gb) = (by_pk(&a.following), by_pk(&b.following));
    let only = |x: &HashMap<String, MiniUser>, y: &HashMap<String, MiniUser>| -> Vec<MiniUser> {
        let mut v: Vec<MiniUser> = x.iter().filter(|(k, _)| !y.contains_key(*k)).map(|(_, u)| u.clone()).collect();
        v.sort_by(|p, q| p.username.cmp(&q.username));
        v
    };
    SnapshotDiff { from: a.taken_at, to: b.taken_at, new_followers: only(&fb, &fa), lost_followers: only(&fa, &fb), new_following: only(&gb, &ga), lost_following: only(&ga, &gb) }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn u(pk: &str) -> MiniUser {
        MiniUser { pk: pk.into(), username: format!("u{}", pk), ..Default::default() }
    }

    #[test]
    fn analyze_sets() {
        let followers = vec![u("1"), u("2")];
        let following = vec![u("2"), u("3"), u("4")];
        let wl: HashSet<String> = ["4".to_string()].into_iter().collect();
        let a = analyze(followers, following, &wl);
        assert_eq!(a.not_following_back.iter().map(|x| x.pk.as_str()).collect::<Vec<_>>(), vec!["3"]);
        assert_eq!(a.fans.iter().map(|x| x.pk.as_str()).collect::<Vec<_>>(), vec!["1"]);
        assert_eq!(a.mutuals.iter().map(|x| x.pk.as_str()).collect::<Vec<_>>(), vec!["2"]);
        assert_eq!(a.whitelisted, 1);
    }

    #[test]
    fn diff_snapshots() {
        let a = Snapshot { taken_at: 1, owner: "me".into(), followers: vec![u("1"), u("2")], following: vec![u("9")] };
        let b = Snapshot { taken_at: 2, owner: "me".into(), followers: vec![u("2"), u("3")], following: vec![] };
        let d = snapshot_diff(&a, &b);
        assert_eq!(d.new_followers[0].pk, "3");
        assert_eq!(d.lost_followers[0].pk, "1");
        assert_eq!(d.lost_following[0].pk, "9");
        assert!(d.new_following.is_empty());
    }
}

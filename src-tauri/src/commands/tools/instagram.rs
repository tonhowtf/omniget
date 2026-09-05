//! Comandos da categoria Instagram. A sessão vem do gerenciador de cookies
//! (bucket `instagram.com`, preenchido pela extensão); cada comando recebe
//! o `slug` da conta (None = `_default`). Jobs longos reportam em
//! `tool-progress` com id `ig:<job>` e podem ser cancelados por `tool_ig_cancel`.

use std::sync::atomic::AtomicBool;
use std::sync::{Arc, OnceLock};

use omniget_core::core::tools::instagram::{self as ig, analytics, follow, media, profile, publish, social, IgClient, Session};
use serde::Serialize;

use super::{err, progress};

const DOMAIN: &str = "instagram.com";

pub(crate) fn load_client(slug: Option<&str>) -> Result<Arc<IgClient>, String> {
    let slug_name = slug.unwrap_or("_default");
    let content = crate::cookies::storage::read_account_file(DOMAIN, slug_name).map_err(|_| ig::IgError::NoSession.to_string())?;
    let session = Session::from_netscape(&content).map_err(|e| e.to_string())?;
    crate::cookies::touch_last_used(DOMAIN, slug_name);
    IgClient::new(session).map_err(err)
}

struct Job {
    id: String,
    flag: Arc<AtomicBool>,
}

impl Job {
    fn new(id: &str) -> Self {
        Job { id: id.to_string(), flag: ig::job_start(id) }
    }
}

impl Drop for Job {
    fn drop(&mut self) {
        ig::job_finish(&self.id);
    }
}

// ── Conta ────────────────────────────────────────────────────────────────

#[derive(Debug, Serialize)]
pub struct IgAccount {
    pub slug: String,
    pub alias: String,
    pub captured_at_ms: i64,
    pub cookie_count: usize,
    pub has_session: bool,
    pub user_id: String,
}

#[tauri::command]
pub fn tool_ig_accounts() -> Vec<IgAccount> {
    let registry = crate::cookies::load_registry();
    let Some(bucket) = registry.buckets.get(DOMAIN) else { return Vec::new() };
    bucket
        .accounts
        .iter()
        .map(|a| {
            let content = crate::cookies::storage::read_account_file(DOMAIN, &a.slug).unwrap_or_default();
            let session = Session::from_netscape(&content).ok();
            IgAccount {
                slug: a.slug.clone(),
                alias: a.alias.clone(),
                captured_at_ms: a.captured_at_ms,
                cookie_count: a.cookie_count,
                has_session: session.is_some(),
                user_id: session.map(|s| s.user_id).unwrap_or_default(),
            }
        })
        .collect()
}

#[tauri::command]
pub async fn tool_ig_whoami(slug: Option<String>) -> Result<profile::UserInfo, String> {
    let client = load_client(slug.as_deref())?;
    profile::whoami(&client).await.map_err(err)
}

#[tauri::command]
pub fn tool_ig_parse(input: String) -> ig::IgTarget {
    ig::parse_target(&input)
}

#[tauri::command]
pub fn tool_ig_cancel(job: String) -> bool {
    ig::job_cancel(&job)
}

// ── Download ─────────────────────────────────────────────────────────────

#[tauri::command]
pub async fn tool_ig_post(slug: Option<String>, url: String) -> Result<media::MediaItem, String> {
    let client = load_client(slug.as_deref())?;
    match ig::parse_target(&url) {
        ig::IgTarget::Post { shortcode } => media::post_info(&client, &shortcode).await.map_err(err),
        _ => Err("cole o link de um post, reel ou IGTV".into()),
    }
}

/// Resolve qualquer link (post, story, highlight, perfil) em itens de mídia.
async fn resolve_items(client: &IgClient, url: &str, flag: &AtomicBool, p: &omniget_core::core::tools::ProgressFn, job: &str) -> Result<Vec<media::MediaItem>, String> {
    match ig::parse_target(url) {
        ig::IgTarget::Post { shortcode } => Ok(vec![media::post_info(client, &shortcode).await.map_err(err)?]),
        ig::IgTarget::Story { username, media_id } => {
            let user = profile::resolve_user(client, &username).await.map_err(err)?;
            let mut items = profile::stories_of(client, &user.pk).await.map_err(err)?;
            if let Some(id) = media_id {
                items.retain(|i| i.pk == id);
            }
            Ok(items)
        }
        ig::IgTarget::Highlight { id } => profile::highlight_items(client, &id).await.map_err(err),
        ig::IgTarget::Profile { username, tab } => {
            let user = profile::resolve_user(client, &username).await.map_err(err)?;
            match tab.as_str() {
                "reels" => profile::user_reels(client, &user.pk, 24, flag, p, job).await.map_err(err),
                "tagged" => profile::user_tagged(client, &user.pk, 24, flag, p, job).await.map_err(err),
                _ => profile::user_posts(client, &user.username, 24, flag, p, job).await.map_err(err),
            }
        }
        ig::IgTarget::Tag { name } => social::tag_media(client, &name, "recent", 24, flag, p, job).await.map_err(err),
        ig::IgTarget::Unknown => Err(format!("nao reconheci este link do Instagram: {}", url)),
    }
}

#[tauri::command]
pub async fn tool_ig_resolve(app: tauri::AppHandle, slug: Option<String>, url: String, job: String) -> Result<Vec<media::MediaItem>, String> {
    let client = load_client(slug.as_deref())?;
    let j = Job::new(&job);
    resolve_items(&client, &url, &j.flag, &progress(&app), &job).await
}

#[tauri::command]
pub async fn tool_ig_download(app: tauri::AppHandle, slug: Option<String>, items: Vec<media::MediaItem>, dest: String, opts: media::DownloadOptions, job: String) -> Result<media::DownloadResult, String> {
    let client = load_client(slug.as_deref())?;
    let j = Job::new(&job);
    media::download_items(&client, &items, &dest, &opts, &progress(&app), &job, &j.flag).await.map_err(err)
}

#[derive(Debug, Serialize)]
pub struct BulkResult {
    pub result: media::DownloadResult,
    pub errors: Vec<String>,
    pub items: usize,
}

/// Vários links de uma vez (um por linha), com pausa entre eles.
#[tauri::command]
pub async fn tool_ig_download_bulk(app: tauri::AppHandle, slug: Option<String>, urls: Vec<String>, dest: String, opts: media::DownloadOptions, job: String) -> Result<BulkResult, String> {
    let client = load_client(slug.as_deref())?;
    let j = Job::new(&job);
    let p = progress(&app);
    let mut items = Vec::new();
    let mut errors = Vec::new();
    let total = urls.len() as u64;
    for (i, url) in urls.iter().enumerate() {
        if ig::cancelled(&j.flag) {
            break;
        }
        let url = url.trim();
        if url.is_empty() {
            continue;
        }
        omniget_core::core::tools::report(&p, &format!("ig:{}", job), "resolve", i as u64, Some(total), Some(url.to_string()));
        match resolve_items(&client, url, &j.flag, &p, &job).await {
            Ok(mut v) => items.append(&mut v),
            Err(e) => errors.push(format!("{}: {}", url, e)),
        }
        client.pause().await;
    }
    let result = media::download_items(&client, &items, &dest, &opts, &p, &job, &j.flag).await.map_err(err)?;
    Ok(BulkResult { items: items.len(), result, errors })
}

// ── Perfil, stories e highlights ─────────────────────────────────────────

#[tauri::command]
pub async fn tool_ig_profile(slug: Option<String>, user: String) -> Result<profile::UserInfo, String> {
    let client = load_client(slug.as_deref())?;
    profile::resolve_user(&client, &user).await.map_err(err)
}

#[tauri::command]
pub async fn tool_ig_friendship(slug: Option<String>, user_id: String) -> Result<profile::Friendship, String> {
    let client = load_client(slug.as_deref())?;
    profile::friendship(&client, &user_id).await.map_err(err)
}

/// `tab` = posts | reels | tagged | saved | stories | highlights (todos os itens).
#[tauri::command]
pub async fn tool_ig_profile_media(app: tauri::AppHandle, slug: Option<String>, user: String, tab: String, limit: usize, job: String) -> Result<Vec<media::MediaItem>, String> {
    let client = load_client(slug.as_deref())?;
    let j = Job::new(&job);
    let p = progress(&app);
    if tab == "saved" {
        return profile::saved(&client, limit, &j.flag, &p, &job).await.map_err(err);
    }
    let info = profile::resolve_user(&client, &user).await.map_err(err)?;
    match tab.as_str() {
        "reels" => profile::user_reels(&client, &info.pk, limit, &j.flag, &p, &job).await.map_err(err),
        "tagged" => profile::user_tagged(&client, &info.pk, limit, &j.flag, &p, &job).await.map_err(err),
        "stories" => profile::stories_of(&client, &info.pk).await.map_err(err),
        "highlights" => {
            let hls = profile::highlights(&client, &info.pk).await.map_err(err)?;
            let mut out = Vec::new();
            for (i, h) in hls.iter().enumerate() {
                if ig::cancelled(&j.flag) {
                    break;
                }
                omniget_core::core::tools::report(&p, &format!("ig:{}", job), "highlights", i as u64, Some(hls.len() as u64), Some(h.title.clone()));
                out.extend(profile::highlight_items(&client, &h.id).await.map_err(err)?);
                client.pause().await;
            }
            Ok(out)
        }
        _ => profile::user_posts(&client, &info.username, limit, &j.flag, &p, &job).await.map_err(err),
    }
}

#[tauri::command]
pub async fn tool_ig_stories(slug: Option<String>, user: String) -> Result<Vec<profile::Reel>, String> {
    let client = load_client(slug.as_deref())?;
    let info = profile::resolve_user(&client, &user).await.map_err(err)?;
    profile::reels_media(&client, &[info.pk]).await.map_err(err)
}

#[tauri::command]
pub async fn tool_ig_stories_tray(slug: Option<String>) -> Result<Vec<profile::TrayEntry>, String> {
    let client = load_client(slug.as_deref())?;
    profile::stories_tray(&client).await.map_err(err)
}

#[tauri::command]
pub async fn tool_ig_highlights(slug: Option<String>, user: String) -> Result<Vec<profile::Highlight>, String> {
    let client = load_client(slug.as_deref())?;
    let info = profile::resolve_user(&client, &user).await.map_err(err)?;
    profile::highlights(&client, &info.pk).await.map_err(err)
}

#[tauri::command]
pub async fn tool_ig_highlight_items(slug: Option<String>, id: String) -> Result<Vec<media::MediaItem>, String> {
    let client = load_client(slug.as_deref())?;
    profile::highlight_items(&client, &id).await.map_err(err)
}

#[derive(Debug, Serialize)]
pub struct StoryViewers {
    pub total: u64,
    pub viewers: Vec<follow::MiniUser>,
}

#[tauri::command]
pub async fn tool_ig_story_viewers(slug: Option<String>, story_pk: String, job: String) -> Result<StoryViewers, String> {
    let client = load_client(slug.as_deref())?;
    let j = Job::new(&job);
    let (total, viewers) = profile::story_viewers(&client, &story_pk, &j.flag).await.map_err(err)?;
    Ok(StoryViewers { total, viewers })
}

// ── Seguidores ───────────────────────────────────────────────────────────

fn owner_of(client: &IgClient) -> String {
    client.session.user_id.clone()
}

#[tauri::command]
pub async fn tool_ig_follow_lists(app: tauri::AppHandle, slug: Option<String>, user: Option<String>, limit: usize, job: String) -> Result<follow::FollowAnalysis, String> {
    let client = load_client(slug.as_deref())?;
    let j = Job::new(&job);
    let p = progress(&app);
    let target = match user.filter(|u| !u.trim().is_empty()) {
        Some(u) => profile::resolve_user(&client, &u).await.map_err(err)?.pk,
        None => owner_of(&client),
    };
    let followers = follow::followers(&client, &target, limit, &j.flag, &p, &job).await.map_err(err)?;
    let following = follow::following(&client, &target, limit, &j.flag, &p, &job).await.map_err(err)?;
    let wl = follow::whitelist_keys(&follow::whitelist_get(&owner_of(&client)));
    Ok(follow::analyze(followers, following, &wl))
}

#[tauri::command]
pub fn tool_ig_whitelist_get(slug: Option<String>) -> Result<follow::Whitelist, String> {
    let client = load_client(slug.as_deref())?;
    Ok(follow::whitelist_get(&owner_of(&client)))
}

#[tauri::command]
pub fn tool_ig_whitelist_set(slug: Option<String>, users: Vec<follow::MiniUser>) -> Result<follow::Whitelist, String> {
    let client = load_client(slug.as_deref())?;
    let wl = follow::Whitelist { users };
    follow::whitelist_set(&owner_of(&client), &wl).map_err(err)?;
    Ok(wl)
}

#[tauri::command]
pub fn tool_ig_actions_today(slug: Option<String>) -> Result<u32, String> {
    let client = load_client(slug.as_deref())?;
    Ok(follow::actions_today(&owner_of(&client)))
}

/// `action` = unfollow | remove_follower.
#[tauri::command]
pub async fn tool_ig_actions(app: tauri::AppHandle, slug: Option<String>, action: String, users: Vec<follow::MiniUser>, pacing: Option<follow::Pacing>, job: String) -> Result<follow::ActionReport, String> {
    let client = load_client(slug.as_deref())?;
    let j = Job::new(&job);
    Ok(follow::run_actions(&client, &action, users, &pacing.unwrap_or_default(), &j.flag, &progress(&app), &job).await)
}

#[tauri::command]
pub async fn tool_ig_resolve_users(slug: Option<String>, usernames: Vec<String>, job: String) -> Result<Vec<follow::MiniUser>, String> {
    let client = load_client(slug.as_deref())?;
    let j = Job::new(&job);
    let mut out = Vec::new();
    for u in usernames {
        if ig::cancelled(&j.flag) {
            break;
        }
        if let Ok(info) = profile::resolve_user(&client, &u).await {
            out.push(follow::MiniUser { pk: info.pk, username: info.username, full_name: info.full_name, is_private: info.is_private, is_verified: info.is_verified, profile_pic_url: info.profile_pic_url });
        }
        client.pause().await;
    }
    Ok(out)
}

#[tauri::command]
pub async fn tool_ig_snapshot_take(app: tauri::AppHandle, slug: Option<String>, job: String) -> Result<follow::SnapshotMeta, String> {
    let client = load_client(slug.as_deref())?;
    let j = Job::new(&job);
    let p = progress(&app);
    let owner = owner_of(&client);
    let followers = follow::followers(&client, &owner, 0, &j.flag, &p, &job).await.map_err(err)?;
    let following = follow::following(&client, &owner, 0, &j.flag, &p, &job).await.map_err(err)?;
    if ig::cancelled(&j.flag) {
        return Err("cancelado".into());
    }
    let snap = follow::Snapshot { taken_at: chrono::Utc::now().timestamp(), owner: owner.clone(), followers, following };
    let file = follow::snapshot_save(&snap).map_err(err)?;
    Ok(follow::SnapshotMeta { file, taken_at: snap.taken_at, followers: snap.followers.len(), following: snap.following.len() })
}

#[tauri::command]
pub fn tool_ig_snapshots(slug: Option<String>) -> Result<Vec<follow::SnapshotMeta>, String> {
    let client = load_client(slug.as_deref())?;
    Ok(follow::snapshots_list(&owner_of(&client)))
}

#[tauri::command]
pub fn tool_ig_snapshot_diff(from: String, to: String) -> follow::SnapshotDiff {
    follow::snapshot_diff(&follow::snapshot_load(&from), &follow::snapshot_load(&to))
}

#[tauri::command]
pub fn tool_ig_snapshot_delete(file: String) -> Result<(), String> {
    let root = ig::data_dir().join("snapshots");
    let p = std::path::Path::new(&file);
    if !p.starts_with(&root) {
        return Err("caminho fora da pasta de snapshots".into());
    }
    std::fs::remove_file(p).map_err(err)
}

#[tauri::command]
pub async fn tool_ig_ghosts(app: tauri::AppHandle, slug: Option<String>, posts_limit: usize, comment_pages: usize, job: String) -> Result<analytics::GhostReport, String> {
    let client = load_client(slug.as_deref())?;
    let j = Job::new(&job);
    let p = progress(&app);
    let me = profile::whoami(&client).await.map_err(err)?;
    let followers = follow::followers(&client, &me.pk, 0, &j.flag, &p, &job).await.map_err(err)?;
    let posts = profile::user_posts(&client, &me.username, posts_limit.max(1), &j.flag, &p, &job).await.map_err(err)?;
    analytics::ghosts(&client, followers, &posts, comment_pages, &j.flag, &p, &job).await.map_err(err)
}

// ── Export oficial ───────────────────────────────────────────────────────

#[tauri::command]
pub async fn tool_ig_export(path: String) -> Result<analytics::ExportReport, String> {
    tokio::task::spawn_blocking(move || analytics::analyze_export(&path)).await.map_err(err)?.map_err(err)
}

#[tauri::command]
pub async fn tool_ig_read_text(path: String) -> Result<String, String> {
    tokio::fs::read_to_string(&path).await.map_err(err)
}

/// Grava um CSV (linhas já montadas pela UI).
#[tauri::command]
pub fn tool_ig_write_csv(path: String, rows: Vec<Vec<String>>) -> Result<String, String> {
    let mut out = String::from("\u{feff}");
    for row in rows {
        let line: Vec<String> = row.iter().map(|c| if c.contains([',', '"', '\n']) { format!("\"{}\"", c.replace('"', "\"\"")) } else { c.clone() }).collect();
        out.push_str(&line.join(","));
        out.push('\n');
    }
    std::fs::write(&path, out).map_err(err)?;
    Ok(path)
}

// ── Analytics ────────────────────────────────────────────────────────────

#[tauri::command]
pub async fn tool_ig_analytics(app: tauri::AppHandle, slug: Option<String>, users: Vec<String>, posts_limit: usize, job: String) -> Result<Vec<analytics::ProfileStats>, String> {
    let client = load_client(slug.as_deref())?;
    let j = Job::new(&job);
    let p = progress(&app);
    let mut out = Vec::new();
    for u in users.iter().filter(|u| !u.trim().is_empty()) {
        if ig::cancelled(&j.flag) {
            break;
        }
        let info = profile::resolve_user(&client, u).await.map_err(err)?;
        let posts = match profile::user_posts(&client, &info.username, posts_limit.max(1), &j.flag, &p, &job).await {
            Ok(v) => v,
            Err(ig::IgError::Private) => Vec::new(),
            Err(e) => return Err(e.to_string()),
        };
        out.push(analytics::compute(info, &posts));
        client.pause().await;
    }
    Ok(out)
}

#[derive(Debug, Serialize)]
pub struct HashtagResult {
    pub info: social::TagInfo,
    pub items: Vec<media::MediaItem>,
}

#[tauri::command]
pub async fn tool_ig_hashtag(app: tauri::AppHandle, slug: Option<String>, tag: String, tab: String, limit: usize, job: String) -> Result<HashtagResult, String> {
    let client = load_client(slug.as_deref())?;
    let j = Job::new(&job);
    let info = social::tag_info(&client, &tag).await.map_err(err)?;
    let items = social::tag_media(&client, &tag, &tab, limit, &j.flag, &progress(&app), &job).await.unwrap_or_default();
    Ok(HashtagResult { info, items })
}

#[derive(Debug, Serialize)]
pub struct CommentsResult {
    pub item: media::MediaItem,
    pub comments: Vec<social::Comment>,
}

#[tauri::command]
pub async fn tool_ig_comments(app: tauri::AppHandle, slug: Option<String>, url: String, limit: usize, job: String) -> Result<CommentsResult, String> {
    let client = load_client(slug.as_deref())?;
    let j = Job::new(&job);
    let ig::IgTarget::Post { shortcode } = ig::parse_target(&url) else { return Err("cole o link de um post ou reel".into()) };
    let item = media::post_info(&client, &shortcode).await.map_err(err)?;
    let comments = social::comments(&client, &item.pk, limit, &j.flag, &progress(&app), &job).await.map_err(err)?;
    Ok(CommentsResult { item, comments })
}

#[derive(Debug, Serialize)]
pub struct LikersResult {
    pub item: media::MediaItem,
    pub count: u64,
    pub users: Vec<follow::MiniUser>,
}

#[tauri::command]
pub async fn tool_ig_likers(slug: Option<String>, url: String) -> Result<LikersResult, String> {
    let client = load_client(slug.as_deref())?;
    let ig::IgTarget::Post { shortcode } = ig::parse_target(&url) else { return Err("cole o link de um post ou reel".into()) };
    let item = media::post_info(&client, &shortcode).await.map_err(err)?;
    let (count, users) = social::likers(&client, &item.pk).await.map_err(err)?;
    Ok(LikersResult { item, count, users })
}

#[tauri::command]
pub fn tool_ig_giveaway(comments: Vec<social::Comment>, rules: social::GiveawayRules) -> social::GiveawayResult {
    social::giveaway(&comments, &rules)
}

// ── Publicar e agendar ───────────────────────────────────────────────────

#[tauri::command]
pub async fn tool_ig_publish(app: tauri::AppHandle, slug: Option<String>, request: publish::PublishRequest, job: String) -> Result<publish::PublishResult, String> {
    let client = load_client(slug.as_deref())?;
    let _j = Job::new(&job);
    publish::publish_web(&client, &request, &progress(&app), &job).await.map_err(err)
}

#[tauri::command]
pub async fn tool_ig_publish_graph(app: tauri::AppHandle, auth: publish::GraphAuth, request: publish::PublishRequest, job: String) -> Result<publish::PublishResult, String> {
    let _j = Job::new(&job);
    publish::publish_graph(&auth, &request, &progress(&app), &job).await.map_err(err)
}

#[tauri::command]
pub fn tool_ig_schedule_list(app: tauri::AppHandle) -> publish::ScheduleStore {
    start_scheduler(app);
    publish::schedule_list()
}

#[tauri::command]
pub fn tool_ig_schedule_add(app: tauri::AppHandle, post: publish::ScheduledPost) -> Result<publish::ScheduleStore, String> {
    start_scheduler(app);
    publish::schedule_add(post).map_err(err)
}

#[tauri::command]
pub fn tool_ig_schedule_remove(id: String) -> Result<publish::ScheduleStore, String> {
    publish::schedule_remove(&id).map_err(err)
}

static SCHEDULER: OnceLock<()> = OnceLock::new();

/// Laço que publica os agendamentos vencidos enquanto o app está aberto.
pub fn start_scheduler(app: tauri::AppHandle) {
    if SCHEDULER.set(()).is_err() {
        return;
    }
    tauri::async_runtime::spawn(async move {
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(30)).await;
            let now = chrono::Utc::now().timestamp();
            let Some(mut post) = publish::schedule_due(now) else { continue };
            post.status = "running".into();
            let _ = publish::schedule_update(&post);
            let job = format!("schedule:{}", post.id);
            let p = progress(&app);
            let outcome = if post.mode == "graph" {
                match &post.graph {
                    Some(auth) => publish::publish_graph(auth, &post.request, &p, &job).await,
                    None => Err(anyhow::anyhow!("agendamento sem token da API")),
                }
            } else {
                match load_client(post.account_slug.as_deref()) {
                    Ok(client) => publish::publish_web(&client, &post.request, &p, &job).await,
                    Err(e) => Err(anyhow::anyhow!(e)),
                }
            };
            match outcome {
                Ok(r) => {
                    post.status = "done".into();
                    post.result = Some(r);
                }
                Err(e) => {
                    post.status = "failed".into();
                    post.error = Some(e.to_string());
                }
            }
            let _ = publish::schedule_update(&post);
            use tauri::Emitter;
            let _ = app.emit("ig-schedule-changed", &post.id);
        }
    });
}


//! Auto-download for followed channels: rules decide what enters the app's
//! download queue, a disk cap is checked before enqueueing, and retention
//! trims what the auto-downloader itself fetched (never anything else).
//!
//! Scheduling follows ytzero's adaptive refresh (median gap between uploads
//! / 24, clamped) with jitter added, plus a per-round request budget.

use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::time::Duration;

use omniget_core::core::tube::{AutoRule, Format, FormatKind, Item, StorageStats, VideoItem};
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_notification::NotificationExt;
use unicode_normalization::UnicodeNormalization;

use super::db::{self, FeedRow, SubRow};
use super::download::{self, TubeDownloadReq};
use super::local;

const MIN: i64 = 60_000;
const HOUR: i64 = 60 * MIN;
const DAY: i64 = 24 * HOUR;
const GIB: u64 = 1024 * 1024 * 1024;

const INTERVAL_MIN_MS: i64 = 30 * MIN;
const INTERVAL_MAX_MS: i64 = 12 * HOUR;
const INTERVAL_DEFAULT_MS: i64 = 2 * HOUR;

const TICK: Duration = Duration::from_secs(60);
const ROUND_EVERY_MS: i64 = 5 * MIN;
const RETENTION_EVERY_MS: i64 = 10 * MIN;
/// Channels that may hit the network in one round.
const ROUND_BUDGET: usize = 40;
/// Candidates resolved (size estimate, Shorts check) per round.
const ENRICH_BUDGET: usize = 12;
const ENQUEUE_BUDGET: usize = 8;
/// Free space kept on the volume after the estimated download.
const FREE_MARGIN: u64 = 2 * GIB;
/// A queued row whose download vanished from the queue and its history.
const QUEUED_LOST_MS: i64 = 6 * HOUR;

static ROUND: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

// ---------- scheduling (pure) ----------

/// Next check interval from the channel's upload dates (any order).
pub fn adaptive_interval_ms(published: &[i64], now: i64) -> i64 {
    let mut dates: Vec<i64> = published.to_vec();
    dates.sort_unstable();
    let base = if dates.len() < 3 {
        INTERVAL_DEFAULT_MS
    } else {
        let mut gaps: Vec<i64> = dates.windows(2).map(|w| w[1] - w[0]).collect();
        gaps.sort_unstable();
        let median = gaps[gaps.len() / 2];
        (median / 24).clamp(INTERVAL_MIN_MS, INTERVAL_MAX_MS)
    };
    // A quiet channel is not worth checking every half hour.
    let floor = match dates.last() {
        Some(last) => match now - last {
            d if d <= 2 * DAY => 0,
            d if d <= 7 * DAY => 3 * HOUR,
            d if d <= 30 * DAY => 6 * HOUR,
            _ => INTERVAL_MAX_MS,
        },
        None => 0,
    };
    base.max(floor).min(INTERVAL_MAX_MS)
}

/// ±10 %, stable for a given channel within a minute, so channels added
/// together drift apart instead of firing in lockstep.
pub fn jitter(ms: i64, seed: &str, now: i64) -> i64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    seed.hash(&mut h);
    (now / MIN).hash(&mut h);
    let r = (h.finish() % 2001) as i64 - 1000;
    ms + ms * r / 10_000
}

/// 15 min, 30 min, 1 h... up to a day.
pub fn backoff_ms(fails: u32) -> i64 {
    let exp = fails.saturating_sub(1).min(10);
    (15 * MIN * (1i64 << exp)).min(DAY)
}

// ---------- rules (pure) ----------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    Match,
    Reject,
    /// Would match if an unknown field (duration, Shorts, date) turns out right.
    NeedsInfo,
}

pub fn normalize(s: &str) -> String {
    s.nfd()
        .filter(|c| !unicode_normalization::char::is_combining_mark(*c))
        .collect::<String>()
        .to_lowercase()
}

fn has_word(haystack: &str, words: &[String]) -> bool {
    words
        .iter()
        .map(|w| normalize(w.trim()))
        .filter(|w| !w.is_empty())
        .any(|w| haystack.contains(&w))
}

/// One rule against one feed row. `sub` is the followed channel (None when
/// the row belongs to a channel no longer followed). Without `max_age_days`
/// only videos published after the channel was followed count, so turning a
/// rule on never pulls a channel's back catalog.
pub fn evaluate(
    rule: &AutoRule,
    sub: Option<&SubRow>,
    row: &FeedRow,
    now: i64,
    strict: bool,
) -> Verdict {
    let Some(sub) = sub else {
        return Verdict::Reject;
    };
    if row.state != "seen" {
        return Verdict::Reject;
    }
    let source_ok = if rule.channel_ids.is_empty() {
        sub.auto_download
    } else {
        rule.channel_ids.iter().any(|c| c == &row.channel_id)
    };
    if !source_ok {
        return Verdict::Reject;
    }
    if row.is_live && !rule.include_live {
        return Verdict::Reject;
    }
    let title = normalize(&row.title);
    if has_word(&title, &rule.exclude_words) {
        return Verdict::Reject;
    }
    if rule.include_words.iter().any(|w| !w.trim().is_empty())
        && !has_word(&title, &rule.include_words)
    {
        return Verdict::Reject;
    }

    let mut unknown = false;
    let cutoff = match rule.max_age_days.filter(|d| *d > 0) {
        Some(days) => now - days as i64 * DAY,
        None => sub.added_ms,
    };
    match row.published_ms {
        Some(p) if p < cutoff => return Verdict::Reject,
        Some(_) => {}
        None => unknown = true,
    }
    if !rule.include_shorts {
        match row.is_short {
            Some(true) => return Verdict::Reject,
            Some(false) => {}
            None => unknown = true,
        }
    }
    let min = rule.min_duration_s.filter(|m| *m > 0);
    let max = rule.max_duration_s.filter(|m| *m > 0);
    if min.is_some() || max.is_some() {
        match row.duration_s {
            Some(d) => {
                if min.is_some_and(|m| d < m) || max.is_some_and(|m| d > m) {
                    return Verdict::Reject;
                }
            }
            None => unknown = true,
        }
    }
    match (unknown, strict) {
        (false, _) => Verdict::Match,
        (true, true) => Verdict::Reject,
        (true, false) => Verdict::NeedsInfo,
    }
}

// ---------- sizes and caps (pure) ----------

fn quality_height(quality: &str) -> Option<u32> {
    quality.trim().trim_end_matches('p').parse().ok()
}

fn format_bytes(f: &Format, duration_s: u32) -> u64 {
    f.content_length
        .unwrap_or(f.bitrate.saturating_mul(duration_s as u64) / 8)
}

/// Size the rule's quality would produce, from the resolved formats.
pub fn estimate_bytes(formats: &[Format], duration_s: u32, quality: &str) -> Option<u64> {
    let audio = formats
        .iter()
        .filter(|f| f.kind == FormatKind::Audio)
        .max_by_key(|f| f.bitrate)
        .map(|f| format_bytes(f, duration_s));
    if quality == "audio" {
        return audio;
    }
    let cap = quality_height(quality).unwrap_or(u32::MAX);
    let video = formats
        .iter()
        .filter(|f| f.kind != FormatKind::Audio && f.height.unwrap_or(0) <= cap)
        .max_by_key(|f| (f.height.unwrap_or(0), f.bitrate))?;
    let v = format_bytes(video, duration_s);
    Some(if video.kind == FormatKind::Muxed {
        v
    } else {
        v + audio.unwrap_or(0)
    })
}

/// Rough size when nothing was resolved: typical bitrates per quality.
pub fn fallback_bytes(duration_s: Option<u32>, quality: &str) -> u64 {
    let bps: u64 = match quality {
        "audio" => 160_000,
        q => match quality_height(q) {
            Some(h) if h <= 480 => 1_200_000,
            Some(h) if h <= 720 => 2_500_000,
            Some(h) if h <= 1080 => 5_000_000,
            _ => 12_000_000,
        },
    };
    match duration_s {
        Some(d) => bps * d as u64 / 8,
        None => GIB / 2,
    }
}

/// Retention evicts unpinned files oldest first, so a new download fits a
/// cap as long as the pinned files plus what is in flight leave room.
pub fn fits_cap(pinned: u64, in_flight: u64, est: u64, cap: u64) -> bool {
    cap == 0 || pinned.saturating_add(in_flight).saturating_add(est) <= cap
}

fn gb(v: f64) -> u64 {
    if v > 0.0 {
        (v * GIB as f64) as u64
    } else {
        0
    }
}

/// Which auto-downloaded files to delete, oldest download first. Per rule:
/// `keep_days`, `keep_per_channel` (newest by publish date), `max_gb`; then
/// the global cap over everything left. Pinned rows are never picked.
pub fn retention_plan(
    owned: &[FeedRow],
    rules: &[AutoRule],
    global_cap: u64,
    now: i64,
) -> Vec<(String, String)> {
    let mut doomed: HashSet<usize> = HashSet::new();
    let downloaded = |i: &usize| owned[*i].downloaded_ms.unwrap_or(0);

    for rule in rules {
        let mine: Vec<usize> = (0..owned.len())
            .filter(|i| owned[*i].rule_id.as_deref() == Some(rule.id.as_str()) && !owned[*i].pinned)
            .collect();
        if rule.keep_days > 0 {
            let limit = now - rule.keep_days as i64 * DAY;
            doomed.extend(mine.iter().filter(|i| downloaded(i) < limit));
        }
        if rule.keep_per_channel > 0 {
            let mut by_channel: HashMap<&str, Vec<usize>> = HashMap::new();
            for i in &mine {
                by_channel
                    .entry(owned[*i].channel_id.as_str())
                    .or_default()
                    .push(*i);
            }
            for (_, mut list) in by_channel {
                list.sort_by_key(|i| {
                    std::cmp::Reverse(owned[*i].published_ms.unwrap_or(downloaded(i)))
                });
                doomed.extend(list.into_iter().skip(rule.keep_per_channel as usize));
            }
        }
        let cap = gb(rule.max_gb);
        if cap > 0 {
            let mut left: Vec<usize> = mine
                .iter()
                .copied()
                .filter(|i| !doomed.contains(i))
                .collect();
            let pinned: u64 = owned
                .iter()
                .filter(|r| r.pinned && r.rule_id.as_deref() == Some(rule.id.as_str()))
                .map(|r| r.bytes.unwrap_or(0))
                .sum();
            let mut total = pinned
                + left
                    .iter()
                    .map(|i| owned[*i].bytes.unwrap_or(0))
                    .sum::<u64>();
            left.sort_by_key(downloaded);
            for i in left {
                if total <= cap {
                    break;
                }
                total = total.saturating_sub(owned[i].bytes.unwrap_or(0));
                doomed.insert(i);
            }
        }
    }

    if global_cap > 0 {
        let mut total: u64 = (0..owned.len())
            .filter(|i| !doomed.contains(i))
            .map(|i| owned[i].bytes.unwrap_or(0))
            .sum();
        let mut left: Vec<usize> = (0..owned.len())
            .filter(|i| !doomed.contains(i) && !owned[*i].pinned)
            .collect();
        left.sort_by_key(downloaded);
        for i in left {
            if total <= global_cap {
                break;
            }
            total = total.saturating_sub(owned[i].bytes.unwrap_or(0));
            doomed.insert(i);
        }
    }

    let mut out: Vec<usize> = doomed.into_iter().collect();
    out.sort_by_key(downloaded);
    out.into_iter()
        .map(|i| (owned[i].channel_id.clone(), owned[i].video_id.clone()))
        .collect()
}

// ---------- rule commands ----------

fn clean_rule(mut rule: AutoRule) -> AutoRule {
    if rule.id.trim().is_empty() {
        rule.id = format!("r{}", db::now_ms());
    }
    rule.name = rule.name.trim().chars().take(120).collect();
    let tidy = |v: &mut Vec<String>| {
        v.iter_mut().for_each(|w| *w = w.trim().to_string());
        v.retain(|w| !w.is_empty());
        v.truncate(50);
    };
    tidy(&mut rule.include_words);
    tidy(&mut rule.exclude_words);
    tidy(&mut rule.channel_ids);
    if !matches!(rule.quality.as_str(), "best" | "audio") && quality_height(&rule.quality).is_none()
    {
        rule.quality = "best".into();
    }
    if !matches!(rule.sponsorblock.as_str(), "off" | "mark" | "remove") {
        rule.sponsorblock = "off".into();
    }
    if !rule.max_gb.is_finite() || rule.max_gb < 0.0 {
        rule.max_gb = 0.0;
    }
    rule
}

#[tauri::command]
pub async fn tube_rules_list() -> Result<Vec<AutoRule>, String> {
    db::with(db::rules_all)
}

#[tauri::command]
pub async fn tube_rules_save(rule: AutoRule) -> Result<AutoRule, String> {
    let rule = clean_rule(rule);
    db::with(|c| db::rule_put(c, &rule))?;
    Ok(rule)
}

#[tauri::command]
pub async fn tube_rules_remove(id: String) -> Result<(), String> {
    db::with(|c| db::rule_remove(c, &id))
}

fn row_item(row: &FeedRow, sub: Option<&SubRow>) -> Item {
    Item::Video(VideoItem {
        id: row.video_id.clone(),
        title: row.title.clone(),
        channel: Some(omniget_core::core::tube::ChannelRef {
            id: row.channel_id.clone(),
            name: sub.map(|s| s.name.clone()).unwrap_or_default(),
            avatar: sub.and_then(|s| s.avatar.clone()),
            ..Default::default()
        }),
        thumbs: row
            .thumb
            .clone()
            .map(|url| {
                vec![omniget_core::core::tube::Thumb {
                    url,
                    width: 0,
                    height: 0,
                }]
            })
            .unwrap_or_default(),
        duration_s: row.duration_s,
        views: row.views,
        published_ms: row.published_ms,
        ..Default::default()
    })
}

/// What the rule would pick from the cached feeds right now (ignores
/// `enabled`, so a rule can be tested before it is turned on). Rows whose
/// duration or Shorts status is still unknown are included.
#[tauri::command]
pub async fn tube_rules_preview(rule: AutoRule) -> Result<Vec<Item>, String> {
    let rule = clean_rule(rule);
    let now = db::now_ms();
    let (subs, rows) = db::with(|c| Ok((db::subs_all(c)?, db::feed_recent(c, 2000)?)))?;
    let subs: HashMap<&str, &SubRow> = subs.iter().map(|s| (s.channel_id.as_str(), s)).collect();
    Ok(rows
        .iter()
        .filter(|r| {
            evaluate(
                &rule,
                subs.get(r.channel_id.as_str()).copied(),
                r,
                now,
                false,
            ) != Verdict::Reject
        })
        .take(100)
        .map(|r| row_item(r, subs.get(r.channel_id.as_str()).copied()))
        .collect())
}

// ---------- storage ----------

/// Nearest existing ancestor, so free space works before the folder exists.
fn existing_dir(path: &std::path::Path) -> Option<std::path::PathBuf> {
    path.ancestors()
        .find(|p| p.is_dir())
        .map(|p| p.to_path_buf())
}

fn free_space(folder: &str) -> Option<u64> {
    existing_dir(std::path::Path::new(folder))
        .and_then(|d| crate::core::preflight::available_space(&d))
}

#[tauri::command]
pub async fn tube_storage_stats(app: AppHandle) -> Result<StorageStats, String> {
    let owned = db::with(db::feed_owned)?;
    let cap = gb(crate::storage::config::load_settings(&app)
        .tube
        .storage_cap_gb);
    Ok(StorageStats {
        total_bytes: owned.iter().map(|r| r.bytes.unwrap_or(0)).sum(),
        files: owned.len() as u32,
        free_bytes: free_space(&download::default_folder(&app)),
        cap_bytes: (cap > 0).then_some(cap),
    })
}

// ---------- poller ----------

#[tauri::command]
pub async fn tube_auto_check_now(app: AppHandle) -> Result<u32, String> {
    if local::paused_until().is_some() {
        return Err("Paused after the service refused requests; try again later".into());
    }
    run_round(&app, true).await
}

pub fn spawn_poller(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        // Let startup work (queue recovery, history hydrate) settle first.
        tokio::time::sleep(Duration::from_secs(45)).await;
        let mut last_round = 0i64;
        let mut last_retention = 0i64;
        loop {
            reconcile(&app).await;
            let now = db::now_ms();
            let enabled = crate::storage::config::load_settings(&app).tube.enabled;
            if enabled && now - last_round >= ROUND_EVERY_MS {
                last_round = now;
                if local::paused_until().is_none() {
                    if let Err(e) = run_round(&app, false).await {
                        tracing::warn!("[tube] auto-download round failed: {e}");
                    }
                }
            }
            if now - last_retention >= RETENTION_EVERY_MS {
                last_retention = now;
                retention(&app).await;
            }
            tokio::time::sleep(TICK).await;
        }
    });
}

fn notify_new(app: &AppHandle, sub: &SubRow, name: Option<&str>, new: &[VideoItem]) {
    let Some(first) = new.first() else { return };
    let body = if new.len() > 1 {
        format!("{} (+{})", first.title, new.len() - 1)
    } else {
        first.title.clone()
    };
    let _ = app
        .notification()
        .builder()
        .title(name.unwrap_or(&sub.name))
        .body(body)
        .show();
}

#[derive(Clone, serde::Serialize)]
struct NewVideosEvent {
    channel_id: String,
    video_ids: Vec<String>,
}

/// Checks due channels, then enqueues what the enabled rules pick. Returns
/// how many downloads entered the queue.
async fn run_round(app: &AppHandle, force: bool) -> Result<u32, String> {
    let _guard = ROUND.lock().await;
    let now = db::now_ms();
    let (subs, rules) = db::with(|c| Ok((db::subs_all(c)?, db::rules_all(c)?)))?;
    let rules: Vec<AutoRule> = rules.into_iter().filter(|r| r.enabled).collect();
    let ruled: HashSet<&str> = rules
        .iter()
        .flat_map(|r| r.channel_ids.iter().map(String::as_str))
        .collect();

    let mut due: Vec<&SubRow> = subs
        .iter()
        .filter(|s| s.auto_download || s.notify || ruled.contains(s.channel_id.as_str()))
        .filter(|s| force || s.next_check_ms <= now)
        .collect();
    due.sort_by_key(|s| s.next_check_ms);
    due.truncate(ROUND_BUDGET);

    for (i, sub) in due.iter().enumerate() {
        if i > 0 {
            let pause = 1_000 + (jitter(1_000, &sub.channel_id, now) - 900) * 4;
            tokio::time::sleep(Duration::from_millis(pause as u64)).await;
        }
        match local::check_channel(sub).await {
            Ok(checked) => {
                if !checked.new.is_empty() {
                    let _ = app.emit(
                        "tube-new-videos",
                        NewVideosEvent {
                            channel_id: sub.channel_id.clone(),
                            video_ids: checked.new.iter().map(|v| v.id.clone()).collect(),
                        },
                    );
                    // The first fetch of a channel is its back catalog, not news.
                    if sub.notify && sub.last_checked_ms.is_some() {
                        notify_new(app, sub, checked.name.as_deref(), &checked.new);
                    }
                }
            }
            Err(e) if local::is_refusal(&e) => break,
            Err(_) => {}
        }
    }

    if rules.is_empty() || local::paused_until().is_some() {
        return Ok(0);
    }
    enqueue_matches(app, &rules).await
}

struct Pick {
    row: FeedRow,
    rule: AutoRule,
}

async fn enqueue_matches(app: &AppHandle, rules: &[AutoRule]) -> Result<u32, String> {
    let now = db::now_ms();
    let (subs, seen, owned, queued) = db::with(|c| {
        Ok((
            db::subs_all(c)?,
            db::feed_by_state(c, "seen")?,
            db::feed_owned(c)?,
            db::feed_by_state(c, "queued")?,
        ))
    })?;
    let subs_by: HashMap<&str, &SubRow> = subs.iter().map(|s| (s.channel_id.as_str(), s)).collect();

    let mut picks: Vec<Pick> = Vec::new();
    let mut taken: HashSet<String> = HashSet::new();
    let mut rows = seen;
    rows.sort_by_key(|r| std::cmp::Reverse(r.published_ms.unwrap_or(0)));
    for row in rows {
        if taken.contains(&row.video_id) {
            continue;
        }
        let sub = subs_by.get(row.channel_id.as_str()).copied();
        if let Some(rule) = rules
            .iter()
            .find(|r| evaluate(r, sub, &row, now, false) != Verdict::Reject)
        {
            taken.insert(row.video_id.clone());
            picks.push(Pick {
                row,
                rule: rule.clone(),
            });
        }
    }
    if picks.is_empty() {
        return Ok(0);
    }

    let settings = crate::storage::config::load_settings(app);
    let global_cap = gb(settings.tube.storage_cap_gb);
    let mut in_flight_all: u64 = queued.iter().map(|r| r.bytes.unwrap_or(0)).sum();
    let pinned_all: u64 = owned
        .iter()
        .filter(|r| r.pinned)
        .map(|r| r.bytes.unwrap_or(0))
        .sum();
    let mut in_flight_rule: HashMap<String, u64> = HashMap::new();
    for r in &queued {
        if let Some(id) = &r.rule_id {
            *in_flight_rule.entry(id.clone()).or_default() += r.bytes.unwrap_or(0);
        }
    }

    let mut enqueued = 0u32;
    for (enriched, Pick { mut row, rule }) in picks.into_iter().enumerate() {
        if enqueued as usize >= ENQUEUE_BUDGET || enriched >= ENRICH_BUDGET {
            break;
        }
        let sub = subs_by.get(row.channel_id.as_str()).copied();

        // Resolve once: duration for the rule, formats for the size estimate.
        let mut est = None;
        match super::streams::resolve_for_download(&row.video_id, false).await {
            Ok(set) => {
                if set.duration_s > 0 {
                    row.duration_s = Some(set.duration_s);
                }
                row.is_live |= set.is_live;
                est = estimate_bytes(&set.formats, set.duration_s, &rule.quality);
            }
            Err(e) => {
                if local::note_error(&e) {
                    break;
                }
                tracing::debug!("[tube] resolve for auto-download failed: {e}");
            }
        }
        if row.is_short.is_none() && !rule.include_shorts {
            row.is_short = match local::short_from_duration(row.duration_s) {
                Some(s) => Some(s),
                None => local::classify_short(&row.video_id).await,
            };
        }
        let _ = db::with(|c| {
            db::feed_set_meta(
                c,
                &row.channel_id,
                &row.video_id,
                row.duration_s,
                row.is_short,
            )
        });
        if evaluate(&rule, sub, &row, now, true) != Verdict::Match {
            // Stored metadata makes the next round reject it without a
            // request; an unknown Shorts status is retried later.
            continue;
        }

        let est = est.unwrap_or_else(|| fallback_bytes(row.duration_s, &rule.quality));
        let base = rule
            .folder
            .clone()
            .filter(|f| !f.trim().is_empty())
            .unwrap_or_else(|| download::default_folder(app));
        if free_space(&base).is_some_and(|free| free < est + FREE_MARGIN) {
            tracing::warn!("[tube] auto-download stopped: not enough free space");
            break;
        }
        let pinned_rule: u64 = owned
            .iter()
            .filter(|r| r.pinned && r.rule_id.as_deref() == Some(rule.id.as_str()))
            .map(|r| r.bytes.unwrap_or(0))
            .sum();
        let flight_rule = in_flight_rule.get(&rule.id).copied().unwrap_or(0);
        if !fits_cap(pinned_rule, flight_rule, est, gb(rule.max_gb))
            || !fits_cap(pinned_all, in_flight_all, est, global_cap)
        {
            tracing::info!("[tube] auto-download skipped a video: storage cap reached");
            continue;
        }

        let channel_dir = sub
            .map(|s| sanitize_filename::sanitize(&s.name))
            .filter(|n| !n.trim().is_empty())
            .unwrap_or_else(|| row.channel_id.clone());
        let req = TubeDownloadReq {
            video_id: row.video_id.clone(),
            audio_only: rule.quality == "audio",
            height: quality_height(&rule.quality),
            sponsorblock: Some(rule.sponsorblock.clone()).filter(|s| !s.is_empty()),
            folder: Some(
                std::path::Path::new(&base)
                    .join(channel_dir)
                    .to_string_lossy()
                    .to_string(),
            ),
            channel_id: Some(row.channel_id.clone()),
            ..Default::default()
        };
        let plan = download::plan_for(app, &req);
        match download::enqueue(app, plan).await {
            Ok(dl_id) => {
                let _ = db::with(|c| {
                    db::feed_mark_queued(c, &row.channel_id, &row.video_id, &rule.id, dl_id, now)?;
                    c.execute(
                        "UPDATE feed SET bytes = ?3 WHERE channel_id = ?1 AND video_id = ?2",
                        rusqlite::params![row.channel_id, row.video_id, est as i64],
                    )?;
                    Ok(())
                });
                in_flight_all += est;
                *in_flight_rule.entry(rule.id.clone()).or_default() += est;
                enqueued += 1;
            }
            Err(e) => tracing::warn!("[tube] auto-download enqueue failed: {e}"),
        }
    }
    if enqueued > 0 {
        let _ = app.emit("tube-autodl-queued", enqueued);
    }
    Ok(enqueued)
}

/// Moves queued rows to downloaded/skipped once the queue settles them.
async fn reconcile(app: &AppHandle) {
    let Ok(queued) = db::with(|c| db::feed_by_state(c, "queued")) else {
        return;
    };
    if queued.is_empty() {
        return;
    }
    use crate::core::queue::QueueStatus;
    let live: HashMap<u64, (QueueStatus, Option<String>, Option<u64>)> = {
        let q = app.state::<crate::AppState>().download_queue.clone();
        let q = q.lock().await;
        q.items
            .iter()
            .map(|i| {
                (
                    i.id,
                    (i.status.clone(), i.file_path.clone(), i.file_size_bytes),
                )
            })
            .collect()
    };
    let history: HashMap<u64, crate::core::queue_history::HistoryEntry> =
        crate::core::queue_history::list()
            .into_iter()
            .map(|h| (h.id, h))
            .collect();
    let now = db::now_ms();

    for row in queued {
        let outcome: Option<Result<(String, Option<u64>), ()>> = match row.dl_id {
            Some(id) => match live.get(&id) {
                Some((QueueStatus::Complete { success: true }, Some(path), size)) => {
                    Some(Ok((path.clone(), *size)))
                }
                Some((QueueStatus::Complete { .. }, _, _))
                | Some((
                    QueueStatus::Error {
                        retryable: false, ..
                    },
                    _,
                    _,
                )) => Some(Err(())),
                Some(_) => None,
                None => match history.get(&id) {
                    Some(h) if h.success => h
                        .file_path
                        .clone()
                        .map(|p| Ok((p, h.file_size_bytes)))
                        .or(Some(Err(()))),
                    Some(_) => Some(Err(())),
                    None if now - row.queued_ms.unwrap_or(0) > QUEUED_LOST_MS => Some(Err(())),
                    None => None,
                },
            },
            None => Some(Err(())),
        };
        let res = match outcome {
            None => continue,
            Some(Ok((path, size))) => {
                let bytes = size
                    .filter(|s| *s > 0)
                    .or_else(|| std::fs::metadata(&path).ok().map(|m| m.len()))
                    .unwrap_or(0);
                db::with(|c| {
                    db::feed_mark_downloaded(c, &row.channel_id, &row.video_id, &path, bytes, now)
                })
            }
            // Failed or removed from the queue by the user: don't retry forever.
            Some(Err(())) => {
                db::with(|c| db::feed_set_state(c, &row.channel_id, &row.video_id, "skipped"))
            }
        };
        if let Err(e) = res {
            tracing::warn!("[tube] reconcile failed: {e}");
        }
    }
}

fn discard(path: &std::path::Path) -> Result<(), String> {
    omniget_core::core::washmybelly::fsops::to_trash(path).or_else(|e| {
        tracing::debug!("[tube] trash unavailable, removing: {e}");
        if path.is_dir() {
            std::fs::remove_dir_all(path).map_err(|e| e.to_string())
        } else {
            std::fs::remove_file(path).map_err(|e| e.to_string())
        }
    })
}

/// Deletes (to the system trash) only files recorded in `feed.path` by the
/// auto-downloader, then leaves a `deleted` tombstone so they never return.
async fn retention(app: &AppHandle) {
    let now = db::now_ms();
    let _ = db::with(|c| db::cache_prune(c, now));
    let Ok((owned, rules)) = db::with(|c| Ok((db::feed_owned(c)?, db::rules_all(c)?))) else {
        return;
    };
    if owned.is_empty() {
        return;
    }
    // A file the user moved or deleted is gone for us too.
    let (present, missing): (Vec<FeedRow>, Vec<FeedRow>) = owned.into_iter().partition(|r| {
        r.path
            .as_deref()
            .is_some_and(|p| std::path::Path::new(p).exists())
    });
    for r in &missing {
        let _ = db::with(|c| db::feed_set_state(c, &r.channel_id, &r.video_id, "deleted"));
    }
    let cap = gb(crate::storage::config::load_settings(app)
        .tube
        .storage_cap_gb);
    let plan = retention_plan(&present, &rules, cap, now);
    let by_key: HashMap<(&str, &str), &FeedRow> = present
        .iter()
        .map(|r| ((r.channel_id.as_str(), r.video_id.as_str()), r))
        .collect();
    for (channel_id, video_id) in plan {
        let Some(row) = by_key.get(&(channel_id.as_str(), video_id.as_str())) else {
            continue;
        };
        let Some(path) = row.path.as_deref() else {
            continue;
        };
        let path = std::path::PathBuf::from(path);
        match tokio::task::spawn_blocking(move || discard(&path)).await {
            Ok(Ok(())) => {
                let _ = db::with(|c| db::feed_set_state(c, &channel_id, &video_id, "deleted"));
            }
            Ok(Err(e)) => tracing::warn!("[tube] retention could not remove a file: {e}"),
            Err(e) => tracing::warn!("[tube] retention task failed: {e}"),
        }
    }
}

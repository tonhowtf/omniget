use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::json;

use super::auth::TelegramSessionHandle;
use super::batch;
use super::events;
use super::history::{self, MediaFilter};
use super::template;
use crate::core::telegram::host;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WatchRule {
    pub id: String,
    pub chat_id: i64,
    pub chat_type: String,
    #[serde(default)]
    pub chat_title: String,
    #[serde(default = "yes")]
    pub enabled: bool,
    #[serde(default)]
    pub media_types: Vec<String>,
    #[serde(default)]
    pub min_size: Option<f64>,
    #[serde(default)]
    pub max_size: Option<f64>,
    #[serde(default)]
    pub keyword: Option<String>,
    #[serde(default)]
    pub ext_include: Vec<String>,
    #[serde(default)]
    pub ext_exclude: Vec<String>,
    #[serde(default)]
    pub output_dir: Option<String>,
    #[serde(default)]
    pub template: Option<String>,
    #[serde(default)]
    pub last_message_id: i32,
    #[serde(default)]
    pub created_at: i64,
    #[serde(default)]
    pub last_run_at: Option<i64>,
    #[serde(default)]
    pub saved_count: u64,
    #[serde(default)]
    pub last_error: Option<String>,
    #[serde(default)]
    pub dedupe: Option<bool>,
    #[serde(default)]
    pub add_to_watch: bool,
    #[serde(default)]
    pub duplicate_count: u64,
}

fn yes() -> bool {
    true
}

impl WatchRule {
    fn filter(&self) -> MediaFilter {
        MediaFilter {
            media_types: Some(self.media_types.clone()).filter(|t| !t.is_empty()),
            min_size: self.min_size,
            max_size: self.max_size,
            keyword: self.keyword.clone(),
            ext_include: Some(self.ext_include.clone()),
            ext_exclude: Some(self.ext_exclude.clone()),
            ..MediaFilter::default()
        }
    }

    fn dedupe(&self) -> bool {
        self.dedupe.unwrap_or_else(super::dedupe::enabled)
    }

    fn root(&self) -> PathBuf {
        self.output_dir
            .as_deref()
            .filter(|d| !d.trim().is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(super::download_dir)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Store {
    #[serde(default = "default_interval")]
    interval_min: u32,
    #[serde(default)]
    rules: Vec<WatchRule>,
}

fn default_interval() -> u32 {
    5
}

impl Default for Store {
    fn default() -> Self {
        Self {
            interval_min: default_interval(),
            rules: Vec::new(),
        }
    }
}

fn store_path() -> PathBuf {
    host::data_dir().join("watchers.json")
}

fn store() -> &'static Mutex<Store> {
    static STORE: OnceLock<Mutex<Store>> = OnceLock::new();
    STORE.get_or_init(|| {
        let raw = std::fs::read(store_path()).ok();
        let loaded = raw.and_then(|b| {
            serde_json::from_slice::<Store>(&b).ok().or_else(|| {
                serde_json::from_slice::<Vec<WatchRule>>(&b)
                    .ok()
                    .map(|rules| Store {
                        rules,
                        ..Store::default()
                    })
            })
        });
        Mutex::new(loaded.unwrap_or_default())
    })
}

fn persist(s: &Store) {
    let path = store_path();
    let tmp = path.with_extension("json.tmp");
    match serde_json::to_vec_pretty(s) {
        Ok(bytes) => {
            if let Err(e) = std::fs::write(&tmp, bytes).and_then(|_| std::fs::rename(&tmp, &path)) {
                tracing::warn!("[tg-watch] saving watchers.json failed: {}", e);
            }
        }
        Err(e) => tracing::warn!("[tg-watch] serializing watchers failed: {}", e),
    }
}

fn with_store<T>(f: impl FnOnce(&mut Store) -> T) -> T {
    let mut g = store().lock().unwrap_or_else(|p| p.into_inner());
    let out = f(&mut g);
    persist(&g);
    out
}

fn read_store<T>(f: impl FnOnce(&Store) -> T) -> T {
    let g = store().lock().unwrap_or_else(|p| p.into_inner());
    f(&g)
}

pub fn rules_for_new_message(chat_id: i64, message_id: i32) -> Vec<String> {
    read_store(|s| {
        s.rules
            .iter()
            .filter(|r| r.enabled && r.chat_id == chat_id && message_id > r.last_message_id)
            .map(|r| r.id.clone())
            .collect()
    })
}

pub fn list() -> Vec<WatchRule> {
    read_store(|s| s.rules.clone())
}

pub fn interval_min() -> u32 {
    read_store(|s| s.interval_min)
}

pub fn set_interval_min(n: u32) -> u32 {
    with_store(|s| {
        s.interval_min = n.clamp(1, 24 * 60);
        s.interval_min
    })
}

fn get(id: &str) -> Option<WatchRule> {
    read_store(|s| s.rules.iter().find(|r| r.id == id).cloned())
}

fn update_rule(id: &str, f: impl FnOnce(&mut WatchRule)) -> Option<WatchRule> {
    with_store(|s| {
        let r = s.rules.iter_mut().find(|r| r.id == id)?;
        f(r);
        Some(r.clone())
    })
}

fn normalize(rule: &mut WatchRule) {
    rule.media_types
        .retain(|t| history::MEDIA_TYPES.contains(&t.as_str()));
    for list in [&mut rule.ext_include, &mut rule.ext_exclude] {
        *list = list
            .iter()
            .map(|e| e.trim().trim_start_matches('.').to_ascii_lowercase())
            .filter(|e| !e.is_empty())
            .collect();
    }
    if rule
        .template
        .as_deref()
        .is_some_and(|t| t.trim().is_empty())
    {
        rule.template = None;
    }
}

pub async fn add(
    handle: &TelegramSessionHandle,
    active: &super::ActiveDownloads,
    rule: &serde_json::Value,
    backfill: usize,
) -> Result<WatchRule, String> {
    let mut v = history::camelize(rule);
    if let Some(obj) = v.as_object_mut() {
        obj.entry("id")
            .or_insert_with(|| json!(format!("w{}", events::next_id())));
        obj.entry("createdAt")
            .or_insert_with(|| json!(super::now_secs()));
        obj.entry("lastMessageId").or_insert(json!(0));
    }
    let mut rule: WatchRule = serde_json::from_value(v).map_err(|e| format!("rule: {}", e))?;
    normalize(&mut rule);
    if rule.last_message_id <= 0 {
        rule.last_message_id = history::latest_message_id(handle, rule.chat_id, &rule.chat_type)
            .await
            .map_err(|e| e.to_string())?;
    }
    with_store(|s| {
        s.rules.retain(|r| r.id != rule.id);
        s.rules.push(rule.clone());
    });

    if backfill > 0 {
        let mut filter = rule.filter();
        filter.limit = Some(backfill as u64);
        let scan_id = super::index::new_scan_id();
        let (rows, _) = super::index::scan_filtered(
            handle,
            rule.chat_id,
            &rule.chat_type,
            &filter,
            &scan_id,
            5_000,
            false,
            0,
        )
        .await
        .map_err(|e| e.to_string())?;
        let items: Vec<_> = rows.into_iter().map(|(it, _)| it).collect();
        let found = items.len();
        let queued = queue(handle, active, &rule, items).await;
        events::emit(
            "telegram-watch-run",
            json!({ "id": rule.id, "found": found, "queued": queued, "trigger": "backfill" }),
        );
    }
    Ok(rule)
}

pub fn update(id: &str, patch: &serde_json::Value) -> Result<WatchRule, String> {
    let current = get(id).ok_or("Rule not found")?;
    let mut merged = serde_json::to_value(&current).map_err(|e| e.to_string())?;
    if let (Some(obj), Some(p)) = (merged.as_object_mut(), history::camelize(patch).as_object()) {
        for (k, val) in p {
            if k == "id" || k == "createdAt" {
                continue;
            }
            obj.insert(k.clone(), val.clone());
        }
    }
    let mut rule: WatchRule = serde_json::from_value(merged).map_err(|e| format!("rule: {}", e))?;
    normalize(&mut rule);
    update_rule(id, |r| *r = rule.clone()).ok_or_else(|| "Rule not found".into())
}

pub fn remove(id: &str) -> bool {
    with_store(|s| {
        let before = s.rules.len();
        s.rules.retain(|r| r.id != id);
        s.rules.len() != before
    })
}

async fn queue(
    handle: &TelegramSessionHandle,
    active: &super::ActiveDownloads,
    rule: &WatchRule,
    mut items: Vec<super::api::TelegramMediaItem>,
) -> usize {
    if items.is_empty() {
        return 0;
    }
    items.reverse();
    let root = rule.root();
    let planned: Vec<batch::Planned> = items
        .iter()
        .map(|it| {
            batch::plan_item(
                &root,
                rule.template.as_deref(),
                Some(template::CHAT_DEFAULT),
                rule.chat_id,
                &rule.chat_type,
                &rule.chat_title,
                it,
            )
        })
        .collect();
    let queued = planned.iter().filter(|p| !batch::already_there(p)).count();
    let title = if rule.chat_title.is_empty() {
        "Auto-save".to_string()
    } else {
        format!("{} · auto-save", rule.chat_title)
    };
    let (batch_id, join) = batch::start(
        handle.clone(),
        active.clone(),
        planned,
        title,
        root,
        super::concurrent_downloads(),
        true,
        rule.dedupe(),
    )
    .await;
    let id = rule.id.clone();
    let add_to_watch = rule.add_to_watch;
    tokio::spawn(async move {
        if let Ok(outcome) = join.await {
            if outcome.ok > 0 || outcome.duplicates > 0 {
                update_rule(&id, |r| {
                    r.saved_count += outcome.ok as u64;
                    r.duplicate_count += outcome.duplicates as u64;
                });
            }
            events::emit(
                "telegram-watch-saved",
                json!({
                    "id": id,
                    "batch_id": batch_id,
                    "saved": outcome.ok,
                    "duplicates": outcome.duplicates,
                    "failed": outcome.failed,
                    "paths": outcome.paths,
                    "add_to_watch": add_to_watch,
                }),
            );
        }
    });
    queued
}

fn running() -> &'static Mutex<HashSet<String>> {
    static RUNNING: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
    RUNNING.get_or_init(|| Mutex::new(HashSet::new()))
}

const MAX_SCAN_PER_RUN: usize = 2_000;

pub async fn run_rule(
    handle: &TelegramSessionHandle,
    active: &super::ActiveDownloads,
    id: &str,
) -> Result<serde_json::Value, String> {
    run_rule_with(handle, active, id, "manual").await
}

pub async fn run_rule_with(
    handle: &TelegramSessionHandle,
    active: &super::ActiveDownloads,
    id: &str,
    trigger: &str,
) -> Result<serde_json::Value, String> {
    let rule = get(id).ok_or("Rule not found")?;
    {
        let mut r = running().lock().unwrap_or_else(|p| p.into_inner());
        if !r.insert(id.to_string()) {
            return Ok(json!({ "id": id, "found": 0, "queued": 0, "busy": true }));
        }
    }
    let result = run_rule_inner(handle, active, rule).await;
    running()
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .remove(id);
    let now = super::now_secs();
    match result {
        Ok((found, queued, max_id)) => {
            update_rule(id, |r| {
                r.last_run_at = Some(now);
                r.last_error = None;
                if max_id > r.last_message_id {
                    r.last_message_id = max_id;
                }
            });
            let payload = json!({ "id": id, "found": found, "queued": queued, "trigger": trigger });
            events::emit("telegram-watch-run", payload.clone());
            Ok(payload)
        }
        Err(e) => {
            update_rule(id, |r| {
                r.last_run_at = Some(now);
                r.last_error = Some(e.clone());
            });
            events::emit(
                "telegram-watch-run",
                json!({ "id": id, "found": 0, "queued": 0, "error": e, "trigger": trigger }),
            );
            Err(e)
        }
    }
}

async fn run_rule_inner(
    handle: &TelegramSessionHandle,
    active: &super::ActiveDownloads,
    rule: WatchRule,
) -> Result<(usize, usize, i32), String> {
    if rule.last_message_id <= 0 {
        let latest = history::latest_message_id(handle, rule.chat_id, &rule.chat_type)
            .await
            .map_err(|e| e.to_string())?;
        return Ok((0, 0, latest));
    }
    let scan_id = super::index::new_scan_id();
    let cancel = super::index::register_scan(&scan_id);
    let after = rule.last_message_id;
    let opts = super::index::UpdateOpts {
        scan_id: &scan_id,
        max_scan: MAX_SCAN_PER_RUN,
        takeout: false,
        filter: None,
        stop_at_id: after,
        emit: false,
    };
    let res = super::index::update(
        handle,
        rule.chat_id,
        &rule.chat_type,
        &opts,
        &cancel,
        |cov, _| cov.oldest_id == 0 || cov.oldest_id > after,
    )
    .await;
    super::index::unregister_scan(&scan_id);
    let res = res.map_err(|e| e.to_string())?;
    if res.scanned >= MAX_SCAN_PER_RUN {
        tracing::warn!(
            "[tg-watch] rule {}: more than {} new messages since the last run; older ones were skipped",
            rule.id,
            MAX_SCAN_PER_RUN
        );
    }
    let items = super::index::query(rule.chat_id, &rule.filter(), after);
    let max_id = res.coverage.newest_id;
    let found = items.len();
    let queued = queue(handle, active, &rule, items).await;
    Ok((found, queued, max_id))
}

pub async fn run_all(
    handle: &TelegramSessionHandle,
    active: &super::ActiveDownloads,
) -> Vec<serde_json::Value> {
    run_all_with(handle, active, "manual").await
}

pub async fn run_all_with(
    handle: &TelegramSessionHandle,
    active: &super::ActiveDownloads,
    trigger: &str,
) -> Vec<serde_json::Value> {
    let ids: Vec<String> = list()
        .into_iter()
        .filter(|r| r.enabled)
        .map(|r| r.id)
        .collect();
    let mut out = Vec::new();
    for id in ids {
        match run_rule_with(handle, active, &id, trigger).await {
            Ok(v) => out.push(v),
            Err(e) => out.push(json!({ "id": id, "found": 0, "queued": 0, "error": e })),
        }
    }
    out
}

pub fn spawn_background_loop(handle: TelegramSessionHandle, active: super::ActiveDownloads) {
    static STARTED: AtomicBool = AtomicBool::new(false);
    if STARTED.swap(true, Ordering::SeqCst) {
        return;
    }
    tokio::spawn(async move {
        let mut last_pass = std::time::Instant::now()
            .checked_sub(Duration::from_secs(interval_min() as u64 * 60))
            .unwrap_or_else(std::time::Instant::now)
            + Duration::from_secs(60);
        loop {
            tokio::time::sleep(Duration::from_secs(30)).await;
            let every = Duration::from_secs(interval_min().max(1) as u64 * 60);
            if last_pass.elapsed() < every {
                continue;
            }
            if !list().iter().any(|r| r.enabled) {
                continue;
            }
            super::restore_session(&handle).await;
            if handle.lock().await.client.is_none() {
                continue;
            }
            last_pass = std::time::Instant::now();
            let results = Box::pin(run_all_with(&handle, &active, "interval")).await;
            tracing::info!("[tg-watch] pass done: {} rules", results.len());
        }
    });
}

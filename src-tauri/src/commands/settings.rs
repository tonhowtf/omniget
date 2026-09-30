//! Settings commands. The five `settings_*_v3` commands are the contract of
//! `06-api-contract.md`; `get_settings`/`update_settings`/`reset_settings`
//! are transitional wrappers over the same repository. Every one of them
//! answers with the public DTO: no proxy password, manual cookie or bridge
//! token ever reaches the UI through here.

use std::sync::Arc;
use std::time::Duration;

use serde::Serialize;
use serde_json::{Map, Value};

use crate::models::settings::{
    apply_patch, apply_reset, apply_secret_operations, parse_secret_operations, AppSettings,
    FieldError, FieldErrorCode, PatchError, ProxySettings, ResetScope, SettingsPublicV3,
    SettingsSection, WORKSPACE_APPEARANCE_MAX_BYTES, WORKSPACE_APPEARANCE_SOURCE,
};
use crate::storage::config;
use crate::storage::settings_effects::{
    self, EffectId, EffectsRegistry, PendingEffect, RetryOutcome,
};
use crate::storage::settings_repository::{
    self, parse_revision, ApiError, Commit, Gate, ImportOutcome, LoadFailure, LoadStatus,
    RepoError, SettingsRepository, UnavailableCode,
};

// ---------------------------------------------------------------------------
// v3 DTOs
// ---------------------------------------------------------------------------

/// `ReadySnapshot` or `FailedSnapshot` of the contract.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingsSnapshotV3 {
    pub settings: Option<SettingsPublicV3>,
    pub revision: Option<String>,
    pub load_status: LoadStatus,
    pub applied: bool,
    pub pending_effects: Vec<PendingEffect>,
    pub field_errors: Vec<FieldError>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RetryResultV3 {
    pub outcome: RetryOutcome,
    pub snapshot: SettingsSnapshotV3,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ImportOutcomeV3 {
    Imported,
    AlreadyImported,
}

#[derive(Debug, Clone, Serialize)]
pub struct ImportResultV3 {
    pub outcome: ImportOutcomeV3,
    pub snapshot: SettingsSnapshotV3,
}

// ---------------------------------------------------------------------------
// v3 services (no AppHandle: the tests drive them with a temp profile)
// ---------------------------------------------------------------------------

pub fn snapshot_v3(
    repo: &SettingsRepository,
    effects: Option<&Arc<EffectsRegistry>>,
) -> SettingsSnapshotV3 {
    let snap = repo.snapshot();
    match snap.gate {
        Gate::Open => {
            let pending = effects.map(|e| e.pending()).unwrap_or_default();
            SettingsSnapshotV3 {
                settings: Some(snap.settings.to_public()),
                revision: Some(snap.revision.to_string()),
                load_status: snap.status,
                applied: pending.is_empty(),
                pending_effects: pending,
                field_errors: Vec::new(),
            }
        }
        Gate::Failed { code, recovery_id } => SettingsSnapshotV3 {
            settings: None,
            revision: None,
            load_status: LoadStatus::Failed { code, recovery_id },
            applied: false,
            pending_effects: Vec::new(),
            field_errors: Vec::new(),
        },
        Gate::Indeterminate { observed, .. } => SettingsSnapshotV3 {
            settings: None,
            revision: observed.map(|r| r.to_string()),
            load_status: LoadStatus::Failed {
                code: LoadFailure::ReadFailed,
                recovery_id: None,
            },
            applied: false,
            pending_effects: Vec::new(),
            field_errors: Vec::new(),
        },
    }
}

fn schedule(effects: Option<&Arc<EffectsRegistry>>, commit: &Commit) {
    if !commit.changed {
        return;
    }
    if let Some(registry) = effects {
        let ids = settings_effects::effects_for_change(&commit.before, &commit.after);
        registry.schedule(&ids, commit.revision, &commit.before, &commit.after);
    }
}

fn invalid(errors: Vec<FieldError>) -> ApiError {
    ApiError::validation(errors)
}

fn request_object<'a>(
    request: &'a Value,
    allowed: &[&str],
) -> Result<&'a Map<String, Value>, ApiError> {
    let Value::Object(obj) = request else {
        return Err(invalid(vec![FieldError::new(
            "request",
            FieldErrorCode::InvalidType,
        )]));
    };
    let unknown: Vec<_> = obj
        .keys()
        .filter(|k| !allowed.contains(&k.as_str()))
        .map(|k| FieldError::new(k.clone(), FieldErrorCode::UnknownField))
        .collect();
    if unknown.is_empty() {
        Ok(obj)
    } else {
        Err(invalid(unknown))
    }
}

fn revision_field(obj: &Map<String, Value>, key: &str) -> Result<u64, FieldError> {
    match obj.get(key) {
        None => Err(FieldError::new(key, FieldErrorCode::Required)),
        Some(Value::String(s)) => {
            parse_revision(s).ok_or_else(|| FieldError::new(key, FieldErrorCode::InvalidValue))
        }
        Some(_) => Err(FieldError::new(key, FieldErrorCode::InvalidType)),
    }
}

fn patch_error(e: PatchError) -> RepoError {
    match e {
        PatchError::Invalid(errors) => RepoError::Validation(
            errors
                .into_iter()
                .map(|mut e| {
                    if e.path.is_empty() {
                        e.path = "patch".into();
                    }
                    e
                })
                .collect(),
        ),
        PatchError::AppearanceImportRequired => {
            RepoError::Unavailable(UnavailableCode::RecoveryRequired)
        }
    }
}

/// A changed hotkey binding has to parse before it is saved; registering
/// it (and keeping the old one on failure) is the shortcuts effect.
fn validate_bindings(before: &AppSettings, next: &AppSettings) -> Result<(), RepoError> {
    use tauri_plugin_global_shortcut::Shortcut;
    let pairs = [
        (
            "download.hotkey_binding",
            &before.download.hotkey_binding,
            &next.download.hotkey_binding,
        ),
        (
            "download.clip_hotkey_binding",
            &before.download.clip_hotkey_binding,
            &next.download.clip_hotkey_binding,
        ),
        (
            "download.music_hotkey_binding",
            &before.download.music_hotkey_binding,
            &next.download.music_hotkey_binding,
        ),
    ];
    let errors: Vec<_> = pairs
        .into_iter()
        .filter(|(_, old, new)| old != new && !new.trim().is_empty())
        .filter(|(_, _, new)| new.parse::<Shortcut>().is_err())
        .map(|(path, _, _)| FieldError::new(path, FieldErrorCode::InvalidValue))
        .collect();
    if errors.is_empty() {
        Ok(())
    } else {
        Err(RepoError::Validation(errors))
    }
}

fn patched(current: &AppSettings, patch: &Value) -> Result<AppSettings, RepoError> {
    let next = apply_patch(current, patch).map_err(patch_error)?;
    validate_bindings(current, &next)?;
    Ok(next)
}

pub fn get_v3(
    repo: &SettingsRepository,
    effects: Option<&Arc<EffectsRegistry>>,
) -> (SettingsSnapshotV3, Option<Commit>) {
    let adopted = repo.reconcile();
    if let Some(commit) = &adopted {
        schedule(effects, commit);
    }
    (snapshot_v3(repo, effects), adopted)
}

pub fn patch_v3(
    repo: &SettingsRepository,
    effects: Option<&Arc<EffectsRegistry>>,
    request: &Value,
) -> Result<(SettingsSnapshotV3, Commit), ApiError> {
    let obj = request_object(request, &["expectedRevision", "patch", "secretOperations"])?;
    let mut errors = Vec::new();
    let expected = revision_field(obj, "expectedRevision")
        .map_err(|e| errors.push(e))
        .ok();
    let patch = obj.get("patch");
    if patch.is_none() {
        errors.push(FieldError::new("patch", FieldErrorCode::Required));
    }
    let ops = match parse_secret_operations(obj.get("secretOperations")) {
        Ok(ops) => Some(ops),
        Err(mut e) => {
            errors.append(&mut e);
            None
        }
    };
    let (Some(expected), Some(patch), Some(ops), true) = (expected, patch, ops, errors.is_empty())
    else {
        return Err(invalid(errors));
    };
    let commit = repo.mutate(Some(expected), |current| {
        let mut next = patched(current, patch)?;
        apply_secret_operations(&mut next, &ops);
        Ok(next)
    })?;
    schedule(effects, &commit);
    Ok((snapshot_v3(repo, effects), commit))
}

fn parse_scope(v: Option<&Value>) -> Result<ResetScope, FieldError> {
    let Some(v) = v else {
        return Err(FieldError::new("scope", FieldErrorCode::Required));
    };
    let Value::Object(obj) = v else {
        return Err(FieldError::new("scope", FieldErrorCode::InvalidType));
    };
    let kind = obj.get("kind").and_then(Value::as_str);
    let allowed: &[&str] = match kind {
        Some("app") => &["kind"],
        Some("section") => &["kind", "section"],
        _ => return Err(FieldError::new("scope.kind", FieldErrorCode::InvalidEnum)),
    };
    if let Some(extra) = obj.keys().find(|k| !allowed.contains(&k.as_str())) {
        return Err(FieldError::new(
            format!("scope.{extra}"),
            FieldErrorCode::UnknownField,
        ));
    }
    if kind == Some("app") {
        return Ok(ResetScope::App);
    }
    obj.get("section")
        .and_then(Value::as_str)
        .and_then(SettingsSection::parse)
        .map(ResetScope::Section)
        .ok_or_else(|| FieldError::new("scope.section", FieldErrorCode::InvalidEnum))
}

pub fn reset_v3(
    repo: &SettingsRepository,
    effects: Option<&Arc<EffectsRegistry>>,
    request: &Value,
) -> Result<(SettingsSnapshotV3, Commit), ApiError> {
    let obj = request_object(request, &["expectedRevision", "scope"])?;
    let expected = revision_field(obj, "expectedRevision");
    let scope = parse_scope(obj.get("scope"));
    let (expected, scope) = match (expected, scope) {
        (Ok(r), Ok(s)) => (r, s),
        (a, b) => return Err(invalid([a.err(), b.err()].into_iter().flatten().collect())),
    };
    let commit = repo.mutate(Some(expected), |current| Ok(apply_reset(current, scope)))?;
    schedule(effects, &commit);
    Ok((snapshot_v3(repo, effects), commit))
}

pub fn retry_v3(
    repo: &SettingsRepository,
    effects: Option<&Arc<EffectsRegistry>>,
    request: &Value,
    wait: Duration,
) -> Result<RetryResultV3, ApiError> {
    let obj = request_object(request, &["effectId", "targetRevision"])?;
    let id = match obj.get("effectId") {
        None => Err(FieldError::new("effectId", FieldErrorCode::Required)),
        Some(Value::String(s)) => EffectId::parse(s)
            .ok_or_else(|| FieldError::new("effectId", FieldErrorCode::InvalidEnum)),
        Some(_) => Err(FieldError::new("effectId", FieldErrorCode::InvalidType)),
    };
    let target = revision_field(obj, "targetRevision");
    let (id, target) = match (id, target) {
        (Ok(i), Ok(t)) => (i, t),
        (a, b) => return Err(invalid([a.err(), b.err()].into_iter().flatten().collect())),
    };
    let snap = repo.snapshot();
    match snap.gate {
        Gate::Open => {}
        Gate::Failed { .. } => return Err(ApiError::unavailable(UnavailableCode::LoadFailed)),
        Gate::Indeterminate {
            attempted,
            observed,
        } => {
            return Err(RepoError::Indeterminate {
                attempted,
                observed,
            }
            .into())
        }
    }
    if target > snap.revision {
        return Err(invalid(vec![FieldError::new(
            "targetRevision",
            FieldErrorCode::OutOfRange,
        )]));
    }
    let outcome = match effects {
        Some(registry) => registry.retry(id, target, snap.revision, &snap.settings, wait),
        None => RetryOutcome::Pending,
    };
    Ok(RetryResultV3 {
        outcome,
        snapshot: snapshot_v3(repo, effects),
    })
}

pub fn import_v3(
    repo: &SettingsRepository,
    effects: Option<&Arc<EffectsRegistry>>,
    request: &Value,
) -> Result<(ImportResultV3, Commit), ApiError> {
    let obj = request_object(request, &["source", "rawWorkspace"])?;
    let mut errors = Vec::new();
    match obj.get("source") {
        Some(Value::String(s)) if s == WORKSPACE_APPEARANCE_SOURCE => {}
        Some(Value::String(_)) => {
            errors.push(FieldError::new("source", FieldErrorCode::InvalidEnum))
        }
        Some(_) => errors.push(FieldError::new("source", FieldErrorCode::InvalidType)),
        None => errors.push(FieldError::new("source", FieldErrorCode::Required)),
    }
    let raw = match obj.get("rawWorkspace") {
        Some(Value::Null) => None,
        Some(Value::String(s)) if s.len() <= WORKSPACE_APPEARANCE_MAX_BYTES => Some(s.as_str()),
        Some(Value::String(_)) => {
            errors.push(FieldError::new("rawWorkspace", FieldErrorCode::OutOfRange));
            None
        }
        Some(_) => {
            errors.push(FieldError::new("rawWorkspace", FieldErrorCode::InvalidType));
            None
        }
        None => {
            errors.push(FieldError::new("rawWorkspace", FieldErrorCode::Required));
            None
        }
    };
    if !errors.is_empty() {
        return Err(invalid(errors));
    }
    let (outcome, commit) = repo.import_appearance(raw)?;
    schedule(effects, &commit);
    let outcome = match outcome {
        ImportOutcome::Imported => ImportOutcomeV3::Imported,
        ImportOutcome::AlreadyImported => ImportOutcomeV3::AlreadyImported,
    };
    Ok((
        ImportResultV3 {
            outcome,
            snapshot: snapshot_v3(repo, effects),
        },
        commit,
    ))
}

// ---------------------------------------------------------------------------
// v3 commands
// ---------------------------------------------------------------------------

async fn blocking<T: Send + 'static>(
    f: impl FnOnce() -> Result<T, ApiError> + Send + 'static,
) -> Result<T, ApiError> {
    tauri::async_runtime::spawn_blocking(f)
        .await
        .unwrap_or_else(|_| Err(ApiError::unavailable(UnavailableCode::LoadFailed)))
}

#[tauri::command]
pub async fn settings_get_v3(app: tauri::AppHandle) -> Result<SettingsSnapshotV3, ApiError> {
    blocking(move || {
        let repo = settings_repository::global();
        let (snapshot, adopted) = get_v3(&repo, settings_effects::registry().as_ref());
        if adopted.is_some() {
            settings_effects::emit_changed(&app);
        }
        Ok(snapshot)
    })
    .await
}

#[tauri::command]
pub async fn settings_patch_v3(
    app: tauri::AppHandle,
    request: Value,
) -> Result<SettingsSnapshotV3, ApiError> {
    blocking(move || {
        let repo = settings_repository::global();
        let (snapshot, commit) = patch_v3(&repo, settings_effects::registry().as_ref(), &request)?;
        if commit.changed {
            settings_effects::emit_changed(&app);
        }
        Ok(snapshot)
    })
    .await
}

#[tauri::command]
pub async fn settings_reset_v3(
    app: tauri::AppHandle,
    request: Value,
) -> Result<SettingsSnapshotV3, ApiError> {
    blocking(move || {
        let repo = settings_repository::global();
        let (snapshot, commit) = reset_v3(&repo, settings_effects::registry().as_ref(), &request)?;
        if commit.changed {
            settings_effects::emit_changed(&app);
        }
        Ok(snapshot)
    })
    .await
}

#[tauri::command]
pub async fn settings_retry_effect_v3(request: Value) -> Result<RetryResultV3, ApiError> {
    blocking(move || {
        let repo = settings_repository::global();
        retry_v3(
            &repo,
            settings_effects::registry().as_ref(),
            &request,
            Duration::from_secs(5),
        )
    })
    .await
}

#[tauri::command]
pub async fn settings_import_appearance_v3(
    app: tauri::AppHandle,
    request: Value,
) -> Result<ImportResultV3, ApiError> {
    blocking(move || {
        let repo = settings_repository::global();
        let (result, commit) = import_v3(&repo, settings_effects::registry().as_ref(), &request)?;
        if commit.changed {
            settings_effects::emit_changed(&app);
        }
        Ok(result)
    })
    .await
}

// ---------------------------------------------------------------------------
// Transitional wrappers (same repository, same validation, redacted DTO)
// ---------------------------------------------------------------------------

#[tauri::command]
pub fn get_settings(app: tauri::AppHandle) -> Result<SettingsPublicV3, ApiError> {
    let repo = settings_repository::global();
    if let Some(commit) = repo.reconcile() {
        settings_effects::after_commit(&app, &commit);
    }
    let snap = repo.snapshot();
    match snap.gate {
        Gate::Open => Ok(snap.settings.to_public()),
        _ => Err(ApiError::unavailable(UnavailableCode::LoadFailed)),
    }
}

#[tauri::command]
pub fn update_settings(
    app: tauri::AppHandle,
    partial: String,
) -> Result<SettingsPublicV3, ApiError> {
    let patch: Value = serde_json::from_str(&partial)
        .map_err(|_| invalid(vec![FieldError::new("patch", FieldErrorCode::InvalidType)]))?;
    let commit = settings_repository::global().mutate(None, |current| patched(current, &patch))?;
    settings_effects::after_commit(&app, &commit);
    Ok(commit.after.to_public())
}

/// Resets the app scope of the reset policy; secrets, the bridge token and
/// port, consents and unknown keys are kept.
#[tauri::command]
pub fn reset_settings(app: tauri::AppHandle) -> Result<SettingsPublicV3, ApiError> {
    let commit = settings_repository::global()
        .mutate(None, |current| Ok(apply_reset(current, ResetScope::App)))?;
    settings_effects::after_commit(&app, &commit);
    Ok(commit.after.to_public())
}

#[tauri::command]
pub fn mark_onboarding_complete(app: tauri::AppHandle) -> Result<(), String> {
    config::update_settings(&app, |s| s.onboarding_completed = true)
        .map(|_| ())
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn mark_legal_acknowledged(app: tauri::AppHandle) -> Result<(), String> {
    config::update_settings(&app, |s| s.legal_acknowledged = true)
        .map(|_| ())
        .map_err(|e| e.to_string())
}

/// The proxy URL the updater plugin needs, built here so the password never
/// travels in the settings snapshot. Same rule as the old `proxyUrl()` of
/// `src/lib/updater.ts`. Never logged.
#[tauri::command]
pub fn get_updater_proxy_url() -> Result<Option<String>, String> {
    Ok(updater_proxy_url(
        &settings_repository::global().settings().proxy,
    ))
}

pub fn updater_proxy_url(proxy: &ProxySettings) -> Option<String> {
    if !proxy.enabled || proxy.host.is_empty() || proxy.port == 0 {
        return None;
    }
    let scheme = match proxy.proxy_type.as_str() {
        "socks5" => "socks5",
        "https" => "https",
        _ => "http",
    };
    let auth = if proxy.username.is_empty() {
        String::new()
    } else {
        format!(
            "{}:{}@",
            encode_uri_component(&proxy.username),
            encode_uri_component(&proxy.password)
        )
    };
    Some(format!("{scheme}://{auth}{}:{}", proxy.host, proxy.port))
}

/// JavaScript's `encodeURIComponent`: everything but `A-Z a-z 0-9 - _ . ! ~
/// * ' ( )` becomes UTF-8 percent escapes.
fn encode_uri_component(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b"-_.!~*'()".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

#[derive(serde::Serialize)]
pub struct BridgeInfo {
    pub enabled: bool,
    pub port: u16,
    pub token: String,
    pub url: String,
}

/// Returns the localhost bridge connection info for the Settings UI.
///
/// `port == 0` means the bridge has not finished binding yet (the user can
/// retry shortly). `token` may be empty until the first run completes.
#[tauri::command]
pub fn get_bridge_info(app: tauri::AppHandle) -> Result<BridgeInfo, String> {
    let settings = config::load_settings(&app);
    let url = if settings.bridge.port == 0 {
        String::new()
    } else {
        crate::local_bridge::build_pairing_url(settings.bridge.port)
    };
    Ok(BridgeInfo {
        enabled: settings.bridge.enabled,
        port: settings.bridge.port,
        token: settings.bridge.token,
        url,
    })
}

/// Forces a fresh token to be generated. Used by the Settings UI when the
/// user wants to revoke a previously-paired browser.
#[tauri::command]
pub fn rotate_bridge_token(app: tauri::AppHandle) -> Result<BridgeInfo, String> {
    let token = crate::local_bridge::generate_token();
    config::update_settings(&app, |s| s.bridge.token = token).map_err(|e| e.to_string())?;
    get_bridge_info(app)
}

#[derive(serde::Serialize)]
pub struct BridgePairStatus {
    pub ok: bool,
    pub window_secs: i64,
    pub port: u16,
    pub enabled: bool,
}

/// Opens a short, single-use, user-consented pairing window so an installed
/// browser extension can fetch the bearer token from `GET /v1/pair` without
/// the user copy-pasting it.
#[tauri::command]
pub fn bridge_open_pairing(app: tauri::AppHandle) -> Result<BridgePairStatus, String> {
    let settings = config::load_settings(&app);
    if !settings.bridge.enabled {
        return Ok(BridgePairStatus {
            ok: false,
            window_secs: 0,
            port: settings.bridge.port,
            enabled: false,
        });
    }
    let window_secs = crate::local_bridge::open_pairing_window();
    Ok(BridgePairStatus {
        ok: true,
        window_secs,
        port: settings.bridge.port,
        enabled: true,
    })
}

#[cfg(test)]
mod v3_contract {
    use super::*;
    use crate::storage::settings_effects::{ApplyOutcome, EffectApplier, EffectState};
    use crate::storage::settings_repository::{RECOVERY_DIR, SETTINGS_FILE};
    use serde_json::json;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Mutex;

    /// Counts every local effect; stands in for proxy/hotkey/autostart/...
    /// so nothing touches the OS, a provider or the network.
    #[derive(Default)]
    struct Counting {
        calls: AtomicUsize,
        ids: Mutex<Vec<EffectId>>,
        fail: std::sync::atomic::AtomicBool,
    }

    impl EffectApplier for Counting {
        fn apply(&self, id: EffectId, _: &AppSettings, _: &AppSettings) -> ApplyOutcome {
            self.calls.fetch_add(1, Ordering::SeqCst);
            self.ids.lock().unwrap().push(id);
            if self.fail.load(Ordering::SeqCst) {
                ApplyOutcome::Failed(settings_effects::FailCode::ApplyFailed)
            } else {
                ApplyOutcome::Applied
            }
        }
    }

    fn setup() -> (
        tempfile::TempDir,
        SettingsRepository,
        Arc<EffectsRegistry>,
        Arc<Counting>,
    ) {
        let dir = tempfile::tempdir().unwrap();
        let mut s = AppSettings::default();
        s.proxy.password = "sentinel-proxy-secret".into();
        s.advanced.twitter_manual_cookie = "sentinel-cookie-secret".into();
        s.bridge.token = "sentinel-bridge-secret".into();
        s.bridge.port = 47_001;
        s.legal_acknowledged = true;
        let mut app = serde_json::to_value(s).unwrap();
        app["unknown_secret_like"] = json!("sentinel-unknown-secret");
        std::fs::write(
            dir.path().join(SETTINGS_FILE),
            serde_json::to_vec(&json!({ "app_settings": app, "_settings_revision": 10 })).unwrap(),
        )
        .unwrap();
        let repo = SettingsRepository::open(dir.path().into());
        let applier = Arc::new(Counting::default());
        let registry = EffectsRegistry::new(applier.clone(), Arc::new(|| {}));
        (dir, repo, registry, applier)
    }

    fn import(repo: &SettingsRepository, reg: &Arc<EffectsRegistry>) {
        import_v3(
            repo,
            Some(reg),
            &json!({ "source": WORKSPACE_APPEARANCE_SOURCE, "rawWorkspace": null }),
        )
        .unwrap();
    }

    fn err_json(e: ApiError) -> Value {
        serde_json::to_value(e).unwrap()
    }

    // API-01/02: revisions travel as strings; no secret in any public output.
    #[test]
    fn get_returns_a_redacted_ready_snapshot() {
        let (_dir, repo, reg, _) = setup();
        let (snap, _) = get_v3(&repo, Some(&reg));
        let v = serde_json::to_value(&snap).unwrap();
        assert_eq!(v["revision"], json!("10"));
        assert_eq!(v["loadStatus"], json!({ "kind": "loaded" }));
        assert_eq!(v["applied"], json!(true));
        assert_eq!(v["fieldErrors"], json!([]));
        assert_eq!(v["settings"]["proxy"]["has_password"], json!(true));
        assert_eq!(v["settings"]["bridge"]["has_token"], json!(true));
        assert_eq!(v["settings"]["schema_version"], json!(3));
        let text = v.to_string();
        assert!(!text.contains("sentinel"), "{text}");
        let legacy = serde_json::to_string(&repo.settings().to_public()).unwrap();
        assert!(!legacy.contains("sentinel"));
    }

    // API-03
    #[test]
    fn secret_operations_keep_set_and_remove_only_their_target() {
        let (_dir, repo, reg, _) = setup();
        let (snap, _) = patch_v3(
            &repo,
            Some(&reg),
            &json!({ "expectedRevision": "10", "patch": { "download": { "video_quality": "1080p" } } }),
        )
        .unwrap();
        assert_eq!(snap.revision.as_deref(), Some("11"));
        assert_eq!(repo.settings().proxy.password, "sentinel-proxy-secret");

        patch_v3(
            &repo,
            Some(&reg),
            &json!({ "expectedRevision": "11", "patch": {},
                     "secretOperations": { "proxyPassword": { "op": "set", "value": "new-pass" },
                                           "twitterManualCookie": { "op": "keep" } } }),
        )
        .unwrap();
        let s = repo.settings();
        assert_eq!(s.proxy.password, "new-pass");
        assert_eq!(s.advanced.twitter_manual_cookie, "sentinel-cookie-secret");
        patch_v3(
            &repo,
            Some(&reg),
            &json!({ "expectedRevision": "12", "patch": {},
                     "secretOperations": { "twitterManualCookie": { "op": "remove" } } }),
        )
        .unwrap();
        let s = repo.settings();
        assert_eq!(s.advanced.twitter_manual_cookie, "");
        assert_eq!(s.proxy.password, "new-pass");
        assert_eq!(s.bridge.token, "sentinel-bridge-secret");

        let before = std::fs::read(repo.path()).unwrap();
        for bad in [
            json!({ "expectedRevision": "13", "patch": { "proxy": { "password": "" } } }),
            json!({ "expectedRevision": "13", "patch": {},
                    "secretOperations": { "proxyPassword": { "op": "set", "value": "" } } }),
            json!({ "expectedRevision": "13", "patch": {},
                    "secretOperations": { "proxyPassword": { "op": "keep", "value": "x" } } }),
            json!({ "expectedRevision": "13", "patch": {}, "extra": 1 }),
            json!({ "expectedRevision": 13, "patch": {} }),
            json!({ "expectedRevision": "013", "patch": {} }),
            json!({ "patch": {} }),
        ] {
            let e = err_json(patch_v3(&repo, Some(&reg), &bad).unwrap_err());
            assert_eq!(e["kind"], "validation", "{bad}");
            assert!(!e.to_string().contains("sentinel"));
        }
        assert_eq!(std::fs::read(repo.path()).unwrap(), before);
        // An empty patch with no effective operation changes nothing.
        let (snap, commit) = patch_v3(
            &repo,
            Some(&reg),
            &json!({ "expectedRevision": "13", "patch": {} }),
        )
        .unwrap();
        assert!(!commit.changed);
        assert_eq!(snap.revision.as_deref(), Some("13"));
    }

    // API-04
    #[test]
    fn a_stale_revision_is_a_conflict_before_anything_is_written() {
        let (_dir, repo, reg, applier) = setup();
        patch_v3(
            &repo,
            Some(&reg),
            &json!({ "expectedRevision": "10", "patch": { "proxy": { "enabled": false, "port": 3128 } } }),
        )
        .unwrap();
        reg.wait_idle(&EffectId::ALL, Duration::from_secs(5));
        let before = std::fs::read(repo.path()).unwrap();
        let calls = applier.calls.load(Ordering::SeqCst);
        assert_eq!(calls, 1, "the proxy effect of the first commit");
        let e = err_json(
            patch_v3(
                &repo,
                Some(&reg),
                &json!({ "expectedRevision": "10", "patch": { "start_minimized": true } }),
            )
            .unwrap_err(),
        );
        assert_eq!(
            e,
            json!({ "kind": "conflict", "code": "revision_conflict",
                    "currentRevision": "11", "fieldErrors": [] })
        );
        assert_eq!(std::fs::read(repo.path()).unwrap(), before);
        reg.wait_idle(&EffectId::ALL, Duration::from_secs(5));
        assert_eq!(applier.calls.load(Ordering::SeqCst), calls);
    }

    // The audit regression through the real patch path.
    #[test]
    fn an_out_of_range_patch_changes_nothing() {
        let (_dir, repo, reg, applier) = setup();
        let before = std::fs::read(repo.path()).unwrap();
        let e = err_json(
            patch_v3(
                &repo,
                Some(&reg),
                &json!({ "expectedRevision": "10",
                         "patch": { "proxy": { "port": 70000 }, "download": { "video_quality": "1080p" } } }),
            )
            .unwrap_err(),
        );
        assert_eq!(
            e["fieldErrors"],
            json!([{ "path": "proxy.port", "code": "out_of_range" }])
        );
        assert_eq!(std::fs::read(repo.path()).unwrap(), before);
        assert_eq!(repo.settings().download.video_quality, "720p");
        assert_eq!(applier.calls.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn appearance_waits_for_the_import_and_a_bad_hotkey_is_refused() {
        let (_dir, repo, reg, _) = setup();
        let e = err_json(
            patch_v3(
                &repo,
                Some(&reg),
                &json!({ "expectedRevision": "10", "patch": { "appearance": { "preset": "dracula" } } }),
            )
            .unwrap_err(),
        );
        assert_eq!(e["kind"], "unavailable");
        assert_eq!(e["code"], "recovery_required");
        import(&repo, &reg);
        let (snap, _) = patch_v3(
            &repo,
            Some(&reg),
            &json!({ "expectedRevision": "11", "patch": { "appearance": { "preset": "dracula" } } }),
        )
        .unwrap();
        let s = snap.settings.unwrap();
        assert_eq!(s.appearance.theme, "dracula");
        assert_eq!(s.appearance.preset, "dracula");

        let e = err_json(
            patch_v3(
                &repo,
                Some(&reg),
                &json!({ "expectedRevision": "12",
                         "patch": { "download": { "hotkey_binding": "Ctrl+NotAKey+" } } }),
            )
            .unwrap_err(),
        );
        assert_eq!(
            e["fieldErrors"],
            json!([{ "path": "download.hotkey_binding", "code": "invalid_value" }])
        );
    }

    // API-06 through the command path: saved is not applied, a failed
    // effect is pending and retryable, the retry applies the latest.
    #[test]
    fn a_failed_effect_is_saved_and_pending_until_retried() {
        let (_dir, repo, reg, applier) = setup();
        applier.fail.store(true, Ordering::SeqCst);
        let (snap, _) = patch_v3(
            &repo,
            Some(&reg),
            &json!({ "expectedRevision": "10", "patch": { "start_with_system": true } }),
        )
        .unwrap();
        assert_eq!(snap.revision.as_deref(), Some("11"));
        reg.wait_idle(&[EffectId::AppAutostart], Duration::from_secs(5));
        let snap = snapshot_v3(&repo, Some(&reg));
        assert!(!snap.applied);
        assert_eq!(snap.pending_effects[0].id, EffectId::AppAutostart);
        assert_eq!(snap.pending_effects[0].state, EffectState::Failed);
        assert_eq!(snap.pending_effects[0].target_revision, "11");
        // Retrying writes nothing and never bumps the revision.
        let bytes = std::fs::read(repo.path()).unwrap();
        applier.fail.store(false, Ordering::SeqCst);
        let r = retry_v3(
            &repo,
            Some(&reg),
            &json!({ "effectId": "app.autostart", "targetRevision": "11" }),
            Duration::from_secs(5),
        )
        .unwrap();
        assert_eq!(r.outcome, RetryOutcome::Applied);
        assert!(r.snapshot.applied);
        assert_eq!(std::fs::read(repo.path()).unwrap(), bytes);
        assert_eq!(repo.revision(), 11);
        // A future revision is a validation error; an unknown id too.
        for bad in [
            json!({ "effectId": "app.autostart", "targetRevision": "12" }),
            json!({ "effectId": "app.nope", "targetRevision": "11" }),
            json!({ "effectId": "app.autostart", "targetRevision": "11", "value": true }),
        ] {
            let e = err_json(retry_v3(&repo, Some(&reg), &bad, Duration::ZERO).unwrap_err());
            assert_eq!(e["kind"], "validation", "{bad}");
        }
    }

    // API-07 through the command path.
    #[test]
    fn two_imports_give_one_imported_and_one_already_imported() {
        let (dir, repo, reg, _) = setup();
        let repo = Arc::new(repo);
        let handles: Vec<_> = (0..2)
            .map(|_| {
                let repo = repo.clone();
                let reg = reg.clone();
                std::thread::spawn(move || {
                    import_v3(
                        &repo,
                        Some(&reg),
                        &json!({ "source": WORKSPACE_APPEARANCE_SOURCE,
                                 "rawWorkspace": "{\"mode\":\"dark\",\"preset\":\"orange-soft\"}" }),
                    )
                    .unwrap()
                    .0
                    .outcome
                })
            })
            .collect();
        let mut outcomes: Vec<_> = handles.into_iter().map(|h| h.join().unwrap()).collect();
        outcomes.sort_by_key(|o| *o == ImportOutcomeV3::AlreadyImported);
        assert_eq!(
            outcomes,
            vec![ImportOutcomeV3::Imported, ImportOutcomeV3::AlreadyImported]
        );
        assert_eq!(repo.revision(), 11);
        assert_eq!(
            std::fs::read_dir(dir.path().join(RECOVERY_DIR))
                .unwrap()
                .count(),
            1
        );
        for bad in [
            json!({ "source": "other", "rawWorkspace": null }),
            json!({ "source": WORKSPACE_APPEARANCE_SOURCE }),
            json!({ "source": WORKSPACE_APPEARANCE_SOURCE, "rawWorkspace": 5 }),
            json!({ "source": WORKSPACE_APPEARANCE_SOURCE, "rawWorkspace": "x".repeat(65_537) }),
        ] {
            let e = err_json(import_v3(&repo, Some(&reg), &bad).unwrap_err());
            assert_eq!(e["kind"], "validation");
        }
    }

    // API-10
    #[test]
    fn a_reset_keeps_secrets_bridge_consents_and_unknown_keys_and_reapplies_effects() {
        let (_dir, repo, reg, applier) = setup();
        import(&repo, &reg);
        patch_v3(
            &repo,
            Some(&reg),
            &json!({ "expectedRevision": "11", "patch": {
                "appearance": { "language": "pt", "mode": "dark" },
                "advanced": { "max_concurrent_segments": 4 },
                "download": { "video_quality": "1080p" } } }),
        )
        .unwrap();
        reg.wait_idle(&EffectId::ALL, Duration::from_secs(5));
        applier.ids.lock().unwrap().clear();

        // Section: only its intersection with the allowlist.
        let (snap, _) = reset_v3(
            &repo,
            Some(&reg),
            &json!({ "expectedRevision": "12", "scope": { "kind": "section", "section": "general" } }),
        )
        .unwrap();
        let s = snap.settings.unwrap();
        assert_eq!(s.appearance.language, "en");
        assert_eq!(s.appearance.mode, "dark");
        assert_eq!(s.download.video_quality, "1080p");

        let (snap, _) = reset_v3(
            &repo,
            Some(&reg),
            &json!({ "expectedRevision": "13", "scope": { "kind": "app" } }),
        )
        .unwrap();
        let public = snap.settings.unwrap();
        assert_eq!(public.appearance.mode, "system");
        assert_eq!(public.appearance.migration_version, 1);
        let s = repo.settings();
        assert_eq!(s.download.video_quality, "720p");
        assert_eq!(s.advanced.max_concurrent_segments, 20);
        assert_eq!(s.proxy.password, "sentinel-proxy-secret");
        assert_eq!(s.advanced.twitter_manual_cookie, "sentinel-cookie-secret");
        assert_eq!(s.bridge.token, "sentinel-bridge-secret");
        assert_eq!(s.bridge.port, 47_001);
        assert!(s.legal_acknowledged);
        let disk: Value = serde_json::from_slice(&std::fs::read(repo.path()).unwrap()).unwrap();
        assert_eq!(
            disk["app_settings"]["unknown_secret_like"],
            "sentinel-unknown-secret"
        );
        // The changed runtime domain is applied again through the registry.
        reg.wait_idle(&EffectId::ALL, Duration::from_secs(5));
        assert!(applier
            .ids
            .lock()
            .unwrap()
            .contains(&EffectId::DownloadsSegments));
        // A lost confirmation retried on the old revision is a conflict,
        // not a second reset.
        let e = err_json(
            reset_v3(
                &repo,
                Some(&reg),
                &json!({ "expectedRevision": "13", "scope": { "kind": "app" } }),
            )
            .unwrap_err(),
        );
        assert_eq!(e["kind"], "conflict");
        for bad in [
            json!({ "expectedRevision": "14", "scope": { "kind": "section", "section": "secrets" } }),
            json!({ "expectedRevision": "14", "scope": { "kind": "app", "path": "proxy" } }),
            json!({ "expectedRevision": "14" }),
        ] {
            let e = err_json(reset_v3(&repo, Some(&reg), &bad).unwrap_err());
            assert_eq!(e["kind"], "validation", "{bad}");
        }
    }

    // API-12: save/reset/import/retry only ever reach the local applier for
    // the domains that changed; appearance and plain preferences run none.
    #[test]
    fn mutations_only_reach_the_local_effects_of_changed_fields() {
        let (_dir, repo, reg, applier) = setup();
        import(&repo, &reg);
        patch_v3(
            &repo,
            Some(&reg),
            &json!({ "expectedRevision": "11", "patch": {
                "appearance": { "mode": "dark" }, "download": { "video_quality": "1080p" } } }),
        )
        .unwrap();
        reg.wait_idle(&EffectId::ALL, Duration::from_secs(5));
        assert_eq!(applier.calls.load(Ordering::SeqCst), 0);
        patch_v3(
            &repo,
            Some(&reg),
            &json!({ "expectedRevision": "12", "patch": { "llm": { "jobs_max_parallel": 6 } } }),
        )
        .unwrap();
        reg.wait_idle(&EffectId::ALL, Duration::from_secs(5));
        assert_eq!(*applier.ids.lock().unwrap(), vec![EffectId::LlmJobs]);
    }

    // API-09: a failed load is a FailedSnapshot and blocks the mutators.
    #[test]
    fn an_unreadable_profile_is_a_failed_snapshot() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(SETTINGS_FILE), b"{ broken").unwrap();
        let repo = SettingsRepository::open(dir.path().into());
        let (snap, _) = get_v3(&repo, None);
        let v = serde_json::to_value(&snap).unwrap();
        assert_eq!(v["settings"], Value::Null);
        assert_eq!(
            v["loadStatus"],
            json!({ "kind": "failed", "code": "invalid_document", "recoveryId": null })
        );
        assert_eq!(v["applied"], json!(false));
        let e = err_json(
            patch_v3(
                &repo,
                None,
                &json!({ "expectedRevision": "0", "patch": { "start_minimized": true } }),
            )
            .unwrap_err(),
        );
        assert_eq!(e["code"], "load_failed");
        assert_eq!(std::fs::read(repo.path()).unwrap(), b"{ broken");
    }

    #[test]
    fn the_updater_proxy_url_matches_the_old_frontend_rule() {
        let mut p = ProxySettings::default();
        assert_eq!(updater_proxy_url(&p), None);
        p.enabled = true;
        assert_eq!(updater_proxy_url(&p), None, "no host");
        p.host = "127.0.0.1".into();
        p.port = 0;
        assert_eq!(updater_proxy_url(&p), None, "no port");
        p.port = 1080;
        assert_eq!(
            updater_proxy_url(&p).as_deref(),
            Some("http://127.0.0.1:1080")
        );
        p.proxy_type = "socks5".into();
        p.username = "us er@x".into();
        p.password = "p:w/d!*".into();
        assert_eq!(
            updater_proxy_url(&p).as_deref(),
            Some("socks5://us%20er%40x:p%3Aw%2Fd!*@127.0.0.1:1080")
        );
        p.proxy_type = "https".into();
        p.password.clear();
        assert_eq!(
            updater_proxy_url(&p).as_deref(),
            Some("https://us%20er%40x:@127.0.0.1:1080")
        );
        p.proxy_type = "ftp".into();
        p.username.clear();
        assert_eq!(
            updater_proxy_url(&p).as_deref(),
            Some("http://127.0.0.1:1080")
        );
        assert_eq!(encode_uri_component("ção"), "%C3%A7%C3%A3o");
    }
}

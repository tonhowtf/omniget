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

pub fn autostart_wanted(s: &AppSettings) -> bool {
    s.start_with_system || s.run_in_background
}

pub fn reconcile_close(s: &mut AppSettings, old_background: bool, old_exit_on_close: bool) {
    if s.run_in_background && s.exit_on_close {
        if old_background && !old_exit_on_close {
            s.run_in_background = false;
        } else {
            s.exit_on_close = false;
        }
    }
}

pub const APP_LANGUAGES: &[&str] = &[
    "en", "ru", "el", "pt", "zh", "zh-TW", "ja", "it", "fr", "es", "fa", "lo",
];

pub fn apply_live(app: &tauri::AppHandle, before: &AppSettings, after: &AppSettings) {
    crate::commands::washmybelly::sync_settings(&after.washmybelly);
    if after.appearance.language != before.appearance.language {
        crate::canvas::companion::on_language(app, &after.appearance.language);
    }
}

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

fn validate_bindings(before: &AppSettings, next: &AppSettings) -> Result<(), RepoError> {
    use tauri_plugin_global_shortcut::Shortcut;
    let pairs = [
        (
            "download.hotkey_binding",
            &before.download.hotkey_binding,
            &next.download.hotkey_binding,
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
    let mut next = apply_patch(current, patch).map_err(patch_error)?;
    validate_bindings(current, &next)?;
    if next.appearance.language != current.appearance.language
        && !APP_LANGUAGES.contains(&next.appearance.language.as_str())
    {
        return Err(RepoError::Validation(vec![FieldError::new(
            "appearance.language",
            FieldErrorCode::InvalidValue,
        )]));
    }
    reconcile_close(&mut next, current.run_in_background, current.exit_on_close);
    next.canvas.normalize();
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
            apply_live(&app, &commit.before, &commit.after);
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
            apply_live(&app, &commit.before, &commit.after);
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

#[tauri::command]
pub fn rotate_bridge_token(app: tauri::AppHandle) -> Result<BridgeInfo, String> {
    let token = crate::local_bridge::generate_token();
    omniget_core::core::llm::cli_runtime::session::register_secret_value(&token);
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

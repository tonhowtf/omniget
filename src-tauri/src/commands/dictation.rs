use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, LazyLock, Mutex, RwLock};
use std::time::{Duration, Instant};

use serde::Serialize;
use tauri::{AppHandle, Emitter};
use tauri_plugin_clipboard_manager::ClipboardExt;
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutState};

use omniget_core::core::asr;
use omniget_core::core::dictation::capture::{self, Capture, Recording};
use omniget_core::core::dictation::devices::{self, Device, Platform};
use omniget_core::core::dictation::paste;
use omniget_core::core::dictation::shortcut as keys;
use omniget_core::core::dictation::store::{self, Entry, Mode, Output, Settings};
use omniget_core::core::dictation::text::{self, PostProcess};

const STATE_EVENT: &str = "dictation-state";
const LEVEL_EVENT: &str = "dictation-level";
const MODEL_EVENT: &str = "dictation-model-progress";
const TAIL: Duration = Duration::from_millis(250);
const HOLD_MIN: Duration = Duration::from_millis(300);
const MIN_SAMPLES: usize = (capture::SAMPLE_RATE as usize) / 4;
const SILENCE_DBFS: f32 = -58.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
enum Source {
    Page,
    Shortcut,
}

#[derive(Debug, Clone, Serialize)]
pub struct ErrorInfo {
    pub code: String,
    pub detail: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct StateEvent {
    pub phase: &'static str,
    pub session: u64,
    source: Source,
    pub started_at: Option<i64>,
    pub max_ms: u64,
    pub entry: Option<Entry>,
    pub outcome: Option<&'static str>,
    pub error: Option<ErrorInfo>,
    pub notice: Option<&'static str>,
}

impl StateEvent {
    fn idle(session: u64) -> Self {
        Self {
            phase: "idle",
            session,
            source: Source::Page,
            started_at: None,
            max_ms: 0,
            entry: None,
            outcome: None,
            error: None,
            notice: None,
        }
    }
}

struct Live {
    session: u64,
    capture: Capture,
    source: Source,
    started: Instant,
    started_at: i64,
    max_ms: u64,
}

struct Inner {
    session: u64,
    live: Option<Live>,
    busy: bool,
    start_source: Option<Source>,
    pending_release: bool,
    last: StateEvent,
}

static STATE: LazyLock<Mutex<Inner>> = LazyLock::new(|| {
    Mutex::new(Inner {
        session: 0,
        live: None,
        busy: false,
        start_source: None,
        pending_release: false,
        last: StateEvent::idle(0),
    })
});

static SETTINGS: RwLock<Option<Settings>> = RwLock::new(None);
static STORE_LOCK: Mutex<()> = Mutex::new(());
static INSTALLING: AtomicBool = AtomicBool::new(false);
static INITIALIZED: AtomicBool = AtomicBool::new(false);

#[derive(Default)]
struct Reg {
    current: Option<Shortcut>,
    error: Option<String>,
    paused: bool,
    esc: Option<Shortcut>,
}

static REG: LazyLock<Mutex<Reg>> = LazyLock::new(|| Mutex::new(Reg::default()));
static APPLY: Mutex<()> = Mutex::new(());

fn state() -> std::sync::MutexGuard<'static, Inner> {
    STATE.lock().unwrap_or_else(|p| p.into_inner())
}

fn reg() -> std::sync::MutexGuard<'static, Reg> {
    REG.lock().unwrap_or_else(|p| p.into_inner())
}

fn serial() -> std::sync::MutexGuard<'static, ()> {
    APPLY.lock().unwrap_or_else(|p| p.into_inner())
}

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

fn data_dir() -> Result<PathBuf, String> {
    store::dir().ok_or_else(|| "NO_DATA_DIR".to_string())
}

fn current_settings() -> Settings {
    if let Ok(g) = SETTINGS.read() {
        if let Some(s) = g.as_ref() {
            return s.clone();
        }
    }
    let loaded = store::dir()
        .map(|d| store::load_settings(&d))
        .unwrap_or_default();
    if let Ok(mut g) = SETTINGS.write() {
        *g = Some(loaded.clone());
    }
    loaded
}

fn is_mac() -> bool {
    cfg!(target_os = "macos")
}

fn emit_state(app: &AppHandle, event: StateEvent) {
    {
        let mut s = state();
        if event.session >= s.last.session {
            s.last = event.clone();
        }
    }
    let _ = app.emit(STATE_EVENT, event);
}

fn error_event(session: u64, source: Source, code: &str, detail: Option<String>) -> StateEvent {
    StateEvent {
        phase: "error",
        source,
        error: Some(ErrorInfo {
            code: code.to_string(),
            detail: detail.filter(|d| !d.trim().is_empty()),
        }),
        ..StateEvent::idle(session)
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ShortcutStatus {
    pub enabled: bool,
    pub shortcut: String,
    pub registered: bool,
    pub paused: bool,
    pub error: Option<String>,
}

fn shortcut_status(settings: &Settings) -> ShortcutStatus {
    let r = reg();
    ShortcutStatus {
        enabled: settings.enabled,
        shortcut: settings.shortcut.clone(),
        registered: r.current.is_some(),
        paused: r.paused,
        error: r.error.clone(),
    }
}

fn app_conflict(app: &AppHandle, candidate: &Shortcut) -> bool {
    let app_settings = crate::storage::config::load_settings(app);
    let d = &app_settings.download;
    let taken = |enabled: bool, binding: &str| {
        enabled
            && binding
                .parse::<Shortcut>()
                .map(|s| s == *candidate)
                .unwrap_or(false)
    };
    taken(d.hotkey_enabled, &d.hotkey_binding)
        || taken(d.music_hotkey_enabled, &d.music_hotkey_binding)
        || taken(true, crate::canvas::companion::OMNI_SHORTCUT)
}

fn check(app: &AppHandle, raw: &str) -> Result<(String, Shortcut), String> {
    let normalized = keys::normalize(raw, is_mac()).map_err(|e| e.code().to_string())?;
    let parsed = normalized
        .parse::<Shortcut>()
        .map_err(|_| "UNKNOWN_KEY".to_string())?;
    if app_conflict(app, &parsed) {
        return Err("CONFLICT_APP".into());
    }
    Ok((normalized, parsed))
}

fn on_hotkey(
    app: &AppHandle,
    _shortcut: &Shortcut,
    event: tauri_plugin_global_shortcut::ShortcutEvent,
) {
    let pressed = event.state == ShortcutState::Pressed;
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        handle_hotkey(app, pressed).await;
    });
}

fn apply_shortcut(app: &AppHandle, settings: &Settings) -> Result<(), String> {
    let _serial = serial();
    let (paused, current) = {
        let r = reg();
        (r.paused, r.current)
    };
    let wanted = if settings.enabled && !paused {
        Some(check(app, &settings.shortcut)?.1)
    } else {
        None
    };
    if wanted == current {
        reg().error = None;
        return Ok(());
    }
    let gs = app.global_shortcut();
    if let Some(new) = wanted {
        if gs.is_registered(new) {
            reg().error = Some("CONFLICT_APP".into());
            return Err("CONFLICT_APP".into());
        }
        if let Err(e) = gs.on_shortcut(new, on_hotkey) {
            tracing::warn!(
                "[dictation] shortcut {} not registered: {e}",
                settings.shortcut
            );
            reg().error = Some("TAKEN".into());
            return Err("TAKEN".into());
        }
    }
    if let Some(old) = current {
        if let Err(e) = gs.unregister(old) {
            tracing::warn!("[dictation] unregister old shortcut: {e}");
        }
    }
    let mut r = reg();
    r.current = wanted;
    r.error = None;
    Ok(())
}

fn register_escape(app: &AppHandle) {
    let Ok(esc) = "Escape".parse::<Shortcut>() else {
        return;
    };
    let _serial = serial();
    if reg().esc.is_some() || state().live.is_none() {
        return;
    }
    let gs = app.global_shortcut();
    if gs.is_registered(esc) {
        return;
    }
    let res = gs.on_shortcut(esc, |app, _, event| {
        if event.state == ShortcutState::Pressed {
            let app = app.clone();
            tauri::async_runtime::spawn(async move {
                let session = state().live.as_ref().map(|l| l.session);
                if let Some(session) = session {
                    finish(app, session, Finish::Cancel).await;
                }
            });
        }
    });
    match res {
        Ok(()) => reg().esc = Some(esc),
        Err(e) => tracing::warn!("[dictation] Escape not registered: {e}"),
    }
}

fn unregister_escape(app: &AppHandle) {
    let _serial = serial();
    let esc = reg().esc.take();
    if let Some(esc) = esc {
        let _ = app.global_shortcut().unregister(esc);
    }
}

pub fn init(app: &AppHandle) {
    if INITIALIZED.swap(true, Ordering::SeqCst) {
        return;
    }
    let settings = current_settings();
    if let Err(code) = apply_shortcut(app, &settings) {
        tracing::warn!("[dictation] shortcut not active: {code}");
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HotkeyAction {
    Start,
    Finish(u64),
    DeferRelease,
    Ignore,
}

fn hotkey_action(
    mode: Mode,
    pressed: bool,
    live: Option<u64>,
    busy: bool,
    starting_from_shortcut: bool,
) -> HotkeyAction {
    match (mode, pressed, live) {
        (_, true, None) if !busy => HotkeyAction::Start,
        (Mode::Hold, false, Some(session)) | (Mode::Toggle, true, Some(session)) => {
            HotkeyAction::Finish(session)
        }
        (Mode::Hold, false, None) if busy && starting_from_shortcut => HotkeyAction::DeferRelease,
        _ => HotkeyAction::Ignore,
    }
}

async fn handle_hotkey(app: AppHandle, pressed: bool) {
    let settings = current_settings();
    let action = {
        let mut s = state();
        let action = hotkey_action(
            settings.mode,
            pressed,
            s.live.as_ref().map(|l| l.session),
            s.busy,
            s.start_source == Some(Source::Shortcut),
        );
        if action == HotkeyAction::DeferRelease {
            s.pending_release = true;
        }
        action
    };
    match action {
        HotkeyAction::Start => {
            let _ = start(&app, Source::Shortcut).await;
        }
        HotkeyAction::Finish(session) => finish(app, session, Finish::Stop).await,
        HotkeyAction::DeferRelease | HotkeyAction::Ignore => {}
    }
}

async fn ffmpeg_path() -> Option<PathBuf> {
    omniget_core::core::dependencies::find_tool("ffmpeg").await
}

async fn resolve_device(ffmpeg: &Path, settings: &Settings) -> Option<String> {
    let platform = Platform::current();
    if platform == Platform::Windows && settings.device.is_none() {
        return devices::list(ffmpeg)
            .await
            .ok()
            .and_then(|d| d.into_iter().next())
            .map(|d| d.id);
    }
    settings.device.clone()
}

async fn start(app: &AppHandle, source: Source) -> Result<StateEvent, String> {
    let settings = current_settings();
    let session = {
        let mut s = state();
        if s.live.is_some() || s.busy {
            return Err("BUSY".into());
        }
        s.busy = true;
        s.start_source = Some(source);
        s.pending_release = false;
        s.session += 1;
        s.session
    };
    let fail = |code: &str, detail: Option<String>| {
        {
            let mut s = state();
            s.busy = false;
            s.start_source = None;
            s.pending_release = false;
        }
        let ev = error_event(session, source, code, detail);
        emit_state(app, ev.clone());
        Err::<StateEvent, String>(code.to_string())
    };
    let st = asr::status();
    if !asr::is_ready(Some(&settings.model)) {
        let code = if !st.runtime_installed && !st.runtime_downloadable {
            "ASR_UNAVAILABLE"
        } else {
            "MODEL_MISSING"
        };
        return fail(code, None);
    }
    let Some(ffmpeg) = ffmpeg_path().await else {
        return fail("FFMPEG_MISSING", None);
    };
    let platform = Platform::current();
    let device = resolve_device(&ffmpeg, &settings).await;
    if platform == Platform::Windows && device.is_none() {
        return fail("NO_MICROPHONE", None);
    }
    let pulse = platform == Platform::Linux && which_pactl();
    let args = capture::capture_args(platform, device.as_deref(), settings.max_seconds, pulse);
    let max = Duration::from_secs(settings.max_seconds as u64);
    let level_app = app.clone();
    let ended_app = app.clone();
    let spawned = Capture::spawn(
        &ffmpeg,
        &args,
        max,
        move |level| {
            let _ = level_app.emit(
                LEVEL_EVENT,
                serde_json::json!({ "session": session, "level": level }),
            );
        },
        move || {
            tauri::async_runtime::spawn(async move {
                finish(ended_app, session, Finish::Auto).await;
            });
        },
    );
    let capture = match spawned {
        Ok(c) => c,
        Err(e) => return fail("MIC_FAILED", Some(e.to_string())),
    };
    let started_at = now_ms();
    let max_ms = max.as_millis() as u64;
    let released = {
        let mut s = state();
        s.busy = false;
        s.start_source = None;
        s.live = Some(Live {
            session,
            capture,
            source,
            started: Instant::now(),
            started_at,
            max_ms,
        });
        std::mem::take(&mut s.pending_release)
    };
    register_escape(app);
    let ev = StateEvent {
        phase: "listening",
        source,
        started_at: Some(started_at),
        max_ms,
        ..StateEvent::idle(session)
    };
    emit_state(app, ev.clone());
    if released {
        finish(app.clone(), session, Finish::Stop).await;
    }
    Ok(ev)
}

fn which_pactl() -> bool {
    let path = omniget_core::core::process::enhanced_path()
        .or_else(|| std::env::var("PATH").ok())
        .unwrap_or_default();
    std::env::split_paths(&path).any(|d| d.join("pactl").is_file())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Finish {
    Stop,
    Cancel,
    Auto,
}

async fn finish(app: AppHandle, session: u64, how: Finish) {
    let live = {
        let mut s = state();
        match s.live.as_ref() {
            Some(l) if l.session == session => {
                s.busy = how != Finish::Cancel;
                s.live.take()
            }
            _ => None,
        }
    };
    let Some(live) = live else {
        return;
    };
    unregister_escape(&app);
    let settings = current_settings();
    let too_quick = how == Finish::Stop
        && live.source == Source::Shortcut
        && settings.mode == Mode::Hold
        && live.started.elapsed() < HOLD_MIN;
    if how == Finish::Cancel || too_quick {
        let _ = live.capture.stop(Duration::ZERO).await;
        state().busy = false;
        emit_state(
            &app,
            StateEvent {
                notice: Some(if too_quick { "HOLD_HINT" } else { "CANCELLED" }),
                source: live.source,
                ..StateEvent::idle(session)
            },
        );
        return;
    }
    emit_state(
        &app,
        StateEvent {
            phase: "transcribing",
            source: live.source,
            started_at: Some(live.started_at),
            max_ms: live.max_ms,
            notice: (how == Finish::Auto).then_some("MAX_REACHED"),
            ..StateEvent::idle(session)
        },
    );
    let tail = if how == Finish::Auto {
        Duration::ZERO
    } else {
        TAIL
    };
    let result = match live.capture.stop(tail).await {
        Ok(rec) => process(&app, session, live.source, &settings, rec, how).await,
        Err(e) => Err(error_event(
            session,
            live.source,
            "MIC_FAILED",
            Some(e.to_string()),
        )),
    };
    state().busy = false;
    match result {
        Ok(ev) | Err(ev) => emit_state(&app, ev),
    }
}

async fn process(
    app: &AppHandle,
    session: u64,
    source: Source,
    settings: &Settings,
    rec: Recording,
    how: Finish,
) -> Result<StateEvent, StateEvent> {
    let err = |code: &str, detail: Option<String>| error_event(session, source, code, detail);
    if rec.samples.len() < MIN_SAMPLES {
        let failed_early = rec.ended_by_itself && how == Finish::Auto && !rec.stderr.is_empty();
        return Err(
            if failed_early || rec.samples.is_empty() && !rec.stderr.is_empty() {
                err("MIC_FAILED", Some(rec.stderr))
            } else {
                err("TOO_SHORT", None)
            },
        );
    }
    if capture::rms_dbfs(&rec.samples) < SILENCE_DBFS {
        return Err(err("SILENT", None));
    }
    let duration_ms = capture::duration_ms(rec.samples.len());
    let dir = std::env::temp_dir().join("omniget-dictation");
    let wav = capture::write_wav(
        &dir,
        &format!("take-{session}-{}.wav", now_ms()),
        &rec.samples,
    )
    .map_err(|e| err("TRANSCRIBE_FAILED", Some(e.to_string())))?;
    let lang = (settings.language != "auto").then(|| settings.language.clone());
    let transcript = asr::transcribe_input(
        wav.as_os_str(),
        Some(duration_ms),
        lang.as_deref(),
        Some(&settings.model),
        |_| {},
    )
    .await;
    let _ = std::fs::remove_file(&wav);
    let transcript = transcript.map_err(|e| err("TRANSCRIBE_FAILED", Some(e.to_string())))?;
    let language = lang.unwrap_or_else(|| transcript.lang.clone());
    let opts = PostProcess {
        language: &language,
        capitalize: settings.capitalize,
        remove_fillers: settings.remove_fillers,
        replacements: &settings.replacements,
    };
    let final_text = text::postprocess(&transcript.text, &opts);
    if final_text.trim().is_empty() {
        return Err(err("EMPTY_TEXT", None));
    }
    let mut warning: Option<ErrorInfo> = None;
    let outcome: &'static str = match (source, settings.output) {
        (Source::Page, _) => "inserted",
        (Source::Shortcut, Output::Copy) => {
            app.clipboard()
                .write_text(final_text.clone())
                .map_err(|e| err("CLIPBOARD_FAILED", Some(e.to_string())))?;
            "copied"
        }
        (Source::Shortcut, Output::Paste) => {
            match deliver(app, &final_text, settings.restore_clipboard).await {
                Ok(()) => "pasted",
                Err(e) => {
                    warning = Some(e);
                    "copied"
                }
            }
        }
    };
    let entry = Entry {
        id: uuid::Uuid::new_v4().to_string(),
        text: final_text,
        raw: transcript.text,
        created_at: now_ms(),
        duration_ms,
        language,
        model: transcript.model,
        outcome: outcome.to_string(),
    };
    if settings.save_history {
        if let Ok(dir) = data_dir() {
            let _g = STORE_LOCK.lock().unwrap_or_else(|p| p.into_inner());
            if let Err(e) = store::push_history(&dir, entry.clone()) {
                tracing::warn!("[dictation] history not saved: {e}");
            }
        }
    }
    Ok(StateEvent {
        phase: "done",
        source,
        entry: Some(entry),
        outcome: Some(outcome),
        error: warning,
        ..StateEvent::idle(session)
    })
}

async fn deliver(app: &AppHandle, text: &str, restore: bool) -> Result<(), ErrorInfo> {
    let clip = app.clipboard();
    let previous = if restore { clip.read_text().ok() } else { None };
    clip.write_text(text.to_string()).map_err(|e| ErrorInfo {
        code: "CLIPBOARD_FAILED".into(),
        detail: Some(e.to_string()),
    })?;
    tokio::time::sleep(Duration::from_millis(80)).await;
    if let Err(e) = paste::send_paste().await {
        return Err(ErrorInfo {
            code: e.code().into(),
            detail: Some(e.detail()).filter(|d| !d.is_empty()),
        });
    }
    if let Some(previous) = previous {
        tokio::time::sleep(Duration::from_millis(600)).await;
        let still_ours = clip.read_text().map(|t| t == text).unwrap_or(false);
        if still_ours {
            let _ = clip.write_text(previous);
        }
    }
    Ok(())
}

#[derive(Debug, Clone, Serialize)]
pub struct Status {
    pub settings: Settings,
    pub state: StateEvent,
    pub platform: &'static str,
    pub ffmpeg: bool,
    pub paste_supported: bool,
    pub runtime_installed: bool,
    pub runtime_downloadable: bool,
    pub model_ready: bool,
    pub models: Vec<asr::models::ModelStatus>,
    pub installing: bool,
    pub shortcut: ShortcutStatus,
    pub default_shortcut: &'static str,
}

async fn build_status() -> Status {
    let settings = current_settings();
    let st = asr::status();
    let last = state().last.clone();
    Status {
        shortcut: shortcut_status(&settings),
        model_ready: asr::is_ready(Some(&settings.model)),
        settings,
        state: last,
        platform: Platform::current().id(),
        ffmpeg: ffmpeg_path().await.is_some(),
        paste_supported: paste::paste_supported(),
        runtime_installed: st.runtime_installed,
        runtime_downloadable: st.runtime_downloadable,
        models: st.models,
        installing: INSTALLING.load(Ordering::SeqCst),
        default_shortcut: keys::default_shortcut(is_mac()),
    }
}

#[tauri::command]
pub async fn dictation_status(app: AppHandle) -> Status {
    init(&app);
    build_status().await
}

#[tauri::command]
pub async fn dictation_save_settings(app: AppHandle, settings: Settings) -> Result<Status, String> {
    init(&app);
    let mut next = settings.sanitized();
    let before = current_settings();
    if next.enabled || next.shortcut != before.shortcut {
        next.shortcut = check(&app, &next.shortcut)?.0;
    }
    apply_shortcut(&app, &next)?;
    let dir = data_dir()?;
    {
        let _g = STORE_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        store::save_settings(&dir, &next).map_err(|e| format!("SAVE_FAILED: {e}"))?;
    }
    if let Ok(mut g) = SETTINGS.write() {
        *g = Some(next);
    }
    Ok(build_status().await)
}

#[derive(Debug, Clone, Serialize)]
pub struct ShortcutCheck {
    pub normalized: Option<String>,
    pub error: Option<String>,
}

#[tauri::command]
pub fn dictation_check_shortcut(app: AppHandle, shortcut: String) -> ShortcutCheck {
    match check(&app, &shortcut) {
        Ok((normalized, _)) => ShortcutCheck {
            normalized: Some(normalized),
            error: None,
        },
        Err(code) => ShortcutCheck {
            normalized: None,
            error: Some(code),
        },
    }
}

#[tauri::command]
pub async fn dictation_pause_shortcut(app: AppHandle, paused: bool) -> ShortcutStatus {
    reg().paused = paused;
    let settings = current_settings();
    if let Err(code) = apply_shortcut(&app, &settings) {
        tracing::warn!("[dictation] shortcut not restored: {code}");
    }
    shortcut_status(&settings)
}

#[tauri::command]
pub async fn dictation_devices() -> Result<Vec<Device>, String> {
    let Some(ffmpeg) = ffmpeg_path().await else {
        return Err("FFMPEG_MISSING".into());
    };
    devices::list(&ffmpeg).await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn dictation_start(app: AppHandle) -> Result<StateEvent, String> {
    init(&app);
    start(&app, Source::Page).await
}

#[tauri::command]
pub async fn dictation_stop(app: AppHandle) {
    let session = state().live.as_ref().map(|l| l.session);
    if let Some(session) = session {
        finish(app, session, Finish::Stop).await;
    }
}

#[tauri::command]
pub async fn dictation_cancel(app: AppHandle) {
    let session = state().live.as_ref().map(|l| l.session);
    if let Some(session) = session {
        finish(app, session, Finish::Cancel).await;
    }
}

#[tauri::command]
pub fn dictation_history() -> Vec<Entry> {
    let Ok(dir) = data_dir() else {
        return Vec::new();
    };
    let _g = STORE_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    store::load_history(&dir)
}

#[tauri::command]
pub fn dictation_history_delete(id: String) -> Result<Vec<Entry>, String> {
    let dir = data_dir()?;
    let _g = STORE_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    store::delete_history(&dir, &id).map_err(|e| format!("SAVE_FAILED: {e}"))
}

#[tauri::command]
pub fn dictation_history_clear() -> Result<(), String> {
    let dir = data_dir()?;
    let _g = STORE_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    store::clear_history(&dir).map_err(|e| format!("SAVE_FAILED: {e}"))
}

#[tauri::command]
pub fn dictation_copy(app: AppHandle, text: String) -> Result<(), String> {
    app.clipboard()
        .write_text(text)
        .map_err(|e| format!("CLIPBOARD_FAILED: {e}"))
}

#[tauri::command]
pub fn dictation_preview(
    text: String,
    settings: Settings,
    fallback_language: Option<String>,
) -> String {
    let settings = settings.sanitized();
    let language = preview_language(&settings.language, fallback_language.as_deref());
    text::postprocess(
        &text,
        &PostProcess {
            language: &language,
            capitalize: settings.capitalize,
            remove_fillers: settings.remove_fillers,
            replacements: &settings.replacements,
        },
    )
}

fn preview_language(chosen: &str, fallback: Option<&str>) -> String {
    if chosen != "auto" {
        return chosen.to_string();
    }
    fallback
        .map(str::trim)
        .filter(|l| !l.is_empty() && *l != "auto")
        .unwrap_or("en")
        .to_string()
}

#[tauri::command]
pub async fn dictation_open_privacy(pane: String) -> Result<(), String> {
    let platform = Platform::current();
    let url = paste::privacy_url(platform, &pane).ok_or_else(|| "UNSUPPORTED".to_string())?;
    let mut cmd = match platform {
        Platform::Windows => {
            let mut c = omniget_core::core::process::command("explorer");
            c.arg(url);
            c
        }
        _ => {
            let mut c = omniget_core::core::process::command("open");
            c.arg(url);
            c
        }
    };
    cmd.status().await.map(|_| ()).map_err(|e| e.to_string())
}

#[derive(Debug, Clone, Serialize)]
struct ModelProgress<'a> {
    model: &'a str,
    stage: &'static str,
    fraction: f64,
}

#[tauri::command]
pub async fn dictation_install_model(
    app: AppHandle,
    model: Option<String>,
) -> Result<Status, String> {
    if INSTALLING.swap(true, Ordering::SeqCst) {
        return Err("BUSY".into());
    }
    let model = model.unwrap_or_else(|| current_settings().model);
    let result = install(&app, &model).await;
    INSTALLING.store(false, Ordering::SeqCst);
    result?;
    Ok(build_status().await)
}

async fn install(app: &AppHandle, model: &str) -> Result<(), String> {
    let spec = asr::models::pick(Some(model)).map_err(|e| format!("MODEL_UNKNOWN: {e}"))?;
    let st = asr::status();
    if !st.runtime_installed && !st.runtime_downloadable {
        return Err("ASR_UNAVAILABLE".into());
    }
    let total = asr::models::total_bytes(spec).max(1);
    let seen: Arc<Mutex<HashMap<String, u64>>> = Arc::new(Mutex::new(HashMap::new()));
    let app2 = app.clone();
    let id = spec.id.to_string();
    let last = Arc::new(Mutex::new(-1.0f64));
    let progress: omniget_core::core::tools::ProgressFn = Arc::new(move |p| {
        let (stage, fraction) = if p.id.starts_with(&id) {
            let mut m = seen.lock().unwrap_or_else(|e| e.into_inner());
            m.insert(p.id.clone(), p.done);
            let done: u64 = m.values().sum();
            ("model", (done as f64 / total as f64).min(1.0))
        } else {
            let f = p
                .total
                .filter(|t| *t > 0)
                .map(|t| p.done as f64 / t as f64)
                .unwrap_or(0.0);
            ("runtime", f.min(1.0))
        };
        let rounded = (fraction * 100.0).floor();
        let mut l = last.lock().unwrap_or_else(|e| e.into_inner());
        if rounded != *l {
            *l = rounded;
            let _ = app2.emit(
                MODEL_EVENT,
                ModelProgress {
                    model: &id,
                    stage,
                    fraction,
                },
            );
        }
    });
    asr::install(Some(spec.id), &progress)
        .await
        .map(|_| ())
        .map_err(|e| format!("MODEL_DOWNLOAD_FAILED: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hold_release_during_start_is_deferred_not_dropped() {
        assert_eq!(
            hotkey_action(Mode::Hold, false, None, true, true),
            HotkeyAction::DeferRelease
        );
        assert_eq!(
            hotkey_action(Mode::Hold, false, None, true, false),
            HotkeyAction::Ignore
        );
        assert_eq!(
            hotkey_action(Mode::Hold, false, None, false, false),
            HotkeyAction::Ignore
        );
    }

    #[test]
    fn hotkey_starts_and_finishes_by_mode() {
        assert_eq!(
            hotkey_action(Mode::Hold, true, None, false, false),
            HotkeyAction::Start
        );
        assert_eq!(
            hotkey_action(Mode::Hold, true, None, true, true),
            HotkeyAction::Ignore
        );
        assert_eq!(
            hotkey_action(Mode::Hold, false, Some(3), false, false),
            HotkeyAction::Finish(3)
        );
        assert_eq!(
            hotkey_action(Mode::Toggle, true, Some(4), false, false),
            HotkeyAction::Finish(4)
        );
        assert_eq!(
            hotkey_action(Mode::Toggle, false, Some(4), false, false),
            HotkeyAction::Ignore
        );
    }

    #[test]
    fn preview_uses_fallback_only_on_auto() {
        assert_eq!(preview_language("pt", Some("en")), "pt");
        assert_eq!(preview_language("auto", Some("pt-BR")), "pt-BR");
        assert_eq!(preview_language("auto", Some(" ")), "en");
        assert_eq!(preview_language("auto", None), "en");
    }
}

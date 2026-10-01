//! Settings schema 3: the public DTO without secrets, strict patch
//! validation, secret operations, scoped reset and the one-time appearance
//! import. Pure functions over [`AppSettings`]; persistence, revisions and
//! effects belong to the desktop repository.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use super::{
    AccessibilitySettings, AppSettings, CanvasSettings, DownloadSettings, LastDownloadOptions,
    LeagueSettings, LlmSettings, MusicSettings, ReadSettings, RpcSettings, TelegramSettings,
    TubeSettings, TypographySettings, VideoSettings, WashMyBellySettings, WorldSettings,
};

/// Current `schema_version` of `AppSettings`.
pub const SETTINGS_SCHEMA_VERSION: u32 = 3;

/// Visual preset of a new install.
pub const DEFAULT_PRESET: &str = "omniget-desktop";

/// The locales the app ships a catalog for.
pub const SUPPORTED_LANGUAGES: [&str; 12] = [
    "en", "pt", "es", "fr", "it", "ru", "el", "fa", "lo", "ja", "zh", "zh-TW",
];

/// Workspace presets of the legacy `omniget.workspace.appearance.v1` key.
pub const WORKSPACE_PRESETS: [&str; 6] = [
    "amber-minimal",
    "orange-soft",
    "claude-amber",
    "amber-hearth",
    "amber-slate",
    "orange-vivid",
];

/// Named themes of "More styles": selecting one makes it the preset and the
/// legacy `theme` at the same time.
pub const NAMED_THEMES: [&str; 12] = [
    "catppuccin-latte",
    "catppuccin-frappe",
    "catppuccin-macchiato",
    "catppuccin-mocha",
    "one-dark-pro",
    "dracula",
    "nyxvamp-veil",
    "nyxvamp-obsidian",
    "nyxvamp-radiance",
    "eink-day",
    "eink-sepia",
    "eink-night",
];

pub const MODES: [&str; 3] = ["system", "light", "dark"];

/// The only source the appearance import accepts.
pub const WORKSPACE_APPEARANCE_SOURCE: &str = "omniget.workspace.appearance.v1";
/// Largest `rawWorkspace` the import reads, in UTF-8 bytes.
pub const WORKSPACE_APPEARANCE_MAX_BYTES: usize = 65_536;

const PROXY_TYPES: [&str; 3] = ["http", "https", "socks5"];

pub fn is_named_theme(id: &str) -> bool {
    NAMED_THEMES.contains(&id)
}

fn is_known_preset(id: &str) -> bool {
    id == DEFAULT_PRESET || WORKSPACE_PRESETS.contains(&id) || is_named_theme(id)
}

fn derive_mode(theme: &str) -> String {
    if MODES.contains(&theme) {
        theme.to_string()
    } else {
        "system".to_string()
    }
}

fn derive_preset(theme: &str) -> String {
    if is_named_theme(theme) {
        theme.to_string()
    } else {
        DEFAULT_PRESET.to_string()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SettingsSection {
    General,
    Appearance,
    Downloads,
    Ai,
    Integrations,
    Advanced,
}

impl SettingsSection {
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "general" => Self::General,
            "appearance" => Self::Appearance,
            "downloads" => Self::Downloads,
            "ai" => Self::Ai,
            "integrations" => Self::Integrations,
            "advanced" => Self::Advanced,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FieldErrorCode {
    UnknownField,
    ProtectedField,
    InvalidType,
    OutOfRange,
    InvalidEnum,
    InvalidValue,
    Conflict,
    Required,
}

/// A schema path and a stable code. Never carries the submitted value.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FieldError {
    pub path: String,
    pub code: FieldErrorCode,
}

impl FieldError {
    pub fn new(path: impl Into<String>, code: FieldErrorCode) -> Self {
        Self {
            path: path.into(),
            code,
        }
    }
}

// ---------------------------------------------------------------------------
// Public DTO
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct AppearancePublic {
    pub theme: String,
    pub language: String,
    pub sidebar_collapsed: bool,
    pub mode: String,
    pub preset: String,
    pub legacy_theme: Option<String>,
    pub migration_version: u32,
}

#[derive(Debug, Clone, Serialize)]
pub struct AdvancedPublic {
    pub max_concurrent_segments: u32,
    pub max_retries: u32,
    pub max_concurrent_downloads: u32,
    pub concurrent_fragments: u32,
    pub stagger_delay_ms: u64,
    pub torrent_listen_port: u16,
    pub torrent_auto_trackers: bool,
    pub torrent_upnp: bool,
    pub prevent_sleep: bool,
    pub cookies_from_browser: String,
    pub has_twitter_manual_cookie: bool,
    pub user_agent: String,
    pub insecure_tls: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct ProxyPublic {
    pub enabled: bool,
    pub proxy_type: String,
    pub host: String,
    pub port: u16,
    pub username: String,
    pub has_password: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct BridgePublic {
    pub enabled: bool,
    pub port: u16,
    pub has_token: bool,
    pub legacy_agent_routes: bool,
}

/// What the UI may see of the settings: every known field except the proxy
/// password, the manual X cookie and the bridge token, which become `has_*`
/// booleans. Unknown keys of the file never reach it.
#[derive(Debug, Clone, Serialize)]
pub struct SettingsPublicV3 {
    pub schema_version: u32,
    pub appearance: AppearancePublic,
    pub download: DownloadSettings,
    pub advanced: AdvancedPublic,
    pub telegram: TelegramSettings,
    pub proxy: ProxyPublic,
    pub onboarding_completed: bool,
    pub start_with_system: bool,
    pub start_minimized: bool,
    pub exit_on_close: bool,
    pub run_in_background: bool,
    pub portable_mode: bool,
    pub legal_acknowledged: bool,
    pub last_download_options: LastDownloadOptions,
    pub typography: TypographySettings,
    pub rpc: RpcSettings,
    pub bridge: BridgePublic,
    pub league: LeagueSettings,
    pub music: MusicSettings,
    pub video: VideoSettings,
    pub tube: TubeSettings,
    pub washmybelly: WashMyBellySettings,
    pub read: ReadSettings,
    pub accessibility: AccessibilitySettings,
    pub world: WorldSettings,
    pub canvas: CanvasSettings,
    pub llm: LlmSettings,
}

impl AppSettings {
    /// Defaults of a profile created by this version. Differs from
    /// [`AppSettings::default`] (which old profiles fall back to field by
    /// field) only in the visual preset and its typography.
    pub fn fresh_install() -> Self {
        let mut s = Self::default();
        s.appearance.mode = Some("system".into());
        s.appearance.preset = Some(DEFAULT_PRESET.into());
        s.typography = TypographySettings {
            font_display: "system-ui".into(),
            font_body: "system-ui".into(),
            font_mono: "ui-monospace".into(),
            line_height_base: 1.55,
            spacing_scale: 1.0,
            preset_name: Some(DEFAULT_PRESET.into()),
        };
        normalize_appearance(&mut s);
        s
    }

    pub fn to_public(&self) -> SettingsPublicV3 {
        let a = &self.appearance;
        let adv = &self.advanced;
        SettingsPublicV3 {
            schema_version: self.schema_version,
            appearance: AppearancePublic {
                theme: a.theme.clone(),
                language: a.language.clone(),
                sidebar_collapsed: a.sidebar_collapsed,
                mode: a.mode.clone().unwrap_or_else(|| derive_mode(&a.theme)),
                preset: a.preset.clone().unwrap_or_else(|| derive_preset(&a.theme)),
                legacy_theme: a.legacy_theme.clone(),
                migration_version: a.migration_version,
            },
            download: self.download.clone(),
            advanced: AdvancedPublic {
                max_concurrent_segments: adv.max_concurrent_segments,
                max_retries: adv.max_retries,
                max_concurrent_downloads: adv.max_concurrent_downloads,
                concurrent_fragments: adv.concurrent_fragments,
                stagger_delay_ms: adv.stagger_delay_ms,
                torrent_listen_port: adv.torrent_listen_port,
                torrent_auto_trackers: adv.torrent_auto_trackers,
                torrent_upnp: adv.torrent_upnp,
                prevent_sleep: adv.prevent_sleep,
                cookies_from_browser: adv.cookies_from_browser.clone(),
                has_twitter_manual_cookie: !adv.twitter_manual_cookie.is_empty(),
                user_agent: adv.user_agent.clone(),
                insecure_tls: adv.insecure_tls,
            },
            telegram: self.telegram.clone(),
            proxy: ProxyPublic {
                enabled: self.proxy.enabled,
                proxy_type: self.proxy.proxy_type.clone(),
                host: self.proxy.host.clone(),
                port: self.proxy.port,
                username: self.proxy.username.clone(),
                has_password: !self.proxy.password.is_empty(),
            },
            onboarding_completed: self.onboarding_completed,
            start_with_system: self.start_with_system,
            start_minimized: self.start_minimized,
            exit_on_close: self.exit_on_close,
            run_in_background: self.run_in_background,
            portable_mode: self.portable_mode,
            legal_acknowledged: self.legal_acknowledged,
            last_download_options: self.last_download_options.clone(),
            typography: self.typography.clone(),
            rpc: self.rpc.clone(),
            bridge: BridgePublic {
                enabled: self.bridge.enabled,
                port: self.bridge.port,
                has_token: !self.bridge.token.is_empty(),
                legacy_agent_routes: self.bridge.legacy_agent_routes,
            },
            league: self.league.clone(),
            music: self.music.clone(),
            video: self.video.clone(),
            tube: self.tube.clone(),
            washmybelly: self.washmybelly.clone(),
            read: self.read.clone(),
            accessibility: self.accessibility.clone(),
            world: self.world.clone(),
            canvas: self.canvas.clone(),
            llm: self.llm.clone(),
        }
    }
}

/// Keeps the legacy `theme` coherent with `mode`/`preset` once both exist:
/// a named theme is its own `theme`, anything else stores the mode.
pub fn normalize_appearance(s: &mut AppSettings) {
    let a = &mut s.appearance;
    if let (Some(mode), Some(preset)) = (a.mode.as_ref(), a.preset.as_ref()) {
        a.theme = if is_named_theme(preset) {
            preset.clone()
        } else {
            mode.clone()
        };
    }
}

// ---------------------------------------------------------------------------
// JSON paths
// ---------------------------------------------------------------------------

pub fn get_path<'a>(v: &'a Value, path: &str) -> Option<&'a Value> {
    path.split('.').try_fold(v, |cur, key| cur.get(key))
}

pub fn set_path(v: &mut Value, path: &str, new: Value) {
    let mut cur = v;
    let mut parts = path.split('.').peekable();
    while let Some(key) = parts.next() {
        if !cur.is_object() {
            *cur = Value::Object(Map::new());
        }
        let obj = cur.as_object_mut().expect("object");
        if parts.peek().is_none() {
            obj.insert(key.to_string(), new);
            return;
        }
        cur = obj
            .entry(key.to_string())
            .or_insert_with(|| Value::Object(Map::new()));
    }
}

// ---------------------------------------------------------------------------
// Patch
// ---------------------------------------------------------------------------

/// Fields a patch may never carry: secrets (they go through
/// `secretOperations` or dedicated commands), managed metadata and the
/// migration marker.
const PROTECTED: [&str; 7] = [
    "schema_version",
    "appearance.migration_version",
    "appearance.legacy_theme",
    "proxy.password",
    "advanced.twitter_manual_cookie",
    "bridge.token",
    "bridge.port",
];

/// Visual fields locked until the appearance import committed.
const APPEARANCE_GATED: [&str; 3] = ["appearance.theme", "appearance.mode", "appearance.preset"];

const FINITE_FLOATS: [&str; 3] = [
    "typography.line_height_base",
    "typography.spacing_scale",
    "world.measured_median_ms",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PatchError {
    Invalid(Vec<FieldError>),
    /// Theme/mode/preset edited before `appearance.migration_version >= 1`.
    AppearanceImportRequired,
}

/// Every known field with a representative non-null value, so optional
/// fields are known keys with a known JSON kind.
fn schema_template() -> Value {
    let mut s = AppSettings::default();
    s.appearance.mode = Some(String::new());
    s.appearance.preset = Some(String::new());
    s.appearance.legacy_theme = Some(String::new());
    s.appearance.migration_version = 1;
    s.last_download_options.mode = Some(String::new());
    s.last_download_options.quality = Some(String::new());
    s.typography.preset_name = Some(String::new());
    s.world.tier_override = Some(0);
    s.world.tier_measured = Some(0);
    s.world.backend_override = Some(String::new());
    s.world.measured_median_ms = Some(0.5);
    s.world.measured_app_version = Some(String::new());
    s.llm.ask_timeout_job_s = Some(0);
    s.llm.jobs_max_per_agent = Some(0);
    serde_json::to_value(s).expect("settings serialize")
}

fn collect_leaves(
    patch: &Map<String, Value>,
    template: &Value,
    prefix: &str,
    errors: &mut Vec<FieldError>,
    leaves: &mut Vec<(String, Value)>,
) {
    for (key, value) in patch {
        let path = if prefix.is_empty() {
            key.clone()
        } else {
            format!("{prefix}.{key}")
        };
        if PROTECTED.contains(&path.as_str()) || key.starts_with("has_") {
            errors.push(FieldError::new(path, FieldErrorCode::ProtectedField));
            continue;
        }
        let Some(slot) = template.get(key) else {
            errors.push(FieldError::new(path, FieldErrorCode::UnknownField));
            continue;
        };
        match (slot, value) {
            (Value::Object(_), Value::Object(inner)) => {
                collect_leaves(inner, slot, &path, errors, leaves)
            }
            (Value::Object(_), _) | (_, Value::Object(_)) => {
                errors.push(FieldError::new(path, FieldErrorCode::InvalidType))
            }
            _ => leaves.push((path, value.clone())),
        }
    }
}

fn type_error_code(template: Option<&Value>, value: &Value) -> FieldErrorCode {
    match (template, value) {
        (Some(Value::Number(t)), Value::Number(v)) => {
            let integral_template = t.is_u64() || t.is_i64();
            let integral_value = v.is_u64() || v.is_i64();
            if integral_template && !integral_value {
                FieldErrorCode::InvalidType
            } else {
                FieldErrorCode::OutOfRange
            }
        }
        _ => FieldErrorCode::InvalidType,
    }
}

fn str_at(v: &Value, path: &str) -> Option<String> {
    get_path(v, path)
        .and_then(|x| x.as_str())
        .map(str::to_string)
}

/// Validates `patch` (an object of known public fields, snake_case paths)
/// against `current` and returns the next settings. Nothing is written:
/// an invalid patch leaves the caller's copy untouched.
///
/// Old values that are valid for their Rust type are never re-validated;
/// product ranges apply only to the fields this patch touches.
pub fn apply_patch(current: &AppSettings, patch: &Value) -> Result<AppSettings, PatchError> {
    let Value::Object(map) = patch else {
        return Err(PatchError::Invalid(vec![FieldError::new(
            "",
            FieldErrorCode::InvalidType,
        )]));
    };
    let template = schema_template();
    let mut errors = Vec::new();
    let mut leaves = Vec::new();
    collect_leaves(map, &template, "", &mut errors, &mut leaves);
    if !errors.is_empty() {
        return Err(PatchError::Invalid(errors));
    }
    if current.appearance.migration_version < 1
        && leaves
            .iter()
            .any(|(p, _)| APPEARANCE_GATED.contains(&p.as_str()))
    {
        return Err(PatchError::AppearanceImportRequired);
    }

    let base = serde_json::to_value(current).expect("settings serialize");
    for (path, value) in &leaves {
        let mut candidate = base.clone();
        set_path(&mut candidate, path, value.clone());
        if serde_json::from_value::<AppSettings>(candidate).is_err() {
            errors.push(FieldError::new(
                path.clone(),
                type_error_code(get_path(&template, path), value),
            ));
        }
    }
    if !errors.is_empty() {
        return Err(PatchError::Invalid(errors));
    }

    let mut merged = base;
    for (path, value) in &leaves {
        set_path(&mut merged, path, value.clone());
    }
    let mut next: AppSettings = serde_json::from_value(merged.clone()).map_err(|_| {
        PatchError::Invalid(vec![FieldError::new("", FieldErrorCode::InvalidValue)])
    })?;
    let touched = |p: &str| leaves.iter().any(|(path, _)| path == p);
    let touched_prefix = |p: &str| leaves.iter().any(|(path, _)| path.starts_with(p));

    for path in FINITE_FLOATS {
        if touched(path) {
            if let Some(f) = get_path(&merged, path).and_then(Value::as_f64) {
                // f32 fields turn a huge JSON number into infinity.
                let finite = match path {
                    "world.measured_median_ms" => f.is_finite(),
                    _ => f.is_finite() && (f as f32).is_finite(),
                };
                if !finite {
                    errors.push(FieldError::new(path, FieldErrorCode::OutOfRange));
                }
            }
        }
    }
    if touched("advanced.max_concurrent_downloads")
        && !(1..=10).contains(&next.advanced.max_concurrent_downloads)
    {
        errors.push(FieldError::new(
            "advanced.max_concurrent_downloads",
            FieldErrorCode::OutOfRange,
        ));
    }
    if touched("proxy.proxy_type") && !PROXY_TYPES.contains(&next.proxy.proxy_type.as_str()) {
        errors.push(FieldError::new(
            "proxy.proxy_type",
            FieldErrorCode::InvalidEnum,
        ));
    }
    if touched_prefix("proxy.") && next.proxy.enabled {
        if !touched("proxy.proxy_type") && !PROXY_TYPES.contains(&next.proxy.proxy_type.as_str()) {
            errors.push(FieldError::new(
                "proxy.proxy_type",
                FieldErrorCode::InvalidEnum,
            ));
        }
        if next.proxy.port == 0 {
            errors.push(FieldError::new("proxy.port", FieldErrorCode::OutOfRange));
        }
        if next.proxy.host.trim().is_empty() {
            errors.push(FieldError::new("proxy.host", FieldErrorCode::Required));
        }
    }
    if touched("appearance.language")
        && !SUPPORTED_LANGUAGES.contains(&next.appearance.language.as_str())
    {
        errors.push(FieldError::new(
            "appearance.language",
            FieldErrorCode::InvalidEnum,
        ));
    }
    if touched("appearance.mode") {
        let ok = str_at(&merged, "appearance.mode").is_some_and(|m| MODES.contains(&m.as_str()));
        if !ok {
            errors.push(FieldError::new(
                "appearance.mode",
                FieldErrorCode::InvalidEnum,
            ));
        }
    }
    if touched("appearance.preset") {
        let ok = str_at(&merged, "appearance.preset").is_some_and(|p| is_known_preset(&p));
        if !ok {
            errors.push(FieldError::new(
                "appearance.preset",
                FieldErrorCode::InvalidEnum,
            ));
        }
    }
    if touched("appearance.theme") {
        let t = next.appearance.theme.as_str();
        if !(MODES.contains(&t) || is_named_theme(t)) {
            errors.push(FieldError::new(
                "appearance.theme",
                FieldErrorCode::InvalidEnum,
            ));
        }
    }
    if !errors.is_empty() {
        return Err(PatchError::Invalid(errors));
    }

    // A legacy writer that only knows `theme` moves the schema 3 fields.
    if touched("appearance.theme") && !touched("appearance.mode") && !touched("appearance.preset") {
        let theme = next.appearance.theme.clone();
        let a = &mut next.appearance;
        if MODES.contains(&theme.as_str()) {
            a.mode = Some(theme);
            if a.preset.as_deref().is_none_or(is_named_theme) {
                a.preset = Some(DEFAULT_PRESET.into());
            }
        } else {
            a.preset = Some(theme);
            if a.mode.is_none() {
                a.mode = Some("system".into());
            }
        }
    }
    normalize_appearance(&mut next);
    Ok(next)
}

// ---------------------------------------------------------------------------
// Secret operations
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum SecretOperation {
    #[default]
    Keep,
    Set(String),
    Remove,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SecretOperations {
    pub proxy_password: SecretOperation,
    pub twitter_manual_cookie: SecretOperation,
}

impl SecretOperations {
    pub fn is_noop(&self) -> bool {
        self.proxy_password == SecretOperation::Keep
            && self.twitter_manual_cookie == SecretOperation::Keep
    }
}

fn parse_secret_operation(v: &Value, path: &str) -> Result<SecretOperation, FieldError> {
    let Value::Object(obj) = v else {
        return Err(FieldError::new(path, FieldErrorCode::InvalidType));
    };
    let op = match obj.get("op") {
        Some(Value::String(op)) => op.as_str(),
        Some(_) => {
            return Err(FieldError::new(
                format!("{path}.op"),
                FieldErrorCode::InvalidType,
            ))
        }
        None => {
            return Err(FieldError::new(
                format!("{path}.op"),
                FieldErrorCode::Required,
            ))
        }
    };
    let allowed: &[&str] = match op {
        "keep" | "remove" => &["op"],
        "set" => &["op", "value"],
        _ => {
            return Err(FieldError::new(
                format!("{path}.op"),
                FieldErrorCode::InvalidEnum,
            ))
        }
    };
    if let Some(extra) = obj.keys().find(|k| !allowed.contains(&k.as_str())) {
        return Err(FieldError::new(
            format!("{path}.{extra}"),
            FieldErrorCode::UnknownField,
        ));
    }
    Ok(match op {
        "keep" => SecretOperation::Keep,
        "remove" => SecretOperation::Remove,
        _ => match obj.get("value") {
            Some(Value::String(s)) if !s.is_empty() => SecretOperation::Set(s.clone()),
            Some(Value::String(_)) => {
                return Err(FieldError::new(
                    format!("{path}.value"),
                    FieldErrorCode::InvalidValue,
                ))
            }
            Some(_) => {
                return Err(FieldError::new(
                    format!("{path}.value"),
                    FieldErrorCode::InvalidType,
                ))
            }
            None => {
                return Err(FieldError::new(
                    format!("{path}.value"),
                    FieldErrorCode::Required,
                ))
            }
        },
    })
}

/// Parses `secretOperations` (absent or null = keep everything).
pub fn parse_secret_operations(v: Option<&Value>) -> Result<SecretOperations, Vec<FieldError>> {
    let mut ops = SecretOperations::default();
    let obj = match v {
        None | Some(Value::Null) => return Ok(ops),
        Some(Value::Object(obj)) => obj,
        Some(_) => {
            return Err(vec![FieldError::new(
                "secretOperations",
                FieldErrorCode::InvalidType,
            )])
        }
    };
    let mut errors = Vec::new();
    for (key, value) in obj {
        let path = format!("secretOperations.{key}");
        let slot = match key.as_str() {
            "proxyPassword" => &mut ops.proxy_password,
            "twitterManualCookie" => &mut ops.twitter_manual_cookie,
            _ => {
                errors.push(FieldError::new(path, FieldErrorCode::UnknownField));
                continue;
            }
        };
        match parse_secret_operation(value, &path) {
            Ok(op) => *slot = op,
            Err(e) => errors.push(e),
        }
    }
    if errors.is_empty() {
        Ok(ops)
    } else {
        Err(errors)
    }
}

pub fn apply_secret_operations(s: &mut AppSettings, ops: &SecretOperations) {
    fn apply(slot: &mut String, op: &SecretOperation) {
        match op {
            SecretOperation::Keep => {}
            SecretOperation::Set(v) => *slot = v.clone(),
            SecretOperation::Remove => slot.clear(),
        }
    }
    apply(&mut s.proxy.password, &ops.proxy_password);
    apply(
        &mut s.advanced.twitter_manual_cookie,
        &ops.twitter_manual_cookie,
    );
}

// ---------------------------------------------------------------------------
// Reset
// ---------------------------------------------------------------------------

/// The exact allowlist of `reset-policy.json`, by key. Anything not listed
/// (secrets, bridge token and port, consents, enablement of integrations,
/// derived calibration, unknown keys) survives every reset.
pub const RESET_ALLOWLIST: [(&str, SettingsSection); 127] = [
    ("appearance.theme", SettingsSection::Appearance),
    ("appearance.language", SettingsSection::General),
    ("download.default_output_dir", SettingsSection::Downloads),
    ("download.saved_output_dirs", SettingsSection::Downloads),
    ("download.always_ask_path", SettingsSection::Downloads),
    ("download.video_quality", SettingsSection::Downloads),
    ("download.skip_existing", SettingsSection::Downloads),
    ("download.download_descriptions", SettingsSection::Downloads),
    ("download.embed_metadata", SettingsSection::Downloads),
    ("download.embed_thumbnail", SettingsSection::Downloads),
    ("download.write_nfo_sidecar", SettingsSection::Downloads),
    ("download.clipboard_detection", SettingsSection::Downloads),
    (
        "download.auto_download_on_paste",
        SettingsSection::Downloads,
    ),
    ("download.filename_template", SettingsSection::Downloads),
    ("download.organize_by_platform", SettingsSection::Downloads),
    ("download.download_subtitles", SettingsSection::Downloads),
    (
        "download.include_auto_subtitles",
        SettingsSection::Downloads,
    ),
    ("download.caption_locale", SettingsSection::Downloads),
    ("download.keep_vtt", SettingsSection::Downloads),
    ("download.translate_metadata", SettingsSection::Downloads),
    ("download.youtube_sponsorblock", SettingsSection::Downloads),
    ("download.sponsorblock_mode", SettingsSection::Downloads),
    (
        "download.sponsorblock_categories",
        SettingsSection::Downloads,
    ),
    ("download.split_by_chapters", SettingsSection::Downloads),
    ("download.live_from_start", SettingsSection::Downloads),
    ("download.speed_limit", SettingsSection::Downloads),
    ("download.hotkey_enabled", SettingsSection::Downloads),
    ("download.hotkey_binding", SettingsSection::Downloads),
    ("download.clip_hotkey_enabled", SettingsSection::Downloads),
    ("download.clip_hotkey_binding", SettingsSection::Downloads),
    ("download.music_hotkey_enabled", SettingsSection::Downloads),
    ("download.music_hotkey_binding", SettingsSection::Downloads),
    ("download.music_audio_format", SettingsSection::Downloads),
    ("download.extra_ytdlp_flags", SettingsSection::Downloads),
    (
        "download.copy_to_clipboard_on_hotkey",
        SettingsSection::Downloads,
    ),
    (
        "download.always_use_managed_cookies",
        SettingsSection::Downloads,
    ),
    (
        "download.continuous_lecture_numbers",
        SettingsSection::Downloads,
    ),
    (
        "download.bilibili_danmaku_enabled",
        SettingsSection::Downloads,
    ),
    (
        "download.bilibili_danmaku_format",
        SettingsSection::Downloads,
    ),
    ("download.bilibili_container", SettingsSection::Downloads),
    ("download.bilibili_nfo_enabled", SettingsSection::Downloads),
    (
        "download.bilibili_cover_sidecar",
        SettingsSection::Downloads,
    ),
    ("download.bilibili_cover_format", SettingsSection::Downloads),
    ("download.bilibili_naming_video", SettingsSection::Downloads),
    (
        "download.bilibili_naming_multi_part",
        SettingsSection::Downloads,
    ),
    (
        "download.bilibili_naming_bangumi",
        SettingsSection::Downloads,
    ),
    (
        "download.bilibili_naming_cheese",
        SettingsSection::Downloads,
    ),
    (
        "download.bilibili_naming_collection",
        SettingsSection::Downloads,
    ),
    ("download.bilibili_cdn_hosts", SettingsSection::Downloads),
    (
        "download.bilibili_cdn_prefer_alternatives",
        SettingsSection::Downloads,
    ),
    ("download.bilibili_preferred_qn", SettingsSection::Downloads),
    (
        "download.bilibili_preferred_codec",
        SettingsSection::Downloads,
    ),
    (
        "download.bilibili_preferred_audio_qn",
        SettingsSection::Downloads,
    ),
    (
        "advanced.max_concurrent_segments",
        SettingsSection::Advanced,
    ),
    ("advanced.max_retries", SettingsSection::Advanced),
    (
        "advanced.max_concurrent_downloads",
        SettingsSection::Advanced,
    ),
    ("advanced.concurrent_fragments", SettingsSection::Advanced),
    ("advanced.stagger_delay_ms", SettingsSection::Advanced),
    ("advanced.torrent_listen_port", SettingsSection::Advanced),
    ("advanced.torrent_auto_trackers", SettingsSection::Advanced),
    ("advanced.torrent_upnp", SettingsSection::Advanced),
    ("advanced.prevent_sleep", SettingsSection::Advanced),
    ("advanced.user_agent", SettingsSection::Advanced),
    ("advanced.insecure_tls", SettingsSection::Advanced),
    ("telegram.concurrent_downloads", SettingsSection::Advanced),
    ("telegram.fix_file_extensions", SettingsSection::Advanced),
    ("proxy.enabled", SettingsSection::Advanced),
    ("proxy.proxy_type", SettingsSection::Advanced),
    ("proxy.host", SettingsSection::Advanced),
    ("proxy.port", SettingsSection::Advanced),
    ("start_with_system", SettingsSection::General),
    ("start_minimized", SettingsSection::General),
    ("exit_on_close", SettingsSection::General),
    ("run_in_background", SettingsSection::General),
    ("typography.font_display", SettingsSection::Appearance),
    ("typography.font_body", SettingsSection::Appearance),
    ("typography.font_mono", SettingsSection::Appearance),
    ("typography.line_height_base", SettingsSection::Appearance),
    ("typography.spacing_scale", SettingsSection::Appearance),
    ("typography.preset_name", SettingsSection::Appearance),
    ("rpc.app_id", SettingsSection::Integrations),
    ("rpc.large_image_key", SettingsSection::Integrations),
    ("league.auto_accept", SettingsSection::Integrations),
    ("league.auto_accept_delay", SettingsSection::Integrations),
    ("league.notify_ready_check", SettingsSection::Integrations),
    ("league.auto_pick", SettingsSection::Integrations),
    ("league.auto_ban", SettingsSection::Integrations),
    ("league.auto_ban_delay", SettingsSection::Integrations),
    ("league.auto_lock", SettingsSection::Integrations),
    ("league.auto_lock_at_timeout", SettingsSection::Integrations),
    ("league.auto_runes", SettingsSection::Integrations),
    ("league.auto_honor", SettingsSection::Integrations),
    ("league.auto_play_again", SettingsSection::Integrations),
    ("league.auto_requeue", SettingsSection::Integrations),
    ("league.auto_accept_swaps", SettingsSection::Integrations),
    ("league.auto_reconnect", SettingsSection::Integrations),
    ("league.auto_trade", SettingsSection::Integrations),
    ("league.auto_message", SettingsSection::Integrations),
    ("league.pick_champions", SettingsSection::Integrations),
    ("league.ban_champions", SettingsSection::Integrations),
    ("league.pick_random", SettingsSection::Integrations),
    ("league.skin_roulette", SettingsSection::Integrations),
    (
        "league.skin_roulette_include_base",
        SettingsSection::Integrations,
    ),
    ("league.ward_roulette", SettingsSection::Integrations),
    ("league.sgp_enabled", SettingsSection::Integrations),
    ("league.coach_style", SettingsSection::Integrations),
    ("accessibility.reduce_motion", SettingsSection::Appearance),
    (
        "accessibility.reduce_transparency",
        SettingsSection::Appearance,
    ),
    ("accessibility.disable_haptics", SettingsSection::Appearance),
    ("world.tier_override", SettingsSection::Integrations),
    ("world.backend_override", SettingsSection::Integrations),
    ("world.city_backend", SettingsSection::Integrations),
    ("llm.prune_enabled", SettingsSection::Ai),
    ("llm.prune_judge", SettingsSection::Ai),
    ("llm.prune_min_tokens", SettingsSection::Ai),
    ("llm.turn_idle_timeout_s", SettingsSection::Ai),
    ("llm.turn_tool_idle_timeout_s", SettingsSection::Ai),
    ("llm.turn_start_timeout_s", SettingsSection::Ai),
    ("llm.job_default_deadline_min", SettingsSection::Ai),
    ("llm.ask_timeout_chat_s", SettingsSection::Ai),
    ("llm.ask_timeout_job_s", SettingsSection::Ai),
    ("llm.approval_notify_delay_s", SettingsSection::Ai),
    ("llm.jobs_max_parallel", SettingsSection::Ai),
    ("llm.jobs_max_per_agent", SettingsSection::Ai),
    // `newSchemaAllowlist`: the schema 3 appearance fields.
    ("appearance.mode", SettingsSection::Appearance),
    ("appearance.preset", SettingsSection::Appearance),
    ("appearance.legacy_theme", SettingsSection::Appearance),
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResetScope {
    App,
    Section(SettingsSection),
}

/// Paths a reset of `scope` restores: the allowlist, or its intersection
/// with one section.
pub fn reset_paths(scope: ResetScope) -> Vec<&'static str> {
    RESET_ALLOWLIST
        .iter()
        .filter(|(_, section)| match scope {
            ResetScope::App => true,
            ResetScope::Section(s) => *section == s,
        })
        .map(|(path, _)| *path)
        .collect()
}

/// Restores the allowlisted fields of `scope` to the defaults of a new
/// install and keeps every other field, including the migration marker.
pub fn apply_reset(current: &AppSettings, scope: ResetScope) -> AppSettings {
    let defaults = serde_json::to_value(AppSettings::fresh_install()).expect("serialize");
    let mut value = serde_json::to_value(current).expect("serialize");
    for path in reset_paths(scope) {
        let default = get_path(&defaults, path).cloned().unwrap_or(Value::Null);
        set_path(&mut value, path, default);
    }
    let mut next: AppSettings = match serde_json::from_value(value) {
        Ok(next) => next,
        // Every default is valid for its own type; keep the current values
        // rather than invent anything if that ever stops being true.
        Err(_) => current.clone(),
    };
    normalize_appearance(&mut next);
    next
}

// ---------------------------------------------------------------------------
// Appearance import
// ---------------------------------------------------------------------------

/// The valid parts of the legacy workspace entry: only `mode` and `preset`,
/// each validated on its own. Anything else in the payload is ignored.
pub fn parse_workspace_appearance(raw: Option<&str>) -> (Option<String>, Option<String>) {
    let Some(value) = raw.and_then(|r| serde_json::from_str::<Value>(r).ok()) else {
        return (None, None);
    };
    let mode = value
        .get("mode")
        .and_then(Value::as_str)
        .filter(|m| MODES.contains(m))
        .map(str::to_string);
    let preset = value
        .get("preset")
        .and_then(Value::as_str)
        .filter(|p| WORKSPACE_PRESETS.contains(p))
        .map(str::to_string);
    (mode, preset)
}

/// Builds the schema 3 appearance: a valid workspace entry wins field by
/// field, then values already set (a new install), then the legacy theme.
/// The original theme is kept in `legacy_theme`; the marker becomes 1.
pub fn import_appearance(current: &AppSettings, raw_workspace: Option<&str>) -> AppSettings {
    let (ws_mode, ws_preset) = parse_workspace_appearance(raw_workspace);
    let mut next = current.clone();
    let legacy = current.appearance.theme.clone();
    let a = &mut next.appearance;
    a.mode = Some(
        ws_mode
            .or_else(|| a.mode.clone())
            .unwrap_or_else(|| derive_mode(&legacy)),
    );
    a.preset = Some(
        ws_preset
            .or_else(|| a.preset.clone())
            .unwrap_or_else(|| derive_preset(&legacy)),
    );
    a.legacy_theme = Some(legacy);
    a.migration_version = 1;
    normalize_appearance(&mut next);
    next
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn keys(v: &Value, prefix: &str, out: &mut Vec<String>) {
        if let Value::Object(map) = v {
            for (k, child) in map {
                let p = if prefix.is_empty() {
                    k.clone()
                } else {
                    format!("{prefix}.{k}")
                };
                if child.is_object() {
                    keys(child, &p, out);
                } else {
                    out.push(p);
                }
            }
        }
    }

    fn imported() -> AppSettings {
        import_appearance(&AppSettings::default(), None)
    }

    #[test]
    fn the_public_dto_is_the_private_model_minus_the_secrets() {
        let mut private = Vec::new();
        keys(&schema_template(), "", &mut private);
        let mut public = Vec::new();
        keys(
            &serde_json::to_value(AppSettings::default().to_public()).unwrap(),
            "",
            &mut public,
        );
        let expected: Vec<String> = private
            .into_iter()
            .map(|p| match p.as_str() {
                "proxy.password" => "proxy.has_password".into(),
                "advanced.twitter_manual_cookie" => "advanced.has_twitter_manual_cookie".into(),
                "bridge.token" => "bridge.has_token".into(),
                _ => p,
            })
            .collect();
        let mut a = expected.clone();
        a.sort();
        let mut b = public.clone();
        b.sort();
        assert_eq!(a, b);
    }

    #[test]
    fn secrets_never_reach_the_public_dto() {
        let mut s = AppSettings::default();
        s.proxy.password = "sentinel-proxy-secret".into();
        s.advanced.twitter_manual_cookie = "sentinel-cookie-secret".into();
        s.bridge.token = "sentinel-bridge-secret".into();
        let text = serde_json::to_string(&s.to_public()).unwrap();
        assert!(!text.contains("sentinel"));
        let v: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(v["proxy"]["has_password"], json!(true));
        assert_eq!(v["advanced"]["has_twitter_manual_cookie"], json!(true));
        assert_eq!(v["bridge"]["has_token"], json!(true));
        assert!(v["proxy"].get("password").is_none());
        assert!(v["bridge"].get("token").is_none());
    }

    #[test]
    fn a_new_install_gets_the_desktop_preset_and_system_fonts() {
        let s = AppSettings::fresh_install();
        assert_eq!(s.schema_version, SETTINGS_SCHEMA_VERSION);
        assert_eq!(s.appearance.preset.as_deref(), Some(DEFAULT_PRESET));
        assert_eq!(s.appearance.mode.as_deref(), Some("system"));
        assert_eq!(s.appearance.migration_version, 0);
        assert_eq!(s.typography.font_display, "system-ui");
        assert_eq!(s.typography.font_body, "system-ui");
        assert_eq!(s.typography.font_mono, "ui-monospace");
        assert_eq!(s.typography.line_height_base, 1.55);
        assert_eq!(s.typography.spacing_scale, 1.0);
        // Operational defaults do not move for a new install.
        let d = AppSettings::default();
        assert_eq!(s.download.video_quality, d.download.video_quality);
        assert_eq!(
            s.advanced.max_concurrent_downloads,
            d.advanced.max_concurrent_downloads
        );
        assert!(!s.proxy.enabled && !s.start_with_system && !s.download.clipboard_detection);
    }

    #[test]
    fn a_schema_2_profile_migrates_without_losing_anything() {
        let mut old = AppSettings {
            schema_version: 2,
            ..AppSettings::default()
        };
        old.world.enabled = false;
        old.typography.font_body = "Inter".into();
        old.appearance.theme = "dracula".into();
        assert!(old.migrate());
        assert_eq!(old.schema_version, 3);
        assert!(!old.world.enabled, "v3 does not re-enable the world");
        assert_eq!(old.typography.font_body, "Inter");
        assert_eq!(old.appearance.theme, "dracula");
        assert_eq!(old.appearance.migration_version, 0);
        let public = old.to_public();
        assert_eq!(public.appearance.preset, "dracula");
        assert_eq!(public.appearance.mode, "system");
        assert!(!old.migrate());
    }

    #[test]
    fn a_patch_must_be_an_object_of_known_fields() {
        let s = imported();
        assert_eq!(
            apply_patch(&s, &json!(null)).unwrap_err(),
            PatchError::Invalid(vec![FieldError::new("", FieldErrorCode::InvalidType)])
        );
        assert!(apply_patch(&s, &json!([1])).is_err());
        let Err(PatchError::Invalid(errs)) = apply_patch(
            &s,
            &json!({ "download": { "nope": 1 }, "bogus": true, "proxy": 3 }),
        ) else {
            panic!("expected errors")
        };
        assert!(errs.contains(&FieldError::new(
            "download.nope",
            FieldErrorCode::UnknownField
        )));
        assert!(errs.contains(&FieldError::new("bogus", FieldErrorCode::UnknownField)));
        assert!(errs.contains(&FieldError::new("proxy", FieldErrorCode::InvalidType)));
    }

    #[test]
    fn secrets_and_managed_fields_are_protected_in_a_patch() {
        let s = imported();
        for patch in [
            json!({ "proxy": { "password": "" } }),
            json!({ "proxy": { "has_password": true } }),
            json!({ "advanced": { "twitter_manual_cookie": "x" } }),
            json!({ "bridge": { "token": "x" } }),
            json!({ "bridge": { "port": 1234 } }),
            json!({ "schema_version": 3 }),
            json!({ "appearance": { "migration_version": 0 } }),
        ] {
            let Err(PatchError::Invalid(errs)) = apply_patch(&s, &patch) else {
                panic!("{patch} must be refused")
            };
            assert_eq!(errs[0].code, FieldErrorCode::ProtectedField, "{patch}");
        }
    }

    #[test]
    fn types_ranges_and_enums_are_checked_per_field() {
        let s = imported();
        let err = |p: Value| match apply_patch(&s, &p) {
            Err(PatchError::Invalid(e)) => e,
            other => panic!("{p}: {other:?}"),
        };
        assert_eq!(
            err(json!({ "proxy": { "port": 70000 } })),
            vec![FieldError::new("proxy.port", FieldErrorCode::OutOfRange)]
        );
        assert_eq!(
            err(json!({ "proxy": { "port": "80" } })),
            vec![FieldError::new("proxy.port", FieldErrorCode::InvalidType)]
        );
        assert_eq!(
            err(json!({ "advanced": { "max_retries": 1.5 } })),
            vec![FieldError::new(
                "advanced.max_retries",
                FieldErrorCode::InvalidType
            )]
        );
        assert_eq!(
            err(json!({ "advanced": { "max_concurrent_downloads": 11 } })),
            vec![FieldError::new(
                "advanced.max_concurrent_downloads",
                FieldErrorCode::OutOfRange
            )]
        );
        assert_eq!(
            err(json!({ "advanced": { "max_concurrent_downloads": 0 } }))[0].code,
            FieldErrorCode::OutOfRange
        );
        assert_eq!(
            err(json!({ "typography": { "line_height_base": 1e39 } })),
            vec![FieldError::new(
                "typography.line_height_base",
                FieldErrorCode::OutOfRange
            )]
        );
        assert_eq!(
            err(json!({ "appearance": { "mode": "sepia" } })),
            vec![FieldError::new(
                "appearance.mode",
                FieldErrorCode::InvalidEnum
            )]
        );
        assert_eq!(
            err(json!({ "appearance": { "language": "xx" } }))[0].code,
            FieldErrorCode::InvalidEnum
        );
        assert_eq!(
            err(json!({ "download": { "video_quality": null } }))[0].code,
            FieldErrorCode::InvalidType
        );
        // Declared nullable fields accept null.
        let next = apply_patch(&s, &json!({ "llm": { "ask_timeout_job_s": null } })).unwrap();
        assert_eq!(next.llm.ask_timeout_job_s, None);
        let next = apply_patch(&s, &json!({ "world": { "tier_override": null } })).unwrap();
        assert_eq!(next.world.tier_override, None);
    }

    #[test]
    fn the_proxy_accepts_the_three_backend_protocols_and_needs_host_and_port() {
        let s = imported();
        for kind in ["http", "https", "socks5"] {
            let next = apply_patch(
                &s,
                &json!({ "proxy": { "enabled": true, "proxy_type": kind, "host": "127.0.0.1", "port": 1080 } }),
            )
            .unwrap();
            assert_eq!(next.proxy.proxy_type, kind);
        }
        let Err(PatchError::Invalid(errs)) = apply_patch(
            &s,
            &json!({ "proxy": { "enabled": true, "host": " ", "port": 0, "proxy_type": "socks4" } }),
        ) else {
            panic!()
        };
        assert!(errs.contains(&FieldError::new("proxy.port", FieldErrorCode::OutOfRange)));
        assert!(errs.contains(&FieldError::new("proxy.host", FieldErrorCode::Required)));
        assert!(errs.contains(&FieldError::new(
            "proxy.proxy_type",
            FieldErrorCode::InvalidEnum
        )));
        // Disabled proxy: port 0 and empty host are not an error.
        assert!(apply_patch(&s, &json!({ "proxy": { "enabled": false, "port": 0 } })).is_ok());
    }

    #[test]
    fn old_valid_values_outside_the_ui_ranges_survive_other_edits() {
        let mut s = imported();
        s.llm.ask_timeout_chat_s = 30;
        s.advanced.max_concurrent_segments = 0;
        let next = apply_patch(&s, &json!({ "download": { "video_quality": "1080p" } })).unwrap();
        assert_eq!(next.llm.ask_timeout_chat_s, 30);
        assert_eq!(next.advanced.max_concurrent_segments, 0);
        // And a zero the type allows is not turned into a default.
        let next = apply_patch(&s, &json!({ "llm": { "approval_notify_delay_s": 0 } })).unwrap();
        assert_eq!(next.llm.approval_notify_delay_s, 0);
    }

    #[test]
    fn appearance_is_locked_until_the_import_committed() {
        let s = AppSettings::default();
        assert_eq!(
            apply_patch(&s, &json!({ "appearance": { "mode": "dark" } })).unwrap_err(),
            PatchError::AppearanceImportRequired
        );
        assert_eq!(
            apply_patch(&s, &json!({ "appearance": { "theme": "dark" } })).unwrap_err(),
            PatchError::AppearanceImportRequired
        );
        // Language is not behind the visual barrier.
        let next = apply_patch(&s, &json!({ "appearance": { "language": "pt" } })).unwrap();
        assert_eq!(next.appearance.language, "pt");
    }

    #[test]
    fn the_legacy_theme_stays_coherent_with_mode_and_preset() {
        let s = imported();
        let next = apply_patch(&s, &json!({ "appearance": { "preset": "dracula" } })).unwrap();
        assert_eq!(next.appearance.theme, "dracula");
        let next = apply_patch(&next, &json!({ "appearance": { "theme": "light" } })).unwrap();
        assert_eq!(next.appearance.mode.as_deref(), Some("light"));
        assert_eq!(next.appearance.preset.as_deref(), Some(DEFAULT_PRESET));
        assert_eq!(next.appearance.theme, "light");
        let next = apply_patch(
            &next,
            &json!({ "appearance": { "preset": "amber-slate", "mode": "dark" } }),
        )
        .unwrap();
        assert_eq!(next.appearance.theme, "dark");
    }

    #[test]
    fn secret_operations_are_strict() {
        assert_eq!(
            parse_secret_operations(None),
            Ok(SecretOperations::default())
        );
        let ops = parse_secret_operations(Some(&json!({
            "proxyPassword": { "op": "set", "value": " spaced " },
            "twitterManualCookie": { "op": "remove" }
        })))
        .unwrap();
        assert_eq!(ops.proxy_password, SecretOperation::Set(" spaced ".into()));
        assert_eq!(ops.twitter_manual_cookie, SecretOperation::Remove);
        for bad in [
            json!({ "proxyPassword": { "op": "set", "value": "" } }),
            json!({ "proxyPassword": { "op": "keep", "value": "x" } }),
            json!({ "proxyPassword": { "op": "remove", "extra": 1 } }),
            json!({ "proxyPassword": { "op": "rotate" } }),
            json!({ "bridgeToken": { "op": "remove" } }),
            json!("keep"),
        ] {
            assert!(parse_secret_operations(Some(&bad)).is_err(), "{bad}");
        }
        let mut s = AppSettings::default();
        s.proxy.password = "old".into();
        s.advanced.twitter_manual_cookie = "cookie".into();
        apply_secret_operations(&mut s, &SecretOperations::default());
        assert_eq!(s.proxy.password, "old");
        apply_secret_operations(&mut s, &ops);
        assert_eq!(s.proxy.password, " spaced ");
        assert_eq!(s.advanced.twitter_manual_cookie, "");
    }

    #[test]
    fn a_section_reset_touches_only_its_allowlisted_fields() {
        let mut s = imported();
        s.appearance.language = "pt".into();
        s.start_with_system = true;
        s.download.video_quality = "1080p".into();
        s.proxy.password = "secret".into();
        s.proxy.username = "user".into();
        s.bridge.token = "token".into();
        s.bridge.port = 43_210;
        s.legal_acknowledged = true;
        s.onboarding_completed = true;
        s.rpc.enabled = false;
        s.world.tier_measured = Some(2);
        let general = apply_reset(&s, ResetScope::Section(SettingsSection::General));
        assert_eq!(general.appearance.language, "en");
        assert!(!general.start_with_system);
        assert_eq!(general.download.video_quality, "1080p");

        let app = apply_reset(&s, ResetScope::App);
        assert_eq!(app.download.video_quality, "720p");
        assert_eq!(app.proxy.password, "secret");
        assert_eq!(app.proxy.username, "user");
        assert_eq!(app.bridge.token, "token");
        assert_eq!(app.bridge.port, 43_210);
        assert!(app.legal_acknowledged && app.onboarding_completed);
        assert!(!app.rpc.enabled);
        assert_eq!(app.world.tier_measured, Some(2));
        assert_eq!(app.appearance.migration_version, 1);
        assert_eq!(app.appearance.preset.as_deref(), Some(DEFAULT_PRESET));
        assert_eq!(app.appearance.legacy_theme, None);
        assert_eq!(app.typography.font_body, "system-ui");
    }

    #[test]
    fn the_reset_allowlist_has_no_duplicates() {
        let mut paths: Vec<_> = RESET_ALLOWLIST.iter().map(|(p, _)| *p).collect();
        let n = paths.len();
        paths.sort();
        paths.dedup();
        assert_eq!(paths.len(), n);
        let template = schema_template();
        for p in paths {
            assert!(
                get_path(&template, p).is_some(),
                "{p} is not a settings field"
            );
        }
    }

    #[test]
    fn the_import_prefers_a_valid_workspace_entry_field_by_field() {
        let mut legacy = AppSettings::default();
        legacy.appearance.theme = "dark".into();
        let next = import_appearance(
            &legacy,
            Some(r#"{"preset":"amber-slate","mode":"light","extra":"ignored"}"#),
        );
        assert_eq!(next.appearance.mode.as_deref(), Some("light"));
        assert_eq!(next.appearance.preset.as_deref(), Some("amber-slate"));
        assert_eq!(next.appearance.legacy_theme.as_deref(), Some("dark"));
        assert_eq!(next.appearance.migration_version, 1);
        assert_eq!(next.appearance.theme, "light");

        let next = import_appearance(&legacy, Some(r#"{"preset":"nope","mode":"light"}"#));
        assert_eq!(next.appearance.preset.as_deref(), Some(DEFAULT_PRESET));
        let next = import_appearance(&legacy, Some("{not json"));
        assert_eq!(next.appearance.mode.as_deref(), Some("dark"));

        let mut named = AppSettings::default();
        named.appearance.theme = "catppuccin-mocha".into();
        let next = import_appearance(&named, None);
        assert_eq!(next.appearance.preset.as_deref(), Some("catppuccin-mocha"));
        assert_eq!(next.appearance.theme, "catppuccin-mocha");
    }
}

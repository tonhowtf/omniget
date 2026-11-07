use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppSettings {
    pub schema_version: u32,
    pub appearance: AppearanceSettings,
    pub download: DownloadSettings,
    pub advanced: AdvancedSettings,
    #[serde(default)]
    pub telegram: TelegramSettings,
    #[serde(default)]
    pub proxy: ProxySettings,
    #[serde(default)]
    pub onboarding_completed: bool,
    #[serde(default, alias = "start_with_windows")]
    pub start_with_system: bool,
    #[serde(default)]
    pub start_minimized: bool,
    #[serde(default)]
    pub exit_on_close: bool,
    #[serde(default)]
    pub run_in_background: bool,
    #[serde(default)]
    pub portable_mode: bool,
    #[serde(default)]
    pub legal_acknowledged: bool,
    #[serde(default)]
    pub last_download_options: LastDownloadOptions,
    #[serde(default)]
    pub typography: TypographySettings,
    #[serde(default)]
    pub rpc: RpcSettings,
    #[serde(default)]
    pub bridge: BridgeSettings,
    #[serde(default)]
    pub league: LeagueSettings,
    #[serde(default)]
    pub music: MusicSettings,
    #[serde(default)]
    pub video: VideoSettings,
    #[serde(default)]
    pub tube: TubeSettings,
    pub washmybelly: WashMyBellySettings,
    #[serde(default)]
    pub read: ReadSettings,
    #[serde(default)]
    pub accessibility: AccessibilitySettings,
    #[serde(default)]
    pub world: WorldSettings,
    #[serde(default)]
    pub llm: LlmSettings,
    #[serde(default)]
    pub canvas: CanvasSettings,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmSettings {
    #[serde(default = "default_turn_idle_timeout_s")]
    pub turn_idle_timeout_s: u32,
    #[serde(default = "default_turn_tool_idle_timeout_s")]
    pub turn_tool_idle_timeout_s: u32,
    #[serde(default = "default_turn_start_timeout_s")]
    pub turn_start_timeout_s: u32,
    #[serde(default = "default_job_deadline_min")]
    pub job_default_deadline_min: u32,
    #[serde(default = "default_ask_timeout_chat_s")]
    pub ask_timeout_chat_s: u32,
    #[serde(default)]
    pub ask_timeout_job_s: Option<u32>,
    #[serde(default = "default_approval_notify_delay_s")]
    pub approval_notify_delay_s: u32,
    #[serde(default = "default_jobs_max_parallel")]
    pub jobs_max_parallel: u32,
    #[serde(default)]
    pub jobs_max_per_agent: Option<u32>,
    #[serde(default)]
    pub auto_resume: bool,
    #[serde(default = "default_audit_retention_days")]
    pub audit_retention_days: u32,
}

fn default_audit_retention_days() -> u32 {
    90
}

fn default_turn_idle_timeout_s() -> u32 {
    600
}

fn default_turn_tool_idle_timeout_s() -> u32 {
    1800
}

fn default_turn_start_timeout_s() -> u32 {
    90
}

fn default_job_deadline_min() -> u32 {
    60
}

fn default_ask_timeout_chat_s() -> u32 {
    1800
}

fn default_approval_notify_delay_s() -> u32 {
    5
}

fn default_jobs_max_parallel() -> u32 {
    4
}

impl Default for LlmSettings {
    fn default() -> Self {
        Self {
            turn_idle_timeout_s: default_turn_idle_timeout_s(),
            turn_tool_idle_timeout_s: default_turn_tool_idle_timeout_s(),
            turn_start_timeout_s: default_turn_start_timeout_s(),
            job_default_deadline_min: default_job_deadline_min(),
            ask_timeout_chat_s: default_ask_timeout_chat_s(),
            ask_timeout_job_s: None,
            approval_notify_delay_s: default_approval_notify_delay_s(),
            jobs_max_parallel: default_jobs_max_parallel(),
            jobs_max_per_agent: None,
            auto_resume: false,
            audit_retention_days: default_audit_retention_days(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorldSettings {
    #[serde(default = "default_world_enabled")]
    pub enabled: bool,
}

pub const CANVAS_GRID: f64 = 20.0;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct CanvasSettings {
    pub background: String,
    pub zoom_step: f64,
    pub zoom_modifier: String,
    pub right_drag_pan: bool,
    pub focus_on_hover: bool,
    pub selection_style: String,
    pub node_material: String,
    pub cord_style: String,
    pub rope_routing: String,
    pub spell_check: bool,
    pub always_copy_files: bool,
    pub default_agent: String,
    pub defaults: CanvasDefaults,
    pub terminal: CanvasTerminalSettings,
    pub notifications: CanvasNotificationSettings,
    pub power: CanvasPowerSettings,
    pub agent_presets: std::collections::BTreeMap<String, AgentPreset>,
    pub shortcuts: std::collections::BTreeMap<String, String>,
    pub skills: CanvasSkillSettings,
    pub images: CanvasImageSettings,
    pub performance: CanvasPerformanceSettings,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct CanvasImageSettings {
    pub any_zoom: bool,
    pub quality: String,
    pub download_web: bool,
}

impl Default for CanvasImageSettings {
    fn default() -> Self {
        Self {
            any_zoom: true,
            quality: "balanced".into(),
            download_web: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct CanvasPerformanceSettings {
    pub mounted_terminals: u32,
    pub live_portals: u32,
    pub force_canvas2d: bool,
    pub rendering: String,
}

impl Default for CanvasPerformanceSettings {
    fn default() -> Self {
        Self {
            mounted_terminals: 12,
            live_portals: 4,
            force_canvas2d: false,
            rendering: "faithful".into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct CanvasSkillSettings {
    pub roots: std::collections::BTreeMap<String, String>,
}

impl Default for CanvasSkillSettings {
    fn default() -> Self {
        Self {
            roots: ["claude", "codex", "gemini", "opencode"]
                .into_iter()
                .map(|a| (a.to_string(), String::new()))
                .collect(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct CanvasDefaults {
    pub note: CanvasNoteDefaults,
    pub terminal: CanvasSizeDefault,
    pub text: CanvasSizeDefault,
    pub portal: CanvasSizeDefault,
    pub agent_placement: String,
    pub portal_placement: String,
}

impl Default for CanvasDefaults {
    fn default() -> Self {
        Self {
            note: CanvasNoteDefaults::default(),
            terminal: CanvasSizeDefault::default(),
            text: CanvasSizeDefault::default(),
            portal: CanvasSizeDefault::default(),
            agent_placement: "free".into(),
            portal_placement: "free".into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct CanvasNoteDefaults {
    pub size: String,
    pub font: String,
    pub render: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct CanvasSizeDefault {
    pub size: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct CanvasTerminalSettings {
    pub font_family: String,
    pub font_size: f64,
    pub theme: String,
    pub padding: CanvasPadding,
    pub option_as_meta: bool,
    pub memory_limit_mb: u32,
    pub shell_fallback: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct CanvasPadding {
    pub x: f64,
    pub y: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct CanvasNotificationSettings {
    pub attention: bool,
    pub badges_for_non_maestro: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct CanvasPowerSettings {
    pub prevent_sleep: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct AgentPreset {
    pub command: String,
    pub args: Vec<String>,
    pub env: std::collections::BTreeMap<String, String>,
    pub icon: Option<String>,
    pub color: Option<String>,
    pub maestro: bool,
    pub start_in_chat_mode: bool,
    pub cwd: Option<String>,
}

impl Default for CanvasSettings {
    fn default() -> Self {
        Self {
            background: "grid".into(),
            zoom_step: 0.1,
            zoom_modifier: "alt".into(),
            right_drag_pan: false,
            focus_on_hover: false,
            selection_style: "dashed".into(),
            node_material: "opaque".into(),
            cord_style: "bezier".into(),
            rope_routing: "behind_nodes".into(),
            spell_check: false,
            always_copy_files: false,
            default_agent: "shell".into(),
            defaults: CanvasDefaults::default(),
            terminal: CanvasTerminalSettings::default(),
            notifications: CanvasNotificationSettings::default(),
            power: CanvasPowerSettings::default(),
            agent_presets: Default::default(),
            shortcuts: Default::default(),
            skills: Default::default(),
            images: CanvasImageSettings::default(),
            performance: CanvasPerformanceSettings::default(),
        }
    }
}

impl Default for CanvasNoteDefaults {
    fn default() -> Self {
        Self {
            size: String::new(),
            font: String::new(),
            render: "formatted".into(),
        }
    }
}

impl Default for CanvasTerminalSettings {
    fn default() -> Self {
        Self {
            font_family: String::new(),
            font_size: 0.0,
            theme: "system".into(),
            padding: CanvasPadding::default(),
            option_as_meta: false,
            memory_limit_mb: 0,
            shell_fallback: true,
        }
    }
}

impl Default for CanvasNotificationSettings {
    fn default() -> Self {
        Self {
            attention: true,
            badges_for_non_maestro: true,
        }
    }
}

impl Default for CanvasPowerSettings {
    fn default() -> Self {
        Self {
            prevent_sleep: "while_working".into(),
        }
    }
}

pub fn parse_canvas_size(s: &str) -> Option<(f64, f64)> {
    let (w, h) = s.trim().split_once(['x', 'X', '×'])?;
    let w: f64 = w.trim().parse().ok()?;
    let h: f64 = h.trim().parse().ok()?;
    if !(w.is_finite() && h.is_finite()) || w <= 0.0 || h <= 0.0 {
        return None;
    }
    let snap = |v: f64| ((v / CANVAS_GRID).round().max(1.0) * CANVAS_GRID).min(20_000.0);
    Some((snap(w), snap(h)))
}

fn snap_size(s: &mut String) {
    *s = match parse_canvas_size(s) {
        Some((w, h)) => format!("{w}x{h}"),
        None => String::new(),
    };
}

pub fn canvas_token(s: &str) -> String {
    let mut out = String::new();
    for (i, c) in s.trim().chars().enumerate() {
        if c.is_ascii_uppercase() {
            if i > 0 {
                out.push('_');
            }
            out.push(c.to_ascii_lowercase());
        } else if c == '-' || c == ' ' {
            out.push('_');
        } else {
            out.push(c);
        }
    }
    out
}

fn pick(v: &mut String, allowed: &[&str], default: &str) {
    let t = canvas_token(v);
    *v = if allowed.contains(&t.as_str()) {
        t
    } else {
        default.to_string()
    };
}

impl CanvasSettings {
    pub const BACKGROUNDS: &'static [&'static str] = &["grid", "plain", "transparent"];
    pub const ZOOM_MODIFIERS: &'static [&'static str] = &["alt", "ctrl", "cmd"];
    pub const SELECTION_STYLES: &'static [&'static str] =
        &["solid", "dashed", "corners", "corner_dots", "elevation"];
    pub const MATERIALS: &'static [&'static str] = &["opaque", "glass"];
    pub const CORD_STYLES: &'static [&'static str] = &["bezier", "rope", "circuit"];
    pub const ROUTINGS: &'static [&'static str] = &["behind_nodes", "avoid_nodes", "circuit"];
    pub const SLEEP_MODES: &'static [&'static str] = &["never", "always", "while_working"];
    pub const IMAGE_QUALITIES: &'static [&'static str] = &["light", "balanced", "sharp"];
    pub const RENDERINGS: &'static [&'static str] = &["faithful", "light"];
    pub const AGENT_PLACEMENTS: &'static [&'static str] = &["free", "stack"];
    pub const PORTAL_PLACEMENTS: &'static [&'static str] = &["free", "beside"];

    pub fn normalize(&mut self) {
        let d = Self::default();
        pick(&mut self.background, Self::BACKGROUNDS, &d.background);
        if !self.zoom_step.is_finite() {
            self.zoom_step = d.zoom_step;
        }
        self.zoom_step = (self.zoom_step.clamp(0.05, 1.0) * 100.0).round() / 100.0;
        let m = canvas_token(&self.zoom_modifier);
        self.zoom_modifier = match m.as_str() {
            "option" | "opt" => "alt".into(),
            "control" => "ctrl".into(),
            "command" | "meta" | "primary" => "cmd".into(),
            _ => m,
        };
        pick(
            &mut self.zoom_modifier,
            Self::ZOOM_MODIFIERS,
            &d.zoom_modifier,
        );
        pick(
            &mut self.selection_style,
            Self::SELECTION_STYLES,
            &d.selection_style,
        );
        pick(&mut self.node_material, Self::MATERIALS, &d.node_material);
        pick(&mut self.cord_style, Self::CORD_STYLES, &d.cord_style);
        pick(&mut self.rope_routing, Self::ROUTINGS, &d.rope_routing);
        if self.rope_routing == "circuit" {
            self.cord_style = "circuit".into();
            self.rope_routing = d.rope_routing.clone();
        }
        pick(
            &mut self.power.prevent_sleep,
            Self::SLEEP_MODES,
            &d.power.prevent_sleep,
        );
        if self.default_agent.trim().is_empty() {
            self.default_agent = d.default_agent;
        }
        snap_size(&mut self.defaults.note.size);
        snap_size(&mut self.defaults.terminal.size);
        snap_size(&mut self.defaults.text.size);
        snap_size(&mut self.defaults.portal.size);
        pick(
            &mut self.defaults.agent_placement,
            Self::AGENT_PLACEMENTS,
            &d.defaults.agent_placement,
        );
        pick(
            &mut self.defaults.portal_placement,
            Self::PORTAL_PLACEMENTS,
            &d.defaults.portal_placement,
        );
        let r = canvas_token(&self.defaults.note.render);
        self.defaults.note.render = if r == "raw" { r } else { "formatted".into() };
        let t = &mut self.terminal;
        if !t.font_size.is_finite() || t.font_size < 0.0 {
            t.font_size = 0.0;
        }
        if t.font_size > 0.0 {
            t.font_size = t.font_size.clamp(6.0, 48.0);
        }
        if t.theme.trim().is_empty() {
            t.theme = "system".into();
        }
        t.padding.x = if t.padding.x.is_finite() {
            t.padding.x.clamp(0.0, 64.0)
        } else {
            0.0
        };
        t.padding.y = if t.padding.y.is_finite() {
            t.padding.y.clamp(0.0, 64.0)
        } else {
            0.0
        };
        self.agent_presets
            .retain(|name, p| !name.trim().is_empty() && !p.command.trim().is_empty());
        pick(
            &mut self.images.quality,
            Self::IMAGE_QUALITIES,
            &d.images.quality,
        );
        pick(
            &mut self.performance.rendering,
            Self::RENDERINGS,
            &d.performance.rendering,
        );
        let perf = &mut self.performance;
        perf.mounted_terminals = perf.mounted_terminals.clamp(1, 64);
        perf.live_portals = perf.live_portals.clamp(0, 16);
    }

    pub fn image_budget_mb(&self) -> u32 {
        match self.images.quality.as_str() {
            "light" => 128,
            "sharp" => 512,
            _ => 256,
        }
    }
}

fn default_world_enabled() -> bool {
    true
}

impl Default for WorldSettings {
    fn default() -> Self {
        Self {
            enabled: default_world_enabled(),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AccessibilitySettings {
    #[serde(default)]
    pub reduce_motion: bool,
    #[serde(default)]
    pub reduce_transparency: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MusicSettings {
    #[serde(default = "default_music_enabled")]
    pub enabled: bool,
}

fn default_music_enabled() -> bool {
    true
}

impl Default for MusicSettings {
    fn default() -> Self {
        Self {
            enabled: default_music_enabled(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VideoSettings {
    #[serde(default = "default_video_enabled")]
    pub enabled: bool,
}

fn default_video_enabled() -> bool {
    true
}

impl Default for VideoSettings {
    fn default() -> Self {
        Self {
            enabled: default_video_enabled(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct TubeSettings {
    pub enabled: bool,
    pub look: String,
    pub start_face: String,
    pub start_page: String,
    pub use_account: bool,
    pub default_quality: String,
    pub prefer_codec: String,
    pub audio_quality: String,
    pub autoplay_next: bool,
    pub autoplay_on_open: bool,
    pub preview_on_hover: bool,
    pub normalize_volume: bool,
    pub send_history: bool,
    pub endless_radio: bool,
    pub sponsorblock: bool,
    pub sb_categories: std::collections::BTreeMap<String, String>,
    pub sb_server: String,
    pub sb_whitelist: Vec<String>,
    pub sb_download: String,
    pub dearrow: bool,
    pub dearrow_thumbs: bool,
    pub ryd: bool,
    pub hide_shorts: bool,
    pub hide_live: bool,
    pub hide_upcoming: bool,
    pub hide_members: bool,
    pub hide_mixes: bool,
    pub hide_playables: bool,
    pub hide_posts: bool,
    pub hide_watched: bool,
    pub watched_threshold: u8,
    pub hide_home_feed: bool,
    pub hide_related: bool,
    pub hide_comments: bool,
    pub hide_chips: bool,
    pub hide_metrics: bool,
    pub infinite_scroll: bool,
    pub grid_columns: u8,
    pub blocked_channels: Vec<TubeBlockedChannel>,
    pub blocked_words: Vec<String>,
    pub whole_word: bool,
    pub min_duration_s: u32,
    pub max_duration_s: u32,
    pub lyrics_provider: String,
    pub download_folder: Option<String>,
    pub storage_cap_gb: f64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TubeBlockedChannel {
    pub id: String,
    #[serde(default)]
    pub name: String,
}

impl Default for TubeSettings {
    fn default() -> Self {
        let sb_categories = [
            ("sponsor", "skip"),
            ("selfpromo", "ask"),
            ("interaction", "ask"),
            ("intro", "show"),
            ("outro", "show"),
            ("preview", "show"),
            ("hook", "show"),
            ("music_offtopic", "skip"),
            ("filler", "off"),
            ("poi_highlight", "show"),
            ("exclusive_access", "show"),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
        Self {
            enabled: true,
            look: "classic".into(),
            start_face: "video".into(),
            start_page: "home".into(),
            use_account: true,
            default_quality: "auto".into(),
            prefer_codec: "auto".into(),
            audio_quality: "high".into(),
            autoplay_next: true,
            autoplay_on_open: true,
            preview_on_hover: true,
            normalize_volume: true,
            send_history: true,
            endless_radio: true,
            sponsorblock: true,
            sb_categories,
            sb_server: "https://sponsor.ajay.app".into(),
            sb_whitelist: Vec::new(),
            sb_download: "off".into(),
            dearrow: false,
            dearrow_thumbs: false,
            ryd: true,
            hide_shorts: false,
            hide_live: false,
            hide_upcoming: false,
            hide_members: true,
            hide_mixes: true,
            hide_playables: true,
            hide_posts: false,
            hide_watched: false,
            watched_threshold: 85,
            hide_home_feed: false,
            hide_related: false,
            hide_comments: false,
            hide_chips: false,
            hide_metrics: false,
            infinite_scroll: true,
            grid_columns: 0,
            blocked_channels: Vec::new(),
            blocked_words: Vec::new(),
            whole_word: true,
            min_duration_s: 0,
            max_duration_s: 0,
            lyrics_provider: "auto".into(),
            download_folder: None,
            storage_cap_gb: 0.0,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReadSettings {
    #[serde(default = "default_read_enabled")]
    pub enabled: bool,
}

fn default_read_enabled() -> bool {
    true
}

impl Default for ReadSettings {
    fn default() -> Self {
        Self {
            enabled: default_read_enabled(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LeagueSettings {
    #[serde(default = "default_league_enabled")]
    pub enabled: bool,
    #[serde(default)]
    pub auto_accept: bool,
    #[serde(default)]
    pub auto_accept_delay: u8,
    #[serde(default = "default_notify_ready_check")]
    pub notify_ready_check: bool,
    #[serde(default)]
    pub auto_pick: bool,
    #[serde(default)]
    pub auto_ban: bool,
    #[serde(default)]
    pub auto_ban_delay: u8,
    #[serde(default)]
    pub auto_lock: bool,
    #[serde(default)]
    pub auto_lock_at_timeout: bool,
    #[serde(default)]
    pub auto_runes: bool,
    #[serde(default)]
    pub auto_honor: bool,
    #[serde(default)]
    pub auto_play_again: bool,
    #[serde(default)]
    pub auto_requeue: bool,
    #[serde(default)]
    pub auto_accept_swaps: bool,
    #[serde(default)]
    pub auto_reconnect: bool,
    #[serde(default)]
    pub auto_trade: String,
    #[serde(default)]
    pub auto_message: String,
    #[serde(default)]
    pub pick_champions: Vec<i64>,
    #[serde(default)]
    pub ban_champions: Vec<i64>,
    #[serde(default)]
    pub pick_random: bool,
    #[serde(default)]
    pub skin_roulette: bool,
    #[serde(default)]
    pub skin_roulette_include_base: bool,
    #[serde(default)]
    pub ward_roulette: bool,
    #[serde(default = "default_sgp_enabled")]
    pub sgp_enabled: bool,
    #[serde(default = "default_coach_style")]
    pub coach_style: String,
}

fn default_sgp_enabled() -> bool {
    true
}

fn default_coach_style() -> String {
    "objective".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WashMyBellySettings {
    #[serde(default = "default_wmb_enabled")]
    pub enabled: bool,
    #[serde(default = "default_wmb_quarantine_days")]
    pub quarantine_days: u32,
    #[serde(default)]
    pub protected_paths_extra: Vec<String>,
    #[serde(default)]
    pub auto_hold_power: bool,
    #[serde(default)]
    pub perf_tool_path: String,
}

fn default_wmb_enabled() -> bool {
    true
}

fn default_wmb_quarantine_days() -> u32 {
    7
}

impl Default for WashMyBellySettings {
    fn default() -> Self {
        Self {
            enabled: default_wmb_enabled(),
            quarantine_days: default_wmb_quarantine_days(),
            protected_paths_extra: Vec::new(),
            auto_hold_power: false,
            perf_tool_path: String::new(),
        }
    }
}

fn default_league_enabled() -> bool {
    true
}

fn default_notify_ready_check() -> bool {
    true
}

impl Default for LeagueSettings {
    fn default() -> Self {
        Self {
            enabled: default_league_enabled(),
            auto_accept: false,
            auto_accept_delay: 0,
            notify_ready_check: default_notify_ready_check(),
            auto_pick: false,
            auto_ban: false,
            auto_ban_delay: 0,
            auto_lock: false,
            auto_lock_at_timeout: false,
            auto_runes: false,
            auto_honor: false,
            auto_play_again: false,
            auto_requeue: false,
            auto_accept_swaps: false,
            auto_reconnect: false,
            auto_trade: String::new(),
            auto_message: String::new(),
            pick_champions: Vec::new(),
            ban_champions: Vec::new(),
            pick_random: false,
            skin_roulette: false,
            skin_roulette_include_base: false,
            ward_roulette: false,
            sgp_enabled: default_sgp_enabled(),
            coach_style: default_coach_style(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RpcSettings {
    #[serde(default = "default_rpc_enabled")]
    pub enabled: bool,
    #[serde(default = "default_rpc_app_id")]
    pub app_id: String,
    #[serde(default = "default_rpc_image_key")]
    pub large_image_key: String,
}

impl Default for RpcSettings {
    fn default() -> Self {
        Self {
            enabled: default_rpc_enabled(),
            app_id: default_rpc_app_id(),
            large_image_key: default_rpc_image_key(),
        }
    }
}

fn default_rpc_enabled() -> bool {
    true
}

fn default_rpc_app_id() -> String {
    "1502867748353478656".into()
}

fn default_rpc_image_key() -> String {
    "omniget_music".into()
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct LastDownloadOptions {
    #[serde(default)]
    pub mode: Option<String>,
    #[serde(default)]
    pub quality: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppearanceSettings {
    pub theme: String,
    pub language: String,
    #[serde(default)]
    pub sidebar_collapsed: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preset: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub legacy_theme: Option<String>,
    #[serde(default, skip_serializing_if = "is_zero_u32")]
    pub migration_version: u32,
}

fn is_zero_u32(v: &u32) -> bool {
    *v == 0
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DownloadSettings {
    pub default_output_dir: PathBuf,
    #[serde(default)]
    pub saved_output_dirs: Vec<String>,
    pub always_ask_path: bool,
    pub video_quality: String,
    pub skip_existing: bool,
    #[serde(default = "default_true")]
    pub embed_metadata: bool,
    #[serde(default = "default_true")]
    pub embed_thumbnail: bool,
    #[serde(default)]
    pub playlist_number_files: bool,
    #[serde(default = "default_video_compatibility")]
    pub video_compatibility: String,
    #[serde(default)]
    pub write_nfo_sidecar: bool,
    #[serde(default)]
    pub clipboard_detection: bool,
    #[serde(default)]
    pub auto_download_on_paste: bool,
    #[serde(default = "default_filename_template")]
    pub filename_template: String,
    #[serde(default)]
    pub organize_by_platform: bool,
    #[serde(default)]
    pub download_subtitles: bool,
    #[serde(default)]
    pub include_auto_subtitles: bool,
    #[serde(default = "default_caption_locale")]
    pub caption_locale: String,
    #[serde(default)]
    pub keep_vtt: bool,
    #[serde(default)]
    pub translate_metadata: bool,
    #[serde(default)]
    pub youtube_sponsorblock: bool,
    #[serde(default = "default_sponsorblock_mode")]
    pub sponsorblock_mode: String,
    #[serde(default = "default_sponsorblock_categories")]
    pub sponsorblock_categories: Vec<String>,
    #[serde(default)]
    pub split_by_chapters: bool,
    #[serde(default)]
    pub live_from_start: bool,
    #[serde(default)]
    pub speed_limit: String,
    #[serde(default)]
    pub hotkey_enabled: bool,
    #[serde(default = "default_hotkey_binding")]
    pub hotkey_binding: String,
    #[serde(default)]
    pub music_hotkey_enabled: bool,
    #[serde(default = "default_music_hotkey_binding")]
    pub music_hotkey_binding: String,
    #[serde(default = "default_music_audio_format")]
    pub music_audio_format: String,
    #[serde(default = "default_true")]
    pub copy_to_clipboard_on_hotkey: bool,
    #[serde(default)]
    pub cookie_file: String,
    #[serde(default = "default_true")]
    pub always_use_managed_cookies: bool,
    #[serde(default)]
    pub bilibili_danmaku_enabled: bool,
    #[serde(default = "default_bilibili_danmaku_format")]
    pub bilibili_danmaku_format: String,
    #[serde(default = "default_bilibili_container")]
    pub bilibili_container: String,
    #[serde(default)]
    pub bilibili_nfo_enabled: bool,
    #[serde(default)]
    pub bilibili_cover_sidecar: bool,
    #[serde(default = "default_bilibili_cover_format")]
    pub bilibili_cover_format: String,
    #[serde(default = "default_bilibili_naming_video")]
    pub bilibili_naming_video: String,
    #[serde(default = "default_bilibili_naming_multi_part")]
    pub bilibili_naming_multi_part: String,
    #[serde(default = "default_bilibili_naming_bangumi")]
    pub bilibili_naming_bangumi: String,
    #[serde(default = "default_bilibili_naming_cheese")]
    pub bilibili_naming_cheese: String,
    #[serde(default = "default_bilibili_naming_collection")]
    pub bilibili_naming_collection: String,
    #[serde(default)]
    pub bilibili_cdn_hosts: String,
    #[serde(default)]
    pub bilibili_cdn_prefer_alternatives: bool,
    #[serde(default = "default_bilibili_preferred_qn")]
    pub bilibili_preferred_qn: u32,
    #[serde(default = "default_bilibili_preferred_codec")]
    pub bilibili_preferred_codec: u32,
    #[serde(default = "default_bilibili_preferred_audio_qn")]
    pub bilibili_preferred_audio_qn: u32,
}

fn default_bilibili_preferred_qn() -> u32 {
    200
}

fn default_bilibili_preferred_codec() -> u32 {
    20
}

fn default_bilibili_preferred_audio_qn() -> u32 {
    30300
}

fn default_bilibili_cover_format() -> String {
    "jpg".to_string()
}

fn default_bilibili_naming_video() -> String {
    "{title}".to_string()
}

fn default_bilibili_naming_multi_part() -> String {
    "{parent_title}/P{page} - {leaf_title}".to_string()
}

fn default_bilibili_naming_bangumi() -> String {
    "{series_title}/Season {season_number}/{episode_number_pad2} - {episode_title}".to_string()
}

fn default_bilibili_naming_cheese() -> String {
    "{series_title}/{section_title}/{episode_number_pad2} - {episode_title}".to_string()
}

fn default_bilibili_naming_collection() -> String {
    "{collection_title}/{title}".to_string()
}

fn default_bilibili_danmaku_format() -> String {
    "xml".to_string()
}

fn default_bilibili_container() -> String {
    "mp4".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdvancedSettings {
    pub max_concurrent_segments: u32,
    pub max_retries: u32,
    #[serde(default = "default_max_concurrent_downloads")]
    pub max_concurrent_downloads: u32,
    #[serde(default = "default_concurrent_fragments")]
    pub concurrent_fragments: u32,
    #[serde(default = "default_stagger_delay_ms")]
    pub stagger_delay_ms: u64,
    #[serde(default = "default_torrent_listen_port")]
    pub torrent_listen_port: u16,
    #[serde(default = "default_torrent_auto_trackers")]
    pub torrent_auto_trackers: bool,
    #[serde(default = "default_torrent_upnp")]
    pub torrent_upnp: bool,
    #[serde(default)]
    pub torrent_stop_seeding: bool,
    #[serde(default = "default_prevent_sleep")]
    pub prevent_sleep: bool,
    #[serde(default)]
    pub cookies_from_browser: String,
    #[serde(default)]
    pub twitter_manual_cookie: String,
    #[serde(default)]
    pub user_agent: String,
    #[serde(default)]
    pub insecure_tls: bool,
}

fn default_concurrent_fragments() -> u32 {
    8
}

fn default_sponsorblock_mode() -> String {
    "remove".to_string()
}

fn default_sponsorblock_categories() -> Vec<String> {
    vec![
        "sponsor".to_string(),
        "selfpromo".to_string(),
        "interaction".to_string(),
    ]
}

fn default_max_concurrent_downloads() -> u32 {
    2
}

fn default_stagger_delay_ms() -> u64 {
    150
}

fn default_torrent_auto_trackers() -> bool {
    true
}

fn default_torrent_upnp() -> bool {
    true
}

fn default_prevent_sleep() -> bool {
    true
}

fn default_torrent_listen_port() -> u16 {
    6881
}

fn default_true() -> bool {
    true
}

fn default_video_compatibility() -> String {
    "best".into()
}

pub fn default_filename_template() -> String {
    "%(title).200s [%(id)s].%(ext)s".into()
}

fn default_hotkey_binding() -> String {
    "CmdOrCtrl+Shift+D".into()
}

fn default_music_hotkey_binding() -> String {
    "CmdOrCtrl+Shift+M".into()
}

fn default_music_audio_format() -> String {
    "m4a".into()
}

fn default_caption_locale() -> String {
    "en".into()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TelegramSettings {
    pub concurrent_downloads: u32,
    pub fix_file_extensions: bool,
    #[serde(default = "default_telegram_enabled")]
    pub enabled: bool,
    #[serde(default)]
    pub prefs: serde_json::Value,
}

fn default_telegram_enabled() -> bool {
    true
}

impl Default for TelegramSettings {
    fn default() -> Self {
        Self {
            concurrent_downloads: 3,
            fix_file_extensions: true,
            enabled: default_telegram_enabled(),
            prefs: serde_json::Value::Object(Default::default()),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProxySettings {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_proxy_type")]
    pub proxy_type: String,
    #[serde(default)]
    pub host: String,
    #[serde(default = "default_proxy_port")]
    pub port: u16,
    #[serde(default)]
    pub username: String,
    #[serde(default)]
    pub password: String,
}

impl Default for ProxySettings {
    fn default() -> Self {
        Self {
            enabled: false,
            proxy_type: default_proxy_type(),
            host: String::new(),
            port: default_proxy_port(),
            username: String::new(),
            password: String::new(),
        }
    }
}

fn default_proxy_type() -> String {
    "http".into()
}

fn default_proxy_port() -> u16 {
    8080
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TypographySettings {
    #[serde(default = "default_font_display")]
    pub font_display: String,
    #[serde(default = "default_font_body")]
    pub font_body: String,
    #[serde(default = "default_font_mono")]
    pub font_mono: String,
    #[serde(default = "default_line_height_base")]
    pub line_height_base: f32,
    #[serde(default = "default_spacing_scale")]
    pub spacing_scale: f32,
    #[serde(default)]
    pub preset_name: Option<String>,
}

fn default_font_display() -> String {
    "Bricolage Grotesque Variable".into()
}

fn default_font_body() -> String {
    "Inter".into()
}

fn default_font_mono() -> String {
    "IBM Plex Mono".into()
}

fn default_line_height_base() -> f32 {
    1.55
}

fn default_spacing_scale() -> f32 {
    1.0
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BridgeSettings {
    #[serde(default = "default_bridge_enabled")]
    pub enabled: bool,
    #[serde(default)]
    pub port: u16,
    #[serde(default)]
    pub token: String,
    #[serde(default = "default_bridge_enabled")]
    pub legacy_agent_routes: bool,
    #[serde(default)]
    pub canvas_port: u16,
}

impl Default for BridgeSettings {
    fn default() -> Self {
        Self {
            enabled: default_bridge_enabled(),
            port: 0,
            token: String::new(),
            legacy_agent_routes: true,
            canvas_port: 0,
        }
    }
}

fn default_bridge_enabled() -> bool {
    true
}

impl Default for TypographySettings {
    fn default() -> Self {
        Self {
            font_display: default_font_display(),
            font_body: default_font_body(),
            font_mono: default_font_mono(),
            line_height_base: default_line_height_base(),
            spacing_scale: default_spacing_scale(),
            preset_name: Some("omniget-default".into()),
        }
    }
}

impl AppSettings {
    pub fn migrate(&mut self) -> bool {
        if self.schema_version >= SETTINGS_SCHEMA_VERSION {
            return false;
        }
        if self.schema_version < 2 {
            self.world.enabled = true;
        }
        self.schema_version = SETTINGS_SCHEMA_VERSION;
        true
    }
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            schema_version: SETTINGS_SCHEMA_VERSION,
            appearance: AppearanceSettings {
                theme: "system".into(),
                language: "en".into(),
                sidebar_collapsed: false,
                mode: None,
                preset: None,
                legacy_theme: None,
                migration_version: 0,
            },
            download: DownloadSettings {
                default_output_dir: dirs::download_dir().unwrap_or_else(|| PathBuf::from(".")),
                saved_output_dirs: Vec::new(),
                always_ask_path: false,
                video_quality: "720p".into(),
                skip_existing: true,
                embed_metadata: true,
                embed_thumbnail: true,
                playlist_number_files: false,
                video_compatibility: default_video_compatibility(),
                write_nfo_sidecar: false,
                clipboard_detection: false,
                auto_download_on_paste: false,
                filename_template: default_filename_template(),
                organize_by_platform: false,
                download_subtitles: false,
                include_auto_subtitles: false,
                caption_locale: default_caption_locale(),
                keep_vtt: false,
                translate_metadata: false,
                youtube_sponsorblock: false,
                sponsorblock_mode: "remove".to_string(),
                sponsorblock_categories: vec![
                    "sponsor".to_string(),
                    "selfpromo".to_string(),
                    "interaction".to_string(),
                ],
                split_by_chapters: false,
                live_from_start: false,
                speed_limit: String::new(),
                hotkey_enabled: false,
                hotkey_binding: default_hotkey_binding(),
                music_hotkey_enabled: false,
                music_hotkey_binding: default_music_hotkey_binding(),
                music_audio_format: default_music_audio_format(),
                copy_to_clipboard_on_hotkey: true,
                cookie_file: String::new(),
                always_use_managed_cookies: true,
                bilibili_danmaku_enabled: false,
                bilibili_danmaku_format: "xml".into(),
                bilibili_container: "mp4".into(),
                bilibili_nfo_enabled: false,
                bilibili_cover_sidecar: false,
                bilibili_cover_format: "jpg".into(),
                bilibili_naming_video: default_bilibili_naming_video(),
                bilibili_naming_multi_part: default_bilibili_naming_multi_part(),
                bilibili_naming_bangumi: default_bilibili_naming_bangumi(),
                bilibili_naming_cheese: default_bilibili_naming_cheese(),
                bilibili_naming_collection: default_bilibili_naming_collection(),
                bilibili_cdn_hosts: String::new(),
                bilibili_cdn_prefer_alternatives: false,
                bilibili_preferred_qn: default_bilibili_preferred_qn(),
                bilibili_preferred_codec: default_bilibili_preferred_codec(),
                bilibili_preferred_audio_qn: default_bilibili_preferred_audio_qn(),
            },
            advanced: AdvancedSettings {
                max_concurrent_segments: 20,
                max_retries: 3,
                max_concurrent_downloads: 2,
                concurrent_fragments: 8,
                stagger_delay_ms: 150,
                torrent_listen_port: 6881,
                torrent_auto_trackers: true,
                torrent_upnp: true,
                torrent_stop_seeding: false,
                prevent_sleep: true,
                cookies_from_browser: String::new(),
                twitter_manual_cookie: String::new(),
                user_agent: String::new(),
                insecure_tls: false,
            },
            telegram: TelegramSettings::default(),
            proxy: ProxySettings::default(),
            onboarding_completed: false,
            start_with_system: false,
            start_minimized: false,
            exit_on_close: false,
            run_in_background: false,
            portable_mode: false,
            legal_acknowledged: false,
            last_download_options: LastDownloadOptions::default(),
            typography: TypographySettings::default(),
            rpc: RpcSettings::default(),
            bridge: BridgeSettings::default(),
            league: LeagueSettings::default(),
            music: MusicSettings::default(),
            video: VideoSettings::default(),
            tube: TubeSettings::default(),
            washmybelly: WashMyBellySettings::default(),
            read: ReadSettings::default(),
            accessibility: AccessibilitySettings::default(),
            world: WorldSettings::default(),
            llm: LlmSettings::default(),
            canvas: CanvasSettings::default(),
        }
    }
}

pub use v3::*;

#[path = "settings_v3.rs"]
mod v3;

#[cfg(test)]
mod backcompat_tests {
    use super::*;

    #[test]
    fn canvas_settings_snap_and_reset_unknown_choices() {
        assert_eq!(parse_canvas_size("333x211"), Some((340.0, 220.0)));
        assert_eq!(parse_canvas_size("nope"), None);
        let mut c = CanvasSettings {
            selection_style: "cornerDots".into(),
            cord_style: "spaghetti".into(),
            zoom_step: 7.0,
            zoom_modifier: "option".into(),
            ..CanvasSettings::default()
        };
        c.defaults.note.size = "333x211".into();
        c.power.prevent_sleep = "whileWorking".into();
        c.images.quality = "Sharp".into();
        c.performance.mounted_terminals = 0;
        c.performance.live_portals = 99;
        c.normalize();
        assert_eq!(c.images.quality, "sharp");
        assert_eq!(c.image_budget_mb(), 512);
        assert_eq!(c.performance.mounted_terminals, 1);
        assert_eq!(c.performance.live_portals, 16);
        assert_eq!(c.selection_style, "corner_dots");
        assert_eq!(c.cord_style, "bezier");
        assert_eq!(c.zoom_step, 1.0);
        assert_eq!(c.zoom_modifier, "alt");
        assert_eq!(c.defaults.note.size, "340x220");
        assert_eq!(c.power.prevent_sleep, "while_working");
        let again = c.clone();
        c.normalize();
        assert_eq!(c, again);
        let mut old = serde_json::to_value(AppSettings::default()).unwrap();
        old.as_object_mut().unwrap().remove("canvas");
        let s: AppSettings = serde_json::from_value(old).unwrap();
        assert_eq!(s.canvas, CanvasSettings::default());
    }

    #[test]
    fn an_install_from_before_the_world_gets_it_switched_on_once() {
        let mut old = AppSettings {
            schema_version: 1,
            ..AppSettings::default()
        };
        old.world.enabled = false;
        assert!(old.migrate());
        assert!(old.world.enabled);
        old.world.enabled = false;
        assert!(!old.migrate());
        assert!(!old.world.enabled);
    }

    #[test]
    fn settings_com_campos_da_cidade_e_do_omnidisc_ainda_carregam() {
        let mut json = serde_json::to_value(AppSettings::default()).unwrap();
        json["omnidisc"] =
            serde_json::json!({ "voice": { "ptt_key": "F8", "vad_threshold_db": -40.0 } });
        json["world"] = serde_json::json!({
            "enabled": false,
            "tier_override": 2,
            "backend_override": "gl2",
            "city_backend": "3d",
            "tier_measured": 3,
            "measured_median_ms": 4.5,
            "measured_app_version": "1.0.0",
            "city_server": "https://chat.tonho.wtf"
        });
        let parsed: AppSettings =
            serde_json::from_value(json).expect("settings antigo precisa desserializar");
        assert!(!parsed.world.enabled);
        let back = serde_json::to_value(&parsed).unwrap();
        assert!(back.get("omnidisc").is_none());
        assert_eq!(back["world"], serde_json::json!({ "enabled": false }));
    }

    #[test]
    fn settings_da_v0_7_6_carregam_com_tls_verificado() {
        let json = serde_json::json!({
            "max_concurrent_segments": 20,
            "max_retries": 3,
            "max_concurrent_downloads": 4,
            "concurrent_fragments": 8,
            "prevent_sleep": false,
            "cookies_from_browser": "",
            "twitter_manual_cookie": "",
            "user_agent": "Mozilla/5.0 teste"
        });
        let parsed: AdvancedSettings =
            serde_json::from_value(json).expect("settings antigo precisa desserializar");

        assert!(
            !parsed.insecure_tls,
            "campo ausente precisa virar verificacao de certificado LIGADA"
        );
        assert_eq!(parsed.max_concurrent_downloads, 4);
        assert_eq!(parsed.concurrent_fragments, 8);
        assert!(!parsed.prevent_sleep);
        assert_eq!(parsed.user_agent, "Mozilla/5.0 teste");
    }

    #[test]
    fn insecure_tls_ligado_sobrevive_ao_round_trip() {
        let json = serde_json::json!({
            "max_concurrent_segments": 20,
            "max_retries": 3,
            "insecure_tls": true
        });
        let parsed: AdvancedSettings = serde_json::from_value(json).expect("desserializa");
        assert!(parsed.insecure_tls);
        let round = serde_json::to_value(&parsed).expect("serializa");
        let back: AdvancedSettings = serde_json::from_value(round).expect("volta");
        assert!(back.insecure_tls);
    }

    #[test]
    fn settings_json_sem_accessibility_ainda_abre() {
        let atual = serde_json::to_value(AppSettings::default()).expect("serializa");
        let mut anterior = atual.clone();
        let obj = anterior.as_object_mut().expect("objeto");
        let removida = obj.remove("accessibility");
        assert!(
            removida.is_some(),
            "o campo tem que existir hoje, senao o teste nao prova nada"
        );

        let parsed: AppSettings =
            serde_json::from_value(anterior).expect("arquivo pre-0.8.0 tem que abrir");

        assert!(!parsed.accessibility.reduce_motion);
        assert!(!parsed.accessibility.reduce_transparency);
        assert_eq!(
            parsed.appearance.theme,
            AppSettings::default().appearance.theme
        );
        assert_eq!(parsed.schema_version, AppSettings::default().schema_version);
    }

    #[test]
    fn acessibilidade_ligada_sobrevive_ao_round_trip() {
        let mut s = AppSettings::default();
        s.accessibility.reduce_motion = true;
        s.accessibility.reduce_transparency = true;
        let round = serde_json::to_value(&s).expect("serializa");
        let back: AppSettings = serde_json::from_value(round).expect("volta");
        assert!(back.accessibility.reduce_motion);
        assert!(back.accessibility.reduce_transparency);
    }

    #[test]
    fn settings_json_sem_saved_output_dirs_ainda_abre() {
        let atual = serde_json::to_value(AppSettings::default()).expect("serializa");
        let mut anterior = atual.clone();
        let download = anterior
            .get_mut("download")
            .and_then(|v| v.as_object_mut())
            .expect("download");
        let removida = download.remove("saved_output_dirs");
        assert!(
            removida.is_some(),
            "o campo tem que existir hoje, senao o teste nao prova nada"
        );

        let parsed: AppSettings =
            serde_json::from_value(anterior).expect("arquivo antigo tem que abrir");
        assert!(parsed.download.saved_output_dirs.is_empty());
        assert_eq!(
            parsed.download.default_output_dir,
            AppSettings::default().download.default_output_dir
        );
    }

    #[test]
    fn saved_output_dirs_sobrevive_ao_round_trip() {
        let mut s = AppSettings::default();
        s.download.saved_output_dirs = vec!["/tmp/custom".into(), "/tmp/other".into()];
        let round = serde_json::to_value(&s).expect("serializa");
        let back: AppSettings = serde_json::from_value(round).expect("volta");
        assert_eq!(
            back.download.saved_output_dirs,
            vec!["/tmp/custom".to_string(), "/tmp/other".to_string()]
        );
        assert_eq!(
            back.download.default_output_dir,
            s.download.default_output_dir
        );
    }

    #[test]
    fn settings_json_without_exit_on_close_keeps_the_tray() {
        let current = serde_json::to_value(AppSettings::default()).expect("serialize");
        let mut previous = current.clone();
        let removed = previous
            .as_object_mut()
            .expect("object")
            .remove("exit_on_close");
        assert!(
            removed.is_some(),
            "the field has to exist today, or the test proves nothing"
        );

        let parsed: AppSettings =
            serde_json::from_value(previous).expect("an old file has to load");
        assert!(!parsed.exit_on_close);
    }

    #[test]
    fn exit_on_close_survives_the_round_trip() {
        let s = AppSettings {
            exit_on_close: true,
            ..AppSettings::default()
        };
        let round = serde_json::to_value(&s).expect("serialize");
        let back: AppSettings = serde_json::from_value(round).expect("deserialize");
        assert!(back.exit_on_close);
    }

    #[test]
    fn an_llm_block_without_the_watchdog_fields_loads_the_defaults() {
        let old = serde_json::json!({
            "prune_enabled": true,
            "prune_judge": "local",
            "prune_min_tokens": 40000
        });
        let llm: LlmSettings = serde_json::from_value(old).expect("an old file has to load");
        assert_eq!(llm.turn_idle_timeout_s, 600);
        assert_eq!(llm.turn_tool_idle_timeout_s, 1800);
        assert_eq!(llm.turn_start_timeout_s, 90);
        assert_eq!(llm.job_default_deadline_min, 60);
        assert_eq!(llm.ask_timeout_chat_s, 1800);
        assert_eq!(llm.ask_timeout_job_s, None);
        assert_eq!(llm.approval_notify_delay_s, 5);
        let mut all = serde_json::to_value(AppSettings::default()).expect("serialize");
        all.as_object_mut().expect("object").remove("llm");
        let parsed: AppSettings = serde_json::from_value(all).expect("loads");
        assert_eq!(parsed.llm.turn_idle_timeout_s, 600);
    }

    #[test]
    fn an_llm_block_without_the_jobs_fields_loads_the_defaults() {
        let llm: LlmSettings =
            serde_json::from_value(serde_json::json!({ "prune_enabled": false })).expect("loads");
        assert_eq!(llm.jobs_max_parallel, 4);
        assert_eq!(llm.jobs_max_per_agent, None);
        assert_eq!(LlmSettings::default().jobs_max_parallel, 4);
        let back: LlmSettings =
            serde_json::from_value(serde_json::to_value(&llm).unwrap()).expect("round trip");
        assert_eq!(back.jobs_max_parallel, 4);
    }
}

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
    pub washmybelly: WashMyBellySettings,
    #[serde(default)]
    pub accessibility: AccessibilitySettings,
    #[serde(default)]
    pub omnidisc: OmnidiscSettings,
    #[serde(default)]
    pub world: WorldSettings,
    #[serde(default)]
    pub llm: LlmSettings,
}

/// Settings of the `/llm` section that are not per-agent.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmSettings {
    /// Context pruning master switch. Off: the Coordinator keeps a disabled
    /// pruner and nothing is judged or rewritten.
    #[serde(default)]
    pub prune_enabled: bool,
    /// `"local"` (offline MiniLM) or `"jev"` (TypeSafe System One). Anything
    /// unknown parses as local, so a typo never sends spans to a third party.
    /// The Jev key is not here: it lives in the secret store.
    #[serde(default = "default_prune_judge")]
    pub prune_judge: String,
    /// Conversation size (estimated tokens) under which nothing is judged.
    /// The ported default suits cloud models; a small local context wants less.
    #[serde(default = "default_prune_min_tokens")]
    pub prune_min_tokens: u32,
    /// A CLI turn (`claude -p`, `codex exec`) with no stdout line for this
    /// long is stuck: its process tree is killed and the job fails `stalled`.
    /// 600 s is ~39x the longest silence measured over the 33 missions x 3
    /// reps of round r2 (15.3 s, `estudos/87-onda1/sp4-stream-gaps.json`),
    /// kept generous because those missions never compact nor think long.
    #[serde(default = "default_turn_idle_timeout_s")]
    pub turn_idle_timeout_s: u32,
    /// The same window while a tool the CLI started has not answered yet
    /// (`tool_use` without `tool_result`): a test suite or a build is silent
    /// by nature, and killing it would lose real work.
    #[serde(default = "default_turn_tool_idle_timeout_s")]
    pub turn_tool_idle_timeout_s: u32,
    /// No line at all this long after the spawn = the prompt never took
    /// (`ERR_CLI_NO_START`). The stream's `system/init` comes before any
    /// model call, so a healthy CLI prints within a couple of seconds.
    #[serde(default = "default_turn_start_timeout_s")]
    pub turn_start_timeout_s: u32,
    /// Wall-clock ceiling of every background job (`run`, `loop` round,
    /// `trigger`) that does not set its own. A mission keeps its own guard.
    #[serde(default = "default_job_deadline_min")]
    pub job_default_deadline_min: u32,
    /// How long a permission question of the chat waits for an answer. The
    /// old fixed 120 s denied in silence whenever nobody looked in time.
    #[serde(default = "default_ask_timeout_chat_s")]
    pub ask_timeout_chat_s: u32,
    /// The same for a background job. `None` = until the job's own deadline,
    /// so a question never outlives the job nor dies before it.
    #[serde(default)]
    pub ask_timeout_job_s: Option<u32>,
    /// A question still unanswered this long after it was raised becomes an
    /// OS notification ("needs you"). One answered sooner never notifies.
    #[serde(default = "default_approval_notify_delay_s")]
    pub approval_notify_delay_s: u32,
    /// Background jobs (runs, Loop rounds, triggers, mission tasks) running
    /// at the same time, 1–16. The old fixed 2 queued the third job behind
    /// any mission (W1, W2); 4 is Orca's coordinator default (`78#P4`).
    #[serde(default = "default_jobs_max_parallel")]
    pub jobs_max_parallel: u32,
    /// Jobs of one agent at the same time; `None` = only the global cap.
    #[serde(default)]
    pub jobs_max_per_agent: Option<u32>,
}

fn default_prune_min_tokens() -> u32 {
    50_000
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

fn default_prune_judge() -> String {
    "local".into()
}

impl Default for LlmSettings {
    fn default() -> Self {
        Self {
            prune_enabled: false,
            prune_judge: default_prune_judge(),
            prune_min_tokens: default_prune_min_tokens(),
            turn_idle_timeout_s: default_turn_idle_timeout_s(),
            turn_tool_idle_timeout_s: default_turn_tool_idle_timeout_s(),
            turn_start_timeout_s: default_turn_start_timeout_s(),
            job_default_deadline_min: default_job_deadline_min(),
            ask_timeout_chat_s: default_ask_timeout_chat_s(),
            ask_timeout_job_s: None,
            approval_notify_delay_s: default_approval_notify_delay_s(),
            jobs_max_parallel: default_jobs_max_parallel(),
            jobs_max_per_agent: None,
        }
    }
}

/// Settings of the agents' world (`/world`). Phase 6 only carries the render
/// tier: `tier_override` pins a tier by hand (None = automatic, i.e. measured),
/// and the `tier_measured*` trio is the last calibration, kept so the UI can show
/// it and so a version change can trigger a recalibration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorldSettings {
    /// Shows `/world` in the navigation.
    #[serde(default = "default_world_enabled")]
    pub enabled: bool,
    #[serde(default)]
    pub tier_override: Option<u8>,
    /// Pins the renderer backend (`gl2`, `gl1`, `canvas2d`); None picks by tier.
    #[serde(default)]
    pub backend_override: Option<String>,
    #[serde(default)]
    pub tier_measured: Option<u8>,
    #[serde(default)]
    pub measured_median_ms: Option<f64>,
    #[serde(default)]
    pub measured_app_version: Option<String>,
    /// Lets the agents in the house think through the LLM stack (plan, reflect).
    /// Off by default: it is the only part of the world that spends tokens.
    #[serde(default)]
    pub thinking: bool,
    /// Minimum real seconds between two thoughts in the whole house.
    #[serde(default = "default_think_interval_s")]
    pub think_interval_s: u32,
    /// Room server for open houses and visits (`wss://host/v1/room`). Empty =
    /// the built-in default; a self-hosted `omniworld-server` goes here.
    #[serde(default)]
    pub room_server: String,
    /// OmniDisc instance that hosts the city (`/world/v1`). Empty means the
    /// public one, `https://chat.tonho.wtf`.
    #[serde(default)]
    pub city_server: String,
}

fn default_world_enabled() -> bool {
    true
}

fn default_think_interval_s() -> u32 {
    120
}

impl Default for WorldSettings {
    fn default() -> Self {
        Self {
            enabled: default_world_enabled(),
            tier_override: None,
            backend_override: None,
            tier_measured: None,
            measured_median_ms: None,
            measured_app_version: None,
            thinking: false,
            think_interval_s: default_think_interval_s(),
            room_server: String::new(),
            city_server: String::new(),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct OmnidiscSettings {
    #[serde(default)]
    pub voice: OmnidiscVoiceSettings,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OmnidiscVoiceSettings {
    #[serde(default)]
    pub ptt_key: String,
    #[serde(default = "default_true")]
    pub noise_suppression: bool,
    #[serde(default)]
    pub input_device: Option<String>,
    #[serde(default)]
    pub output_device: Option<String>,
    #[serde(default = "default_vad_threshold_db")]
    pub vad_threshold_db: f32,
    /// How much of everyone else's audio to duck while you speak, 0-100 %.
    #[serde(default)]
    pub ducking_percent: u8,
    /// Force every connection through a TURN relay so the other side never
    /// learns your IP. Needs a TURN server on the instance.
    #[serde(default)]
    pub relay_only: bool,
}

impl Default for OmnidiscVoiceSettings {
    fn default() -> Self {
        Self {
            ptt_key: String::new(),
            noise_suppression: true,
            input_device: None,
            output_device: None,
            vad_threshold_db: default_vad_threshold_db(),
            ducking_percent: 0,
            relay_only: false,
        }
    }
}

fn default_vad_threshold_db() -> f32 {
    -45.0
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AccessibilitySettings {
    #[serde(default)]
    pub reduce_motion: bool,
    #[serde(default)]
    pub reduce_transparency: bool,
    #[serde(default)]
    pub disable_haptics: bool,
}

/// `/music` superpower. Player preferences (EQ, crossfade, volume) are per
/// device and live in the webview; only the on/off switch is here.
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

/// `/video` superpower (Watch). Player preferences live in `video.db`; only
/// the on/off switch is here.
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
    /// Auto-pick ignores the list and locks a random legal champion instead.
    #[serde(default)]
    pub pick_random: bool,
    /// Rolls a random owned skin (and chroma) the moment a champion is locked.
    #[serde(default)]
    pub skin_roulette: bool,
    #[serde(default)]
    pub skin_roulette_include_base: bool,
    /// Rolls a random owned ward skin on lock-in.
    #[serde(default)]
    pub ward_roulette: bool,
    /// Falls back to the client's backend (SGP) when the local history is thin.
    #[serde(default = "default_sgp_enabled")]
    pub sgp_enabled: bool,
    /// Tone of the AI coach: "objective", "roast" or "praise".
    #[serde(default = "default_coach_style")]
    pub coach_style: String,
}

fn default_sgp_enabled() -> bool {
    true
}

fn default_coach_style() -> String {
    "objective".to_string()
}

/// The washmybelly superpower (cleanup, uninstall, privacy, power, FPS).
/// On by default because nothing runs on its own: no scan starts until the
/// person opens the screen or an agent calls a `wmb_*` tool.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WashMyBellySettings {
    #[serde(default = "default_wmb_enabled")]
    pub enabled: bool,
    /// Days a quarantined item is kept before it is purged for good.
    #[serde(default = "default_wmb_quarantine_days")]
    pub quarantine_days: u32,
    /// Extra folders no rule may touch, on top of the built-in list.
    #[serde(default)]
    pub protected_paths_extra: Vec<String>,
    /// Hold the performance power profile while big downloads and agents run.
    #[serde(default)]
    pub auto_hold_power: bool,
    /// A PresentMon (Windows) or MangoHud path picked by the person.
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
    /// Sidebar shown as an icon-only rail.
    #[serde(default)]
    pub sidebar_collapsed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DownloadSettings {
    pub default_output_dir: PathBuf,
    #[serde(default)]
    pub saved_output_dirs: Vec<String>,
    pub always_ask_path: bool,
    pub video_quality: String,
    pub skip_existing: bool,
    pub download_descriptions: bool,
    #[serde(default = "default_true")]
    pub embed_metadata: bool,
    #[serde(default = "default_true")]
    pub embed_thumbnail: bool,
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
    pub clip_hotkey_enabled: bool,
    #[serde(default = "default_clip_hotkey_binding")]
    pub clip_hotkey_binding: String,
    #[serde(default)]
    pub music_hotkey_enabled: bool,
    #[serde(default = "default_music_hotkey_binding")]
    pub music_hotkey_binding: String,
    #[serde(default = "default_music_audio_format")]
    pub music_audio_format: String,
    #[serde(default)]
    pub extra_ytdlp_flags: Vec<String>,
    #[serde(default = "default_true")]
    pub copy_to_clipboard_on_hotkey: bool,
    #[serde(default)]
    pub cookie_file: String,
    #[serde(default = "default_true")]
    pub always_use_managed_cookies: bool,
    #[serde(default)]
    pub continuous_lecture_numbers: bool,
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
    #[serde(default = "default_prevent_sleep")]
    pub prevent_sleep: bool,
    #[serde(default)]
    pub cookies_from_browser: String,
    #[serde(default)]
    pub twitter_manual_cookie: String,
    #[serde(default)]
    pub user_agent: String,
    /// Desliga a verificação de certificado TLS do yt-dlp. Default `false`:
    /// só existe para rede corporativa com TLS interceptado.
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

pub fn default_filename_template() -> String {
    "%(title).200s [%(id)s].%(ext)s".into()
}

fn default_hotkey_binding() -> String {
    "CmdOrCtrl+Shift+D".into()
}

fn default_clip_hotkey_binding() -> String {
    "CmdOrCtrl+Shift+B".into()
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
}

impl Default for TelegramSettings {
    fn default() -> Self {
        Self {
            concurrent_downloads: 3,
            fix_file_extensions: true,
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

/// Local HTTP bridge that lets the browser extension talk to the desktop app
/// without depending on a Chrome native-messaging manifest (which would
/// otherwise force us to maintain an explicit allowlist of extension IDs).
///
/// The token is generated lazily on first launch and shown in the app
/// Settings; the user pastes it into the extension's options page once,
/// after which the extension authenticates each request with
/// `Authorization: Bearer <token>`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BridgeSettings {
    #[serde(default = "default_bridge_enabled")]
    pub enabled: bool,
    /// Bound port on `127.0.0.1`. `0` means "pick at startup, then persist".
    #[serde(default)]
    pub port: u16,
    /// Random token used to authenticate extension requests.
    /// Empty means "regenerate on next launch".
    #[serde(default)]
    pub token: String,
}

impl Default for BridgeSettings {
    fn default() -> Self {
        Self {
            enabled: default_bridge_enabled(),
            port: 0,
            token: String::new(),
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
    /// One-time upgrades of a stored settings file. Returns true when
    /// something changed and the file should be written back.
    ///
    /// v2 (0.10.0): the World left the experimental flag. Installs from before
    /// have `world.enabled: false` on disk and would never see `/world`.
    pub fn migrate(&mut self) -> bool {
        if self.schema_version >= 2 {
            return false;
        }
        self.world.enabled = true;
        self.schema_version = 2;
        true
    }
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            schema_version: 2,
            appearance: AppearanceSettings {
                theme: "system".into(),
                language: "en".into(),
                sidebar_collapsed: false,
            },
            download: DownloadSettings {
                default_output_dir: dirs::download_dir().unwrap_or_else(|| PathBuf::from(".")),
                saved_output_dirs: Vec::new(),
                always_ask_path: false,
                video_quality: "720p".into(),
                skip_existing: true,
                download_descriptions: true,
                embed_metadata: true,
                embed_thumbnail: true,
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
                clip_hotkey_enabled: false,
                clip_hotkey_binding: default_clip_hotkey_binding(),
                music_hotkey_enabled: false,
                music_hotkey_binding: default_music_hotkey_binding(),
                music_audio_format: default_music_audio_format(),
                extra_ytdlp_flags: Vec::new(),
                copy_to_clipboard_on_hotkey: true,
                cookie_file: String::new(),
                always_use_managed_cookies: true,
                continuous_lecture_numbers: false,
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
            portable_mode: false,
            legal_acknowledged: false,
            last_download_options: LastDownloadOptions::default(),
            typography: TypographySettings::default(),
            rpc: RpcSettings::default(),
            bridge: BridgeSettings::default(),
            league: LeagueSettings::default(),
            music: MusicSettings::default(),
            video: VideoSettings::default(),
            washmybelly: WashMyBellySettings::default(),
            accessibility: AccessibilitySettings::default(),
            omnidisc: OmnidiscSettings::default(),
            world: WorldSettings::default(),
            llm: LlmSettings::default(),
        }
    }
}

#[cfg(test)]
mod backcompat_tests {
    use super::*;

    #[test]
    fn an_install_from_before_the_world_gets_it_switched_on_once() {
        let mut old = AppSettings {
            schema_version: 1,
            ..AppSettings::default()
        };
        old.world.enabled = false;
        assert!(old.migrate());
        assert!(old.world.enabled);
        // The user's later choice is theirs: no second migration.
        old.world.enabled = false;
        assert!(!old.migrate());
        assert!(!old.world.enabled);
    }

    #[test]
    fn settings_da_v0_7_6_carregam_com_tls_verificado() {
        // settings.json de uma instalacao 0.7.6, antes de `insecure_tls`
        // existir. O campo ausente precisa virar `false` (certificado
        // verificado) sem derrubar a desserializacao nem tocar em nenhuma
        // outra preferencia de `advanced`. Essa e a categoria de bug que ja
        // zerou as preferencias de usuarios neste projeto.
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

    // A #220 adicionou `accessibility` ao AppSettings sem teste de migracao.
    // Um settings.json escrito antes da 0.8.0 nao tem esse campo; se ele perder
    // o `serde(default)` num refactor, todo usuario existente perde a
    // configuracao inteira, em silencio, no primeiro boot depois do update.
    //
    // Em vez de escrever um JSON antigo a mao (que envelhece e vira ficcao),
    // este teste serializa o default atual e *remove* a chave nova — que e
    // exatamente a forma de um arquivo gravado antes de o campo existir.
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
        // E o que o usuario ja tinha nao pode sumir junto.
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

    /// A settings file written before the watchdog knew nothing of it: the
    /// `/llm` block loads with the documented defaults, never zeros (a zero
    /// idle window would kill every turn on its first silence).
    #[test]
    fn an_llm_block_without_the_watchdog_fields_loads_the_defaults() {
        let old = serde_json::json!({
            "prune_enabled": true,
            "prune_judge": "local",
            "prune_min_tokens": 40000
        });
        let llm: LlmSettings = serde_json::from_value(old).expect("an old file has to load");
        assert!(llm.prune_enabled);
        assert_eq!(llm.turn_idle_timeout_s, 600);
        assert_eq!(llm.turn_tool_idle_timeout_s, 1800);
        assert_eq!(llm.turn_start_timeout_s, 90);
        assert_eq!(llm.job_default_deadline_min, 60);
        assert_eq!(llm.ask_timeout_chat_s, 1800);
        assert_eq!(llm.ask_timeout_job_s, None);
        assert_eq!(llm.approval_notify_delay_s, 5);
        // And a whole file without the block at all.
        let mut all = serde_json::to_value(AppSettings::default()).expect("serialize");
        all.as_object_mut().expect("object").remove("llm");
        let parsed: AppSettings = serde_json::from_value(all).expect("loads");
        assert_eq!(parsed.llm.turn_idle_timeout_s, 600);
    }

    /// 1.6: a settings file from before the parallel cap loads with 4 jobs
    /// at a time and no per-agent cap (the old constant was 2).
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

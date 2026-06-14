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
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DownloadSettings {
    pub default_output_dir: PathBuf,
    pub always_ask_path: bool,
    pub video_quality: String,
    pub skip_existing: bool,
    pub download_attachments: bool,
    pub download_descriptions: bool,
    #[serde(default = "default_true")]
    pub embed_metadata: bool,
    #[serde(default = "default_true")]
    pub embed_thumbnail: bool,
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

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
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

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            schema_version: 1,
            appearance: AppearanceSettings {
                theme: "system".into(),
                language: "en".into(),
            },
            download: DownloadSettings {
                default_output_dir: dirs::download_dir().unwrap_or_else(|| PathBuf::from(".")),
                always_ask_path: false,
                video_quality: "720p".into(),
                skip_existing: true,
                download_attachments: true,
                download_descriptions: true,
                embed_metadata: true,
                embed_thumbnail: true,
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
            },
            telegram: TelegramSettings::default(),
            proxy: ProxySettings::default(),
            onboarding_completed: false,
            start_with_system: false,
            start_minimized: false,
            portable_mode: false,
            legal_acknowledged: false,
            last_download_options: LastDownloadOptions::default(),
            typography: TypographySettings::default(),
            rpc: RpcSettings::default(),
            bridge: BridgeSettings::default(),
        }
    }
}

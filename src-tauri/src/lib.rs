use std::collections::HashMap;
use std::sync::Arc;

use platforms::hotmart::api::Course;
use platforms::hotmart::auth::HotmartSession;
use platforms::telegram::auth::{TelegramSessionHandle, TelegramState};
use tokio_util::sync::CancellationToken;

pub mod commands;
pub mod core;
pub mod models;
pub mod platforms;
pub mod storage;
pub mod tray;

pub struct CoursesCache {
    pub courses: Vec<Course>,
    pub fetched_at: std::time::Instant,
}

pub struct AppState {
    pub hotmart_session: Arc<tokio::sync::Mutex<Option<HotmartSession>>>,
    pub active_downloads: Arc<tokio::sync::Mutex<HashMap<u64, CancellationToken>>>,
    pub active_generic_downloads: Arc<tokio::sync::Mutex<HashMap<u64, (String, CancellationToken)>>>,
    pub active_conversions: Arc<tokio::sync::Mutex<HashMap<u64, CancellationToken>>>,
    pub registry: core::registry::PlatformRegistry,
    pub courses_cache: Arc<tokio::sync::Mutex<Option<CoursesCache>>>,
    pub session_validated_at: Arc<tokio::sync::Mutex<Option<std::time::Instant>>>,
    pub telegram_session: TelegramSessionHandle,
    pub download_queue: Arc<tokio::sync::Mutex<core::queue::DownloadQueue>>,
    pub auth_registry: core::auth::AuthRegistry,
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tracing_subscriber::fmt::init();

    let session = Arc::new(tokio::sync::Mutex::new(None));
    let telegram_session: TelegramSessionHandle =
        Arc::new(tokio::sync::Mutex::new(TelegramState::new()));

    let mut registry = core::registry::PlatformRegistry::new();
    registry.register(Arc::new(
        platforms::hotmart::downloader::HotmartDownloader::new(
            session.clone(),
            models::settings::AppSettings::default().download,
            20,
            3,
        ),
    ));
    registry.register(Arc::new(
        platforms::instagram::InstagramDownloader::new(),
    ));
    registry.register(Arc::new(
        platforms::pinterest::PinterestDownloader::new(),
    ));
    registry.register(Arc::new(
        platforms::tiktok::TikTokDownloader::new(),
    ));
    registry.register(Arc::new(
        platforms::twitter::TwitterDownloader::new(),
    ));
    registry.register(Arc::new(
        platforms::twitch::TwitchClipsDownloader::new(),
    ));
    registry.register(Arc::new(
        platforms::bluesky::BlueskyDownloader::new(),
    ));
    registry.register(Arc::new(
        platforms::reddit::RedditDownloader::new(),
    ));
    registry.register(Arc::new(
        platforms::youtube::YouTubeDownloader::new(),
    ));
    registry.register(Arc::new(
        platforms::telegram::downloader::TelegramDownloader::new(
            telegram_session.clone(),
        ),
    ));

    let mut auth_registry = core::auth::AuthRegistry::new();
    auth_registry.register(Arc::new(core::auth::SpotifyAuth::new(
        String::new(),
    )));
    auth_registry.register(Arc::new(core::auth::BrowserCookieAuth::new(
        "instagram",
        "https://www.instagram.com/accounts/login/",
        "instagram.com",
        ".instagram.com",
    )));

    let state = AppState {
        hotmart_session: session,
        active_downloads: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
        active_generic_downloads: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
        active_conversions: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
        registry,
        courses_cache: Arc::new(tokio::sync::Mutex::new(None)),
        session_validated_at: Arc::new(tokio::sync::Mutex::new(None)),
        telegram_session,
        download_queue: Arc::new(tokio::sync::Mutex::new(core::queue::DownloadQueue::new(2))),
        auth_registry,
    };

    tauri::Builder::default()
        .manage(state)
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_store::Builder::default().build())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .setup(|app| {
            tray::setup(app.handle())?;
            Ok(())
        })
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .invoke_handler(tauri::generate_handler![
            commands::auth::hotmart_login,
            commands::auth::hotmart_check_session,
            commands::auth::hotmart_logout,
            commands::courses::hotmart_list_courses,
            commands::courses::hotmart_refresh_courses,
            commands::courses::hotmart_get_modules,
            commands::downloads::start_course_download,
            commands::downloads::cancel_course_download,
            commands::downloads::get_active_downloads,
            commands::downloads::detect_platform,
            commands::downloads::get_media_formats,
            commands::downloads::download_from_url,
            commands::downloads::cancel_generic_download,
            commands::downloads::pause_download,
            commands::downloads::resume_download,
            commands::downloads::retry_download,
            commands::downloads::remove_download,
            commands::downloads::get_queue_state,
            commands::downloads::update_max_concurrent,
            commands::downloads::clear_finished_downloads,
            commands::downloads::reveal_file,
            commands::settings::get_settings,
            commands::settings::update_settings,
            commands::settings::reset_settings,
            commands::telegram::telegram_check_session,
            commands::telegram::telegram_qr_start,
            commands::telegram::telegram_qr_poll,
            commands::telegram::telegram_send_code,
            commands::telegram::telegram_verify_code,
            commands::telegram::telegram_verify_2fa,
            commands::telegram::telegram_logout,
            commands::telegram::telegram_list_chats,
            commands::telegram::telegram_list_media,
            commands::telegram::telegram_download_media,
            commands::telegram::telegram_download_batch,
            commands::telegram::telegram_cancel_batch,
            commands::convert::probe_file,
            commands::convert::convert_file,
            commands::convert::cancel_conversion,
            commands::convert::get_hwaccel_info,
            commands::dependencies::check_dependencies,
            commands::dependencies::install_dependency,
            commands::search::search_videos,
            commands::platform_auth::platform_auth_check,
            commands::platform_auth::platform_auth_login,
            commands::platform_auth::platform_auth_logout,
            commands::platform_auth::platform_auth_list,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

use std::collections::HashMap;
use std::sync::Arc;

use platforms::hotmart::api::Course;
use platforms::hotmart::auth::HotmartSession;
use tokio_util::sync::CancellationToken;

pub mod commands;
pub mod core;
pub mod models;
pub mod platforms;
pub mod storage;

pub struct CoursesCache {
    pub courses: Vec<Course>,
    pub fetched_at: std::time::Instant,
}

pub struct AppState {
    pub hotmart_session: Arc<tokio::sync::Mutex<Option<HotmartSession>>>,
    pub active_downloads: Arc<tokio::sync::Mutex<HashMap<u64, CancellationToken>>>,
    pub registry: core::registry::PlatformRegistry,
    pub courses_cache: Arc<tokio::sync::Mutex<Option<CoursesCache>>>,
    pub session_validated_at: Arc<tokio::sync::Mutex<Option<std::time::Instant>>>,
}

#[tauri::command]
fn greet(name: &str) -> String {
    format!("Hello, {}! You've been greeted from Rust!", name)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tracing_subscriber::fmt::init();

    let session = Arc::new(tokio::sync::Mutex::new(None));

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
        platforms::vimeo::VimeoDownloader::new(),
    ));

    let state = AppState {
        hotmart_session: session,
        active_downloads: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
        registry,
        courses_cache: Arc::new(tokio::sync::Mutex::new(None)),
        session_validated_at: Arc::new(tokio::sync::Mutex::new(None)),
    };

    tauri::Builder::default()
        .manage(state)
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_store::Builder::default().build())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_fs::init())
        .invoke_handler(tauri::generate_handler![
            greet,
            commands::auth::hotmart_login,
            commands::auth::hotmart_check_session,
            commands::auth::hotmart_logout,
            commands::auth::hotmart_debug_auth,
            commands::courses::hotmart_list_courses,
            commands::courses::hotmart_refresh_courses,
            commands::courses::hotmart_get_modules,
            commands::downloads::start_course_download,
            commands::downloads::cancel_course_download,
            commands::downloads::get_active_downloads,
            commands::downloads::detect_platform,
            commands::downloads::download_from_url,
            commands::downloads::reveal_file,
            commands::settings::get_settings,
            commands::settings::update_settings,
            commands::settings::reset_settings,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

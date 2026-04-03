use std::collections::HashMap;
use std::sync::Arc;
use tauri::Manager;





use tokio_util::sync::CancellationToken;

pub struct P2pSendHandle {
    pub cancel_token: CancellationToken,
    pub paused: Arc<std::sync::atomic::AtomicBool>,
}
pub type ActiveP2pSends = Arc<tokio::sync::Mutex<HashMap<String, P2pSendHandle>>>;

pub mod commands;
pub mod core;
pub mod external_url;
pub mod hotkey;
pub mod models;
pub mod native_host;
pub mod platforms;
pub mod plugin_host;
pub mod plugin_loader;
pub mod storage;
pub mod tray;













































































































































pub struct AppState {
    pub active_downloads: Arc<tokio::sync::Mutex<HashMap<u64, CancellationToken>>>,
    pub active_generic_downloads: Arc<tokio::sync::Mutex<HashMap<u64, (String, CancellationToken)>>>,
    pub registry: core::registry::PlatformRegistry,
    pub download_queue: Arc<tokio::sync::Mutex<core::queue::DownloadQueue>>,
    pub torrent_session: Arc<tokio::sync::Mutex<Option<Arc<librqbit::Session>>>>,
    pub active_p2p_sends: ActiveP2pSends,
    pub frontend_ready: Arc<tokio::sync::Mutex<bool>>,
    pub pending_external_events: Arc<tokio::sync::Mutex<Vec<external_url::ExternalUrlEvent>>>,
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tracing_subscriber::fmt::init();

    let mut registry = core::registry::PlatformRegistry::new();
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
        platforms::vimeo::VimeoDownloader::new(),
    ));
    registry.register(Arc::new(
        platforms::bilibili::BilibiliDownloader::new(),
    ));
    let torrent_session: Arc<tokio::sync::Mutex<Option<Arc<librqbit::Session>>>> =
        Arc::new(tokio::sync::Mutex::new(None));
    registry.register(Arc::new(
        platforms::magnet::MagnetDownloader::new(torrent_session.clone()),
    ));
    registry.register(Arc::new(
        platforms::p2p::P2pDownloader::new(),
    ));
    registry.register(Arc::new(
        platforms::generic_ytdlp::GenericYtdlpDownloader::new(),
    ));

    let state = AppState {
        active_downloads: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
        active_generic_downloads: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
        registry,
        download_queue: Arc::new(tokio::sync::Mutex::new(core::queue::DownloadQueue::new(2))),
        torrent_session,
        active_p2p_sends: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
        frontend_ready: Arc::new(tokio::sync::Mutex::new(false)),
        pending_external_events: Arc::new(tokio::sync::Mutex::new(Vec::new())),
    };

    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, argv, _cwd| {
            if let Some(url) = external_url::find_external_url_arg(argv.iter().skip(1).map(|arg| arg.as_str())) {
                let app_handle = app.clone();
                tauri::async_runtime::spawn(async move {
                    if let Err(error) = external_url::handle_external_url(&app_handle, url, "command-line").await {
                        tracing::warn!("Failed to handle external URL from second instance: {}", error);
                    }
                });
            } else {
                tray::show_window(app);
            }
        }))
        .manage(state)
        .manage(Arc::new(tokio::sync::Mutex::new(
            plugin_loader::PluginManager::new(
                core::paths::app_data_dir()
                    .unwrap_or_else(|| std::path::PathBuf::from("."))
                    .join("plugins"),
            ),
        )))
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_store::Builder::default().build())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(|app, _shortcut, event| {
                    if event.state == tauri_plugin_global_shortcut::ShortcutState::Pressed {
                        hotkey::on_hotkey_pressed(app);
                    }
                })
                .build(),
        )
        .plugin(tauri_plugin_clipboard_manager::init())
        .setup(|app| {
            let settings = storage::config::load_settings(app.handle());
            core::http_client::init_proxy(settings.proxy.clone());
            core::ytdlp::set_ext_cookie_path_fn(|| native_host::extension_cookie_file_path());
            core::ytdlp::set_global_cookie_file_fn(|| {
                let s = storage::config::load_settings_standalone();
                let cf = s.download.cookie_file.clone();
                if !cf.is_empty() && std::path::Path::new(&cf).exists() { Some(cf) } else { None }
            });
            core::ytdlp::set_cookies_from_browser_fn(|| {
                storage::config::load_settings_standalone().advanced.cookies_from_browser
            });
            tray::setup(app.handle())?;
            hotkey::register_from_settings(app.handle());
            if let Err(error) = native_host::ensure_registered() {
                tracing::warn!("Failed to register Chrome native host: {}", error);
            }
            {
                let plugins_dir = core::paths::app_data_dir()
                    .unwrap_or_else(|| std::path::PathBuf::from("."))
                    .join("plugins");
                let host = std::sync::Arc::new(plugin_host::PluginHostImpl::new(
                    app.handle().clone(),
                    plugins_dir,
                ));
                let plugin_mgr = app.handle().state::<std::sync::Arc<tokio::sync::Mutex<plugin_loader::PluginManager>>>();
                let mut mgr = plugin_mgr.blocking_lock();
                mgr.load_all(host);
            }

            tauri::async_runtime::spawn(async {
                if let Some(ytdlp) = core::ytdlp::find_ytdlp_cached().await {
                    match core::ytdlp::check_ytdlp_update(&ytdlp).await {
                        Ok(true) => tracing::info!("yt-dlp updated successfully"),
                        Ok(false) => tracing::debug!("yt-dlp already up to date"),
                        Err(e) => tracing::warn!("yt-dlp update check failed: {}", e),
                    }
                }
                core::dependencies::ensure_js_runtime().await;
            });

            if let Some(url) = external_url::find_external_url_arg(std::env::args().skip(1)) {
                let app_handle = app.handle().clone();
                tauri::async_runtime::spawn(async move {
                    if let Err(error) = external_url::handle_external_url(&app_handle, url, "command-line").await {
                        tracing::warn!("Failed to handle startup external URL: {}", error);
                    }
                });
            }
            Ok(())
        })
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                if window.label() == "main" {
                    api.prevent_close();
                    let _ = window.hide();
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            commands::diagnostics::get_rate_limit_stats,
            commands::downloads::detect_platform,
            commands::downloads::check_cookie_error,
            commands::downloads::get_media_formats,
            commands::downloads::prefetch_media_info,
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
            commands::integration::register_external_frontend,
            commands::settings::get_settings,
            commands::settings::update_settings,
            commands::settings::reset_settings,
            commands::settings::mark_onboarding_complete,
            commands::autostart::set_autostart,
            commands::autostart::get_autostart_status,
            commands::dependencies::check_dependencies,
            commands::dependencies::check_ytdlp_available,
            commands::dependencies::install_dependency,
            commands::search::search_videos,
            commands::plugins::list_plugins,
            commands::plugins::get_plugin_frontend_path,
            commands::plugins::set_plugin_enabled,
            commands::plugins::uninstall_plugin,
            commands::plugins::get_loaded_plugin_manifests,
            commands::plugins::plugin_command,
            commands::plugins::fetch_marketplace_registry,
            commands::plugins::install_plugin_from_registry,
            commands::plugins::get_plugin_i18n,
            commands::plugins::check_plugin_updates,
            commands::plugins::update_plugin,
            commands::p2p::p2p_send_file,
            commands::p2p::p2p_cancel_send,
            commands::p2p::p2p_pause_send,
            commands::p2p::p2p_resume_send,
            commands::p2p::p2p_get_active_sends,
            commands::p2p::p2p_validate_code,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

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
    pub auth_registry: core::auth::AuthRegistry,
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

    let auth_registry = core::auth::AuthRegistry::new();

    let state = AppState {
        active_downloads: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
        active_generic_downloads: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
        registry,
        download_queue: Arc::new(tokio::sync::Mutex::new(core::queue::DownloadQueue::new(2))),
        auth_registry,
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
        .plugin(omniget_plugin_courses::init())
        .plugin(omniget_plugin_telegram::init())
        .setup(|app| {
            let settings = storage::config::load_settings(app.handle());
            core::http_client::init_proxy(settings.proxy.clone());
            core::ytdlp::set_ext_cookie_path_fn(|| native_host::extension_cookie_file_path());
            core::ytdlp::set_global_cookie_file_fn(|| {
                let s = storage::config::load_settings_standalone();
                let cf = s.download.cookie_file.clone();
                if !cf.is_empty() && std::path::Path::new(&cf).exists() { Some(cf) } else { None }
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
            commands::platform_auth::platform_auth_check,
            commands::platform_auth::platform_auth_login,
            commands::platform_auth::platform_auth_logout,
            commands::platform_auth::platform_auth_list,
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
            omniget_plugin_courses::commands::auth::hotmart_login,
            omniget_plugin_courses::commands::auth::hotmart_check_session,
            omniget_plugin_courses::commands::auth::hotmart_logout,
            omniget_plugin_courses::commands::courses::hotmart_list_courses,
            omniget_plugin_courses::commands::courses::hotmart_refresh_courses,
            omniget_plugin_courses::commands::courses::hotmart_get_modules,
            omniget_plugin_courses::commands::downloads::start_course_download,
            omniget_plugin_courses::commands::downloads::cancel_course_download,
            omniget_plugin_courses::commands::downloads::get_active_downloads,
            omniget_plugin_courses::commands::kiwify::kiwify_login,
            omniget_plugin_courses::commands::kiwify::kiwify_login_token,
            omniget_plugin_courses::commands::kiwify::kiwify_check_session,
            omniget_plugin_courses::commands::kiwify::kiwify_logout,
            omniget_plugin_courses::commands::kiwify::kiwify_list_courses,
            omniget_plugin_courses::commands::kiwify::kiwify_refresh_courses,
            omniget_plugin_courses::commands::kiwify::start_kiwify_course_download,
            omniget_plugin_courses::commands::skool::skool_login,
            omniget_plugin_courses::commands::skool::skool_login_token,
            omniget_plugin_courses::commands::skool::skool_check_session,
            omniget_plugin_courses::commands::skool::skool_logout,
            omniget_plugin_courses::commands::skool::skool_list_groups,
            omniget_plugin_courses::commands::skool::skool_refresh_groups,
            omniget_plugin_courses::commands::skool::start_skool_course_download,
            omniget_plugin_courses::commands::udemy_auth::udemy_login,
            omniget_plugin_courses::commands::udemy_auth::udemy_login_cookies,
            omniget_plugin_courses::commands::udemy_auth::udemy_check_session,
            omniget_plugin_courses::commands::udemy_auth::udemy_get_portal,
            omniget_plugin_courses::commands::udemy_auth::udemy_logout,
            omniget_plugin_courses::commands::udemy_courses::udemy_list_courses,
            omniget_plugin_courses::commands::udemy_courses::udemy_refresh_courses,
            omniget_plugin_courses::commands::udemy_downloads::start_udemy_course_download,
            omniget_plugin_courses::commands::udemy_downloads::cancel_udemy_course_download,
            omniget_plugin_courses::commands::afya::afya_login,
            omniget_plugin_courses::commands::afya::afya_login_token,
            omniget_plugin_courses::commands::afya::afya_check_session,
            omniget_plugin_courses::commands::afya::afya_logout,
            omniget_plugin_courses::commands::afya::afya_list_courses,
            omniget_plugin_courses::commands::afya::afya_refresh_courses,
            omniget_plugin_courses::commands::afya::start_afya_course_download,
            omniget_plugin_courses::commands::alpaclass::alpaclass_login,
            omniget_plugin_courses::commands::alpaclass::alpaclass_check_session,
            omniget_plugin_courses::commands::alpaclass::alpaclass_logout,
            omniget_plugin_courses::commands::alpaclass::alpaclass_list_courses,
            omniget_plugin_courses::commands::alpaclass::alpaclass_refresh_courses,
            omniget_plugin_courses::commands::alpaclass::start_alpaclass_course_download,
            omniget_plugin_courses::commands::areademembros::areademembros_login_token,
            omniget_plugin_courses::commands::areademembros::areademembros_check_session,
            omniget_plugin_courses::commands::areademembros::areademembros_logout,
            omniget_plugin_courses::commands::areademembros::areademembros_list_courses,
            omniget_plugin_courses::commands::areademembros::areademembros_refresh_courses,
            omniget_plugin_courses::commands::areademembros::start_areademembros_course_download,
            omniget_plugin_courses::commands::astronmembers::astron_login,
            omniget_plugin_courses::commands::astronmembers::astron_login_token,
            omniget_plugin_courses::commands::astronmembers::astron_check_session,
            omniget_plugin_courses::commands::astronmembers::astron_logout,
            omniget_plugin_courses::commands::astronmembers::astron_list_courses,
            omniget_plugin_courses::commands::astronmembers::astron_refresh_courses,
            omniget_plugin_courses::commands::astronmembers::start_astron_course_download,
            omniget_plugin_courses::commands::cademi_cmd::cademi_login,
            omniget_plugin_courses::commands::cademi_cmd::cademi_login_cookie,
            omniget_plugin_courses::commands::cademi_cmd::cademi_check_session,
            omniget_plugin_courses::commands::cademi_cmd::cademi_logout,
            omniget_plugin_courses::commands::cademi_cmd::cademi_list_courses,
            omniget_plugin_courses::commands::cademi_cmd::cademi_refresh_courses,
            omniget_plugin_courses::commands::cademi_cmd::start_cademi_course_download,
            omniget_plugin_courses::commands::cakto::cakto_login,
            omniget_plugin_courses::commands::cakto::cakto_login_token,
            omniget_plugin_courses::commands::cakto::cakto_check_session,
            omniget_plugin_courses::commands::cakto::cakto_logout,
            omniget_plugin_courses::commands::cakto::cakto_list_courses,
            omniget_plugin_courses::commands::cakto::cakto_refresh_courses,
            omniget_plugin_courses::commands::cakto::start_cakto_course_download,
            omniget_plugin_courses::commands::caktomembers::caktomembers_login_token,
            omniget_plugin_courses::commands::caktomembers::caktomembers_check_session,
            omniget_plugin_courses::commands::caktomembers::caktomembers_logout,
            omniget_plugin_courses::commands::caktomembers::caktomembers_list_courses,
            omniget_plugin_courses::commands::caktomembers::caktomembers_refresh_courses,
            omniget_plugin_courses::commands::caktomembers::start_caktomembers_course_download,
            omniget_plugin_courses::commands::curseduca::curseduca_login,
            omniget_plugin_courses::commands::curseduca::curseduca_login_token,
            omniget_plugin_courses::commands::curseduca::curseduca_check_session,
            omniget_plugin_courses::commands::curseduca::curseduca_logout,
            omniget_plugin_courses::commands::curseduca::curseduca_list_courses,
            omniget_plugin_courses::commands::curseduca::curseduca_refresh_courses,
            omniget_plugin_courses::commands::curseduca::start_curseduca_course_download,
            omniget_plugin_courses::commands::dsa::dsa_login_token,
            omniget_plugin_courses::commands::dsa::dsa_check_session,
            omniget_plugin_courses::commands::dsa::dsa_logout,
            omniget_plugin_courses::commands::dsa::dsa_list_courses,
            omniget_plugin_courses::commands::dsa::dsa_refresh_courses,
            omniget_plugin_courses::commands::dsa::start_dsa_course_download,
            omniget_plugin_courses::commands::entregadigital::entregadigital_login_token,
            omniget_plugin_courses::commands::entregadigital::entregadigital_check_session,
            omniget_plugin_courses::commands::entregadigital::entregadigital_logout,
            omniget_plugin_courses::commands::entregadigital::entregadigital_list_courses,
            omniget_plugin_courses::commands::entregadigital::entregadigital_refresh_courses,
            omniget_plugin_courses::commands::entregadigital::start_entregadigital_course_download,
            omniget_plugin_courses::commands::estrategia_concursos::estrategia_concursos_login_token,
            omniget_plugin_courses::commands::estrategia_concursos::estrategia_concursos_check_session,
            omniget_plugin_courses::commands::estrategia_concursos::estrategia_concursos_logout,
            omniget_plugin_courses::commands::estrategia_concursos::estrategia_concursos_list_courses,
            omniget_plugin_courses::commands::estrategia_concursos::estrategia_concursos_refresh_courses,
            omniget_plugin_courses::commands::estrategia_concursos::start_estrategia_concursos_course_download,
            omniget_plugin_courses::commands::estrategia_ldi::estrategia_ldi_login_token,
            omniget_plugin_courses::commands::estrategia_ldi::estrategia_ldi_check_session,
            omniget_plugin_courses::commands::estrategia_ldi::estrategia_ldi_logout,
            omniget_plugin_courses::commands::estrategia_ldi::estrategia_ldi_list_courses,
            omniget_plugin_courses::commands::estrategia_ldi::estrategia_ldi_refresh_courses,
            omniget_plugin_courses::commands::estrategia_ldi::start_estrategia_ldi_course_download,
            omniget_plugin_courses::commands::estrategia_militares::estrategia_militares_login_token,
            omniget_plugin_courses::commands::estrategia_militares::estrategia_militares_check_session,
            omniget_plugin_courses::commands::estrategia_militares::estrategia_militares_logout,
            omniget_plugin_courses::commands::estrategia_militares::estrategia_militares_list_courses,
            omniget_plugin_courses::commands::estrategia_militares::estrategia_militares_search_courses,
            omniget_plugin_courses::commands::estrategia_militares::estrategia_militares_refresh_courses,
            omniget_plugin_courses::commands::estrategia_militares::start_estrategia_militares_course_download,
            omniget_plugin_courses::commands::fluencyacademy::fluency_login,
            omniget_plugin_courses::commands::fluencyacademy::fluency_login_token,
            omniget_plugin_courses::commands::fluencyacademy::fluency_check_session,
            omniget_plugin_courses::commands::fluencyacademy::fluency_logout,
            omniget_plugin_courses::commands::fluencyacademy::fluency_list_courses,
            omniget_plugin_courses::commands::fluencyacademy::fluency_refresh_courses,
            omniget_plugin_courses::commands::fluencyacademy::start_fluency_course_download,
            omniget_plugin_courses::commands::grancursos::grancursos_login_cookies,
            omniget_plugin_courses::commands::grancursos::grancursos_check_session,
            omniget_plugin_courses::commands::grancursos::grancursos_logout,
            omniget_plugin_courses::commands::grancursos::grancursos_list_courses,
            omniget_plugin_courses::commands::grancursos::grancursos_refresh_courses,
            omniget_plugin_courses::commands::grancursos::start_grancursos_course_download,
            omniget_plugin_courses::commands::greatcourses::wondrium_login,
            omniget_plugin_courses::commands::greatcourses::wondrium_login_token,
            omniget_plugin_courses::commands::greatcourses::wondrium_check_session,
            omniget_plugin_courses::commands::greatcourses::wondrium_logout,
            omniget_plugin_courses::commands::greatcourses::wondrium_list_courses,
            omniget_plugin_courses::commands::greatcourses::wondrium_refresh_courses,
            omniget_plugin_courses::commands::greatcourses::start_wondrium_course_download,
            omniget_plugin_courses::commands::greenn::greenn_login_token,
            omniget_plugin_courses::commands::greenn::greenn_check_session,
            omniget_plugin_courses::commands::greenn::greenn_logout,
            omniget_plugin_courses::commands::greenn::greenn_list_courses,
            omniget_plugin_courses::commands::greenn::greenn_refresh_courses,
            omniget_plugin_courses::commands::greenn::start_greenn_course_download,
            omniget_plugin_courses::commands::gumroad::gumroad_login,
            omniget_plugin_courses::commands::gumroad::gumroad_login_token,
            omniget_plugin_courses::commands::gumroad::gumroad_check_session,
            omniget_plugin_courses::commands::gumroad::gumroad_logout,
            omniget_plugin_courses::commands::gumroad::gumroad_list_products,
            omniget_plugin_courses::commands::gumroad::gumroad_refresh_products,
            omniget_plugin_courses::commands::gumroad::start_gumroad_download,
            omniget_plugin_courses::commands::kajabi::kajabi_request_login_link,
            omniget_plugin_courses::commands::kajabi::kajabi_verify_login,
            omniget_plugin_courses::commands::kajabi::kajabi_login_token,
            omniget_plugin_courses::commands::kajabi::kajabi_check_session,
            omniget_plugin_courses::commands::kajabi::kajabi_logout,
            omniget_plugin_courses::commands::kajabi::kajabi_list_sites,
            omniget_plugin_courses::commands::kajabi::kajabi_set_site,
            omniget_plugin_courses::commands::kajabi::kajabi_list_courses,
            omniget_plugin_courses::commands::kajabi::kajabi_refresh_courses,
            omniget_plugin_courses::commands::kajabi::start_kajabi_course_download,
            omniget_plugin_courses::commands::kirvano::kirvano_login,
            omniget_plugin_courses::commands::kirvano::kirvano_login_token,
            omniget_plugin_courses::commands::kirvano::kirvano_check_session,
            omniget_plugin_courses::commands::kirvano::kirvano_logout,
            omniget_plugin_courses::commands::kirvano::kirvano_list_courses,
            omniget_plugin_courses::commands::kirvano::kirvano_refresh_courses,
            omniget_plugin_courses::commands::kirvano::start_kirvano_course_download,
            omniget_plugin_courses::commands::masterclass::masterclass_login_cookies,
            omniget_plugin_courses::commands::masterclass::masterclass_check_session,
            omniget_plugin_courses::commands::masterclass::masterclass_logout,
            omniget_plugin_courses::commands::masterclass::masterclass_list_courses,
            omniget_plugin_courses::commands::masterclass::masterclass_refresh_courses,
            omniget_plugin_courses::commands::masterclass::start_masterclass_course_download,
            omniget_plugin_courses::commands::medcel::medcel_login,
            omniget_plugin_courses::commands::medcel::medcel_login_token,
            omniget_plugin_courses::commands::medcel::medcel_check_session,
            omniget_plugin_courses::commands::medcel::medcel_logout,
            omniget_plugin_courses::commands::medcel::medcel_list_courses,
            omniget_plugin_courses::commands::medcel::medcel_refresh_courses,
            omniget_plugin_courses::commands::medcel::start_medcel_course_download,
            omniget_plugin_courses::commands::medcof::medcof_login_token,
            omniget_plugin_courses::commands::medcof::medcof_check_session,
            omniget_plugin_courses::commands::medcof::medcof_logout,
            omniget_plugin_courses::commands::medcof::medcof_list_courses,
            omniget_plugin_courses::commands::medcof::medcof_refresh_courses,
            omniget_plugin_courses::commands::medcof::start_medcof_course_download,
            omniget_plugin_courses::commands::medway::medway_login_token,
            omniget_plugin_courses::commands::medway::medway_check_session,
            omniget_plugin_courses::commands::medway::medway_logout,
            omniget_plugin_courses::commands::medway::medway_list_courses,
            omniget_plugin_courses::commands::medway::medway_refresh_courses,
            omniget_plugin_courses::commands::medway::start_medway_course_download,
            omniget_plugin_courses::commands::memberkit_cmd::memberkit_login,
            omniget_plugin_courses::commands::memberkit_cmd::memberkit_login_cookie,
            omniget_plugin_courses::commands::memberkit_cmd::memberkit_check_session,
            omniget_plugin_courses::commands::memberkit_cmd::memberkit_logout,
            omniget_plugin_courses::commands::memberkit_cmd::memberkit_list_courses,
            omniget_plugin_courses::commands::memberkit_cmd::memberkit_refresh_courses,
            omniget_plugin_courses::commands::memberkit_cmd::start_memberkit_course_download,
            omniget_plugin_courses::commands::nutror::nutror_login_token,
            omniget_plugin_courses::commands::nutror::nutror_check_session,
            omniget_plugin_courses::commands::nutror::nutror_logout,
            omniget_plugin_courses::commands::nutror::nutror_list_courses,
            omniget_plugin_courses::commands::nutror::nutror_refresh_courses,
            omniget_plugin_courses::commands::nutror::start_nutror_course_download,
            omniget_plugin_courses::commands::pluralsight::pluralsight_login_cookies,
            omniget_plugin_courses::commands::pluralsight::pluralsight_check_session,
            omniget_plugin_courses::commands::pluralsight::pluralsight_logout,
            omniget_plugin_courses::commands::pluralsight::pluralsight_list_courses,
            omniget_plugin_courses::commands::pluralsight::pluralsight_refresh_courses,
            omniget_plugin_courses::commands::pluralsight::start_pluralsight_course_download,
            omniget_plugin_courses::commands::rocketseat::rocketseat_login_token,
            omniget_plugin_courses::commands::rocketseat::rocketseat_check_session,
            omniget_plugin_courses::commands::rocketseat::rocketseat_logout,
            omniget_plugin_courses::commands::rocketseat::rocketseat_list_courses,
            omniget_plugin_courses::commands::rocketseat::rocketseat_search_courses,
            omniget_plugin_courses::commands::rocketseat::rocketseat_refresh_courses,
            omniget_plugin_courses::commands::rocketseat::start_rocketseat_course_download,
            omniget_plugin_courses::commands::teachable::teachable_request_otp,
            omniget_plugin_courses::commands::teachable::teachable_verify_otp,
            omniget_plugin_courses::commands::teachable::teachable_login_token,
            omniget_plugin_courses::commands::teachable::teachable_check_session,
            omniget_plugin_courses::commands::teachable::teachable_logout,
            omniget_plugin_courses::commands::teachable::teachable_set_school,
            omniget_plugin_courses::commands::teachable::teachable_list_schools,
            omniget_plugin_courses::commands::teachable::teachable_list_courses,
            omniget_plugin_courses::commands::teachable::teachable_refresh_courses,
            omniget_plugin_courses::commands::teachable::start_teachable_course_download,
            omniget_plugin_courses::commands::themembers::themembers_login,
            omniget_plugin_courses::commands::themembers::themembers_login_token,
            omniget_plugin_courses::commands::themembers::themembers_check_session,
            omniget_plugin_courses::commands::themembers::themembers_logout,
            omniget_plugin_courses::commands::themembers::themembers_list_courses,
            omniget_plugin_courses::commands::themembers::themembers_refresh_courses,
            omniget_plugin_courses::commands::themembers::start_themembers_course_download,
            omniget_plugin_courses::commands::thinkific::thinkific_login,
            omniget_plugin_courses::commands::thinkific::thinkific_check_session,
            omniget_plugin_courses::commands::thinkific::thinkific_logout,
            omniget_plugin_courses::commands::thinkific::thinkific_list_courses,
            omniget_plugin_courses::commands::thinkific::thinkific_refresh_courses,
            omniget_plugin_courses::commands::thinkific::start_thinkific_course_download,
            omniget_plugin_courses::commands::voomp::voomp_login_token,
            omniget_plugin_courses::commands::voomp::voomp_check_session,
            omniget_plugin_courses::commands::voomp::voomp_logout,
            omniget_plugin_courses::commands::voomp::voomp_list_courses,
            omniget_plugin_courses::commands::voomp::voomp_refresh_courses,
            omniget_plugin_courses::commands::voomp::start_voomp_course_download,
            omniget_plugin_telegram::commands::telegram::telegram_check_session,
            omniget_plugin_telegram::commands::telegram::telegram_qr_start,
            omniget_plugin_telegram::commands::telegram::telegram_qr_poll,
            omniget_plugin_telegram::commands::telegram::telegram_send_code,
            omniget_plugin_telegram::commands::telegram::telegram_verify_code,
            omniget_plugin_telegram::commands::telegram::telegram_verify_2fa,
            omniget_plugin_telegram::commands::telegram::telegram_logout,
            omniget_plugin_telegram::commands::telegram::telegram_list_chats,
            omniget_plugin_telegram::commands::telegram::telegram_list_media,
            omniget_plugin_telegram::commands::telegram::telegram_download_media,
            omniget_plugin_telegram::commands::telegram::telegram_download_batch,
            omniget_plugin_telegram::commands::telegram::telegram_cancel_batch,
            omniget_plugin_telegram::commands::telegram::telegram_get_thumbnail,
            omniget_plugin_telegram::commands::telegram::telegram_search_media,
            omniget_plugin_telegram::commands::telegram::telegram_get_chat_photo,
            omniget_plugin_telegram::commands::telegram::telegram_clear_thumbnail_cache,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

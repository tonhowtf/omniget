#![recursion_limit = "256"]

use std::collections::HashMap;
use std::sync::Arc;
use tauri::Manager;
use tauri_plugin_deep_link::DeepLinkExt;

use tokio_util::sync::CancellationToken;

pub struct P2pSendHandle {
    pub cancel_token: CancellationToken,
    pub paused: Arc<std::sync::atomic::AtomicBool>,
}
pub type ActiveP2pSends = Arc<tokio::sync::Mutex<HashMap<String, P2pSendHandle>>>;

pub mod agentkit_boot;
pub mod approvals;
pub mod bridge_tokens;
pub mod canvas;
pub mod commands;
pub mod company;
pub mod connect;
pub mod cookies;
pub mod core;
pub mod extension_storage;
pub mod external_url;
mod handlers;
pub mod hotkey;
pub mod ipc_guard;
pub mod job_commands;
pub mod job_workflow;
pub mod jobs;
pub mod limits_strip;
pub mod llm_attention;
pub mod llm_audit;
pub mod llm_budget;
pub mod llm_manager;
pub mod local_bridge;
pub mod local_bridge_agentkit;
pub mod local_bridge_canvas;
pub mod local_bridge_company;
pub mod local_bridge_debug;
pub mod local_bridge_jobs;
pub mod local_bridge_llm;
pub mod local_bridge_observe;
pub mod local_bridge_remote;
pub mod local_bridge_telegram;
pub mod local_bridge_wmb;
pub mod mcp;
pub mod missions;
pub mod models;
mod native_agent;
mod native_agent_export;
mod native_agent_media;
mod native_agent_record;
mod native_capture;
pub mod platforms;
pub mod portal;
pub mod preview;
pub mod profile;
pub mod pty;
pub mod secrets;
pub mod storage;
pub mod threads_host;
pub mod tray;
pub mod usage_tray;

struct DesktopCookieProvider;

impl omniget_core::platforms::cookie_provider::CookieProvider for DesktopCookieProvider {
    fn cookie_path_for(&self, domain: &str) -> Option<std::path::PathBuf> {
        let root = crate::cookies::root_domain_of(domain);
        if root.is_empty() {
            return None;
        }

        let slug = omniget_core::core::log_hook::current_cookie_slug();
        let path = crate::cookies::account_path_for_consumer(&root, slug.as_deref())
            .or_else(|| crate::cookies::account_path_for_consumer(&root, None))?;
        crate::cookies::touch_last_used(&root, slug.as_deref().unwrap_or("_default"));
        Some(path)
    }

    fn cookie_path_for_account(&self, domain: &str, slug: &str) -> Option<std::path::PathBuf> {
        let root = crate::cookies::root_domain_of(domain);
        if root.is_empty() {
            return None;
        }
        let path = crate::cookies::account_path_for_consumer(&root, Some(slug))?;
        crate::cookies::touch_last_used(&root, slug);
        Some(path)
    }

    fn manual_cookie_header(&self, domain: &str) -> Option<String> {
        let root = crate::cookies::root_domain_of(domain);
        if root != "x.com" && root != "twitter.com" {
            return None;
        }

        let raw = storage::config::load_settings_standalone()
            .advanced
            .twitter_manual_cookie;
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            return None;
        }

        let parsed = core::cookie_parser::parse_cookie_input(trimmed, "");
        if !parsed.cookie_string.trim().is_empty() {
            Some(parsed.cookie_string)
        } else {
            Some(trimmed.to_string())
        }
    }
}

struct DesktopBilibiliRuntimeProvider;

impl omniget_core::platforms::bilibili::BilibiliRuntimeProvider for DesktopBilibiliRuntimeProvider {
    fn active_account_slug(&self) -> Option<String> {
        let registry = crate::cookies::load_registry();
        let bucket = registry.buckets.get("bilibili.com")?;

        if let Some(selected) = omniget_core::core::log_hook::current_cookie_slug() {
            if bucket
                .accounts
                .iter()
                .any(|a| a.slug == selected && a.slug != "_anonymous" && a.cookie_count > 0)
            {
                return Some(selected);
            }
        }

        let native = bucket
            .accounts
            .iter()
            .filter(|a| a.slug != "_anonymous" && a.slug != "_default" && a.cookie_count > 0)
            .max_by_key(|a| a.last_used_at_ms.unwrap_or(a.captured_at_ms))
            .map(|a| a.slug.clone());
        if native.is_some() {
            return native;
        }

        bucket
            .accounts
            .iter()
            .filter(|a| a.slug == "_default" && a.cookie_count > 0)
            .find(|a| desktop_bilibili_slug_has_session_cookie(&a.slug))
            .map(|a| a.slug.clone())
    }

    fn settings(&self) -> omniget_core::models::settings::AppSettings {
        storage::config::load_settings_standalone()
    }

    fn persist_account(
        &self,
        cookies: &[omniget_core::platforms::bilibili::BilibiliAuthCookie],
        uname: &str,
        source_label: &str,
    ) -> Result<String, String> {
        let slug = omniget_core::platforms::bilibili::auth::slug_from_uname(uname);
        let entries: Vec<crate::extension_storage::ExtensionCookie> = cookies
            .iter()
            .map(|cookie| crate::extension_storage::ExtensionCookie {
                domain: cookie.domain.clone(),
                http_only: cookie.http_only,
                path: cookie.path.clone(),
                secure: cookie.secure,
                expires: cookie.expires,
                name: cookie.name.clone(),
                value: cookie.value.clone(),
                host_only: cookie.host_only,
                same_site: cookie.same_site.clone(),
            })
            .collect();

        crate::cookies::storage::write_account_file("bilibili.com", &slug, &entries)
            .map_err(|e| e.to_string())?;

        let mut registry = crate::cookies::storage::load_registry();
        let now_ms = crate::cookies::storage::current_unix_ms();
        let bucket = registry
            .buckets
            .entry("bilibili.com".to_string())
            .or_insert_with(|| crate::cookies::storage::BucketEntry {
                platform_kind: "bilibili".to_string(),
                accounts: Vec::new(),
            });
        bucket.platform_kind = "bilibili".to_string();

        let alias = format!("{} · {}", uname, source_label);
        if let Some(existing) = bucket.accounts.iter_mut().find(|a| a.slug == slug) {
            existing.captured_at_ms = now_ms;
            existing.cookie_count = entries.len();
            existing.source_label = Some(source_label.to_string());
            existing.alias = alias;
        } else {
            bucket.accounts.push(crate::cookies::storage::AccountEntry {
                slug: slug.clone(),
                alias,
                source_url: Some("https://www.bilibili.com".to_string()),
                source_label: Some(source_label.to_string()),
                captured_at_ms: now_ms,
                cookie_count: entries.len(),
                last_used_at_ms: Some(now_ms),
            });
        }

        crate::cookies::storage::save_registry(&registry).map_err(|e| e.to_string())?;
        Ok(slug)
    }
}

fn desktop_bilibili_slug_has_session_cookie(slug: &str) -> bool {
    let path = match crate::cookies::account_path_for_consumer("bilibili.com", Some(slug)) {
        Some(p) => p,
        None => return false,
    };
    let content = match std::fs::read_to_string(&path) {
        Ok(c) => c,
        Err(_) => return false,
    };
    content.lines().any(|line| {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            return false;
        }
        line.split('\t').nth(5) == Some("SESSDATA")
    })
}

pub struct AppState {
    pub active_downloads: Arc<tokio::sync::Mutex<HashMap<u64, CancellationToken>>>,
    pub active_generic_downloads:
        Arc<tokio::sync::Mutex<HashMap<u64, (String, CancellationToken)>>>,
    pub registry: core::registry::PlatformRegistry,
    pub download_queue: Arc<tokio::sync::Mutex<core::queue::DownloadQueue>>,
    pub torrent_session: Arc<tokio::sync::Mutex<Option<Arc<librqbit::Session>>>>,
    pub active_p2p_sends: ActiveP2pSends,
    pub frontend_ready: Arc<tokio::sync::Mutex<bool>>,
    pub pending_external_events: Arc<tokio::sync::Mutex<Vec<external_url::ExternalUrlEvent>>>,
    pub profile: Arc<profile::ProfileManager>,
    pub llm: Arc<llm_manager::LlmManager>,
}

fn launched_hidden(app: &tauri::AppHandle) -> bool {
    let s = storage::config::load_settings(app);
    let args: Vec<String> = std::env::args().collect();
    omniget_core::core::app_launch::starts_hidden(&args, s.start_minimized, s.run_in_background)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    canvas::diagnostics::init_tracing();

    tracing::info!(
        "{}",
        core::portable::startup_banner(
            env!("CARGO_PKG_VERSION"),
            std::process::id(),
            std::env::var("OMNIGET_PORTABLE").ok().as_deref() == Some("1"),
            core::paths::app_data_dir().as_deref(),
        )
    );
    #[cfg(debug_assertions)]
    if std::env::var("OMNIGET_DEV_PROFILE").ok().as_deref() == Some("1") {
        tracing::info!(
            "[dev] debug build: using the dev profile at {} instead of the installed app's; set {}=1 to use the real one",
            core::paths::app_data_dir()
                .map(|d| d.display().to_string())
                .unwrap_or_default(),
            core::paths::DEV_REAL_PROFILE_ENV
        );
    }

    let mut registry = core::registry::PlatformRegistry::new();
    registry.register(Arc::new(platforms::gallerydl::GalleryDlDownloader::new()));
    registry.register(Arc::new(omniget_core::platforms::InstagramDownloader::new()));
    registry.register(Arc::new(omniget_core::platforms::ThreadsDownloader::new()));
    registry.register(Arc::new(omniget_core::platforms::PinterestDownloader::new()));
    registry.register(Arc::new(omniget_core::platforms::TikTokDownloader::new()));
    registry.register(Arc::new(omniget_core::platforms::TwitterDownloader::new()));
    registry.register(Arc::new(
        omniget_core::platforms::TwitchClipsDownloader::new(),
    ));
    registry.register(Arc::new(omniget_core::platforms::BlueskyDownloader::new()));
    registry.register(Arc::new(omniget_core::platforms::RedditDownloader::new()));
    registry.register(Arc::new(omniget_core::platforms::YouTubeDownloader::new()));
    registry.register(Arc::new(omniget_core::platforms::VimeoDownloader::new()));
    registry.register(Arc::new(omniget_core::platforms::BilibiliDownloader::new()));
    registry.register(Arc::new(omniget_core::platforms::DouyinDownloader::new()));
    let torrent_session: Arc<tokio::sync::Mutex<Option<Arc<librqbit::Session>>>> =
        Arc::new(tokio::sync::Mutex::new(None));
    registry.register(Arc::new(platforms::magnet::MagnetDownloader::new(
        torrent_session.clone(),
    )));
    registry.register(Arc::new(omniget_core::platforms::P2pDownloader::new()));
    registry.register(Arc::new(
        omniget_core::platforms::DirectFileDownloader::new(),
    ));
    registry.register(Arc::new(
        omniget_core::platforms::generic_ytdlp::GenericYtdlpDownloader::new(),
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
        profile: Arc::new(profile::ProfileManager::new()),
        llm: Arc::new(llm_manager::LlmManager::new()),
    };

    let second_profile = std::env::var_os("OMNIGET_DATA_DIR").is_some()
        && std::env::var("OMNIGET_PORTABLE").ok().as_deref() != Some("1");
    let builder = tauri::Builder::default();
    let builder = if second_profile {
        builder
    } else {
        builder.plugin(tauri_plugin_single_instance::init(|app, argv, _cwd| {
            if let Some(link) = argv.iter().find(|a| a.starts_with("omniget://canvas")) {
                crate::canvas::spotlight::open_link(app, link);
                return;
            }
            if let Some(url) =
                external_url::find_external_url_arg(argv.iter().skip(1).map(|arg| arg.as_str()))
            {
                let app_handle = app.clone();
                tauri::async_runtime::spawn(async move {
                    if let Err(error) =
                        external_url::handle_external_url(&app_handle, url, "command-line").await
                    {
                        tracing::warn!(
                            "Failed to handle external URL from second instance: {}",
                            error
                        );
                    }
                });
            } else if !argv
                .iter()
                .any(|a| a == omniget_core::core::app_launch::BACKGROUND_FLAG)
            {
                tray::show_window(app);
            }
        }))
    };
    builder
        .manage(state)
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(|app, shortcut, event| {
                    let pressed = event.state == tauri_plugin_global_shortcut::ShortcutState::Pressed;
                    if pressed {
                        hotkey::on_hotkey_pressed(app, shortcut);
                    }
                })
                .build(),
        )
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_deep_link::init())
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            Some(vec![omniget_core::core::app_launch::AUTOSTART_FLAG]),
        ))
        .setup(|app| {
            crate::agentkit_boot::spawn_ensure_shim();
            crate::pty::manager().bind(app.handle());
            crate::canvas::chat::bind(app.handle());
            crate::portal::bind(app.handle());
            crate::canvas::routines::start(app.handle());
            crate::canvas::companion::start(app.handle());
            crate::canvas::power::start(app.handle());
            crate::canvas::generate::start(app.handle());
            let _ = crate::canvas::artifacts_http::ensure();
            crate::canvas::spotlight::start();
            tauri::async_runtime::spawn(crate::local_bridge_remote::boot(app.handle().clone()));
            omniget_core::core::secrets::install(std::sync::Arc::new(secrets::AppSecretStore));
            crate::llm_audit::boot(app.handle());
            crate::llm_budget::boot(app.handle());
            crate::job_workflow::boot();

            // WHY: the main window is built here, not in tauri.conf.json, because only here can it get .data_directory() for WebView2.
            // A janela principal e criada aqui, e nao pelo `tauri.conf.json`
            // (`"create": false`), porque so daqui da para passar
            // `.data_directory(...)` ao WebView2. O Tauri resolve esse caminho
            // para `LocalData/{identifier}` e cria o diretorio *antes* do setup
            // rodar, entao a variavel de ambiente que o `main.rs` define nunca
            // chega a ser lida — que e a causa da issue #195, o modo portatil
            // deixando uma pasta fora do diretorio do app.
            {
                #[cfg_attr(not(any(windows, target_os = "linux")), allow(unused_mut))]
                let mut builder = tauri::WebviewWindowBuilder::from_config(
                    app.handle(),
                    &app.config().app.windows[0],
                )?;

                #[cfg(any(windows, target_os = "linux"))]
                let webview_dir = core::portable::portable_webview_dir_from_env();
                #[cfg(all(debug_assertions, any(windows, target_os = "linux")))]
                let webview_dir = webview_dir.or_else(core::portable::dev_webview_dir_from_env);
                #[cfg(any(windows, target_os = "linux"))]
                if let Some(webview_dir) = webview_dir {
                    if let Err(e) = std::fs::create_dir_all(&webview_dir) {
                        tracing::warn!(
                            "[portable] nao foi possivel criar {}: {} — o WebView2 vai usar o diretorio padrao",
                            webview_dir.display(),
                            e
                        );
                    } else {
                        tracing::info!(
                            "[portable] WebView2 apontado para {}",
                            webview_dir.display()
                        );
                        builder = builder.data_directory(webview_dir);
                    }
                }

                #[cfg(target_os = "macos")]
                if core::portable::portable_webview_dir_from_env().is_some() {
                    tracing::warn!(
                        "[portable] no macOS o WebView guarda dados em ~/Library mesmo em modo \
                         portatil — o wry nao permite redirecionar. Ver github.com/tonhowtf/omniget/issues/227"
                    );
                }

                let builder = builder.visible(!launched_hidden(app.handle()));
                builder.build()?;

                tracing::info!("[startup] main window created");
            }

            if let Ok(raw) = std::env::var("OMNIGET_SMOKE_EXIT_MS") {
                if let Ok(ms) = raw.trim().parse::<u64>() {
                    let handle = app.handle().clone();
                    std::thread::spawn(move || {
                        std::thread::sleep(std::time::Duration::from_millis(ms));
                        tracing::info!("[startup] smoke mode: exiting cleanly");
                        handle.exit(0);
                    });
                }
            }

            if commands::pet::load_prefs().enabled {
                let handle = app.handle().clone();
                tauri::async_runtime::spawn(async move {
                    if let Err(error) = commands::pet::pet_open(handle).await {
                        tracing::warn!("pet window did not reopen: {error}");
                    }
                });
            }

            limits_strip::commands::restore(app.handle());

            {
                let handle = app.handle().clone();
                omniget_core::platforms::bilibili::notify::set_emitter(Box::new(
                    move |event: &str, payload: serde_json::Value| {
                        use tauri::Emitter;
                        let _ = handle.emit(event, payload);
                    },
                ));
            }
            let settings = storage::settings_repository::global().settings();
            core::http_client::init_proxy(settings.proxy.clone());
            core::http_fetcher::set_global_max_concurrent_segments(
                settings.advanced.max_concurrent_segments as usize,
            );
            omniget_core::platforms::cookie_provider::set_cookie_provider(Arc::new(
                DesktopCookieProvider,
            ));
            omniget_core::platforms::bilibili::set_runtime_provider(Arc::new(
                DesktopBilibiliRuntimeProvider,
            ));
            core::ytdlp::set_per_domain_cookie_fn(|url| {
                let parsed = url::Url::parse(url).ok()?;
                let host = parsed.host_str()?;
                let root = crate::cookies::root_domain_of(host);
                if root.is_empty() {
                    return None;
                }
                let slug = omniget_core::core::log_hook::current_cookie_slug();
                let path = crate::cookies::account_path_for_consumer(&root, slug.as_deref())?;
                crate::cookies::touch_last_used(&root, slug.as_deref().unwrap_or("_default"));
                Some(path)
            });
            core::ytdlp::set_managed_cookies_only_fn(|| {
                storage::config::load_settings_standalone()
                    .download
                    .always_use_managed_cookies
            });
            core::ytdlp::set_global_cookie_file_fn(|| {
                let s = storage::config::load_settings_standalone();
                let cf = s.download.cookie_file.clone();
                if !cf.is_empty() && std::path::Path::new(&cf).exists() {
                    Some(cf)
                } else {
                    None
                }
            });
            core::ytdlp::set_cookies_from_browser_fn(|| {
                storage::config::load_settings_standalone()
                    .advanced
                    .cookies_from_browser
            });
            core::ytdlp::set_manual_cookie_header_fn(|| {
                storage::config::load_settings_standalone()
                    .advanced
                    .twitter_manual_cookie
            });
            core::ytdlp::set_ext_referer_fn(|url| {
                extension_storage::read_extension_metadata(url).and_then(|m| m.referer)
            });
            core::ytdlp::set_include_auto_subs_fn(|| {
                storage::config::load_settings_standalone()
                    .download
                    .include_auto_subtitles
            });
            core::ytdlp::set_caption_locale_fn(|| {
                let s = storage::config::load_settings_standalone();
                let locale = s.download.caption_locale.trim().to_string();
                if locale.is_empty() || locale.eq_ignore_ascii_case("auto") {
                    let lang = s.appearance.language.trim().to_string();
                    if lang.is_empty() {
                        "en".to_string()
                    } else {
                        lang
                    }
                } else {
                    locale
                }
            });
            core::ytdlp::set_keep_vtt_fn(|| {
                storage::config::load_settings_standalone()
                    .download
                    .keep_vtt
            });
            core::ytdlp::set_translate_metadata_fn(|| {
                let s = storage::config::load_settings_standalone();
                if s.download.translate_metadata {
                    let lang = s.appearance.language.trim();
                    if lang.is_empty() {
                        None
                    } else {
                        Some(lang.to_string())
                    }
                } else {
                    None
                }
            });
            core::ytdlp::set_sponsorblock_fn(|| {
                storage::config::load_settings_standalone()
                    .download
                    .youtube_sponsorblock
            });
            core::ytdlp::set_sponsorblock_mode_fn(|| {
                storage::config::load_settings_standalone()
                    .download
                    .sponsorblock_mode
            });
            core::ytdlp::set_sponsorblock_categories_fn(|| {
                storage::config::load_settings_standalone()
                    .download
                    .sponsorblock_categories
            });
            core::ytdlp::set_split_chapters_fn(|| {
                storage::config::load_settings_standalone()
                    .download
                    .split_by_chapters
            });
            core::ytdlp::set_embed_metadata_fn(|| {
                storage::config::load_settings_standalone()
                    .download
                    .embed_metadata
            });
            core::ytdlp::set_embed_thumbnail_fn(|| {
                storage::config::load_settings_standalone()
                    .download
                    .embed_thumbnail
            });
            core::ytdlp::set_speed_limit_fn(|| {
                let v = storage::config::load_settings_standalone()
                    .download
                    .speed_limit;
                let t = v.trim();
                if t.is_empty() {
                    None
                } else {
                    Some(t.to_string())
                }
            });
            core::ytdlp::set_live_from_start_fn(|| {
                storage::config::load_settings_standalone()
                    .download
                    .live_from_start
            });
            core::ytdlp::set_insecure_tls_fn(|| {
                storage::config::load_settings_standalone()
                    .advanced
                    .insecure_tls
            });
            core::ytdlp::set_concurrent_fragments_fn(|| {
                storage::config::load_settings_standalone()
                    .advanced
                    .concurrent_fragments
            });
            core::ytdlp::set_user_agent_fn(|| {
                let v = storage::config::load_settings_standalone()
                    .advanced
                    .user_agent;
                let t = v.trim();
                if t.is_empty() {
                    None
                } else {
                    Some(t.to_string())
                }
            });
            {
                if core::download_journal::init().is_err() {
                    tracing::warn!("download journal unavailable; evidence persistence is incomplete");
                }
                let app_handle = app.handle().clone();
                omniget_core::core::log_hook::set_log_sink(std::sync::Arc::new(move |id, line| {
                    let line = core::flight_recorder::redact(line);
                    let should_emit = core::download_log::push_line(id, &line);
                    if should_emit {
                        let _ = tauri::Emitter::emit(
                            &app_handle,
                            "download-log-update",
                            serde_json::json!({ "id": id, "line": line }),
                        );
                    }
                }));
            }
            core::recovery::init_from_disk();
            core::queue_history::init_from_disk();
            std::thread::spawn(omniget_core::core::secrets::forget_removed_meetings_credentials);
            core::channels::init_from_disk();
            core::channel_poller::start(app.handle().clone());
            if let Some(dir) = crate::core::paths::app_data_dir() {
                omniget_core::core::tube::innertube::set_clients_override(
                    dir.join("tube").join("clients.json"),
                );
            }
            commands::tube::account::spawn_session_refresh(app.handle().clone());
            commands::tube::autodl::spawn_poller(app.handle().clone());
            core::queue::start_scheduler(app.handle().clone());
            commands::league::start_background(app.handle().clone());
            commands::telegram::install(app.handle());
            commands::washmybelly::start_background(app.handle().clone());
            commands::global_dl::start_background(app.handle().clone());
            {
                let app_handle = app.handle().clone();
                tauri::async_runtime::spawn(async move {
                    let state = app_handle.state::<AppState>();
                    let snapshot = {
                        let mut q = state.download_queue.lock().await;
                        q.hydrate_from_history();
                        q.get_state()
                    };
                    if !snapshot.is_empty() {
                        core::queue::emit_queue_state_from_state(&app_handle, snapshot);
                    }
                });
            }
            {
                let pending = core::recovery::list();
                if !pending.is_empty() {
                    let app_handle = app.handle().clone();
                    tauri::async_runtime::spawn(async move {
                        tokio::time::sleep(std::time::Duration::from_millis(800)).await;
                        let _ = tauri::Emitter::emit(
                            &app_handle,
                            "recovery-available",
                            serde_json::json!({ "count": pending.len() }),
                        );
                    });
                }
            }
            {
                let app_handle = app.handle().clone();
                app.deep_link().on_open_url(move |event| {
                    for url in event.urls() {
                        let raw = url.to_string();
                        let handle = app_handle.clone();
                        if raw.starts_with("omniget://openrouter-auth") {
                            let code = url
                                .query_pairs()
                                .find(|(k, _)| k == "code")
                                .map(|(_, v)| v.into_owned())
                                .unwrap_or_default();
                            let state = url
                                .query_pairs()
                                .find(|(k, _)| k == "state")
                                .map(|(_, v)| v.into_owned());
                            tauri::async_runtime::spawn(async move {
                                use tauri::Emitter;
                                let payload = match omniget_core::core::tools::ai_keys::pkce_finish(
                                    &code,
                                    state.as_deref(),
                                )
                                .await
                                {
                                    Ok(key) => serde_json::json!({ "ok": true, "key": key }),
                                    Err(error) => {
                                        tracing::warn!("OpenRouter PKCE exchange failed: {error}");
                                        serde_json::json!({ "ok": false, "error": error.to_string() })
                                    }
                                };
                                let _ = handle.emit("llm://openrouter-auth", payload);
                            });
                            continue;
                        }
                        if crate::canvas::spotlight::open_link(&handle, &raw) {
                            continue;
                        }
                        if raw.starts_with("omniget://watch") {
                            use tauri::Emitter;
                            let q: std::collections::HashMap<String, String> =
                                url.query_pairs().into_owned().collect();
                            crate::tray::show_window(&handle);
                            let _ = handle.emit(
                                "video-deep-link",
                                serde_json::json!({
                                    "key": q.get("key").cloned().unwrap_or_default(),
                                    "t": q.get("t").and_then(|t| t.parse::<f64>().ok()).unwrap_or(0.0),
                                }),
                            );
                            continue;
                        }
                        tauri::async_runtime::spawn(async move {
                            if let Err(error) =
                                external_url::handle_external_url(&handle, raw, "deep-link").await
                            {
                                tracing::warn!("Failed to handle deep-link URL: {}", error);
                            }
                        });
                    }
                });
            }
            #[cfg(any(target_os = "linux", target_os = "windows"))]
            {
                if let Err(error) = app.deep_link().register_all() {
                    tracing::warn!("Failed to register omniget:// scheme: {}", error);
                }
            }
            tray::setup(app.handle())?;
            usage_tray::setup(app.handle());
            hotkey::register_from_settings(app.handle());
            storage::settings_effects::install(app.handle().clone());

            extension_storage::cleanup_legacy_native_messaging();

            if !cookies::storage::has_been_migrated() {
                match cookies::migrate_legacy_if_needed() {
                    Ok(_) => {
                        let legacy = extension_storage::extension_cookie_file_path();
                        if legacy.exists() {
                            if let Err(err) = std::fs::remove_file(&legacy) {
                                tracing::warn!(
                                    "cookies: failed to remove legacy file {}: {}",
                                    legacy.display(),
                                    err
                                );
                            }
                        }
                    }
                    Err(err) => tracing::warn!("cookies: legacy migration skipped: {err}"),
                }
            } else {
                let legacy = extension_storage::extension_cookie_file_path();
                if legacy.exists() {
                    if let Err(err) = std::fs::remove_file(&legacy) {
                        tracing::warn!(
                            "cookies: failed to remove stale legacy file {}: {}",
                            legacy.display(),
                            err
                        );
                    }
                }
            }

            {
                let app_handle = app.handle().clone();
                tauri::async_runtime::spawn(async move {
                    local_bridge::spawn(app_handle.clone()).await;
                    jobs::boot(&app_handle);
                    company::boot(&app_handle);
                });
            }
            std::thread::Builder::new()
                .name("startup-checks".into())
                .spawn(|| {
                    let rt = tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()
                        .expect("startup runtime");
                    rt.block_on(async {
                        if let Err(e) = core::ytdlp::ensure_ytdlp().await {
                            tracing::warn!("yt-dlp not available at startup: {}", e);
                        }
                        core::dependencies::ensure_js_runtime().await;
                        let _ = tokio::task::spawn_blocking(
                            core::ytdlp::cleanup_stale_pyinstaller_dirs,
                        )
                        .await;
                        tokio::time::sleep(std::time::Duration::from_secs(45)).await;
                        for _ in 0..12 {
                            let Some(ytdlp) = core::ytdlp::find_ytdlp_cached().await else {
                                break;
                            };
                            match core::ytdlp::check_ytdlp_update_detailed(&ytdlp).await {
                                Ok(core::ytdlp::UpdateCheck::Updated) => {
                                    tracing::info!("yt-dlp updated successfully");
                                    break;
                                }
                                Ok(core::ytdlp::UpdateCheck::UpToDate)
                                | Ok(core::ytdlp::UpdateCheck::AlreadyChecked) => {
                                    tracing::debug!("yt-dlp already up to date");
                                    break;
                                }
                                Ok(core::ytdlp::UpdateCheck::Busy) => {
                                    tracing::debug!(
                                        "yt-dlp update check deferred: downloads running"
                                    );
                                    tokio::time::sleep(std::time::Duration::from_secs(300))
                                        .await;
                                }
                                Err(e) => {
                                    tracing::warn!("yt-dlp update check failed: {}", e);
                                    break;
                                }
                            }
                        }
                    });
                })
                .ok();

            if let Some(url) = external_url::find_external_url_arg(std::env::args().skip(1)) {
                let app_handle = app.handle().clone();
                tauri::async_runtime::spawn(async move {
                    if let Err(error) =
                        external_url::handle_external_url(&app_handle, url, "command-line").await
                    {
                        tracing::warn!("Failed to handle startup external URL: {}", error);
                    }
                });
            }

            let hidden = launched_hidden(app.handle());
            if hidden {
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.hide();
                }
                let args: Vec<String> = std::env::args().collect();
                let has = |flag: &str| args.iter().any(|a| a == flag);
                crate::llm_audit::app_started_hidden(
                    if has(omniget_core::core::app_launch::BACKGROUND_FLAG) {
                        "cli"
                    } else if has(omniget_core::core::app_launch::AUTOSTART_FLAG) {
                        "login"
                    } else {
                        "tray"
                    },
                );
            }
            if let Some(dir) = omniget_core::core::paths::app_data_dir() {
                if let Some(launch) = omniget_core::core::app_launch::current(hidden) {
                    if let Err(e) = omniget_core::core::app_launch::record_in(&dir, &launch) {
                        tracing::warn!("[startup] app-launch.json: {e}");
                    }
                }
            }

            Ok(())
        })
        .on_window_event(|window, event| match event {
            tauri::WindowEvent::CloseRequested { api, .. } if window.label() == "main" => {
                api.prevent_close();
                let s = storage::config::load_settings(window.app_handle());
                if s.exit_on_close && !s.run_in_background {
                    tray::request_quit(window.app_handle());
                } else {
                    let _ = window.hide();
                }
            }
            _ => {}
        })
        .invoke_handler(handlers::invoke_handler())
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app_handle, event| {
            if let tauri::RunEvent::ExitRequested { .. } = &event {
                omniget_core::core::washmybelly::game::shutdown();
                omniget_core::core::telegram::net::mihomo::shutdown();
                tauri::async_runtime::block_on(omniget_core::core::llm::local_servers::stop());
                crate::pty::manager().shutdown_all();
                crate::canvas::doc::flush_on_exit();
                tauri::async_runtime::block_on(async {
                    let _ = tokio::time::timeout(
                        std::time::Duration::from_secs(5),
                        crate::threads_host::shutdown(),
                    )
                    .await;
                });
                let state = app_handle.state::<AppState>();
                let session_mutex = state.torrent_session.clone();
                tauri::async_runtime::block_on(async move {
                    let session_guard = session_mutex.lock().await;
                    let session = session_guard.as_ref().cloned();
                    drop(session_guard);
                    if let Some(session) = session {
                        match tokio::time::timeout(
                            std::time::Duration::from_secs(5),
                            session.stop(),
                        )
                        .await
                        {
                            Ok(()) => tracing::info!("torrent session stopped cleanly"),
                            Err(_) => tracing::warn!(
                                "torrent session stop timed out after 5s; exiting anyway"
                            ),
                        }
                    }
                });
            }
        });
}

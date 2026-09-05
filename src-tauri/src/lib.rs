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

pub mod commands;
pub mod cookies;
pub mod core;
pub mod extension_storage;
pub mod external_url;
pub mod hotkey;
pub mod local_bridge;
pub mod mcp;
pub mod models;
pub mod platforms;
pub mod plugin_host;
pub mod plugin_loader;
pub mod storage;
pub mod tray;

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
    pub omnidisc_gateways: commands::omnidisc::gateway::Gateways,
    pub omnidisc_voice: Arc<commands::omnidisc::voice::VoiceManager>,
    pub omnidisc_stream: Arc<commands::omnidisc::stream::StreamManager>,
    pub omnidisc_mls: Arc<commands::omnidisc::mls::MlsManager>,
    pub omnidisc_uploads: Arc<commands::omnidisc::upload::UploadManager>,
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tracing_subscriber::fmt::init();

    // Emitido antes do single-instance poder sair em silencio: ver
    // core::portable::startup_banner.
    tracing::info!(
        "{}",
        core::portable::startup_banner(
            env!("CARGO_PKG_VERSION"),
            std::process::id(),
            std::env::var("OMNIGET_PORTABLE").ok().as_deref() == Some("1"),
            core::paths::app_data_dir().as_deref(),
        )
    );

    let mut registry = core::registry::PlatformRegistry::new();
    // gallery-dl first: it claims bulk-listing URLs (profiles, subreddits)
    // that the single-post Twitter/Reddit downloaders would otherwise match
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
        omnidisc_gateways: commands::omnidisc::gateway::new_gateways(),
        omnidisc_voice: Arc::new(commands::omnidisc::voice::VoiceManager::new()),
        omnidisc_stream: Arc::new(commands::omnidisc::stream::StreamManager::default()),
        omnidisc_mls: Arc::new(commands::omnidisc::mls::MlsManager::default()),
        omnidisc_uploads: Arc::new(commands::omnidisc::upload::UploadManager::default()),
    };

    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, argv, _cwd| {
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
            } else {
                tray::show_window(app);
            }
        }))
        .manage(state)
        .manage(Arc::new(tokio::sync::RwLock::new(
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
                .with_handler(|app, shortcut, event| {
                    let pressed = event.state == tauri_plugin_global_shortcut::ShortcutState::Pressed;
                    if hotkey::handle_ptt(app, shortcut, pressed) {
                        return;
                    }
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
            None,
        ))
        .setup(|app| {
            // A janela principal e criada aqui, e nao pelo `tauri.conf.json`
            // (`"create": false`), porque so daqui da para passar
            // `.data_directory(...)` ao WebView2. O Tauri resolve esse caminho
            // para `LocalData/{identifier}` e cria o diretorio *antes* do setup
            // rodar, entao a variavel de ambiente que o `main.rs` define nunca
            // chega a ser lida — que e a causa da issue #195, o modo portatil
            // deixando uma pasta fora do diretorio do app.
            {
                // `mut` so e usado no ramo Windows abaixo; sem o cfg_attr isto
                // vira warning novo em macOS e Linux e reprova o portao de clippy.
                #[cfg_attr(not(any(windows, target_os = "linux")), allow(unused_mut))]
                let mut builder = tauri::WebviewWindowBuilder::from_config(
                    app.handle(),
                    &app.config().app.windows[0],
                )?;

                // Nao e so Windows. No Linux o wry usa este caminho para
                // `base_data_directory`, `base_cache_directory` e os cookies do
                // WebKitGTK; sem ele o modo portatil deixa
                // `XDG_DATA_HOME/wtf.tonho.omniget` no perfil do usuario — a
                // mesma #209, fora do Windows. Foi o smoke test do B55 que
                // pegou isso, na primeira vez que rodou.
                //
                // macOS fica de fora porque o wry nao le `data_directory` no
                // WKWebView: incluir daria a impressao de cobrir um caso que
                // nao esta coberto.
                #[cfg(any(windows, target_os = "linux"))]
                if let Some(webview_dir) = core::portable::portable_webview_dir_from_env() {
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

                // macOS nao tem para onde apontar: o wry nao le `data_directory`
                // no WKWebView (nenhuma referencia em `src/wkwebview/`), ao
                // contrario do webkitgtk, que a usa para base_data_directory,
                // base_cache_directory e cookies.
                //
                // Entao o modo portatil nao cumpre o que promete aqui, e dizer
                // isso e melhor do que deixar o usuario achar que o pendrive nao
                // deixou rastro. Issue #227.
                #[cfg(target_os = "macos")]
                if core::portable::portable_webview_dir_from_env().is_some() {
                    tracing::warn!(
                        "[portable] no macOS o WebView guarda dados em ~/Library mesmo em modo \
                         portatil — o wry nao permite redirecionar. Ver github.com/tonhowtf/omniget/issues/227"
                    );
                }

                builder.build()?;

                // A unica prova de que a janela subiu. O CI compila em todas as
                // plataformas mas nunca abriu o app: foi assim que a #209 passou
                // batido. O smoke test do CI casa exatamente esta linha.
                tracing::info!("[startup] main window created");
            }

            // Modo smoke: sobe, prova que a janela existe, e sai sozinho com 0.
            // So existe para o CI conseguir responder "abre?", que e a pergunta
            // que compilar nunca responde.
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

            commands::host_queue::register_event_listeners(app.handle());
            {
                let handle = app.handle().clone();
                platforms::bilibili::notify::set_emitter(Box::new(
                    move |event: &str, payload: serde_json::Value| {
                        use tauri::Emitter;
                        let _ = handle.emit(event, payload);
                    },
                ));
            }
            {
                let handle = app.handle().clone();
                omniget_core::platforms::bilibili::notify::set_emitter(Box::new(
                    move |event: &str, payload: serde_json::Value| {
                        use tauri::Emitter;
                        let _ = handle.emit(event, payload);
                    },
                ));
            }
            let settings = storage::config::load_settings(app.handle());
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
                let app_handle = app.handle().clone();
                omniget_core::core::log_hook::set_log_sink(std::sync::Arc::new(move |id, line| {
                    // B33: caixa-preta. Toda linha de log de download passa por
                    // aqui, e o `record` redige antes de guardar — e o unico
                    // ponto onde da para capturar o historico sem instrumentar
                    // cada chamada uma por uma.
                    core::flight_recorder::record(line);
                    let should_emit = core::download_log::push_line(id, line);
                    if should_emit {
                        // A linha vai junto para o card mostrar "a última
                        // coisa que o yt-dlp disse" sem pedir o log inteiro
                        // a cada evento.
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
            core::channels::init_from_disk();
            core::channel_poller::start(app.handle().clone());
            core::queue::start_scheduler(app.handle().clone());
            commands::league::start_background(app.handle().clone());
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
            // Make sure the OS routes `omniget://` to this binary even when the
            // package didn't ship a desktop file with `MimeType=x-scheme-handler/omniget;`
            // or when the user is running from an AppImage / unpacked build.
            // No-op on macOS (which uses Info.plist registered by the bundler).
            #[cfg(any(target_os = "linux", target_os = "windows"))]
            {
                if let Err(error) = app.deep_link().register_all() {
                    tracing::warn!("Failed to register omniget:// scheme: {}", error);
                }
            }
            tray::setup(app.handle())?;
            hotkey::register_from_settings(app.handle());
            commands::omnidisc::voice::start(app.handle());

            // Migration: drop the manifests / binary copies the previous
            // native-messaging code left under `~/.config/...` so Chrome and
            // Firefox stop trying to start the legacy host process.
            extension_storage::cleanup_legacy_native_messaging();

            // Cookie Manager: reparticiona `chrome-extension-cookies.txt` legacy em
            // `cookies/<domain>/_default.txt` (CK-1). Quando o registry já tem
            // buckets, a migration vira no-op. Após migration bem-sucedida, o
            // arquivo legacy é deletado — a partir de CK-7b o multi-file é a
            // única fonte de verdade.
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
                // Registry já populado — significa migration anterior já rolou; se
                // o legacy ainda existe (instalação intermediária), remove agora.
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

            // Start the localhost HTTP bridge for the browser extension. This
            // replaces the previous Chrome native-messaging path, which forced
            // us to maintain a hard-coded extension-ID allowlist. Bridge auth
            // is via a per-installation token shown in the Settings UI.
            {
                let app_handle = app.handle().clone();
                tauri::async_runtime::spawn(async move {
                    local_bridge::spawn(app_handle).await;
                });
            }
            {
                let plugins_dir = core::paths::app_data_dir()
                    .unwrap_or_else(|| std::path::PathBuf::from("."))
                    .join("plugins");
                let host: std::sync::Arc<dyn omniget_plugin_sdk::PluginHost> = std::sync::Arc::new(
                    plugin_host::PluginHostImpl::new(app.handle().clone(), plugins_dir),
                );
                let plugin_mgr = app
                    .handle()
                    .state::<std::sync::Arc<tokio::sync::RwLock<plugin_loader::PluginManager>>>();
                let mgr_for_plugins = std::sync::Arc::clone(&*plugin_mgr);
                let app_emit = app.handle().clone();
                std::thread::Builder::new()
                    .name("plugins-bootstrap".into())
                    .spawn(move || {
                        use tauri::Emitter;

                        // Load already-installed plugins first so they are
                        // usable immediately (and offline), without waiting
                        // on any of the network calls below.
                        {
                            let mut mgr = mgr_for_plugins.blocking_write();
                            mgr.load_all(std::sync::Arc::clone(&host));
                        }
                        let _ = app_emit.emit("plugins-changed", ());

                        let rt = match tokio::runtime::Runtime::new() {
                            Ok(rt) => rt,
                            Err(e) => {
                                tracing::warn!("plugins-bootstrap runtime failed: {}", e);
                                return;
                            }
                        };
                        rt.block_on(commands::plugins::ensure_default_plugins(
                            std::sync::Arc::clone(&mgr_for_plugins),
                        ));
                        rt.block_on(commands::plugins::auto_update_plugins(
                            std::sync::Arc::clone(&mgr_for_plugins),
                        ));

                        // Load anything newly installed above. load_all is
                        // not idempotent (re-inserting drops the previously
                        // loaded plugin and its dylib), so only load entries
                        // that are not loaded yet.
                        {
                            let mut mgr = mgr_for_plugins.blocking_write();
                            let to_load: Vec<String> = mgr
                                .installed_plugins()
                                .iter()
                                .filter(|p| p.enabled && !mgr.is_loaded(&p.id))
                                .map(|p| p.id.clone())
                                .collect();
                            for id in &to_load {
                                let _ = mgr.load_one(id, std::sync::Arc::clone(&host));
                            }
                        }
                        let _ = app_emit.emit("plugins-changed", ());
                    })
                    .ok();
            }

            std::thread::Builder::new()
                .name("startup-checks".into())
                .spawn(|| {
                    let rt = tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()
                        .expect("startup runtime");
                    rt.block_on(async {
                        // Garante o yt-dlp (zipapp quando há Python, senão o
                        // onefile) antes do primeiro download pedir por ele.
                        if let Err(e) = core::ytdlp::ensure_ytdlp().await {
                            tracing::warn!("yt-dlp not available at startup: {}", e);
                        }
                        core::dependencies::ensure_js_runtime().await;
                        let _ = tokio::task::spawn_blocking(
                            core::ytdlp::cleanup_stale_pyinstaller_dirs,
                        )
                        .await;
                        // O `--update-to` é um bootstrap inteiro do yt-dlp
                        // mais rede, e trocar o binário com download ativo
                        // quebra o processo (B2). Espera o app assentar e só
                        // roda sem yt-dlp em andamento; se a fila está cheia,
                        // tenta de novo a cada 5 min por até uma hora.
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

            if settings.start_minimized {
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.hide();
                }
            }

            Ok(())
        })
        .on_window_event(|window, event| match event {
            tauri::WindowEvent::CloseRequested { api, .. } if window.label() == "main" => {
                api.prevent_close();
                let _ = window.hide();
            }
            tauri::WindowEvent::Destroyed
                if window.label().starts_with("omnidisc-stream-") =>
            {
                commands::omnidisc::stream::on_stream_window_destroyed(
                    &window.app_handle().clone(),
                    window.label(),
                );
            }
            _ => {}
        })
        .invoke_handler(tauri::generate_handler![
            commands::auth_webview::open_auth_webview,
            commands::omnidisc::omnidisc_connect,
            commands::omnidisc::auth::omnidisc_register,
            commands::omnidisc::auth::omnidisc_login,
            commands::omnidisc::auth::omnidisc_logout,
            commands::omnidisc::auth::omnidisc_has_session,
            commands::omnidisc::api::omnidisc_list_messages,
            commands::omnidisc::api::omnidisc_send_message,
            commands::omnidisc::api::omnidisc_edit_message,
            commands::omnidisc::api::omnidisc_delete_message,
            commands::omnidisc::api::omnidisc_add_reaction,
            commands::omnidisc::api::omnidisc_remove_reaction,
            commands::omnidisc::api::omnidisc_ack,
            commands::omnidisc::gateway::omnidisc_typing,
            commands::omnidisc::api::omnidisc_create_guild,
            commands::omnidisc::api::omnidisc_create_channel,
            commands::omnidisc::api::omnidisc_create_invite,
            commands::omnidisc::api::omnidisc_join_invite,
            commands::omnidisc::api::omnidisc_create_dm,
            commands::omnidisc::api::omnidisc_update_me,
            commands::omnidisc::api::omnidisc_get_user,
            commands::omnidisc::api::omnidisc_get_guild,
            commands::omnidisc::api::omnidisc_get_me,
            commands::omnidisc::api::omnidisc_search,
            commands::omnidisc::api::omnidisc_list_pins,
            commands::omnidisc::api::omnidisc_pin_message,
            commands::omnidisc::api::omnidisc_list_relationships,
            commands::omnidisc::api::omnidisc_add_relationship,
            commands::omnidisc::api::omnidisc_accept_relationship,
            commands::omnidisc::api::omnidisc_remove_relationship,
            commands::omnidisc::api::omnidisc_block_user,
            commands::omnidisc::api::omnidisc_list_notes,
            commands::omnidisc::api::omnidisc_put_note,
            commands::omnidisc::api::omnidisc_update_guild,
            commands::omnidisc::api::omnidisc_delete_guild,
            commands::omnidisc::api::omnidisc_leave_guild,
            commands::omnidisc::api::omnidisc_transfer_guild,
            commands::omnidisc::api::omnidisc_create_role,
            commands::omnidisc::api::omnidisc_update_role,
            commands::omnidisc::api::omnidisc_delete_role,
            commands::omnidisc::api::omnidisc_set_member_role,
            commands::omnidisc::api::omnidisc_update_member,
            commands::omnidisc::api::omnidisc_kick_member,
            commands::omnidisc::api::omnidisc_ban_member,
            commands::omnidisc::api::omnidisc_unban_member,
            commands::omnidisc::api::omnidisc_list_bans,
            commands::omnidisc::api::omnidisc_audit_log,
            commands::omnidisc::api::omnidisc_update_channel,
            commands::omnidisc::api::omnidisc_delete_channel,
            commands::omnidisc::api::omnidisc_put_overwrite,
            commands::omnidisc::api::omnidisc_delete_overwrite,
            commands::omnidisc::api::omnidisc_list_sessions,
            commands::omnidisc::api::omnidisc_revoke_session,
            commands::omnidisc::api::omnidisc_revoke_other_sessions,
            commands::omnidisc::device::omnidisc_device_fingerprint,
            commands::omnidisc::device::omnidisc_list_user_devices,
            commands::omnidisc::device::omnidisc_revoke_device,
            commands::omnidisc::mls::omnidisc_mls_sync,
            commands::omnidisc::mls::omnidisc_mls_status,
            commands::omnidisc::mls::omnidisc_mls_recall,
            commands::omnidisc::mls::omnidisc_mls_device_revoked,
            commands::omnidisc::upload::omnidisc_instance_limits,
            commands::omnidisc::upload::omnidisc_stage_file,
            commands::omnidisc::upload::omnidisc_upload_start,
            commands::omnidisc::upload::omnidisc_upload_cancel,
            commands::omnidisc::upload::omnidisc_download_attachment,
            commands::omnidisc::gateway::omnidisc_gateway_connect,
            commands::omnidisc::gateway::omnidisc_gateway_disconnect,
            commands::omnidisc::gateway::omnidisc_gateway_send,
            commands::omnidisc::gateway::omnidisc_gateway_status,
            commands::omnidisc::voice::omnidisc_voice_join,
            commands::omnidisc::voice::omnidisc_voice_leave,
            commands::omnidisc::voice::omnidisc_voice_set_mute,
            commands::omnidisc::voice::omnidisc_voice_set_deaf,
            commands::omnidisc::voice::omnidisc_voice_set_volume,
            commands::omnidisc::voice::omnidisc_voice_devices,
            commands::omnidisc::voice::omnidisc_voice_set_device,
            commands::omnidisc::voice::omnidisc_voice_stats,
            commands::omnidisc::voice::omnidisc_voice_ptt,
            commands::omnidisc::voice::omnidisc_voice_status,
            commands::omnidisc::voice::omnidisc_voice_set_noise_suppression,
            commands::omnidisc::voice::omnidisc_voice_mic_test,
            commands::omnidisc::voice::omnidisc_voice_set_ducking,
            commands::omnidisc::voice::omnidisc_voice_ptt_status,
            commands::omnidisc::stream::omnidisc_media_capabilities,
            commands::omnidisc::stream::omnidisc_stream_sources,
            commands::omnidisc::stream::omnidisc_stream_start,
            commands::omnidisc::stream::omnidisc_stream_stop,
            commands::omnidisc::stream::omnidisc_stream_stats,
            commands::omnidisc::stream::omnidisc_stream_set_volume,
            commands::omnidisc::stream::omnidisc_stream_set_viewport,
            commands::omnidisc::stream::omnidisc_stream_watch,
            commands::omnidisc::stream::omnidisc_stream_unwatch,
            commands::league::league_status,
            commands::league::league_get,
            commands::league::league_install_dir,
            commands::league::league_set_positions,
            commands::league::league_end_of_game_stats,
            commands::league::league_set_icon,
            commands::league::league_set_profile_background,
            commands::league::league_set_status,
            commands::league::league_owned_skins,
            commands::league::league_summoner,
            commands::league::league_ranked,
            commands::league::league_gameflow,
            commands::league::league_match_detail,
            commands::league::league_player_history,
            commands::league::league_perks,
            commands::league::league_match_history,
            commands::league::league_accept_ready_check,
            commands::league::league_auto_accept_set,
            commands::league::league_auto_accept_get,
            commands::league::league_lobby_queues,
            commands::league::league_create_lobby,
            commands::league::league_start_matchmaking,
            commands::league::league_stop_matchmaking,
            commands::league::league_leave_lobby,
            commands::league::league_play_again,
            commands::league::league_champ_select_session,
            commands::league::league_bench_swap,
            commands::league::league_restart_ux,
            commands::league::league_reroll,
            commands::league::league_reroll_keeping_champion,
            commands::league::league_live_game,
            commands::league::league_game_players,
            commands::league::league_player_report,
            commands::league::league_match_analysis,
            commands::league::league_live_events,
            commands::league::league_live_metrics,
            commands::league::league_search_player,
            commands::league::league_duos,
            commands::league::league_jungle_report,
            commands::league::league_apply_runes,
            commands::league::league_send_chat,
            commands::league::league_rune_recommendations,
            commands::league::league_champion_meta,
            commands::league::league_champion_tiers,
            commands::league::league_champion_build,
            commands::league::league_ability_cooldowns,
            commands::league::league_spectate,
            commands::league::league_dodge,
            commands::league::profile::league_profile_state,
            commands::league::profile::league_set_chat_rank,
            commands::league::profile::league_reset_chat_rank,
            commands::league::profile::league_set_challenge_crystal,
            commands::league::profile::league_set_chat_icon,
            commands::league::profile::league_challenges,
            commands::league::profile::league_set_challenge_prefs,
            commands::league::profile::league_set_regalia,
            commands::league::profile::league_friends,
            commands::league::profile::league_remove_friends,
            commands::league::profile::league_random_champion,
            commands::league::profile::league_declare_champion,
            commands::league::skins::league_roll_skin,
            commands::league::skins::league_roll_ward,
            commands::league::skins::league_skin_carousel,
            commands::league::sgp::league_sgp_status,
            commands::league::sgp::league_sgp_match_history,
            commands::league::sgp::league_sgp_ranked,
            commands::league::sgp::league_sgp_summoners,
            commands::league::sgp::league_sgp_download_replay,
            commands::league::coach::league_coach_review,
            commands::league::coach::league_coach_trends,
            commands::league::coach::league_coach_ask,
            commands::league::coach::league_coach_ready,
            commands::bilibili_auth::bilibili_qr_generate,
            commands::bilibili_auth::bilibili_qr_poll,
            commands::bilibili_auth::bilibili_captcha_challenge,
            commands::bilibili_auth::bilibili_sms_send,
            commands::bilibili_auth::bilibili_sms_verify,
            commands::bilibili_auth::bilibili_account_status,
            commands::bilibili_auth::bilibili_import_watch_later,
            commands::bilibili_auth::bilibili_import_history,
            commands::bilibili_auth::bilibili_preview_info,
            commands::bilibili_auth::bilibili_webview_login,
            commands::browser_extension::browser_extension_status,
            commands::browser_extension::browser_extension_export,
            commands::browser_extension::browser_extension_open_folder,
            cookies::commands::cookies_list,
            cookies::commands::cookies_read,
            cookies::commands::cookies_import,
            cookies::commands::cookies_clear,
            cookies::commands::cookies_clear_batch,
            cookies::commands::cookies_rename,
            cookies::commands::cookies_accounts_for_url,
            cookies::commands::cookies_read_as_json,
            cookies::commands::cookies_import_file,
            cookies::commands::cookies_export_to,
            cookies::commands::cookies_add_account,
            cookies::commands::cookies_health,
            cookies::commands::cookies_test,
            commands::clip::clip_video,
            commands::reencode::reencode_video,
            commands::diagnostics::get_hwaccel_info,
            commands::diagnostics::diagnose_download_error,
            commands::downloads::detect_platform,
            commands::downloads::check_cookie_error,
            commands::downloads::validate_output_path,
            commands::downloads::get_media_formats,
            commands::downloads::prefetch_media_info,
            commands::downloads::download_from_url,
            commands::downloads::playlist_entries,
            commands::downloads::torrent_contents,
            commands::channels::channels_list,
            commands::channels::channel_add,
            commands::channels::channel_remove,
            commands::channels::channel_update,
            commands::channels::channel_check_now,
            commands::channels::sync_channels_tray,
            commands::ai::ai_get_config,
            commands::ai::ai_set_config,
            commands::ai::ai_test,
            commands::ai::ai_summarize_url,
            commands::ai::whisper_generate,
            commands::ai::ai_history_list,
            commands::ai::ai_history_clear,
            commands::video_ops::video_op_silence_estimate,
            commands::video_ops::video_op_preset,
            commands::video_ops::video_op_propose,
            commands::video_ops::video_op_run,
            commands::video_ops::detect_shot_changes,
            commands::video_ops::waveform_peaks,
            commands::subtitle_ws::subtitle_load,
            commands::subtitle_ws::subtitle_save,
            commands::subtitle_ws::subtitle_translate,
            commands::subtitle_ws::subtitle_grammar_fix,
            commands::downloads::metadata_fetch,
            commands::downloads::thumbnails_list,
            commands::downloads::thumbnail_save,
            commands::downloads::subtitles_list,
            commands::downloads::subtitles_save,
            commands::downloads::subtitles_merge,
            commands::downloads::comments_fetch,
            commands::downloads::chapters_fetch,
            commands::downloads::tools_save_text,
            commands::downloads::livechat_fetch,
            commands::downloads::download_with_custom_args,
            commands::downloads::cancel_generic_download,
            commands::yt_templates::yt_templates_list,
            commands::yt_templates::yt_templates_save,
            commands::yt_templates::yt_templates_delete,
            commands::downloads::pause_download,
            commands::downloads::resume_download,
            commands::downloads::pause_all_downloads,
            commands::downloads::resume_all_downloads,
            commands::downloads::reorder_queue,
            commands::downloads::retry_download,
            commands::downloads::remove_download,
            commands::downloads::update_max_concurrent,
            commands::downloads::clear_finished_downloads,
            commands::downloads::get_download_log,
            commands::downloads::get_download_command,
            commands::downloads::retry_download_with_command,
            commands::downloads::parse_batch_file,
            commands::downloads::get_recovery_items,
            commands::downloads::discard_recovery,
            commands::downloads::restore_recovery,
            commands::downloads::get_download_history,
            commands::downloads::clear_download_history,
            commands::downloads::reveal_file,
            commands::downloads::open_path_default,
            commands::host_queue::host_queue_enqueue_external,
            commands::host_queue::host_queue_report_progress,
            commands::host_queue::host_queue_report_complete,
            commands::integration::register_external_frontend,
            commands::settings::get_settings,
            commands::settings::update_settings,
            commands::settings::reset_settings,
            commands::settings::mark_onboarding_complete,
            commands::settings::mark_legal_acknowledged,
            commands::rpc::rpc_test_connection,
            commands::rpc::rpc_set_source,
            commands::rpc::rpc_clear_source,
            commands::rpc::rpc_set_idle_stats,
            commands::settings::get_bridge_info,
            commands::settings::rotate_bridge_token,
            commands::settings::bridge_open_pairing,
            commands::dependencies::check_dependencies,
            commands::spicetify::spicetify_status,
            commands::spicetify::spicetify_install,
            commands::spicetify::spicetify_action,
            commands::spicetify::spicetify_set_theme,
            commands::spicetify::spicetify_remove_addon,
            commands::spicetify::spicetify_install_marketplace,
            commands::tools::text::tool_humanize,
            commands::tools::ai::tool_ollama_status,
            commands::tools::ai::tool_ollama_recommended,
            commands::tools::ai::tool_ollama_pull,
            commands::tools::ai::tool_ollama_delete,
            commands::tools::ai::tool_pricing_info,
            commands::tools::ai::tool_pricing_search,
            commands::tools::ai::tool_pricing_for,
            commands::tools::ai::tool_usage_report,
            commands::tools::ai::tool_usage_clear,
            commands::tools::documents::tool_slideshare,
            commands::tools::documents::tool_gdocs_parse,
            commands::tools::documents::tool_gdocs_download,
            commands::tools::documents::tool_calameo,
            commands::tools::documents::tool_gallery_status,
            commands::tools::documents::tool_gallery_install,
            commands::tools::documents::tool_gallery_download,
            commands::tools::downloads::tool_aria2_status,
            commands::tools::downloads::tool_aria2_download,
            commands::tools::downloads::tool_manifest_download,
            commands::tools::files::tool_dupes_scan,
            commands::tools::files::tool_dupes_delete,
            commands::tools::files::tool_rename_plan,
            commands::tools::files::tool_rename_apply,
            commands::tools::files::tool_file_search_backend,
            commands::tools::files::tool_file_search,
            commands::tools::files::tool_awake_set,
            commands::tools::files::tool_awake_get,
            commands::tools::images::tool_upscale_status,
            commands::tools::images::tool_upscale_install,
            commands::tools::images::tool_upscale_run,
            commands::tools::images::tool_resize,
            commands::tools::images::tool_ocr_status,
            commands::tools::images::tool_ocr_run,
            commands::tools::phone::tool_kde_status,
            commands::tools::phone::tool_kde_share,
            commands::tools::phone::tool_kde_ping,
            commands::tools::phone::tool_kde_refresh,
            commands::tools::speech::tool_whisper_status,
            commands::tools::speech::tool_whisper_install,
            commands::tools::speech::tool_whisper_model_download,
            commands::tools::speech::tool_whisper_model_remove,
            commands::tools::speech::tool_whisper_transcribe,
            commands::tools::speech::tool_tts_voices,
            commands::tools::speech::tool_tts_speak,
            commands::tools::speech::tool_srt_translate,
            commands::tools::speech::tool_dub,
            commands::tools::ai::tool_keys_kinds,
            commands::tools::ai::tool_keys_list,
            commands::tools::ai::tool_keys_save,
            commands::tools::ai::tool_keys_delete,
            commands::tools::ai::tool_keys_test,
            commands::tools::ai::tool_keys_balance,
            commands::tools::ai::tool_keys_models,
            commands::tools::ai::tool_keys_export,
            commands::tools::ai::tool_keys_use,
            commands::tools::ai::tool_mcp_status,
            commands::tools::ai::tool_mcp_set_enabled,
            commands::tools::ai::tool_mcp_selftest,
            commands::tools::desktop::tool_hotkeys_get,
            commands::tools::desktop::tool_hotkey_set,
            commands::tools::desktop::tool_autoclick_start,
            commands::tools::desktop::tool_autoclick_stop,
            commands::tools::desktop::tool_autoclick_state,
            commands::tools::desktop::tool_autoclick_mouse,
            commands::tools::desktop::tool_dictation_devices,
            commands::tools::desktop::tool_dictation_options,
            commands::tools::desktop::tool_dictation_set_options,
            commands::tools::desktop::tool_dictation_state,
            commands::tools::desktop::tool_dictation_start,
            commands::tools::desktop::tool_dictation_stop,
            commands::tools::desktop::tool_record_sources,
            commands::tools::desktop::tool_record_state,
            commands::tools::desktop::tool_record_start,
            commands::tools::desktop::tool_record_stop,
            commands::tools::desktop::tool_record_save_replay,
            commands::tools::desktop::tool_vs_status,
            commands::tools::desktop::tool_vs_launch,
            commands::tools::desktop::tool_vs_clone,
            commands::tools::desktop::tool_vs_design,
            commands::tools::desktop::tool_vs_isolate,
            commands::tools::pdf::tool_pdf_status,
            commands::tools::pdf::tool_pdf_info,
            commands::tools::pdf::tool_pdf_merge,
            commands::tools::pdf::tool_pdf_split,
            commands::tools::pdf::tool_pdf_render,
            commands::tools::pdf::tool_pdf_text,
            commands::tools::pdf::tool_pdf_from_images,
            commands::tools::pdf::tool_pdf_compress,
            commands::tools::pdf::tool_pdf_sanitize,
            commands::tools::pdf::tool_pdf_ocr,
            commands::tools::pdf::tool_pdf_office,
            commands::tools::system::tool_win_tweaks_status,
            commands::tools::system::tool_win_tweak_apply,
            commands::tools::system::tool_clean_scan,
            commands::tools::system::tool_clean_run,
            commands::tools::system::tool_disk_volumes,
            commands::tools::system::tool_disk_scan,
            commands::tools::system::tool_disk_trash,
            commands::tools::system::tool_startup_list,
            commands::tools::system::tool_startup_set,
            commands::tools::system::tool_uninstall_list,
            commands::tools::system::tool_uninstall_leftovers,
            commands::tools::system::tool_uninstall_run,
            commands::tools::system::tool_debloat_list,
            commands::tools::system::tool_debloat_remove,
            commands::tools::system::tool_debloat_restore,
            commands::tools::system::tool_registry_scan,
            commands::tools::system::tool_registry_fix,
            commands::tools::system::tool_registry_backups_dir,
            commands::tools::system::tool_updater_status,
            commands::tools::system::tool_updater_upgrade,
            commands::tools::youtube::tool_sponsorblock,
            commands::tools::youtube::tool_ryd,
            commands::tools::youtube::tool_yt_video_id,
            commands::tools::youtube::tool_save_url,
            commands::tools::x::tool_x_session,
            commands::tools::x::tool_x_query_ids_refresh,
            commands::tools::x::tool_x_cancel,
            commands::tools::x::tool_x_post,
            commands::tools::x::tool_x_thread,
            commands::tools::x::tool_x_export_posts,
            commands::tools::x::tool_x_export_users,
            commands::tools::x::tool_x_render_posts,
            commands::tools::x::tool_x_profile,
            commands::tools::x::tool_x_profile_lookup,
            commands::tools::x::tool_x_media,
            commands::tools::x::tool_x_media_posts,
            commands::tools::x::tool_x_search,
            commands::tools::x::tool_x_trends,
            commands::tools::x::tool_x_bookmarks_export,
            commands::tools::x::tool_x_follows_audit,
            commands::tools::x::tool_x_unfollow,
            commands::tools::x::tool_x_whitelist_get,
            commands::tools::x::tool_x_whitelist_set,
            commands::tools::x::tool_x_archive_open,
            commands::tools::x::tool_x_archive_export,
            commands::tools::x::tool_x_grok_config,
            commands::tools::x::tool_x_grok_config_set,
            commands::tools::x::tool_x_grok_ask,
            commands::tools::x::tool_x_data_url,
            commands::tools::x::tool_x_save_data_url,
            commands::tools::x::tool_x_write_text,
            commands::tools::pinterest::tool_pin_inspect,
            commands::tools::pinterest::tool_pin_list,
            commands::tools::pinterest::tool_pin_related,
            commands::tools::pinterest::tool_pin_boards_search,
            commands::tools::pinterest::tool_pin_download,
            commands::tools::pinterest::tool_pin_download_many,
            commands::tools::pinterest::tool_pin_backup,
            commands::tools::pinterest::tool_pin_dupes,
            commands::tools::pinterest::tool_pin_unsave,
            commands::tools::pinterest::tool_pin_palette,
            commands::tools::pinterest::tool_pin_export,
            commands::tools::pinterest::tool_pin_keywords,
            commands::tools::pinterest::tool_pin_source,
            commands::tools::pinterest::tool_pin_expand,
            commands::tools::instagram::tool_ig_accounts,
            commands::tools::instagram::tool_ig_whoami,
            commands::tools::instagram::tool_ig_parse,
            commands::tools::instagram::tool_ig_cancel,
            commands::tools::instagram::tool_ig_post,
            commands::tools::instagram::tool_ig_resolve,
            commands::tools::instagram::tool_ig_download,
            commands::tools::instagram::tool_ig_download_bulk,
            commands::tools::instagram::tool_ig_profile,
            commands::tools::instagram::tool_ig_friendship,
            commands::tools::instagram::tool_ig_profile_media,
            commands::tools::instagram::tool_ig_stories,
            commands::tools::instagram::tool_ig_stories_tray,
            commands::tools::instagram::tool_ig_highlights,
            commands::tools::instagram::tool_ig_highlight_items,
            commands::tools::instagram::tool_ig_story_viewers,
            commands::tools::instagram::tool_ig_follow_lists,
            commands::tools::instagram::tool_ig_whitelist_get,
            commands::tools::instagram::tool_ig_whitelist_set,
            commands::tools::instagram::tool_ig_actions_today,
            commands::tools::instagram::tool_ig_actions,
            commands::tools::instagram::tool_ig_resolve_users,
            commands::tools::instagram::tool_ig_snapshot_take,
            commands::tools::instagram::tool_ig_snapshots,
            commands::tools::instagram::tool_ig_snapshot_diff,
            commands::tools::instagram::tool_ig_snapshot_delete,
            commands::tools::instagram::tool_ig_ghosts,
            commands::tools::instagram::tool_ig_export,
            commands::tools::instagram::tool_ig_write_csv,
            commands::tools::instagram::tool_ig_read_text,
            commands::tools::instagram::tool_ig_analytics,
            commands::tools::instagram::tool_ig_hashtag,
            commands::tools::instagram::tool_ig_comments,
            commands::tools::instagram::tool_ig_likers,
            commands::tools::instagram::tool_ig_giveaway,
            commands::tools::instagram::tool_ig_publish,
            commands::tools::instagram::tool_ig_publish_graph,
            commands::tools::instagram::tool_ig_schedule_list,
            commands::tools::instagram::tool_ig_schedule_add,
            commands::tools::instagram::tool_ig_schedule_remove,
            commands::dependencies::check_ytdlp_available,
            commands::dependencies::install_dependency,
            commands::dependencies::dependency_archived_versions,
            commands::diagnostics::flight_recorder_dump,
            commands::diagnostics::flight_recorder_clear,
            commands::diagnostics::preflight_batch,
            commands::dependencies::rollback_dependency,
            commands::dependencies::clear_dependency_path,
            commands::dependencies::dependency_custom_path,
            commands::rules::list_rules,
            commands::rules::save_rules,
            commands::rules::preview_rule_match,
            commands::media_history::check_media_changed,
            commands::media_history::record_media_snapshot,
            commands::dedupe::deduplicate_files,
            commands::dedupe::content_store_stats,
            commands::smart_speed::compute_silence_map,
            commands::smart_speed::silence_skip_target,
            commands::smart_speed::forget_silence_map,
            commands::torrent_playback::torrent_playback_readiness,
            commands::dependencies::dependency_variants,
            commands::dependencies::dependency_install_dir,
            commands::dependencies::set_dependency_path,
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
            commands::app_lifecycle::force_exit_app,
            commands::app_lifecycle::get_debug_info,
            commands::app_lifecycle::get_portable_info,
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app_handle, event| {
            if let tauri::RunEvent::ExitRequested { .. } = &event {
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

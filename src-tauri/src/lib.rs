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
mod meetings_capture;
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
pub mod world_bench;

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

/// Whether this start shows no window (3.2): `--background`, "start in
/// tray only", or a start at login with agents in the background.
fn launched_hidden(app: &tauri::AppHandle) -> bool {
    let s = storage::config::load_settings(app);
    let args: Vec<String> = std::env::args().collect();
    omniget_core::core::app_launch::starts_hidden(&args, s.start_minimized, s.run_in_background)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // Console log plus the ring buffer Report a Problem reads.
    canvas::diagnostics::init_tracing();

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
    // Set by `main.rs` when a debug build moved itself off the real profile.
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
        profile: Arc::new(profile::ProfileManager::new()),
        llm: Arc::new(llm_manager::LlmManager::new()),
    };

    // A second profile (`OMNIGET_DATA_DIR` set by hand or by the debug-build dev
    // profile in `main.rs`, not by portable mode) is its own app: own settings,
    // own bridge port, no single-instance lock.
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
                // A CLI that starts the app in the background while it is
                // already running must not pop its window up (3.2).
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
            // The login item says it is one, so a start at login can stay
            // hidden when the person runs agents in the background (3.2).
            Some(vec![omniget_core::core::app_launch::AUTOSTART_FLAG]),
        ))
        .setup(|app| {
            crate::agentkit_boot::spawn_ensure_shim();
            // Terminals (Central and canvas): install the shared manager early
            // so `term://` events flow before the first terminal command.
            crate::pty::manager().bind(app.handle());
            crate::canvas::chat::bind(app.handle());
            crate::portal::bind(app.handle());
            crate::canvas::routines::start(app.handle());
            crate::canvas::companion::start(app.handle());
            crate::canvas::power::start(app.handle());
            // Spotlight: the canvas mirrored as .webloc items (macOS).
            crate::canvas::spotlight::start();
            // Acesso remoto (opt-in): segundo listener só com /remote/*; nada roda desligado.
            tauri::async_runtime::spawn(crate::local_bridge_remote::boot(app.handle().clone()));
            // Provider keys (and anything else the core stores) go through the
            // app's store — keychain where the policy allows it — instead of
            // the core's file default. Must run before any secret is read.
            omniget_core::core::secrets::install(std::sync::Arc::new(secrets::AppSecretStore));
            // The /llm audit log (2.8) is open before any job or question.
            crate::llm_audit::boot(app.handle());
            // The 3D city: MCP edits reach the window, a saved council resumes.
            commands::city3d::city::install(app.handle());
            // The budget policies (3.3) too: a paused scope starts nothing.
            crate::llm_budget::boot(app.handle());
            // `.omniget/workflow.md` (3.10): last good versions and trust.
            crate::job_workflow::boot();

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
                let webview_dir = core::portable::portable_webview_dir_from_env();
                // The debug-build dev profile (`main.rs`) keeps the WebView's
                // storage apart from the installed app's too; the other windows
                // get it from `core::portable::with_dev_webview_dir`. macOS
                // needs nothing: an unbundled dev binary already gets its own
                // WebKit folder, named after the executable.
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

                // Started hidden (3.2): `--background` from the CLI, or a
                // start at login with agents in the background. The window
                // is born invisible instead of flashing and hiding.
                let builder = builder.visible(!launched_hidden(app.handle()));
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

            // Modo bench do mundo (`OMNIGET_WORLD_BENCH=<cenario>`): mesmo molde
            // do smoke; abre `/world?bench=`, recebe o JSON e sai. Ver `world_bench.rs`.
            world_bench::maybe_run(app);

            // The pet survives restarts when the user left it on.
            if commands::pet::load_prefs().enabled {
                let handle = app.handle().clone();
                tauri::async_runtime::spawn(async move {
                    if let Err(error) = commands::pet::pet_open(handle).await {
                        tracing::warn!("pet window did not reopen: {error}");
                    }
                });
            }

            // So does the limits strip; off by default, and then this is a no-op.
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
            // The one owner of settings.json: opened (and its profile lock
            // taken) before anything reads or writes the settings.
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
                    // B33: caixa-preta. Toda linha de log de download passa por
                    // aqui, e o `record` redige antes de guardar — e o unico
                    // ponto onde da para capturar o historico sem instrumentar
                    // cada chamada uma por uma.
                    let line = core::flight_recorder::redact(line);
                    let should_emit = core::download_log::push_line(id, &line);
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
            std::thread::spawn(omniget_core::core::meetings::recover_interrupted);
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
                        // Sign in with OpenRouter (PKCE) comes back on the same
                        // scheme; it is a key exchange, not a media URL.
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
                        // Spotlight results and canvas links: open the node.
                        if crate::canvas::spotlight::open_link(&handle, &raw) {
                            continue;
                        }
                        // links in notes exported by /video: open the player there
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
            usage_tray::setup(app.handle());
            hotkey::register_from_settings(app.handle());
            // From here on, confirmed settings changes reach the running app
            // through the effects registry (after the boot code above
            // applied the loaded settings).
            storage::settings_effects::install(app.handle().clone());

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

            let hidden = launched_hidden(app.handle());
            if hidden {
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.hide();
                }
                // Nobody saw this start: the audit log says who asked for it.
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
            // Where this app lives, for the CLI that starts it when it is not
            // running (3.2).
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
                // Agents in the background (3.2) keep the app in the tray:
                // closing the window must not stop their jobs.
                if s.exit_on_close && !s.run_in_background {
                    tray::request_quit(window.app_handle());
                } else {
                    let _ = window.hide();
                }
            }
            _ => {}
        })
        .invoke_handler(ipc_guard::guard(tauri::generate_handler![
            commands::video::video_status,
            commands::video::video_roots_add,
            commands::video::video_roots_remove,
            commands::video::video_scan,
            commands::video::video_scan_cancel,
            commands::video::video_items,
            commands::video::video_item,
            commands::video::video_home,
            commands::video::video_shows,
            commands::video::video_show,
            commands::video::video_courses,
            commands::video::video_course,
            commands::video::video_course_import,
            commands::video::video_course_remove,
            commands::video::video_play,
            commands::video::video_play_fallback,
            commands::video::video_progress,
            commands::video::video_set_watched,
            commands::video::video_continue_remove,
            commands::video::video_history,
            commands::video::video_history_clear,
            commands::video::video_prefs_get,
            commands::video::video_prefs_set,
            commands::video::video_open_external,
            commands::video::video_reveal,
            commands::video::link::video_link_resolve,
            commands::video::notes::video_notes,
            commands::video::notes::video_note_add,
            commands::video::notes::video_note_update,
            commands::video::notes::video_note_delete,
            commands::video::notes::video_notes_export,
            commands::video::study::video_transcript,
            commands::video::study::video_transcribe,
            commands::video::study::video_transcript_search,
            commands::video::study::video_silences,
            commands::video::study::video_ask,
            commands::video::study::video_cards,
            commands::video::study::video_cards_due,
            commands::video::study::video_card_add,
            commands::video::study::video_card_review,
            commands::video::study::video_card_delete,
            commands::video::study::video_anki_export,
            commands::video::sources::video_sources,
            commands::video::sources::video_source_remove,
            commands::video::sources::video_source_toggle,
            commands::video::jellyfin::video_jf_connect,
            commands::video::jellyfin::video_jf_quickconnect_start,
            commands::video::jellyfin::video_jf_quickconnect_poll,
            commands::video::jellyfin::video_jf_views,
            commands::video::jellyfin::video_jf_items,
            commands::video::jellyfin::video_jf_resume,
            commands::video::jellyfin::video_jf_nextup,
            commands::video::jellyfin::video_jf_item,
            commands::video::jellyfin::video_jf_children,
            commands::video::jellyfin::video_jf_play,
            commands::video::jellyfin::video_jf_report,
            commands::video::iptv::video_iptv_add,
            commands::video::iptv::video_iptv_add_xtream,
            commands::video::iptv::video_iptv_refresh,
            commands::video::iptv::video_iptv_groups,
            commands::video::iptv::video_iptv_channels,
            commands::video::iptv::video_iptv_epg,
            commands::video::iptv::video_iptv_favorite,
            commands::video::iptv::video_iptv_play,
            commands::video::addons::video_addon_install,
            commands::video::addons::video_addons,
            commands::video::addons::video_addon_remove,
            commands::video::addons::video_addon_catalog,
            commands::video::addons::video_addon_meta,
            commands::video::addons::video_addon_streams,
            commands::video::addons::video_addon_play,
            commands::video::anilist::video_anilist_set_token,
            commands::video::anilist::video_anilist_viewer,
            commands::video::anilist::video_anilist_search,
            commands::video::anilist::video_anilist_media,
            commands::video::anilist::video_anilist_list,
            commands::video::anilist::video_anilist_progress,
            commands::video::anilist::video_aniskip,
            commands::video::cast::video_cast_discover,
            commands::video::cast::video_cast_start,
            commands::video::cast::video_cast_control,
            commands::video::cast::video_cast_stop,
            commands::video::cast::video_cast_status,
            commands::video::video_open_path,
            commands::video::video_source_for_key,
            commands::telegram::telegram_call,
            commands::telegram::telegram_net_status,
            commands::telegram::telegram_net_scan,
            commands::telegram::telegram_net_doctor,
            commands::telegram::telegram_net_add_links,
            commands::telegram::telegram_net_add_route,
            commands::telegram::telegram_net_remove_route,
            commands::telegram::telegram_net_keep,
            commands::telegram::telegram_net_remove_failed,
            commands::telegram::telegram_net_test,
            commands::telegram::telegram_net_set_mode,
            commands::telegram::telegram_net_set_opts,
            commands::telegram::telegram_net_add_subscription,
            commands::telegram::telegram_net_remove_subscription,
            commands::telegram::telegram_net_refresh_subscriptions,
            commands::telegram::telegram_net_mihomo_status,
            commands::telegram::telegram_net_mihomo_set,
            commands::telegram::telegram_net_mihomo_restart,
            commands::telegram::telegram_net_mihomo_remove_node,
            commands::tube::account::tube_status,
            commands::tube::account::tube_account,
            commands::tube::account::tube_login,
            commands::tube::account::tube_logout,
            commands::tube::streams::tube_streams,
            commands::tube::streams::tube_streams_fallback,
            commands::tube::browse::tube_home,
            commands::tube::browse::tube_search,
            commands::tube::browse::tube_suggest,
            commands::tube::browse::tube_continuation,
            commands::tube::browse::tube_watch,
            commands::tube::browse::tube_comments,
            commands::tube::browse::tube_channel,
            commands::tube::browse::tube_playlist,
            commands::tube::browse::tube_subscriptions_feed,
            commands::tube::browse::tube_library,
            commands::tube::browse::tube_history,
            commands::tube::browse::tube_shorts,
            commands::tube::browse::tube_action,
            commands::tube::browse::tube_save_playlists,
            commands::tube::browse::tube_playlist_create,
            commands::tube::music::tube_music_home,
            commands::tube::music::tube_music_explore,
            commands::tube::music::tube_music_browse,
            commands::tube::music::tube_music_search,
            commands::tube::music::tube_music_album,
            commands::tube::music::tube_music_artist,
            commands::tube::music::tube_music_playlist,
            commands::tube::music::tube_music_next,
            commands::tube::music::tube_music_lyrics,
            commands::tube::music::tube_music_related,
            commands::tube::music::tube_music_library,
            commands::tube::local::tube_subs_list,
            commands::tube::local::tube_subs_add,
            commands::tube::local::tube_subs_remove,
            commands::tube::local::tube_subs_update,
            commands::tube::local::tube_local_history,
            commands::tube::local::tube_local_history_clear,
            commands::tube::local::tube_local_history_remove,
            commands::tube::local::tube_progress,
            commands::tube::local::tube_resume_for,
            commands::tube::local::tube_import,
            commands::tube::local::tube_export_subs,
            commands::tube::extras::tube_sponsorblock,
            commands::tube::extras::tube_dearrow,
            commands::tube::extras::tube_ryd,
            commands::tube::download::tube_download,
            commands::tube::autodl::tube_rules_list,
            commands::tube::autodl::tube_rules_save,
            commands::tube::autodl::tube_rules_remove,
            commands::tube::autodl::tube_rules_preview,
            commands::tube::autodl::tube_storage_stats,
            commands::tube::autodl::tube_auto_check_now,
            commands::music::music_status,
            commands::music::music_roots_add,
            commands::music::music_roots_remove,
            commands::music::music_scan,
            commands::music::music_scan_cancel,
            commands::music::music_tracks,
            commands::music::music_tracks_by_ids,
            commands::music::music_import_paths,
            commands::music::music_remove_tracks,
            commands::music::music_record_play,
            commands::music::music_set_rating,
            commands::music::music_set_liked,
            commands::music::music_tags_read,
            commands::music::music_tags_write,
            commands::music::music_write_replaygain,
            commands::music::music_peaks,
            commands::music::music_analyze,
            commands::music::music_analysis_cancel,
            commands::music::music_spectrogram,
            commands::music::music_lyrics_get,
            commands::music::music_lyrics_search,
            commands::music::music_lyrics_save,
            commands::music::music_playlists,
            commands::music::music_playlist_tracks,
            commands::music::music_smart_preview,
            commands::music::music_playlist_create,
            commands::music::music_playlist_update,
            commands::music::music_playlist_delete,
            commands::music::music_playlist_set_items,
            commands::music::music_playlist_add,
            commands::music::music_playlist_import,
            commands::music::music_playlist_export,
            commands::music::music_export_m3u,
            commands::music::music_history,
            commands::music::music_history_clear,
            commands::music::music_stats,
            commands::music::music_duplicates,
            commands::music::music_rename_preview,
            commands::music::music_rename_apply,
            commands::music::music_mb_search,
            commands::music::music_convert,
            commands::music::music_convert_cancel,
            commands::music::music_convert_jobs,
            commands::music::music_convert_clear,
            commands::music::music_radio_search,
            commands::music::music_radio_tags,
            commands::music::music_radio_stations,
            commands::music::music_radio_save,
            commands::music::music_radio_remove,
            commands::music::music_radio_played,
            commands::music::music_radio_resolve,
            commands::global_dl::gd_status,
            commands::global_dl::gd_browser_start,
            commands::global_dl::gd_browser_stop,
            commands::global_dl::gd_browser_set_path,
            commands::global_dl::gd_tab,
            commands::global_dl::gd_sniffed,
            commands::global_dl::gd_sniffed_clear,
            commands::global_dl::gd_resolve,
            commands::global_dl::gd_download,
            commands::global_dl::gd_download_sniffed,
            commands::global_dl::gd_job_cancel,
            commands::global_dl::gd_jobs_clear,
            commands::global_dl::gd_connector_install_dev,
            commands::global_dl::gd_connector_remove,
            commands::global_dl::gd_connector_options,
            commands::global_dl::gd_connector_options_set,
            commands::global_dl::gd_view,
            commands::global_dl::gd_unview,
            commands::global_dl::gd_mouse,
            commands::global_dl::gd_key,
            commands::global_dl::gd_insert_text,
            commands::global_dl::gd_selection,
            commands::global_dl::gd_history,
            commands::global_dl::gd_nav_state,
            commands::global_dl::gd_go,
            commands::global_dl::gd_dialog,
            commands::global_dl::gd_set_files,
            commands::global_dl::gd_switch_mode,
            commands::global_dl::gd_show_window,
            commands::global_dl::gd_set_block_ads,
            commands::global_dl::gd_clear_data,
            commands::global_dl::gd_sources,
            commands::global_dl::gd_import,
            commands::washmybelly::washmybelly_capabilities,
            commands::washmybelly::washmybelly_call,
            commands::washmybelly::washmybelly_ui,
            commands::washmybelly::washmybelly_monitor_start,
            commands::washmybelly::washmybelly_monitor_stop,
            commands::washmybelly::washmybelly_approvals,
            commands::washmybelly::washmybelly_approval_answer,
            commands::read::convert::read_tools,
            commands::read::convert::read_tool_set_path,
            commands::read::convert::read_devices,
            commands::read::convert::read_convert_jobs,
            commands::read::convert::read_convert_clear,
            commands::read::convert::read_convert,
            commands::read::convert::read_merge_chapters,
            commands::read::devices::read_devices_detect,
            commands::read::devices::read_send_to_device,
            commands::read::devices::read_mail_config,
            commands::read::devices::read_send_email,
            commands::read::library::read_books,
            commands::read::library::read_book,
            commands::read::library::read_search,
            commands::read::library::read_update_book,
            commands::read::library::read_bulk_update,
            commands::read::library::read_remove_books,
            commands::read::library::read_save_progress,
            commands::read::library::read_log_session,
            commands::read::library::read_shelves,
            commands::read::library::read_shelf_save,
            commands::read::library::read_shelf_delete,
            commands::read::library::read_shelf_reorder,
            commands::read::library::read_shelf_add,
            commands::read::library::read_shelf_remove,
            commands::read::library::read_annotations,
            commands::read::library::read_annotation_save,
            commands::read::library::read_annotation_delete,
            commands::read::library::read_annotations_export,
            commands::read::library::read_notes_dir,
            commands::read::library::read_stats,
            commands::read::library::read_set_goal,
            commands::read::library::read_duplicates,
            commands::read::manga::read_series_open,
            commands::read::manga::read_series_follow,
            commands::read::manga::read_series_list,
            commands::read::manga::read_series_get,
            commands::read::manga::read_series_refresh,
            commands::read::manga::read_series_update,
            commands::read::manga::read_series_remove,
            commands::read::manga::read_chapters_mark,
            commands::read::manga::read_chapters_mark_previous,
            commands::read::manga::read_chapter_bookmark,
            commands::read::manga::read_chapter_progress,
            commands::read::manga::read_chapter_open,
            commands::read::manga::read_updates_feed,
            commands::read::manga::read_updates_check,
            commands::read::manga::read_update_settings,
            commands::read::manga::read_categories,
            commands::read::manga::read_category_save,
            commands::read::manga::read_category_delete,
            commands::read::manga::read_series_migrate,
            commands::read::manga::read_chapters_download,
            commands::read::manga::read_download_queue,
            commands::read::manga::read_download_cancel,
            commands::read::manga::read_chapters_delete_downloads,
            commands::read::manga::read_download_ahead,
            commands::read::read_status,
            commands::read::ai::read_ai,
            commands::read::library::read_organize,
            commands::read::convert::read_webnovel,
            commands::read::sources::read_suwayomi_extensions,
            commands::read::sources::read_suwayomi_extension,
            commands::read::sources::read_suwayomi_add_repo,
            commands::read::manga::read_tracker_status,
            commands::read::manga::read_tracker_login,
            commands::read::manga::read_tracker_search,
            commands::read::manga::read_tracker_link,
            commands::read::ai::read_translate_page,
            commands::read::read_prepare,
            commands::read::read_save_url,
            commands::read::library::read_vocab_add,
            commands::read::library::read_vocab_list,
            commands::read::library::read_vocab_delete,
            commands::read::library::read_vocab_export,
            commands::read::read_set_download_dir,
            commands::read::read_roots_add,
            commands::read::read_roots_remove,
            commands::read::read_scan,
            commands::read::read_scan_cancel,
            commands::read::read_import_paths,
            commands::read::read_set_cover,
            commands::read::read_comic_pages,
            commands::read::read_share_set,
            commands::read::sources::read_sources,
            commands::read::sources::read_browse,
            commands::read::sources::read_global_search,
            commands::read::sources::read_opds_open,
            commands::read::sources::read_opds_download_url,
            commands::read::sources::read_catalogs,
            commands::read::sources::read_catalog_add,
            commands::read::sources::read_catalog_remove,
            commands::read::sources::read_catalog_toggle,
            commands::read::sources::read_repo_install,
            commands::read::sources::read_source_test,
            commands::read::sources::read_series_details,
            commands::read::sources::read_chapter_pages,
            commands::read::sources::read_meta_lookup,
            commands::read::sync::read_kosync_config,
            commands::read::sync::read_kosync_push,
            commands::read::sync::read_kosync_pull,
            commands::read::sync::read_backup_export,
            commands::read::sync::read_backup_import,
            commands::read::sync::read_import_mihon,
            commands::league::league_status,
            commands::league::league_get,
            commands::league::league_set_positions,
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
            commands::league::profile::league_friends,
            commands::league::profile::league_remove_friends,
            commands::league::profile::league_random_champion,
            commands::league::profile::league_declare_champion,
            commands::league::skins::league_roll_skin,
            commands::league::skins::league_roll_ward,
            commands::league::sgp::league_sgp_download_replay,
            commands::league::coach::league_coach_review,
            commands::league::coach::league_coach_trends,
            commands::league::coach::league_coach_ask,
            commands::league::coach::league_coach_ready,
            commands::bilibili_auth::bilibili_qr_generate,
            commands::bilibili_auth::bilibili_qr_poll,
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
            cookies::commands::cookies_import_file,
            cookies::commands::cookies_export_to,
            cookies::commands::cookies_add_account,
            cookies::commands::cookies_health,
            cookies::commands::cookies_test,
            commands::reencode::reencode_video,
            commands::diagnostics::diagnose_download_error,
            commands::downloads::detect_platform,
            commands::downloads::check_cookie_error,
            commands::downloads::validate_output_path,
            commands::downloads::get_media_formats,
            commands::stream_links::list_stream_links,
            commands::gallery_grab::gallery_grab,
            commands::gallery_grab::gallery_grab_cancel,
            commands::live_record::live_record_start,
            commands::live_record::live_record_stop,
            commands::manifest_tracks::manifest_tracks_list,
            commands::manifest_tracks::manifest_tracks_download,
            commands::manifest_tracks::manifest_tracks_cancel,
            commands::tv_library::tv_library_sync,
            commands::tv_library::tv_library_cancel,
            commands::post_text::save_post_text,
            commands::loudness_normalize::normalize_loudness,
            commands::quick_action::quick_action_status,
            commands::quick_action::quick_action_install,
            commands::quick_action::quick_action_remove,
            commands::size_check::check_download_size,
            commands::channel_picker::list_channel_videos,
            commands::proxy_check::check_proxy,
            commands::profile_sync::profile_sync_list,
            commands::profile_sync::profile_sync_add,
            commands::profile_sync::profile_sync_remove,
            commands::profile_sync::profile_sync_run,
            commands::link_export::export_link_list,
            commands::curl_import::curl_command_inspect,
            commands::curl_import::curl_command_download,
            commands::after_downloads::after_downloads_get,
            commands::after_downloads::after_downloads_set,
            commands::aria2_remote::aria2_remote_overview,
            commands::aria2_remote::aria2_remote_add,
            commands::aria2_remote::aria2_remote_task,
            commands::downloads::prefetch_media_info,
            commands::downloads::download_from_url,
            commands::link_router::route_links,
            commands::trim_preview::trim_strip,
            commands::trim_preview::trim_frame,
            commands::trim_preview::trim_export,
            commands::join_videos::join_check,
            commands::join_videos::join_videos,
            commands::page_files::scan_page_files,
            commands::downloads::playlist_entries,
            commands::downloads::torrent_contents,
            commands::channels::channels_list,
            commands::channels::channel_add,
            commands::channels::channel_remove,
            commands::channels::channel_update,
            commands::channels::channel_check_now,
            commands::channels::sync_channels_tray,
            commands::channels::sync_tray_strings,
            commands::ai::ai_get_config,
            commands::ai::ai_set_config,
            commands::ai::ai_test,
            commands::ai::ai_summarize_url,
            commands::ai::ai_history_list,
            commands::ai::ai_history_clear,
            commands::meetings::meetings_list,
            commands::meetings::meeting_get,
            commands::meetings::meeting_markdown,
            commands::meetings::meeting_update,
            commands::meetings::meeting_set_action_done,
            commands::meetings::meeting_delete,
            commands::meetings::meeting_import_file,
            commands::meetings::meeting_import_url,
            commands::meetings::meeting_retranscribe,
            commands::meetings::meeting_resummarize,
            commands::meetings::meeting_post_teams,
            commands::meetings::meeting_capture_support,
            commands::meetings::meeting_record_start,
            commands::meetings::meeting_record_status,
            commands::meetings::meeting_record_stop,
            commands::meetings::meeting_bot_send,
            commands::meetings::meeting_bot_refresh,
            commands::meetings::meeting_bot_stop,
            commands::meetings::meetings_settings_get,
            commands::meetings::meetings_settings_set,
            commands::meetings::meetings_whisper_status,
            commands::meetings::meetings_whisper_install,
            commands::meetings::meetings_whisper_model_download,
            commands::meetings::meetings_whisper_model_remove,
            commands::teams::teams_status,
            commands::teams::teams_configure,
            commands::teams::teams_sign_in_start,
            commands::teams::teams_sign_in_finish,
            commands::teams::teams_sign_out,
            commands::teams::teams_chats,
            commands::teams::teams_export,
            commands::teams::teams_calendar,
            commands::teams::teams_transcript_import,
            commands::video_ops::video_op_silence_estimate,
            commands::video_ops::video_op_preset,
            commands::video_ops::video_op_propose,
            commands::video_ops::video_op_run,
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
            commands::downloads::retry_download_with_command,
            commands::downloads::parse_batch_file,
            commands::downloads::get_recovery_items,
            commands::downloads::discard_recovery,
            commands::downloads::restore_recovery,
            commands::downloads::get_download_history,
            commands::downloads::get_download_queue,
            commands::downloads::clear_download_history,
            commands::downloads::reveal_file,
            commands::downloads::open_path_default,
            commands::integration::register_external_frontend,
            commands::settings::get_settings,
            commands::settings::update_settings,
            commands::settings::reset_settings,
            commands::settings::settings_get_v3,
            commands::settings::settings_patch_v3,
            commands::settings::settings_reset_v3,
            commands::settings::settings_retry_effect_v3,
            commands::settings::settings_import_appearance_v3,
            commands::settings::get_updater_proxy_url,
            commands::settings::mark_onboarding_complete,
            commands::settings::mark_legal_acknowledged,
            commands::rpc::rpc_test_connection,
            commands::rpc::rpc_set_idle_stats,
            commands::settings::get_bridge_info,
            commands::settings::rotate_bridge_token,
            commands::settings::bridge_open_pairing,
            commands::dependencies::check_dependencies,
            commands::llm::keys::tool_usage_report,
            commands::llm::keys::tool_keys_kinds,
            commands::llm::keys::tool_keys_list,
            commands::llm::keys::tool_keys_save,
            commands::llm::keys::tool_keys_test,
            commands::llm::keys::tool_mcp_status,
            commands::llm::keys::tool_mcp_clients,
            commands::llm::keys::tool_mcp_client_snippets,
            commands::llm::keys::tool_mcp_client_create,
            connect::connect_pending,
            connect::connect_answer,
            connect::connect_tools,
            connect::connect_tool,
            connect::connect_install_skill,
            connect::connect_prompt,
            commands::llm::keys::tool_mcp_client_revoke,
            commands::llm::keys::tool_mcp_root_grant,
            commands::llm::keys::tool_mcp_root_revoke,
            commands::llm::keys::tool_mcp_executors,
            commands::llm::keys::tool_mcp_execution_grant,
            commands::llm::keys::tool_mcp_network_grant,
            commands::llm::keys::tool_mcp_gateway_grant,
            commands::llm::keys::tool_mcp_media_configure,
            commands::llm::keys::tool_mcp_media_budget,
            commands::llm::keys::tool_mcp_set_enabled,
            commands::llm::keys::tool_mcp_selftest,
            mcp::auth::tool_mcp_auth_requests,
            mcp::auth::tool_mcp_auth_request_answer,
            commands::dependencies::check_ytdlp_available,
            commands::dependencies::install_dependency,
            commands::dependencies::dependency_archived_versions,
            commands::diagnostics::flight_recorder_dump,
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
            commands::dependencies::dependency_install_dir,
            commands::dependencies::set_dependency_path,
            commands::search::search_videos,
            commands::search_sites::search_sites,
            commands::p2p::p2p_send_file,
            commands::p2p::p2p_cancel_send,
            commands::p2p::p2p_pause_send,
            commands::p2p::p2p_resume_send,
            commands::app_lifecycle::force_exit_app,
            commands::app_lifecycle::get_debug_info,
            commands::app_lifecycle::get_portable_info,
            // Fase 1 (perfil local) — f1-perfil-core
            commands::profile::profile_get,
            commands::profile::profile_set_nickname,
            commands::profile::profile_set_skin,
            // Fase 6 (bench do mundo) — f6-bench
            commands::world_bench::world_bench_report,
            // Fase 2 (/llm) — f2-llm-commands, f2-wire-probe; Fase 5 (pet) — f5-pet-window
            // central:begin (costurado pelo orquestrador; não editar à mão)
            commands::central::agentkit::agentkit_targets,
            commands::central::agentkit::agentkit_detect,
            commands::central::agentkit::agentkit_parse_preview,
            commands::central::agentkit::agentkit_plan,
            commands::central::agentkit::agentkit_apply,
            commands::central::agentkit::agentkit_uninstall,
            commands::central::agentkit::agentkit_installed,
            commands::central::agentkit::agentkit_drift,
            commands::central::agentkit::agentkit_restore,
            commands::central::agentkit::agentkit_transactions,
            commands::central::agentkit::agentkit_import_installed,
            commands::central::threads::threads_snapshot,
            commands::central::threads::threads_events_after,
            commands::central::threads::threads_dispatch,
            commands::central::threads::threads_turns_page,
            commands::central::threads::threads_drivers,
            commands::central::vcs::vcs_status,
            commands::central::vcs::vcs_branches,
            commands::central::vcs::vcs_log,
            commands::central::vcs::vcs_worktree_create,
            commands::central::vcs::vcs_worktree_cancel,
            commands::central::vcs::vcs_worktree_rename_branch,
            commands::central::vcs::vcs_worktree_remove,
            commands::central::vcs::vcs_worktree_list,
            commands::central::vcs::vcs_worktree_cleanup,
            commands::central::vcs::vcs_checkpoint_capture,
            commands::central::vcs::vcs_checkpoint_list,
            commands::central::vcs::vcs_checkpoint_delete,
            commands::central::vcs::vcs_diff_turn,
            commands::central::vcs::vcs_diff_full,
            commands::central::vcs::vcs_restore_turn,
            commands::central::vcs::vcs_working_changes,
            commands::central::vcs::vcs_commit_suggest,
            commands::central::vcs::vcs_commit,
            commands::central::vcs::vcs_push,
            commands::central::vcs::vcs_host,
            commands::central::vcs::vcs_pr_create,
            commands::central::vcs::vcs_pr_view,
            commands::central::pty::pty_open,
            commands::central::pty::pty_write,
            commands::central::pty::pty_resize,
            commands::central::pty::pty_close,
            commands::central::pty::pty_list,
            commands::central::pty::pty_attach,
            commands::central::pty::pty_detach,
            commands::central::pty::pty_ack,
            commands::central::pty::pty_clear,
            commands::central::clitools::clitools_list,
            commands::central::clitools::clitools_detect,
            commands::central::clitools::clitools_latest,
            commands::central::clitools::clitools_plan,
            commands::central::clitools::clitools_run,
            commands::central::clitools::clitools_login,
            commands::central::clitools::clitools_acp_registry,
            commands::central::clitools::clitools_acp_install,
            commands::central::clitools::clitools_doctor,
            commands::central::catalog::catalog_meta,
            commands::central::catalog::catalog_search,
            commands::central::catalog::catalog_facets,
            commands::central::catalog::catalog_item,
            commands::central::catalog::catalog_item_files,
            commands::central::catalog::catalog_mcp_registry_search,
            commands::central::catalog::catalog_sources_list,
            commands::central::catalog::catalog_sources_add,
            commands::central::catalog::catalog_sources_rescan,
            commands::central::catalog::catalog_sources_remove,
            commands::central::catalog::catalog_marketplace,
            commands::central::catalog::catalog_marketplaces,
            commands::central::catalog::catalog_collections_list,
            commands::central::catalog::catalog_collections_create,
            commands::central::catalog::catalog_collections_rename,
            commands::central::catalog::catalog_collections_set_defaults,
            commands::central::catalog::catalog_collections_delete,
            commands::central::catalog::catalog_collections_add,
            commands::central::catalog::catalog_collections_remove,
            commands::central::catalog::catalog_collections_move,
            commands::central::catalog::catalog_collections_reorder,
            commands::central::catalog::catalog_collections_export,
            commands::central::catalog::catalog_collections_import,
            commands::central::catalog::catalog_install_index,
            commands::central::sessions::sessions_list,
            commands::central::sessions::sessions_get,
            commands::central::sessions::sessions_search,
            commands::central::sessions::sessions_search_in,
            commands::central::sessions::sessions_analysis,
            commands::central::sessions::sessions_usage,
            commands::central::sessions::sessions_heatmap,
            commands::central::sessions::sessions_agents,
            commands::central::sessions::sessions_team,
            commands::central::sessions::sessions_retro,
            commands::central::sessions::sessions_export,
            commands::central::sessions::sessions_import,
            commands::central::sessions::sessions_resume_command,
            commands::central::sessions::sessions_sources,
            commands::central::sessions::sessions_active,
            commands::central::sessions::sessions_refresh,
            commands::central::guard::guard_scan_component,
            commands::central::guard::guard_scan_path,
            commands::central::guard::guard_scan_paths,
            commands::central::guard::guard_scan_installed,
            commands::central::guard::guard_config_stats,
            commands::central::guard::guard_rules,
            local_bridge_observe::agentkit_observe_component,
            local_bridge_observe::agentkit_observe_revoke,
            local_bridge_observe::agentkit_observe_tools,
            commands::central::agentkit::agentkit_compat,
            commands::central::agentkit::agentkit_profiles,
            commands::central::agentkit::agentkit_profile_plan,
            commands::central::threads::threads_checkpoints,
            commands::central::threads::threads_diff,
            commands::central::threads::threads_revert_to_turn,
            commands::central::threads::threads_worktree_cancel,
            commands::central::threads::threads_git_status,
            commands::central::threads::threads_git_commit_message,
            commands::central::threads::threads_git_commit,
            commands::central::threads::threads_git_push,
            commands::central::threads::threads_git_pr_create,
            commands::central::threads::threads_git_pr,
            commands::central::threads::threads_git_pr_refresh,
            commands::central::threads::threads_files_search,
            commands::central::threads::threads_terminal_open,
            commands::central::threads::threads_terminal_close,
            commands::central::threads::threads_usage,
            commands::central::threads::threads_limits,
            commands::central::threads::threads_import_external,
            commands::central::threads::threads_resume_external,
            mcp::mcp_thread_get,
            mcp::mcp_thread_set,
            commands::central::agentkit::agentkit_plugins_list,
            commands::central::agentkit::agentkit_plugin_set_enabled,
            commands::central::remote::remote_status,
            commands::central::remote::remote_start,
            commands::central::remote::remote_stop,
            commands::central::remote::remote_pair_create,
            commands::central::remote::remote_pair_cancel,
            commands::central::remote::remote_devices,
            commands::central::remote::remote_device_revoke,
            commands::central::remote::remote_device_rename,
            commands::central::remote::remote_access_log,
            commands::central::remote::remote_access_log_clear,
            commands::central::remote::remote_tailscale_status,
            commands::central::remote::remote_tailscale_serve,
            commands::central::remote::remote_ssh_command,
            commands::central::preview::preview_discover_ports,
            commands::central::preview::preview_open,
            commands::central::preview::preview_set_bounds,
            commands::central::preview::preview_hide,
            commands::central::preview::preview_close,
            commands::central::preview::preview_navigate,
            commands::central::preview::preview_history,
            commands::central::preview::preview_state,
            commands::central::preview::preview_screenshot,
            commands::central::preview::preview_report,
            commands::central::preview::snapshot_list_windows,
            commands::central::preview::snapshot_capture,
            commands::central::preview::snapshot_request_permission,
            commands::central::clitools::clitools_config_files,
            commands::central::clitools::clitools_updates,
            commands::central::clitools::clitools_updates_cached,
            commands::central::arena::arena_create,
            commands::central::arena::arena_list,
            commands::central::arena::arena_get,
            commands::central::arena::arena_diff,
            commands::central::arena::arena_vote,
            commands::central::arena::arena_pick,
            commands::central::arena::arena_verify,
            commands::central::arena::arena_stop,
            commands::central::arena::arena_apply_preview,
            commands::central::arena::arena_apply,
            commands::central::arena::arena_cleanup,
            commands::central::arena::arena_delete,
            commands::central::arena::arena_scoreboard,
            commands::central::arena::arena_estimate,
            commands::central::arena::arena_compare,
            commands::central::run::run_runners,
            commands::central::run::run_loop_prepare,
            commands::central::run::run_loop_start,
            commands::central::run::run_agent_now,
            commands::central::run::run_playbook_prepare,
            commands::central::run::run_playbook_start,
            commands::central::run::run_playbooks_list,
            commands::central::run::run_playbook_cancel,
            commands::central::run::run_playbook_delete,
            commands::central::run::run_wizard_detect,
            commands::central::run::run_wizard_plan,
            commands::central::run::run_wizard_review,
            commands::central::run::run_sandbox_status,
            commands::central::run::run_sandbox_dockerfile,
            commands::central::run::run_sandbox_diff,
            commands::central::run::run_sandbox_apply,
            commands::central::run::run_sandbox_discard,
            // central:end
            commands::llm::roster::llm_roster_list,
            commands::llm::roster::llm_roster_create,
            commands::llm::roster::llm_roster_update,
            commands::llm::roster::llm_roster_delete,
            commands::llm::roster::llm_roster_apply_template,
            commands::llm::prompts::sync_llm_prompts,
            commands::llm::chat::llm_conversation_list,
            commands::llm::chat::llm_conversation_get,
            commands::llm::chat::llm_turn_start,
            commands::llm::help::help_turn_start,
            commands::llm::help::help_tool_call,
            commands::llm::chat::llm_turn_cancel,
            commands::llm::chat::llm_tool_answer,
            commands::llm::chat::llm_tool_asks_pending,
            approvals::llm_approvals_list,
            approvals::llm_approval_respond,
            commands::llm::chat::llm_workspace_set,
            commands::llm::chat::llm_workspace_get,
            commands::llm::chat::llm_turn_undo,
            native_agent::native_agent_register,
            native_agent::native_agent_reply,
            local_bridge_debug::debug_report,
            local_bridge_debug::debug_native_snapshot,
            commands::llm::skills::llm_skills_install_repo,
            commands::llm::roster::llm_acp_detect,
            commands::llm::roster::llm_acp_agent_create,
            commands::llm::jobs::llm_jobs_list,
            commands::llm::jobs::llm_job_get,
            commands::llm::jobs::llm_job_submit,
            commands::company::company_rpc,
            commands::company::company_run_stop,
            commands::company::company_engine_status,
            commands::company::company_runtime_templates,
            commands::company::company_hire,
            commands::llm::jobs::llm_job_cancel,
            commands::llm::jobs::llm_job_delete,
            commands::llm::jobs::llm_loops_list,
            commands::llm::jobs::llm_loop_create,
            commands::llm::jobs::llm_loop_cancel,
            commands::llm::jobs::llm_loop_delete,
            commands::llm::jobs::llm_triggers_list,
            commands::llm::jobs::llm_bridge_tokens,
            commands::llm::jobs::llm_bridge_token_mint,
            commands::llm::jobs::llm_bridge_token_revoke,
            commands::llm::jobs::llm_bridge_legacy_set,
            commands::llm::jobs::llm_trigger_save,
            commands::llm::jobs::llm_trigger_delete,
            commands::llm::jobs::llm_trigger_fire,
            commands::llm::chat::llm_permission_rules_get,
            commands::llm::chat::llm_permission_rules_set,
            commands::llm::chat::llm_conversation_run_mode,
            commands::llm::chat::llm_conversation_run_mode_set,
            llm_audit::llm_audit_list,
            llm_audit::llm_audit_retention_set,
            llm_budget::llm_budget_state,
            llm_budget::llm_budget_policy_save,
            llm_budget::llm_budget_policy_delete,
            llm_budget::llm_budget_incident_resolve,
            commands::llm::chat::llm_switch_model,
            commands::llm::models::llm_models_list,
            commands::llm::observatory::llm_telemetry_snapshot,
            commands::llm::prune::llm_prune_status,
            commands::llm::watchdog::llm_watchdog_get,
            commands::llm::watchdog::llm_watchdog_set,
            commands::llm::prune::llm_prune_set_config,
            commands::llm::prune::llm_prune_set_jev_key,
            commands::llm::wire_probe::llm_wire_probe_run,
            commands::llm::wire_probe::llm_wire_probe_last,
            commands::pet::pet_open,
            commands::pet::pet_close,
            commands::pet::pet_set_corner,
            commands::pet::pet_set_click_through,
            commands::pet::pet_capabilities,
            commands::pet::pet_set_ask_pending,
            limits_strip::commands::limits_strip_get_prefs,
            limits_strip::commands::limits_strip_set_prefs,
            limits_strip::commands::limits_strip_close,
            limits_strip::commands::limits_strip_state,
            limits_strip::commands::limits_strip_refresh,
            limits_strip::commands::limits_strip_set_expanded,
            usage_tray::usage_tray_view,
            usage_tray::usage_tray_toggle_account,
            usage_tray::usage_tray_set,
            usage_tray::usage_tray_refresh,
            usage_tray::usage_tray_open_claude,
            usage_tray::usage_tray_open_app,
            usage_tray::usage_tray_resize,
            usage_tray::usage_tray_open_panel,
            usage_tray::usage_tray_close_panel,
            usage_tray::usage_tray_sync_strings,
            usage_tray::usage_tray_sync_strings,
            commands::llm::keys::tool_ai_keys_openrouter_pkce,
            // Rodada 3: Fase 3 (MCP, skills) e Fase 4 (contas)
            commands::llm::mcp::llm_mcp_list,
            commands::llm::mcp::llm_mcp_upsert,
            commands::llm::mcp::llm_mcp_remove,
            commands::llm::mcp::llm_mcp_test,
            commands::llm::mcp::llm_mcp_tools,
            commands::llm::mcp::llm_mcp_grant,
            commands::llm::skills::llm_skills_list,
            commands::llm::skills::llm_skills_install_dir,
            commands::llm::skills::llm_skills_install_zip,
            commands::llm::skills::llm_skills_install_git,
            commands::llm::skills::llm_skills_remove,
            commands::llm::skills::llm_skills_catalog,
            commands::llm::skills::llm_skills_install_catalog,
            commands::llm::skills::llm_skills_confirm_install,
            commands::llm::skills::llm_skills_discard_install,
            commands::llm::skills::llm_skills_pending,
            commands::llm::skills::llm_skills_scanner,
            commands::llm::accounts::llm_accounts_list,
            commands::llm::accounts::llm_accounts_detect,
            commands::llm::accounts::llm_accounts_create,
            commands::llm::accounts::llm_accounts_remove,
            commands::llm::accounts::llm_accounts_login,
            commands::llm::accounts::llm_accounts_set_disabled,
            commands::llm::accounts::llm_cli_usage_report,
            commands::llm::accounts::llm_accounts_set_sandbox,
            commands::llm::accounts::llm_accounts_activate,
            commands::llm::accounts::llm_accounts_set_chain,
            commands::llm::accounts::llm_accounts_rotation,
            // Mundo: a cidade
            commands::world::city::city_join,
            commands::world::city::city_leave,
            commands::world::city::city_input,
            commands::world::city::city_interest,
            commands::world::city::city_chat,
            commands::city3d::generate::city3d_providers_status,
            commands::city3d::generate::city3d_tripo_key_set,
            commands::city3d::generate::city3d_tripo_balance,
            commands::city3d::generate::city3d_generate3d,
            commands::city3d::generate::city3d_autorig_configure,
            commands::city3d::generate::city3d_autorig_install_weights,
            commands::city3d::generate::city3d_autorig_install_cli,
            commands::city3d::generate::city3d_autorig,
            commands::city3d::avatar::city3d_avatar_catalog,
            commands::city3d::avatar::city3d_genome_random,
            commands::city3d::avatar::city3d_genome_from_traits,
            commands::city3d::avatar::city3d_avatar_get,
            commands::city3d::avatar::city3d_avatar_set,
            commands::city3d::avatar::city3d_npc_identities,
            commands::city3d::avatar::city3d_avatar_export_vrm,
            commands::city3d::codegeo::city3d_kit,
            commands::city3d::codegeo::city3d_kit_stats,
            commands::city3d::atlas::city3d_kit_atlas,
            commands::city3d::codegeo::city3d_codegeo_run,
            commands::city3d::codegeo::city3d_typologies,
            commands::city3d::codegeo::city3d_typology_save,
            commands::city3d::codegeo::city3d_typology_delete,
            commands::city3d::codegeo::city3d_codegeo_examples,
            commands::city3d::codegeo::city3d_codegeo_reference,
            commands::city3d::asset3d::city3d_import,
            commands::city3d::asset3d::city3d_import_bytes,
            commands::city3d::asset3d::city3d_assets_list,
            commands::city3d::asset3d::city3d_asset_glb,
            commands::city3d::asset3d::city3d_asset_thumb,
            commands::city3d::asset3d::city3d_asset_delete,
            commands::city3d::asset3d::city3d_asset_export,
            commands::city3d::asset3d::city3d_export_vox,
            commands::city3d::asset3d::city3d_export_write,
            commands::city3d::city::city3d_providers,
            commands::city3d::city::city3d_status,
            commands::city3d::city::city3d_found_example,
            commands::city3d::city::city3d_found_start,
            commands::city3d::city::city3d_found_cancel,
            commands::city3d::city::city3d_region,
            commands::city3d::city::city3d_region_mesh,
            commands::city3d::city::city3d_propose,
            commands::city3d::city::city3d_apply,
            commands::city3d::city::city3d_review,
            commands::city3d::city::city3d_preview,
            commands::city3d::city::city3d_log,
            commands::city3d::city::city3d_policy_set,
            commands::city3d::city::city3d_council_tick,
            commands::city3d::city::city3d_council_schedule,
            commands::city3d::city::city3d_budget_set,
            commands::city3d::city::city3d_works_tick,
            commands::city3d::city::city3d_server_connect,
            commands::city3d::city::city3d_server_disconnect,
            commands::city3d::city::city3d_server_status,
            commands::city3d::live::city3d_camera_list,
            commands::city3d::live::city3d_camera_start,
            commands::city3d::live::city3d_camera_stop,
            commands::city3d::live::city3d_camera_assets,
            commands::city3d::live::city3d_camera_asset,
            commands::world::city::city_resync,
            // canvas: stitched from handoffs (begin)
            commands::canvas::doc::canvas_workspaces,
            commands::canvas::doc::canvas_workspace_create,
            commands::canvas::doc::canvas_workspace_update,
            commands::canvas::doc::canvas_workspace_delete,
            commands::canvas::doc::canvas_load,
            commands::canvas::doc::canvas_apply,
            commands::canvas::doc::canvas_flush,
            commands::canvas::doc::canvas_note_read,
            commands::canvas::doc::canvas_note_write,
            commands::canvas::doc::canvas_note_list,
            commands::canvas::doc::canvas_note_move,
            commands::canvas::doc::canvas_layout_free_spot,
            commands::canvas::doc::canvas_layout_arrange,
            commands::canvas::doc::canvas_export_selection,
            commands::canvas::doc::canvas_import_partitura,
            commands::canvas::term::term_open,
            commands::canvas::term::canvas_term_ensure,
            commands::canvas::term::term_close,
            commands::canvas::term::term_list,
            commands::canvas::term::term_subscribe,
            commands::canvas::term::term_set_visible,
            commands::canvas::term::term_unsubscribe,
            commands::canvas::term::term_frame_done,
            commands::canvas::term::term_key,
            commands::canvas::term::term_text,
            commands::canvas::term::term_paste,
            commands::canvas::term::term_resize,
            commands::canvas::term::term_scroll,
            commands::canvas::term::term_mouse,
            commands::canvas::term::term_select,
            commands::canvas::term::term_copy,
            commands::canvas::term::term_search,
            commands::canvas::term::term_focus,
            commands::canvas::term::term_link,
            commands::canvas::term::term_screen,
            commands::canvas::term::term_clear,
            commands::canvas::term::term_scroll_lock,
            commands::canvas::term::term_search_count,
            commands::canvas::term::term_search_clear,
            commands::canvas::term::term_themes,
            commands::canvas::floors::canvas_floor_list,
            commands::canvas::floors::canvas_floor_pending,
            commands::canvas::floors::canvas_floor_activity,
            commands::canvas::floors::canvas_floor_pending_dismiss,
            commands::canvas::floors::canvas_floor_create,
            commands::canvas::floors::canvas_floor_land_preview,
            commands::canvas::floors::canvas_floor_land,
            commands::canvas::floors::canvas_floor_discard,
            commands::canvas::floors::canvas_floor_run_hook,
            commands::canvas::floors::canvas_floor_hooks_get,
            commands::canvas::floors::canvas_floor_hooks_set,
            commands::canvas::floors::canvas_floor_update,
            commands::canvas::floors::canvas_floor_fetch,
            commands::canvas::floors::canvas_floor_spawn_context,
            commands::canvas::floors::canvas_env_get,
            commands::canvas::floors::canvas_env_set,
            commands::canvas::floors::canvas_env_test,
            commands::canvas::floors::canvas_env_providers_get,
            commands::canvas::floors::canvas_env_providers_set,
            commands::canvas::floors::canvas_env_sessions,
            commands::canvas::floors::canvas_env_session_kill,
            commands::canvas::partituras::canvas_partitura_list,
            commands::canvas::partituras::canvas_partitura_get,
            commands::canvas::partituras::canvas_partitura_save,
            commands::canvas::partituras::canvas_partitura_edit,
            commands::canvas::partituras::canvas_partitura_delete,
            commands::canvas::partituras::canvas_partitura_preview,
            commands::canvas::partituras::canvas_partitura_import,
            commands::canvas::partituras::canvas_partitura_read_file,
            commands::canvas::partituras::canvas_partitura_add_to_library,
            commands::canvas::partituras::canvas_partitura_export_file,
            commands::canvas::routines::canvas_routine_list,
            commands::canvas::routines::canvas_routine_save,
            commands::canvas::routines::canvas_routine_set_enabled,
            commands::canvas::routines::canvas_routine_delete,
            commands::canvas::routines::canvas_routine_run_now,
            commands::canvas::routines::canvas_routine_history,
            commands::canvas::routines::canvas_routine_history_clear,
            commands::canvas::maestro::canvas_maestro_set,
            commands::canvas::files::canvas_files_list,
            commands::canvas::files::canvas_files_read,
            commands::canvas::files::canvas_file_node_preview,
            commands::canvas::files::canvas_files_write,
            commands::canvas::files::canvas_files_create,
            commands::canvas::files::canvas_files_rename,
            commands::canvas::files::canvas_files_delete,
            commands::canvas::files::canvas_files_search,
            commands::canvas::files::canvas_files_definition,
            commands::canvas::files::canvas_files_remote,
            commands::canvas::files::canvas_files_git_status,
            commands::canvas::files::canvas_files_git_diff,
            commands::canvas::files::canvas_files_git_op,
            commands::canvas::files::canvas_files_kind,
            commands::canvas::files::canvas_paths_probe,
            commands::canvas::files::canvas_image_data,
            commands::canvas::portal::canvas_portal_open,
            commands::canvas::portal::canvas_portal_close,
            commands::canvas::portal::canvas_portal_set_bounds,
            commands::canvas::portal::canvas_portal_set_visible,
            commands::canvas::portal::canvas_portal_navigate,
            commands::canvas::portal::canvas_portal_snapshot_image,
            commands::canvas::portal::canvas_portal_set_user_agent,
            commands::canvas::portal::canvas_portal_states,
            commands::canvas::portal::canvas_portal_report,
            commands::canvas::roles::canvas_roles_list,
            commands::canvas::roles::canvas_roles_get,
            commands::canvas::roles::canvas_roles_save,
            commands::canvas::roles::canvas_roles_delete,
            commands::canvas::roles::canvas_roles_assign,
            commands::canvas::roles::canvas_roles_launch,
            commands::canvas::roles::canvas_roles_discover,
            commands::canvas::roles::canvas_roles_import,
            commands::canvas::chat::canvas_chat_history,
            commands::canvas::chat::canvas_chat_send,
            commands::canvas::chat::canvas_chat_clear,
            commands::canvas::chat::canvas_attachment_save,
            commands::canvas::cords::canvas_terminal_env,
            commands::canvas::cords::canvas_cord_test,
            commands::canvas::cords::canvas_asks_list,
            commands::canvas::cords::canvas_inbox,
            commands::canvas::cords::canvas_mark_seen,
            commands::canvas::companion::canvas_companion_settings_get,
            commands::canvas::companion::canvas_companion_settings_set,
            commands::canvas::companion::canvas_companion_history,
            commands::canvas::companion::canvas_companion_dismiss,
            commands::canvas::companion::canvas_companion_act,
            commands::canvas::companion::canvas_companion_summarize,
            commands::canvas::companion::canvas_companion_models,
            commands::canvas::remote::canvas_remote_status,
            commands::canvas::remote::canvas_remote_pair_code,
            commands::canvas::remote::canvas_visit_status,
            commands::canvas::remote::canvas_visit_open,
            commands::canvas::remote::canvas_visit_close,
            commands::canvas::remote::canvas_visit_share,
            commands::canvas::remote::canvas_visit_join,
            commands::canvas::remote::canvas_visit_leave,
            commands::canvas::remote::canvas_visit_chat,
            commands::canvas::remote::canvas_visit_react,
            commands::canvas::remote::canvas_visit_refresh,
            // parity-gaps
            commands::canvas::workspace_io::canvas_workspace_export,
            commands::canvas::workspace_io::canvas_workspace_import_preview,
            commands::canvas::workspace_io::canvas_workspace_import,
            commands::canvas::workspace_io::canvas_workspace_backups,
            commands::canvas::workspace_io::canvas_workspace_backup_now,
            commands::canvas::workspace_io::canvas_workspace_backup_diff,
            commands::canvas::workspace_io::canvas_workspace_backup_restore,
            commands::canvas::clipboard::canvas_clipboard_copy,
            commands::canvas::clipboard::canvas_clipboard_paste,
            commands::canvas::clipboard::canvas_clipboard_read,
            commands::canvas::chat::canvas_chat_delete_entry,
            commands::canvas::chat::canvas_chat_wipe,
            commands::canvas::chat::canvas_chat_select_thread,
            commands::canvas::chat::canvas_chat_mark_read,
            commands::canvas::chat::canvas_chat_interrupt,
            commands::canvas::chat::canvas_chat_search,
            commands::canvas::portal::canvas_portal_design_arm,
            commands::canvas::portal::canvas_portal_design_disarm,
            commands::canvas::portal::canvas_portal_design_targets,
            commands::canvas::portal::canvas_portal_design_send,
            commands::canvas::portal::canvas_portal_trust_cert,
            commands::canvas::portal::canvas_portal_inspect,
            commands::canvas::portal::canvas_portal_copy_screenshot,
            commands::canvas::companion::canvas_companion_ask,
            commands::canvas::companion::canvas_omni_window,
            commands::canvas::cords::canvas_cord_link,
            canvas::prefs::canvas_settings_get,
            canvas::prefs::canvas_settings_replace,
            canvas::prefs::canvas_agent_presets,
            commands::canvas::workspace_meta::canvas_workspace_icon,
            commands::canvas::workspace_meta::canvas_workspace_agent_files,
            commands::canvas::workspace_meta::canvas_workspace_open_ide,
            commands::canvas::workspace_meta::canvas_ides,
            commands::canvas::workspace_meta::canvas_session_boot,
            commands::canvas::workspace_meta::canvas_session_mark,
            commands::canvas::workspace_meta::canvas_data_dir_move,
            commands::canvas::floors::canvas_env_ssh_hosts,
            commands::canvas::floors::canvas_env_files,
            commands::canvas::floors::canvas_env_drop,
            commands::canvas::floors::canvas_skills_info,
            commands::central::remote::remote_private_only_set,
            canvas::diagnostics::canvas_report_problem,
            // canvas: stitched from handoffs (end)
            commands::world::city::city_api,
            commands::world::city::city_asset,
            commands::world::city::city_session,
            commands::world::city::city_login,
            commands::world::city::city_register,
            commands::world::city::city_logout,
            // assist:begin
            commands::assist::memory::assist_memory_list,
            commands::assist::memory::assist_memory_scopes,
            commands::assist::memory::assist_memory_history,
            commands::assist::memory::assist_memory_create,
            commands::assist::memory::assist_memory_correct,
            commands::assist::memory::assist_memory_confirm,
            commands::assist::memory::assist_memory_retract,
            commands::assist::memory::assist_memory_forget,
            commands::assist::memory::assist_memory_export,
            commands::assist::memory::assist_memory_import,
            commands::assist::memory::assist_memory_backups,
            commands::assist::memory::assist_memory_delete_backups,
            commands::assist::memory::assist_memory_reindex,
            commands::assist::memory::assist_memory_stats,
            commands::assist::memory::assist_memory_entities,
            commands::assist::memory::assist_memory_search,
            commands::assist::memory::assist_memory_set_settings,
            commands::assist::memory::assist_memory_prepare_semantic,
            commands::assist::memory::assist_memory_retain_text,
            commands::assist::reading::assist_reading_overview,
            commands::assist::reading::assist_reading_list_all,
            commands::assist::reading::assist_reading_start_journey,
            commands::assist::reading::assist_reading_update_journey,
            commands::assist::reading::assist_reading_delete_journey,
            commands::assist::reading::assist_reading_record_progress,
            commands::assist::reading::assist_reading_note_context,
            commands::assist::reading::assist_reading_record_viewing,
            commands::assist::reading::assist_reading_forget_reaction,
            commands::assist::reading::assist_reading_set_prefs,
            commands::assist::reading::assist_reading_install_skill,
            commands::assist::bots::assist_bot_create,
            commands::assist::bots::assist_bot_get,
            commands::assist::bots::assist_bot_save_profile,
            commands::assist::bots::assist_bot_set_connection,
            commands::assist::bots::assist_bot_bind_skill,
            commands::assist::bots::assist_bot_unbind_skill,
            commands::assist::bots::assist_bot_skill_grants,
            commands::assist::bots::assist_bot_capabilities,
            commands::assist::bots::assist_bot_skill_reads,
            commands::llm::skills::llm_skills_status,
            commands::llm::skills::llm_skills_reproject,
            commands::llm::skills::llm_skills_accept,
            commands::llm::skills::llm_skills_reinstall,
            commands::assist::runs::assist_runs_list,
            commands::assist::runs::assist_run_get,
            commands::assist::runs::assist_run_diff,
            commands::assist::runs::assist_run_cancel,
            commands::assist::runs::assist_run_resolve,
            commands::llm::jobs::llm_job_resume,
            commands::llm::jobs::llm_job_mark_done,
            commands::llm::jobs::llm_job_discard,
            commands::llm::jobs::llm_loop_resume,
            commands::llm::jobs::llm_loop_settle,
            commands::llm::jobs::llm_trigger_mute,
            commands::llm::jobs::llm_job_seen,
            commands::llm::jobs::llm_jobs_state,
            commands::llm::jobs::llm_workflow_status,
            commands::llm::jobs::llm_workflow_trust,
            commands::llm::jobs::llm_jobs_limits_set,
            commands::llm::jobs::llm_job_diff,
            commands::llm::jobs::llm_job_remove_worktree,
            commands::llm::jobs::llm_job_hunks,
            commands::llm::jobs::llm_job_discard_hunk,
            commands::llm::jobs::llm_job_notes_list,
            commands::llm::jobs::llm_job_note_add,
            commands::llm::jobs::llm_job_note_delete,
            commands::llm::jobs::llm_job_notes_send,
            commands::llm::jobs::llm_job_integrate_preview,
            commands::llm::jobs::llm_job_integrate,
            commands::llm::jobs::llm_job_discard_worktree,
            commands::llm::jobs::llm_job_walkthrough,
            commands::llm::jobs::llm_ui_focus,
            commands::llm::jobs::sync_llm_notify_strings,
            commands::assist::groups::assist_group_list,
            commands::assist::groups::assist_group_create,
            commands::assist::groups::assist_group_update,
            commands::assist::groups::assist_group_delete,
            commands::assist::groups::assist_group_messages,
            commands::assist::groups::assist_group_send,
            commands::assist::groups::assist_group_cancel,
            commands::assist::groups::assist_group_shares,
            commands::assist::groups::assist_group_share,
            commands::assist::groups::assist_group_unshare,
            commands::assist::groups::assist_conversation_context,
            // assist:end
            // missions:start (autonomy: missions, learning, packs)
            commands::assist::missions::assist_mission_list,
            commands::assist::missions::assist_mission_get,
            commands::assist::missions::assist_mission_create,
            commands::assist::missions::assist_mission_from_chat,
            commands::assist::missions::assist_mission_start,
            commands::assist::missions::assist_mission_pause,
            commands::assist::missions::assist_mission_resume,
            commands::assist::missions::assist_mission_cancel,
            commands::assist::missions::assist_mission_delete,
            commands::assist::missions::assist_mission_note,
            commands::assist::missions::assist_mission_revise_criteria,
            commands::assist::missions::assist_mission_accept,
            commands::assist::missions::assist_mission_unblock_task,
            commands::assist::missions::assist_mission_allow_replay,
            commands::assist::missions::assist_mission_revise_budget,
            commands::assist::missions::assist_mission_verify,
            commands::assist::missions::assist_mission_diagnostics,
            commands::assist::missions::assist_mission_log,
            commands::assist::missions::assist_mission_presets,
            commands::assist::missions::assist_mission_apply_preset,
            commands::assist::missions::assist_mission_group_plan,
            commands::assist::missions::assist_mcp_derived_bot_revoke,
            commands::assist::learning::assist_learning_overview,
            commands::assist::learning::assist_learning_settings_save,
            commands::assist::learning::assist_learning_observe,
            commands::assist::learning::assist_learning_revoke,
            commands::assist::learning::assist_learning_propose,
            commands::assist::learning::assist_learning_candidate,
            commands::assist::learning::assist_learning_add_case,
            commands::assist::learning::assist_learning_delete_case,
            commands::assist::learning::assist_learning_evaluate,
            commands::assist::learning::assist_learning_eval_status,
            commands::assist::learning::assist_learning_promote,
            commands::assist::learning::assist_learning_rollback,
            commands::assist::learning::assist_learning_export,
            commands::assist::learning::assist_learning_forget,
            commands::assist::packs::assist_packs_catalog,
            commands::assist::packs::assist_packs_plan,
            commands::assist::packs::assist_packs_apply,
            commands::assist::packs::assist_packs_items,
            commands::assist::packs::assist_packs_text,
            commands::assist::packs::assist_packs_history,
            commands::assist::packs::assist_packs_rollback,
            commands::assist::packs::assist_packs_agent_to_bot,
            commands::llm::keys::tool_mcp_execution_grant_revoke,
            commands::llm::keys::tool_mcp_execution_grants_list,
            // missions:end
        ]))
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app_handle, event| {
            if let tauri::RunEvent::ExitRequested { .. } = &event {
                // Game mode never outlives OmniGet (Windows power plan,
                // priorities, screen kept awake).
                omniget_core::core::washmybelly::game::shutdown();
                // The optional mihomo core of /telegram dies with the app.
                omniget_core::core::telegram::net::mihomo::shutdown();
                // The managed llama-server dies with the app (phase 2).
                tauri::async_runtime::block_on(omniget_core::core::llm::local_servers::stop());
                // Central: terminal shells and the CLI driver sessions of the
                // threads die with the app too (bounded, never blocks exit).
                crate::pty::manager().shutdown_all();
                // Canvas: debounced document saves reach the disk before exit.
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

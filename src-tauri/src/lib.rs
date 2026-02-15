use std::collections::HashSet;
use std::sync::Arc;

use platforms::hotmart::auth::HotmartSession;

pub mod commands;
pub mod core;
pub mod models;
pub mod platforms;
pub mod storage;

pub struct AppState {
    pub hotmart_session: Arc<tokio::sync::Mutex<Option<HotmartSession>>>,
    pub active_downloads: Arc<tokio::sync::Mutex<HashSet<u64>>>,
}

#[tauri::command]
fn greet(name: &str) -> String {
    format!("Hello, {}! You've been greeted from Rust!", name)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tracing_subscriber::fmt::init();

    let state = AppState {
        hotmart_session: Arc::new(tokio::sync::Mutex::new(None)),
        active_downloads: Arc::new(tokio::sync::Mutex::new(HashSet::new())),
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
            commands::courses::hotmart_get_modules,
            commands::downloads::start_course_download,
            commands::downloads::get_active_downloads,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

//! Tauri side of the Global Downloader superpower. The work lives in
//! `omniget_core::core::global_dl`; this file forwards its events to the
//! window and plugs `ytdlp`/`dash` items into the Downloads queue.

use std::path::PathBuf;

use omniget_core::core::global_dl::{browser, connector, jobs, sniffer, tools, Node};
use serde_json::{json, Value};
use tauri::{AppHandle, Emitter};

/// Forwards core events as `global-dl:event` and wires the queue handoff.
pub fn start_background(app: AppHandle) {
    let mut rx = omniget_core::core::global_dl::events().subscribe();
    let emitter = app.clone();
    tauri::async_runtime::spawn(async move {
        loop {
            match rx.recv().await {
                Ok(ev) => {
                    let _ = emitter.emit("global-dl:event", ev);
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(_) => break,
            }
        }
    });
    jobs::set_handoff(Box::new(move |url| {
        let app = app.clone();
        tauri::async_runtime::spawn(async move {
            let _ = crate::external_url::handle_external_url(&app, url, "global-dl").await;
        });
    }));
}

#[tauri::command]
pub fn gd_status() -> Value {
    json!({
        "browser": browser::status(),
        "connectors": connector::list(),
        "jobs": jobs::list(),
        "out_dir": tools::default_out_dir(),
    })
}

#[tauri::command]
pub async fn gd_browser_start(url: Option<String>) -> Result<Value, String> {
    let b = browser::start(url, true).await?;
    Ok(json!({ "tabs": b.tabs() }))
}

#[tauri::command]
pub async fn gd_browser_stop() -> Result<(), String> {
    browser::stop().await
}

#[tauri::command]
pub fn gd_browser_set_path(path: Option<String>) -> Result<(), String> {
    browser::set_browser_path(path)
}

#[tauri::command]
pub async fn gd_tab(action: String, tab_id: Option<String>, url: Option<String>) -> Result<Value, String> {
    let b = browser::require()?;
    let need_tab = || tab_id.clone().ok_or("tab_id is required".to_string());
    match action.as_str() {
        "open" => {
            let id = b.open_tab(url.as_deref().unwrap_or("about:blank")).await?;
            return Ok(json!({ "id": id }));
        }
        "close" => b.close_tab(&need_tab()?).await?,
        "focus" => b.activate_tab(&need_tab()?).await?,
        "navigate" => b.navigate(&need_tab()?, url.as_deref().unwrap_or("about:blank")).await?,
        other => return Err(format!("unknown tab action {other}")),
    }
    Ok(json!({ "tabs": b.tabs() }))
}

#[tauri::command]
pub fn gd_sniffed(tab_id: Option<String>) -> Value {
    json!(sniffer::sniffed(tab_id.as_deref()))
}

#[tauri::command]
pub fn gd_sniffed_clear(tab_id: Option<String>) {
    sniffer::clear(tab_id.as_deref());
}

#[tauri::command]
pub async fn gd_resolve(url: String, tab_id: Option<String>) -> Result<Value, String> {
    let r = connector::resolve(&url, None, tab_id, None).await?;
    serde_json::to_value(r).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn gd_download(
    tree: Node,
    selected: Option<Vec<String>>,
    out_dir: Option<String>,
    max_height: Option<u32>,
) -> Result<Value, String> {
    let out = out_dir
        .filter(|d| !d.trim().is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(tools::default_out_dir);
    let job = jobs::start(tree, selected, out, max_height)?;
    serde_json::to_value(job).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn gd_download_sniffed(id: u64, out_dir: Option<String>) -> Result<Value, String> {
    let tree = tools::tree_for_sniffed(id)?;
    gd_download(tree, None, out_dir, None)
}

#[tauri::command]
pub fn gd_job_cancel(id: u64) -> bool {
    jobs::cancel(id)
}

#[tauri::command]
pub fn gd_jobs_clear() {
    jobs::clear_finished();
}

#[tauri::command]
pub fn gd_connector_install_dev(dir: String) -> Result<Value, String> {
    let m = connector::install_dev(&PathBuf::from(dir))?;
    serde_json::to_value(m).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn gd_connector_remove(id: String) -> Result<(), String> {
    connector::remove(&id)
}

#[tauri::command]
pub fn gd_connector_options(id: String) -> Result<Value, String> {
    connector::get_options(&id)
}

#[tauri::command]
pub fn gd_connector_options_set(id: String, values: Value) -> Result<Value, String> {
    connector::set_options(&id, &values)
}

// ── Embedded browser: view, input, navigation, settings ──

#[tauri::command]
pub async fn gd_view(tab_id: String, width: u32, height: u32, dpr: f64) -> Result<(), String> {
    browser::require()?.view(&tab_id, width, height, dpr).await
}

#[tauri::command]
pub async fn gd_unview() -> Result<(), String> {
    if let Some(b) = browser::current() {
        b.unview().await;
    }
    Ok(())
}

#[tauri::command]
pub async fn gd_mouse(tab_id: String, params: Value) -> Result<(), String> {
    browser::require()?.mouse(&tab_id, params).await
}

#[tauri::command]
pub async fn gd_key(tab_id: String, params: Value) -> Result<(), String> {
    browser::require()?.key(&tab_id, params).await
}

#[tauri::command]
pub async fn gd_insert_text(tab_id: String, text: String) -> Result<(), String> {
    browser::require()?.insert_text(&tab_id, &text).await
}

/// The page's selected text, for Cmd/Ctrl+C in the embedded view.
#[tauri::command]
pub async fn gd_selection(tab_id: String) -> Result<String, String> {
    let v = browser::require()?
        .eval(
            &tab_id,
            "(function(){var a=document.activeElement;if(a&&typeof a.selectionStart==='number'&&a.value!==undefined)return a.value.slice(a.selectionStart,a.selectionEnd);return String(getSelection()||'')})()",
            std::time::Duration::from_secs(5),
        )
        .await?;
    Ok(v.as_str().unwrap_or_default().to_string())
}

#[tauri::command]
pub async fn gd_history(tab_id: String, action: String) -> Result<(), String> {
    browser::require()?.history(&tab_id, &action).await
}

#[tauri::command]
pub async fn gd_nav_state(tab_id: String) -> Result<Value, String> {
    let (back, forward) = browser::require()?.can_go(&tab_id).await;
    Ok(json!({ "back": back, "forward": forward }))
}

/// Address bar: a URL, a bare host or search words, in `tab_id` or a new
/// (optionally private) tab.
#[tauri::command]
pub async fn gd_go(input: String, tab_id: Option<String>, new_tab: Option<bool>, private: Option<bool>) -> Result<Value, String> {
    let b = browser::start(None, true).await?;
    let url = browser::address_to_url(&input);
    let id = match (tab_id, new_tab.unwrap_or(false), private.unwrap_or(false)) {
        (_, _, true) => b.open_private_tab(&url).await?,
        (Some(t), false, false) => {
            b.navigate(&t, &url).await?;
            t
        }
        _ => b.open_tab(&url).await?,
    };
    Ok(json!({ "id": id, "url": url }))
}

#[tauri::command]
pub async fn gd_dialog(tab_id: String, accept: bool, text: Option<String>) -> Result<(), String> {
    browser::require()?.answer_dialog(&tab_id, accept, text).await
}

#[tauri::command]
pub async fn gd_set_files(tab_id: String, backend_node_id: i64, files: Vec<String>) -> Result<(), String> {
    browser::require()?.set_files(&tab_id, backend_node_id, files).await
}

#[tauri::command]
pub async fn gd_switch_mode(mode: String) -> Result<Value, String> {
    let b = browser::switch_mode(&mode).await?;
    Ok(json!({ "tabs": b.tabs() }))
}

#[tauri::command]
pub async fn gd_show_window(tab_id: String) -> Result<(), String> {
    browser::require()?.show_window(&tab_id).await
}

#[tauri::command]
pub async fn gd_set_block_ads(on: bool) -> Result<(), String> {
    browser::set_config("block_ads", json!(on))?;
    if let Some(b) = browser::current() {
        b.apply_blocking(on).await;
    }
    Ok(())
}

#[tauri::command]
pub async fn gd_clear_data() -> Result<(), String> {
    browser::require()?.clear_data().await
}

#[tauri::command]
pub fn gd_sources() -> Value {
    json!({
        "sources": omniget_core::core::global_dl::profiles::sources(),
        "imported": omniget_core::core::global_dl::profiles::imported(),
    })
}

#[tauri::command]
pub async fn gd_import(source: String, profile: Option<String>) -> Result<Value, String> {
    omniget_core::core::global_dl::profiles::import(&source, profile.as_deref().unwrap_or("")).await
}

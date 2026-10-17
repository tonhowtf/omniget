use std::path::PathBuf;
use std::time::Duration;

use omniget_core::core::global_dl::{
    browser, connector, jobs, page, profiles, sniffer, tools, Node,
};
use serde_json::{json, Value};
use tauri::{AppHandle, Emitter};

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
pub async fn gd_tab(
    action: String,
    tab_id: Option<String>,
    url: Option<String>,
) -> Result<Value, String> {
    let b = browser::require()?;
    let need_tab = || tab_id.clone().ok_or("tab_id is required".to_string());
    match action.as_str() {
        "open" => {
            let id = b.open_tab(url.as_deref().unwrap_or("about:blank")).await?;
            return Ok(json!({ "id": id }));
        }
        "close" => b.close_tab(&need_tab()?).await?,
        "focus" => b.activate_tab(&need_tab()?).await?,
        "navigate" => {
            b.navigate(&need_tab()?, url.as_deref().unwrap_or("about:blank"))
                .await?
        }
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
pub async fn gd_find(url: String, reload: Option<bool>) -> Result<Value, String> {
    let b = browser::start(None, true).await?;
    let tab = match b.tabs().into_iter().rev().find(|t| t.url == url) {
        Some(t) => {
            if reload == Some(true) {
                b.history(&t.id, "reload").await?;
                page::wait_loaded(&b, &t.id, Duration::from_secs(20)).await;
            }
            t.id
        }
        None => {
            let id = b.open_tab(&url).await?;
            page::wait_loaded(&b, &id, Duration::from_secs(20)).await;
            id
        }
    };
    let r = connector::resolve(&url, None, Some(tab), None).await?;
    serde_json::to_value(r).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn gd_browser_show(url: Option<String>) -> Result<Value, String> {
    if browser::mode() != "window" || browser::current().is_some_and(|b| b.headless) {
        browser::switch_mode("window").await?;
    }
    let b = browser::start(None, true).await?;
    let url = url.filter(|u| !u.trim().is_empty());
    let existing = b
        .tabs()
        .into_iter()
        .rev()
        .find(|t| url.as_deref().is_none_or(|u| t.url == u));
    let tab = match (existing, url) {
        (Some(t), _) => t.id,
        (None, Some(u)) => b.open_tab(&u).await?,
        (None, None) => b.open_tab("about:blank").await?,
    };
    b.show_window(&tab).await?;
    Ok(json!({ "tabs": b.tabs(), "mode": "window" }))
}

#[tauri::command]
pub async fn gd_browser_hide() -> Result<(), String> {
    if browser::mode() == "window" {
        browser::switch_mode("embedded").await?;
    }
    Ok(())
}

#[tauri::command]
pub fn gd_browser_sources() -> Value {
    json!({ "sources": profiles::sources(), "imported": profiles::imported() })
}

#[tauri::command]
pub async fn gd_browser_import(source: String, profile: Option<String>) -> Result<Value, String> {
    profiles::import(&source, profile.as_deref().unwrap_or_default()).await
}

#[tauri::command]
pub async fn gd_download(
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
pub async fn gd_download_sniffed(id: u64, out_dir: Option<String>) -> Result<Value, String> {
    let tree = tools::tree_for_sniffed(id)?;
    gd_download(tree, None, out_dir, None).await
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

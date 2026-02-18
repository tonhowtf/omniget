use std::sync::Arc;
use std::time::Duration;

use anyhow::anyhow;
use tauri::Manager;

pub async fn ensure_api_webview(
    app: &tauri::AppHandle,
    result_store: &Arc<std::sync::Mutex<Option<String>>>,
) -> anyhow::Result<tauri::WebviewWindow> {
    if let Some(existing) = app.get_webview_window("udemy-api") {
        return Ok(existing);
    }

    let store = result_store.clone();

    let window = tauri::WebviewWindowBuilder::new(
        app,
        "udemy-api",
        tauri::WebviewUrl::External("https://www.udemy.com/".parse().unwrap()),
    )
    .visible(false)
    .inner_size(100.0, 100.0)
    .initialization_script(
        r#"window.__omniget_fetch = function(url) {
            fetch(url, {
                credentials: 'include',
                headers: {
                    'Accept': 'application/json, text/plain, */*',
                    'X-Requested-With': 'XMLHttpRequest'
                }
            })
            .then(function(r) { return r.text() })
            .then(function(text) {
                window.location.href = 'https://omniget-api-result.local/?data=' + encodeURIComponent(text);
            })
            .catch(function(err) {
                window.location.href = 'https://omniget-api-result.local/?error=' + encodeURIComponent(err.message);
            });
        };"#,
    )
    .on_navigation(move |url| {
        if url.host_str() == Some("omniget-api-result.local") {
            for (key, value) in url.query_pairs() {
                if key == "data" {
                    *store.lock().unwrap() = Some(value.to_string());
                    return false;
                }
                if key == "error" {
                    *store.lock().unwrap() =
                        Some(format!("{{\"__fetch_error\":\"{}\"}}", value));
                    return false;
                }
            }
            return false;
        }
        true
    })
    .build()
    .map_err(|e| anyhow!("Failed to create API webview: {}", e))?;

    tokio::time::sleep(Duration::from_secs(3)).await;

    Ok(window)
}

pub async fn webview_get(
    window: &tauri::WebviewWindow,
    url: &str,
    result_store: &Arc<std::sync::Mutex<Option<String>>>,
) -> anyhow::Result<String> {
    *result_store.lock().unwrap() = None;

    let js = format!("window.__omniget_fetch('{}')", url);
    window
        .eval(&js)
        .map_err(|e| anyhow!("eval failed: {}", e))?;

    let start = std::time::Instant::now();
    let timeout = Duration::from_secs(30);
    loop {
        if let Some(data) = result_store.lock().unwrap().take() {
            if let Some(err_msg) = data.strip_prefix("{\"__fetch_error\":\"") {
                let err_msg = err_msg.trim_end_matches("\"}");
                return Err(anyhow!("Fetch error: {}", err_msg));
            }
            return Ok(data);
        }
        if start.elapsed() > timeout {
            return Err(anyhow!("Timeout waiting for API response"));
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

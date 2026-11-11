pub mod mcp;

use serde_json::Value;

use omniget_core::core::devtools::{http, pack, ports, scrape, util, DevError};

const BLOCKING_LIMIT: std::time::Duration = std::time::Duration::from_secs(30);

async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> Result<T, DevError> + Send + 'static,
) -> Result<T, String> {
    blocking_within(BLOCKING_LIMIT, work).await
}

async fn blocking_within<T: Send + 'static>(
    limit: std::time::Duration,
    work: impl FnOnce() -> Result<T, DevError> + Send + 'static,
) -> Result<T, String> {
    match tokio::time::timeout(limit, tokio::task::spawn_blocking(work)).await {
        Ok(done) => done.map_err(|e| e.to_string())?.map_err(|e| e.wire()),
        Err(_) => Err(format!(
            "TIMEOUT: {}",
            serde_json::json!({
                "message": format!("no answer after {} s", limit.as_secs()),
                "retryable": true,
                "next_step": "the system may be waiting for the person to allow OmniGet to read this folder (macOS: System Settings, Privacy & Security, Files and Folders); ask them to allow it, then retry",
            })
        )),
    }
}

#[tauri::command]
pub async fn devtools_pack_plan(request: pack::PackRequest) -> Result<pack::Plan, String> {
    blocking(move || pack::plan(&request)).await
}

#[tauri::command]
pub async fn devtools_pack_build(request: pack::PackRequest) -> Result<pack::Built, String> {
    blocking(move || pack::build(&request)).await
}

#[tauri::command]
pub async fn devtools_scrape_fetch(request: scrape::FetchRequest) -> Result<scrape::Page, String> {
    scrape::fetch(request).await.map_err(|e| e.wire())
}

#[tauri::command]
pub async fn devtools_scrape_select(
    request: scrape::SelectRequest,
) -> Result<scrape::Selection, String> {
    scrape::select_from(request).await.map_err(|e| e.wire())
}

#[tauri::command]
pub async fn devtools_http_send(request: http::HttpRequest) -> Result<http::HttpResponse, String> {
    http::send(request).await.map_err(|e| e.wire())
}

#[tauri::command]
pub async fn devtools_ports_list() -> Result<Vec<ports::Listener>, String> {
    blocking(ports::list).await
}

#[tauri::command]
pub async fn devtools_ports_kill(
    port: Option<u16>,
    pid: Option<u32>,
    confirm: bool,
) -> Result<ports::KillResult, String> {
    blocking(move || ports::kill(port, pid, confirm)).await
}

#[tauri::command]
pub async fn devtools_util(op: String, args: Value) -> Result<Value, String> {
    blocking(move || util::run(&op, &args)).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn blocking_work_that_never_answers_is_a_timeout_with_a_next_step() {
        let slow = || {
            std::thread::sleep(std::time::Duration::from_millis(300));
            Ok::<_, DevError>(1)
        };
        let error = blocking_within(std::time::Duration::from_millis(20), slow)
            .await
            .unwrap_err();
        let (code, detail) = error.split_once(": ").unwrap();
        let detail: Value = serde_json::from_str(detail).unwrap();
        assert_eq!(code, "TIMEOUT");
        assert_eq!(detail["retryable"], true);
        assert!(detail["next_step"].as_str().unwrap().contains("allow"));
    }

    #[tokio::test]
    async fn blocking_work_that_answers_in_time_is_returned() {
        let fast = || Ok::<_, DevError>(7);
        assert_eq!(
            blocking_within(std::time::Duration::from_secs(5), fast).await,
            Ok(7)
        );
    }
}

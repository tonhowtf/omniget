pub mod mcp;

use std::path::PathBuf;

use omniget_core::core::transfer::jobs::{self, Snapshot};
use omniget_core::core::transfer::listing::Listing;
use omniget_core::core::transfer::ops::{self, Managed};
use omniget_core::core::transfer::profiles::Draft;
use omniget_core::core::transfer::service::{self, AppSecrets, Resolved, ServerView, TargetInput};
use omniget_core::core::transfer::TransferError;

async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> Result<T, TransferError> + Send + 'static,
) -> Result<T, TransferError> {
    tokio::task::spawn_blocking(work)
        .await
        .map_err(|e| TransferError::Storage(e.to_string()))?
}

async fn resolved(target: TargetInput) -> Result<Resolved, TransferError> {
    blocking(move || service::resolve(&service::data_dir()?, &AppSecrets, &target)).await
}

pub async fn servers() -> Result<Vec<ServerView>, TransferError> {
    blocking(|| service::servers(&service::data_dir()?, &AppSecrets)).await
}

pub async fn save_server(
    draft: Draft,
    secret: Option<String>,
) -> Result<ServerView, TransferError> {
    blocking(move || {
        service::save_server(
            &service::data_dir()?,
            &AppSecrets,
            &draft,
            secret.as_deref(),
        )
    })
    .await
}

pub async fn remove_server(name: String) -> Result<String, TransferError> {
    blocking(move || service::remove_server(&service::data_dir()?, &AppSecrets, &name))
        .await
        .map(|removed| removed.name)
}

pub async fn forget_host(host: String, port: Option<u32>) -> Result<usize, TransferError> {
    blocking(move || service::forget_host(&service::data_dir()?, &host, port)).await
}

pub async fn list(
    target: TargetInput,
    path: Option<String>,
    limit: Option<usize>,
) -> Result<Listing, TransferError> {
    let resolved = resolved(target).await?;
    ops::list(&resolved.connection, resolved.path(path.as_deref()), limit).await
}

pub async fn start_get(
    target: TargetInput,
    path: String,
    save_to: String,
    overwrite: bool,
) -> Result<String, TransferError> {
    let resolved = resolved(target).await?;
    let save_to = PathBuf::from(save_to);
    jobs::start("download", &path, save_to.clone(), |report| {
        let remote = path.clone();
        async move {
            let totals = report.clone();
            ops::get(
                &resolved.connection,
                &remote,
                &save_to,
                overwrite,
                &move |total| totals.total(total),
                &move |bytes| report.bytes(bytes),
            )
            .await
        }
    })
}

pub async fn start_put(
    target: TargetInput,
    local_path: String,
    path: String,
    overwrite: bool,
) -> Result<String, TransferError> {
    let resolved = resolved(target).await?;
    let local = PathBuf::from(local_path);
    let remote = match resolved.start_path.as_deref() {
        Some(start) if path.is_empty() => format!("{start}/"),
        _ => path,
    };
    jobs::start("upload", &remote, local.clone(), |report| {
        let remote = remote.clone();
        async move {
            let totals = report.clone();
            ops::put(
                &resolved.connection,
                &local,
                &remote,
                overwrite,
                &move |total| totals.total(total),
                &move |bytes| report.bytes(bytes),
            )
            .await
        }
    })
}

pub async fn manage(
    target: TargetInput,
    op: &str,
    path: &str,
    to: Option<&str>,
    confirm: bool,
) -> Result<Managed, TransferError> {
    let run = |connection| async move {
        match op {
            "mkdir" => ops::mkdir(connection, path).await,
            "rename" => {
                let to = to.filter(|to| !to.is_empty()).ok_or_else(|| {
                    TransferError::InvalidInput("to is required for op=rename".into())
                })?;
                ops::rename(connection, path, to).await
            }
            "delete" => ops::delete(connection, path, confirm).await,
            other => Err(TransferError::InvalidInput(format!(
                "unknown op \"{other}\"; use mkdir, rename or delete"
            ))),
        }
    };
    if !matches!(op, "mkdir" | "rename" | "delete") {
        return Err(TransferError::InvalidInput(format!(
            "unknown op \"{op}\"; use mkdir, rename or delete"
        )));
    }
    let resolved = resolved(target).await?;
    run(&resolved.connection).await
}

#[tauri::command]
pub async fn transfer_servers_list() -> Result<Vec<ServerView>, String> {
    servers().await.map_err(|e| e.wire())
}

#[tauri::command]
pub async fn transfer_server_save(
    draft: Draft,
    secret: Option<String>,
) -> Result<ServerView, String> {
    save_server(draft, secret).await.map_err(|e| e.wire())
}

#[tauri::command]
pub async fn transfer_server_remove(name: String) -> Result<String, String> {
    remove_server(name).await.map_err(|e| e.wire())
}

#[tauri::command]
pub async fn transfer_host_forget(host: String, port: u32) -> Result<usize, String> {
    forget_host(host, Some(port)).await.map_err(|e| e.wire())
}

#[tauri::command]
pub async fn transfer_list(
    target: TargetInput,
    path: Option<String>,
    limit: Option<usize>,
) -> Result<Listing, String> {
    list(target, path, limit).await.map_err(|e| e.wire())
}

#[tauri::command]
pub async fn transfer_get(
    target: TargetInput,
    path: String,
    save_to: String,
    overwrite: bool,
) -> Result<Snapshot, String> {
    let id = start_get(target, path, save_to, overwrite)
        .await
        .map_err(|e| e.wire())?;
    jobs::wait(&id, 0).await.map_err(|e| e.wire())
}

#[tauri::command]
pub async fn transfer_put(
    target: TargetInput,
    local_path: String,
    path: String,
    overwrite: bool,
) -> Result<Snapshot, String> {
    let id = start_put(target, local_path, path, overwrite)
        .await
        .map_err(|e| e.wire())?;
    jobs::wait(&id, 0).await.map_err(|e| e.wire())
}

#[tauri::command]
pub async fn transfer_status(
    transfer_id: String,
    wait_seconds: Option<u64>,
) -> Result<Snapshot, String> {
    jobs::wait(&transfer_id, wait_seconds.unwrap_or(0))
        .await
        .map_err(|e| e.wire())
}

#[tauri::command]
pub async fn transfer_manage(
    target: TargetInput,
    op: String,
    path: String,
    to: Option<String>,
    confirm: bool,
) -> Result<Managed, String> {
    manage(target, &op, &path, to.as_deref(), confirm)
        .await
        .map_err(|e| e.wire())
}

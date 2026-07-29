#[tauri::command]
pub async fn get_hwaccel_info() -> omniget_core::core::hwaccel::HwAccelInfo {
    omniget_core::core::hwaccel::detect_hwaccel().await
}

#[tauri::command]
pub fn diagnose_download_error(stderr: String) -> Option<crate::core::root_cause::Diagnosis> {
    crate::core::root_cause::diagnose(&stderr)
}

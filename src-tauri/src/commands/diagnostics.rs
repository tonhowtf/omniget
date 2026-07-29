#[tauri::command]
pub async fn get_hwaccel_info() -> omniget_core::core::hwaccel::HwAccelInfo {
    omniget_core::core::hwaccel::detect_hwaccel().await
}

#[tauri::command]
pub fn diagnose_download_error(stderr: String) -> Option<crate::core::root_cause::Diagnosis> {
    crate::core::root_cause::diagnose(&stderr)
}

/// Despejo da caixa-preta, para colar num relatorio de bug.
///
/// As linhas ja saem redigidas pelo `flight_recorder::redact` — token, cookie e
/// caminho com nome de usuario nao chegam ate aqui.
#[tauri::command]
pub fn flight_recorder_dump() -> Vec<String> {
    crate::core::flight_recorder::dump()
}

/// Apaga o que a caixa-preta guardou.
///
/// Existe porque o usuario tem que poder decidir nao mandar nada, mesmo depois
/// de ter aberto o relatorio.
#[tauri::command]
pub fn flight_recorder_clear() {
    crate::core::flight_recorder::clear();
}

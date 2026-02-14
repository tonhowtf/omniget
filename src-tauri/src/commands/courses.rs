use crate::platforms::hotmart::api::{self, Course, Module};
use crate::AppState;

#[tauri::command]
pub async fn hotmart_list_courses(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<Course>, String> {
    let guard = state.hotmart_session.lock().await;
    let session = guard
        .as_ref()
        .ok_or_else(|| "Não autenticado. Faça login primeiro.".to_string())?;

    let mut courses = api::list_courses(session).await.map_err(|e| e.to_string())?;

    match api::get_subdomains(session).await {
        Ok(subdomains) => api::merge_subdomains(&mut courses, &subdomains),
        Err(e) => tracing::warn!("Falha ao buscar subdomínios: {}. Continuando sem eles.", e),
    }

    Ok(courses)
}

#[tauri::command]
pub async fn hotmart_get_modules(
    state: tauri::State<'_, AppState>,
    course_id: u64,
    slug: String,
) -> Result<Vec<Module>, String> {
    let guard = state.hotmart_session.lock().await;
    let session = guard
        .as_ref()
        .ok_or_else(|| "Não autenticado. Faça login primeiro.".to_string())?;

    api::get_modules(session, &slug, course_id)
        .await
        .map_err(|e| e.to_string())
}

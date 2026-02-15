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

    // Step 1: get subdomains via check_token
    let subdomains = match api::get_subdomains(session).await {
        Ok(s) => {
            tracing::info!("Subdomínios carregados: {}", s.len());
            s
        }
        Err(e) => {
            tracing::warn!("Falha ao buscar subdomínios: {}. Continuando sem eles.", e);
            vec![]
        }
    };

    // Step 2: list courses
    let mut courses = api::list_courses(session).await.map_err(|e| e.to_string())?;

    // Step 3: merge subdomains
    api::merge_subdomains(&mut courses, &subdomains);

    // Step 4: fetch prices for each course
    for course in &mut courses {
        match api::get_course_price(session, course.id).await {
            Ok(price) => {
                course.price = Some(price);
                tracing::info!("- {} (ID: {}, slug: {:?}, preço: R${:.2})", course.name, course.id, course.slug, price);
            }
            Err(e) => {
                tracing::warn!("Preço não disponível para {} (ID: {}): {}", course.name, course.id, e);
            }
        }
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

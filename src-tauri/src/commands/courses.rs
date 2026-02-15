use std::time::{Duration, Instant};

use crate::platforms::hotmart::api::{self, Course, Module};
use crate::{AppState, CoursesCache};

const COURSES_CACHE_TTL: Duration = Duration::from_secs(10 * 60);

async fn fetch_courses_from_api(state: &tauri::State<'_, AppState>) -> Result<Vec<Course>, String> {
    let guard = state.hotmart_session.lock().await;
    let session = guard
        .as_ref()
        .ok_or_else(|| "Não autenticado. Faça login primeiro.".to_string())?;

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

    let mut courses = api::list_courses(session).await.map_err(|e| e.to_string())?;

    api::merge_subdomains(&mut courses, &subdomains);

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

    let mut cache = state.courses_cache.lock().await;
    *cache = Some(CoursesCache {
        courses: courses.clone(),
        fetched_at: Instant::now(),
    });

    Ok(courses)
}

#[tauri::command]
pub async fn hotmart_list_courses(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<Course>, String> {
    {
        let cache = state.courses_cache.lock().await;
        if let Some(ref cached) = *cache {
            if cached.fetched_at.elapsed() < COURSES_CACHE_TTL {
                tracing::info!(
                    "Retornando {} cursos do cache ({:?} atrás)",
                    cached.courses.len(),
                    cached.fetched_at.elapsed()
                );
                return Ok(cached.courses.clone());
            }
        }
    }

    fetch_courses_from_api(&state).await
}

#[tauri::command]
pub async fn hotmart_refresh_courses(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<Course>, String> {
    {
        let mut cache = state.courses_cache.lock().await;
        *cache = None;
    }
    tracing::info!("Cache de cursos invalidado, recarregando...");
    fetch_courses_from_api(&state).await
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

use std::time::{Duration, Instant};

use crate::platforms::hotmart::api::{self, Course, Module};
use crate::{AppState, CoursesCache};

const COURSES_CACHE_TTL: Duration = Duration::from_secs(10 * 60);

async fn fetch_courses_from_api(state: &tauri::State<'_, AppState>) -> Result<Vec<Course>, String> {
    let guard = state.hotmart_session.lock().await;
    let session = guard
        .as_ref()
        .ok_or_else(|| "Not authenticated. Please log in first.".to_string())?;

    let subdomains = api::get_subdomains(session).await.unwrap_or_default();

    let mut courses = api::list_courses(session).await.map_err(|e| e.to_string())?;

    api::merge_subdomains(&mut courses, &subdomains);

    for course in &mut courses {
        if let Ok(price) = api::get_course_price(session, course.id).await {
            course.price = Some(price);
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
        .ok_or_else(|| "Not authenticated. Please log in first.".to_string())?;

    api::get_modules(session, &slug, course_id)
        .await
        .map_err(|e| e.to_string())
}

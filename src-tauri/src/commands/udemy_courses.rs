use std::time::{Duration, Instant};

use crate::platforms::udemy::api::{self, UdemyCourse};
use crate::{AppState, UdemyCoursesCache};

const COURSES_CACHE_TTL: Duration = Duration::from_secs(10 * 60);
const DEFAULT_PORTAL: &str = "www";

async fn fetch_courses_from_api(state: &tauri::State<'_, AppState>) -> Result<Vec<UdemyCourse>, String> {
    let guard = state.udemy_session.lock().await;
    let session = guard
        .as_ref()
        .ok_or_else(|| "Not authenticated. Please log in first.".to_string())?;

    let courses = api::list_all_courses(session, DEFAULT_PORTAL)
        .await
        .map_err(|e| e.to_string())?;

    drop(guard);

    let mut cache = state.udemy_courses_cache.lock().await;
    *cache = Some(UdemyCoursesCache {
        courses: courses.clone(),
        fetched_at: Instant::now(),
    });

    Ok(courses)
}

#[tauri::command]
pub async fn udemy_list_courses(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<UdemyCourse>, String> {
    {
        let cache = state.udemy_courses_cache.lock().await;
        if let Some(ref cached) = *cache {
            if cached.fetched_at.elapsed() < COURSES_CACHE_TTL {
                return Ok(cached.courses.clone());
            }
        }
    }

    fetch_courses_from_api(&state).await
}

#[tauri::command]
pub async fn udemy_refresh_courses(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<UdemyCourse>, String> {
    {
        let mut cache = state.udemy_courses_cache.lock().await;
        *cache = None;
    }
    fetch_courses_from_api(&state).await
}

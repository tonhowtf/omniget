use std::time::Instant;

use crate::platforms::udemy::api::UdemyCourse;
use crate::platforms::udemy::webview_api;
use crate::{AppState, UdemyCoursesCache};

const COURSES_CACHE_TTL: std::time::Duration = std::time::Duration::from_secs(10 * 60);

async fn get_portal(state: &tauri::State<'_, AppState>) -> String {
    let guard = state.udemy_session.lock().await;
    guard
        .as_ref()
        .map(|s| s.portal_name.clone())
        .unwrap_or_else(|| "www".into())
}

fn parse_courses_from_results(results: &[serde_json::Value]) -> Vec<UdemyCourse> {
    results
        .iter()
        .filter_map(|item| {
            let id = item.get("id")?.as_u64()?;
            let title = item.get("title")?.as_str().unwrap_or("").to_string();
            let published_title = item
                .get("published_title")?
                .as_str()
                .unwrap_or("")
                .to_string();
            let url = item
                .get("url")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            let image_url = item
                .get("image_240x135")
                .or_else(|| item.get("image_480x270"))
                .or_else(|| item.get("image_url"))
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            let num_published_lectures = item
                .get("num_published_lectures")
                .and_then(|v| v.as_u64())
                .map(|n| n as u32);

            Some(UdemyCourse {
                id,
                title,
                published_title,
                url,
                image_url,
                num_published_lectures,
            })
        })
        .collect()
}

async fn fetch_courses_via_api(
    state: &tauri::State<'_, AppState>,
) -> Result<Vec<UdemyCourse>, String> {
    let (client, portal) = {
        let guard = state.udemy_session.lock().await;
        let session = guard.as_ref().ok_or("not_authenticated")?;
        (session.client.clone(), session.portal_name.clone())
    };

    let url = format!(
        "https://{}.udemy.com/api-2.0/users/me/subscribed-courses?fields[course]=id,url,title,published_title,image_240x135,num_published_lectures&ordering=-last_accessed,-access_time&page=1&page_size=10000",
        portal
    );

    tracing::info!("[udemy-api] fetching courses via direct API for portal={}", portal);

    let resp = client
        .get(&url)
        .send()
        .await
        .map_err(|e| format!("API request failed: {}", e))?;

    if !resp.status().is_success() {
        return Err(format!("API returned status {}", resp.status()));
    }

    let body = resp.text().await.map_err(|e| format!("Read body failed: {}", e))?;
    let data: serde_json::Value =
        serde_json::from_str(&body).map_err(|e| format!("JSON parse error: {}", e))?;

    let results = data
        .get("results")
        .and_then(|r| r.as_array())
        .cloned()
        .unwrap_or_default();

    let mut courses = parse_courses_from_results(&results);

    let sub_url = format!(
        "https://{}.udemy.com/api-2.0/users/me/subscription-course-enrollments?fields[course]=id,title,published_title,image_240x135,num_published_lectures&page=1&page_size=50",
        portal
    );

    if let Ok(sub_resp) = client.get(&sub_url).send().await {
        if sub_resp.status().is_success() {
            if let Ok(sub_body) = sub_resp.text().await {
                if let Ok(sub_data) = serde_json::from_str::<serde_json::Value>(&sub_body) {
                    if let Some(sub_results) = sub_data.get("results").and_then(|r| r.as_array()) {
                        let existing_ids: std::collections::HashSet<u64> =
                            courses.iter().map(|c| c.id).collect();
                        for c in parse_courses_from_results(sub_results) {
                            if !existing_ids.contains(&c.id) {
                                courses.push(c);
                            }
                        }
                    }
                }
            }
        }
    }

    tracing::info!("[udemy-api] found {} total courses via direct API", courses.len());

    let mut cache = state.udemy_courses_cache.lock().await;
    *cache = Some(UdemyCoursesCache {
        courses: courses.clone(),
        fetched_at: Instant::now(),
    });

    Ok(courses)
}

async fn fetch_courses_via_webview(
    app: &tauri::AppHandle,
    state: &tauri::State<'_, AppState>,
) -> Result<Vec<UdemyCourse>, String> {
    let portal = get_portal(state).await;

    let result_store = state.udemy_api_result.clone();

    let window = {
        let mut wv_guard = state.udemy_api_webview.lock().await;
        match &*wv_guard {
            Some(w) => w.clone(),
            None => {
                let w = webview_api::ensure_api_webview(app, &result_store, &portal)
                    .await
                    .map_err(|e| e.to_string())?;
                *wv_guard = Some(w.clone());
                w
            }
        }
    };

    let url = format!(
        "https://{}.udemy.com/api-2.0/users/me/subscribed-courses?fields[course]=id,url,title,published_title,image_240x135,num_published_lectures&ordering=-last_accessed,-access_time&page=1&page_size=10000",
        portal
    );

    tracing::info!("[udemy-api] fetching courses via webview");

    let body = webview_api::webview_get(&window, &url, &result_store)
        .await
        .map_err(|e| e.to_string())?;

    let data: serde_json::Value =
        serde_json::from_str(&body).map_err(|e| format!("JSON parse error: {}", e))?;

    let results = data
        .get("results")
        .and_then(|r| r.as_array())
        .cloned()
        .unwrap_or_default();

    let mut courses = parse_courses_from_results(&results);

    let sub_url = format!(
        "https://{}.udemy.com/api-2.0/users/me/subscription-course-enrollments?fields[course]=id,title,published_title,image_240x135,num_published_lectures&page=1&page_size=50",
        portal
    );

    if let Ok(sub_body) = webview_api::webview_get(&window, &sub_url, &result_store).await {
        if let Ok(sub_data) = serde_json::from_str::<serde_json::Value>(&sub_body) {
            if let Some(sub_results) = sub_data.get("results").and_then(|r| r.as_array()) {
                let existing_ids: std::collections::HashSet<u64> =
                    courses.iter().map(|c| c.id).collect();
                for c in parse_courses_from_results(sub_results) {
                    if !existing_ids.contains(&c.id) {
                        courses.push(c);
                    }
                }
            }
        }
    }

    tracing::info!("[udemy-api] found {} total courses via webview", courses.len());

    let mut cache = state.udemy_courses_cache.lock().await;
    *cache = Some(UdemyCoursesCache {
        courses: courses.clone(),
        fetched_at: Instant::now(),
    });

    Ok(courses)
}

async fn fetch_courses(
    app: &tauri::AppHandle,
    state: &tauri::State<'_, AppState>,
) -> Result<Vec<UdemyCourse>, String> {
    let portal = get_portal(state).await;

    if portal != "www" {
        match fetch_courses_via_api(state).await {
            Ok(courses) => return Ok(courses),
            Err(e) => {
                tracing::warn!("[udemy-api] direct API failed for portal={}, falling back to webview: {}", portal, e);
            }
        }
    }

    fetch_courses_via_webview(app, state).await
}

#[tauri::command]
pub async fn udemy_list_courses(
    app: tauri::AppHandle,
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

    fetch_courses(&app, &state).await
}

#[tauri::command]
pub async fn udemy_refresh_courses(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
) -> Result<Vec<UdemyCourse>, String> {
    {
        let mut cache = state.udemy_courses_cache.lock().await;
        *cache = None;
    }
    fetch_courses(&app, &state).await
}

use std::time::Duration;

use anyhow::{anyhow, Result};
use regex::Regex;
use serde::{Deserialize, Serialize};

use super::auth::UdemySession;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UdemyCourse {
    pub id: u64,
    pub title: String,
    pub published_title: String,
    pub url: Option<String>,
    pub image_url: Option<String>,
    pub num_published_lectures: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UdemyChapter {
    pub id: u64,
    pub title: String,
    pub object_index: u32,
    pub lectures: Vec<UdemyLecture>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UdemyLecture {
    pub id: u64,
    pub title: String,
    pub object_index: u32,
    pub lecture_class: String,
    pub asset: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UdemyCurriculum {
    pub course_id: u64,
    pub title: String,
    pub chapters: Vec<UdemyChapter>,
    pub total_lectures: u32,
    pub total_video_lectures: u32,
    pub drm_video_lectures: u32,
}

pub fn extract_course_name(url: &str) -> Option<(String, String)> {
    let re = Regex::new(
        r"(?i)://(.+?)\.udemy\.com/(?:course(?:/draft)*/)?([a-zA-Z0-9_-]+)"
    ).ok()?;
    let caps = re.captures(url)?;
    let portal_name = caps.get(1)?.as_str().to_string();
    let course_slug = caps.get(2)?.as_str().to_string();
    Some((portal_name, course_slug))
}

async fn api_get_with_retry(
    client: &reqwest::Client,
    url: &str,
    params: Option<&[(&str, &str)]>,
) -> Result<reqwest::Response> {
    let max_attempts: u32 = 3;
    let mut last_err = None;

    for attempt in 0..max_attempts {
        let mut req = client.get(url);
        if let Some(p) = params {
            req = req.query(p);
        }

        match req.send().await {
            Ok(resp) => {
                let status = resp.status();
                tracing::info!("[udemy-api] {} → {}", url, status);
                if status.is_success() {
                    return Ok(resp);
                }
                if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN {
                    let body = resp.text().await.unwrap_or_default();
                    tracing::error!("[udemy-api] auth error for {}: {} — {}", url, status, &body[..body.len().min(500)]);
                    return Err(anyhow!("API auth error ({}): {}", status, body));
                }
                let body = resp.text().await.unwrap_or_default();
                tracing::warn!("[udemy-api] non-success for {}: {} — {}", url, status, &body[..body.len().min(500)]);
                last_err = Some(anyhow!("API returned {}: {}", status, body));
            }
            Err(e) => {
                last_err = Some(anyhow!("Request failed: {}", e));
            }
        }

        if attempt < max_attempts - 1 {
            let backoff = Duration::from_millis(500 * 2u64.pow(attempt));
            tracing::warn!(
                "[udemy-api] attempt {}/{} failed, retrying in {:?}",
                attempt + 1,
                max_attempts,
                backoff
            );
            tokio::time::sleep(backoff).await;
        }
    }

    Err(last_err.unwrap_or_else(|| anyhow!("All retry attempts failed")))
}

async fn handle_pagination(
    session: &UdemySession,
    initial_url: &str,
    params: Option<&[(&str, &str)]>,
) -> Result<serde_json::Value> {
    let resp = api_get_with_retry(&session.client, initial_url, params).await?;

    let resp_text = resp.text().await
        .map_err(|e| anyhow!("Failed to read response body: {}", e))?;

    let mut data: serde_json::Value = serde_json::from_str(&resp_text)
        .map_err(|e| {
            let preview = if resp_text.len() > 500 { &resp_text[..500] } else { &resp_text };
            tracing::error!("[udemy-api] JSON parse failed. Body preview: {}", preview);
            anyhow!("Failed to parse JSON: {} — body starts with: {}", e, &resp_text[..resp_text.len().min(200)])
        })?;

    let count = data.get("count").and_then(|c| c.as_u64());
    if count.is_none() {
        tracing::warn!("[udemy-api] response missing 'count' field. Keys: {:?}",
            data.as_object().map(|o| o.keys().collect::<Vec<_>>()));
        return Ok(data);
    }

    let mut page = 1u32;
    loop {
        let next_url = data.get("next").and_then(|n| n.as_str()).map(|s| s.to_string());
        match next_url {
            Some(url) if !url.is_empty() => {
                page += 1;
                tracing::info!("[udemy-api] fetching page {}", page);

                tokio::time::sleep(Duration::from_millis(200)).await;

                let resp = match api_get_with_retry(&session.client, &url, None).await {
                    Ok(r) => r,
                    Err(e) => {
                        tracing::error!("[udemy-api] page {} failed: {}", page, e);
                        break;
                    }
                };

                let page_data: serde_json::Value = resp.json().await
                    .map_err(|e| anyhow!("Failed to parse page JSON: {}", e))?;

                if let Some(new_results) = page_data.get("results").and_then(|r| r.as_array()) {
                    if let Some(existing) = data.get_mut("results").and_then(|r| r.as_array_mut()) {
                        existing.extend(new_results.iter().cloned());
                    }
                }

                if let Some(next) = page_data.get("next") {
                    data["next"] = next.clone();
                } else {
                    data["next"] = serde_json::Value::Null;
                }
            }
            _ => break,
        }
    }

    Ok(data)
}

fn parse_course_from_json(item: &serde_json::Value) -> Option<UdemyCourse> {
    let id = item.get("id")?.as_u64()?;
    let title = item.get("title")?.as_str().unwrap_or("").to_string();
    let published_title = item.get("published_title")?.as_str().unwrap_or("").to_string();
    let url = item.get("url").and_then(|v| v.as_str()).map(|s| s.to_string());

    let image_url = item.get("image_240x135")
        .or_else(|| item.get("image_480x270"))
        .or_else(|| item.get("image_url"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let num_published_lectures = item.get("num_published_lectures")
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
}

pub async fn list_my_courses(
    session: &UdemySession,
    portal_name: &str,
) -> Result<Vec<UdemyCourse>> {
    let url = format!(
        "https://{}.udemy.com/api-2.0/users/me/subscribed-courses?fields[course]=id,url,title,published_title,image_240x135,num_published_lectures&ordering=-last_accessed,-access_time&page=1&page_size=10000",
        portal_name
    );

    tracing::info!("[udemy-api] fetching subscribed courses from {}", portal_name);

    let data = handle_pagination(session, &url, None).await?;

    let results = data.get("results")
        .and_then(|r| r.as_array())
        .cloned()
        .unwrap_or_default();

    let courses: Vec<UdemyCourse> = results
        .iter()
        .filter_map(parse_course_from_json)
        .collect();

    tracing::info!("[udemy-api] found {} subscribed courses", courses.len());
    Ok(courses)
}

pub async fn list_subscription_courses(
    session: &UdemySession,
    portal_name: &str,
) -> Result<Vec<UdemyCourse>> {
    let url = format!(
        "https://{}.udemy.com/api-2.0/users/me/subscription-course-enrollments?fields[course]=title,published_title,image_240x135,num_published_lectures&page=1&page_size=50",
        portal_name
    );

    tracing::info!("[udemy-api] fetching subscription course enrollments from {}", portal_name);

    let data = handle_pagination(session, &url, None).await?;

    let results = data.get("results")
        .and_then(|r| r.as_array())
        .cloned()
        .unwrap_or_default();

    let courses: Vec<UdemyCourse> = results
        .iter()
        .filter_map(parse_course_from_json)
        .collect();

    tracing::info!("[udemy-api] found {} subscription courses", courses.len());
    Ok(courses)
}

pub async fn list_all_courses(
    session: &UdemySession,
    portal_name: &str,
) -> Result<Vec<UdemyCourse>> {
    let my_courses_result = list_my_courses(session, portal_name).await;
    let sub_courses_result = list_subscription_courses(session, portal_name).await;

    if let (Err(err), Err(_)) = (&my_courses_result, &sub_courses_result) {
        tracing::error!("[udemy-api] both course fetches failed: {}", err);
        return Err(my_courses_result.unwrap_err());
    }

    let mut my_courses = match my_courses_result {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!("[udemy-api] subscribed courses failed (continuing with subscription): {}", e);
            Vec::new()
        }
    };

    let sub_courses = match sub_courses_result {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!("[udemy-api] subscription enrollments failed (continuing with subscribed): {}", e);
            Vec::new()
        }
    };

    let existing_ids: std::collections::HashSet<u64> = my_courses.iter().map(|c| c.id).collect();
    for course in sub_courses {
        if !existing_ids.contains(&course.id) {
            my_courses.push(course);
        }
    }

    tracing::info!("[udemy-api] total unique courses: {}", my_courses.len());
    Ok(my_courses)
}

pub fn parse_curriculum(course_id: u64, results: &[serde_json::Value]) -> Result<UdemyCurriculum> {
    let mut chapters: Vec<UdemyChapter> = Vec::new();
    let mut current_chapter: Option<UdemyChapter> = None;
    let mut total_lectures: u32 = 0;
    let mut total_video_lectures: u32 = 0;
    let mut drm_video_lectures: u32 = 0;
    let mut course_title = String::new();

    for item in results {
        let class = item.get("_class").and_then(|c| c.as_str()).unwrap_or("");

        match class {
            "chapter" => {
                if let Some(ch) = current_chapter.take() {
                    chapters.push(ch);
                }

                let id = item.get("id").and_then(|v| v.as_u64()).unwrap_or(0);
                let title = item.get("title").and_then(|v| v.as_str()).unwrap_or("").to_string();
                let object_index = item.get("object_index").and_then(|v| v.as_u64()).unwrap_or(0) as u32;

                if course_title.is_empty() && !title.is_empty() {
                    course_title = title.clone();
                }

                current_chapter = Some(UdemyChapter {
                    id,
                    title,
                    object_index,
                    lectures: Vec::new(),
                });
            }
            "lecture" | "quiz" | "practice" => {
                let id = item.get("id").and_then(|v| v.as_u64()).unwrap_or(0);
                let title = item.get("title").and_then(|v| v.as_str()).unwrap_or("").to_string();
                let object_index = item.get("object_index").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
                let asset = item.get("asset").cloned();

                if class == "lecture" {
                    total_lectures += 1;

                    if let Some(ref a) = asset {
                        let asset_type = a.get("asset_type")
                            .or_else(|| a.get("assetType"))
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_lowercase();

                        if asset_type == "video" {
                            total_video_lectures += 1;
                            let has_stream_urls = a.get("stream_urls")
                                .map(|v| !v.is_null())
                                .unwrap_or(false);
                            let has_media_sources = a.get("media_sources")
                                .map(|v| !v.is_null())
                                .unwrap_or(false);

                            if !has_stream_urls && has_media_sources {
                                drm_video_lectures += 1;
                            }
                        }
                    }
                }

                let lecture = UdemyLecture {
                    id,
                    title,
                    object_index,
                    lecture_class: class.to_string(),
                    asset,
                };

                if let Some(ref mut ch) = current_chapter {
                    ch.lectures.push(lecture);
                } else {
                    let mut implicit_chapter = UdemyChapter {
                        id: 0,
                        title: "Introduction".to_string(),
                        object_index: 0,
                        lectures: Vec::new(),
                    };
                    implicit_chapter.lectures.push(lecture);
                    current_chapter = Some(implicit_chapter);
                }
            }
            _ => {}
        }
    }

    if let Some(ch) = current_chapter.take() {
        chapters.push(ch);
    }

    tracing::info!(
        "[udemy-api] curriculum: {} chapters, {} lectures, {} video, {} drm",
        chapters.len(), total_lectures, total_video_lectures, drm_video_lectures
    );

    Ok(UdemyCurriculum {
        course_id,
        title: course_title,
        chapters,
        total_lectures,
        total_video_lectures,
        drm_video_lectures,
    })
}

pub async fn get_course_curriculum(
    session: &UdemySession,
    portal_name: &str,
    course_id: u64,
) -> Result<UdemyCurriculum> {
    let url = format!(
        "https://{}.udemy.com/api-2.0/courses/{}/subscriber-curriculum-items/",
        portal_name, course_id
    );

    let params: &[(&str, &str)] = &[
        ("fields[lecture]", "title,object_index,asset,supplementary_assets"),
        ("fields[quiz]", "title,object_index,type"),
        ("fields[practice]", "title,object_index"),
        ("fields[chapter]", "title,object_index"),
        ("fields[asset]", "title,filename,asset_type,status,is_external,media_license_token,course_is_drmed,media_sources,captions,stream_urls,download_urls,external_url,body"),
        ("page_size", "200"),
    ];

    tracing::info!("[udemy-api] fetching curriculum for course {}", course_id);

    let data = handle_pagination(session, &url, Some(params)).await?;

    let results = data.get("results")
        .and_then(|r| r.as_array())
        .cloned()
        .unwrap_or_default();

    parse_curriculum(course_id, &results)
}

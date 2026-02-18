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

async fn handle_pagination(
    session: &UdemySession,
    initial_url: &str,
    params: Option<&[(&str, &str)]>,
) -> Result<serde_json::Value> {
    let mut request = session.client.get(initial_url);
    if let Some(p) = params {
        request = request.query(p);
    }

    let resp = request
        .send()
        .await
        .map_err(|e| anyhow!("Request failed: {}", e))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(anyhow!("API returned {}: {}", status, body));
    }

    let mut data: serde_json::Value = resp.json().await
        .map_err(|e| anyhow!("Failed to parse JSON: {}", e))?;

    let count = data.get("count").and_then(|c| c.as_u64());
    if count.is_none() {
        tracing::warn!("[udemy-api] response missing 'count' field");
        return Ok(data);
    }

    let mut page = 1u32;
    loop {
        let next_url = data.get("next").and_then(|n| n.as_str()).map(|s| s.to_string());
        match next_url {
            Some(url) if !url.is_empty() => {
                page += 1;
                tracing::info!("[udemy-api] fetching page {}", page);

                let resp = session.client.get(&url)
                    .send()
                    .await
                    .map_err(|e| anyhow!("Pagination request failed: {}", e))?;

                if !resp.status().is_success() {
                    tracing::error!("[udemy-api] page {} returned {}", page, resp.status());
                    break;
                }

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
    let mut my_courses = list_my_courses(session, portal_name).await.unwrap_or_default();

    let sub_courses = list_subscription_courses(session, portal_name).await.unwrap_or_default();

    let existing_ids: std::collections::HashSet<u64> = my_courses.iter().map(|c| c.id).collect();
    for course in sub_courses {
        if !existing_ids.contains(&course.id) {
            my_courses.push(course);
        }
    }

    tracing::info!("[udemy-api] total unique courses: {}", my_courses.len());
    Ok(my_courses)
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
        ("fields[chapter]", "title,object_index"),
        ("fields[asset]", "title,filename,asset_type,status,media_sources,captions,stream_urls,download_urls,body"),
        ("page_size", "200"),
    ];

    tracing::info!("[udemy-api] fetching curriculum for course {}", course_id);

    let data = handle_pagination(session, &url, Some(params)).await?;

    let results = data.get("results")
        .and_then(|r| r.as_array())
        .cloned()
        .unwrap_or_default();

    let mut chapters: Vec<UdemyChapter> = Vec::new();
    let mut current_chapter: Option<UdemyChapter> = None;
    let mut total_lectures: u32 = 0;
    let mut course_title = String::new();

    for item in &results {
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

                let lecture = UdemyLecture {
                    id,
                    title,
                    object_index,
                    lecture_class: class.to_string(),
                    asset,
                };

                if class == "lecture" {
                    total_lectures += 1;
                }

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
        "[udemy-api] curriculum: {} chapters, {} lectures",
        chapters.len(),
        total_lectures
    );

    Ok(UdemyCurriculum {
        course_id,
        title: course_title,
        chapters,
        total_lectures,
    })
}

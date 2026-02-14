use anyhow::anyhow;
use serde::{Deserialize, Serialize};

use super::auth::HotmartSession;
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Course {
    pub id: u64,
    pub name: String,
    pub slug: Option<String>,
    pub seller: String,
    pub subdomain: Option<String>,
    pub is_hotmart_club: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Module {
    pub id: String,
    pub name: String,
    pub pages: Vec<PageInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PageInfo {
    pub hash: String,
    pub name: String,
    pub page_type: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Lesson {
    pub content: Option<String>,
    pub has_player_media: bool,
    pub medias_src: Vec<MediaSrc>,
    pub attachments: Vec<Attachment>,
    pub complementary_readings: Option<Vec<ReadingLink>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MediaSrc {
    pub media_name: String,
    pub media_src_url: String,
    pub media_type: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Attachment {
    pub file_membership_id: String,
    pub file_name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReadingLink {
    pub title: Option<String>,
    pub url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AttachmentInfo {
    pub url: String,
    pub file_name: Option<String>,
}

#[derive(Debug, Clone)]
pub struct SubdomainInfo {
    pub product_id: u64,
    pub subdomain: String,
}

pub async fn list_courses(session: &HotmartSession) -> anyhow::Result<Vec<Course>> {
    tracing::info!("Listando cursos Hotmart...");

    let resp = session
        .client
        .get("https://api-hub.cb.hotmart.com/club-drive-api/rest/v2/purchase/?archived=UNARCHIVED")
        .header("Authorization", format!("Bearer {}", session.token))
        .header("Origin", "https://consumer.hotmart.com")
        .send()
        .await?
        .error_for_status()?;

    let body: serde_json::Value = resp.json().await?;

    let purchases = body
        .as_array()
        .or_else(|| body.get("purchases").and_then(|p| p.as_array()))
        .ok_or_else(|| anyhow!("Formato inesperado na resposta de cursos"))?;

    let mut courses = Vec::new();
    for p in purchases {
        let product = p.get("product").unwrap_or(p);
        let id = product
            .get("id")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        let name = product
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let seller = p
            .get("producer")
            .or_else(|| p.get("seller"))
            .and_then(|s| s.get("name").and_then(|n| n.as_str()))
            .unwrap_or("")
            .to_string();
        let is_hotmart_club = p
            .get("accessRights")
            .and_then(|a| a.get("hasClubAccess"))
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        courses.push(Course {
            id,
            name,
            slug: None,
            seller,
            subdomain: None,
            is_hotmart_club,
        });
    }

    tracing::info!("{} cursos encontrados", courses.len());
    Ok(courses)
}

pub async fn get_subdomains(session: &HotmartSession) -> anyhow::Result<Vec<SubdomainInfo>> {
    tracing::info!("Buscando subdomínios...");

    let url = format!(
        "https://api-sec-vlc.hotmart.com/security/oauth/check_token?token={}",
        session.token
    );
    let resp = session
        .client
        .get(&url)
        .header("Authorization", format!("Bearer {}", session.token))
        .send()
        .await?
        .error_for_status()?;

    let body: serde_json::Value = resp.json().await?;

    let resources = body
        .get("resources")
        .and_then(|r| r.as_array())
        .ok_or_else(|| anyhow!("Campo 'resources' não encontrado em check_token"))?;

    let mut subdomains = Vec::new();
    for res in resources {
        let resource = match res.get("resource") {
            Some(r) => r,
            None => continue,
        };
        let product_id = resource
            .get("productId")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        let subdomain = resource
            .get("subdomain")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        if product_id > 0 && !subdomain.is_empty() {
            subdomains.push(SubdomainInfo {
                product_id,
                subdomain,
            });
        }
    }

    tracing::info!("{} subdomínios encontrados", subdomains.len());
    Ok(subdomains)
}

pub fn merge_subdomains(courses: &mut [Course], subdomains: &[SubdomainInfo]) {
    for course in courses.iter_mut() {
        if let Some(info) = subdomains.iter().find(|s| s.product_id == course.id) {
            course.subdomain = Some(info.subdomain.clone());
            course.slug = Some(info.subdomain.clone());
        }
    }
}

pub async fn get_modules(
    session: &HotmartSession,
    slug: &str,
    product_id: u64,
) -> anyhow::Result<Vec<Module>> {
    tracing::info!("Buscando módulos do curso {} (slug={})", product_id, slug);

    let resp = session
        .client
        .get("https://api-club-course-consumption-gateway.hotmart.com/v1/navigation")
        .header("Authorization", format!("Bearer {}", session.token))
        .header("Origin", "https://consumer.hotmart.com")
        .header("Host", "api-club-course-consumption-gateway.hotmart.com")
        .header("slug", slug)
        .header("x-product-id", product_id.to_string())
        .send()
        .await?
        .error_for_status()?;

    let body: serde_json::Value = resp.json().await?;

    let modules_json = body
        .as_array()
        .or_else(|| body.get("modules").and_then(|m| m.as_array()))
        .ok_or_else(|| anyhow!("Formato inesperado na resposta de módulos"))?;

    let mut modules = Vec::new();
    for m in modules_json {
        let id = m
            .get("id")
            .map(|v| match v {
                serde_json::Value::Number(n) => n.to_string(),
                serde_json::Value::String(s) => s.clone(),
                _ => String::new(),
            })
            .unwrap_or_default();
        let name = m
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        let pages_json = m
            .get("pages")
            .and_then(|p| p.as_array())
            .cloned()
            .unwrap_or_default();

        let pages = pages_json
            .iter()
            .map(|p| PageInfo {
                hash: p
                    .get("hash")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
                name: p
                    .get("name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
                page_type: p
                    .get("type")
                    .or_else(|| p.get("pageType"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
            })
            .collect();

        modules.push(Module { id, name, pages });
    }

    tracing::info!("{} módulos encontrados", modules.len());
    Ok(modules)
}

pub async fn get_lesson(
    session: &HotmartSession,
    slug: &str,
    product_id: u64,
    page_hash: &str,
) -> anyhow::Result<Lesson> {
    tracing::info!("Buscando lição {}", page_hash);

    let url = format!(
        "https://api-club-course-consumption-gateway.hotmart.com/v1/lesson/{}",
        page_hash
    );
    let resp = session
        .client
        .get(&url)
        .header("Authorization", format!("Bearer {}", session.token))
        .header("Origin", "https://consumer.hotmart.com")
        .header("Host", "api-club-course-consumption-gateway.hotmart.com")
        .header("slug", slug)
        .header("x-product-id", product_id.to_string())
        .send()
        .await?
        .error_for_status()?;

    let body: serde_json::Value = resp.json().await?;

    let content = body.get("content").and_then(|v| v.as_str()).map(String::from);
    let has_player_media = body
        .get("hasPlayerMedia")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    let medias_src: Vec<MediaSrc> = body
        .get("mediasSrc")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .map(|m| MediaSrc {
                    media_name: m
                        .get("mediaName")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string(),
                    media_src_url: m
                        .get("mediaSrcUrl")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string(),
                    media_type: m
                        .get("mediaType")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string(),
                })
                .collect()
        })
        .unwrap_or_default();

    let attachments: Vec<Attachment> = body
        .get("attachments")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .map(|a| Attachment {
                    file_membership_id: a
                        .get("fileMembershipId")
                        .map(|v| match v {
                            serde_json::Value::Number(n) => n.to_string(),
                            serde_json::Value::String(s) => s.clone(),
                            _ => String::new(),
                        })
                        .unwrap_or_default(),
                    file_name: a
                        .get("fileName")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string(),
                })
                .collect()
        })
        .unwrap_or_default();

    let complementary_readings = body
        .get("complementaryReadings")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .map(|r| ReadingLink {
                    title: r.get("title").and_then(|v| v.as_str()).map(String::from),
                    url: r.get("url").and_then(|v| v.as_str()).map(String::from),
                })
                .collect()
        });

    tracing::info!(
        "Lição {}: {} medias, {} attachments",
        page_hash,
        medias_src.len(),
        attachments.len()
    );

    Ok(Lesson {
        content,
        has_player_media,
        medias_src,
        attachments,
        complementary_readings,
    })
}

pub async fn get_attachment_url(
    session: &HotmartSession,
    id: &str,
) -> anyhow::Result<AttachmentInfo> {
    tracing::info!("Buscando URL de download do attachment {}", id);

    let url = format!(
        "https://api-club-hot-club-api.cb.hotmart.com/rest/v3/attachment/{}/download",
        id
    );
    let resp = session
        .client
        .get(&url)
        .header("Authorization", format!("Bearer {}", session.token))
        .header("Origin", "https://consumer.hotmart.com")
        .send()
        .await?
        .error_for_status()?;

    let body: serde_json::Value = resp.json().await?;

    let download_url = body
        .get("url")
        .or_else(|| body.get("directDownloadUrl"))
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow!("URL de download não encontrada para attachment {}", id))?
        .to_string();
    let file_name = body
        .get("fileName")
        .and_then(|v| v.as_str())
        .map(String::from);

    tracing::info!("Attachment {} -> {}", id, &download_url[..60.min(download_url.len())]);

    Ok(AttachmentInfo {
        url: download_url,
        file_name,
    })
}

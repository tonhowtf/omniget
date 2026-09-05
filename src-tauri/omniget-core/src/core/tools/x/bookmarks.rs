//! Exportar favoritos (estudo 67): as tres fases do xarchive (pastas →
//! conteudo das pastas → todos os bookmarks), sem o limite de 800 da API
//! oficial. Precisa da sessao.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::{client::XClient, ProgressFn, XPost};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BookmarkFolder {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BookmarkItem {
    pub post: XPost,
    pub folders: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BookmarksResult {
    pub count: usize,
    pub folders: Vec<BookmarkFolder>,
    pub files: Vec<String>,
    pub media_files: usize,
    pub cancelled: bool,
    pub preview: Vec<BookmarkItem>,
}

pub const JOB: &str = "x-bookmarks";

fn folders_from(v: &Value) -> Vec<BookmarkFolder> {
    let mut out = Vec::new();
    fn walk(v: &Value, out: &mut Vec<BookmarkFolder>) {
        match v {
            Value::Object(m) => {
                if let (Some(id), Some(name)) = (
                    m.get("id").and_then(|x| x.as_str()),
                    m.get("name").and_then(|x| x.as_str()),
                ) {
                    if m.contains_key("media") || m.contains_key("bookmark_count") || m.len() <= 4 {
                        out.push(BookmarkFolder {
                            id: id.to_string(),
                            name: name.to_string(),
                            count: 0,
                        });
                        return;
                    }
                }
                m.values().for_each(|c| walk(c, out));
            }
            Value::Array(a) => a.iter().for_each(|c| walk(c, out)),
            _ => {}
        }
    }
    if let Some(slice) = find_key(v, "bookmark_collections_slice") {
        walk(&slice, &mut out);
    }
    out
}

fn find_key(v: &Value, key: &str) -> Option<Value> {
    match v {
        Value::Object(m) => {
            if let Some(x) = m.get(key) {
                return Some(x.clone());
            }
            m.values().find_map(|c| find_key(c, key))
        }
        Value::Array(a) => a.iter().find_map(|c| find_key(c, key)),
        _ => None,
    }
}

pub async fn folders(client: &XClient) -> anyhow::Result<Vec<BookmarkFolder>> {
    let v = client
        .gql_get("BookmarkFoldersSlice", json!({}), json!({}), None)
        .await?;
    Ok(folders_from(&v))
}

/// Lista tudo (com pastas) sem gravar nada; `max` = 0 e sem limite.
pub async fn collect(
    max: usize,
    progress: &ProgressFn,
) -> anyhow::Result<(Vec<BookmarkItem>, Vec<BookmarkFolder>, bool)> {
    let client = XClient::new()?;
    client.require_login()?;
    super::clear_cancel(JOB);
    super::report(progress, JOB, "folders", 0, None, None);
    let mut folders = folders(&client).await.unwrap_or_default();
    let mut folder_of: std::collections::HashMap<String, Vec<String>> =
        std::collections::HashMap::new();
    for f in folders.iter_mut() {
        if super::cancelled(JOB) {
            break;
        }
        super::report(progress, JOB, "folder", 0, None, Some(f.name.clone()));
        let vars =
            json!({ "bookmark_collection_id": f.id, "count": 50, "includePromotedContent": false });
        let name = f.name.clone();
        let mut ids: Vec<String> = Vec::new();
        let _ = client
            .paginate("BookmarkFolderTimeline", vars, json!({}), 0, JOB, |page| {
                let posts = super::parse::tweets_from(page);
                let n = posts.len();
                ids.extend(posts.into_iter().map(|p| p.id));
                n
            })
            .await;
        f.count = ids.len();
        for id in ids {
            folder_of.entry(id).or_default().push(name.clone());
        }
    }
    let mut posts: Vec<XPost> = Vec::new();
    let vars = json!({ "count": 100, "includePromotedContent": false, "withClientEventToken": false, "withBirdwatchNotes": false, "withVoice": true, "withV2Timeline": true });
    let p2 = progress.clone();
    client
        .paginate(
            "Bookmarks",
            vars,
            json!({ "graphql_timeline_v2_bookmark_timeline": true }),
            max,
            JOB,
            |page| {
                let got = super::parse::tweets_from(page);
                let n = got.len();
                posts.extend(got);
                super::report(&p2, JOB, "bookmarks", posts.len() as u64, None, None);
                n
            },
        )
        .await?;
    let cancelled = super::cancelled(JOB);
    let mut posts = super::dedup_posts(posts);
    if max > 0 {
        posts.truncate(max);
    }
    let items = posts
        .into_iter()
        .map(|p| BookmarkItem {
            folders: folder_of.get(&p.id).cloned().unwrap_or_default(),
            post: p,
        })
        .collect();
    Ok((items, folders, cancelled))
}

fn csv(items: &[BookmarkItem]) -> String {
    let mut out = String::from(
        "id,url,created_at,author,name,text,likes,reposts,replies,views,folders,media_urls\n",
    );
    for it in items {
        let p = &it.post;
        let row = [
            p.id.clone(),
            p.url.clone(),
            p.created_at.clone(),
            p.author.handle.clone(),
            p.author.name.clone(),
            p.text.clone(),
            p.likes.to_string(),
            p.reposts.to_string(),
            p.replies.to_string(),
            p.views.to_string(),
            it.folders.join("; "),
            p.media
                .iter()
                .map(|m| m.url.clone())
                .collect::<Vec<_>>()
                .join(" "),
        ];
        out.push_str(
            &row.iter()
                .map(|c| super::export::csv_escape(c))
                .collect::<Vec<_>>()
                .join(","),
        );
        out.push('\n');
    }
    out
}

pub async fn export(
    dest: &str,
    formats: &[String],
    with_media: bool,
    max: usize,
    progress: ProgressFn,
) -> anyhow::Result<BookmarksResult> {
    let (items, folders, cancelled) = collect(max, &progress).await?;
    let dir = std::path::Path::new(dest);
    std::fs::create_dir_all(dir)?;
    let stamp = chrono::Local::now().format("%Y-%m-%d").to_string();
    let mut files = Vec::new();
    let posts: Vec<XPost> = items.iter().map(|i| i.post.clone()).collect();
    for f in formats {
        let path = dir.join(format!(
            "x-bookmarks-{}.{}",
            stamp,
            super::export::ext_for(f)
        ));
        let written = match f.as_str() {
            "json" => {
                std::fs::write(&path, serde_json::to_string_pretty(&items)?)?;
                path.to_string_lossy().to_string()
            }
            "csv" => {
                std::fs::write(&path, csv(&items))?;
                path.to_string_lossy().to_string()
            }
            other => super::export::write_posts(&posts, other, &path, "Favoritos do X")?,
        };
        files.push(written);
    }
    let mut media_files = 0;
    if with_media && !cancelled {
        let r =
            super::media::download_posts(&posts, &dir.join("media"), true, true, JOB, &progress)
                .await?;
        media_files = r.files.len();
    }
    super::report(
        &progress,
        JOB,
        "done",
        items.len() as u64,
        Some(items.len() as u64),
        None,
    );
    Ok(BookmarksResult {
        count: items.len(),
        folders,
        files,
        media_files,
        cancelled,
        preview: items.into_iter().take(60).collect(),
    })
}

use serde::Deserialize;
use serde_json::{json, Value};

use super::{Hit, Page, RemoteChapter, RemotePage};

pub struct Conn {
    pub base: String,
    user: String,
    pass: String,
    http: reqwest::Client,
}

#[derive(Debug, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct SourceNode {
    pub id: String,
    pub display_name: String,
    pub lang: String,
    #[serde(default)]
    pub icon_url: String,
    #[serde(default)]
    pub supports_latest: bool,
    #[serde(default)]
    pub is_nsfw: bool,
}

impl Conn {
    pub fn new(base: &str, user: &str, pass: &str) -> Self {
        Self {
            base: base.trim().trim_end_matches('/').to_string(),
            user: user.to_string(),
            pass: pass.to_string(),
            http: super::client(),
        }
    }

    fn with_auth(&self, r: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        if self.user.is_empty() {
            r
        } else {
            r.basic_auth(&self.user, Some(&self.pass))
        }
    }

    pub async fn about(&self) -> Result<String, String> {
        let r = self
            .with_auth(
                self.http
                    .get(format!("{}/api/v1/settings/about/", self.base)),
            )
            .timeout(std::time::Duration::from_secs(4))
            .send()
            .await
            .map_err(|e| format!("Suwayomi not reachable at {}: {e}", self.base))?;
        if r.status() == reqwest::StatusCode::UNAUTHORIZED {
            return Err("Suwayomi asks for a login".into());
        }
        let v: Value = r.json().await.map_err(|e| e.to_string())?;
        Ok(v["version"].as_str().unwrap_or("").to_string())
    }

    async fn gql(&self, query: &str, vars: Value) -> Result<Value, String> {
        let r = self
            .with_auth(self.http.post(format!("{}/api/graphql", self.base)))
            .json(&json!({ "query": query, "variables": vars }))
            .send()
            .await
            .map_err(|e| e.to_string())?;
        if !r.status().is_success() {
            return Err(format!("Suwayomi: {}", r.status()));
        }
        let v: Value = r.json().await.map_err(|e| e.to_string())?;
        if let Some(e) = v["errors"].as_array().and_then(|a| a.first()) {
            return Err(format!(
                "Suwayomi: {}",
                e["message"].as_str().unwrap_or("error")
            ));
        }
        Ok(v["data"].clone())
    }

    fn abs(&self, path: &str) -> String {
        if path.starts_with("http") {
            return path.to_string();
        }
        let mut u = match url::Url::parse(&self.base).and_then(|b| b.join(path)) {
            Ok(u) => u,
            Err(_) => return format!("{}{path}", self.base),
        };
        if !self.user.is_empty() {
            let _ = u.set_username(&self.user);
            let _ = u.set_password(Some(&self.pass));
        }
        u.to_string()
    }

    pub async fn sources(&self) -> Result<Vec<SourceNode>, String> {
        let d = self
            .gql(
                "query { sources { nodes { id displayName lang iconUrl supportsLatest isNsfw } } }",
                json!({}),
            )
            .await?;
        let mut list: Vec<SourceNode> =
            serde_json::from_value(d["sources"]["nodes"].clone()).map_err(|e| e.to_string())?;
        list.retain(|s| s.id != "0");
        for s in &mut list {
            s.icon_url = self.abs(&s.icon_url);
        }
        Ok(list)
    }

    pub async fn browse(
        &self,
        source: &str,
        mode: &str,
        query: &str,
        page: u32,
    ) -> Result<Page, String> {
        let kind = match mode {
            "latest" => "LATEST",
            "search" => "SEARCH",
            _ => "POPULAR",
        };
        let d = self
            .gql(
                "mutation($source: LongString!, $type: FetchSourceMangaType!, $page: Int!, $query: String) {
                   fetchSourceManga(input: { source: $source, type: $type, page: $page, query: $query }) {
                     hasNextPage
                     mangas { id title thumbnailUrl author description genre status realUrl }
                   }
                 }",
                json!({ "source": source, "type": kind, "page": page.max(1), "query": query }),
            )
            .await?;
        let r = &d["fetchSourceManga"];
        let hits = r["mangas"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|m| self.hit(m))
            .collect();
        Ok(Page {
            hits,
            total: None,
            next: r["hasNextPage"]
                .as_bool()
                .unwrap_or(false)
                .then(|| (page.max(1) + 1).to_string()),
        })
    }

    fn hit(&self, m: &Value) -> Hit {
        let s = |k: &str| m[k].as_str().unwrap_or("").to_string();
        Hit {
            source: "suwayomi".into(),
            key: m["id"].to_string(),
            title: s("title"),
            authors: s("author")
                .split(',')
                .map(str::trim)
                .filter(|a| !a.is_empty())
                .map(str::to_string)
                .collect(),
            cover: m["thumbnailUrl"].as_str().map(|t| self.abs(t)),
            summary: s("description"),
            language: String::new(),
            year: None,
            subjects: m["genre"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|g| g.as_str().map(str::to_string))
                .collect(),
            kind: "series".into(),
            downloads: vec![],
            url: s("realUrl"),
            extra: json!({ "status": s("status") }),
        }
    }

    pub async fn details(&self, manga_id: &str) -> Result<(Hit, Vec<RemoteChapter>), String> {
        let id: i64 = manga_id.parse().map_err(|_| "bad manga id")?;
        let d = self
            .gql(
                "mutation($id: Int!) {
                   fetchManga(input: { id: $id }) { manga { id title author artist description genre status thumbnailUrl realUrl } }
                   fetchChapters(input: { mangaId: $id }) {
                     chapters { id name chapterNumber scanlator uploadDate sourceOrder pageCount realUrl }
                   }
                 }",
                json!({ "id": id }),
            )
            .await?;
        let hit = self.hit(&d["fetchManga"]["manga"]);
        let mut chapters: Vec<(i64, RemoteChapter)> = d["fetchChapters"]["chapters"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|c| {
                (
                    c["sourceOrder"].as_i64().unwrap_or(0),
                    RemoteChapter {
                        key: c["id"].to_string(),
                        title: c["name"].as_str().unwrap_or("").to_string(),
                        number: c["chapterNumber"].as_f64().filter(|n| *n >= 0.0),
                        volume: None,
                        language: String::new(),
                        scanlator: c["scanlator"].as_str().unwrap_or("").to_string(),
                        published: c["uploadDate"]
                            .as_str()
                            .and_then(|s| s.parse().ok())
                            .or_else(|| c["uploadDate"].as_i64()),
                        pages: c["pageCount"].as_u64().map(|p| p as u32).filter(|p| *p > 0),
                        external_url: None,
                    },
                )
            })
            .collect();
        chapters.sort_by_key(|c| std::cmp::Reverse(c.0));
        Ok((hit, chapters.into_iter().map(|(_, c)| c).collect()))
    }

    pub async fn pages(&self, chapter_id: &str) -> Result<Vec<RemotePage>, String> {
        let id: i64 = chapter_id.parse().map_err(|_| "bad chapter id")?;
        let d = self
            .gql(
                "mutation($id: Int!) { fetchChapterPages(input: { chapterId: $id }) { pages } }",
                json!({ "id": id }),
            )
            .await?;
        Ok(d["fetchChapterPages"]["pages"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|p| p.as_str())
            .map(|p| RemotePage {
                url: self.abs(p),
                ..Default::default()
            })
            .collect())
    }

    pub async fn add_repo(&self, index_url: &str) -> Result<(), String> {
        self.gql(
            "mutation($url: String!) { addExtensionStore(input: { indexUrl: $url }) { clientMutationId } }",
            json!({ "url": index_url }),
        )
        .await
        .map(|_| ())
    }

    pub async fn extensions(&self) -> Result<Vec<Value>, String> {
        let d = self
            .gql(
                "mutation { fetchExtensions(input: {}) { extensions { pkgName name lang versionName isInstalled hasUpdate isObsolete isNsfw iconUrl } } }",
                json!({}),
            )
            .await?;
        Ok(d["fetchExtensions"]["extensions"]
            .as_array()
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .map(|mut e| {
                if let Some(i) = e["iconUrl"].as_str() {
                    e["iconUrl"] = Value::String(self.abs(i));
                }
                e
            })
            .collect())
    }

    pub async fn extension(&self, pkg: &str, action: &str) -> Result<(), String> {
        let patch = match action {
            "install" => json!({ "install": true }),
            "update" => json!({ "update": true }),
            _ => json!({ "uninstall": true }),
        };
        self.gql(
            "mutation($id: String!, $patch: UpdateExtensionPatchInput!) { updateExtension(input: { id: $id, patch: $patch }) { clientMutationId } }",
            json!({ "id": pkg, "patch": patch }),
        )
        .await
        .map(|_| ())
    }
}

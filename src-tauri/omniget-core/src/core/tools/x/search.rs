//! Busca avancada e trends (estudo 67). A query e montada na UI com os
//! operadores do X (`from:`, `since:`, `min_faves:`, `filter:media`…); aqui
//! ela roda no FxTwitter (publico) e, se a sessao existir e o FxTwitter
//! falhar, no `SearchTimeline` do GraphQL.

use serde::{Deserialize, Serialize};
use serde_json::json;

use super::XPost;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchPage {
    pub posts: Vec<XPost>,
    pub cursor: Option<String>,
    pub source: String,
}

/// `feed`: "latest" | "top".
pub async fn search(query: &str, feed: &str, cursor: Option<&str>) -> anyhow::Result<SearchPage> {
    let q = query.trim();
    if q.is_empty() {
        anyhow::bail!("busca vazia");
    }
    let feed = if feed == "top" { "top" } else { "latest" };
    match super::fx::search(q, feed, cursor).await {
        Ok(page) => Ok(SearchPage {
            posts: page.items,
            cursor: page.cursor,
            source: "fxtwitter".into(),
        }),
        Err(fx_err) => {
            let client = super::client::XClient::new()?;
            if !client.authed() {
                return Err(fx_err);
            }
            let mut vars = json!({
                "rawQuery": q,
                "count": 40,
                "querySource": "typed_query",
                "product": if feed == "top" { "Top" } else { "Latest" },
                "withGrokTranslatedBio": true
            });
            if let Some(c) = cursor {
                vars["cursor"] = json!(c);
            }
            let v = client
                .gql_get(
                    "SearchTimeline",
                    vars,
                    json!({}),
                    Some(json!({"withArticleRichContentState": false})),
                )
                .await?;
            Ok(SearchPage {
                posts: super::parse::tweets_from(&v),
                cursor: super::parse::bottom_cursor(&v),
                source: "graphql".into(),
            })
        }
    }
}

pub async fn trends() -> anyhow::Result<Vec<super::fx::Trend>> {
    super::fx::trends().await
}

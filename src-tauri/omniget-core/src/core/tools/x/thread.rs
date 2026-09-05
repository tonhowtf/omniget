//! Desenrolar thread (estudo 67): o que o Thread Reader App faz. Via
//! publica pelo FxTwitter `/2/thread/{id}`; se falhar, `TweetDetail` da
//! sessao e a reconstrucao do XActions (`thread.js`): posts do mesmo autor
//! ligados por `in_reply_to`, seguindo os cursores de "mostrar mais".

use anyhow::anyhow;
use serde::{Deserialize, Serialize};
use serde_json::json;

use super::{client::XClient, XPost};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Thread {
    pub focal: XPost,
    pub posts: Vec<XPost>,
    pub truncated: bool,
    /// "fxtwitter" | "graphql"
    pub source: String,
}

impl Thread {
    pub fn title(&self) -> String {
        let first = self.posts.first().unwrap_or(&self.focal);
        let line = first.text.lines().next().unwrap_or("").trim();
        let short: String = line.chars().take(80).collect();
        if short.is_empty() {
            format!("Thread de @{}", first.author.handle)
        } else {
            short
        }
    }
}

pub async fn unroll(input: &str) -> anyhow::Result<Thread> {
    let id = super::post_id_from(input).ok_or_else(|| anyhow!("nao reconheci um post do X em: {}", input))?;
    match super::fx::thread(&id).await {
        Ok((focal, posts, truncated)) => {
            let posts = if posts.is_empty() { vec![focal.clone()] } else { posts };
            Ok(Thread { focal, posts, truncated, source: "fxtwitter".into() })
        }
        Err(fx_err) => {
            tracing::info!("[x] fxtwitter falhou ({}), tentando GraphQL", fx_err);
            unroll_graphql(&id).await.map_err(|e| anyhow!("{} / {}", fx_err, e))
        }
    }
}

async fn unroll_graphql(id: &str) -> anyhow::Result<Thread> {
    let client = XClient::new()?;
    let mut all: Vec<XPost> = Vec::new();
    let mut cursor: Option<String> = None;
    for _ in 0..4 {
        let mut vars = json!({
            "focalTweetId": id,
            "with_rux_injections": false,
            "rankingMode": "Relevance",
            "includePromotedContent": false,
            "withCommunity": true,
            "withQuickPromoteEligibilityTweetFields": true,
            "withBirdwatchNotes": true,
            "withVoice": true,
            "withV2Timeline": true
        });
        if let Some(c) = &cursor {
            vars["cursor"] = json!(c);
        }
        let v = client
            .gql_get("TweetDetail", vars, json!({}), Some(json!({"withArticleRichContentState": true, "withArticlePlainText": false, "withGrokAnalyze": false, "withDisallowedReplyControls": false})))
            .await?;
        all.extend(super::parse::tweets_from(&v));
        // cursores de "mostrar mais desta conversa"
        let next = find_show_more(&v);
        if next.is_none() || Some(&next.clone().unwrap()) == cursor.as_ref() {
            break;
        }
        cursor = next;
    }
    let all = super::dedup_posts(all);
    let focal = all.iter().find(|p| p.id == id).cloned().ok_or_else(|| anyhow!("post indisponivel"))?;
    let author = focal.author.handle.to_ascii_lowercase();
    let mut by_id: std::collections::HashMap<String, XPost> = all.iter().filter(|p| p.author.handle.to_ascii_lowercase() == author).map(|p| (p.id.clone(), p.clone())).collect();
    // sobe ate a raiz
    let mut root = focal.clone();
    while let Some(parent) = root.reply_to_id.clone().and_then(|pid| by_id.get(&pid).cloned()) {
        root = parent;
    }
    // desce pela cadeia de respostas do autor
    let mut chain = vec![root.clone()];
    by_id.remove(&root.id);
    loop {
        let last = chain.last().unwrap().id.clone();
        let next = by_id.values().filter(|p| p.reply_to_id.as_deref() == Some(last.as_str())).min_by_key(|p| p.timestamp).cloned();
        match next {
            Some(p) => {
                by_id.remove(&p.id);
                chain.push(p);
            }
            None => break,
        }
    }
    if !chain.iter().any(|p| p.id == focal.id) {
        chain = vec![focal.clone()];
    }
    Ok(Thread { focal, posts: chain, truncated: false, source: "graphql".into() })
}

fn find_show_more(v: &serde_json::Value) -> Option<String> {
    use serde_json::Value;
    fn walk(v: &Value) -> Option<String> {
        match v {
            Value::Object(m) => {
                let ct = m.get("cursorType").and_then(|c| c.as_str()).unwrap_or("");
                if ct == "ShowMoreThreads" || ct == "ShowMoreThreadsPrompt" || ct == "Bottom" {
                    if let Some(val) = m.get("value").and_then(|x| x.as_str()) {
                        return Some(val.to_string());
                    }
                }
                m.values().find_map(walk)
            }
            Value::Array(a) => a.iter().find_map(walk),
            _ => None,
        }
    }
    walk(v)
}

//! Comentários, curtidas, sorteio e hashtags.

use std::collections::HashSet;
use std::sync::atomic::AtomicBool;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::follow::MiniUser;
use super::media::{parse_item, MediaItem};
use super::{b, s, u, IgClient, IgError};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Comment {
    pub pk: String,
    pub text: String,
    pub created_at: i64,
    pub user: MiniUser,
    pub like_count: u64,
    pub reply_count: u64,
    pub mentions: Vec<String>,
}

fn parse_comment(v: &Value) -> Comment {
    let text = s(v, "text");
    let mentions = text
        .split_whitespace()
        .filter_map(|w| w.strip_prefix('@'))
        .map(|w| {
            w.trim_end_matches(|c: char| !c.is_alphanumeric() && c != '_' && c != '.')
                .to_lowercase()
        })
        .filter(|w| !w.is_empty())
        .collect();
    Comment {
        pk: s(v, "pk"),
        created_at: v
            .get("created_at_utc")
            .or(v.get("created_at"))
            .and_then(|x| x.as_i64())
            .unwrap_or(0),
        user: v.get("user").map(MiniUser::from_value).unwrap_or_default(),
        like_count: u(v, "comment_like_count"),
        reply_count: u(v, "child_comment_count"),
        mentions,
        text,
    }
}

/// `GET /api/v1/media/{pk}/comments/` paginado por `min_id`.
pub async fn comments(
    client: &IgClient,
    media_pk: &str,
    limit: usize,
    flag: &AtomicBool,
    progress: &super::super::ProgressFn,
    job: &str,
) -> Result<Vec<Comment>, IgError> {
    let mut out = Vec::new();
    let mut min_id: Option<String> = None;
    let id = format!("ig:{}", job);
    let mut seen: HashSet<String> = HashSet::new();
    loop {
        let mut q = vec![
            ("can_support_threading", "true".to_string()),
            ("permalink_enabled", "false".to_string()),
        ];
        if let Some(m) = &min_id {
            q.push(("min_id", m.clone()));
        }
        let json = client
            .get_json(&format!("/api/v1/media/{}/comments/", media_pk), &q)
            .await?;
        let list = json
            .get("comments")
            .and_then(|c| c.as_array())
            .cloned()
            .unwrap_or_default();
        let mut added = 0;
        for c in &list {
            let cm = parse_comment(c);
            if seen.insert(cm.pk.clone()) {
                out.push(cm);
                added += 1;
            }
        }
        super::super::report(
            progress,
            &id,
            "comments",
            out.len() as u64,
            if limit > 0 {
                Some(limit as u64)
            } else {
                u(&json, "comment_count").checked_sub(0)
            },
            None,
        );
        min_id = json.get("next_min_id").and_then(|m| match m {
            Value::String(x) => Some(x.clone()),
            Value::Number(n) => Some(n.to_string()),
            _ => None,
        });
        let more = b(&json, "has_more_comments") || b(&json, "has_more_headload_comments");
        if added == 0
            || !more
            || min_id.is_none()
            || (limit > 0 && out.len() >= limit)
            || super::cancelled(flag)
        {
            break;
        }
        client.pause().await;
    }
    if limit > 0 {
        out.truncate(limit);
    }
    Ok(out)
}

/// `GET /api/v1/media/{pk}/likers/` (uma resposta só, até ~1000 contas).
pub async fn likers(client: &IgClient, media_pk: &str) -> Result<(u64, Vec<MiniUser>), IgError> {
    let json = client
        .get_json(&format!("/api/v1/media/{}/likers/", media_pk), &[])
        .await?;
    let users = json
        .get("users")
        .and_then(|u| u.as_array())
        .map(|a| a.iter().map(MiniUser::from_value).collect())
        .unwrap_or_default();
    Ok((u(&json, "user_count"), users))
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct GiveawayRules {
    pub winners: usize,
    /// Um comentário por pessoa (a mais recente conta uma vez).
    pub unique_users: bool,
    /// Precisa mencionar pelo menos N contas.
    pub min_mentions: usize,
    /// Palavra/hashtag obrigatória (vazio = nenhuma).
    pub keyword: String,
    /// Não deixar o dono do post nem estas contas ganharem.
    pub exclude: Vec<String>,
    pub owner_username: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct GiveawayResult {
    pub eligible: usize,
    pub winners: Vec<Comment>,
    pub seed: u64,
}

pub fn giveaway(comments: &[Comment], rules: &GiveawayRules) -> GiveawayResult {
    let keyword = rules.keyword.trim().to_lowercase();
    let excluded: HashSet<String> = rules
        .exclude
        .iter()
        .map(|e| e.trim_start_matches('@').to_lowercase())
        .chain(std::iter::once(rules.owner_username.to_lowercase()))
        .filter(|e| !e.is_empty())
        .collect();
    let mut seen_users: HashSet<String> = HashSet::new();
    let mut pool: Vec<Comment> = Vec::new();
    for c in comments {
        let uname = c.user.username.to_lowercase();
        if excluded.contains(&uname) {
            continue;
        }
        if c.mentions.len() < rules.min_mentions {
            continue;
        }
        if !keyword.is_empty() && !c.text.to_lowercase().contains(&keyword) {
            continue;
        }
        if rules.unique_users && !seen_users.insert(uname) {
            continue;
        }
        pool.push(c.clone());
    }
    let eligible = pool.len();
    let seed = rand::random::<u64>();
    // Fisher-Yates com um LCG determinístico a partir da semente, para o
    // resultado poder ser reproduzido pela semente exibida.
    let mut state = seed | 1;
    let mut next = move || {
        state = state
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        state >> 33
    };
    for i in (1..pool.len()).rev() {
        let j = (next() % (i as u64 + 1)) as usize;
        pool.swap(i, j);
    }
    pool.truncate(rules.winners.max(1));
    GiveawayResult {
        eligible,
        winners: pool,
        seed,
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct TagInfo {
    pub name: String,
    pub media_count: u64,
    pub formatted_media_count: String,
    pub profile_pic_url: String,
    pub following: bool,
}

/// `GET /api/v1/tags/web_info/?tag_name=`
pub async fn tag_info(client: &IgClient, tag: &str) -> Result<TagInfo, IgError> {
    let tag = tag.trim().trim_start_matches('#').to_lowercase();
    let json = client
        .get_json("/api/v1/tags/web_info/", &[("tag_name", tag.clone())])
        .await?;
    let d = json.get("data").cloned().unwrap_or(json.clone());
    Ok(TagInfo {
        name: {
            let n = s(&d, "name");
            if n.is_empty() {
                tag
            } else {
                n
            }
        },
        media_count: u(&d, "media_count"),
        formatted_media_count: s(&d, "formatted_media_count"),
        profile_pic_url: s(&d, "profile_pic_url"),
        following: b(&d, "following"),
    })
}

/// `POST /api/v1/tags/{tag}/sections/` — `tab` = "recent" | "top".
pub async fn tag_media(
    client: &IgClient,
    tag: &str,
    tab: &str,
    limit: usize,
    flag: &AtomicBool,
    progress: &super::super::ProgressFn,
    job: &str,
) -> Result<Vec<MediaItem>, IgError> {
    let tag = tag.trim().trim_start_matches('#').to_lowercase();
    let mut out = Vec::new();
    let mut max_id: Option<String> = None;
    let mut page: Option<String> = None;
    let id = format!("ig:{}", job);
    loop {
        let mut form = vec![
            ("include_persistent", "0".to_string()),
            ("surface", "grid".to_string()),
            ("tab", tab.to_string()),
        ];
        if let Some(m) = &max_id {
            form.push(("max_id", m.clone()));
        }
        if let Some(p) = &page {
            form.push(("page", p.clone()));
        }
        let json = client
            .post_form(&format!("/api/v1/tags/{}/sections/", tag), &form)
            .await?;
        let sections = json
            .get("sections")
            .and_then(|s| s.as_array())
            .cloned()
            .unwrap_or_default();
        let mut added = 0;
        for sec in &sections {
            for m in sec
                .get("layout_content")
                .and_then(|l| l.get("medias"))
                .and_then(|m| m.as_array())
                .cloned()
                .unwrap_or_default()
            {
                if let Some(item) = m.get("media").and_then(parse_item) {
                    out.push(item);
                    added += 1;
                }
            }
        }
        super::super::report(
            progress,
            &id,
            "list",
            out.len() as u64,
            if limit > 0 { Some(limit as u64) } else { None },
            None,
        );
        max_id = json
            .get("next_max_id")
            .and_then(|m| m.as_str())
            .map(|m| m.to_string());
        page = json.get("next_page").map(|p| match p {
            Value::String(x) => x.clone(),
            other => other.to_string(),
        });
        if added == 0
            || !b(&json, "more_available")
            || max_id.is_none()
            || (limit > 0 && out.len() >= limit)
            || super::cancelled(flag)
        {
            break;
        }
        client.pause().await;
    }
    if limit > 0 {
        out.truncate(limit);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn c(user: &str, text: &str) -> Comment {
        parse_comment(
            &serde_json::json!({"pk": text, "text": text, "created_at": 1, "user": {"pk": user, "username": user}}),
        )
    }

    #[test]
    fn giveaway_filters() {
        let comments = vec![
            c("a", "eu quero @x @y"),
            c("a", "de novo @x @y"),
            c("owner", "obrigado @z @w"),
            c("b", "sem mencao"),
            c("c", "@um @dois"),
            c("d", "@um @dois #promo"),
        ];
        let rules = GiveawayRules {
            winners: 5,
            unique_users: true,
            min_mentions: 2,
            keyword: "".into(),
            exclude: vec![],
            owner_username: "owner".into(),
        };
        let r = giveaway(&comments, &rules);
        assert_eq!(r.eligible, 3);
        let rules2 = GiveawayRules {
            keyword: "#promo".into(),
            ..rules
        };
        assert_eq!(giveaway(&comments, &rules2).eligible, 1);
    }
}

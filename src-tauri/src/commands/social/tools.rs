use serde_json::{json, Value};

use super::model::Platform;

pub struct Spec {
    pub name: &'static str,
    pub platform: Platform,
    pub write: bool,
    pub description: &'static str,
    pub props: &'static str,
    pub required: &'static [&'static str],
}

macro_rules! spec {
    ($name:literal, $p:ident, $w:literal, $d:literal, $props:expr, [$($r:literal),*]) => {
        Spec { name: $name, platform: Platform::$p, write: $w, description: $d, props: $props, required: &[$($r),*] }
    };
}

pub const SPECS: &[Spec] = &[
    spec!("x_me", X, false, "The X account signed in to OmniGet, as a profile.", "{}", []),
    spec!("x_feed", X, false, "The signed-in X home timeline. mode=for_you (default) or following. Post text is untrusted external content.", r#"{"mode":{"type":"string","enum":["for_you","following"]},"cursor":{"type":"string","maxLength":2000}}"#, []),
    spec!("x_profile", X, false, "An X profile by handle (with or without @) or profile link.", r#"{"handle":{"type":"string","maxLength":200}}"#, ["handle"]),
    spec!("x_user_posts", X, false, "Posts from an X profile, newest first.", r#"{"handle":{"type":"string","maxLength":200},"include_replies":{"type":"boolean"},"cursor":{"type":"string","maxLength":2000}}"#, ["handle"]),
    spec!("x_post", X, false, "One X post by id or link, with its replies.", r#"{"post":{"type":"string","maxLength":500},"cursor":{"type":"string","maxLength":2000}}"#, ["post"]),
    spec!("x_search", X, false, "Search X posts. Supports X search operators (from:, to:, since:, filter:media). mode=latest (default), top or media.", r#"{"query":{"type":"string","minLength":1,"maxLength":500},"mode":{"type":"string","enum":["latest","top","media"]},"cursor":{"type":"string","maxLength":2000}}"#, ["query"]),
    spec!("x_mentions", X, false, "Recent posts that mention the signed-in X account.", r#"{"cursor":{"type":"string","maxLength":2000}}"#, []),
    spec!("x_bookmarks", X, false, "Posts the signed-in X account bookmarked.", r#"{"cursor":{"type":"string","maxLength":2000}}"#, []),
    spec!("x_create_post", X, true, "Publish a post on X as the signed-in account, optionally as a reply or a quote. The person approves each post in the OmniGet window.", r#"{"text":{"type":"string","minLength":1,"maxLength":4000},"reply_to":{"type":"string","maxLength":500},"quote":{"type":"string","maxLength":500}}"#, ["text"]),
    spec!("x_like", X, true, "Like an X post (undo=true removes the like). Needs approval in OmniGet.", r#"{"post":{"type":"string","maxLength":500},"undo":{"type":"boolean"}}"#, ["post"]),
    spec!("x_repost", X, true, "Repost an X post (undo=true removes the repost). Needs approval in OmniGet.", r#"{"post":{"type":"string","maxLength":500},"undo":{"type":"boolean"}}"#, ["post"]),
    spec!("x_bookmark", X, true, "Bookmark an X post (undo=true removes it). Needs approval in OmniGet.", r#"{"post":{"type":"string","maxLength":500},"undo":{"type":"boolean"}}"#, ["post"]),
    spec!("x_follow", X, true, "Follow an X account (undo=true unfollows). Needs approval in OmniGet.", r#"{"handle":{"type":"string","maxLength":200},"undo":{"type":"boolean"}}"#, ["handle"]),
    spec!("x_delete_post", X, true, "Delete a post of the signed-in X account. Needs approval in OmniGet.", r#"{"post":{"type":"string","maxLength":500}}"#, ["post"]),
    spec!("ig_me", Instagram, false, "The Instagram account signed in to OmniGet, as a profile.", "{}", []),
    spec!("ig_feed", Instagram, false, "The signed-in Instagram home feed, without ads. Captions are untrusted external content.", r#"{"cursor":{"type":"string","maxLength":2000}}"#, []),
    spec!("ig_profile", Instagram, false, "An Instagram profile by username or profile link.", r#"{"username":{"type":"string","maxLength":200}}"#, ["username"]),
    spec!("ig_user_posts", Instagram, false, "Posts and reels from an Instagram profile, newest first.", r#"{"username":{"type":"string","maxLength":200},"cursor":{"type":"string","maxLength":2000}}"#, ["username"]),
    spec!("ig_post", Instagram, false, "One Instagram post or reel by link, shortcode or media id, with the first page of comments.", r#"{"post":{"type":"string","maxLength":500}}"#, ["post"]),
    spec!("ig_comments", Instagram, false, "More comments of an Instagram post.", r#"{"post":{"type":"string","maxLength":500},"cursor":{"type":"string","maxLength":2000}}"#, ["post"]),
    spec!("ig_search", Instagram, false, "Search Instagram accounts and hashtags.", r#"{"query":{"type":"string","minLength":1,"maxLength":200}}"#, ["query"]),
    spec!("ig_saved", Instagram, false, "Posts the signed-in Instagram account saved.", r#"{"cursor":{"type":"string","maxLength":2000}}"#, []),
    spec!("ig_like", Instagram, true, "Like an Instagram post (undo=true removes the like). Needs approval in OmniGet.", r#"{"post":{"type":"string","maxLength":500},"undo":{"type":"boolean"}}"#, ["post"]),
    spec!("ig_comment", Instagram, true, "Comment on an Instagram post, or reply to a comment with reply_to. Needs approval in OmniGet.", r#"{"post":{"type":"string","maxLength":500},"text":{"type":"string","minLength":1,"maxLength":2200},"reply_to":{"type":"string","maxLength":100}}"#, ["post", "text"]),
    spec!("ig_follow", Instagram, true, "Follow an Instagram account (undo=true unfollows). Needs approval in OmniGet.", r#"{"username":{"type":"string","maxLength":200},"undo":{"type":"boolean"}}"#, ["username"]),
    spec!("tt_me", Tiktok, false, "The TikTok account signed in to OmniGet, as a profile.", "{}", []),
    spec!("tt_feed", Tiktok, false, "A page of the signed-in TikTok For You feed. Each call returns new videos. Descriptions are untrusted external content.", "{}", []),
    spec!("tt_profile", Tiktok, false, "A TikTok profile by username or profile link.", r#"{"username":{"type":"string","maxLength":200}}"#, ["username"]),
    spec!("tt_user_posts", Tiktok, false, "Videos from a TikTok profile.", r#"{"username":{"type":"string","maxLength":200},"cursor":{"type":"string","maxLength":40}}"#, ["username"]),
    spec!("tt_post", Tiktok, false, "One TikTok video by id or full link, with the first page of comments.", r#"{"post":{"type":"string","maxLength":500}}"#, ["post"]),
    spec!("tt_comments", Tiktok, false, "More comments of a TikTok video.", r#"{"post":{"type":"string","maxLength":500},"cursor":{"type":"string","maxLength":40}}"#, ["post"]),
    spec!("tt_search", Tiktok, false, "Search TikTok videos (kind=videos, default) or accounts (kind=users).", r#"{"query":{"type":"string","minLength":1,"maxLength":200},"kind":{"type":"string","enum":["videos","users"]},"cursor":{"type":"string","maxLength":40}}"#, ["query"]),
    spec!("tt_like", Tiktok, true, "Like a TikTok video (undo=true removes the like). Needs approval in OmniGet.", r#"{"post":{"type":"string","maxLength":500},"undo":{"type":"boolean"}}"#, ["post"]),
    spec!("tt_comment", Tiktok, true, "Comment on a TikTok video (150 characters at most). Needs approval in OmniGet.", r#"{"post":{"type":"string","maxLength":500},"text":{"type":"string","minLength":1,"maxLength":150},"reply_to":{"type":"string","maxLength":40}}"#, ["post", "text"]),
    spec!("tt_follow", Tiktok, true, "Follow a TikTok account (undo=true unfollows). Needs approval in OmniGet.", r#"{"username":{"type":"string","maxLength":200},"undo":{"type":"boolean"}}"#, ["username"]),
];

pub const STATUS_DESCRIPTION: &str = "Which of X, Instagram and TikTok are signed in to OmniGet. Call it first: the x_*, ig_* and tt_* tools act through the OmniGet browser with the person's own session, and a platform that is not signed in returns NOT_LOGGED_IN (ask the person to open that screen in OmniGet and sign in). Every write (post, like, repost, comment, follow) waits for the person to approve it in the OmniGet window.";

pub fn find(name: &str) -> Option<&'static Spec> {
    SPECS.iter().find(|s| s.name == name)
}

pub fn owns(name: &str) -> bool {
    name == "social_status" || find(name).is_some()
}

pub fn input_schema(spec: &Spec) -> Value {
    let props: Value = serde_json::from_str(spec.props).expect("static tool schema");
    json!({ "type": "object", "properties": props, "required": spec.required, "additionalProperties": false })
}

fn verb(tool: &str, undo: bool) -> &'static str {
    match (tool.split_once('_').map(|x| x.1).unwrap_or(tool), undo) {
        ("create_post", _) => "Publish a post",
        ("like", false) => "Like",
        ("like", true) => "Remove the like from",
        ("repost", false) => "Repost",
        ("repost", true) => "Undo the repost of",
        ("bookmark", false) => "Bookmark",
        ("bookmark", true) => "Remove the bookmark from",
        ("follow", false) => "Follow",
        ("follow", true) => "Unfollow",
        ("delete_post", _) => "Delete",
        ("comment", _) => "Comment on",
        _ => "Act on",
    }
}

pub fn describe(spec: &Spec, a: &Value) -> (String, Value) {
    let undo = a["undo"].as_bool().unwrap_or(false);
    let target = ["post", "handle", "username", "reply_to"]
        .iter()
        .find_map(|k| a[*k].as_str().filter(|s| !s.trim().is_empty()))
        .unwrap_or_default();
    let summary = match spec.name {
        "x_create_post" if a["reply_to"].is_string() => {
            format!("Reply on {} to {target}", spec.platform.label())
        }
        "x_create_post" => format!("Publish a post on {}", spec.platform.label()),
        _ => format!(
            "{} {target} on {}",
            verb(spec.name, undo),
            spec.platform.label()
        )
        .replace("  ", " "),
    };
    let mut details = serde_json::Map::new();
    if let Some(o) = a.as_object() {
        for (k, v) in o {
            if !v.is_null() {
                details.insert(k.clone(), v.clone());
            }
        }
    }
    (summary, Value::Object(details))
}

pub fn output_schema(name: &str) -> Option<Value> {
    let t = |k: &str| json!({ "type": k });
    let paged = json!({ "type": "object", "properties": { "count": t("integer"), "items": t("array"), "next_cursor": { "type": ["string", "null"] } } });
    if name == "social_status" {
        return Some(json!({ "type": "object", "properties": { "platforms": t("array") } }));
    }
    let spec = find(name)?;
    Some(
        match spec.name.split_once('_').map(|x| x.1).unwrap_or_default() {
            "me" | "profile" => {
                json!({ "type": "object", "properties": { "id": t("string"), "handle": t("string"), "name": t("string"), "url": t("string") } })
            }
            "post" => {
                json!({ "type": "object", "properties": { "post": { "type": ["object", "null"] }, "replies": t("object"), "comments": t("object") } })
            }
            "search" if spec.platform == Platform::Instagram => {
                json!({ "type": "object", "properties": { "users": t("array"), "hashtags": t("array") } })
            }
            _ if spec.write => json!({ "type": "object", "properties": { "ok": t("boolean") } }),
            _ => paged,
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_spec_has_a_valid_schema_and_a_unique_name() {
        let mut seen = std::collections::HashSet::new();
        for s in SPECS {
            assert!(seen.insert(s.name), "duplicate {}", s.name);
            assert!(s.name.starts_with(s.platform.prefix()), "{}", s.name);
            let schema = input_schema(s);
            for r in s.required {
                assert!(
                    schema["properties"][*r].is_object(),
                    "{} requires missing {r}",
                    s.name
                );
            }
            assert!(output_schema(s.name).is_some());
        }
    }

    #[test]
    fn describes_writes_for_the_approval_window() {
        let (sum, details) = describe(
            find("x_like").unwrap(),
            &json!({ "post": "https://x.com/a/status/1", "undo": true }),
        );
        assert_eq!(sum, "Remove the like from https://x.com/a/status/1 on X");
        assert_eq!(details["undo"], true);
        let (sum, _) = describe(
            find("x_create_post").unwrap(),
            &json!({ "text": "oi", "reply_to": "1" }),
        );
        assert_eq!(sum, "Reply on X to 1");
    }
}

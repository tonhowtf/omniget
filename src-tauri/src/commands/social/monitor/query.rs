use serde::Serialize;
use serde_json::{json, Value};

use crate::commands::social::insights::{invalid, strings};
use crate::commands::social::model::Platform;

pub const MAX_QUERY: usize = 500;

#[derive(Serialize, Default, Debug, PartialEq, Clone)]
pub struct Options {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub phrase: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub any: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub exclude: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub hashtags: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub from: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub to: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub mentions: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lang: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min_likes: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min_reposts: Option<u64>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub exclude_replies: bool,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub exclude_reposts: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sort: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub time: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subreddit: Option<String>,
}

fn count(v: &Value, key: &str) -> Result<Option<u64>, String> {
    match &v[key] {
        Value::Null => Ok(None),
        n => n
            .as_u64()
            .map(|n| Some(n).filter(|n| *n > 0))
            .ok_or_else(|| invalid(format!("options.{key} must be a whole number"))),
    }
}

pub fn options(v: &Value) -> Result<Options, String> {
    if v.is_null() {
        return Ok(Options::default());
    }
    if !v.is_object() {
        return Err(invalid("options must be an object"));
    }
    let text = |key: &str| {
        v[key]
            .as_str()
            .map(|t| t.trim().to_string())
            .filter(|t| !t.is_empty())
    };
    Ok(Options {
        phrase: text("phrase"),
        any: strings(v, "any")?,
        exclude: strings(v, "exclude")?,
        hashtags: strings(v, "hashtags")?,
        from: strings(v, "from")?,
        to: strings(v, "to")?,
        mentions: strings(v, "mentions")?,
        lang: text("lang"),
        min_likes: count(v, "min_likes")?,
        min_reposts: count(v, "min_reposts")?,
        exclude_replies: v["exclude_replies"].as_bool().unwrap_or(false),
        exclude_reposts: v["exclude_reposts"].as_bool().unwrap_or(false),
        sort: text("sort"),
        time: text("time"),
        subreddit: text("subreddit"),
    })
}

pub fn kinds(p: Platform) -> &'static [&'static str] {
    match p {
        Platform::X | Platform::Tiktok => &["search"],
        Platform::Instagram => &["hashtag", "profile"],
        Platform::Reddit => &["search", "subreddit"],
        Platform::Pinterest => &["search"],
        Platform::Github => &["repositories", "issues", "releases", "repo_issues"],
    }
}

fn term(word: &str) -> String {
    let clean = word.replace('"', "");
    let clean = clean.trim();
    if clean.contains(char::is_whitespace) {
        format!("\"{clean}\"")
    } else {
        clean.to_string()
    }
}

fn group(items: Vec<String>) -> Option<String> {
    match items.len() {
        0 => None,
        1 => items.into_iter().next(),
        _ => Some(format!("({})", items.join(" OR "))),
    }
}

fn x_handle(raw: &str) -> Result<String, String> {
    let h = raw.trim().trim_start_matches('@');
    if (1..=15).contains(&h.len()) && h.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'_') {
        Ok(h.to_string())
    } else {
        Err(invalid(format!(
            "{raw} is not an X handle (letters, digits and _ only, 15 at most)"
        )))
    }
}

fn hashtag(raw: &str) -> Result<String, String> {
    let t = raw.trim().trim_start_matches(['#', '＃']);
    if !t.is_empty() && t.chars().all(|c| c.is_alphanumeric() || c == '_') {
        Ok(t.to_string())
    } else {
        Err(invalid(format!(
            "{raw} is not a hashtag (letters, digits and _ only)"
        )))
    }
}

fn handles(list: &[String], prefix: &str) -> Result<Option<String>, String> {
    let mut out = Vec::new();
    for h in list {
        out.push(format!("{prefix}{}", x_handle(h)?));
    }
    Ok(group(out))
}

pub fn x_query(base: &str, o: &Options) -> Result<String, String> {
    let mut parts: Vec<String> = Vec::new();
    let base = base.trim();
    if !base.is_empty() {
        parts.push(base.to_string());
    }
    if let Some(p) = &o.phrase {
        parts.push(format!("\"{}\"", p.replace('"', "").trim()));
    }
    let any: Vec<String> = o
        .any
        .iter()
        .map(|w| term(w))
        .filter(|w| !w.is_empty())
        .collect();
    parts.extend(group(any));
    let mut tags = Vec::new();
    for t in &o.hashtags {
        tags.push(format!("#{}", hashtag(t)?));
    }
    parts.extend(group(tags));
    parts.extend(handles(&o.from, "from:")?);
    parts.extend(handles(&o.to, "to:")?);
    parts.extend(handles(&o.mentions, "@")?);
    if parts.is_empty() {
        return Err(invalid(
            "a monitor needs something to look for: a query, a phrase, any-of words, hashtags or accounts",
        ));
    }
    for w in &o.exclude {
        let t = term(w);
        if !t.is_empty() {
            parts.push(format!("-{t}"));
        }
    }
    if let Some(lang) = &o.lang {
        if !(2..=3).contains(&lang.len()) || !lang.bytes().all(|c| c.is_ascii_alphabetic()) {
            return Err(invalid(
                "options.lang must be a two or three letter language code such as en or pt",
            ));
        }
        parts.push(format!("lang:{}", lang.to_ascii_lowercase()));
    }
    if let Some(n) = o.min_likes {
        parts.push(format!("min_faves:{n}"));
    }
    if let Some(n) = o.min_reposts {
        parts.push(format!("min_retweets:{n}"));
    }
    if o.exclude_replies {
        parts.push("-filter:replies".into());
    }
    if o.exclude_reposts {
        parts.push("-filter:retweets".into());
    }
    let query = parts.join(" ");
    if query.chars().count() > MAX_QUERY {
        return Err(invalid(format!(
            "the final query has {} characters; X search takes {MAX_QUERY} at most",
            query.chars().count()
        )));
    }
    Ok(query)
}

pub fn build(p: Platform, kind: &str, base: &str, o: &Options) -> Result<String, String> {
    if !kinds(p).contains(&kind) {
        return Err(invalid(format!(
            "kind for {} must be one of: {}",
            p.label(),
            kinds(p).join(", ")
        )));
    }
    let reddit_options = Options {
        sort: o.sort.clone(),
        time: o.time.clone(),
        subreddit: o.subreddit.clone(),
        ..Default::default()
    };
    if p == Platform::X {
        if reddit_options != Options::default() {
            return Err(invalid(
                "options.sort, options.time and options.subreddit belong to a Reddit search monitor",
            ));
        }
        return x_query(base, o);
    }
    if p == Platform::Reddit && kind == "search" {
        if *o != reddit_options {
            return Err(invalid(
                "a Reddit search monitor takes only options.sort, options.time and options.subreddit",
            ));
        }
        return reddit_search(base, o);
    }
    if *o != Options::default() {
        return Err(invalid(
            "options are for X search operators and Reddit search; here pass only kind and query",
        ));
    }
    let base = base.trim();
    match (p, kind) {
        (Platform::Reddit, _) => crate::commands::social::reddit::sub_of(base)
            .map(|name| format!("r/{name}"))
            .ok_or_else(|| invalid(format!("{base} is not a subreddit name such as r/rust"))),
        (Platform::Github, "repositories" | "issues") => {
            if base.is_empty() || base.chars().count() > 256 {
                return Err(invalid(
                    "query must be GitHub search text with qualifiers, 1 to 256 characters",
                ));
            }
            Ok(base.to_string())
        }
        (Platform::Github, _) => crate::commands::social::github::repo_of(base).ok_or_else(|| {
            invalid(format!(
                "{base} is not a repository (owner/name or a github.com link)"
            ))
        }),
        (Platform::Instagram, "hashtag") => Ok(format!("#{}", hashtag(base)?)),
        (Platform::Instagram, _) => {
            let user = crate::commands::social::instagram::username_of(base);
            let ok = (1..=30).contains(&user.len())
                && user
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b'.');
            if !ok {
                return Err(invalid(format!("{base} is not an Instagram username")));
            }
            Ok(format!("@{user}"))
        }
        _ => {
            if base.is_empty() || base.chars().count() > 200 {
                return Err(invalid(
                    "query must be a search phrase of 1 to 200 characters",
                ));
            }
            Ok(base.to_string())
        }
    }
}

const REDDIT_SORTS: &[&str] = &["relevance", "hot", "top", "new", "comments"];
const REDDIT_TIMES: &[&str] = &["hour", "day", "week", "month", "year", "all"];

fn reddit_search(base: &str, o: &Options) -> Result<String, String> {
    let base = base.trim();
    if base.is_empty() || base.chars().count() > 200 {
        return Err(invalid(
            "query must be a search phrase of 1 to 200 characters",
        ));
    }
    if let Some(sort) = &o.sort {
        if !REDDIT_SORTS.contains(&sort.as_str()) {
            return Err(invalid(format!(
                "options.sort must be one of {}",
                REDDIT_SORTS.join(", ")
            )));
        }
    }
    if let Some(time) = &o.time {
        if !REDDIT_TIMES.contains(&time.as_str()) {
            return Err(invalid(format!(
                "options.time must be one of {}",
                REDDIT_TIMES.join(", ")
            )));
        }
    }
    if let Some(sub) = &o.subreddit {
        if crate::commands::social::reddit::sub_of(sub).is_none() {
            return Err(invalid(format!(
                "options.subreddit {sub} is not a subreddit name such as r/rust"
            )));
        }
    }
    Ok(base.to_string())
}

pub fn request(
    p: Platform,
    kind: &str,
    query: &str,
    options: &Value,
) -> Option<(&'static str, Value)> {
    Some(match (p, kind) {
        (Platform::Reddit, "search") => {
            let mut args = json!({ "query": query, "kind": "posts", "sort": options["sort"].as_str().unwrap_or("new") });
            if let Some(time) = options["time"].as_str() {
                args["time"] = json!(time);
            }
            if let Some(sub) = options["subreddit"]
                .as_str()
                .and_then(crate::commands::social::reddit::sub_of)
            {
                args["subreddit"] = json!(sub);
            }
            ("search", args)
        }
        (Platform::Reddit, "subreddit") => {
            ("user_posts", json!({ "username": query, "sort": "new" }))
        }
        (Platform::Pinterest, "search") => ("search", json!({ "query": query, "kind": "pins" })),
        (Platform::Github, "mentions") => ("notifications", json!({})),
        (Platform::Github, "repositories") => (
            "search",
            json!({ "query": query, "kind": "repositories", "sort": "updated", "order": "desc" }),
        ),
        (Platform::Github, "issues") => (
            "search",
            json!({ "query": query, "kind": "issues", "sort": "created", "order": "desc" }),
        ),
        (Platform::Github, "releases") => ("releases", json!({ "repo": query })),
        (Platform::Github, "repo_issues") => (
            "issues",
            json!({ "repo": query, "state": "all", "kind": "issues" }),
        ),
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reddit_monitors_are_a_search_or_a_subreddit() {
        let none = Options::default();
        let o = opts(json!({ "sort": "top", "time": "week", "subreddit": "r/rust" }));
        assert_eq!(
            build(Platform::Reddit, "search", " tauri app ", &o).unwrap(),
            "tauri app"
        );
        assert_eq!(
            request(Platform::Reddit, "search", "tauri app", &json!(o)),
            Some((
                "search",
                json!({ "query": "tauri app", "kind": "posts", "sort": "top", "time": "week", "subreddit": "rust" })
            ))
        );
        assert_eq!(
            request(Platform::Reddit, "search", "tauri", &json!({})),
            Some((
                "search",
                json!({ "query": "tauri", "kind": "posts", "sort": "new" })
            ))
        );
        assert_eq!(
            build(
                Platform::Reddit,
                "subreddit",
                "https://www.reddit.com/r/rust/",
                &none
            )
            .unwrap(),
            "r/rust"
        );
        assert_eq!(
            request(Platform::Reddit, "subreddit", "r/rust", &json!({})),
            Some(("user_posts", json!({ "username": "r/rust", "sort": "new" })))
        );
        for (kind, base, o) in [
            ("search", " ", none.clone()),
            ("search", "x", opts(json!({ "sort": "best" }))),
            ("search", "x", opts(json!({ "time": "decade" }))),
            ("search", "x", opts(json!({ "subreddit": "not a sub!" }))),
            ("search", "x", opts(json!({ "lang": "en" }))),
            ("subreddit", "not a sub!", none.clone()),
            ("subreddit", "rust", opts(json!({ "sort": "new" }))),
            ("hashtag", "rust", none.clone()),
        ] {
            let e = build(Platform::Reddit, kind, base, &o).unwrap_err();
            assert!(e.starts_with("INVALID_ARGUMENTS"), "{kind} {base}");
        }
        assert!(build(Platform::X, "search", "x", &opts(json!({ "sort": "new" }))).is_err());
    }

    #[test]
    fn github_monitors_are_searches_or_one_repository() {
        let none = Options::default();
        assert_eq!(
            build(
                Platform::Github,
                "repositories",
                " tauri language:rust stars:>50 ",
                &none
            )
            .unwrap(),
            "tauri language:rust stars:>50"
        );
        assert_eq!(
            request(
                Platform::Github,
                "repositories",
                "tauri language:rust",
                &json!({})
            ),
            Some((
                "search",
                json!({ "query": "tauri language:rust", "kind": "repositories", "sort": "updated", "order": "desc" })
            ))
        );
        assert_eq!(
            build(
                Platform::Github,
                "issues",
                "is:pr is:open label:bug repo:a/b",
                &none
            )
            .unwrap(),
            "is:pr is:open label:bug repo:a/b"
        );
        assert_eq!(
            request(Platform::Github, "issues", "is:pr repo:a/b", &json!({})),
            Some((
                "search",
                json!({ "query": "is:pr repo:a/b", "kind": "issues", "sort": "created", "order": "desc" })
            ))
        );
        assert_eq!(
            build(
                Platform::Github,
                "releases",
                "https://github.com/ana/tool/releases",
                &none
            )
            .unwrap(),
            "ana/tool"
        );
        assert_eq!(
            request(Platform::Github, "releases", "ana/tool", &json!({})),
            Some(("releases", json!({ "repo": "ana/tool" })))
        );
        assert_eq!(
            build(Platform::Github, "repo_issues", "ana/tool", &none).unwrap(),
            "ana/tool"
        );
        assert_eq!(
            request(Platform::Github, "repo_issues", "ana/tool", &json!({})),
            Some((
                "issues",
                json!({ "repo": "ana/tool", "state": "all", "kind": "issues" })
            ))
        );
        assert_eq!(
            request(Platform::Github, "mentions", "", &json!({})),
            Some(("notifications", json!({})))
        );
        assert!(build(Platform::Github, "repositories", " ", &none).is_err());
        assert!(build(Platform::Github, "issues", &"a".repeat(257), &none).is_err());
        assert!(build(Platform::Github, "releases", "just-a-name", &none).is_err());
        assert!(build(Platform::Github, "search", "x", &none).is_err());
        assert!(build(
            Platform::Github,
            "issues",
            "x",
            &opts(json!({ "sort": "new" }))
        )
        .is_err());
    }

    #[test]
    fn pinterest_monitors_are_a_search_phrase() {
        let none = Options::default();
        assert_eq!(
            build(Platform::Pinterest, "search", " desk setup ", &none).unwrap(),
            "desk setup"
        );
        assert_eq!(
            request(Platform::Pinterest, "search", "desk setup", &json!({})),
            Some(("search", json!({ "query": "desk setup", "kind": "pins" })))
        );
        assert!(build(Platform::Pinterest, "search", "", &none).is_err());
        assert!(build(Platform::Pinterest, "hashtag", "x", &none).is_err());
        assert_eq!(request(Platform::X, "search", "x", &json!({})), None);
        assert_eq!(kinds(Platform::Reddit), ["search", "subreddit"]);
        assert_eq!(kinds(Platform::Pinterest), ["search"]);
        assert_eq!(kinds(Platform::Github).len(), 4);
    }

    fn opts(v: Value) -> Options {
        options(&v).unwrap()
    }

    #[test]
    fn options_become_x_search_operators() {
        let o = opts(json!({
            "phrase": "open source", "any": ["tauri", "svelte kit"], "exclude": ["hiring", "job ad"],
            "hashtags": ["#rust", "dev"], "from": ["@ana"], "to": ["bob", "carl"], "mentions": ["dora"],
            "lang": "PT", "min_likes": 10, "min_reposts": 2, "exclude_replies": true, "exclude_reposts": true
        }));
        assert_eq!(
            x_query("downloader", &o).unwrap(),
            "downloader \"open source\" (tauri OR \"svelte kit\") (#rust OR #dev) from:ana (to:bob OR to:carl) @dora -hiring -\"job ad\" lang:pt min_faves:10 min_retweets:2 -filter:replies -filter:retweets"
        );
    }

    #[test]
    fn raw_operators_pass_through_and_single_items_are_not_grouped() {
        assert_eq!(
            x_query("rust OR zig since:2026-01-01", &Options::default()).unwrap(),
            "rust OR zig since:2026-01-01"
        );
        assert_eq!(
            x_query("", &opts(json!({ "any": ["one"] }))).unwrap(),
            "one"
        );
        assert_eq!(
            x_query("", &opts(json!({ "hashtags": "東京, café" }))).unwrap(),
            "(#東京 OR #café)"
        );
    }

    #[test]
    fn a_monitor_needs_a_positive_term_and_valid_values() {
        for bad in [
            json!({}),
            json!({ "exclude": ["spam"], "min_likes": 5 }),
            json!({ "from": ["not a handle"] }),
            json!({ "hashtags": ["two words"] }),
            json!({ "any": ["x"], "lang": "portuguese" }),
        ] {
            let e = x_query("", &opts(bad.clone())).unwrap_err();
            assert!(e.starts_with("INVALID_ARGUMENTS"), "{bad}");
        }
        assert!(options(&json!({ "min_likes": "many" })).is_err());
        assert!(options(&json!("text")).is_err());
        assert!(x_query(&"a".repeat(501), &Options::default()).is_err());
        assert_eq!(opts(json!({ "min_likes": 0 })).min_likes, None);
    }

    #[test]
    fn instagram_and_tiktok_take_a_hashtag_a_profile_or_a_phrase() {
        let none = Options::default();
        assert_eq!(
            build(Platform::Instagram, "hashtag", " #Café ", &none).unwrap(),
            "#Café"
        );
        assert_eq!(
            build(
                Platform::Instagram,
                "profile",
                "https://www.instagram.com/ana.dev/",
                &none
            )
            .unwrap(),
            "@ana.dev"
        );
        assert_eq!(
            build(Platform::Tiktok, "search", " cat videos ", &none).unwrap(),
            "cat videos"
        );
        assert!(build(Platform::Instagram, "search", "x", &none).is_err());
        assert!(build(Platform::Instagram, "hashtag", "two words", &none).is_err());
        assert!(build(Platform::Instagram, "profile", "bad name!", &none).is_err());
        assert!(build(Platform::Tiktok, "search", " ", &none).is_err());
        assert!(build(
            Platform::Tiktok,
            "search",
            "x",
            &opts(json!({ "lang": "en" }))
        )
        .is_err());
        assert!(build(Platform::X, "hashtag", "x", &none).is_err());
        assert_eq!(build(Platform::X, "search", "x", &none).unwrap(), "x");
    }

    #[test]
    fn options_are_stored_without_empty_fields() {
        assert_eq!(json!(Options::default()), json!({}));
        assert_eq!(
            json!(opts(json!({ "any": ["a"], "exclude_replies": true }))),
            json!({ "any": ["a"], "exclude_replies": true })
        );
    }
}

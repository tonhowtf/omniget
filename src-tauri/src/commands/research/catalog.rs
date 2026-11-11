use std::sync::OnceLock;

use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

use super::fail;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Param {
    pub name: String,
    #[serde(rename = "in")]
    pub place: String,
    pub required: bool,
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(rename = "enum", default, skip_serializing_if = "Option::is_none")]
    pub allowed: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub example: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Operation {
    pub id: String,
    pub platform: String,
    pub group: String,
    pub method: String,
    pub path: String,
    pub summary: String,
    pub params: Vec<Param>,
    pub pagination: Vec<String>,
    pub credits: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct Catalog {
    pub base: String,
    pub operations: Vec<Operation>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Request {
    pub post: bool,
    pub url: String,
    pub body: Option<Value>,
}

pub fn catalog() -> &'static Catalog {
    static LOADED: OnceLock<Catalog> = OnceLock::new();
    LOADED.get_or_init(|| {
        serde_json::from_str(include_str!("endpoints.json")).unwrap_or_else(|_| Catalog {
            base: "https://api.scrapecreators.com".into(),
            operations: Vec::new(),
        })
    })
}

pub fn find(endpoint: &str) -> Result<&'static Operation, String> {
    let wanted = endpoint.trim();
    let operations = &catalog().operations;
    operations
        .iter()
        .find(|op| op.id == wanted)
        .or_else(|| {
            operations
                .iter()
                .find(|op| op.path == wanted && op.method == "GET")
        })
        .or_else(|| operations.iter().find(|op| op.path == wanted))
        .ok_or_else(|| {
            let close: Vec<&str> = search(None, Some(wanted))
                .into_iter()
                .take(5)
                .map(|op| op.id.as_str())
                .collect();
            fail(
                "UNKNOWN_ENDPOINT",
                &format!("no operation has the id or path \"{wanted}\""),
                false,
                "call research_endpoints with platform or query to get a valid id",
                json!({ "candidates": close }),
            )
        })
}

pub fn platforms() -> Vec<Value> {
    let mut out: Vec<(String, String, usize)> = Vec::new();
    for op in &catalog().operations {
        match out.iter_mut().find(|row| row.0 == op.platform) {
            Some(row) => row.2 += 1,
            None => out.push((op.platform.clone(), op.group.clone(), 1)),
        }
    }
    out.into_iter()
        .map(|(platform, name, operations)| json!({"platform": platform, "name": name, "operations": operations}))
        .collect()
}

pub fn search(platform: Option<&str>, query: Option<&str>) -> Vec<&'static Operation> {
    let platform = platform.map(|p| p.trim().to_lowercase().replace([' ', '-'], "_"));
    let words: Vec<String> = query
        .unwrap_or_default()
        .to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(str::to_string)
        .collect();
    catalog()
        .operations
        .iter()
        .filter(|op| platform.as_deref().is_none_or(|p| op.platform == p))
        .filter(|op| {
            let hay = format!("{} {} {} {}", op.id, op.path, op.group, op.summary).to_lowercase();
            words.iter().all(|w| hay.contains(w.as_str()))
        })
        .collect()
}

fn scalar(param: &Param, value: &Value) -> Result<String, String> {
    let wrong = |expected: &str| {
        fail(
            "INVALID_PARAMS",
            &format!("parameter \"{}\" must be {expected}", param.name),
            false,
            "fix the value and call again",
            Value::Null,
        )
    };
    let text = match (param.kind.as_str(), value) {
        ("boolean", Value::Bool(b)) => b.to_string(),
        ("boolean", Value::String(s)) if s == "true" || s == "false" => s.clone(),
        ("boolean", _) => return Err(wrong("true or false")),
        ("number" | "integer", Value::Number(n)) => n.to_string(),
        ("number" | "integer", Value::String(s)) if s.trim().parse::<f64>().is_ok() => {
            s.trim().to_string()
        }
        ("number" | "integer", _) => return Err(wrong("a number")),
        (_, Value::String(s)) => s.clone(),
        (_, Value::Number(n)) => n.to_string(),
        (_, Value::Bool(b)) => b.to_string(),
        _ => return Err(wrong("a string, number or boolean")),
    };
    if let Some(allowed) = &param.allowed {
        if !allowed.contains(&text) {
            return Err(fail(
                "INVALID_PARAMS",
                &format!("parameter \"{}\" does not accept \"{text}\"", param.name),
                false,
                "use one of the allowed values",
                json!({ "allowed": allowed }),
            ));
        }
    }
    Ok(text)
}

fn typed(param: &Param, text: String) -> Value {
    match param.kind.as_str() {
        "boolean" => Value::Bool(text == "true"),
        "number" | "integer" => text
            .parse::<f64>()
            .ok()
            .and_then(serde_json::Number::from_f64)
            .map_or(Value::String(text), Value::Number),
        _ => Value::String(text),
    }
}

pub fn request(op: &Operation, params: &Value) -> Result<Request, String> {
    let empty = Map::new();
    let given = match params {
        Value::Null => &empty,
        Value::Object(map) => map,
        _ => {
            return Err(fail(
                "INVALID_PARAMS",
                "params must be an object of parameter names and values",
                false,
                "pass params as {\"name\": value}",
                Value::Null,
            ))
        }
    };
    let names: Vec<&str> = op.params.iter().map(|p| p.name.as_str()).collect();
    if let Some(unknown) = given.keys().find(|k| !names.contains(&k.as_str())) {
        return Err(fail(
            "INVALID_PARAMS",
            &format!("{} has no parameter \"{unknown}\"", op.id),
            false,
            "use only the parameters listed for this operation",
            json!({ "allowed": names }),
        ));
    }
    let mut url = url::Url::parse(&format!("{}{}", catalog().base, op.path))
        .map_err(|e| fail("INVALID_PARAMS", &e.to_string(), false, "", Value::Null))?;
    let mut body = Map::new();
    for param in &op.params {
        let value = given.get(&param.name).filter(|v| !v.is_null());
        let Some(value) = value else {
            if param.required {
                return Err(fail(
                    "INVALID_PARAMS",
                    &format!("{} needs the parameter \"{}\"", op.id, param.name),
                    false,
                    "add the missing parameter and call again",
                    json!({ "missing": param.name, "description": param.description }),
                ));
            }
            continue;
        };
        let text = scalar(param, value)?;
        if param.place == "body" {
            body.insert(param.name.clone(), typed(param, text));
        } else {
            url.query_pairs_mut().append_pair(&param.name, &text);
        }
    }
    let post = op.method == "POST";
    Ok(Request {
        post,
        url: url.into(),
        body: post.then_some(Value::Object(body)),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn code(error: &str) -> (String, Value) {
        let (code, detail) = error.split_once(": ").unwrap();
        (code.to_string(), serde_json::from_str(detail).unwrap())
    }

    #[test]
    fn the_embedded_catalog_holds_every_operation_with_unique_ids() {
        let ops = &catalog().operations;
        assert_eq!(catalog().base, "https://api.scrapecreators.com");
        assert!(ops.len() >= 199, "{}", ops.len());
        let mut ids: Vec<&str> = ops.iter().map(|op| op.id.as_str()).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), ops.len());
        for platform in [
            "reddit",
            "pinterest",
            "github",
            "youtube",
            "linkedin",
            "facebook",
            "threads",
            "bluesky",
            "twitch",
            "snapchat",
            "facebook_ad_library",
            "linktree",
        ] {
            assert!(ops.iter().any(|op| op.platform == platform), "{platform}");
        }
    }

    #[test]
    fn the_workflows_depend_on_operations_that_exist_with_these_parameters() {
        for (id, path, params) in [
            (
                "tiktok_search_keyword",
                "/v1/tiktok/search/keyword",
                vec!["query", "cursor"],
            ),
            (
                "tiktok_search_hashtag",
                "/v1/tiktok/search/hashtag",
                vec!["hashtag"],
            ),
            ("tiktok_profile", "/v1/tiktok/profile", vec!["handle"]),
            (
                "tiktok_profile_videos",
                "/v3/tiktok/profile/videos",
                vec!["handle", "sort_by", "max_cursor"],
            ),
            (
                "tiktok_video",
                "/v2/tiktok/video",
                vec!["url", "get_transcript"],
            ),
            (
                "tiktok_video_transcript",
                "/v1/tiktok/video/transcript",
                vec!["url", "use_ai_as_fallback"],
            ),
            (
                "facebook_adlibrary_company_ads",
                "/v1/facebook/adLibrary/company/ads",
                vec!["pageId", "companyName", "country", "status", "cursor"],
            ),
            (
                "facebook_adlibrary_search_companies",
                "/v1/facebook/adLibrary/search/companies",
                vec!["query"],
            ),
            (
                "instagram_user_reels",
                "/v1/instagram/user/reels",
                vec!["handle", "max_id"],
            ),
            ("instagram_post", "/v1/instagram/post", vec!["url"]),
            (
                "account_credit_balance",
                "/v1/account/credit-balance",
                vec![],
            ),
            ("linktree", "/v1/linktree", vec!["url"]),
        ] {
            let op = find(id).unwrap();
            assert_eq!(op.path, path);
            for name in params {
                assert!(op.params.iter().any(|p| p.name == name), "{id} {name}");
            }
        }
    }

    #[test]
    fn an_operation_is_found_by_id_or_by_path_and_a_miss_offers_candidates() {
        assert_eq!(find("/v1/reddit/search").unwrap().id, "reddit_search");
        assert_eq!(find("/v1/reddit/post/comments").unwrap().method, "GET");
        assert_eq!(find("reddit_post_comments_post").unwrap().method, "POST");
        let error = find("reddit").unwrap_err();
        let (code, detail) = code(&error);
        assert_eq!(code, "UNKNOWN_ENDPOINT");
        assert!(!detail["candidates"].as_array().unwrap().is_empty());
        assert!(detail["next_step"]
            .as_str()
            .unwrap()
            .contains("research_endpoints"));
    }

    #[test]
    fn platforms_are_counted_and_search_narrows_by_platform_and_words() {
        let listed = platforms();
        assert!(listed.len() >= 38);
        let total: u64 = listed
            .iter()
            .map(|p| p["operations"].as_u64().unwrap())
            .sum();
        assert_eq!(total as usize, catalog().operations.len());
        assert_eq!(search(Some("Bluesky"), None).len(), 3);
        let found = search(None, Some("tiktok transcript"));
        assert!(found.iter().any(|op| op.id == "tiktok_video_transcript"));
        assert!(found.iter().all(|op| op.platform.starts_with("tiktok")));
        assert!(search(Some("nowhere"), None).is_empty());
    }

    #[test]
    fn a_request_url_carries_validated_and_encoded_parameters() {
        let op = find("tiktok_search_keyword").unwrap();
        let built = request(
            op,
            &json!({"query": "home gym & more", "trim": true, "cursor": 10, "region": null}),
        )
        .unwrap();
        assert!(!built.post);
        assert_eq!(built.body, None);
        assert_eq!(
            built.url,
            "https://api.scrapecreators.com/v1/tiktok/search/keyword?query=home+gym+%26+more&trim=true&cursor=10"
        );
        let none = request(find("account_credit_balance").unwrap(), &Value::Null).unwrap();
        assert_eq!(
            none.url,
            "https://api.scrapecreators.com/v1/account/credit-balance"
        );
    }

    #[test]
    fn a_post_operation_sends_typed_values_in_the_body() {
        let op = find("reddit_post_comments_post").unwrap();
        let built = request(
            op,
            &json!({"url": "https://www.reddit.com/r/a/comments/1", "trim": "true"}),
        )
        .unwrap();
        assert!(built.post);
        assert_eq!(
            built.url,
            "https://api.scrapecreators.com/v1/reddit/post/comments"
        );
        assert_eq!(
            built.body,
            Some(json!({"url": "https://www.reddit.com/r/a/comments/1", "trim": true}))
        );
    }

    #[test]
    fn missing_unknown_mistyped_and_out_of_enum_parameters_are_refused() {
        let op = find("tiktok_search_keyword").unwrap();
        let (c, detail) = code(&request(op, &json!({})).unwrap_err());
        assert_eq!(
            (c.as_str(), detail["missing"].as_str()),
            ("INVALID_PARAMS", Some("query"))
        );
        let (_, detail) = code(&request(op, &json!({"query": "a", "handle": "b"})).unwrap_err());
        assert!(detail["allowed"]
            .as_array()
            .unwrap()
            .contains(&json!("query")));
        let (_, detail) =
            code(&request(op, &json!({"query": "a", "sort_by": "newest"})).unwrap_err());
        assert!(detail["allowed"]
            .as_array()
            .unwrap()
            .contains(&json!("relevance")));
        assert!(request(op, &json!({"query": "a", "trim": "yes"})).is_err());
        assert!(request(op, &json!({"query": "a", "cursor": "ten"})).is_err());
        assert!(request(op, &json!({"query": ["a"]})).is_err());
        assert!(request(op, &json!("query=a")).is_err());
        assert_eq!(detail["retryable"], false);
    }
}

use serde_json::{json, Value};

use super::{ads, catalog, client, fail, reels, tiktok};

pub const TOOLS: &[&str] = &[
    "research_endpoints",
    "research_call",
    "research_usage",
    "research_tiktok_creators",
    "research_tiktok_brief",
    "research_meta_ads",
    "research_reels",
];

const API: &str = "https://api.scrapecreators.com";

pub fn url_of(name: &str, a: &Value) -> Option<&'static str> {
    let offline = match name {
        "research_endpoints" => true,
        "research_usage" => a["refresh"].as_bool() != Some(true),
        "research_meta_ads" => a["op"].as_str() == Some("list"),
        "research_tiktok_creators" => a["queries"].as_array().is_none_or(Vec::is_empty),
        _ => false,
    };
    (!offline).then_some(API)
}

fn endpoints(a: &Value) -> Value {
    let platform = a["platform"].as_str().filter(|p| !p.trim().is_empty());
    let query = a["query"].as_str().filter(|q| !q.trim().is_empty());
    if platform.is_none() && query.is_none() {
        let platforms = catalog::platforms();
        return json!({
            "total": platforms.len(),
            "returned": platforms.len(),
            "truncated": false,
            "operations_total": catalog::catalog().operations.len(),
            "platforms": platforms,
            "next_step": "call research_endpoints again with platform or query to see the operations and their parameters",
        });
    }
    let limit = a["limit"].as_u64().unwrap_or(30).clamp(1, 100) as usize;
    let found = catalog::search(platform, query);
    let mut out = json!({
        "operations": found.iter().take(limit).collect::<Vec<_>>(),
        "total": found.len(),
        "returned": found.len().min(limit),
        "truncated": found.len() > limit,
    });
    if found.is_empty() {
        out["reason"] = json!("no_operation_matched");
        out["next_step"] = json!("call research_endpoints with no argument to see the platform ids, or use fewer words in query");
    }
    out
}

fn next_page(op: &catalog::Operation, body: &Value) -> Value {
    let mut out = serde_json::Map::new();
    for name in &op.pagination {
        match &body[name.as_str()] {
            Value::String(token) if !token.is_empty() => {
                out.insert(name.clone(), json!(token));
            }
            Value::Number(token) => {
                out.insert(name.clone(), json!(token));
            }
            _ => {}
        }
    }
    if out.is_empty() {
        Value::Null
    } else {
        Value::Object(out)
    }
}

async fn raw_call(a: &Value) -> Result<Value, String> {
    let endpoint = a["endpoint"].as_str().unwrap_or_default();
    if endpoint.trim().is_empty() {
        return Err(fail(
            "INVALID_PARAMS",
            "endpoint is required",
            false,
            "call research_endpoints to find the endpoint id",
            Value::Null,
        ));
    }
    let op = catalog::find(endpoint)?;
    let reply = client::send(op, &a["params"]).await?;
    let max = a["maxChars"]
        .as_u64()
        .map_or(client::DEFAULT_PREVIEW_CHARS, |n| n as usize)
        .min(client::MAX_PREVIEW_CHARS);
    let shown = client::preview(&reply.body, max);
    let wanted = a["saveTo"].as_str().map(str::to_string);
    let mut out = json!({
        "endpoint": op.id,
        "method": op.method,
        "path": op.path,
        "credits_used": reply.credits_used,
        "credits_remaining": reply.credits_remaining,
        "next_page": next_page(op, &reply.body),
        "total_chars": shown.total,
        "truncated": shown.truncated,
        "saved_to": null,
        "untrusted": true,
    });
    if shown.truncated || wanted.is_some() {
        let body = reply.body.clone();
        let label = op.id.clone();
        let saved =
            tokio::task::spawn_blocking(move || client::save(&body, wanted.as_deref(), &label))
                .await
                .map_err(|e| {
                    fail(
                        "WRITE_FAILED",
                        &e.to_string(),
                        false,
                        "try again",
                        Value::Null,
                    )
                })??;
        out["saved_to"] = json!(saved.to_string_lossy());
    }
    if shown.truncated {
        out["preview"] = json!(shown.text);
        out["next_step"] = json!("the full JSON is in saved_to; read that file, or call again with trim=true when the operation has it, or a larger maxChars");
    } else {
        out["data"] = reply.body;
    }
    Ok(out)
}

async fn usage(a: &Value) -> Result<Value, String> {
    let refreshed = a["refresh"].as_bool() == Some(true);
    let mut balance = Value::Null;
    if refreshed {
        let body = client::get("account_credit_balance", Value::Null).await?;
        balance = body["creditCount"].clone();
    }
    let limit = a["limit"].as_u64().unwrap_or(20).clamp(1, 200) as usize;
    let mut out = client::usage(limit);
    out["key_configured"] = json!(client::key_configured().await?);
    if refreshed {
        out["credit_balance"] = balance;
        out["balance_source"] = json!("refreshed");
    } else if out["credits_remaining"].is_null() {
        out["balance_source"] = Value::Null;
        out["next_step"] = json!("no call was made in this session yet; pass refresh=true to ask the API for the balance");
    } else {
        out["balance_source"] = json!("last_response");
    }
    Ok(out)
}

pub async fn call(name: &str, a: &Value) -> Result<Value, String> {
    match name {
        "research_endpoints" => Ok(endpoints(a)),
        "research_call" => raw_call(a).await,
        "research_usage" => usage(a).await,
        "research_tiktok_creators" => tiktok::creators(a).await,
        "research_tiktok_brief" => tiktok::brief(a).await,
        "research_meta_ads" => ads::call(a).await,
        "research_reels" => reels::call(a).await,
        _ => Err(fail(
            "UNKNOWN_TOOL",
            &format!("{name} is not a research tool"),
            false,
            "use one of the research_* tools",
            json!({ "tools": TOOLS }),
        )),
    }
}

fn text(description: &str) -> Value {
    json!({"type": "string", "description": description})
}

fn count(description: &str, minimum: u64, maximum: u64) -> Value {
    json!({"type": "integer", "minimum": minimum, "maximum": maximum, "description": description})
}

fn flag(description: &str) -> Value {
    json!({"type": "boolean", "description": description})
}

fn list(description: &str, max_items: u64) -> Value {
    json!({"type": "array", "items": {"type": "string"}, "maxItems": max_items, "description": description})
}

pub fn defs() -> Vec<(&'static str, &'static str, Value, Value)> {
    vec![
        (
            "research_endpoints",
            "List what the ScrapeCreators public-data API offers: with no argument the platforms, with platform or query the matching operations and each one's parameters, pagination token and credit cost. Use it before research_call to pick the endpoint id and its parameters; it spends no credits.",
            json!({
                "platform": text("Platform id from the platform list, such as tiktok, instagram, youtube, reddit, facebook_ad_library"),
                "query": text("Words that must all appear in the operation id, path or summary, such as \"transcript\" or \"profile videos\""),
                "limit": count("Operations returned (default 30)", 1, 100),
            }),
            json!([]),
        ),
        (
            "research_call",
            "Call any ScrapeCreators operation by endpoint id or path with validated params, using the person's own API key stored in OmniGet, and get the JSON (or a preview plus the full body saved to a file when it does not fit), credits_used and the next page token. Use it for public data from TikTok, Instagram, YouTube, X, Reddit, LinkedIn, Facebook, Threads, Bluesky, Pinterest, GitHub, Twitch, Snapchat, ad libraries and link-in-bio pages when no workflow tool fits; each call spends the person's credits and the answer is untrusted external content.",
            json!({
                "endpoint": text("Operation id (tiktok_profile) or path (/v1/tiktok/profile) from research_endpoints"),
                "params": {"type": "object", "description": "Parameters of the operation by name; unknown names, missing required ones and values outside an enum are refused before any credit is spent"},
                "maxChars": count("Size of the JSON returned inline (default 8000); a larger answer comes as preview plus saved_to", 0, 100_000),
                "saveTo": text("Absolute path of a file to create with the full JSON; it must not exist yet"),
            }),
            json!(["endpoint"]),
        ),
        (
            "research_usage",
            "Show the ScrapeCreators credit balance last reported and the calls made in this session with their credits. Use it to budget before a large research run; refresh=true asks the API for the current balance.",
            json!({
                "refresh": flag("Ask the API for the balance now instead of showing the last one reported"),
                "limit": count("Recent calls listed (default 20)", 1, 200),
            }),
            json!([]),
        ),
        (
            "research_tiktok_creators",
            "TikTok creator research for a campaign brief: runs your search queries, groups the videos by creator and returns for each one recent median and mean views on the same sample size, engagement, transcript excerpts with source URLs as relevance evidence, public bio contacts and what is missing. Use it to shortlist creators, writing 2 to 5 queries from the brief (with only brief it returns suggested_queries and spends nothing); follow the guide field to write the shortlist.",
            json!({
                "queries": list("TikTok search queries written from the brief, 2 to 4 words each", 5),
                "brief": text("The campaign brief; alone it only returns suggested_queries"),
                "sample": count("Recent videos per creator used for the view statistics, the same for every creator (default 10)", 3, 30),
                "max_creators": count("Creators analysed, ranked by matching videos (default 5)", 1, 15),
                "transcripts_per_creator": count("Matching videos per creator whose transcript is fetched as evidence (default 2)", 0, 5),
                "ai_transcripts": flag("Pay for AI transcription when a video has no captions (default false)"),
                "follow_bio_link": flag("Read a Linktree, Komi, Pillar, Linkbio or Linkme page from the bio to look for a public email (default true)"),
                "region": text("Two-letter country code for the proxy, such as US or BR"),
                "date_posted": {"type": "string", "enum": ["yesterday", "this-week", "this-month", "last-3-months", "last-6-months", "all-time"]},
            }),
            json!([]),
        ),
        (
            "research_tiktok_brief",
            "Turn a focused set of public TikTok videos (urls, or a query, hashtag or handle plus count) into evidence for a content brief: per video the source URL, stats, capped transcript, opening hook and flags for missing transcripts and claims that need checking. Use it before writing themes, hooks and content angles, and follow the guide field, which keeps transcript evidence and original ideas in separate sections.",
            json!({
                "urls": list("TikTok video URLs", 20),
                "query": text("Instead of urls: keyword search, most viewed first"),
                "hashtag": text("Instead of urls: hashtag without #"),
                "handle": text("Instead of urls: a creator's most recent videos"),
                "count": count("Videos taken from query, hashtag or handle (default 5)", 1, 20),
                "transcript_chars": count("Transcript characters returned per video (default 1500)", 200, 6000),
                "ai_transcripts": flag("Pay for AI transcription when a video has no captions (default false)"),
                "region": text("Two-letter country code for the proxy"),
            }),
            json!([]),
        ),
        (
            "research_meta_ads",
            "Monitor a brand's public Meta Ad Library ads: op=find looks up the page, op=baseline saves the current ads, op=compare fetches again and reports new ads, ads no longer listed and changed text, CTA, format or landing URL with archive IDs, public URLs and running time, op=list shows saved baselines. Use it for a weekly creative review; it cannot show spend, conversions or targeting, and every result repeats those caveats.",
            json!({
                "op": {"type": "string", "enum": ["find", "baseline", "compare", "list"]},
                "query": text("find: brand or page name"),
                "page_id": text("baseline: Ad Library page id from op=find"),
                "company": text("baseline: company name, when there is no page_id"),
                "country": text("baseline: one two-letter country code, or ALL (default)"),
                "status": {"type": "string", "enum": ["ACTIVE", "ALL", "INACTIVE"], "description": "baseline: which ads to track (default ACTIVE)"},
                "name": text("baseline: name to save it under (default the company or page id); compare: the saved name"),
                "max_ads": count("Ads fetched at most (default 100); a cut fetch is reported as complete=false", 1, 300),
                "update": flag("compare: replace the saved baseline with this fetch after comparing (default false)"),
                "limit": count("Ads listed per section (default 30)", 1, 200),
            }),
            json!(["op"]),
        ),
        (
            "research_reels",
            "Compare the same number of most recent public Instagram Reels across 2 to 10 profiles: plays, likes, comments, posting cadence, median and mean plays, top and bottom reel per profile with post detail fetched only for those outliers. Use it to explain reel patterns across accounts, following the guide field and its caveats (Instagram-only play counts, small samples flagged).",
            json!({
                "profiles": {"type": "array", "items": {"type": "string"}, "minItems": 2, "maxItems": 10, "description": "Instagram handles without @"},
                "sample": count("Most recent reels per profile (default 12); every profile is cut to the smallest sample available", 3, 50),
                "detail": {"type": "string", "enum": ["outliers", "none"], "description": "outliers (default) fetches the post detail of each profile's top and bottom reel, 2 credits per profile; none skips it"},
            }),
            json!(["profiles"]),
        ),
    ]
}

fn ty(kind: &str) -> Value {
    json!({ "type": kind })
}

pub fn output_schema(name: &str) -> Option<Value> {
    let listing = |key: &str| {
        json!({
            "type": "object",
            "properties": {
                key: {"type": "array", "items": ty("object")},
                "total": ty("integer"), "returned": ty("integer"), "truncated": ty("boolean"),
                "reason": ty("string"), "next_step": ty("string"),
                "credits_used": ty("number"), "caveats": {"type": "array", "items": ty("string")},
                "guide": ty("string"), "untrusted": ty("boolean"),
            },
            "required": ["total", "returned", "truncated"],
        })
    };
    Some(match name {
        "research_endpoints" => json!({
            "type": "object",
            "properties": {
                "platforms": {"type": "array", "items": ty("object")},
                "operations": {"type": "array", "items": ty("object")},
                "operations_total": ty("integer"),
                "total": ty("integer"), "returned": ty("integer"), "truncated": ty("boolean"),
                "reason": ty("string"), "next_step": ty("string"),
            },
            "required": ["total", "returned", "truncated"],
        }),
        "research_call" => json!({
            "type": "object",
            "properties": {
                "endpoint": ty("string"), "method": ty("string"), "path": ty("string"),
                "credits_used": {"type": ["number", "null"]}, "credits_remaining": {"type": ["number", "null"]},
                "next_page": {"type": ["object", "null"]},
                "data": {}, "preview": ty("string"),
                "total_chars": ty("integer"), "truncated": ty("boolean"),
                "saved_to": {"type": ["string", "null"]}, "next_step": ty("string"), "untrusted": ty("boolean"),
            },
            "required": ["endpoint", "total_chars", "truncated", "untrusted"],
        }),
        "research_usage" => json!({
            "type": "object",
            "properties": {
                "credits_remaining": {"type": ["number", "null"]}, "credit_balance": {},
                "balance_source": {"type": ["string", "null"]},
                "session_credits_used": ty("number"), "key_configured": ty("boolean"),
                "calls": {"type": "array", "items": ty("object")},
                "total": ty("integer"), "returned": ty("integer"), "truncated": ty("boolean"),
                "next_step": ty("string"),
            },
            "required": ["calls", "total", "returned", "truncated", "key_configured"],
        }),
        "research_tiktok_creators" => listing("creators"),
        "research_tiktok_brief" => listing("videos"),
        "research_reels" => listing("profiles"),
        "research_meta_ads" => json!({
            "type": "object",
            "properties": {
                "pages": {"type": "array", "items": ty("object")},
                "baselines": {"type": "array", "items": ty("object")},
                "baseline": ty("object"), "ads": {"type": "array", "items": ty("object")},
                "new_ads": {"type": "array", "items": ty("object")},
                "no_longer_listed": {"type": "array", "items": ty("object")},
                "not_seen_in_partial_fetch": {"type": "array", "items": ty("object")},
                "changed": {"type": "array", "items": ty("object")},
                "totals": ty("object"), "fetched": ty("integer"), "complete": ty("boolean"),
                "replaced": ty("boolean"), "baseline_updated": ty("boolean"),
                "total": ty("integer"), "returned": ty("integer"), "truncated": ty("boolean"),
                "reason": ty("string"), "next_step": ty("string"),
                "credits_used": ty("number"), "caveats": {"type": "array", "items": ty("string")},
                "guide": ty("string"), "untrusted": ty("boolean"),
            },
            "required": ["truncated"],
        }),
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_tool_has_a_two_sentence_definition_and_an_output_schema() {
        let defs = defs();
        let names: Vec<&str> = defs.iter().map(|d| d.0).collect();
        assert_eq!(names, TOOLS);
        for (name, description, props, required) in &defs {
            assert!(
                description.matches(". ").count() <= 1,
                "{name}: two sentences"
            );
            assert!(props.is_object(), "{name}");
            for key in required.as_array().unwrap() {
                assert!(props.get(key.as_str().unwrap()).is_some(), "{name} {key}");
            }
            assert!(output_schema(name).is_some(), "{name}");
            assert!(
                !props.to_string().to_lowercase().contains("api key"),
                "{name}: the key is never an argument"
            );
            assert!(
                props.get("key").is_none()
                    && props.get("apiKey").is_none()
                    && props.get("api_key").is_none()
            );
        }
        assert_eq!(
            defs[5].2["op"]["enum"],
            json!(["find", "baseline", "compare", "list"])
        );
        assert_eq!(output_schema("dev_http"), None);
    }

    #[test]
    fn endpoints_lists_platforms_first_and_then_operations_with_parameters() {
        let all = endpoints(&json!({}));
        assert!(all["platforms"].as_array().unwrap().len() >= 38);
        assert!(all["operations_total"].as_u64().unwrap() >= 199);
        assert!(all["next_step"].is_string());
        let reddit = endpoints(&json!({"platform": "reddit"}));
        assert_eq!(reddit["total"], 8);
        assert_eq!(reddit["truncated"], false);
        let search = reddit["operations"]
            .as_array()
            .unwrap()
            .iter()
            .find(|op| op["id"] == "reddit_search")
            .unwrap();
        assert_eq!(search["path"], "/v1/reddit/search");
        assert_eq!(search["pagination"], json!(["after"]));
        assert!(search["params"]
            .as_array()
            .unwrap()
            .iter()
            .any(|p| p["name"] == "query" && p["required"] == true));
        let capped = endpoints(&json!({"platform": "tiktok", "limit": 3}));
        assert_eq!(
            (capped["returned"].as_u64(), capped["truncated"].as_bool()),
            (Some(3), Some(true))
        );
        let none = endpoints(&json!({"query": "zzzz"}));
        assert_eq!(none["reason"], "no_operation_matched");
    }

    #[test]
    fn the_next_page_token_is_read_from_the_answer_by_the_operation_hint() {
        let op = catalog::find("tiktok_profile_videos").unwrap();
        assert_eq!(
            next_page(op, &json!({"max_cursor": 1_748_286_476_000_u64})),
            json!({"max_cursor": 1_748_286_476_000_u64})
        );
        assert_eq!(next_page(op, &json!({"max_cursor": null})), Value::Null);
        let op = catalog::find("instagram_user_reels").unwrap();
        assert_eq!(
            next_page(op, &json!({"max_id": "QVFE"})),
            json!({"max_id": "QVFE"})
        );
    }

    #[test]
    fn only_tools_that_reach_the_api_expose_a_url_to_the_network_check() {
        assert_eq!(url_of("research_endpoints", &json!({})), None);
        assert_eq!(url_of("research_usage", &json!({})), None);
        assert_eq!(
            url_of("research_usage", &json!({"refresh": true})),
            Some(API)
        );
        assert_eq!(url_of("research_meta_ads", &json!({"op": "list"})), None);
        assert_eq!(
            url_of("research_meta_ads", &json!({"op": "compare"})),
            Some(API)
        );
        assert_eq!(
            url_of("research_tiktok_creators", &json!({"brief": "x"})),
            None
        );
        assert_eq!(
            url_of("research_tiktok_creators", &json!({"queries": ["x"]})),
            Some(API)
        );
        assert_eq!(url_of("research_call", &json!({})), Some(API));
    }

    #[tokio::test]
    async fn without_a_key_every_network_tool_answers_no_api_key_with_a_next_step() {
        let dir = tempfile::tempdir().unwrap();
        let env = crate::secrets::test_env::EnvGuard::acquire();
        env.set(
            omniget_core::core::secrets::SECRETS_DIR_ENV,
            dir.path().to_str(),
        );
        for (tool, args) in [
            (
                "research_call",
                json!({"endpoint": "tiktok_profile", "params": {"handle": "x"}}),
            ),
            ("research_usage", json!({"refresh": true})),
            ("research_tiktok_creators", json!({"queries": ["home gym"]})),
            (
                "research_tiktok_brief",
                json!({"urls": ["https://www.tiktok.com/@a/video/1"]}),
            ),
            ("research_tiktok_brief", json!({"query": "home gym"})),
            ("research_meta_ads", json!({"op": "find", "query": "nike"})),
            (
                "research_meta_ads",
                json!({"op": "baseline", "company": "nike"}),
            ),
            ("research_reels", json!({"profiles": ["a", "b"]})),
        ] {
            let outcome = call(tool, &args).await;
            if tool == "research_tiktok_brief" && args["urls"].is_array() {
                let out = outcome.unwrap();
                assert_eq!(out["videos"][0]["error"], "NO_API_KEY");
                continue;
            }
            let error = outcome.unwrap_err();
            let (code, detail) = error.split_once(": ").unwrap();
            assert_eq!(code, "NO_API_KEY", "{tool}");
            let detail: Value = serde_json::from_str(detail).unwrap();
            assert!(
                detail["next_step"].as_str().unwrap().contains("Research"),
                "{tool}"
            );
        }
        let usage = call("research_usage", &json!({})).await.unwrap();
        assert_eq!(usage["key_configured"], false);
    }

    #[tokio::test]
    async fn invalid_calls_are_refused_before_the_key_is_even_needed() {
        let unknown = call("research_call", &json!({"endpoint": "nope"}))
            .await
            .unwrap_err();
        assert!(unknown.starts_with("UNKNOWN_ENDPOINT"));
        let invalid = call(
            "research_call",
            &json!({"endpoint": "tiktok_profile", "params": {"nope": 1}}),
        )
        .await
        .unwrap_err();
        assert!(invalid.starts_with("INVALID_PARAMS"));
        assert!(call("research_call", &json!({}))
            .await
            .unwrap_err()
            .starts_with("INVALID_PARAMS"));
        assert!(call("research_nope", &json!({}))
            .await
            .unwrap_err()
            .starts_with("UNKNOWN_TOOL"));
    }

    #[tokio::test]
    #[ignore = "network and real credits: needs SCRAPECREATORS_API_KEY, run by hand with --ignored"]
    async fn the_live_api_answers_two_cheap_calls_end_to_end() {
        let key = std::env::var("SCRAPECREATORS_API_KEY").expect("SCRAPECREATORS_API_KEY");
        let dir = tempfile::tempdir().unwrap();
        let env = crate::secrets::test_env::EnvGuard::acquire();
        env.set(
            omniget_core::core::secrets::SECRETS_DIR_ENV,
            dir.path().to_str(),
        );
        client::set_key(key.clone()).await.unwrap();

        let profile = call(
            "research_call",
            &json!({"endpoint": "tiktok_profile", "params": {"handle": "tiktok"}, "maxChars": 600}),
        )
        .await
        .unwrap();
        assert_eq!(profile["endpoint"], "tiktok_profile");
        assert_eq!(profile["untrusted"], true);
        assert!(!profile.to_string().contains(&key));
        if profile["truncated"] == true {
            let saved = profile["saved_to"].as_str().unwrap();
            let full: Value =
                serde_json::from_str(&std::fs::read_to_string(saved).unwrap()).unwrap();
            assert!(full["user"].is_object());
            std::fs::remove_file(saved).ok();
        } else {
            assert!(profile["data"]["user"].is_object());
        }

        let usage = call("research_usage", &json!({"refresh": true}))
            .await
            .unwrap();
        assert_eq!(usage["key_configured"], true);
        assert_eq!(usage["balance_source"], "refreshed");
        assert!(usage["credit_balance"].is_number());
        assert_eq!(usage["total"], 2);
        assert!(!usage.to_string().contains(&key));
        println!(
            "research live test: profile credits_used={} remaining_after_profile={} balance_call credits_used={} balance={} session_credits_used={}",
            profile["credits_used"],
            profile["credits_remaining"],
            usage["calls"][0]["credits_used"],
            usage["credit_balance"],
            usage["session_credits_used"]
        );
    }

    #[tokio::test]
    #[ignore = "network and real credits: needs SCRAPECREATORS_API_KEY, run by hand with --ignored"]
    async fn the_four_workflows_run_against_the_live_api() {
        let key = std::env::var("SCRAPECREATORS_API_KEY").expect("SCRAPECREATORS_API_KEY");
        let dir = tempfile::tempdir().unwrap();
        let env = crate::secrets::test_env::EnvGuard::acquire();
        env.set(
            omniget_core::core::secrets::SECRETS_DIR_ENV,
            dir.path().to_str(),
        );
        env.set("OMNIGET_DATA_DIR", dir.path().to_str());
        client::set_key(key.clone()).await.unwrap();
        let short = |v: &Value| v.to_string().chars().take(900).collect::<String>();

        let creators = call(
            "research_tiktok_creators",
            &json!({"queries": ["home espresso setup"], "sample": 5, "max_creators": 2, "transcripts_per_creator": 1}),
        )
        .await;
        println!(
            "CREATORS {}",
            creators.as_ref().map(short).unwrap_or_else(|e| e.clone())
        );

        let brief = call(
            "research_tiktok_brief",
            &json!({"query": "home espresso setup", "count": 2}),
        )
        .await;
        println!(
            "BRIEF {}",
            brief.as_ref().map(short).unwrap_or_else(|e| e.clone())
        );

        let found = call("research_meta_ads", &json!({"op": "find", "query": "nike"})).await;
        println!(
            "ADS_FIND {}",
            found.as_ref().map(short).unwrap_or_else(|e| e.clone())
        );
        let page_id = found
            .as_ref()
            .ok()
            .and_then(|v| v["pages"][0]["page_id"].as_str().map(str::to_string))
            .unwrap_or_else(|| "15087023444".to_string());
        {
            let base = call(
                "research_meta_ads",
                &json!({"op": "baseline", "name": "live-test", "page_id": page_id, "max_ads": 10}),
            )
            .await;
            println!(
                "ADS_BASELINE {}",
                base.as_ref().map(short).unwrap_or_else(|e| e.clone())
            );
            let diff = call(
                "research_meta_ads",
                &json!({"op": "compare", "name": "live-test", "max_ads": 10}),
            )
            .await;
            println!(
                "ADS_COMPARE {}",
                diff.as_ref().map(short).unwrap_or_else(|e| e.clone())
            );
            assert!(base.is_ok() && diff.is_ok());
        }

        let reels = call(
            "research_reels",
            &json!({"profiles": ["nike", "adidas"], "sample": 5}),
        )
        .await;
        println!(
            "REELS {}",
            reels.as_ref().map(short).unwrap_or_else(|e| e.clone())
        );

        for (name, result) in [
            ("creators", &creators),
            ("brief", &brief),
            ("ads", &found),
            ("reels", &reels),
        ] {
            let text = result
                .as_ref()
                .map(|v| v.to_string())
                .unwrap_or_else(|e| e.clone());
            assert!(!text.contains(&key), "{name} leaked the key");
            assert!(result.is_ok(), "{name}: {text}");
        }
    }
}

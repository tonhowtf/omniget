use std::collections::HashMap;
use std::sync::Arc;

fn cookie_domain_matches(
    request_domain: &str,
    cookie_domain: &str,
    include_subdomains: bool,
) -> bool {
    if include_subdomains {
        request_domain == cookie_domain || request_domain.ends_with(&format!(".{}", cookie_domain))
    } else {
        request_domain == cookie_domain
    }
}

pub fn load_extension_cookies_for_domain(domain: &str) -> Option<Arc<reqwest::cookie::Jar>> {
    let cookie_path = crate::core::ytdlp::ext_cookie_path_if_fresh()?;
    let content = std::fs::read_to_string(&cookie_path).ok()?;

    let jar = reqwest::cookie::Jar::default();
    let mut count = 0usize;
    let request_domain = domain.trim_start_matches('.').to_lowercase();

    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        let is_http_only_line = line.starts_with("#HttpOnly_");
        let effective_line = if is_http_only_line {
            &line["#HttpOnly_".len()..]
        } else if line.starts_with('#') {
            continue;
        } else {
            line
        };

        let parts: Vec<&str> = effective_line.split('\t').collect();
        if parts.len() < 7 {
            continue;
        }

        let raw_domain = parts[0];
        let include_subdomains = parts[1].eq_ignore_ascii_case("TRUE");
        let cookie_domain = raw_domain.trim_start_matches('.').to_lowercase();

        if !cookie_domain_matches(&request_domain, &cookie_domain, include_subdomains) {
            continue;
        }

        let name = parts[5];
        let value = parts[6];
        let url_scheme = if parts[3].eq_ignore_ascii_case("TRUE") {
            "https"
        } else {
            "http"
        };
        let url_str = format!("{}://{}/", url_scheme, cookie_domain);

        if let Ok(url) = url_str.parse::<reqwest::Url>() {
            let cookie_str = format!("{}={}", name, value);
            jar.add_cookie_str(&cookie_str, &url);
            count += 1;
        }
    }

    if count == 0 {
        return None;
    }

    tracing::debug!(
        "[cookies] loaded {} extension cookies for {}",
        count,
        domain
    );
    Some(Arc::new(jar))
}

pub fn load_extension_cookies_for_url(url: &str) -> Option<Arc<reqwest::cookie::Jar>> {
    let domains = normalize_cookie_domains(url);
    for domain in &domains {
        if let Some(jar) = load_extension_cookies_for_domain(domain) {
            return Some(jar);
        }
    }
    None
}

fn normalize_cookie_domains(url: &str) -> Vec<String> {
    let parsed = match url::Url::parse(url) {
        Ok(p) => p,
        Err(_) => return vec![],
    };
    let host = match parsed.host_str() {
        Some(h) => h.to_lowercase(),
        None => return vec![],
    };

    let mut domains = vec![];

    let parts: Vec<&str> = host.split('.').collect();
    if parts.len() >= 2 {
        domains.push(format!(
            "{}.{}",
            parts[parts.len() - 2],
            parts[parts.len() - 1]
        ));
    }

    if host.contains("cdninstagram.com") || host.contains("fbcdn.net") {
        domains.push("instagram.com".to_string());
    }
    if host.contains("twimg.com") {
        domains.push("x.com".to_string());
        domains.push("twitter.com".to_string());
    }
    if host.contains("redd.it") || host.contains("redditstatic.com") {
        domains.push("reddit.com".to_string());
    }
    if host.contains("pstatic.net") || host.contains("pinimg.com") {
        domains.push("pinterest.com".to_string());
    }
    if host.contains("tiktokcdn.com") || host.contains("tiktokv.com") {
        domains.push("tiktok.com".to_string());
    }
    if host.contains("biliapi.net") || host.contains("bilivideo.com") || host.contains("hdslb.com")
    {
        domains.push("bilibili.com".to_string());
    }
    if host.contains("googlevideo.com") || host.contains("ytimg.com") {
        domains.push("youtube.com".to_string());
        domains.push("google.com".to_string());
    }

    domains
}

pub struct ParsedInput {
    pub token: String,
    pub cookie_string: String,
    pub cookies: HashMap<String, String>,
    pub extra_fields: HashMap<String, String>,
}

pub fn parse_cookie_input(input: &str, target_cookie: &str) -> ParsedInput {
    let trimmed = input.trim();

    if trimmed.starts_with('{') || trimmed.starts_with('[') {
        if let Ok(val) = serde_json::from_str::<serde_json::Value>(trimmed) {
            let cookie_array = if let Some(arr) = val.get("cookies").and_then(|c| c.as_array()) {
                arr.clone()
            } else if let Some(arr) = val.as_array() {
                arr.clone()
            } else if val.get("name").is_some() && val.get("value").is_some() {
                vec![val.clone()]
            } else {
                Vec::new()
            };

            if !cookie_array.is_empty() {
                let mut cookies = HashMap::new();
                let mut parts = Vec::new();

                for cookie_obj in &cookie_array {
                    if let (Some(name), Some(value)) = (
                        cookie_obj.get("name").and_then(|n| n.as_str()),
                        cookie_obj.get("value").and_then(|v| v.as_str()),
                    ) {
                        cookies.insert(name.to_string(), value.to_string());
                        parts.push(format!("{}={}", name, value));
                    }
                }

                let cookie_string = parts.join("; ");

                let token = if !target_cookie.is_empty() {
                    if let Some(t) = cookies.get(target_cookie) {
                        t.clone()
                    } else {
                        cookies
                            .values()
                            .find(|v| v.starts_with("eyJ"))
                            .cloned()
                            .unwrap_or_default()
                    }
                } else {
                    cookies
                        .values()
                        .find(|v| v.starts_with("eyJ"))
                        .cloned()
                        .unwrap_or_default()
                };

                return ParsedInput {
                    token,
                    cookie_string,
                    cookies,
                    extra_fields: HashMap::new(),
                };
            }
        }
    }

    if trimmed.contains("; ") || (trimmed.contains('=') && !trimmed.starts_with("eyJ")) {
        let mut cookies = HashMap::new();
        for pair in trimmed.split("; ") {
            if let Some(idx) = pair.find('=') {
                let name = pair[..idx].trim().to_string();
                let value = pair[idx + 1..].trim().to_string();
                cookies.insert(name, value);
            }
        }

        let token = if !target_cookie.is_empty() {
            cookies.get(target_cookie).cloned().unwrap_or_default()
        } else {
            cookies
                .values()
                .find(|v| v.starts_with("eyJ"))
                .cloned()
                .unwrap_or_default()
        };

        return ParsedInput {
            token,
            cookie_string: trimmed.to_string(),
            cookies,
            extra_fields: HashMap::new(),
        };
    }

    let token = trimmed.to_string();
    let cookie_string = if !target_cookie.is_empty() {
        format!("{}={}", target_cookie, token)
    } else {
        String::new()
    };
    let mut cookies = HashMap::new();
    if !target_cookie.is_empty() {
        cookies.insert(target_cookie.to_string(), token.clone());
    }

    ParsedInput {
        token,
        cookie_string,
        cookies,
        extra_fields: HashMap::new(),
    }
}

pub fn parse_bearer_input(input: &str) -> String {
    let trimmed = input.trim();

    if trimmed.starts_with('{') || trimmed.starts_with('[') {
        if let Ok(val) = serde_json::from_str::<serde_json::Value>(trimmed) {
            for key in &[
                "access_token",
                "token",
                "idToken",
                "bearerToken",
                "bearer_token",
            ] {
                if let Some(t) = val.get(*key).and_then(|v| v.as_str()) {
                    return t.to_string();
                }
            }

            let cookie_array = if let Some(arr) = val.get("cookies").and_then(|c| c.as_array()) {
                arr.clone()
            } else if let Some(arr) = val.as_array() {
                arr.clone()
            } else {
                Vec::new()
            };

            for cookie_obj in &cookie_array {
                if let Some(value) = cookie_obj.get("value").and_then(|v| v.as_str()) {
                    if value.starts_with("eyJ") && value.len() > 50 {
                        return value.to_string();
                    }
                }
            }

            for cookie_obj in &cookie_array {
                if let (Some(name), Some(value)) = (
                    cookie_obj.get("name").and_then(|n| n.as_str()),
                    cookie_obj.get("value").and_then(|v| v.as_str()),
                ) {
                    let lower = name.to_lowercase();
                    if (lower.contains("token")
                        || lower.contains("auth")
                        || lower.contains("session")
                        || lower.contains("sid"))
                        && value.len() > 20
                    {
                        return value.to_string();
                    }
                }
            }
        }
    }

    trimmed.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_only_matches_exact_only() {
        assert!(cookie_domain_matches("example.com", "example.com", false));
        assert!(!cookie_domain_matches(
            "sub.example.com",
            "example.com",
            false
        ));
        assert!(!cookie_domain_matches(
            "example.com",
            "sub.example.com",
            false
        ));
    }

    #[test]
    fn include_subdomains_allows_proper_suffix() {
        assert!(cookie_domain_matches("example.com", "example.com", true));
        assert!(cookie_domain_matches(
            "sub.example.com",
            "example.com",
            true
        ));
        assert!(cookie_domain_matches(
            "a.b.example.com",
            "example.com",
            true
        ));
    }

    #[test]
    fn substring_false_positive_rejected() {
        assert!(!cookie_domain_matches("foo.com", "oo.com", true));
        assert!(!cookie_domain_matches("foo.com", "oo.com", false));
        assert!(!cookie_domain_matches(
            "notexample.com",
            "example.com",
            true
        ));
    }

    #[test]
    fn parent_does_not_match_child_cookie() {
        assert!(!cookie_domain_matches(
            "example.com",
            "sub.example.com",
            true
        ));
    }

    #[test]
    fn normalize_strips_subdomain_to_apex() {
        let d = normalize_cookie_domains("https://www.example.com/path");
        assert_eq!(d.first().map(|s| s.as_str()), Some("example.com"));
    }

    #[test]
    fn normalize_preserves_apex_when_already_two_parts() {
        let d = normalize_cookie_domains("https://example.com/");
        assert_eq!(d.first().map(|s| s.as_str()), Some("example.com"));
    }

    #[test]
    fn normalize_maps_instagram_cdn_to_instagram_com() {
        let d = normalize_cookie_domains("https://scontent.cdninstagram.com/v/t51/image.jpg");
        assert!(d.contains(&"instagram.com".to_string()));

        let d2 = normalize_cookie_domains("https://static.fbcdn.net/foo");
        assert!(d2.contains(&"instagram.com".to_string()));
    }

    #[test]
    fn normalize_maps_twimg_to_twitter_and_x() {
        let d = normalize_cookie_domains("https://pbs.twimg.com/media/ABC.jpg");
        assert!(d.contains(&"x.com".to_string()));
        assert!(d.contains(&"twitter.com".to_string()));
    }

    #[test]
    fn normalize_maps_redd_it_to_reddit_com() {
        let d = normalize_cookie_domains("https://v.redd.it/abc123");
        assert!(d.contains(&"reddit.com".to_string()));

        let d2 = normalize_cookie_domains("https://www.redditstatic.com/foo.js");
        assert!(d2.contains(&"reddit.com".to_string()));
    }

    #[test]
    fn normalize_maps_pinimg_to_pinterest_com() {
        let d = normalize_cookie_domains("https://i.pinimg.com/236x/ab/cd/ef/x.jpg");
        assert!(d.contains(&"pinterest.com".to_string()));
    }

    #[test]
    fn normalize_maps_tiktokcdn_to_tiktok_com() {
        let d = normalize_cookie_domains("https://v16-webapp.tiktokcdn.com/video.mp4");
        assert!(d.contains(&"tiktok.com".to_string()));

        let d2 = normalize_cookie_domains("https://v16-cold.tiktokv.com/video.mp4");
        assert!(d2.contains(&"tiktok.com".to_string()));
    }

    #[test]
    fn normalize_maps_bilivideo_to_bilibili_com() {
        let d = normalize_cookie_domains("https://upos-sz-mirrorhw.bilivideo.com/x.mp4");
        assert!(d.contains(&"bilibili.com".to_string()));

        let d2 = normalize_cookie_domains("https://i0.hdslb.com/bfs/cover.jpg");
        assert!(d2.contains(&"bilibili.com".to_string()));
    }

    #[test]
    fn normalize_maps_googlevideo_to_youtube_and_google() {
        let d = normalize_cookie_domains("https://rr1---sn-abcd.googlevideo.com/videoplayback");
        assert!(d.contains(&"youtube.com".to_string()));
        assert!(d.contains(&"google.com".to_string()));

        let d2 = normalize_cookie_domains("https://i.ytimg.com/vi/abc/hq.jpg");
        assert!(d2.contains(&"youtube.com".to_string()));
    }

    #[test]
    fn normalize_returns_empty_for_unparsable_url() {
        let d = normalize_cookie_domains("::not a url::");
        assert!(d.is_empty());
    }

    #[test]
    fn normalize_returns_empty_for_url_without_host() {
        let d = normalize_cookie_domains("file:///tmp/foo");
        assert!(d.is_empty());
    }

    #[test]
    fn parse_cookie_input_handles_semicolon_format() {
        let parsed = parse_cookie_input("sessionid=abc; csrftoken=xyz", "sessionid");

        assert_eq!(parsed.token, "abc");
        assert_eq!(
            parsed.cookies.get("sessionid").map(|s| s.as_str()),
            Some("abc")
        );
        assert_eq!(
            parsed.cookies.get("csrftoken").map(|s| s.as_str()),
            Some("xyz")
        );
    }

    #[test]
    fn parse_cookie_input_semicolon_format_with_missing_target_returns_empty_token() {
        let parsed = parse_cookie_input(
            "other=foo; auth=eyJhbGciOiJIUzI1NiJ9.payload.sig",
            "nonexistent",
        );

        assert_eq!(parsed.token, "");
        assert_eq!(parsed.cookies.len(), 2);
    }

    #[test]
    fn parse_cookie_input_json_array_with_missing_target_falls_back_to_jwt_prefix() {
        let input = r#"[{"name":"other","value":"foo"},{"name":"auth","value":"eyJabcdef"}]"#;
        let parsed = parse_cookie_input(input, "nonexistent");

        assert!(parsed.token.starts_with("eyJ"));
    }

    #[test]
    fn parse_cookie_input_parses_json_array() {
        let input = r#"[
            {"name":"sessionid","value":"abc"},
            {"name":"csrftoken","value":"xyz"}
        ]"#;
        let parsed = parse_cookie_input(input, "sessionid");

        assert_eq!(parsed.token, "abc");
        assert_eq!(parsed.cookies.len(), 2);
        assert!(parsed.cookie_string.contains("sessionid=abc"));
        assert!(parsed.cookie_string.contains("csrftoken=xyz"));
    }

    #[test]
    fn parse_cookie_input_parses_json_object_with_cookies_array() {
        let input = r#"{"cookies":[{"name":"x","value":"1"}]}"#;
        let parsed = parse_cookie_input(input, "x");

        assert_eq!(parsed.token, "1");
        assert_eq!(parsed.cookies.get("x").map(|s| s.as_str()), Some("1"));
    }

    #[test]
    fn parse_cookie_input_treats_raw_token_with_no_equals_as_token_only() {
        let parsed = parse_cookie_input("just_a_token_value", "sessionid");

        assert_eq!(parsed.token, "just_a_token_value");
        assert_eq!(
            parsed.cookies.get("sessionid").map(|s| s.as_str()),
            Some("just_a_token_value"),
        );
        assert_eq!(parsed.cookie_string, "sessionid=just_a_token_value");
    }

    #[test]
    fn parse_cookie_input_accepts_single_cookie_json_object() {
        let input = r#"{"name":"single","value":"v"}"#;
        let parsed = parse_cookie_input(input, "single");

        assert_eq!(parsed.token, "v");
        assert_eq!(parsed.cookies.get("single").map(|s| s.as_str()), Some("v"));
    }

    #[test]
    fn parse_cookie_input_empty_target_finds_jwt_value() {
        let parsed = parse_cookie_input("a=b; token=eyJabcdef", "");

        assert_eq!(parsed.token, "eyJabcdef");
    }

    #[test]
    fn parse_bearer_input_extracts_access_token_from_json() {
        let input = r#"{"access_token":"secret123"}"#;

        assert_eq!(parse_bearer_input(input), "secret123");
    }

    #[test]
    fn parse_bearer_input_prefers_access_token_over_token_field() {
        let input = r#"{"access_token":"want_this","token":"not_this"}"#;

        assert_eq!(parse_bearer_input(input), "want_this");
    }

    #[test]
    fn parse_bearer_input_extracts_jwt_value_from_cookies_array() {
        let input = r#"[{"name":"sess","value":"eyJhbGciOiJIUzI1NiJ9.payloadpayloadpayloadpayloadpayloadpayload.sig"}]"#;

        let out = parse_bearer_input(input);
        assert!(out.starts_with("eyJ"));
        assert!(out.len() > 50);
    }

    #[test]
    fn parse_bearer_input_falls_back_to_auth_like_cookie_name() {
        let input =
            r#"{"cookies":[{"name":"auth_session","value":"long_enough_session_value_xyz"}]}"#;

        assert_eq!(parse_bearer_input(input), "long_enough_session_value_xyz");
    }

    #[test]
    fn parse_bearer_input_returns_raw_when_not_json() {
        let input = "raw_bearer_token_xyz";

        assert_eq!(parse_bearer_input(input), "raw_bearer_token_xyz");
    }

    #[test]
    fn parse_bearer_input_rejects_short_auth_values() {
        let input = r#"{"cookies":[{"name":"auth","value":"short"}]}"#;

        assert_eq!(parse_bearer_input(input), input);
    }

    #[test]
    fn netscape_leading_dot_in_cookie_domain_matches_subdomain() {
        let raw_domain = ".example.com";
        let cookie_domain = raw_domain.trim_start_matches('.').to_lowercase();
        let request_domain = "sub.example.com".trim_start_matches('.').to_lowercase();
        let include_subdomains = raw_domain.starts_with('.');

        assert!(cookie_domain_matches(
            &request_domain,
            &cookie_domain,
            include_subdomains
        ));
    }

    #[test]
    fn netscape_no_leading_dot_is_host_only() {
        let raw_domain = "example.com";
        let cookie_domain = raw_domain.trim_start_matches('.').to_lowercase();
        let request_domain = "sub.example.com".trim_start_matches('.').to_lowercase();

        assert!(!cookie_domain_matches(
            &request_domain,
            &cookie_domain,
            false
        ));
    }
}

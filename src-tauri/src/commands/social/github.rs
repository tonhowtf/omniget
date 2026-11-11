use std::sync::OnceLock;
use std::time::Duration;

use rusqlite::OptionalExtension;
use serde_json::{json, Value};

use super::args::{opt_str, req_str, undo};
use super::model::{
    opt_s, page, s, u, with_extra, Author, Comment, Platform, Post, Profile, Stats,
};
use super::store;

const P: Platform = Platform::Github;
const API: &str = "https://api.github.com";
const JSON: &str = "application/vnd.github+json";
const STAR_JSON: &str = "application/vnd.github.star+json";
const RAW: &str = "application/vnd.github.raw+json";
const PER_PAGE: u32 = 30;
const EXCERPT: usize = 1500;
const SCHEMA: &str =
    "CREATE TABLE IF NOT EXISTS github_config (key TEXT PRIMARY KEY, value TEXT NOT NULL);";
const SECRETS: crate::secrets::Namespace = crate::secrets::Namespace {
    service: "wtf.tonho.omniget.social",
    dir_name: "social",
    dir_env: "OMNIGET_SOCIAL_SECRET_DIR",
};
const TOKEN: &str = "github_token";

pub struct Reply {
    pub status: u16,
    pub body: String,
    pub link: Option<String>,
    pub remaining: Option<u64>,
    pub reset: Option<i64>,
    pub retry_after: Option<u64>,
}

fn token() -> Option<String> {
    crate::secrets::load_secret(SECRETS, TOKEN)
        .ok()
        .flatten()
        .filter(|t| !t.is_empty())
}

fn username() -> Option<String> {
    store::with(SCHEMA, |c| {
        c.query_row(
            "SELECT value FROM github_config WHERE key = 'username'",
            [],
            |r| r.get::<_, String>(0),
        )
        .optional()
    })
    .ok()
    .flatten()
    .filter(|n| !n.is_empty())
}

fn set_username(name: &str) -> Result<(), String> {
    store::with(SCHEMA, |c| {
        if name.is_empty() {
            c.execute("DELETE FROM github_config WHERE key = 'username'", [])
        } else {
            c.execute(
                "INSERT INTO github_config (key, value) VALUES ('username', ?1)
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                [name],
            )
        }
    })
    .map(|_| ())
}

pub fn auth_state() -> Value {
    json!({ "has_token": token().is_some(), "username": username() })
}

pub fn status() -> Value {
    let has_token = token().is_some();
    let named = username().is_some();
    json!({
        "platform": P.id(),
        "browser": false,
        "logged_in": has_token || named,
        "auth": if has_token { "token" } else if named { "username" } else { "none" },
    })
}

pub async fn connect(new_token: Option<String>, name: Option<String>) -> Result<Value, String> {
    if let Some(t) = new_token.map(|t| t.trim().to_string()) {
        if t.is_empty() {
            crate::secrets::delete_secret(SECRETS, TOKEN)?;
        } else {
            let r = send(reqwest::Method::GET, "/user", JSON, None, Some(&t)).await?;
            if let Some(e) = failure(&r, true, now()) {
                return Err(e);
            }
            crate::secrets::save_secret(SECRETS, TOKEN, &t)?;
        }
    }
    if let Some(n) = name {
        let n = n.trim();
        if n.is_empty() {
            set_username("")?;
        } else {
            let login = login_of(n).ok_or("INVALID_ARGUMENTS: that is not a GitHub username")?;
            get(&format!("/users/{login}")).await?;
            set_username(&login)?;
        }
    }
    Ok(auth_state())
}

fn now() -> i64 {
    chrono::Utc::now().timestamp()
}

fn token_required(what: &str) -> String {
    format!(
        "TOKEN_REQUIRED: {}",
        json!({
            "message": format!("{what} needs a GitHub token"),
            "retryable": false,
            "next_step": "Ask the person to open the GitHub screen in OmniGet and save a personal access token there. Never ask for the token in the chat.",
        })
    )
}

pub fn failure(r: &Reply, has_token: bool, now: i64) -> Option<String> {
    if (200..300).contains(&r.status) {
        return None;
    }
    let msg = serde_json::from_str::<Value>(&r.body)
        .ok()
        .and_then(|v| opt_s(&v["message"]))
        .unwrap_or_default();
    let limited = r.status == 429
        || (r.status == 403
            && (r.remaining == Some(0)
                || r.retry_after.is_some()
                || msg.to_ascii_lowercase().contains("rate limit")));
    if limited {
        let wait = r
            .retry_after
            .map(|x| x as i64)
            .or_else(|| r.reset.map(|t| (t - now).max(0)));
        let when = r
            .reset
            .and_then(|t| chrono::DateTime::from_timestamp(t, 0))
            .map(|d| d.to_rfc3339_opts(chrono::SecondsFormat::Secs, true));
        let mut out = String::from("RATE_LIMITED: GitHub's request limit is used up");
        if let Some(w) = when {
            out.push_str(&format!("; it resets at {w}"));
        }
        if let Some(w) = wait {
            out.push_str(&format!(" (in {} min)", (w + 59) / 60));
        }
        if !has_token {
            out.push_str(". Without a token GitHub allows 60 requests an hour; a token saved in the GitHub screen raises it to 5000");
        }
        return Some(out);
    }
    Some(match r.status {
        0 => "NETWORK: could not reach GitHub".to_string(),
        401 => "NOT_LOGGED_IN: GitHub rejected the saved token (expired or revoked); save a new one in the GitHub screen".to_string(),
        403 => format!("REJECTED: GitHub refused the request: {msg} (the token may lack that permission)"),
        404 => "NOT_FOUND: GitHub has nothing at that address, or the token cannot see it".to_string(),
        422 => format!("INVALID_ARGUMENTS: GitHub rejected the request: {msg}"),
        n => format!("UPSTREAM_{n}: GitHub answered {n}: {msg}"),
    })
}

pub fn next_page(link: Option<&str>) -> Option<String> {
    let re = regex::Regex::new(r#"<([^>]+)>;\s*rel="next""#).expect("static regex");
    let url = re.captures(link?)?.get(1)?.as_str().to_string();
    let query = url.split_once('?')?.1;
    query
        .split('&')
        .find_map(|kv| kv.strip_prefix("page="))
        .filter(|p| !p.is_empty() && p.bytes().all(|c| c.is_ascii_digit()))
        .map(str::to_string)
}

fn client() -> &'static reqwest::Client {
    static C: OnceLock<reqwest::Client> = OnceLock::new();
    C.get_or_init(|| {
        reqwest::Client::builder()
            .use_rustls_tls()
            .timeout(Duration::from_secs(20))
            .user_agent("social-desktop-client")
            .build()
            .unwrap_or_default()
    })
}

async fn send(
    method: reqwest::Method,
    path: &str,
    accept: &str,
    body: Option<&Value>,
    bearer: Option<&str>,
) -> Result<Reply, String> {
    let url = if path.starts_with("https://") {
        path.to_string()
    } else {
        format!("{API}{path}")
    };
    let mut req = client()
        .request(method.clone(), url)
        .header("Accept", accept);
    if accept != "text/html" {
        req = req.header("X-GitHub-Api-Version", "2022-11-28");
        if let Some(t) = bearer {
            req = req.bearer_auth(t);
        }
    }
    req = match body {
        Some(b) => req.json(b),
        None if method == reqwest::Method::PUT => req.header("Content-Length", "0"),
        None => req,
    };
    let r = req
        .send()
        .await
        .map_err(|_| "NETWORK: could not reach GitHub".to_string())?;
    let head = |k: &str| {
        r.headers()
            .get(k)
            .and_then(|v| v.to_str().ok())
            .map(str::to_string)
    };
    let link = head("link");
    let remaining = head("x-ratelimit-remaining").and_then(|v| v.parse().ok());
    let reset = head("x-ratelimit-reset").and_then(|v| v.parse().ok());
    let retry_after = head("retry-after").and_then(|v| v.parse().ok());
    let status = r.status().as_u16();
    let body = r
        .text()
        .await
        .map_err(|_| "NETWORK: GitHub closed the connection".to_string())?;
    Ok(Reply {
        status,
        body,
        link,
        remaining,
        reset,
        retry_after,
    })
}

async fn request(
    method: reqwest::Method,
    path: &str,
    accept: &str,
    body: Option<&Value>,
) -> Result<Reply, String> {
    let t = token();
    let r = send(method, path, accept, body, t.as_deref()).await?;
    match failure(&r, t.is_some(), now()) {
        Some(e) => Err(e),
        None => Ok(r),
    }
}

fn parse(r: &Reply) -> Result<Value, String> {
    if r.body.trim().is_empty() {
        return Ok(Value::Null);
    }
    serde_json::from_str(&r.body).map_err(|_| {
        format!(
            "UPSTREAM_CHANGED: GitHub answered {} with something that is not JSON",
            r.status
        )
    })
}

async fn get(path: &str) -> Result<Value, String> {
    parse(&request(reqwest::Method::GET, path, JSON, None).await?)
}

async fn list(path: &str, accept: &str) -> Result<(Vec<Value>, Option<String>), String> {
    let r = request(reqwest::Method::GET, path, accept, None).await?;
    let v = parse(&r)?;
    let items = match v {
        Value::Array(a) => a,
        Value::Object(mut o) => match o.remove("items") {
            Some(Value::Array(a)) => a,
            _ => return Err("UPSTREAM_CHANGED: GitHub did not return a list".into()),
        },
        _ => return Err("UPSTREAM_CHANGED: GitHub did not return a list".into()),
    };
    Ok((items, next_page(r.link.as_deref())))
}

async fn write(method: reqwest::Method, path: &str, body: Option<Value>) -> Result<Value, String> {
    if token().is_none() {
        return Err(token_required("Acting on GitHub"));
    }
    parse(&request(method, path, JSON, body.as_ref()).await?)
}

pub fn listing_url(path: &str, params: &[(&str, &str)], cursor: Option<&str>) -> String {
    let mut q: Vec<String> = params
        .iter()
        .filter(|(_, v)| !v.is_empty())
        .map(|(k, v)| format!("{k}={}", urlencoding::encode(v)))
        .collect();
    q.push(format!("per_page={PER_PAGE}"));
    let page = cursor
        .filter(|c| !c.is_empty() && c.len() <= 4 && c.bytes().all(|b| b.is_ascii_digit()))
        .unwrap_or("1");
    q.push(format!("page={page}"));
    format!("{path}?{}", q.join("&"))
}

pub fn search_url(
    kind: &str,
    query: &str,
    sort: Option<&str>,
    order: Option<&str>,
    cursor: Option<&str>,
) -> Result<String, String> {
    let (path, sorts): (&str, &[&str]) = match kind {
        "repositories" => (
            "/search/repositories",
            &["stars", "forks", "updated", "help-wanted-issues"],
        ),
        "users" => ("/search/users", &["followers", "repositories", "joined"]),
        "issues" => (
            "/search/issues",
            &["comments", "reactions", "created", "updated"],
        ),
        "code" => ("/search/code", &["indexed"]),
        _ => {
            return Err(
                "INVALID_ARGUMENTS: kind must be repositories, users, issues or code".into(),
            )
        }
    };
    if let Some(x) = sort {
        if !sorts.contains(&x) {
            return Err(format!(
                "INVALID_ARGUMENTS: sort for {kind} must be one of {}",
                sorts.join(", ")
            ));
        }
    }
    let order = match order {
        Some("asc") => "asc",
        _ if sort.is_some() => "desc",
        _ => "",
    };
    Ok(listing_url(
        path,
        &[("q", query), ("sort", sort.unwrap_or("")), ("order", order)],
        cursor,
    ))
}

fn valid_name(x: &str, extra: &[char]) -> bool {
    !x.is_empty()
        && x.len() <= 100
        && x != "."
        && x != ".."
        && x.chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || extra.contains(&c))
}

fn path_of(input: &str) -> &str {
    let t = input.trim();
    let t = match t.split_once("github.com/") {
        Some((_, rest)) => rest,
        None => t,
    };
    t.split(['?', '#'])
        .next()
        .unwrap_or_default()
        .trim_matches('/')
}

pub fn login_of(input: &str) -> Option<String> {
    let first = path_of(input.trim().trim_start_matches('@'))
        .split('/')
        .next()?
        .to_string();
    (valid_name(&first, &[]) && first.len() <= 39).then_some(first)
}

pub fn repo_of(input: &str) -> Option<String> {
    let mut parts = path_of(input).split('/');
    let owner = parts.next()?;
    let name = parts.next()?.trim_end_matches(".git");
    (valid_name(owner, &[]) && valid_name(name, &['.', '_'])).then(|| format!("{owner}/{name}"))
}

pub fn issue_ref(input: &str) -> Option<(String, u64)> {
    let t = input.trim();
    if let Some((repo, n)) = t.rsplit_once('#') {
        if !repo.contains("://") {
            return Some((repo_of(repo)?, n.parse().ok()?));
        }
    }
    let re = regex::Regex::new(r"github\.com/([^/]+/[^/]+)/(?:issues|pull)/(\d+)")
        .expect("static regex");
    let c = re.captures(t)?;
    Some((repo_of(&c[1])?, c[2].parse().ok()?))
}

fn ts(v: &Value) -> Option<i64> {
    chrono::DateTime::parse_from_rfc3339(v.as_str()?)
        .ok()
        .map(|d| d.timestamp())
}

fn excerpt(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let mut out: String = text.chars().take(max).collect();
    out.push('…');
    out
}

pub fn author(v: &Value) -> Author {
    Author {
        id: s(&v["id"]),
        handle: opt_s(&v["login"])
            .or_else(|| opt_s(&v["display_login"]))
            .unwrap_or_default(),
        name: opt_s(&v["name"]).unwrap_or_default(),
        avatar: opt_s(&v["avatar_url"]),
        verified: false,
    }
}

pub fn profile(v: &Value, extra: Value) -> Option<Value> {
    let login = opt_s(&v["login"])?;
    let mut e = json!({
        "kind": opt_s(&v["type"]).map(|t| t.to_ascii_lowercase()),
        "company": opt_s(&v["company"]),
        "location": opt_s(&v["location"]),
        "blog": opt_s(&v["blog"]),
        "created_at": ts(&v["created_at"]),
        "gists": u(&v["public_gists"]),
    });
    if let (Some(o), Some(x)) = (e.as_object_mut(), extra.as_object()) {
        o.extend(x.clone());
    }
    Some(with_extra(
        &Profile {
            id: s(&v["id"]),
            url: opt_s(&v["html_url"]).unwrap_or_else(|| format!("https://github.com/{login}")),
            handle: login,
            name: s(&v["name"]),
            avatar: opt_s(&v["avatar_url"]),
            bio: s(&v["bio"]),
            followers: u(&v["followers"]),
            following: u(&v["following"]),
            posts: u(&v["public_repos"]),
            ..Default::default()
        },
        e,
    ))
}

pub fn repo_post(v: &Value) -> Option<Value> {
    let full = opt_s(&v["full_name"])?;
    Some(with_extra(
        &Post {
            url: opt_s(&v["html_url"]).unwrap_or_else(|| format!("https://github.com/{full}")),
            author: author(&v["owner"]),
            text: s(&v["description"]),
            created_at: ts(&v["pushed_at"]).or_else(|| ts(&v["updated_at"])),
            stats: Stats {
                likes: u(&v["stargazers_count"]),
                replies: u(&v["open_issues_count"]),
                reposts: u(&v["forks_count"]),
                views: u(&v["subscribers_count"]),
                saves: None,
            },
            id: full.clone(),
            ..Default::default()
        },
        json!({
            "kind": "repo",
            "title": full,
            "language": opt_s(&v["language"]),
            "topics": v["topics"].as_array().filter(|a| !a.is_empty()),
            "license": opt_s(&v["license"]["spdx_id"]),
            "homepage": opt_s(&v["homepage"]),
            "default_branch": opt_s(&v["default_branch"]),
            "fork": v["fork"].as_bool().filter(|x| *x),
            "archived": v["archived"].as_bool().filter(|x| *x),
            "created_at": ts(&v["created_at"]),
        }),
    ))
}

fn repo_of_api_url(v: &Value) -> Option<String> {
    v.as_str()?
        .split_once("/repos/")
        .and_then(|(_, r)| repo_of(r))
}

pub fn issue_post(v: &Value) -> Option<Value> {
    let number = u(&v["number"])?;
    let repo = repo_of_api_url(&v["repository_url"])
        .or_else(|| repo_of(v["html_url"].as_str().unwrap_or_default()))
        .unwrap_or_default();
    let pull = v["pull_request"].is_object();
    Some(with_extra(
        &Post {
            id: format!("{repo}#{number}"),
            url: s(&v["html_url"]),
            author: author(&v["user"]),
            text: excerpt(&s(&v["body"]), EXCERPT * 2),
            created_at: ts(&v["created_at"]),
            stats: Stats {
                likes: u(&v["reactions"]["total_count"]),
                replies: u(&v["comments"]),
                ..Default::default()
            },
            ..Default::default()
        },
        json!({
            "kind": if pull { "pull" } else { "issue" },
            "title": opt_s(&v["title"]),
            "repo": repo,
            "number": number,
            "state": opt_s(&v["state"]),
            "labels": v["labels"].as_array().map(|a| a.iter().filter_map(|l| opt_s(&l["name"])).collect::<Vec<_>>()).filter(|a| !a.is_empty()),
            "updated_at": ts(&v["updated_at"]),
        }),
    ))
}

pub fn release_post(v: &Value, repo: &str) -> Option<Value> {
    let tag = opt_s(&v["tag_name"])?;
    let assets: Vec<Value> = v["assets"]
        .as_array()
        .map(|a| {
            a.iter()
                .map(|x| json!({ "name": s(&x["name"]), "size": u(&x["size"]), "downloads": u(&x["download_count"]), "url": s(&x["browser_download_url"]) }))
                .collect()
        })
        .unwrap_or_default();
    let downloads: u64 = assets.iter().filter_map(|a| u(&a["downloads"])).sum();
    Some(with_extra(
        &Post {
            id: format!("{repo}@{tag}"),
            url: s(&v["html_url"]),
            author: author(&v["author"]),
            text: excerpt(&s(&v["body"]), EXCERPT * 2),
            created_at: ts(&v["published_at"]).or_else(|| ts(&v["created_at"])),
            ..Default::default()
        },
        json!({
            "kind": "release",
            "title": opt_s(&v["name"]).unwrap_or_else(|| tag.clone()),
            "repo": repo,
            "tag": tag,
            "prerelease": v["prerelease"].as_bool().filter(|x| *x),
            "draft": v["draft"].as_bool().filter(|x| *x),
            "downloads": (!assets.is_empty()).then_some(downloads),
            "assets": (!assets.is_empty()).then_some(assets),
        }),
    ))
}

pub fn event_post(e: &Value) -> Option<Value> {
    let id = opt_s(&e["id"])?;
    let kind = s(&e["type"]);
    let repo = s(&e["repo"]["name"]);
    let p = &e["payload"];
    let home = format!("https://github.com/{repo}");
    let action = opt_s(&p["action"]);
    let did = |fallback: &str| {
        let a = action.clone().unwrap_or_else(|| fallback.to_string());
        let mut c = a.chars();
        match c.next() {
            Some(f) => f.to_ascii_uppercase().to_string() + c.as_str(),
            None => a,
        }
    };
    let (title, text, url) = match kind.as_str() {
        "PushEvent" => {
            let branch = s(&p["ref"]).trim_start_matches("refs/heads/").to_string();
            let lines: Vec<String> = p["commits"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .take(5)
                        .filter_map(|c| c["message"].as_str()?.lines().next().map(str::to_string))
                        .collect()
                })
                .unwrap_or_default();
            let url = match opt_s(&p["head"]) {
                Some(head) => format!("{home}/commit/{head}"),
                None => format!("{home}/commits/{branch}"),
            };
            (
                format!("Pushed to {branch} in {repo}"),
                lines.join("\n"),
                url,
            )
        }
        "ReleaseEvent" => {
            let r = &p["release"];
            let name = opt_s(&r["name"]).unwrap_or_else(|| s(&r["tag_name"]));
            (
                format!("Released {name} in {repo}"),
                excerpt(&s(&r["body"]), EXCERPT),
                opt_s(&r["html_url"]).unwrap_or_else(|| format!("{home}/releases")),
            )
        }
        "IssuesEvent" | "PullRequestEvent" => {
            let (key, word, seg) = if kind == "IssuesEvent" {
                ("issue", "issue", "issues")
            } else {
                ("pull_request", "pull request", "pull")
            };
            let i = &p[key];
            let n = u(&i["number"]).or_else(|| u(&p["number"])).unwrap_or(0);
            let name = opt_s(&i["title"])
                .map(|t| format!(": {t}"))
                .unwrap_or_default();
            (
                format!("{} {word} #{n} in {repo}{name}", did("Updated")),
                excerpt(&s(&i["body"]), EXCERPT),
                opt_s(&i["html_url"]).unwrap_or_else(|| format!("{home}/{seg}/{n}")),
            )
        }
        "IssueCommentEvent" => (
            format!(
                "Commented on #{} in {repo}",
                u(&p["issue"]["number"]).unwrap_or(0)
            ),
            excerpt(&s(&p["comment"]["body"]), EXCERPT),
            opt_s(&p["comment"]["html_url"]).unwrap_or(home),
        ),
        "WatchEvent" => (format!("Starred {repo}"), String::new(), home),
        "ForkEvent" => (
            format!("Forked {repo} to {}", s(&p["forkee"]["full_name"])),
            String::new(),
            opt_s(&p["forkee"]["html_url"]).unwrap_or(home),
        ),
        "CreateEvent" => (
            format!("Created {} {} in {repo}", s(&p["ref_type"]), s(&p["ref"])).replace("  ", " "),
            s(&p["description"]),
            home,
        ),
        other => (
            format!("{} in {repo}", other.trim_end_matches("Event")),
            String::new(),
            home,
        ),
    };
    Some(with_extra(
        &Post {
            id,
            url,
            author: author(&e["actor"]),
            text: if text.is_empty() { title.clone() } else { text },
            created_at: ts(&e["created_at"]),
            ..Default::default()
        },
        json!({ "kind": "event", "event": kind, "repo": repo, "title": title, "action": action }),
    ))
}

pub fn notification(n: &Value) -> Option<Value> {
    let id = opt_s(&n["id"])?;
    let repo = s(&n["repository"]["full_name"]);
    let subject = &n["subject"];
    let kind = s(&subject["type"]);
    let number = subject["url"]
        .as_str()
        .and_then(|x| x.rsplit('/').next())
        .filter(|x| x.bytes().all(|c| c.is_ascii_digit()));
    let url = match (kind.as_str(), number) {
        ("Issue", Some(n)) => format!("https://github.com/{repo}/issues/{n}"),
        ("PullRequest", Some(n)) => format!("https://github.com/{repo}/pull/{n}"),
        _ => format!("https://github.com/{repo}"),
    };
    Some(with_extra(
        &Post {
            id,
            url,
            author: author(&n["repository"]["owner"]),
            text: s(&subject["title"]),
            created_at: ts(&n["updated_at"]),
            ..Default::default()
        },
        json!({ "kind": "notification", "title": opt_s(&subject["title"]), "repo": repo, "subject": kind, "reason": opt_s(&n["reason"]), "unread": n["unread"].as_bool() }),
    ))
}

pub fn comment(c: &Value) -> Comment {
    Comment {
        id: s(&c["id"]),
        author: author(&c["user"]),
        text: s(&c["body"]),
        created_at: ts(&c["created_at"]),
        likes: u(&c["reactions"]["total_count"]),
        replies: None,
    }
}

fn unescape(x: &str) -> String {
    x.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&amp;", "&")
}

fn number(x: &str) -> Option<u64> {
    let digits: String = x.chars().filter(char::is_ascii_digit).collect();
    digits.parse().ok()
}

pub fn trending(html: &str) -> Vec<Value> {
    let tag = regex::Regex::new(r"<[^>]+>").expect("static regex");
    let text = |x: &str| {
        unescape(&tag.replace_all(x, ""))
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
    };
    let name =
        regex::Regex::new(r#"<h2[^>]*>\s*<a[^>]*href="/([^"/]+/[^"/]+)""#).expect("static regex");
    let about =
        regex::Regex::new(r#"(?s)<p class="col-9[^"]*"[^>]*>(.*?)</p>"#).expect("static regex");
    let lang =
        regex::Regex::new(r#"itemprop="programmingLanguage"[^>]*>([^<]+)<"#).expect("static regex");
    let count = |seg: &str, piece: &str| {
        regex::Regex::new(&format!(r#"(?s)href="/[^"]+/{seg}"[^>]*>(.*?)</a>"#))
            .ok()
            .and_then(|re| re.captures(piece).and_then(|c| number(&text(&c[1]))))
    };
    let period = regex::Regex::new(r"([\d,]+)\s+stars?\s+(today|this week|this month)")
        .expect("static regex");
    html.split("<article")
        .skip(1)
        .filter_map(|piece| {
            let full = repo_of(&name.captures(piece)?[1])?;
            let (owner, _) = full.split_once('/')?;
            let gained = period.captures(piece);
            Some(with_extra(
                &Post {
                    id: full.clone(),
                    url: format!("https://github.com/{full}"),
                    author: Author {
                        handle: owner.to_string(),
                        avatar: Some(format!("https://github.com/{owner}.png?size=80")),
                        ..Default::default()
                    },
                    text: about
                        .captures(piece)
                        .map(|c| text(&c[1]))
                        .unwrap_or_default(),
                    stats: Stats {
                        likes: count("stargazers", piece),
                        reposts: count("forks", piece),
                        ..Default::default()
                    },
                    ..Default::default()
                },
                json!({
                    "kind": "repo",
                    "title": full,
                    "language": lang.captures(piece).map(|c| c[1].trim().to_string()),
                    "stars_gained": gained.as_ref().and_then(|c| number(&c[1])),
                    "period": gained.as_ref().map(|c| c[2].to_string()),
                }),
            ))
        })
        .collect()
}

fn cursor(a: &Value) -> Option<String> {
    opt_str(a, "cursor")
}

fn need_repo(a: &Value) -> Result<String, String> {
    repo_of(&req_str(a, "repo")?)
        .ok_or_else(|| "INVALID_ARGUMENTS: repo must be owner/name or a github.com link".into())
}

async fn me_value() -> Result<Value, String> {
    if token().is_some() {
        return get("/user").await;
    }
    match username() {
        Some(n) => get(&format!("/users/{n}")).await,
        None => Err("NOT_LOGGED_IN: no GitHub token or username is set; open the GitHub screen in OmniGet and save one".into()),
    }
}

async fn who(a: &Value) -> Result<String, String> {
    match opt_str(a, "username") {
        Some(n) => login_of(&n).ok_or_else(|| {
            "INVALID_ARGUMENTS: username must be a GitHub login or profile link".into()
        }),
        None => match (token(), username()) {
            (None, Some(n)) => Ok(n),
            _ => opt_s(&me_value().await?["login"])
                .ok_or_else(|| "UPSTREAM_CHANGED: GitHub did not return the account".into()),
        },
    }
}

async fn profiles(path: &str, a: &Value) -> Result<Value, String> {
    let (items, next) = list(&listing_url(path, &[], cursor(a).as_deref()), JSON).await?;
    Ok(page(
        items.iter().filter_map(|x| profile(x, json!({}))).collect(),
        next,
    ))
}

async fn posts(
    path: &str,
    params: &[(&str, &str)],
    a: &Value,
    map: impl Fn(&Value) -> Option<Value>,
) -> Result<Value, String> {
    let (items, next) = list(&listing_url(path, params, cursor(a).as_deref()), JSON).await?;
    Ok(page(items.iter().filter_map(map).collect(), next))
}

async fn repo_detail(repo: &str) -> Result<Value, String> {
    let v = get(&format!("/repos/{repo}")).await?;
    let post = repo_post(&v).ok_or("UPSTREAM_CHANGED: GitHub did not return the repository")?;
    let languages = get(&format!("/repos/{repo}/languages")).await.ok();
    let latest = get(&format!("/repos/{repo}/releases/latest"))
        .await
        .ok()
        .and_then(|r| release_post(&r, repo));
    let readme = request(
        reqwest::Method::GET,
        &format!("/repos/{repo}/readme"),
        RAW,
        None,
    )
    .await
    .ok()
    .map(|r| excerpt(&r.body, EXCERPT));
    Ok(json!({
        "repo": post,
        "languages": languages,
        "latest_release": latest,
        "readme_excerpt": readme,
        "starred": Value::Null,
    }))
}

pub async fn call(tool: &str, a: &Value) -> Result<Value, String> {
    match tool {
        "gh_me" => {
            let auth = if token().is_some() {
                "token"
            } else {
                "username"
            };
            profile(&me_value().await?, json!({ "auth": auth }))
                .ok_or_else(|| "UPSTREAM_CHANGED: GitHub did not return the account".into())
        }
        "gh_user" => {
            let n = who(a).await?;
            profile(&get(&format!("/users/{n}")).await?, json!({}))
                .ok_or_else(|| format!("NOT_FOUND: {n} does not exist on GitHub"))
        }
        "gh_user_posts" => {
            let n = who(a).await?;
            posts(&format!("/users/{n}/events/public"), &[], a, event_post).await
        }
        "gh_feed" => {
            let (n, public) = match (token(), username()) {
                (None, Some(n)) => (n, "/public"),
                _ => (who(&json!({})).await?, ""),
            };
            posts(
                &format!("/users/{n}/received_events{public}"),
                &[],
                a,
                event_post,
            )
            .await
        }
        "gh_repos" => {
            let n = who(a).await?;
            let sort = opt_str(a, "sort").unwrap_or_else(|| "pushed".into());
            if !["pushed", "updated", "created", "full_name"].contains(&sort.as_str()) {
                return Err(
                    "INVALID_ARGUMENTS: sort must be pushed, updated, created or full_name".into(),
                );
            }
            posts(
                &format!("/users/{n}/repos"),
                &[("sort", &sort)],
                a,
                repo_post,
            )
            .await
        }
        "gh_repo" => repo_detail(&need_repo(a)?).await,
        "gh_search" => {
            let kind = opt_str(a, "kind").unwrap_or_else(|| "repositories".into());
            if kind == "code" && token().is_none() {
                return Err(token_required("Code search"));
            }
            let url = search_url(
                &kind,
                &req_str(a, "query")?,
                opt_str(a, "sort").as_deref(),
                opt_str(a, "order").as_deref(),
                cursor(a).as_deref(),
            )?;
            let (items, next) = list(&url, JSON).await?;
            let mapped: Vec<Value> = match kind.as_str() {
                "repositories" => items.iter().filter_map(repo_post).collect(),
                "users" => items.iter().filter_map(|x| profile(x, json!({}))).collect(),
                "issues" => items.iter().filter_map(issue_post).collect(),
                _ => items
                    .iter()
                    .map(|x| json!({ "repo": s(&x["repository"]["full_name"]), "path": s(&x["path"]), "url": s(&x["html_url"]) }))
                    .collect(),
            };
            Ok(page(mapped, next))
        }
        "gh_issues" => {
            let (repo, number) = match opt_str(a, "post").and_then(|p| issue_ref(&p)) {
                Some((r, n)) => (r, Some(n)),
                None => (need_repo(a)?, u(&a["number"])),
            };
            if let Some(n) = number {
                let post = issue_post(&get(&format!("/repos/{repo}/issues/{n}")).await?)
                    .ok_or("UPSTREAM_CHANGED: GitHub did not return the issue")?;
                let (items, next) = list(
                    &listing_url(
                        &format!("/repos/{repo}/issues/{n}/comments"),
                        &[],
                        cursor(a).as_deref(),
                    ),
                    JSON,
                )
                .await?;
                let comments: Vec<Comment> = items.iter().map(comment).collect();
                return Ok(json!({ "post": post, "comments": page(comments, next) }));
            }
            let state = opt_str(a, "state").unwrap_or_else(|| "open".into());
            if !["open", "closed", "all"].contains(&state.as_str()) {
                return Err("INVALID_ARGUMENTS: state must be open, closed or all".into());
            }
            let kind = opt_str(a, "kind").unwrap_or_else(|| "all".into());
            posts(
                &format!("/repos/{repo}/issues"),
                &[("state", &state)],
                a,
                |x| {
                    let pull = x["pull_request"].is_object();
                    match kind.as_str() {
                        "issues" if pull => None,
                        "pulls" if !pull => None,
                        _ => issue_post(x),
                    }
                },
            )
            .await
        }
        "gh_releases" => {
            let repo = need_repo(a)?;
            posts(&format!("/repos/{repo}/releases"), &[], a, |x| {
                release_post(x, &repo)
            })
            .await
        }
        "gh_stargazers" => {
            let repo = need_repo(a)?;
            let (items, next) = list(
                &listing_url(
                    &format!("/repos/{repo}/stargazers"),
                    &[],
                    cursor(a).as_deref(),
                ),
                STAR_JSON,
            )
            .await?;
            Ok(page(
                items
                    .iter()
                    .filter_map(|x| {
                        let user = if x["user"].is_object() { &x["user"] } else { x };
                        profile(user, json!({ "starred_at": ts(&x["starred_at"]) }))
                    })
                    .collect(),
                next,
            ))
        }
        "gh_forks" => {
            let repo = need_repo(a)?;
            let sort = opt_str(a, "sort").unwrap_or_else(|| "newest".into());
            if !["newest", "oldest", "stargazers", "watchers"].contains(&sort.as_str()) {
                return Err(
                    "INVALID_ARGUMENTS: sort must be newest, oldest, stargazers or watchers".into(),
                );
            }
            posts(
                &format!("/repos/{repo}/forks"),
                &[("sort", &sort)],
                a,
                repo_post,
            )
            .await
        }
        "gh_followers" => profiles(&format!("/users/{}/followers", who(a).await?), a).await,
        "gh_following" => profiles(&format!("/users/{}/following", who(a).await?), a).await,
        "gh_notifications" => {
            if token().is_none() {
                return Err(token_required("Reading notifications"));
            }
            let all = if a["all"].as_bool().unwrap_or(false) {
                "true"
            } else {
                "false"
            };
            posts("/notifications", &[("all", all)], a, notification).await
        }
        "gh_trending" => {
            let since = opt_str(a, "since").unwrap_or_else(|| "daily".into());
            if !["daily", "weekly", "monthly"].contains(&since.as_str()) {
                return Err("INVALID_ARGUMENTS: since must be daily, weekly or monthly".into());
            }
            let language = opt_str(a, "language")
                .map(|l| format!("/{}", urlencoding::encode(&l.to_ascii_lowercase())))
                .unwrap_or_default();
            let r = send(
                reqwest::Method::GET,
                &format!("https://github.com/trending{language}?since={since}"),
                "text/html",
                None,
                None,
            )
            .await?;
            if let Some(e) = failure(&r, false, now()) {
                return Err(e);
            }
            let items = trending(&r.body);
            if items.is_empty() {
                return Err("UPSTREAM_CHANGED: the GitHub trending page had no repositories in the expected markup (it is scraped HTML and may have changed)".into());
            }
            let mut out = page(items, None);
            out["source"] = json!("scraped_html");
            Ok(out)
        }
        "gh_star" => {
            let repo = need_repo(a)?;
            let m = if undo(a) {
                reqwest::Method::DELETE
            } else {
                reqwest::Method::PUT
            };
            write(m, &format!("/user/starred/{repo}"), None).await?;
            Ok(json!({ "ok": true, "repo": repo, "starred": !undo(a) }))
        }
        "gh_follow" => {
            let n = login_of(&req_str(a, "username")?)
                .ok_or("INVALID_ARGUMENTS: username must be a GitHub login or profile link")?;
            let m = if undo(a) {
                reqwest::Method::DELETE
            } else {
                reqwest::Method::PUT
            };
            write(m, &format!("/user/following/{n}"), None).await?;
            Ok(json!({ "ok": true, "user": n, "following": !undo(a) }))
        }
        "gh_watch" => {
            let repo = need_repo(a)?;
            let path = format!("/repos/{repo}/subscription");
            if undo(a) {
                write(reqwest::Method::DELETE, &path, None).await?;
            } else {
                write(
                    reqwest::Method::PUT,
                    &path,
                    Some(json!({ "subscribed": true })),
                )
                .await?;
            }
            Ok(json!({ "ok": true, "repo": repo, "watching": !undo(a) }))
        }
        "gh_comment" => {
            let (repo, n) = issue_ref(&req_str(a, "post")?).ok_or(
                "INVALID_ARGUMENTS: post must be owner/name#number or an issue or pull request link",
            )?;
            let v = write(
                reqwest::Method::POST,
                &format!("/repos/{repo}/issues/{n}/comments"),
                Some(json!({ "body": req_str(a, "text")? })),
            )
            .await?;
            Ok(json!({ "ok": true, "comment_id": opt_s(&v["id"]), "url": opt_s(&v["html_url"]) }))
        }
        "gh_issue_create" => {
            let repo = need_repo(a)?;
            let mut body = json!({ "title": req_str(a, "title")? });
            if let Some(text) = opt_str(a, "body") {
                body["body"] = json!(text);
            }
            if let Some(labels) = a["labels"].as_array().filter(|l| !l.is_empty()) {
                body["labels"] = json!(labels);
            }
            let v = write(
                reqwest::Method::POST,
                &format!("/repos/{repo}/issues"),
                Some(body),
            )
            .await?;
            Ok(json!({ "ok": true, "post": issue_post(&v) }))
        }
        _ => Err(format!("UNKNOWN_TOOL: {tool}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reply(status: u16, body: &str) -> Reply {
        Reply {
            status,
            body: body.into(),
            link: None,
            remaining: None,
            reset: None,
            retry_after: None,
        }
    }

    #[test]
    fn link_header_gives_the_next_page() {
        let link = r#"<https://api.github.com/user/1/followers?per_page=30&page=1>; rel="prev", <https://api.github.com/user/1/followers?per_page=30&page=3>; rel="next", <https://api.github.com/user/1/followers?per_page=30&page=9>; rel="last""#;
        assert_eq!(next_page(Some(link)).as_deref(), Some("3"));
        assert_eq!(
            next_page(Some(r#"<https://api.github.com/x?page=1>; rel="first""#)),
            None
        );
        assert_eq!(next_page(None), None);
    }

    #[test]
    fn rate_limit_headers_become_rate_limited_with_the_reset_time() {
        let mut r = reply(403, r#"{"message":"API rate limit exceeded for 1.2.3.4."}"#);
        r.remaining = Some(0);
        r.reset = Some(1_800_000_600);
        let e = failure(&r, false, 1_800_000_000).unwrap();
        assert!(e.starts_with("RATE_LIMITED:"), "{e}");
        assert!(e.contains("2027-01-15T08:10:00Z"), "{e}");
        assert!(e.contains("in 10 min"), "{e}");
        assert!(e.contains("60 requests an hour"), "{e}");
        let with_token = failure(&r, true, 1_800_000_000).unwrap();
        assert!(!with_token.contains("60 requests"));
        let mut secondary = reply(429, "{}");
        secondary.retry_after = Some(61);
        assert!(failure(&secondary, true, 0).unwrap().contains("in 2 min"));
    }

    #[test]
    fn other_statuses_map_to_the_shared_codes() {
        assert!(failure(&reply(200, "[]"), false, 0).is_none());
        assert!(failure(&reply(204, ""), false, 0).is_none());
        assert!(failure(&reply(401, "{}"), true, 0)
            .unwrap()
            .starts_with("NOT_LOGGED_IN"));
        assert!(failure(&reply(404, "{}"), true, 0)
            .unwrap()
            .starts_with("NOT_FOUND"));
        let mut forbidden = reply(403, r#"{"message":"Resource not accessible"}"#);
        forbidden.remaining = Some(4000);
        assert!(failure(&forbidden, true, 0)
            .unwrap()
            .starts_with("REJECTED"));
        assert!(
            failure(&reply(422, r#"{"message":"Validation Failed"}"#), true, 0)
                .unwrap()
                .starts_with("INVALID_ARGUMENTS")
        );
        assert!(failure(&reply(502, "bad"), true, 0)
            .unwrap()
            .starts_with("UPSTREAM_502"));
    }

    #[test]
    fn builds_listing_and_search_urls() {
        assert_eq!(
            listing_url("/users/a/repos", &[("sort", "pushed")], None),
            "/users/a/repos?sort=pushed&per_page=30&page=1"
        );
        assert_eq!(listing_url("/x", &[], Some("7")), "/x?per_page=30&page=7");
        assert_eq!(
            listing_url("/x", &[], Some("7;drop")),
            "/x?per_page=30&page=1"
        );
        assert_eq!(
            search_url(
                "repositories",
                "tauri language:rust stars:>100",
                Some("stars"),
                None,
                Some("2")
            )
            .unwrap(),
            "/search/repositories?q=tauri%20language%3Arust%20stars%3A%3E100&sort=stars&order=desc&per_page=30&page=2"
        );
        assert_eq!(
            search_url("issues", "is:pr is:open repo:a/b", None, None, None).unwrap(),
            "/search/issues?q=is%3Apr%20is%3Aopen%20repo%3Aa%2Fb&per_page=30&page=1"
        );
        assert!(search_url("users", "x", Some("stars"), None, None)
            .unwrap_err()
            .starts_with("INVALID_ARGUMENTS"));
        assert!(search_url("gists", "x", None, None, None).is_err());
    }

    #[test]
    fn reads_logins_repos_and_issue_references() {
        assert_eq!(login_of("@octocat").as_deref(), Some("octocat"));
        assert_eq!(
            login_of("https://github.com/octocat?tab=repositories").as_deref(),
            Some("octocat")
        );
        assert_eq!(login_of("a b"), None);
        assert_eq!(
            repo_of("tauri-apps/tauri").as_deref(),
            Some("tauri-apps/tauri")
        );
        assert_eq!(
            repo_of("https://github.com/rust-lang/rust.git").as_deref(),
            Some("rust-lang/rust")
        );
        assert_eq!(
            repo_of("https://github.com/a/b.c/issues/3").as_deref(),
            Some("a/b.c")
        );
        assert_eq!(repo_of("solo"), None);
        assert_eq!(repo_of("a/../b"), None);
        assert_eq!(issue_ref("a/b#12"), Some(("a/b".into(), 12)));
        assert_eq!(
            issue_ref("https://github.com/a/b/pull/9#issuecomment-1"),
            Some(("a/b".into(), 9))
        );
        assert_eq!(issue_ref("a/b"), None);
    }

    #[test]
    fn parses_users_and_stargazers() {
        let p = profile(
            &json!({ "login": "octocat", "id": 583231, "avatar_url": "a.png", "html_url": "https://github.com/octocat", "type": "User", "name": "The Octocat", "company": "@github", "bio": null, "public_repos": 8, "followers": 100, "following": 9, "created_at": "2011-01-25T18:44:36Z" }),
            json!({ "starred_at": 5 }),
        )
        .unwrap();
        assert_eq!(p["handle"], "octocat");
        assert_eq!(p["id"], "583231");
        assert_eq!(p["followers"], 100);
        assert_eq!(p["posts"], 8);
        assert_eq!(p["bio"], "");
        assert_eq!(p["extra"]["kind"], "user");
        assert_eq!(p["extra"]["created_at"], 1295981076);
        assert_eq!(p["extra"]["starred_at"], 5);
        let slim = profile(&json!({ "login": "a", "id": 1 }), json!({})).unwrap();
        assert_eq!(slim["url"], "https://github.com/a");
        assert!(slim.get("followers").is_none());
        assert!(profile(&json!({ "id": 1 }), json!({})).is_none());
    }

    #[test]
    fn parses_repositories() {
        let r = repo_post(&json!({ "id": 1, "full_name": "a/b", "html_url": "https://github.com/a/b", "description": "demo", "owner": { "login": "a", "id": 2, "avatar_url": "o.png" }, "stargazers_count": 50, "forks_count": 4, "open_issues_count": 3, "subscribers_count": 7, "language": "Rust", "topics": ["cli"], "license": { "spdx_id": "MIT" }, "fork": false, "archived": true, "pushed_at": "2026-01-02T00:00:00Z" })).unwrap();
        assert_eq!(r["id"], "a/b");
        assert_eq!(r["stats"]["likes"], 50);
        assert_eq!(r["stats"]["reposts"], 4);
        assert_eq!(r["author"]["handle"], "a");
        assert_eq!(r["extra"]["kind"], "repo");
        assert_eq!(r["extra"]["language"], "Rust");
        assert_eq!(r["extra"]["license"], "MIT");
        assert_eq!(r["extra"]["archived"], true);
        assert!(r["extra"].get("fork").is_none());
        assert!(repo_post(&json!({})).is_none());
    }

    #[test]
    fn parses_issues_pulls_comments_and_releases() {
        let i = issue_post(&json!({ "number": 12, "title": "Crash", "body": "it breaks", "state": "open", "html_url": "https://github.com/a/b/issues/12", "repository_url": "https://api.github.com/repos/a/b", "user": { "login": "ana", "id": 3 }, "comments": 2, "reactions": { "total_count": 5 }, "labels": [{ "name": "bug" }], "created_at": "2026-01-02T00:00:00Z" })).unwrap();
        assert_eq!(i["id"], "a/b#12");
        assert_eq!(i["extra"]["kind"], "issue");
        assert_eq!(i["extra"]["title"], "Crash");
        assert_eq!(i["extra"]["labels"][0], "bug");
        assert_eq!(i["stats"]["replies"], 2);
        let pr = issue_post(&json!({ "number": 13, "html_url": "https://github.com/a/b/pull/13", "pull_request": {}, "user": {} })).unwrap();
        assert_eq!(pr["extra"]["kind"], "pull");
        assert_eq!(pr["id"], "a/b#13");
        let c = comment(
            &json!({ "id": 99, "body": "same here", "user": { "login": "bia" }, "created_at": "2026-01-02T00:00:00Z", "reactions": { "total_count": 1 } }),
        );
        assert_eq!(c.id, "99");
        assert_eq!(c.author.handle, "bia");
        assert_eq!(c.likes, Some(1));
        let rel = release_post(&json!({ "tag_name": "v1.0", "name": "", "html_url": "u", "author": { "login": "a" }, "body": "notes", "published_at": "2026-01-02T00:00:00Z", "assets": [{ "name": "x.dmg", "size": 10, "download_count": 7, "browser_download_url": "d" }, { "name": "y", "download_count": 3 }] }), "a/b").unwrap();
        assert_eq!(rel["id"], "a/b@v1.0");
        assert_eq!(rel["extra"]["title"], "v1.0");
        assert_eq!(rel["extra"]["downloads"], 10);
        assert!(release_post(&json!({}), "a/b").is_none());
    }

    #[test]
    fn parses_events_as_posts() {
        let actor =
            json!({ "id": 1, "login": "ana", "display_login": "ana", "avatar_url": "a.png" });
        let push = event_post(&json!({ "id": "1", "type": "PushEvent", "actor": actor, "repo": { "name": "a/b" }, "created_at": "2026-01-02T00:00:00Z", "payload": { "ref": "refs/heads/main", "head": "abc", "commits": [{ "message": "fix: one\n\nbody" }, { "message": "feat: two" }] } })).unwrap();
        assert_eq!(push["extra"]["title"], "Pushed to main in a/b");
        assert_eq!(push["text"], "fix: one\nfeat: two");
        assert_eq!(push["url"], "https://github.com/a/b/commit/abc");
        assert_eq!(push["author"]["handle"], "ana");
        let slim = event_post(&json!({ "id": "2", "type": "PushEvent", "repo": { "name": "a/b" }, "payload": { "ref": "refs/heads/dev" } })).unwrap();
        assert_eq!(slim["text"], "Pushed to dev in a/b");
        assert_eq!(slim["url"], "https://github.com/a/b/commits/dev");
        let star = event_post(&json!({ "id": "3", "type": "WatchEvent", "repo": { "name": "a/b" }, "payload": { "action": "started" } })).unwrap();
        assert_eq!(star["text"], "Starred a/b");
        let pr = event_post(&json!({ "id": "4", "type": "PullRequestEvent", "repo": { "name": "a/b" }, "payload": { "action": "opened", "number": 7, "pull_request": { "number": 7 } } })).unwrap();
        assert_eq!(pr["extra"]["title"], "Opened pull request #7 in a/b");
        assert_eq!(pr["url"], "https://github.com/a/b/pull/7");
        let issue = event_post(&json!({ "id": "5", "type": "IssuesEvent", "repo": { "name": "a/b" }, "payload": { "action": "closed", "issue": { "number": 3, "title": "Bug", "html_url": "h" } } })).unwrap();
        assert_eq!(issue["extra"]["title"], "Closed issue #3 in a/b: Bug");
        let rel = event_post(&json!({ "id": "6", "type": "ReleaseEvent", "repo": { "name": "a/b" }, "payload": { "release": { "tag_name": "v2", "html_url": "r" } } })).unwrap();
        assert_eq!(rel["extra"]["title"], "Released v2 in a/b");
        let other = event_post(
            &json!({ "id": "7", "type": "GollumEvent", "repo": { "name": "a/b" }, "payload": {} }),
        )
        .unwrap();
        assert_eq!(other["text"], "Gollum in a/b");
        assert!(event_post(&json!({ "type": "PushEvent" })).is_none());
    }

    #[test]
    fn parses_notifications() {
        let n = notification(&json!({ "id": "9", "unread": true, "reason": "mention", "updated_at": "2026-01-02T00:00:00Z", "subject": { "title": "Crash", "type": "PullRequest", "url": "https://api.github.com/repos/a/b/pulls/4" }, "repository": { "full_name": "a/b", "owner": { "login": "a" } } })).unwrap();
        assert_eq!(n["url"], "https://github.com/a/b/pull/4");
        assert_eq!(n["extra"]["reason"], "mention");
        assert_eq!(n["extra"]["unread"], true);
    }

    #[test]
    fn parses_the_trending_page() {
        let html = r#"<main><article class="Box-row">
          <h2 class="h3 lh-condensed"><a href="/rust-lang/rust" data-view-component="true" class="Link"> <span class="text-normal">rust-lang /</span> rust</a></h2>
          <p class="col-9 color-fg-muted my-1 pr-4">Empowering everyone &amp; more</p>
          <div class="f6 color-fg-muted mt-2">
            <span itemprop="programmingLanguage">Rust</span>
            <a href="/rust-lang/rust/stargazers" class="Link"><svg aria-label="star"><path d="M8"></path></svg> 101,234</a>
            <a href="/rust-lang/rust/forks" class="Link"><svg><path></path></svg> 13,001</a>
            <span class="d-inline-block float-sm-right"><svg></svg> 321 stars today</span>
          </div></article>
          <article class="Box-row"><h2 class="h3"><a href="/a/b">a / b</a></h2></article></main>"#;
        let items = trending(html);
        assert_eq!(items.len(), 2);
        assert_eq!(items[0]["id"], "rust-lang/rust");
        assert_eq!(items[0]["text"], "Empowering everyone & more");
        assert_eq!(items[0]["stats"]["likes"], 101234);
        assert_eq!(items[0]["stats"]["reposts"], 13001);
        assert_eq!(items[0]["extra"]["language"], "Rust");
        assert_eq!(items[0]["extra"]["stars_gained"], 321);
        assert_eq!(items[0]["extra"]["period"], "today");
        assert_eq!(items[1]["id"], "a/b");
        assert!(items[1]["stats"].get("likes").is_none());
        assert!(trending("<html>nothing</html>").is_empty());
    }

    #[test]
    fn token_required_is_a_json_error() {
        let e = token_required("Code search");
        let (code, rest) = e.split_once(": ").unwrap();
        assert_eq!(code, "TOKEN_REQUIRED");
        let v: Value = serde_json::from_str(rest).unwrap();
        assert_eq!(v["retryable"], false);
        assert!(v["next_step"].as_str().unwrap().contains("GitHub screen"));
    }
}

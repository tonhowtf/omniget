use serde::de::DeserializeOwned;
use serde_json::{json, Value};

use omniget_core::core::devtools::{crawl, http, pack, ports, scrape, util, DevError};
use omniget_core::core::proxy_pool;

use crate::commands::proxy_pool as pool;

use super::blocking;

pub const TOOLS: &[&str] = &[
    "dev_pack",
    "dev_scrape",
    "dev_http",
    "dev_ports",
    "dev_util",
    "dev_proxy",
    "dev_crawl",
];

pub const PROXY_READ_OPS: &[&str] = &["status", "detect", "check"];
pub const PROXY_OPS: &[&str] = &[
    "status", "detect", "check", "add", "import", "remove", "rotate", "set",
];

const DEFAULT_PORTS: usize = 100;

fn parsed<T: DeserializeOwned>(a: &Value) -> Result<T, String> {
    serde_json::from_value(a.clone()).map_err(|e| DevError::InvalidInput(e.to_string()).wire())
}

fn shown<T: serde::Serialize>(value: T) -> Result<Value, String> {
    serde_json::to_value(value).map_err(|e| e.to_string())
}

pub fn url_of<'a>(name: &str, a: &'a Value) -> Option<&'a str> {
    matches!(name, "dev_http" | "dev_scrape" | "dev_proxy" | "dev_crawl")
        .then(|| a["url"].as_str())
        .flatten()
}

pub async fn call(name: &str, a: &Value, can_write: bool) -> Result<Value, String> {
    let op = a["op"].as_str().unwrap_or_default();
    match (name, op) {
        ("dev_pack", "plan") => {
            let request: pack::PackRequest = parsed(a)?;
            shown(blocking(move || pack::plan(&request)).await?)
        }
        ("dev_pack", "build") => {
            let request: pack::PackRequest = parsed(a)?;
            shown(blocking(move || pack::build(&request)).await?)
        }
        ("dev_scrape", "fetch") => {
            let request: scrape::FetchRequest = parsed(a)?;
            if request.urls.is_empty() {
                shown(scrape::fetch(request).await.map_err(|e| e.wire())?)
            } else {
                shown(scrape::fetch_many(request).await.map_err(|e| e.wire())?)
            }
        }
        ("dev_scrape", "select") => {
            let request: scrape::SelectRequest = parsed(a)?;
            if request.selector.trim().is_empty() && request.text.is_some() {
                shown(scrape::find_from(request).await.map_err(|e| e.wire())?)
            } else {
                shown(scrape::select_from(request).await.map_err(|e| e.wire())?)
            }
        }
        ("dev_scrape", "similar") => shown(
            scrape::similar_from(parsed(a)?)
                .await
                .map_err(|e| e.wire())?,
        ),
        ("dev_crawl", "run") => shown(crawl::crawl(parsed(a)?).await.map_err(|e| e.wire())?),
        ("dev_crawl", "status") => {
            let limit = a["limit"].as_u64().map(|n| n as usize);
            match a["output"].as_str().map(str::to_string) {
                Some(output) => shown(blocking(move || crawl::status(&output, limit)).await?),
                None => Err(DevError::InvalidInput(
                    "status takes output, the folder a crawl wrote to".into(),
                )
                .wire()),
            }
        }
        ("dev_proxy", op) => proxy(op, a, can_write).await,
        ("dev_http", _) => shown(http::send(parsed(a)?).await.map_err(|e| e.wire())?),
        ("dev_ports", "list") => {
            let limit = a["limit"].as_u64().map_or(DEFAULT_PORTS, |n| n as usize);
            let found = blocking(ports::list).await?;
            let total = found.len();
            let mut out = json!({
                "listeners": found.into_iter().take(limit).collect::<Vec<_>>(),
                "total": total,
                "truncated": total > limit,
            });
            if total == 0 {
                out["reason"] = json!("nothing_listening");
            }
            Ok(out)
        }
        ("dev_ports", "kill") => {
            if !can_write {
                return Err(
                    "TOOL_NOT_AUTHORIZED: dev_ports op=kill needs the devtools_write scope".into(),
                );
            }
            let port = a["port"].as_u64().and_then(|n| u16::try_from(n).ok());
            let pid = a["pid"].as_u64().and_then(|n| u32::try_from(n).ok());
            let confirm = a["confirm"].as_bool() == Some(true);
            shown(blocking(move || ports::kill(port, pid, confirm)).await?)
        }
        ("dev_util", _) => {
            let (op, args) = (op.to_string(), a.clone());
            blocking(move || util::run(&op, &args)).await
        }
        _ => Err(DevError::InvalidInput(format!("unknown op \"{op}\" for {name}")).wire()),
    }
}

fn strings(value: &Value) -> Vec<String> {
    value
        .as_array()
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

async fn proxy(op: &str, a: &Value, can_write: bool) -> Result<Value, String> {
    if !PROXY_OPS.contains(&op) {
        return Err(DevError::InvalidInput(format!("unknown op \"{op}\" for dev_proxy")).wire());
    }
    if !PROXY_READ_OPS.contains(&op) && !can_write {
        return Err(format!(
            "TOOL_NOT_AUTHORIZED: dev_proxy op={op} needs the devtools_write scope"
        ));
    }
    let limit = a["limit"]
        .as_u64()
        .map_or(pool::DEFAULT_ROWS, |n| n as usize);
    match op {
        "status" => Ok(pool::status(limit)),
        "detect" => Ok(pool::detect().await),
        "check" => {
            let url = a["url"].as_str().map(str::to_string);
            pool::check(strings(&a["ids"]), url, limit).await
        }
        "add" => {
            let proxies = strings(&a["proxies"]);
            if proxies.is_empty() {
                return Err(DevError::InvalidInput(
                    "proxies takes one or more addresses such as socks5://127.0.0.1:7890".into(),
                )
                .wire());
            }
            if proxies.iter().any(|raw| proxy_pool::holds_secret(raw)) {
                return Err(DevError::InvalidInput(
                    "a proxy with a password is not accepted as an argument and nothing was added; put the list in a file and call op=import with its path, or ask the person to add it in Settings, Network".into(),
                )
                .wire());
            }
            pool::add(proxies, limit).await
        }
        "import" => match a["file"].as_str().filter(|f| !f.trim().is_empty()) {
            Some(file) => pool::import(file.to_string(), limit).await,
            None => Err(DevError::InvalidInput(
                "file takes the path of a text file with one proxy per line".into(),
            )
            .wire()),
        },
        "remove" => {
            let ids = strings(&a["ids"]);
            let failing = a["failing"].as_bool() == Some(true);
            if ids.is_empty() && !failing {
                return Err(DevError::InvalidInput(
                    "pass ids from op=status, or failing=true to drop every proxy whose last check failed".into(),
                )
                .wire());
            }
            pool::remove(ids, failing, limit).await
        }
        "rotate" => Ok(pool::rotate(limit)),
        _ => {
            let rotation = match a.get("rotation").filter(|v| !v.is_null()) {
                Some(value) => Some(parsed(value)?),
                None => None,
            };
            let enabled = a["enabled"].as_bool();
            if enabled.is_none() && rotation.is_none() {
                return Err(
                    DevError::InvalidInput("set takes enabled, rotation or both".into()).wire(),
                );
            }
            pool::set(enabled, rotation, limit).await
        }
    }
}

fn text(description: &str) -> Value {
    json!({"type": "string", "description": description})
}

fn count(description: &str, minimum: u64, maximum: u64) -> Value {
    json!({"type": "integer", "minimum": minimum, "maximum": maximum, "description": description})
}

fn globs(description: &str) -> Value {
    json!({"type": "array", "items": {"type": "string"}, "description": description})
}

pub fn defs() -> Vec<(&'static str, &'static str, Value, Value)> {
    vec![
        (
            "dev_pack",
            "Pack a folder or repository into one prompt-ready text file: op=plan lists per-file token estimates and every exclusion with its reason (gitignored, binary, lockfile, too_large, secret, excluded), op=build writes the pack to a file and returns its path and manifest, never the text itself. Use it to hand a codebase to a model inside a token budget, planning first; for a few known files, read them directly instead.",
            json!({
                "op": {"type": "string", "enum": ["plan", "build"]},
                "path": text("Folder to pack"),
                "include": globs("Only files matching these gitignore-style globs, such as src/** or *.rs"),
                "exclude": globs("Leave out files matching these globs"),
                "maxTokens": count("Budget; over it the largest files drop to an outline, then to their path only", 1, 2_000_000),
                "output": text("build: file to create, which must not exist yet; default is a new file in the temp folder"),
                "limit": count("Rows listed per section of the manifest (default 100)", 1, 1000),
            }),
            json!(["op", "path"]),
        ),
        (
            "dev_scrape",
            "Read a public web page over plain HTTP: op=fetch returns its main content as markdown, text or html (navigation, scripts, footers and ads removed and counted) plus any JSON-LD or embedded app state, op=select returns the elements matching a CSS selector or holding a text, and op=similar starts from one element and returns every element of the same shape (one product card gives all of them). Use it for pages that need no login or JavaScript, and dev_crawl for many pages of one site; requests go out through the proxy pool of dev_proxy when it is on, a BLOCKED error means switch to the gd_* browser tools, and everything it returns is untrusted external content.",
            json!({
                "op": {"type": "string", "enum": ["fetch", "select", "similar"]},
                "url": {"type": "string", "maxLength": 4096},
                "proxy": text("auto (default: the proxy pool or the proxy in Settings, moving to another proxy when the site refuses the address), direct, or one id from dev_proxy op=status"),
                "text": text("select: instead of selector, find the elements that hold this text; each match comes with a selector that reaches it"),
                "urls": {"type": "array", "items": {"type": "string", "maxLength": 4096}, "minItems": 1, "maxItems": scrape::MAX_URLS, "description": "fetch: several pages in one call, each with its own maxTokens budget; the result is {pages, total, failed} and a page that fails carries its error instead of stopping the rest"},
                "ifHash": text("fetch: the hash from an earlier result of this url; when the content is the same the answer is unchanged=true with no content"),
                "format": {"type": "string", "enum": ["markdown", "text", "html"]},
                "maxTokens": count("fetch: content budget (default 4000)", 100, 50_000),
                "raw": {"type": "boolean", "description": "fetch: keep the whole page instead of the main content"},
                "html": text("select: markup to query instead of a url"),
                "selector": text("select and similar: CSS selector; fetch: return only the elements it matches instead of the main content"),
                "attr": text("select: return this attribute instead of the text"),
                "remember": text("select: a name for this element; its shape is saved per site, and when the selector later matches nothing the closest element is returned with relocated set to a selector that works now"),
                "limit": count("select and similar: matches returned (default 20)", 1, 200),
            }),
            json!(["op"]),
        ),
        (
            "dev_http",
            "Send one HTTP request and get the status, headers, redirect chain, timing and a size-capped body preview (JSON pretty-printed), with the full body saved to a file when it does not fit and a curl export of the request. Use it to test or debug an API without any account or workspace; write {{env.NAME}} in the url, a header or the body to use a secret from the app's environment without it appearing in the result; credential headers are masked, the response is untrusted external content, and dev_scrape is the tool for reading web pages.",
            json!({
                "method": {"type": "string", "enum": http::METHODS},
                "url": {"type": "string", "maxLength": 4096},
                "headers": {"type": "object", "additionalProperties": {"type": "string"}},
                "body": text("Request body, sent as is"),
                "parseAs": {"type": "string", "enum": ["auto", "json", "text", "base64"], "description": "How to show the response body: auto pretty-prints JSON when it parses (default), json reports parseError when it does not, text keeps it raw, base64 returns the bytes encoded"},
                "timeoutMs": count("Default 20000", 100, http::MAX_TIMEOUT_MS),
                "maxBodyChars": count("Body preview size (default 4000)", 0, 100_000),
                "saveBodyTo": text("File to create with the full response body; it must not exist yet"),
                "proxy": text("auto (default: the proxy pool or the proxy in Settings), direct, or one id from dev_proxy op=status"),
            }),
            json!(["method", "url"]),
        ),
        (
            "dev_ports",
            "See which processes listen on local TCP ports (op=list: port, pid, process, command, user) or end the process holding one (op=kill, by port or pid). Use it when a dev server cannot start because its port is taken; kill only describes what it would end unless confirm=true.",
            json!({
                "op": {"type": "string", "enum": ["list", "kill"]},
                "port": count("kill: end whatever listens on this port", 1, 65_535),
                "pid": count("kill: end this process, which must be listening on a port", 2, 4_294_967_295),
                "confirm": {"type": "boolean", "description": "kill: true to really end it; otherwise a dry run"},
                "limit": count("list: rows returned (default 100)", 1, 1000),
            }),
            json!(["op"]),
        ),
        (
            "dev_util",
            "Offline developer utilities chosen by op: json_format, json_minify, json_query, jwt_decode (decodes without verifying the signature), base64_encode, base64_decode, url_encode, url_decode, hash, uuid, timestamp, regex_test and text_diff. Use it for a one-off conversion or check instead of writing a script; nothing leaves this computer.",
            json!({
                "op": {"type": "string", "enum": util::OPS},
                "text": text("Input of every op except uuid and timestamp"),
                "other": text("text_diff: the second text"),
                "query": text("json_query: path such as items[0].name or [\"odd key\"]"),
                "pattern": text("regex_test: the regular expression"),
                "algorithm": {"type": "string", "enum": ["md5", "sha1", "sha256", "sha512"]},
                "file": text("hash: hash this file instead of text"),
                "value": {"description": "timestamp: unix seconds or milliseconds, or an ISO 8601 time; omit for now"},
                "limit": count("regex_test: matches returned (default 50)", 1, 1000),
                "maxChars": count("Output size (default 8000)", 1, 100_000),
            }),
            json!(["op"]),
        ),
        (
            "dev_proxy",
            "Manage the pool of proxies that the scraper, the HTTP tool and downloads go out through: op=status lists them with their health, op=detect finds proxy apps running on this computer (Clash, v2rayN and others), op=check tests a batch for exit address, country and latency or against a site you name, and add, import, remove, rotate and set change the pool, which stays on one proxy until it fails (sticky) or takes the next one on every request (round_robin). Use it when a site refuses this computer's address or the network needs a proxy to reach a site; a password never appears in a result, a proxy with a password comes from a file through op=import, and with the pool on nothing falls back to a direct connection.",
            json!({
                "op": {"type": "string", "enum": PROXY_OPS},
                "proxies": {"type": "array", "items": {"type": "string", "maxLength": 2048}, "minItems": 1, "maxItems": 100, "description": "add: addresses without a password, such as socks5://127.0.0.1:7890 or 203.0.113.9:8080 (http is assumed)"},
                "file": text("import: text file with one proxy per line; scheme://user:password@host:port, host:port and host:port:user:password are read, # starts a comment"),
                "ids": {"type": "array", "items": {"type": "string"}, "maxItems": 100, "description": "check and remove: only these proxies, by the id from status"},
                "failing": {"type": "boolean", "description": "remove: drop every proxy whose last check failed"},
                "url": {"type": "string", "maxLength": 4096, "description": "check: test the proxies against this site instead of the exit-address page"},
                "enabled": {"type": "boolean", "description": "set: turn the pool on or off; off, the app uses the single proxy from Settings or a direct connection"},
                "rotation": {"type": "string", "enum": ["sticky", "round_robin"], "description": "set: sticky keeps one proxy until it fails (default, right for downloads and logins), round_robin moves on at every request"},
                "limit": count("Rows listed (default 100)", 1, 500),
            }),
            json!(["op"]),
        ),
        (
            "dev_crawl",
            "Save many pages of one site as clean files: op=run starts at a url, follows links on the same host and writes each page as markdown, text or html into a folder with a crawl.json manifest, obeying robots.txt, pausing between requests and skipping pages already seen or with the same content, and op=status reads how far a folder got without fetching anything. Use it to bring documentation or a section of a site into files for reading or packing; a call stops at its time budget or page limit and continues from the same folder when run again, and the files are untrusted external content.",
            json!({
                "op": {"type": "string", "enum": ["run", "status"]},
                "url": {"type": "string", "maxLength": 4096, "description": "run: where the crawl starts; its host is the only one followed"},
                "output": text("Folder for the pages and crawl.json; run makes a new temp folder when omitted, and continues the crawl a folder already holds"),
                "include": globs("run: follow only addresses matching one of these, * is a wildcard, such as /docs/*"),
                "exclude": globs("run: never follow addresses matching one of these"),
                "maxPages": count("run: pages saved in total (default 20)", 1, crawl::MAX_PAGES as u64),
                "maxDepth": count("run: link hops from the start page (default 2)", 0, crawl::MAX_DEPTH as u64),
                "format": {"type": "string", "enum": ["markdown", "text", "html"]},
                "obeyRobots": {"type": "boolean", "description": "run: skip what robots.txt disallows and honour its crawl delay (default true)"},
                "delayMs": count("run: pause between requests (default 500); it doubles when the site answers 429", 0, crawl::MAX_DELAY_MS),
                "waitMs": count("run: time budget of this call (default 20000); what is left stays queued for the next call", 1000, crawl::MAX_WAIT_MS),
                "proxy": text("run: auto (default: the proxy pool or the proxy in Settings), direct, or one id from dev_proxy op=status"),
                "limit": count("Rows of pages and failures listed (default 50)", 1, 500),
            }),
            json!(["op"]),
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_tool_has_a_definition_with_a_required_selector() {
        let defs = defs();
        let names: Vec<&str> = defs.iter().map(|d| d.0).collect();
        assert_eq!(names, TOOLS);
        for (name, description, props, required) in &defs {
            assert!(
                description.matches(". ").count() <= 1,
                "{name}: two sentences"
            );
            let selector = if *name == "dev_http" { "method" } else { "op" };
            assert!(
                required.as_array().unwrap().contains(&json!(selector)),
                "{name}"
            );
            assert!(props[selector]["enum"].is_array(), "{name}");
            assert!(props[selector].get("default").is_none(), "{name}");
        }
    }

    #[tokio::test]
    async fn util_runs_end_to_end_and_errors_carry_the_recovery_fields() {
        let out = call(
            "dev_util",
            &json!({"op": "base64_encode", "text": "hi"}),
            false,
        )
        .await
        .unwrap();
        assert_eq!(out["text"], "aGk=");
        let error = call(
            "dev_util",
            &json!({"op": "json_format", "text": "{"}),
            false,
        )
        .await
        .unwrap_err();
        let (code, detail) = error.split_once(": ").unwrap();
        assert_eq!(code, "INVALID_INPUT");
        let detail: Value = serde_json::from_str(detail).unwrap();
        assert_eq!(detail["retryable"], false);
        assert!(detail["next_step"].is_string());
    }

    #[tokio::test]
    async fn pack_plans_then_builds_without_returning_the_text() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.rs"), "pub fn a() {}\n").unwrap();
        let path = dir.path().to_string_lossy().into_owned();
        let plan = call("dev_pack", &json!({"op": "plan", "path": path}), false)
            .await
            .unwrap();
        assert_eq!(plan["totals"]["files"], 1);
        let output = dir
            .path()
            .join("out/pack.txt")
            .to_string_lossy()
            .into_owned();
        let built = call(
            "dev_pack",
            &json!({"op": "build", "path": path, "output": output}),
            false,
        )
        .await
        .unwrap();
        assert_eq!(built["path"], output);
        assert!(!built.to_string().contains("pub fn a()"));
        assert!(std::fs::read_to_string(output)
            .unwrap()
            .contains("pub fn a()"));
    }

    #[tokio::test]
    async fn select_without_a_match_is_an_error_with_candidates() {
        let html = "<div class=\"product-price\">$1</div>";
        let error = call(
            "dev_scrape",
            &json!({"op": "select", "html": html, "selector": ".price"}),
            false,
        )
        .await
        .unwrap_err();
        let (code, detail) = error.split_once(": ").unwrap();
        assert_eq!(code, "NO_MATCH");
        let detail: Value = serde_json::from_str(detail).unwrap();
        assert_eq!(detail["candidates"][0], ".product-price");
    }

    #[tokio::test]
    async fn kill_is_refused_without_the_write_scope_and_dry_by_default() {
        let error = call("dev_ports", &json!({"op": "kill", "port": 1}), false)
            .await
            .unwrap_err();
        assert!(error.starts_with("TOOL_NOT_AUTHORIZED"));
        let error = call("dev_ports", &json!({"op": "kill"}), true)
            .await
            .unwrap_err();
        assert!(error.starts_with("INVALID_INPUT"), "{error}");
    }

    #[tokio::test]
    async fn an_unknown_op_is_an_error_and_never_a_default() {
        let error = call("dev_pack", &json!({"path": "/tmp"}), false)
            .await
            .unwrap_err();
        assert!(error.starts_with("INVALID_INPUT"));
    }

    #[tokio::test]
    #[ignore = "network and live system: run by hand with --ignored"]
    async fn the_live_tools_answer_end_to_end() {
        let page = call(
            "dev_scrape",
            &json!({"op": "fetch", "url": "https://example.com"}),
            false,
        )
        .await
        .unwrap();
        assert_eq!(page["title"], "Example Domain");
        assert_eq!(page["untrusted"], true);
        let response = call(
            "dev_http",
            &json!({"method": "GET", "url": "https://example.com", "headers": {"authorization": "Bearer x"}}),
            false,
        )
        .await
        .unwrap();
        assert_eq!(response["status"], 200);
        assert_eq!(response["request"]["headers"]["authorization"], "***");
        let ports = call("dev_ports", &json!({"op": "list", "limit": 1}), false)
            .await
            .unwrap();
        assert!(ports["listeners"].as_array().unwrap().len() <= 1);
        assert!(ports["total"].is_u64());
    }

    #[tokio::test]
    async fn changing_the_pool_needs_the_write_scope_and_never_takes_a_password() {
        for op in ["add", "import", "remove", "rotate", "set"] {
            let error = call("dev_proxy", &json!({"op": op}), false)
                .await
                .unwrap_err();
            assert!(error.starts_with("TOOL_NOT_AUTHORIZED"), "{op}: {error}");
        }
        let error = call(
            "dev_proxy",
            &json!({"op": "add", "proxies": ["127.0.0.1:7890", "socks5://bob:hunter2@203.0.113.9:1080"]}),
            true,
        )
        .await
        .unwrap_err();
        assert!(error.starts_with("INVALID_INPUT"), "{error}");
        assert!(error.contains("op=import"), "{error}");
        assert!(!error.contains("hunter2"), "{error}");
        for bad in [
            json!({"op": "add"}),
            json!({"op": "import"}),
            json!({"op": "remove"}),
            json!({"op": "set"}),
            json!({"op": "set", "rotation": "random"}),
            json!({"op": "nope"}),
        ] {
            let error = call("dev_proxy", &bad, true).await.unwrap_err();
            assert!(error.starts_with("INVALID_INPUT"), "{bad}: {error}");
        }
    }

    #[tokio::test]
    async fn similar_and_text_search_run_end_to_end_on_markup() {
        let error = call("dev_crawl", &json!({"op": "status"}), false)
            .await
            .unwrap_err();
        assert!(error.starts_with("INVALID_INPUT"), "{error}");
        let html = "<ul><li class=\"item a\">One</li><li class=\"item\">Two</li></ul>";
        let alike = call(
            "dev_scrape",
            &json!({"op": "similar", "html": html, "selector": "li.a"}),
            false,
        )
        .await
        .unwrap();
        assert_eq!(alike["matches"], json!(["One", "Two"]));
        assert_eq!(alike["selectorMatches"], 2);
        assert_eq!(alike["untrusted"], true);
        let found = call(
            "dev_scrape",
            &json!({"op": "select", "html": html, "text": "two"}),
            false,
        )
        .await
        .unwrap();
        assert_eq!(found["found"][0]["text"], "Two");
        assert!(found["found"][0]["selector"]
            .as_str()
            .unwrap()
            .contains("li:nth-of-type(2)"));
    }

    #[test]
    fn only_network_tools_expose_a_url_to_the_network_check() {
        let a = json!({"url": "https://example.com"});
        assert_eq!(url_of("dev_http", &a), Some("https://example.com"));
        assert_eq!(url_of("dev_scrape", &a), Some("https://example.com"));
        assert_eq!(url_of("dev_util", &a), None);
        assert_eq!(url_of("dev_scrape", &json!({"html": "<p>"})), None);
    }
}

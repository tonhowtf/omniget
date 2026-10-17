use std::collections::BTreeMap;

use base64::Engine;
use serde::Serialize;

#[derive(Debug, Clone, PartialEq, Default)]
pub struct CurlRequest {
    pub url: String,
    pub headers: BTreeMap<String, String>,
    pub referer: Option<String>,
    pub user_agent: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct CurlPreview {
    pub url: String,
    pub host: String,
    pub header_names: Vec<String>,
    pub has_cookie: bool,
    pub has_referer: bool,
    pub has_user_agent: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct CurlQueued {
    pub url: String,
    pub already_queued: bool,
}

const DROPPED_HEADERS: &[&str] = &[
    "host",
    "content-length",
    "accept-encoding",
    "range",
    "connection",
    "if-range",
    "if-none-match",
    "if-modified-since",
    "upgrade-insecure-requests",
];

const IGNORED_WITH_VALUE: &[&str] = &[
    "-o",
    "--output",
    "-m",
    "--max-time",
    "--connect-timeout",
    "--retry",
    "--retry-delay",
    "--retry-max-time",
    "-w",
    "--write-out",
    "--limit-rate",
    "-r",
    "--range",
    "-c",
    "--cookie-jar",
    "-x",
    "--proxy",
    "--resolve",
    "--connect-to",
    "--cacert",
    "--capath",
    "-E",
    "--cert",
    "--key",
    "--interface",
    "-Y",
    "--speed-limit",
    "-y",
    "--speed-time",
    "--max-redirs",
    "--output-dir",
    "--trace",
    "--trace-ascii",
    "-D",
    "--dump-header",
];

const SENDS_BODY: &[&str] = &[
    "-d",
    "--data",
    "--data-raw",
    "--data-binary",
    "--data-ascii",
    "--data-urlencode",
    "-F",
    "--form",
    "--form-string",
    "--json",
    "-T",
    "--upload-file",
];

pub fn split_command_line(input: &str) -> Result<Vec<String>, String> {
    let cmd_style = input.contains("^\"") || input.trim_end().ends_with('^');
    let text: String = if cmd_style {
        input
            .replace("^\r\n", " ")
            .replace("^\n", " ")
            .replace("^\"", "\"")
            .replace("^^", "^")
            .replace("^%", "%")
            .replace("^&", "&")
            .replace("^<", "<")
            .replace("^>", ">")
            .replace("^|", "|")
            .replace("^\\", "\\")
    } else {
        input.to_string()
    };
    let chars: Vec<char> = text.chars().collect();
    let mut words = Vec::new();
    let mut cur = String::new();
    let mut in_word = false;
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        match c {
            '\\' if !cmd_style || chars.get(i + 1).is_some_and(|n| *n == '\n' || *n == '\r') => {
                match chars.get(i + 1) {
                    Some('\n') => i += 1,
                    Some('\r') if chars.get(i + 2) == Some(&'\n') => i += 2,
                    Some(n) => {
                        cur.push(*n);
                        in_word = true;
                        i += 1;
                    }
                    None => {}
                }
            }
            '\'' => {
                in_word = true;
                i += 1;
                while i < chars.len() && chars[i] != '\'' {
                    cur.push(chars[i]);
                    i += 1;
                }
                if i >= chars.len() {
                    return Err("CURL_UNCLOSED_QUOTE".into());
                }
            }
            '$' if chars.get(i + 1) == Some(&'\'') => {
                in_word = true;
                i += 2;
                while i < chars.len() && chars[i] != '\'' {
                    if chars[i] == '\\' && i + 1 < chars.len() {
                        i += 1;
                        match chars[i] {
                            'n' => cur.push('\n'),
                            't' => cur.push('\t'),
                            'r' => cur.push('\r'),
                            'u' | 'x' => {
                                let max = if chars[i] == 'u' { 4 } else { 2 };
                                let hex: String = chars[i + 1..]
                                    .iter()
                                    .take(max)
                                    .take_while(|h| h.is_ascii_hexdigit())
                                    .collect();
                                match u32::from_str_radix(&hex, 16).ok().and_then(char::from_u32) {
                                    Some(ch) if !hex.is_empty() => {
                                        cur.push(ch);
                                        i += hex.len();
                                    }
                                    _ => cur.push(chars[i]),
                                }
                            }
                            other => cur.push(other),
                        }
                    } else {
                        cur.push(chars[i]);
                    }
                    i += 1;
                }
                if i >= chars.len() {
                    return Err("CURL_UNCLOSED_QUOTE".into());
                }
            }
            '"' => {
                in_word = true;
                i += 1;
                while i < chars.len() && chars[i] != '"' {
                    let escapes = if cmd_style { "\"" } else { "\"\\$`" };
                    if chars[i] == '\\' && chars.get(i + 1).is_some_and(|n| escapes.contains(*n)) {
                        i += 1;
                    }
                    cur.push(chars[i]);
                    i += 1;
                }
                if i >= chars.len() {
                    return Err("CURL_UNCLOSED_QUOTE".into());
                }
            }
            c if c.is_whitespace() => {
                if in_word {
                    words.push(std::mem::take(&mut cur));
                    in_word = false;
                }
            }
            c => {
                cur.push(c);
                in_word = true;
            }
        }
        i += 1;
    }
    if in_word {
        words.push(cur);
    }
    Ok(words)
}

pub fn parse_curl_command(input: &str) -> Result<CurlRequest, String> {
    let words = split_command_line(input.trim())?;
    let mut it = words.into_iter().peekable();
    match it.next() {
        Some(first)
            if first == "curl"
                || first.ends_with("/curl")
                || first.eq_ignore_ascii_case("curl.exe") => {}
        _ => return Err("CURL_NOT_A_COMMAND".into()),
    }
    let mut req = CurlRequest::default();
    let mut url: Option<String> = None;
    let mut method: Option<String> = None;
    let mut get_with_data = false;
    let mut query: Vec<String> = Vec::new();
    let mut sends_body = false;
    let mut other_positional: Option<String> = None;

    while let Some(word) = it.next() {
        let (flag, inline): (String, Option<String>) = if let Some(rest) = word.strip_prefix("--") {
            match rest.split_once('=') {
                Some((k, v)) => (format!("--{k}"), Some(v.to_string())),
                None => (word.clone(), None),
            }
        } else if word.len() > 2 && word.starts_with('-') && !word.starts_with("--") {
            let short = &word[..2];
            if matches!(short, "-H" | "-b" | "-A" | "-e" | "-u" | "-X" | "-d" | "-F") {
                (short.to_string(), Some(word[2..].to_string()))
            } else {
                (word.clone(), None)
            }
        } else {
            (word.clone(), None)
        };
        let value =
            |it: &mut std::iter::Peekable<std::vec::IntoIter<String>>| -> Result<String, String> {
                match inline.clone() {
                    Some(v) => Ok(v),
                    None => it.next().ok_or_else(|| "CURL_MISSING_VALUE".to_string()),
                }
            };
        match flag.as_str() {
            "-H" | "--header" => {
                let raw = value(&mut it)?;
                let Some((name, val)) = raw.split_once(':') else {
                    continue;
                };
                let name = name.trim().to_ascii_lowercase();
                let val = val.trim().to_string();
                if name.is_empty() || name.starts_with(':') {
                    continue;
                }
                match name.as_str() {
                    "referer" => req.referer = Some(val.clone()),
                    "user-agent" => req.user_agent = Some(val.clone()),
                    _ => {}
                }
                if !DROPPED_HEADERS.contains(&name.as_str()) && !val.is_empty() {
                    req.headers.insert(name, val);
                }
            }
            "-b" | "--cookie" => {
                let v = value(&mut it)?;
                if v.contains('=') {
                    let merged = match req.headers.get("cookie") {
                        Some(prev) => format!("{prev}; {v}"),
                        None => v,
                    };
                    req.headers.insert("cookie".into(), merged);
                }
            }
            "-A" | "--user-agent" => {
                let v = value(&mut it)?;
                req.headers.insert("user-agent".into(), v.clone());
                req.user_agent = Some(v);
            }
            "-e" | "--referer" => {
                let v = value(&mut it)?;
                let v = v.trim_end_matches(";auto").to_string();
                req.headers.insert("referer".into(), v.clone());
                req.referer = Some(v);
            }
            "-u" | "--user" => {
                let v = value(&mut it)?;
                let token = base64::engine::general_purpose::STANDARD.encode(v.as_bytes());
                req.headers
                    .insert("authorization".into(), format!("Basic {token}"));
            }
            "-X" | "--request" => method = Some(value(&mut it)?.to_ascii_uppercase()),
            "-G" | "--get" => get_with_data = true,
            "-I" | "--head" => method = Some("HEAD".into()),
            "--url" => url = Some(value(&mut it)?),
            f if SENDS_BODY.contains(&f) => {
                let v = value(&mut it)?;
                sends_body = true;
                query.push(v);
            }
            f if IGNORED_WITH_VALUE.contains(&f) => {
                value(&mut it)?;
            }
            f if f.starts_with('-') => {}
            _ => {
                let lower = word.to_ascii_lowercase();
                if lower.starts_with("http://") || lower.starts_with("https://") {
                    url.get_or_insert(word);
                } else {
                    other_positional.get_or_insert(word);
                }
            }
        }
    }

    if let Some(m) = method.as_deref() {
        if m != "GET" {
            return Err("CURL_NOT_GET".into());
        }
    }
    if sends_body && !get_with_data {
        return Err("CURL_NOT_GET".into());
    }
    let mut url = match (url, other_positional) {
        (Some(u), _) => u,
        (None, Some(_)) => return Err("CURL_NOT_HTTP".into()),
        (None, None) => return Err("CURL_NO_URL".into()),
    };
    let lower = url.to_ascii_lowercase();
    if !(lower.starts_with("http://") || lower.starts_with("https://")) {
        return Err("CURL_NOT_HTTP".into());
    }
    if get_with_data && !query.is_empty() {
        url.push(if url.contains('?') { '&' } else { '?' });
        url.push_str(&query.join("&"));
    }
    req.url = url;
    Ok(req)
}

fn host_of(url: &str) -> String {
    url::Url::parse(url)
        .ok()
        .and_then(|u| u.host_str().map(str::to_string))
        .unwrap_or_default()
}

fn preview_of(req: &CurlRequest) -> CurlPreview {
    CurlPreview {
        url: crate::core::flight_recorder::redact_url(&req.url),
        host: host_of(&req.url),
        header_names: req.headers.keys().cloned().collect(),
        has_cookie: req.headers.contains_key("cookie"),
        has_referer: req.referer.is_some(),
        has_user_agent: req.user_agent.is_some(),
    }
}

#[tauri::command]
pub fn curl_command_inspect(command: String) -> Result<CurlPreview, String> {
    parse_curl_command(&command).map(|req| preview_of(&req))
}

#[tauri::command]
pub async fn curl_command_download(
    app: tauri::AppHandle,
    command: String,
) -> Result<CurlQueued, String> {
    let req = parse_curl_command(&command)?;
    let payload = crate::extension_storage::ExtensionPayload {
        url: req.url.clone(),
        cookies: None,
        referer: req.referer.clone(),
        headers: Some(req.headers.clone().into_iter().collect()),
        media_type: None,
        content_type: None,
        title: None,
        thumbnail: None,
        open_app: None,
        page_url: req.referer.clone(),
        user_agent: req.user_agent.clone(),
        manifest_text: None,
    };
    crate::extension_storage::write_extension_metadata(&payload).map_err(|e| e.to_string())?;
    let result =
        crate::external_url::queue_url_with_defaults(&app, req.url.clone(), false, None).await;
    // The staged entry carries Cookie/Authorization; only a fresh queue consumes it.
    if !matches!(result, Ok(crate::external_url::QueueUrlOutcome::Queued)) {
        crate::extension_storage::remove_extension_metadata(&req.url);
    }
    let outcome = result?;
    Ok(CurlQueued {
        url: crate::core::flight_recorder::redact_url(&req.url),
        already_queued: outcome == crate::external_url::QueueUrlOutcome::AlreadyQueued,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_chrome_bash_copy() {
        let cmd = "curl 'https://files.example.com/a/b.zip?sig=x%3D1' \\\n  -H 'accept: */*' \\\n  -H 'accept-language: pt-BR,pt;q=0.9' \\\n  -b 'session=abc; theme=dark' \\\n  -H 'referer: https://example.com/page' \\\n  -H 'user-agent: Mozilla/5.0 (Macintosh)' \\\n  -H 'accept-encoding: gzip, deflate, br' \\\n  --compressed";
        let r = parse_curl_command(cmd).unwrap();
        assert_eq!(r.url, "https://files.example.com/a/b.zip?sig=x%3D1");
        assert_eq!(r.headers.get("cookie").unwrap(), "session=abc; theme=dark");
        assert_eq!(r.referer.as_deref(), Some("https://example.com/page"));
        assert_eq!(r.user_agent.as_deref(), Some("Mozilla/5.0 (Macintosh)"));
        assert!(!r.headers.contains_key("accept-encoding"));
        assert_eq!(r.headers.get("accept-language").unwrap(), "pt-BR,pt;q=0.9");
    }

    #[test]
    fn reads_firefox_copy_with_ansi_c_quotes() {
        let cmd = "curl 'https://cdn.example.org/v.mp4' -H 'User-Agent: Firefox' -H $'Cookie: a=\\'1\\'; b=2' -H 'Range: bytes=100-'";
        let r = parse_curl_command(cmd).unwrap();
        assert_eq!(r.headers.get("cookie").unwrap(), "a='1'; b=2");
        assert_eq!(r.headers.get("user-agent").unwrap(), "Firefox");
        assert!(!r.headers.contains_key("range"));
    }

    #[test]
    fn reads_windows_cmd_copy() {
        let cmd = "curl ^\"https://example.com/file.iso^\" ^\n  -H ^\"cookie: k=v^\" ^\n  -H ^\"referer: https://example.com/^\"";
        let r = parse_curl_command(cmd).unwrap();
        assert_eq!(r.url, "https://example.com/file.iso");
        assert_eq!(r.headers.get("cookie").unwrap(), "k=v");
        assert_eq!(r.referer.as_deref(), Some("https://example.com/"));
    }

    #[test]
    fn short_flags_and_basic_auth() {
        let r = parse_curl_command(
            "curl -L -s -o out.bin -u me:secret -A agent -e https://ref/ --url https://h/x",
        )
        .unwrap();
        assert_eq!(r.url, "https://h/x");
        assert_eq!(
            r.headers.get("authorization").unwrap(),
            "Basic bWU6c2VjcmV0"
        );
        assert_eq!(r.user_agent.as_deref(), Some("agent"));
        assert_eq!(r.referer.as_deref(), Some("https://ref/"));
    }

    #[test]
    fn get_with_data_becomes_a_query() {
        let r = parse_curl_command("curl -G https://h/s -d q=1 -d page=2").unwrap();
        assert_eq!(r.url, "https://h/s?q=1&page=2");
    }

    #[test]
    fn rejects_what_it_cannot_download() {
        assert_eq!(
            parse_curl_command("wget https://h/x").unwrap_err(),
            "CURL_NOT_A_COMMAND"
        );
        assert_eq!(
            parse_curl_command("curl https://h/x -X POST").unwrap_err(),
            "CURL_NOT_GET"
        );
        assert_eq!(
            parse_curl_command("curl https://h/x --data-raw '{}'").unwrap_err(),
            "CURL_NOT_GET"
        );
        assert_eq!(
            parse_curl_command("curl -H 'a: b'").unwrap_err(),
            "CURL_NO_URL"
        );
        assert_eq!(
            parse_curl_command("curl ftp://h/x").unwrap_err(),
            "CURL_NOT_HTTP"
        );
        assert_eq!(
            parse_curl_command("curl 'https://h/x").unwrap_err(),
            "CURL_UNCLOSED_QUOTE"
        );
    }

    #[test]
    fn preview_hides_values() {
        let r = parse_curl_command("curl 'https://h/x?token=abc' -b 'sid=secret'").unwrap();
        let p = preview_of(&r);
        assert!(p.has_cookie);
        assert_eq!(p.header_names, vec!["cookie".to_string()]);
        assert_eq!(p.host, "h");
        assert!(!serde_json::to_string(&p).unwrap().contains("secret"));
    }
}

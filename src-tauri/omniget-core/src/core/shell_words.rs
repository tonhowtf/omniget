use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParseError {
    UnclosedQuote(char),
    TrailingBackslash,
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ParseError::UnclosedQuote(q) => write!(f, "unclosed quote: {q}"),
            ParseError::TrailingBackslash => write!(f, "trailing backslash"),
        }
    }
}

impl std::error::Error for ParseError {}

pub fn split(input: &str) -> Result<Vec<String>, ParseError> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut in_token = false;
    let mut chars = input.chars().peekable();

    while let Some(c) = chars.next() {
        match c {
            '\'' => {
                in_token = true;
                loop {
                    match chars.next() {
                        Some('\'') => break,
                        Some(ch) => cur.push(ch),
                        None => return Err(ParseError::UnclosedQuote('\'')),
                    }
                }
            }
            '"' => {
                in_token = true;
                loop {
                    match chars.next() {
                        Some('"') => break,
                        Some('\\') => match chars.next() {
                            Some(esc @ ('"' | '\\' | '$' | '`')) => cur.push(esc),
                            Some(other) => {
                                cur.push('\\');
                                cur.push(other);
                            }
                            None => return Err(ParseError::UnclosedQuote('"')),
                        },
                        Some(ch) => cur.push(ch),
                        None => return Err(ParseError::UnclosedQuote('"')),
                    }
                }
            }
            '\\' => match chars.next() {
                Some('\n') => {}
                Some(ch) => {
                    in_token = true;
                    cur.push(ch);
                }
                None => return Err(ParseError::TrailingBackslash),
            },
            c if c.is_whitespace() => {
                if in_token {
                    out.push(std::mem::take(&mut cur));
                    in_token = false;
                }
            }
            other => {
                in_token = true;
                cur.push(other);
            }
        }
    }
    if in_token {
        out.push(cur);
    }
    Ok(out)
}

fn needs_quoting(arg: &str) -> bool {
    arg.is_empty()
        || arg.chars().any(|c| {
            c.is_whitespace()
                || matches!(
                    c,
                    '\'' | '"'
                        | '\\'
                        | '$'
                        | '`'
                        | '!'
                        | '*'
                        | '?'
                        | '['
                        | ']'
                        | '('
                        | ')'
                        | '{'
                        | '}'
                        | '<'
                        | '>'
                        | '|'
                        | '&'
                        | ';'
                        | '#'
                        | '~'
                )
        })
}

pub fn quote(arg: &str) -> String {
    if !needs_quoting(arg) {
        return arg.to_string();
    }
    if !arg.contains('\'') {
        return format!("'{arg}'");
    }
    let mut s = String::with_capacity(arg.len() + 2);
    s.push('"');
    for c in arg.chars() {
        if matches!(c, '"' | '\\' | '$' | '`') {
            s.push('\\');
        }
        s.push(c);
    }
    s.push('"');
    s
}

pub fn join<I, S>(args: I) -> String
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    args.into_iter()
        .map(|a| quote(a.as_ref()))
        .collect::<Vec<_>>()
        .join(" ")
}

const SECRET_VALUE_FLAGS: &[&str] = &[
    "--password",
    "--video-password",
    "--ap-password",
    "--client-certificate-password",
    "--proxy",
    "--all-proxy",
];

pub fn redact(args: &[String]) -> Vec<String> {
    let mut out = Vec::with_capacity(args.len());
    let mut redact_next = false;
    let mut header_next = false;
    let mut extractor_next = false;
    for a in args {
        if redact_next {
            redact_next = false;
            out.push(redact_proxy_or_secret(a));
            continue;
        }
        if header_next {
            header_next = false;
            out.push(redact_header_value(a));
            continue;
        }
        if extractor_next {
            extractor_next = false;
            out.push(redact_extractor_args(a));
            continue;
        }
        if SECRET_VALUE_FLAGS.contains(&a.as_str()) {
            redact_next = true;
            out.push(a.clone());
            continue;
        }
        if a == "--add-headers" || a == "--add-header" {
            header_next = true;
            out.push(a.clone());
            continue;
        }
        if a == "--extractor-args" {
            extractor_next = true;
            out.push(a.clone());
            continue;
        }
        if let Some((flag, value)) = a.split_once('=') {
            if SECRET_VALUE_FLAGS.contains(&flag) {
                out.push(format!("{flag}={}", redact_proxy_or_secret(value)));
                continue;
            }
            if flag == "--add-headers" || flag == "--add-header" {
                out.push(format!("{flag}={}", redact_header_value(value)));
                continue;
            }
            if flag == "--extractor-args" {
                out.push(format!("{flag}={}", redact_extractor_args(value)));
                continue;
            }
        }
        out.push(a.clone());
    }
    out
}

const IDENTITY_VALUE_FLAGS: &[&str] = &[
    "--username",
    "-u",
    "--ap-username",
    "--twofactor",
    "-2",
    "--cookies",
];

pub fn redact_for_log(args: &[String]) -> Vec<String> {
    let base = redact(args);
    let mut out = Vec::with_capacity(base.len());
    let mut identity_next = false;
    let mut header_next = false;
    for a in base {
        if identity_next {
            identity_next = false;
            out.push("<redacted>".to_string());
            continue;
        }
        if header_next {
            header_next = false;
            out.push(redact_sensitive_header(&a));
            continue;
        }
        if IDENTITY_VALUE_FLAGS.contains(&a.as_str()) {
            identity_next = true;
            out.push(a);
            continue;
        }
        if a == "--add-headers" || a == "--add-header" {
            header_next = true;
            out.push(a);
            continue;
        }
        if let Some((flag, value)) = a.split_once('=') {
            if IDENTITY_VALUE_FLAGS.contains(&flag) {
                out.push(format!("{flag}=<redacted>"));
                continue;
            }
            if flag == "--add-headers" || flag == "--add-header" {
                out.push(format!("{flag}={}", redact_sensitive_header(value)));
                continue;
            }
        }
        out.push(a);
    }
    out
}

fn redact_sensitive_header(v: &str) -> String {
    match v.split_once(':') {
        Some((name, _)) => {
            let n = name.trim().to_ascii_lowercase();
            let sensitive = [
                "cookie", "auth", "session", "token", "csrf", "xsrf", "key", "secret", "claim",
                "sign",
            ]
            .iter()
            .any(|s| n.contains(s));
            if sensitive {
                format!("{}:<redacted>", name.trim())
            } else {
                v.to_string()
            }
        }
        None => v.to_string(),
    }
}

fn redact_header_value(v: &str) -> String {
    match v.split_once(':') {
        Some((name, _)) if name.trim().eq_ignore_ascii_case("cookie") => {
            format!("{}:<redacted>", name.trim())
        }
        Some((name, _)) if name.trim().eq_ignore_ascii_case("authorization") => {
            format!("{}:<redacted>", name.trim())
        }
        _ => v.to_string(),
    }
}

fn redact_proxy_or_secret(v: &str) -> String {
    if let Ok(mut url) = url::Url::parse(v) {
        if url.username().is_empty() && url.password().is_none() {
            return v.to_string();
        }
        let _ = url.set_username("<redacted>");
        let _ = url.set_password(None);
        return url.to_string();
    }
    "<redacted>".to_string()
}

fn redact_extractor_args(v: &str) -> String {
    if !v.contains("po_token") {
        return v.to_string();
    }
    v.split(';')
        .map(|part| {
            let (ie, kv) = match part.split_once(':') {
                Some((ie, kv)) if !ie.contains('=') => (Some(ie), kv),
                _ => (None, part),
            };
            if kv.trim_start().starts_with("po_token") {
                match ie {
                    Some(ie) => format!("{ie}:po_token=<redacted>"),
                    None => "po_token=<redacted>".to_string(),
                }
            } else {
                part.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join(";")
}

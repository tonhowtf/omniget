use regex::{Captures, Regex};
use serde_json::Value;
use std::sync::OnceLock;

const REDACTED: &str = "[redacted]";

fn sensitive_key(key: &str) -> bool {
    let key = key.to_ascii_lowercase().replace(['-', '_'], "");
    matches!(
        key.as_str(),
        "authorization"
            | "proxyauthorization"
            | "cookie"
            | "setcookie"
            | "password"
            | "passwd"
            | "secret"
            | "clientsecret"
            | "apikey"
            | "xapikey"
            | "key"
            | "token"
            | "accesstoken"
            | "refreshtoken"
            | "idtoken"
            | "session"
            | "sessionid"
            | "jwt"
            | "signature"
            | "sig"
            | "credential"
            | "credentials"
            | "auth"
    ) || key.starts_with("xamz")
        || key.starts_with("xgoog")
        || key.ends_with("token")
        || key.ends_with("secret")
}

fn redact_url(raw: &str) -> String {
    let Ok(mut parsed) = url::Url::parse(raw) else {
        return raw.to_owned();
    };
    if !parsed.username().is_empty() || parsed.password().is_some() {
        let _ = parsed.set_username("redacted");
        let _ = parsed.set_password(None);
    }
    if parsed.query().is_some() {
        let pairs: Vec<(String, String)> = parsed
            .query_pairs()
            .map(|(k, v)| {
                let replacement = if sensitive_key(&k) || k.eq_ignore_ascii_case("policy") {
                    REDACTED.to_owned()
                } else {
                    v.into_owned()
                };
                (k.into_owned(), replacement)
            })
            .collect();
        parsed.query_pairs_mut().clear().extend_pairs(pairs);
    }
    if let Some(fragment) = parsed.fragment() {
        let pairs: Vec<(String, String)> = url::form_urlencoded::parse(fragment.as_bytes())
            .map(|(k, v)| {
                (
                    k.to_string(),
                    if sensitive_key(&k) || k.eq_ignore_ascii_case("policy") {
                        REDACTED.to_owned()
                    } else {
                        v.to_string()
                    },
                )
            })
            .collect();
        if pairs.iter().any(|(k, _)| sensitive_key(k)) {
            let encoded = url::form_urlencoded::Serializer::new(String::new())
                .extend_pairs(pairs)
                .finish();
            parsed.set_fragment(Some(&encoded));
        }
    }
    parsed.to_string()
}

pub fn redact_text(text: &str) -> String {
    static URL: OnceLock<Regex> = OnceLock::new();
    static HEADER: OnceLock<Regex> = OnceLock::new();
    static BEARER: OnceLock<Regex> = OnceLock::new();
    static ASSIGNMENT: OnceLock<Regex> = OnceLock::new();
    let urls = URL.get_or_init(|| Regex::new(r#"https?://[^\s<>"']+"#).unwrap());
    let headers = HEADER.get_or_init(|| {
        Regex::new(r"(?im)\b((?:proxy-)?authorization|set-cookie|cookie)\s*[:=]\s*[^\r\n]*")
            .unwrap()
    });
    let bearer =
        BEARER.get_or_init(|| Regex::new(r"(?i)\b(Bearer|Basic)\s+[A-Za-z0-9._~+/=-]+").unwrap());
    let assignments = ASSIGNMENT.get_or_init(|| Regex::new(r#"(?i)\b((?:access[_-]?|refresh[_-]?|id[_-]?|session[_-]?)?token|api[_-]?key|client[_-]?secret|password|passwd|signature)\s*[:=]\s*(?:"[^"\r\n]*"|'[^'\r\n]*'|[^\s,;\]}]+)"#).unwrap());
    let text = urls.replace_all(text, |c: &Captures| redact_url(&c[0]));
    let text = headers.replace_all(&text, |c: &Captures| format!("{}: {REDACTED}", &c[1]));
    let text = bearer.replace_all(&text, |c: &Captures| format!("{} {REDACTED}", &c[1]));
    assignments
        .replace_all(&text, |c: &Captures| format!("{}={REDACTED}", &c[1]))
        .into_owned()
}

pub fn redact_download_output(value: Value) -> Value {
    match value {
        Value::String(text) => {
            if let Ok(parsed @ (Value::Object(_) | Value::Array(_))) =
                serde_json::from_str::<Value>(&text)
            {
                Value::String(redact_download_output(parsed).to_string())
            } else {
                Value::String(redact_text(&text))
            }
        }
        Value::Array(values) => {
            Value::Array(values.into_iter().map(redact_download_output).collect())
        }
        Value::Object(values) => Value::Object(
            values
                .into_iter()
                .map(|(key, value)| {
                    let value = if sensitive_key(&key)
                        && !value.is_boolean()
                        && !value.is_number()
                        && !value.is_null()
                    {
                        Value::String(REDACTED.into())
                    } else {
                        redact_download_output(value)
                    };
                    (key, value)
                })
                .collect(),
        ),
        value => value,
    }
}

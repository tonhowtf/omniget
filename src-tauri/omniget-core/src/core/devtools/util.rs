use std::io::Read;

use base64::engine::general_purpose::{STANDARD, STANDARD_NO_PAD, URL_SAFE_NO_PAD};
use base64::Engine;
use chrono::{DateTime, SecondsFormat, Utc};
use md5::Digest;
use serde_json::{json, Value};

use super::{cap_chars, DevError};

pub const OPS: &[&str] = &[
    "json_format",
    "json_minify",
    "json_query",
    "jwt_decode",
    "base64_encode",
    "base64_decode",
    "url_encode",
    "url_decode",
    "hash",
    "uuid",
    "timestamp",
    "regex_test",
    "text_diff",
];

pub fn run(op: &str, a: &Value) -> Result<Value, DevError> {
    run_at(op, a, Utc::now())
}

pub fn run_at(op: &str, a: &Value, now: DateTime<Utc>) -> Result<Value, DevError> {
    match op {
        "json_format" => {
            let value = parse_json(text(a, op)?)?;
            capped(to_pretty(&value), a)
        }
        "json_minify" => capped(parse_json(text(a, op)?)?.to_string(), a),
        "json_query" => {
            let value = parse_json(text(a, op)?)?;
            let query = arg(a, "query", op)?;
            let found = json_query(&value, query)?;
            let mut out = capped(to_pretty(found), a)?;
            out["value"] = if out["truncated"] == true {
                Value::Null
            } else {
                found.clone()
            };
            out["type"] = json!(json_type(found));
            Ok(out)
        }
        "jwt_decode" => jwt_decode(text(a, op)?, now),
        "base64_encode" => capped(STANDARD.encode(text(a, op)?), a),
        "base64_decode" => base64_decode(text(a, op)?, a),
        "url_encode" => capped(urlencoding::encode(text(a, op)?).into_owned(), a),
        "url_decode" => {
            let decoded = urlencoding::decode(text(a, op)?).map_err(|_| {
                DevError::InvalidInput("the decoded bytes are not UTF-8 text".into())
            })?;
            capped(decoded.into_owned(), a)
        }
        "hash" => hash(a),
        "uuid" => Ok(json!({ "uuid": uuid::Uuid::new_v4().to_string() })),
        "timestamp" => timestamp(&a["value"], now),
        "regex_test" => regex_test(arg(a, "pattern", op)?, text(a, op)?, a),
        "text_diff" => text_diff(text(a, op)?, arg(a, "other", op)?, a),
        other => Err(DevError::InvalidInput(format!(
            "unknown op \"{other}\"; valid ops: {}",
            OPS.join(", ")
        ))),
    }
}

const MAX_INPUT_BYTES: usize = 1024 * 1024;
const DEFAULT_OUTPUT_CHARS: usize = 8000;
const DEFAULT_MATCHES: usize = 50;
const MAX_DIFF_LINES: usize = 2000;
const CONTEXT: usize = 3;

fn arg<'a>(a: &'a Value, name: &str, op: &str) -> Result<&'a str, DevError> {
    let value = a[name].as_str().ok_or_else(|| {
        DevError::InvalidInput(format!("argument \"{name}\" is required for {op}"))
    })?;
    if value.len() > MAX_INPUT_BYTES {
        return Err(DevError::InputTooLarge {
            limit: MAX_INPUT_BYTES,
        });
    }
    Ok(value)
}

fn text<'a>(a: &'a Value, op: &str) -> Result<&'a str, DevError> {
    arg(a, "text", op)
}

fn capped(full: String, a: &Value) -> Result<Value, DevError> {
    let max = a["maxChars"]
        .as_u64()
        .map_or(DEFAULT_OUTPUT_CHARS, |n| n as usize);
    let (text, truncated, total) = cap_chars(&full, max);
    Ok(json!({ "text": text, "truncated": truncated, "total": total }))
}

fn parse_json(text: &str) -> Result<Value, DevError> {
    serde_json::from_str(text).map_err(|e| DevError::InvalidInput(format!("invalid JSON: {e}")))
}

fn to_pretty(value: &Value) -> String {
    format!("{value:#}")
}

fn json_type(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

#[derive(Debug, PartialEq)]
enum Step {
    Key(String),
    Index(usize),
}

fn query_steps(query: &str) -> Result<Vec<Step>, DevError> {
    let bad = || DevError::InvalidInput(format!("invalid query \"{query}\"; use a.b[0][\"key\"]"));
    let mut steps = Vec::new();
    let mut rest = query.trim().trim_start_matches('$');
    while !rest.is_empty() {
        if let Some(after) = rest.strip_prefix('[') {
            let (inside, tail) = match after.chars().next() {
                Some(quote @ ('"' | '\'')) => {
                    let end = after[1..].find(quote).ok_or_else(bad)?;
                    let tail = after[end + 2..].strip_prefix(']').ok_or_else(bad)?;
                    steps.push(Step::Key(after[1..end + 1].to_string()));
                    rest = tail;
                    continue;
                }
                _ => after.split_once(']').ok_or_else(bad)?,
            };
            steps.push(Step::Index(inside.trim().parse().map_err(|_| bad())?));
            rest = tail;
        } else {
            let part = rest.strip_prefix('.').unwrap_or(rest);
            let end = part.find(['.', '[']).unwrap_or(part.len());
            if end == 0 {
                return Err(bad());
            }
            steps.push(Step::Key(part[..end].to_string()));
            rest = &part[end..];
        }
    }
    Ok(steps)
}

fn json_query<'a>(root: &'a Value, query: &str) -> Result<&'a Value, DevError> {
    let mut current = root;
    for step in query_steps(query)? {
        let next = match &step {
            Step::Key(key) => current.get(key),
            Step::Index(index) => current.get(index),
        };
        current = next.ok_or_else(|| {
            let (what, candidates) = match (&step, current) {
                (Step::Key(key), Value::Object(map)) => {
                    let mut keys: Vec<String> = map.keys().cloned().collect();
                    keys.sort();
                    (format!("\"{key}\" in {query}"), keys)
                }
                (Step::Index(index), Value::Array(items)) => (
                    format!(
                        "index {index} in {query} (the array has {} items)",
                        items.len()
                    ),
                    Vec::new(),
                ),
                (Step::Key(key), other) => (
                    format!(
                        "\"{key}\" in {query} (the value there is {})",
                        json_type(other)
                    ),
                    Vec::new(),
                ),
                (Step::Index(index), other) => (
                    format!(
                        "index {index} in {query} (the value there is {})",
                        json_type(other)
                    ),
                    Vec::new(),
                ),
            };
            DevError::NoMatch { what, candidates }
        })?;
    }
    Ok(current)
}

fn iso(time: DateTime<Utc>) -> String {
    time.to_rfc3339_opts(SecondsFormat::AutoSi, true)
}

fn jwt_decode(token: &str, now: DateTime<Utc>) -> Result<Value, DevError> {
    let bad = |what: &str| DevError::InvalidInput(format!("not a JWT: {what}"));
    let parts: Vec<&str> = token.trim().split('.').collect();
    if parts.len() != 3 {
        return Err(bad("expected three parts separated by dots"));
    }
    let part = |index: usize, name: &str| -> Result<Value, DevError> {
        let bytes = URL_SAFE_NO_PAD
            .decode(parts[index].trim_end_matches('='))
            .map_err(|_| bad(&format!("the {name} is not base64url")))?;
        serde_json::from_slice(&bytes).map_err(|_| bad(&format!("the {name} is not JSON")))
    };
    let header = part(0, "header")?;
    let payload = part(1, "payload")?;
    let expires = payload["exp"]
        .as_i64()
        .and_then(|exp| DateTime::from_timestamp(exp, 0));
    Ok(json!({
        "header": header,
        "payload": payload,
        "expiresAt": expires.map(iso),
        "expired": expires.map(|at| at <= now),
        "verified": false,
        "note": "decoded only: the signature was not verified, so the claims are untrusted",
    }))
}

fn base64_decode(input: &str, a: &Value) -> Result<Value, DevError> {
    let clean: String = input
        .chars()
        .filter(|c| !c.is_whitespace() && *c != '=')
        .collect();
    let bytes = STANDARD_NO_PAD
        .decode(&clean)
        .or_else(|_| URL_SAFE_NO_PAD.decode(&clean))
        .map_err(|e| DevError::InvalidInput(format!("invalid base64: {e}")))?;
    match String::from_utf8(bytes) {
        Ok(decoded) => capped(decoded, a),
        Err(e) => {
            let mut out = capped(hex::encode(e.as_bytes()), a)?;
            out["hex"] = out["text"].take();
            out["text"] = Value::Null;
            out["note"] = json!("the decoded bytes are not UTF-8 text, so they are shown as hex");
            Ok(out)
        }
    }
}

fn digest<D: Digest>(mut source: impl Read) -> std::io::Result<(String, u64)> {
    let mut hasher = D::new();
    let mut buffer = [0u8; 64 * 1024];
    let mut bytes = 0u64;
    loop {
        let read = source.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
        bytes += read as u64;
    }
    Ok((hex::encode(hasher.finalize()), bytes))
}

fn hash(a: &Value) -> Result<Value, DevError> {
    let algorithm = arg(a, "algorithm", "hash")?;
    let run = |source: &mut dyn Read| match algorithm {
        "md5" => Ok(digest::<md5::Md5>(source)),
        "sha1" => Ok(digest::<sha1::Sha1>(source)),
        "sha256" => Ok(digest::<sha2::Sha256>(source)),
        "sha512" => Ok(digest::<sha2::Sha512>(source)),
        other => Err(DevError::InvalidInput(format!(
            "unknown algorithm \"{other}\"; use md5, sha1, sha256 or sha512"
        ))),
    };
    let (result, source) = if let Some(path) = a["file"].as_str() {
        let failed = |e: std::io::Error| DevError::Read {
            path: path.into(),
            message: e.to_string(),
        };
        let mut file = std::fs::File::open(path).map_err(failed)?;
        (run(&mut file)?.map_err(failed)?, "file")
    } else {
        let mut bytes = text(a, "hash")?.as_bytes();
        let done = run(&mut bytes)?.map_err(|e| DevError::InvalidInput(e.to_string()))?;
        (done, "text")
    };
    Ok(json!({ "hash": result.0, "algorithm": algorithm, "bytes": result.1, "source": source }))
}

fn timestamp(value: &Value, now: DateTime<Utc>) -> Result<Value, DevError> {
    let from_number = |n: i64| {
        let time = if n.abs() >= 100_000_000_000 {
            DateTime::from_timestamp_millis(n)
        } else {
            DateTime::from_timestamp(n, 0)
        };
        time.ok_or_else(|| DevError::InvalidInput(format!("{n} is out of range for a timestamp")))
    };
    let time = match value {
        Value::Null => now,
        Value::Number(n) => from_number(n.as_i64().ok_or_else(|| {
            DevError::InvalidInput("value must be whole seconds or milliseconds".into())
        })?)?,
        Value::String(s) => match s.trim().parse::<i64>() {
            Ok(n) => from_number(n)?,
            Err(_) => DateTime::parse_from_rfc3339(s.trim())
                .map(|t| t.to_utc())
                .map_err(|_| {
                    DevError::InvalidInput(format!(
                        "\"{s}\" is neither a unix time nor an ISO 8601 time like 2026-01-31T12:00:00Z"
                    ))
                })?,
        },
        _ => {
            return Err(DevError::InvalidInput(
                "value must be a unix time or an ISO 8601 string".into(),
            ))
        }
    };
    Ok(json!({ "unix": time.timestamp(), "unixMs": time.timestamp_millis(), "iso": iso(time) }))
}

fn regex_test(pattern: &str, haystack: &str, a: &Value) -> Result<Value, DevError> {
    let regex = regex::Regex::new(pattern)
        .map_err(|e| DevError::InvalidInput(format!("invalid pattern: {e}")))?;
    let limit = a["limit"].as_u64().map_or(DEFAULT_MATCHES, |n| n as usize);
    let names: Vec<&str> = regex.capture_names().flatten().collect();
    let mut total = 0;
    let mut matches = Vec::new();
    for captures in regex.captures_iter(haystack) {
        total += 1;
        if matches.len() >= limit {
            continue;
        }
        let whole = captures.get(0).map_or(0..0, |m| m.range());
        let groups: Vec<Option<&str>> = captures
            .iter()
            .skip(1)
            .map(|g| g.map(|m| m.as_str()))
            .collect();
        let named: serde_json::Map<String, Value> = names
            .iter()
            .map(|name| {
                let value = captures.name(name).map(|m| m.as_str());
                (name.to_string(), json!(value))
            })
            .collect();
        matches.push(json!({
            "match": &haystack[whole.clone()],
            "start": whole.start,
            "end": whole.end,
            "groups": groups,
            "named": named,
        }));
    }
    let mut out = json!({ "matches": matches, "total": total, "truncated": total > limit });
    if total == 0 {
        out["reason"] = json!("no_match");
    }
    Ok(out)
}

#[derive(Clone, Copy, PartialEq)]
enum Edit {
    Same,
    Removed,
    Added,
}

fn edits<'a>(left: &[&'a str], right: &[&'a str]) -> Vec<(Edit, &'a str)> {
    let (n, m) = (left.len(), right.len());
    let mut table = vec![0u32; (n + 1) * (m + 1)];
    let at = |i: usize, j: usize| i * (m + 1) + j;
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            table[at(i, j)] = if left[i] == right[j] {
                table[at(i + 1, j + 1)] + 1
            } else {
                table[at(i + 1, j)].max(table[at(i, j + 1)])
            };
        }
    }
    let mut out = Vec::with_capacity(n.max(m));
    let (mut i, mut j) = (0, 0);
    while i < n || j < m {
        if i < n && j < m && left[i] == right[j] {
            out.push((Edit::Same, left[i]));
            i += 1;
            j += 1;
        } else if i < n && (j == m || table[at(i + 1, j)] >= table[at(i, j + 1)]) {
            out.push((Edit::Removed, left[i]));
            i += 1;
        } else {
            out.push((Edit::Added, right[j]));
            j += 1;
        }
    }
    out
}

fn text_diff(left: &str, right: &str, a: &Value) -> Result<Value, DevError> {
    let left: Vec<&str> = left.lines().collect();
    let right: Vec<&str> = right.lines().collect();
    if left.len() > MAX_DIFF_LINES || right.len() > MAX_DIFF_LINES {
        return Err(DevError::InvalidInput(format!(
            "text_diff compares at most {MAX_DIFF_LINES} lines per side"
        )));
    }
    let edits = edits(&left, &right);
    let changed: Vec<usize> = edits
        .iter()
        .enumerate()
        .filter(|(_, (edit, _))| *edit != Edit::Same)
        .map(|(index, _)| index)
        .collect();
    let mut hunks: Vec<(usize, usize)> = Vec::new();
    for index in &changed {
        let start = index.saturating_sub(CONTEXT);
        let end = (index + CONTEXT + 1).min(edits.len());
        match hunks.last_mut() {
            Some(last) if start <= last.1 => last.1 = end,
            _ => hunks.push((start, end)),
        }
    }
    let mut text = String::new();
    if !hunks.is_empty() {
        text.push_str("--- text\n+++ other\n");
    }
    for (start, end) in hunks {
        let before = &edits[..start];
        let old_start = before.iter().filter(|(e, _)| *e != Edit::Added).count();
        let new_start = before.iter().filter(|(e, _)| *e != Edit::Removed).count();
        let slice = &edits[start..end];
        let old_len = slice.iter().filter(|(e, _)| *e != Edit::Added).count();
        let new_len = slice.iter().filter(|(e, _)| *e != Edit::Removed).count();
        text.push_str(&format!(
            "@@ -{},{old_len} +{},{new_len} @@\n",
            old_start + usize::from(old_len > 0),
            new_start + usize::from(new_len > 0)
        ));
        for (edit, line) in slice {
            let sign = match edit {
                Edit::Same => ' ',
                Edit::Removed => '-',
                Edit::Added => '+',
            };
            text.push(sign);
            text.push_str(line);
            text.push('\n');
        }
    }
    let count = |kind: Edit| edits.iter().filter(|(e, _)| *e == kind).count();
    let mut out = capped(text, a)?;
    out["added"] = json!(count(Edit::Added));
    out["removed"] = json!(count(Edit::Removed));
    out["identical"] = json!(changed.is_empty());
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine;
    use serde_json::json;

    fn ok(op: &str, a: Value) -> Value {
        run(op, &a).unwrap()
    }

    #[test]
    fn json_format_indents_and_minify_removes_whitespace() {
        let out = ok("json_format", json!({"text": "{\"a\":[1,2]}"}));
        assert_eq!(out["text"], "{\n  \"a\": [\n    1,\n    2\n  ]\n}");
        assert_eq!(out["truncated"], false);
        let out = ok("json_minify", json!({"text": "{ \"a\" : [ 1, 2 ] }"}));
        assert_eq!(out["text"], "{\"a\":[1,2]}");
    }

    #[test]
    fn invalid_json_is_an_error_with_the_position() {
        let error = run("json_format", &json!({"text": "{\"a\":"})).unwrap_err();
        assert!(matches!(&error, DevError::InvalidInput(m) if m.contains("line 1")));
    }

    #[test]
    fn large_output_is_capped_and_says_so() {
        let out = ok(
            "json_format",
            json!({"text": "[1,2,3,4,5,6,7,8,9]", "maxChars": 10}),
        );
        assert_eq!(out["text"].as_str().unwrap().chars().count(), 10);
        assert_eq!(out["truncated"], true);
        assert_eq!(out["total"], 47);
    }

    #[test]
    fn json_query_follows_dots_and_brackets() {
        let text = r#"{"a":{"b":[{"c":1},{"c":2}],"k.e":"dot"}}"#;
        assert_eq!(
            ok("json_query", json!({"text": text, "query": "a.b[1].c"}))["value"],
            2
        );
        assert_eq!(
            ok("json_query", json!({"text": text, "query": "a[\"k.e\"]"}))["value"],
            "dot"
        );
        assert_eq!(
            ok("json_query", json!({"text": text, "query": "a.b"}))["type"],
            "array"
        );
    }

    #[test]
    fn json_query_miss_is_an_error_listing_the_keys_at_that_level() {
        let text = r#"{"user":{"name":"x","email":"y"}}"#;
        let error = run("json_query", &json!({"text": text, "query": "user.nome"})).unwrap_err();
        assert_eq!(
            error,
            DevError::NoMatch {
                what: "\"nome\" in user.nome".into(),
                candidates: vec!["email".into(), "name".into()],
            }
        );
        let error = run("json_query", &json!({"text": "[1]", "query": "[3]"})).unwrap_err();
        assert!(matches!(error, DevError::NoMatch { .. }));
    }

    fn jwt(payload: Value) -> String {
        let enc =
            |v: &Value| base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(v.to_string());
        format!(
            "{}.{}.c2ln",
            enc(&json!({"alg": "HS256", "typ": "JWT"})),
            enc(&payload)
        )
    }

    #[test]
    fn jwt_decode_shows_expiry_and_never_claims_verification() {
        let now = DateTime::parse_from_rfc3339("2026-01-01T00:00:00Z")
            .unwrap()
            .to_utc();
        let out = run_at(
            "jwt_decode",
            &json!({"text": jwt(json!({"sub": "u1", "exp": 1700000000}))}),
            now,
        )
        .unwrap();
        assert_eq!(out["header"]["alg"], "HS256");
        assert_eq!(out["payload"]["sub"], "u1");
        assert_eq!(out["expiresAt"], "2023-11-14T22:13:20Z");
        assert_eq!(out["expired"], true);
        assert_eq!(out["verified"], false);
        let out = run_at(
            "jwt_decode",
            &json!({"text": jwt(json!({"exp": 1900000000u64}))}),
            now,
        )
        .unwrap();
        assert_eq!(out["expired"], false);
        let out = run_at(
            "jwt_decode",
            &json!({"text": jwt(json!({"sub": "u1"}))}),
            now,
        )
        .unwrap();
        assert_eq!(out["expiresAt"], Value::Null);
        assert_eq!(out["expired"], Value::Null);
    }

    #[test]
    fn jwt_decode_rejects_what_is_not_a_token() {
        let error = run("jwt_decode", &json!({"text": "abc.def"})).unwrap_err();
        assert!(matches!(error, DevError::InvalidInput(_)));
    }

    #[test]
    fn base64_round_trips_and_accepts_url_safe_without_padding() {
        assert_eq!(
            ok("base64_encode", json!({"text": "olá?"}))["text"],
            "b2zDoT8="
        );
        assert_eq!(
            ok("base64_decode", json!({"text": "b2zDoT8="}))["text"],
            "olá?"
        );
        assert_eq!(
            ok("base64_decode", json!({"text": "b2zDoT8"}))["text"],
            "olá?"
        );
        assert_eq!(ok("base64_decode", json!({"text": "_-8"}))["hex"], "ffef");
        assert!(matches!(
            run("base64_decode", &json!({"text": "***"})),
            Err(DevError::InvalidInput(_))
        ));
    }

    #[test]
    fn url_encoding_round_trips() {
        assert_eq!(
            ok("url_encode", json!({"text": "a b&c=é"}))["text"],
            "a%20b%26c%3D%C3%A9"
        );
        assert_eq!(
            ok("url_decode", json!({"text": "a%20b%26c%3D%C3%A9"}))["text"],
            "a b&c=é"
        );
    }

    #[test]
    fn hash_matches_the_published_vectors() {
        let h = |alg: &str| {
            ok("hash", json!({"text": "abc", "algorithm": alg}))["hash"]
                .as_str()
                .unwrap()
                .to_string()
        };
        assert_eq!(h("md5"), "900150983cd24fb0d6963f7d28e17f72");
        assert_eq!(h("sha1"), "a9993e364706816aba3e25717850c26c9cd0d89d");
        assert_eq!(
            h("sha256"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert!(h("sha512").starts_with("ddaf35a193617aba"));
        assert!(matches!(
            run("hash", &json!({"text": "abc", "algorithm": "crc"})),
            Err(DevError::InvalidInput(_))
        ));
    }

    #[test]
    fn hash_reads_a_file_when_given_a_path() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.txt");
        std::fs::write(&path, "abc").unwrap();
        let out = ok(
            "hash",
            json!({"file": path.to_str().unwrap(), "algorithm": "md5"}),
        );
        assert_eq!(out["hash"], "900150983cd24fb0d6963f7d28e17f72");
        assert_eq!(out["bytes"], 3);
        assert!(matches!(
            run(
                "hash",
                &json!({"file": "/no/such/file", "algorithm": "md5"})
            ),
            Err(DevError::Read { .. })
        ));
    }

    #[test]
    fn uuid_is_a_version_4() {
        let out = ok("uuid", json!({}));
        let id = uuid::Uuid::parse_str(out["uuid"].as_str().unwrap()).unwrap();
        assert_eq!(id.get_version_num(), 4);
    }

    #[test]
    fn timestamp_converts_both_ways() {
        let out = ok("timestamp", json!({"value": 0}));
        assert_eq!(out["iso"], "1970-01-01T00:00:00Z");
        let out = ok("timestamp", json!({"value": "1700000000"}));
        assert_eq!(out["iso"], "2023-11-14T22:13:20Z");
        let out = ok("timestamp", json!({"value": 1700000000123u64}));
        assert_eq!(out["unix"], 1700000000);
        assert_eq!(out["unixMs"], 1700000000123u64);
        let out = ok("timestamp", json!({"value": "2023-11-14T22:13:20Z"}));
        assert_eq!(out["unix"], 1700000000);
        let out = ok("timestamp", json!({"value": "2023-11-14T19:13:20-03:00"}));
        assert_eq!(out["unix"], 1700000000);
        assert!(matches!(
            run("timestamp", &json!({"value": "yesterday"})),
            Err(DevError::InvalidInput(_))
        ));
    }

    #[test]
    fn timestamp_without_a_value_is_now() {
        let now = DateTime::parse_from_rfc3339("2026-01-01T00:00:00Z")
            .unwrap()
            .to_utc();
        let out = run_at("timestamp", &json!({}), now).unwrap();
        assert_eq!(out["iso"], "2026-01-01T00:00:00Z");
    }

    #[test]
    fn regex_test_returns_matches_with_groups_and_caps_them() {
        let out = ok(
            "regex_test",
            json!({"pattern": r"(\w)(?P<n>\d)", "text": "a1 b2 c3", "limit": 2}),
        );
        assert_eq!(out["total"], 3);
        assert_eq!(out["truncated"], true);
        assert_eq!(out["matches"].as_array().unwrap().len(), 2);
        assert_eq!(out["matches"][1]["match"], "b2");
        assert_eq!(out["matches"][1]["start"], 3);
        assert_eq!(out["matches"][1]["groups"], json!(["b", "2"]));
        assert_eq!(out["matches"][1]["named"]["n"], "2");
    }

    #[test]
    fn regex_test_without_a_match_is_explicit_and_a_bad_pattern_is_an_error() {
        let out = ok("regex_test", json!({"pattern": "z+", "text": "abc"}));
        assert_eq!(out["total"], 0);
        assert_eq!(out["reason"], "no_match");
        assert!(matches!(
            run("regex_test", &json!({"pattern": "(", "text": "abc"})),
            Err(DevError::InvalidInput(_))
        ));
    }

    #[test]
    fn text_diff_is_a_unified_diff() {
        let out = ok(
            "text_diff",
            json!({"text": "a\nb\nc\n", "other": "a\nx\nc\n"}),
        );
        assert_eq!(
            out["text"],
            "--- text\n+++ other\n@@ -1,3 +1,3 @@\n a\n-b\n+x\n c\n"
        );
        assert_eq!(out["added"], 1);
        assert_eq!(out["removed"], 1);
        assert_eq!(out["identical"], false);
    }

    #[test]
    fn text_diff_keeps_three_lines_of_context_and_splits_distant_hunks() {
        let left: Vec<String> = (1..=20).map(|n| n.to_string()).collect();
        let mut right = left.clone();
        right[1] = "two".into();
        right[17] = "eighteen".into();
        let out = ok(
            "text_diff",
            json!({"text": left.join("\n"), "other": right.join("\n")}),
        );
        let text = out["text"].as_str().unwrap();
        assert!(text.contains("@@ -1,5 +1,5 @@\n 1\n-2\n+two\n 3\n 4\n 5\n"));
        assert!(text.contains("@@ -15,6 +15,6 @@\n 15\n 16\n 17\n-18\n+eighteen\n 19\n 20\n"));
        assert!(!text.contains(" 10\n"));
    }

    #[test]
    fn text_diff_of_equal_texts_says_identical() {
        let out = ok("text_diff", json!({"text": "a\n", "other": "a\n"}));
        assert_eq!(out["identical"], true);
        assert_eq!(out["text"], "");
    }

    #[test]
    fn a_missing_argument_or_unknown_op_is_an_error() {
        assert_eq!(
            run("json_format", &json!({})),
            Err(DevError::InvalidInput(
                "argument \"text\" is required for json_format".into()
            ))
        );
        assert!(matches!(
            run("yaml_format", &json!({})),
            Err(DevError::InvalidInput(_))
        ));
    }

    #[test]
    fn every_listed_op_is_implemented() {
        for op in OPS {
            let result = run(op, &json!({}));
            assert!(
                !matches!(&result, Err(DevError::InvalidInput(m)) if m.starts_with("unknown op")),
                "{op}"
            );
            assert!(!matches!(result, Err(DevError::Unavailable(_))), "{op}");
        }
    }
}

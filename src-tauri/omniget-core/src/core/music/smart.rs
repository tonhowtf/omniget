//! Smart playlists: rules the user builds in the UI, turned into a
//! parameterized SQL `WHERE` over the `tracks` table. Field names come from a
//! fixed list, never from the rule text, so a rule can't inject SQL.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Rule {
    pub field: String,
    pub op: String,
    #[serde(default)]
    pub value: serde_json::Value,
    #[serde(default)]
    pub value2: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SmartQuery {
    #[serde(default)]
    pub rules: Vec<Rule>,
    /// true = every rule must match, false = any.
    #[serde(default = "yes")]
    pub match_all: bool,
    #[serde(default)]
    pub order: Option<String>,
    #[serde(default)]
    pub descending: bool,
    #[serde(default)]
    pub limit: Option<u32>,
}

fn yes() -> bool {
    true
}

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Text,
    Number,
    Bool,
    /// Unix milliseconds.
    Date,
}

fn column(field: &str) -> Option<(&'static str, Kind)> {
    Some(match field {
        "title" => ("title", Kind::Text),
        "artist" => ("artist", Kind::Text),
        "album" => ("album", Kind::Text),
        "album_artist" => ("album_artist", Kind::Text),
        "genre" => ("genre", Kind::Text),
        "composer" => ("composer", Kind::Text),
        "comment" => ("comment", Kind::Text),
        "path" => ("path", Kind::Text),
        "format" => ("codec", Kind::Text),
        "key" => ("musical_key", Kind::Text),
        "camelot" => ("camelot", Kind::Text),
        "year" => ("year", Kind::Number),
        "duration" => ("duration_ms / 1000.0", Kind::Number),
        "bitrate" => ("bitrate", Kind::Number),
        "sample_rate" => ("sample_rate", Kind::Number),
        "bpm" => ("bpm", Kind::Number),
        "play_count" => ("play_count", Kind::Number),
        "skip_count" => ("skip_count", Kind::Number),
        "rating" => ("rating", Kind::Number),
        "loudness" => ("loudness_lufs", Kind::Number),
        "liked" => ("liked", Kind::Bool),
        "has_lyrics" => ("has_lyrics", Kind::Bool),
        "added" => ("added_at", Kind::Date),
        "last_played" => ("last_played", Kind::Date),
        "modified" => ("mtime_ms", Kind::Date),
        _ => return None,
    })
}

fn order_column(field: &str) -> Option<&'static str> {
    Some(match field {
        "random" => "RANDOM()",
        "title" => "title COLLATE NOCASE",
        "artist" => "artist COLLATE NOCASE",
        "album" => "album COLLATE NOCASE",
        "year" => "year",
        "added" => "added_at",
        "last_played" => "last_played",
        "play_count" => "play_count",
        "rating" => "rating",
        "bpm" => "bpm",
        "duration" => "duration_ms",
        _ => return None,
    })
}

#[derive(Debug, Clone, PartialEq)]
pub enum Param {
    Text(String),
    Number(f64),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Compiled {
    /// `WHERE ...` body (no keyword), `1` when there are no rules.
    pub where_sql: String,
    pub params: Vec<Param>,
    /// `ORDER BY ... LIMIT ...` tail, may be empty.
    pub tail: String,
}

fn as_text(v: &serde_json::Value) -> String {
    match v {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Null => String::new(),
        other => other.to_string(),
    }
}

fn as_num(v: &serde_json::Value) -> Option<f64> {
    match v {
        serde_json::Value::Number(n) => n.as_f64(),
        serde_json::Value::String(s) => s.trim().parse().ok(),
        serde_json::Value::Bool(b) => Some(if *b { 1.0 } else { 0.0 }),
        _ => None,
    }
}

fn escape_like(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_")
}

fn compile_rule(r: &Rule, now_ms: i64, params: &mut Vec<Param>) -> Option<String> {
    let (col, kind) = column(&r.field)?;
    let sql = match (kind, r.op.as_str()) {
        (Kind::Text, "contains") | (Kind::Text, "not_contains") => {
            params.push(Param::Text(format!(
                "%{}%",
                escape_like(&as_text(&r.value))
            )));
            let not = if r.op == "not_contains" { "NOT " } else { "" };
            format!("COALESCE({col}, '') {not}LIKE ? ESCAPE '\\'")
        }
        (Kind::Text, "starts_with") => {
            params.push(Param::Text(format!("{}%", escape_like(&as_text(&r.value)))));
            format!("COALESCE({col}, '') LIKE ? ESCAPE '\\'")
        }
        (Kind::Text, "is") => {
            params.push(Param::Text(as_text(&r.value)));
            format!("COALESCE({col}, '') = ? COLLATE NOCASE")
        }
        (Kind::Text, "is_not") => {
            params.push(Param::Text(as_text(&r.value)));
            format!("COALESCE({col}, '') <> ? COLLATE NOCASE")
        }
        (Kind::Text, "empty") => format!("COALESCE({col}, '') = ''"),
        (Kind::Text, "not_empty") => format!("COALESCE({col}, '') <> ''"),
        (Kind::Number, op) => {
            let a = as_num(&r.value);
            match op {
                "is" => {
                    params.push(Param::Number(a?));
                    format!("{col} = ?")
                }
                "is_not" => {
                    params.push(Param::Number(a?));
                    format!("COALESCE({col}, -1) <> ?")
                }
                "gt" => {
                    params.push(Param::Number(a?));
                    format!("{col} > ?")
                }
                "lt" => {
                    params.push(Param::Number(a?));
                    format!("{col} < ?")
                }
                "between" => {
                    let b = as_num(&r.value2)?;
                    let a = a?;
                    params.push(Param::Number(a.min(b)));
                    params.push(Param::Number(a.max(b)));
                    format!("{col} BETWEEN ? AND ?")
                }
                "empty" => format!("({col} IS NULL OR {col} = 0)"),
                _ => return None,
            }
        }
        (Kind::Bool, "is_true") => format!("COALESCE({col}, 0) = 1"),
        (Kind::Bool, "is_false") => format!("COALESCE({col}, 0) = 0"),
        (Kind::Date, "in_last_days") | (Kind::Date, "not_in_last_days") => {
            let days = as_num(&r.value)?;
            params.push(Param::Number((now_ms as f64) - days * 86_400_000.0));
            if r.op == "in_last_days" {
                format!("{col} >= ?")
            } else {
                format!("({col} IS NULL OR {col} < ?)")
            }
        }
        (Kind::Date, "never") => format!("{col} IS NULL"),
        _ => return None,
    };
    Some(sql)
}

pub fn compile(q: &SmartQuery, now_ms: i64) -> Compiled {
    let mut params = Vec::new();
    let parts: Vec<String> = q
        .rules
        .iter()
        .filter_map(|r| compile_rule(r, now_ms, &mut params).map(|s| format!("({s})")))
        .collect();
    let where_sql = if parts.is_empty() {
        "1".to_string()
    } else {
        parts.join(if q.match_all { " AND " } else { " OR " })
    };
    let mut tail = String::new();
    if let Some(col) = q.order.as_deref().and_then(order_column) {
        tail.push_str(" ORDER BY ");
        tail.push_str(col);
        if q.descending && col != "RANDOM()" {
            tail.push_str(" DESC");
        }
    }
    if let Some(n) = q.limit.filter(|n| *n > 0) {
        tail.push_str(&format!(" LIMIT {}", n.min(100_000)));
    }
    Compiled {
        where_sql,
        params,
        tail,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn rule(field: &str, op: &str, value: serde_json::Value) -> Rule {
        Rule {
            field: field.into(),
            op: op.into(),
            value,
            value2: serde_json::Value::Null,
        }
    }

    #[test]
    fn builds_and_query_with_params() {
        let q = SmartQuery {
            rules: vec![
                rule("genre", "contains", json!("rock")),
                rule("year", "gt", json!(1990)),
                rule("liked", "is_true", json!(null)),
            ],
            match_all: true,
            order: Some("play_count".into()),
            descending: true,
            limit: Some(50),
        };
        let c = compile(&q, 0);
        assert_eq!(
            c.where_sql,
            "(COALESCE(genre, '') LIKE ? ESCAPE '\\') AND (year > ?) AND (COALESCE(liked, 0) = 1)"
        );
        assert_eq!(
            c.params,
            vec![Param::Text("%rock%".into()), Param::Number(1990.0)]
        );
        assert_eq!(c.tail, " ORDER BY play_count DESC LIMIT 50");
    }

    #[test]
    fn ignores_unknown_fields_and_ops() {
        let q = SmartQuery {
            rules: vec![
                rule("title; DROP TABLE tracks", "is", json!("x")),
                rule("year", "contains", json!("x")),
            ],
            match_all: false,
            order: Some("evil".into()),
            descending: false,
            limit: None,
        };
        let c = compile(&q, 0);
        assert_eq!(c.where_sql, "1");
        assert!(c.params.is_empty());
        assert_eq!(c.tail, "");
    }

    #[test]
    fn escapes_like_wildcards_and_dates() {
        let q = SmartQuery {
            rules: vec![
                rule("title", "contains", json!("100%")),
                rule("added", "in_last_days", json!(7)),
            ],
            match_all: false,
            order: None,
            descending: false,
            limit: None,
        };
        let c = compile(&q, 10 * 86_400_000);
        assert!(c.where_sql.contains(" OR "));
        assert_eq!(c.params[0], Param::Text("%100\\%%".into()));
        assert_eq!(c.params[1], Param::Number(3.0 * 86_400_000.0));
    }
}

use std::collections::HashMap;

use serde::Deserialize;
use serde_json::Value;

use super::{IconData, IconSet, IconsError};

pub const API_BASE: &str = "https://api.iconify.design";
pub const SEARCH_CAP: u32 = 128;
const MAX_NAMES_PER_REQUEST: usize = 200;

pub fn valid_prefix(prefix: &str) -> bool {
    !prefix.is_empty()
        && prefix.len() <= 64
        && prefix
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        && !prefix.starts_with('-')
}

pub fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 128
        && name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'_')
}

pub fn collections_url(base: &str) -> String {
    format!("{}/collections", base.trim_end_matches('/'))
}

pub fn search_url(
    base: &str,
    query: &str,
    limit: u32,
    prefixes: &[String],
    start: u32,
) -> Result<String, IconsError> {
    let query = query.trim();
    if query.is_empty() {
        return Err(IconsError::invalid(
            "query_empty",
            "Type something to search.",
        ));
    }
    if query.chars().count() > 100 {
        return Err(IconsError::invalid(
            "query_too_long",
            "The search is too long. Use at most 100 characters.",
        )
        .with("max", 100));
    }
    if let Some(bad) = prefixes.iter().find(|p| !valid_prefix(p)) {
        return Err(
            IconsError::invalid("unknown_set", format!("Unknown icon set \"{bad}\"."))
                .with("set", bad),
        );
    }
    let limit = limit.clamp(32, SEARCH_CAP);
    let mut url = format!(
        "{}/search?query={}&limit={}",
        base.trim_end_matches('/'),
        urlencoding::encode(query),
        limit
    );
    if start > 0 {
        url.push_str(&format!("&start={start}"));
    }
    if !prefixes.is_empty() {
        url.push_str("&prefixes=");
        url.push_str(&prefixes.join(","));
    }
    Ok(url)
}

pub fn icons_urls(base: &str, prefix: &str, names: &[String]) -> Result<Vec<String>, IconsError> {
    if !valid_prefix(prefix) {
        return Err(
            IconsError::invalid("unknown_set", format!("Unknown icon set \"{prefix}\"."))
                .with("set", prefix),
        );
    }
    if let Some(bad) = names.iter().find(|n| !valid_name(n)) {
        return Err(
            IconsError::invalid("bad_name", format!("Invalid icon name \"{bad}\"."))
                .with("name", bad),
        );
    }
    let mut sorted: Vec<&String> = names.iter().collect();
    sorted.sort();
    sorted.dedup();
    Ok(sorted
        .chunks(MAX_NAMES_PER_REQUEST)
        .map(|chunk| {
            let list: Vec<&str> = chunk.iter().map(|s| s.as_str()).collect();
            format!(
                "{}/{}.json?icons={}",
                base.trim_end_matches('/'),
                prefix,
                list.join(",")
            )
        })
        .collect())
}

fn str_field(v: &Value, key: &str) -> Option<String> {
    v.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

fn str_list(v: &Value, key: &str) -> Vec<String> {
    v.get(key)
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

pub fn parse_collections(text: &str) -> Result<Vec<IconSet>, IconsError> {
    let root: HashMap<String, Value> = serde_json::from_str(text).map_err(|e| {
        IconsError::http(
            "bad_collections",
            format!("The icon server sent a list we cannot read: {e}"),
        )
    })?;
    let mut sets: Vec<IconSet> = root
        .into_iter()
        .filter(|(prefix, v)| {
            valid_prefix(prefix) && !v.get("hidden").and_then(Value::as_bool).unwrap_or(false)
        })
        .map(|(prefix, v)| {
            let author = v.get("author");
            let license = v.get("license");
            let height = match v.get("height") {
                Some(Value::Number(n)) => n.as_u64().map(|h| h as u32),
                Some(Value::Array(a)) => a.first().and_then(Value::as_u64).map(|h| h as u32),
                _ => None,
            };
            IconSet {
                name: str_field(&v, "name").unwrap_or_else(|| prefix.clone()),
                total: v.get("total").and_then(Value::as_u64).unwrap_or(0) as u32,
                author: author.and_then(|a| str_field(a, "name")),
                author_url: author.and_then(|a| str_field(a, "url")),
                license: license.and_then(|l| str_field(l, "title")),
                license_spdx: license.and_then(|l| str_field(l, "spdx")),
                license_url: license.and_then(|l| str_field(l, "url")),
                category: str_field(&v, "category"),
                palette: v.get("palette").and_then(Value::as_bool).unwrap_or(false),
                height,
                samples: str_list(&v, "samples"),
                tags: str_list(&v, "tags"),
                prefix,
            }
        })
        .collect();
    sets.sort_by(|a, b| a.prefix.cmp(&b.prefix));
    Ok(sets)
}

#[derive(Deserialize)]
struct SearchBody {
    #[serde(default)]
    icons: Vec<String>,
    #[serde(default)]
    limit: Option<u32>,
}

pub fn parse_search(text: &str) -> Result<(Vec<String>, bool), IconsError> {
    let body: SearchBody = serde_json::from_str(text).map_err(|e| {
        IconsError::http(
            "bad_search",
            format!("The icon server sent results we cannot read: {e}"),
        )
    })?;
    let limit = body.limit.unwrap_or(SEARCH_CAP) as usize;
    let capped = !body.icons.is_empty() && body.icons.len() >= limit;
    let icons = body
        .icons
        .into_iter()
        .filter(|id| match id.split_once(':') {
            Some((p, n)) => valid_prefix(p) && valid_name(n),
            None => false,
        })
        .collect();
    Ok((icons, capped))
}

#[derive(Deserialize, Default, Clone)]
struct RawIcon {
    #[serde(default)]
    body: Option<String>,
    #[serde(default)]
    parent: Option<String>,
    width: Option<f64>,
    height: Option<f64>,
    left: Option<f64>,
    top: Option<f64>,
    #[serde(default)]
    rotate: Option<u8>,
    #[serde(default, rename = "hFlip")]
    h_flip: Option<bool>,
    #[serde(default, rename = "vFlip")]
    v_flip: Option<bool>,
}

#[derive(Deserialize)]
struct IconsBody {
    #[serde(default)]
    icons: HashMap<String, RawIcon>,
    #[serde(default)]
    aliases: HashMap<String, RawIcon>,
    width: Option<f64>,
    height: Option<f64>,
    left: Option<f64>,
    top: Option<f64>,
}

pub fn parse_icons(text: &str, requested: &[String]) -> Result<Vec<IconData>, IconsError> {
    let doc: IconsBody = serde_json::from_str(text).map_err(|e| {
        IconsError::http(
            "bad_icons",
            format!("The icon server sent icons we cannot read: {e}"),
        )
    })?;
    let default_w = doc.width.unwrap_or(16.0);
    let default_h = doc.height.unwrap_or(16.0);
    let mut out = Vec::new();
    for name in requested {
        if let Some(icon) = resolve(&doc, name, default_w, default_h) {
            out.push(icon);
        }
    }
    Ok(out)
}

fn resolve(doc: &IconsBody, name: &str, default_w: f64, default_h: f64) -> Option<IconData> {
    let mut chain: Vec<&RawIcon> = Vec::new();
    let mut current = name.to_string();
    for _ in 0..8 {
        if let Some(icon) = doc.icons.get(&current) {
            chain.push(icon);
            break;
        }
        let alias = doc.aliases.get(&current)?;
        chain.push(alias);
        current = alias.parent.clone()?;
    }
    let base = *chain.last()?;
    let body = base.body.clone()?;
    let mut rotate: u32 = 0;
    let mut h_flip = false;
    let mut v_flip = false;
    let mut width = None;
    let mut height = None;
    let mut left = None;
    let mut top = None;
    for item in chain.iter() {
        rotate += item.rotate.unwrap_or(0) as u32;
        h_flip ^= item.h_flip.unwrap_or(false);
        v_flip ^= item.v_flip.unwrap_or(false);
        width = width.or(item.width);
        height = height.or(item.height);
        left = left.or(item.left);
        top = top.or(item.top);
    }
    Some(IconData {
        name: name.to_string(),
        body,
        width: width.unwrap_or(default_w),
        height: height.unwrap_or(default_h),
        left: left.or(doc.left).unwrap_or(0.0),
        top: top.or(doc.top).unwrap_or(0.0),
        rotate: (rotate % 4) as u8,
        h_flip,
        v_flip,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_search_url_with_encoding_and_prefixes() {
        let url = search_url(
            API_BASE,
            " arrow left ",
            64,
            &["lucide".into(), "ph".into()],
            0,
        )
        .unwrap();
        assert_eq!(
            url,
            "https://api.iconify.design/search?query=arrow%20left&limit=64&prefixes=lucide,ph"
        );
    }

    #[test]
    fn clamps_search_limit_and_adds_start() {
        let url = search_url("http://x/", "home", 5000, &[], 32).unwrap();
        assert_eq!(url, "http://x/search?query=home&limit=128&start=32");
        let url = search_url("http://x", "home", 1, &[], 0).unwrap();
        assert!(url.ends_with("limit=32"));
    }

    #[test]
    fn rejects_bad_search_input() {
        assert!(matches!(
            search_url(API_BASE, "  ", 64, &[], 0),
            Err(e) if e.code == "query_empty"
        ));
        assert!(search_url(API_BASE, "a", 64, &["../etc".into()], 0).is_err());
    }

    #[test]
    fn builds_icons_urls_sorted_deduped_and_chunked() {
        let urls = icons_urls(
            API_BASE,
            "lucide",
            &["house".into(), "arrow-left".into(), "house".into()],
        )
        .unwrap();
        assert_eq!(
            urls,
            vec!["https://api.iconify.design/lucide.json?icons=arrow-left,house"]
        );
        let many: Vec<String> = (0..450).map(|i| format!("i{i}")).collect();
        assert_eq!(icons_urls(API_BASE, "mdi", &many).unwrap().len(), 3);
        assert!(icons_urls(API_BASE, "Lucide", &["a".into()]).is_err());
        assert!(icons_urls(API_BASE, "lucide", &["a/b".into()]).is_err());
    }

    #[test]
    fn collections_url_trims_slash() {
        assert_eq!(collections_url("http://x/"), "http://x/collections");
    }

    #[test]
    fn parses_collections() {
        let text = r#"{
            "lucide": {"name":"Lucide","total":2020,"author":{"name":"Lucide Contributors","url":"https://l"},
                "license":{"title":"ISC","spdx":"ISC"},"height":24,"category":"UI 24px","palette":false,"samples":["house"]},
            "heroicons": {"name":"HeroIcons","total":1288,"height":[16,24]},
            "secret": {"name":"Hidden","total":3,"hidden":true}
        }"#;
        let sets = parse_collections(text).unwrap();
        assert_eq!(sets.len(), 2);
        assert_eq!(sets[0].prefix, "heroicons");
        assert_eq!(sets[0].height, Some(16));
        assert_eq!(sets[1].license_spdx.as_deref(), Some("ISC"));
        assert_eq!(sets[1].author.as_deref(), Some("Lucide Contributors"));
        assert_eq!(sets[1].samples, vec!["house"]);
    }

    #[test]
    fn parses_search_and_detects_cap() {
        let (icons, capped) =
            parse_search(r#"{"icons":["lucide:home","bad id","ph:house"],"limit":32}"#).unwrap();
        assert_eq!(icons, vec!["lucide:home", "ph:house"]);
        assert!(!capped);
        let ids: Vec<String> = (0..32).map(|i| format!("\"mdi:a{i}\"")).collect();
        let (_, capped) =
            parse_search(&format!(r#"{{"icons":[{}],"limit":32}}"#, ids.join(","))).unwrap();
        assert!(capped);
    }

    #[test]
    fn resolves_aliases_with_transforms() {
        let text = r#"{"prefix":"x","width":24,"height":24,
            "icons":{"arrow":{"body":"<path d=\"M0 0\"/>","hFlip":true},"wide":{"body":"<g/>","width":32}},
            "aliases":{"arrow-left":{"parent":"arrow","rotate":2,"hFlip":true},"back":{"parent":"arrow-left","rotate":3},"ghost":{"parent":"nope"}}}"#;
        let names = ["arrow", "back", "wide", "ghost", "missing"].map(String::from);
        let icons = parse_icons(text, &names).unwrap();
        assert_eq!(icons.len(), 3);
        assert!(icons[0].h_flip);
        assert_eq!(icons[1].name, "back");
        assert_eq!(icons[1].rotate, 1);
        assert!(!icons[1].h_flip);
        assert_eq!(icons[2].width, 32.0);
        assert_eq!(icons[2].height, 24.0);
    }
}

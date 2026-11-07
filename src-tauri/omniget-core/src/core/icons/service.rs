use std::time::Duration;

use futures::future::join_all;

use super::cache::Cache;
use super::{api, CollectionsResult, ErrorKind, IconsError, IconsResult, SearchResult};

const COLLECTIONS_MAX_AGE: i64 = 24 * 3600;
const SEARCH_MAX_AGE: i64 = 6 * 3600;
const PER_SET_LIMIT: u32 = 128;
const MAX_FANOUT: usize = 12;

pub fn client() -> Result<reqwest::Client, IconsError> {
    crate::core::http_client::apply_global_proxy(reqwest::Client::builder())
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(12))
        .user_agent(concat!("OmniGet/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|e| IconsError::http("client", e.to_string()))
}

fn now() -> i64 {
    chrono::Utc::now().timestamp()
}

pub fn is_fresh(fetched_at: i64, now: i64, max_age: i64) -> bool {
    fetched_at <= now && now - fetched_at < max_age
}

async fn fetch(client: &reqwest::Client, url: &str) -> Result<String, IconsError> {
    let resp = client.get(url).send().await.map_err(|e| {
        if e.is_timeout() {
            IconsError::offline("timeout", "The icon server did not answer in time.")
        } else {
            IconsError::offline(
                "unreachable",
                format!("Could not reach the icon server: {e}"),
            )
        }
    })?;
    let status = resp.status();
    if !status.is_success() {
        return Err(IconsError::http(
            "status",
            format!("The icon server answered with error {}.", status.as_u16()),
        )
        .with("status", status.as_u16()));
    }
    resp.text().await.map_err(|e| {
        IconsError::offline(
            "interrupted",
            format!("The download from the icon server stopped: {e}"),
        )
    })
}

async fn cached_get(
    client: &reqwest::Client,
    cache: &Cache,
    url: &str,
    max_age: i64,
    refresh: bool,
) -> Result<(String, bool, i64), IconsError> {
    let hit = cache.read_response(url);
    if let Some(entry) = &hit {
        if !refresh && is_fresh(entry.fetched_at, now(), max_age) {
            return Ok((entry.body.clone(), false, entry.fetched_at));
        }
    }
    match fetch(client, url).await {
        Ok(body) => {
            let at = now();
            let _ = cache.write_response(url, &body, at);
            Ok((body, false, at))
        }
        Err(err) => match hit {
            Some(entry) => Ok((entry.body, true, entry.fetched_at)),
            None => Err(err),
        },
    }
}

pub async fn collections(
    client: &reqwest::Client,
    cache: &Cache,
    base: &str,
    refresh: bool,
) -> Result<CollectionsResult, IconsError> {
    let url = api::collections_url(base);
    let (body, offline, fetched_at) =
        cached_get(client, cache, &url, COLLECTIONS_MAX_AGE, refresh).await?;
    Ok(CollectionsResult {
        sets: api::parse_collections(&body)?,
        offline,
        fetched_at,
    })
}

pub async fn search(
    client: &reqwest::Client,
    cache: &Cache,
    base: &str,
    query: &str,
    prefixes: &[String],
    start: u32,
) -> Result<SearchResult, IconsError> {
    let groups: Vec<Vec<String>> = if prefixes.len() <= 1 {
        vec![prefixes.to_vec()]
    } else {
        prefixes
            .iter()
            .take(MAX_FANOUT)
            .map(|p| vec![p.clone()])
            .collect()
    };
    let urls = groups
        .iter()
        .map(|g| api::search_url(base, query, PER_SET_LIMIT, g, start))
        .collect::<Result<Vec<_>, _>>()?;
    let results = join_all(
        urls.iter()
            .map(|u| cached_get(client, cache, u, SEARCH_MAX_AGE, false)),
    )
    .await;
    let mut icons = Vec::new();
    let mut capped = false;
    let mut more = Vec::new();
    let mut offline = false;
    let mut first_err = None;
    let mut any_ok = false;
    for (group, r) in groups.iter().zip(results) {
        match r.and_then(|(body, off, _)| api::parse_search(&body).map(|p| (p, off))) {
            Ok(((list, cap), off)) => {
                any_ok = true;
                if cap {
                    capped = true;
                    more.extend(group.iter().cloned());
                }
                offline |= off;
                icons.extend(list);
            }
            Err(e) => {
                if e.is(ErrorKind::Offline) {
                    offline = true;
                }
                first_err.get_or_insert(e);
            }
        }
    }
    if !any_ok {
        return Err(first_err.unwrap_or_else(|| IconsError::offline("unreachable", "No results.")));
    }
    let mut seen = std::collections::HashSet::new();
    icons.retain(|id| seen.insert(id.clone()));
    Ok(SearchResult {
        icons,
        capped,
        more,
        next_start: start.saturating_add(PER_SET_LIMIT),
        trimmed: prefixes.len().saturating_sub(MAX_FANOUT),
        offline,
    })
}

pub async fn icons(
    client: &reqwest::Client,
    cache: &Cache,
    base: &str,
    prefix: &str,
    names: &[String],
) -> Result<IconsResult, IconsError> {
    api::icons_urls(base, prefix, names)?;
    let stored = cache.read_icons(prefix);
    let mut found = Vec::new();
    let mut wanted = Vec::new();
    for n in names {
        match stored.get(n) {
            Some(icon) => found.push(icon.clone()),
            None => wanted.push(n.clone()),
        }
    }
    if !found.is_empty() {
        let hits: Vec<String> = found.iter().map(|i| i.name.clone()).collect();
        let _ = cache.touch_icons(prefix, &hits, now());
    }
    let mut offline = false;
    if !wanted.is_empty() {
        let urls = api::icons_urls(base, prefix, &wanted)?;
        for url in urls {
            match fetch(client, &url).await {
                Ok(body) => {
                    let fresh = api::parse_icons(&body, &wanted)?;
                    let _ = cache.merge_icons(prefix, &fresh, now());
                    found.extend(fresh);
                }
                Err(e) if e.is(ErrorKind::Offline) => {
                    if found.is_empty() {
                        return Err(e);
                    }
                    offline = true;
                }
                Err(e) => return Err(e),
            }
        }
    }
    let have: std::collections::HashSet<&str> = found.iter().map(|i| i.name.as_str()).collect();
    let missing = names
        .iter()
        .filter(|n| !have.contains(n.as_str()))
        .cloned()
        .collect();
    Ok(IconsResult {
        prefix: prefix.to_string(),
        icons: found,
        missing,
        offline,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::icons::IconData;

    const DEAD: &str = "http://127.0.0.1:9";

    fn quick() -> reqwest::Client {
        reqwest::Client::builder()
            .connect_timeout(Duration::from_millis(300))
            .timeout(Duration::from_millis(600))
            .build()
            .unwrap()
    }

    #[test]
    fn freshness() {
        assert!(is_fresh(100, 150, 60));
        assert!(!is_fresh(100, 160, 60));
        assert!(!is_fresh(200, 100, 60));
    }

    #[tokio::test]
    async fn collections_fall_back_to_stale_cache_when_offline() {
        let dir = tempfile::tempdir().unwrap();
        let cache = Cache::new(dir.path());
        let url = api::collections_url(DEAD);
        cache
            .write_response(&url, r#"{"lucide":{"name":"Lucide","total":3}}"#, 1)
            .unwrap();
        let res = collections(&quick(), &cache, DEAD, false).await.unwrap();
        assert!(res.offline);
        assert_eq!(res.sets[0].name, "Lucide");
    }

    #[tokio::test]
    async fn fresh_cache_skips_network() {
        let dir = tempfile::tempdir().unwrap();
        let cache = Cache::new(dir.path());
        let url = api::search_url(DEAD, "home", PER_SET_LIMIT, &[], 0).unwrap();
        cache
            .write_response(&url, r#"{"icons":["lucide:house"],"limit":128}"#, now())
            .unwrap();
        let res = search(&quick(), &cache, DEAD, "home", &[], 0)
            .await
            .unwrap();
        assert_eq!(res.icons, vec!["lucide:house"]);
        assert!(!res.offline);
    }

    #[tokio::test]
    async fn search_without_cache_reports_offline() {
        let dir = tempfile::tempdir().unwrap();
        let cache = Cache::new(dir.path());
        let err = search(
            &quick(),
            &cache,
            DEAD,
            "home",
            &["lucide".into(), "ph".into()],
            0,
        )
        .await
        .unwrap_err();
        assert!(err.is(ErrorKind::Offline));
    }

    #[tokio::test]
    async fn icons_served_from_body_cache_offline() {
        let dir = tempfile::tempdir().unwrap();
        let cache = Cache::new(dir.path());
        let house = IconData {
            name: "house".into(),
            body: "<path/>".into(),
            width: 24.0,
            height: 24.0,
            left: 0.0,
            top: 0.0,
            rotate: 0,
            h_flip: false,
            v_flip: false,
        };
        cache.merge_icons("lucide", &[house], 1).unwrap();
        let res = icons(&quick(), &cache, DEAD, "lucide", &["house".into()])
            .await
            .unwrap();
        assert_eq!(res.icons.len(), 1);
        assert!(res.missing.is_empty());
        let res = icons(
            &quick(),
            &cache,
            DEAD,
            "lucide",
            &["house".into(), "star".into()],
        )
        .await
        .unwrap();
        assert!(res.offline);
        assert_eq!(res.missing, vec!["star"]);
        let err = icons(&quick(), &cache, DEAD, "ph", &["star".into()])
            .await
            .unwrap_err();
        assert!(err.is(ErrorKind::Offline));
    }
}

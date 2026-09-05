//! Query IDs do GraphQL do X (estudo 67). O X troca os ids a cada build do
//! site; um id velho responde `404 Query not found`. Tabela fixa (twscrape
//! 2026-08-28, twifork 2026-08-31) + raspagem dos bundles JS do proprio X,
//! como fazem o `update-gql-ops.py` do twscrape e o `queryIds.js` do
//! XActions. Cache em `tools/x/query-ids.json`.

use std::collections::HashMap;
use std::sync::Mutex;

use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};

pub const TABLE: &[(&str, &str)] = &[
    ("AboutAccountQuery", "TzOG2twZEfhr9KmClvVVqA"),
    ("BlueVerifiedFollowers", "u3PkPbg--arppBcwNbF1ig"),
    ("Bookmarks", "iblrFnKr6PZUR-dWpfXG6g"),
    ("BookmarkFoldersSlice", "i78YDd0Tza-dV4SYs58kRg"),
    ("BookmarkFolderTimeline", "g5l-N4fpbp7B-1OAbOdGzw"),
    ("CreateBookmark", "aoDbu3RHznuiSkQ9aNM67Q"),
    ("DeleteBookmark", "Wlmlj2-xzyS1GN3a6cj-mQ"),
    ("CreateGrokConversation", "vvC5uy7pWWHXS2aDi1FZeA"),
    ("GrokConversationItemsByRestId", "JfjvClaXup5BQFcwzcDUpA"),
    ("Followers", "JNyQdTISpzCkj_1fqxDvFg"),
    ("Following", "qGZZDF3mp91q7X22s3HxpA"),
    ("Likes", "BEthBswU1Bt209H5xptp4Q"),
    ("ListLatestTweetsTimeline", "1LE3u14FJjPZUHKFGzos2g"),
    ("ListMembers", "8rYmkvWQe9jRRZdy_-vkGA"),
    ("Retweeters", "ROjiuYueotTnWoI8m2YaiQ"),
    ("SearchTimeline", "hyPfJYJ_XAtDYoslQc-Rgg"),
    ("TweetDetail", "XMOz5h24KAZ86qKffKTLdQ"),
    ("TweetResultByRestId", "7xflPyRiUxGVbJd4uWmbfg"),
    ("UserByRestId", "xvmVfRLmnr1alc5f2dib0Q"),
    ("UserByScreenName", "Gb-d6r0vxPOADdG62OEBpQ"),
    ("UserMedia", "VyudDWQnr9vJNw7GasFz2g"),
    ("UserTweets", "SXVCYB8XHSS25nzIljNtZA"),
    ("UserTweetsAndReplies", "qUpkZU6eN8MbtQb7rC_pYg"),
    ("HomeTimeline", "-X_hcgQzmHGl29-UXxz4sw"),
    ("HomeLatestTimeline", "U0cdisy7QFIoTfu3-Okw0A"),
];

#[derive(Debug, Default, Serialize, Deserialize, Clone)]
struct Cache {
    #[serde(default)]
    fetched_at: i64,
    #[serde(default)]
    ids: HashMap<String, String>,
}

static CACHE: Lazy<Mutex<Option<Cache>>> = Lazy::new(|| Mutex::new(None));

fn cache_path() -> std::path::PathBuf {
    super::x_dir().join("query-ids.json")
}

fn load() -> Cache {
    let mut guard = CACHE.lock().unwrap();
    if let Some(c) = guard.as_ref() {
        return c.clone();
    }
    let c: Cache = std::fs::read_to_string(cache_path())
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default();
    *guard = Some(c.clone());
    c
}

fn save(c: &Cache) {
    if let Ok(s) = serde_json::to_string_pretty(c) {
        let _ = std::fs::write(cache_path(), s);
    }
    *CACHE.lock().unwrap() = Some(c.clone());
}

/// Id atual de uma operacao: cache raspado tem prioridade sobre a tabela.
pub fn id_for(op: &str) -> Option<String> {
    let c = load();
    if let Some(id) = c.ids.get(op) {
        return Some(id.clone());
    }
    TABLE.iter().find(|(name, _)| *name == op).map(|(_, id)| id.to_string())
}

/// Idade do cache em segundos (`None` = nunca raspado).
pub fn cache_age_secs() -> Option<i64> {
    let c = load();
    (c.fetched_at > 0).then(|| chrono::Utc::now().timestamp() - c.fetched_at)
}

pub fn cached_count() -> usize {
    load().ids.len()
}

fn script_urls(html: &str) -> Vec<String> {
    let mut urls: Vec<String> = Vec::new();
    // build atual (x-web / Vite) e legada (responsive-web) linkadas direto
    for re in [
        regex::Regex::new(r"https://[\w.-]+/x-web/[\w./-]+\.js").unwrap(),
        regex::Regex::new(r"https://[\w.-]+/responsive-web/client-web(?:-legacy)?/[\w./~-]+\.js").unwrap(),
    ] {
        for m in re.find_iter(html) {
            urls.push(m.as_str().to_string());
        }
    }
    // manifesto webpack: {id:"hash"} + {id:"nome"} → nome.hasha.js
    let re_hash = regex::Regex::new(r#"(\d+):"([0-9a-f]{7}|[0-9a-f]{16})""#).unwrap();
    let re_name = regex::Regex::new(r#"(\d+):"([^"]+)""#).unwrap();
    let mut hashes: HashMap<String, String> = HashMap::new();
    for c in re_hash.captures_iter(html) {
        hashes.insert(c[1].to_string(), c[2].to_string());
    }
    let mut names: HashMap<String, String> = HashMap::new();
    let re_is_hash = regex::Regex::new(r"^(?:[0-9a-f]{7}|[0-9a-f]{16})$").unwrap();
    for c in re_name.captures_iter(html) {
        if !re_is_hash.is_match(&c[2]) {
            names.insert(c[1].to_string(), c[2].to_string());
        }
    }
    let wanted = ["Bookmark", "Grok", "LoggedInMain", "HoverCard", "UserProfile", "UserHandler", "TweetActivity", "TweetEditHistory", "HomeTimeline", "Follow", "Search", "Explore", "Lists"];
    for (id, hash) in &hashes {
        let name = names.get(id).cloned().unwrap_or_else(|| id.clone());
        if name.starts_with("i18n/") || name.starts_with("ondemand.countries") || name == "vendor" {
            continue;
        }
        if wanted.iter().any(|w| name.contains(w)) {
            urls.push(format!("https://abs.twimg.com/responsive-web/client-web/{}.{}a.js", name, hash));
        }
    }
    let mut seen = std::collections::HashSet::new();
    urls.into_iter().filter(|u| !u.contains("/i18n/") && seen.insert(u.clone())).collect()
}

pub fn extract_ops(js: &str) -> Vec<(String, String)> {
    let re = regex::Regex::new(r#"queryId:\s*"([A-Za-z0-9_-]+)"[^}]{0,300}?operationName:\s*"([A-Za-z0-9_]+)""#).unwrap();
    let re2 = regex::Regex::new(r#"operationName:\s*"([A-Za-z0-9_]+)"[^}]{0,300}?queryId:\s*"([A-Za-z0-9_-]+)""#).unwrap();
    let mut out = Vec::new();
    for c in re.captures_iter(js) {
        out.push((c[2].to_string(), c[1].to_string()));
    }
    for c in re2.captures_iter(js) {
        out.push((c[1].to_string(), c[2].to_string()));
    }
    out
}

/// Carrega a home (com cookies quando houver) e a pagina de login, segue os
/// bundles e atualiza o cache. Devolve quantas operacoes conhece agora.
pub async fn refresh(http: &reqwest::Client, cookie: Option<&str>) -> anyhow::Result<usize> {
    let mut html = String::new();
    for page in ["https://x.com/home", "https://x.com/i/flow/login", "https://x.com/"] {
        let mut req = http.get(page);
        if let Some(c) = cookie {
            req = req.header("Cookie", c);
        }
        if let Ok(resp) = req.send().await {
            if let Ok(text) = resp.text().await {
                if text.contains("document.location = \"") {
                    if let Some(next) = text.split("document.location = \"").nth(1).and_then(|s| s.split('"').next()) {
                        if let Ok(r2) = http.get(next).send().await {
                            if let Ok(t2) = r2.text().await {
                                html.push_str(&t2);
                            }
                        }
                    }
                }
                html.push_str(&text);
            }
        }
    }
    let urls = script_urls(&html);
    if urls.is_empty() {
        anyhow::bail!("nao achei os bundles JS do X na pagina");
    }
    let mut found: HashMap<String, String> = HashMap::new();
    let sem = std::sync::Arc::new(tokio::sync::Semaphore::new(8));
    let mut tasks = Vec::new();
    for url in urls.into_iter().take(60) {
        let http = http.clone();
        let sem = sem.clone();
        tasks.push(tokio::spawn(async move {
            let _p = sem.acquire().await.ok()?;
            let resp = http.get(&url).send().await.ok()?;
            if !resp.status().is_success() {
                return None;
            }
            let text = resp.text().await.ok()?;
            Some(extract_ops(&text))
        }));
    }
    for t in tasks {
        if let Ok(Some(ops)) = t.await {
            for (op, id) in ops {
                found.entry(op).or_insert(id);
            }
        }
    }
    if found.is_empty() {
        anyhow::bail!("baixei os bundles do X mas nao achei operacoes GraphQL neles");
    }
    let mut c = load();
    for (op, id) in found {
        c.ids.insert(op, id);
    }
    c.fetched_at = chrono::Utc::now().timestamp();
    save(&c);
    Ok(c.ids.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_ops_from_bundle() {
        let js = r#"e.exports={queryId:"XMOz5h24KAZ86qKffKTLdQ",operationName:"TweetDetail",operationType:"query",metadata:{}};x={operationName:"Bookmarks",queryId:"iblrFnKr6PZUR-dWpfXG6g"}"#;
        let ops = extract_ops(js);
        assert!(ops.contains(&("TweetDetail".to_string(), "XMOz5h24KAZ86qKffKTLdQ".to_string())));
        assert!(ops.contains(&("Bookmarks".to_string(), "iblrFnKr6PZUR-dWpfXG6g".to_string())));
    }

    #[test]
    fn manifest_urls() {
        let html = r#"<script src="https://abs.twimg.com/responsive-web/client-web/main.abcdef1a.js"></script>p.u=e=>""+({1:"bundle.Bookmarks",2:"vendor"}[e]||e)+"."+({1:"0123abc",2:"deadbee"})[e]+"a.js""#;
        let urls = script_urls(html);
        assert!(urls.iter().any(|u| u.contains("main.abcdef1a.js")));
        assert!(urls.iter().any(|u| u.ends_with("bundle.Bookmarks.0123abca.js")));
        assert!(!urls.iter().any(|u| u.contains("vendor")));
    }
}

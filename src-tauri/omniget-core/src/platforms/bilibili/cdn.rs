const BLOCKED_HOST_PATTERNS: &[&str] = &["szbdyd.com", "mcdn"];

pub fn parse_hosts(setting_value: &str) -> Vec<String> {
    setting_value
        .split([',', '\n'])
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty() && !s.starts_with('#'))
        .map(|s| {
            s.trim_start_matches("http://")
                .trim_start_matches("https://")
                .trim_end_matches('/')
                .to_string()
        })
        .filter(|s| !s.is_empty())
        .collect()
}

pub fn filter_blocked(urls: &[String]) -> Vec<String> {
    urls.iter()
        .filter(|u| !host_is_blocked(u))
        .cloned()
        .collect()
}

pub fn host_is_blocked(url: &str) -> bool {
    let host = match url::Url::parse(url)
        .ok()
        .and_then(|u| u.host_str().map(String::from))
    {
        Some(h) => h.to_lowercase(),
        None => return false,
    };
    BLOCKED_HOST_PATTERNS
        .iter()
        .any(|p| host.contains(&p.to_lowercase()))
}

pub fn replace_host(url: &str, new_host: &str) -> Option<String> {
    let parsed = url::Url::parse(url).ok()?;
    let scheme = parsed.scheme();
    let path = parsed.path();
    let query = parsed
        .query()
        .map(|q| format!("?{}", q))
        .unwrap_or_default();
    Some(format!("{}://{}{}{}", scheme, new_host, path, query))
}

pub fn expand_with_alternatives(
    primary: &str,
    backups: &[String],
    alt_hosts: &[String],
    prefer_alternatives: bool,
) -> Vec<String> {
    let mut originals: Vec<String> = std::iter::once(primary.to_string())
        .chain(backups.iter().cloned())
        .collect();
    originals = filter_blocked(&originals);

    let mut alternatives: Vec<String> = Vec::new();
    for source in &originals {
        for host in alt_hosts {
            if let Some(replaced) = replace_host(source, host) {
                if !alternatives.contains(&replaced) && !originals.contains(&replaced) {
                    alternatives.push(replaced);
                }
            }
        }
    }

    let mut chain: Vec<String> = if prefer_alternatives {
        alternatives.into_iter().chain(originals).collect()
    } else {
        originals.into_iter().chain(alternatives).collect()
    };

    let mut seen = std::collections::HashSet::new();
    chain.retain(|u| seen.insert(u.clone()));
    chain
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_hosts_accepts_csv_and_newlines() {
        let raw = "upos-hz-mirrorcos.bilivideo.com, upos-sz-mirroraliov.bilivideo.com\nhttps://upos-cos-cf.bilibili.com/\n#comment\n";
        let hosts = parse_hosts(raw);
        assert_eq!(
            hosts,
            vec![
                "upos-hz-mirrorcos.bilivideo.com",
                "upos-sz-mirroraliov.bilivideo.com",
                "upos-cos-cf.bilibili.com",
            ]
        );
    }

    #[test]
    fn filter_blocks_known_bad_hosts() {
        let urls = vec![
            "https://upos-sz-mirrorcosbktx.bilivideo.com/a.m4s".into(),
            "https://upos-sz-mirror08hw.bilivideo.szbdyd.com/a.m4s".into(),
            "https://mcdn.bilivideo.cn/a.m4s".into(),
        ];
        let filtered = filter_blocked(&urls);
        assert_eq!(filtered.len(), 1);
        assert!(filtered[0].contains("mirrorcosbktx"));
    }

    #[test]
    fn replace_host_swaps_authority() {
        let out = replace_host(
            "https://upos-sz-mirrorcosbktx.bilivideo.com/p/a.m4s?q=1",
            "upos-hz-mirrorcos.bilivideo.com",
        )
        .unwrap();
        assert_eq!(out, "https://upos-hz-mirrorcos.bilivideo.com/p/a.m4s?q=1");
    }

    #[test]
    fn expand_prefers_alternatives_when_requested() {
        let primary = "https://upos-sz-mirrorcosbktx.bilivideo.com/x.m4s".to_string();
        let alts = vec!["upos-hz-mirrorcos.bilivideo.com".to_string()];
        let chain = expand_with_alternatives(&primary, &[], &alts, true);
        assert!(chain[0].contains("upos-hz-mirrorcos"));
        assert!(chain[1].contains("upos-sz-mirrorcosbktx"));
    }

    #[test]
    fn expand_fallback_keeps_original_first_by_default() {
        let primary = "https://upos-sz-mirrorcosbktx.bilivideo.com/x.m4s".to_string();
        let alts = vec!["upos-hz-mirrorcos.bilivideo.com".to_string()];
        let chain = expand_with_alternatives(&primary, &[], &alts, false);
        assert!(chain[0].contains("upos-sz-mirrorcosbktx"));
        assert!(chain[1].contains("upos-hz-mirrorcos"));
    }
}

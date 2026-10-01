use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Playlist {
    pub entries: Vec<M3uEntry>,
    pub epg_urls: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct M3uEntry {
    pub name: String,
    pub url: String,
    pub group: Option<String>,
    pub tvg_id: Option<String>,
    pub tvg_name: Option<String>,
    pub tvg_logo: Option<String>,
    pub user_agent: Option<String>,
    pub referrer: Option<String>,
    pub headers: Vec<(String, String)>,
    pub catchup: Option<String>,
    pub catchup_source: Option<String>,
    pub catchup_days: Option<u32>,
    pub duration: Option<f64>,
}

#[derive(Default)]
struct Pending {
    has_info: bool,
    duration: Option<f64>,
    name: String,
    attrs: Vec<(String, String)>,
    group: Option<String>,
    opts: Vec<(String, String)>,
}

fn parse_attrs(s: &str) -> (Vec<(String, String)>, Option<String>) {
    let chars: Vec<char> = s.chars().collect();
    let mut i = 0;
    let mut attrs = Vec::new();
    let n = chars.len();
    while i < n {
        while i < n && chars[i].is_whitespace() {
            i += 1;
        }
        if i >= n {
            break;
        }
        if chars[i] == ',' {
            let rest: String = chars[i + 1..].iter().collect();
            return (attrs, Some(rest.trim().to_string()));
        }
        let start = i;
        while i < n && chars[i] != '=' && chars[i] != ',' && !chars[i].is_whitespace() {
            i += 1;
        }
        let key: String = chars[start..i].iter().collect();
        if i < n && chars[i] == '=' {
            i += 1;
            let value: String;
            if i < n && (chars[i] == '"' || chars[i] == '\'') {
                let q = chars[i];
                i += 1;
                let vs = i;
                while i < n && chars[i] != q {
                    i += 1;
                }
                if i < n {
                    value = chars[vs..i].iter().collect();
                    i += 1;
                } else {
                    let mut j = vs;
                    while j < n && chars[j] != ',' {
                        j += 1;
                    }
                    value = chars[vs..j].iter().collect();
                    i = j;
                }
            } else {
                let vs = i;
                while i < n && chars[i] != ',' && !chars[i].is_whitespace() {
                    i += 1;
                }
                value = chars[vs..i].iter().collect();
            }
            if !key.is_empty() {
                attrs.push((key.to_ascii_lowercase(), value.trim().to_string()));
            }
        } else if i == start {
            i += 1;
        }
    }
    (attrs, None)
}

fn attr<'a>(attrs: &'a [(String, String)], keys: &[&str]) -> Option<&'a str> {
    keys.iter().find_map(|k| {
        attrs
            .iter()
            .find(|(ak, v)| ak == k && !v.is_empty())
            .map(|(_, v)| v.as_str())
    })
}

fn decode(v: &str) -> String {
    let v = v.trim().trim_matches(['"', '\'']);
    urlencoding::decode(v)
        .map(|c| c.into_owned())
        .unwrap_or_else(|_| v.to_string())
}

fn header_pairs(s: &str) -> Vec<(String, String)> {
    s.split('&')
        .filter_map(|kv| {
            let (k, v) = kv.split_once('=')?;
            let k = k.trim();
            (!k.is_empty()).then(|| (k.to_string(), decode(v)))
        })
        .collect()
}

fn apply_header(e: &mut M3uEntry, key: &str, value: String) {
    if value.is_empty() {
        return;
    }
    let lk = key.to_ascii_lowercase();
    match lk.as_str() {
        "user-agent" | "useragent" | "http-user-agent" => e.user_agent = Some(value),
        "referer" | "referrer" | "http-referrer" | "http-referer" => e.referrer = Some(value),
        _ => {
            let name = match lk.as_str() {
                "origin" | "http-origin" => "Origin".to_string(),
                "cookie" | "http-cookie" => "Cookie".to_string(),
                _ => key.to_string(),
            };
            e.headers.retain(|(k, _)| !k.eq_ignore_ascii_case(&name));
            e.headers.push((name, value));
        }
    }
}

fn apply_option(e: &mut M3uEntry, key: &str, value: &str) {
    let lk = key.trim().to_ascii_lowercase();
    match lk.as_str() {
        "http-user-agent" | "http-referrer" | "http-referer" | "http-origin" | "http-cookie" => {
            apply_header(e, &lk, value.trim().trim_matches('"').to_string())
        }
        "inputstream.adaptive.stream_headers"
        | "inputstream.adaptive.common_headers"
        | "inputstream.adaptive.manifest_headers" => {
            for (k, v) in header_pairs(value) {
                apply_header(e, &k, v);
            }
        }
        "exthttp" => {
            if let Ok(serde_json::Value::Object(map)) =
                serde_json::from_str::<serde_json::Value>(value)
            {
                for (k, v) in map {
                    if let Some(s) = v.as_str() {
                        apply_header(e, &k, s.to_string());
                    }
                }
            }
        }
        _ => {}
    }
}

fn finish(p: Pending, line: &str) -> Option<M3uEntry> {
    let (url, suffix) = match line.split_once('|') {
        Some((u, s)) => (u.trim(), Some(s)),
        None => (line.trim(), None),
    };
    if url.is_empty() {
        return None;
    }
    let mut e = M3uEntry {
        url: url.to_string(),
        duration: p.duration.filter(|d| *d > 0.0),
        ..M3uEntry::default()
    };
    let a = &p.attrs;
    e.tvg_id = attr(a, &["tvg-id"]).map(str::to_string);
    e.tvg_name = attr(a, &["tvg-name"]).map(str::to_string);
    e.tvg_logo = attr(a, &["tvg-logo", "logo"]).map(str::to_string);
    e.group = attr(a, &["group-title"])
        .map(str::to_string)
        .or(p.group.clone());
    e.catchup = attr(a, &["catchup", "catchup-type"]).map(str::to_string);
    e.catchup_source = attr(a, &["catchup-source"]).map(str::to_string);
    e.catchup_days = attr(a, &["catchup-days", "timeshift"]).and_then(|v| v.trim().parse().ok());
    if let Some(ua) = attr(a, &["user-agent", "http-user-agent", "tvg-user-agent"]) {
        e.user_agent = Some(ua.to_string());
    }
    if let Some(r) = attr(a, &["referrer", "referer", "http-referrer", "http-referer"]) {
        e.referrer = Some(r.to_string());
    }
    for (k, v) in &p.opts {
        apply_option(&mut e, k, v);
    }
    if let Some(s) = suffix {
        for (k, v) in header_pairs(s) {
            apply_header(&mut e, &k, v);
        }
    }
    e.name = if !p.name.is_empty() {
        p.name
    } else if let Some(t) = &e.tvg_name {
        t.clone()
    } else {
        url.rsplit('/')
            .find(|s| !s.is_empty())
            .unwrap_or(url)
            .to_string()
    };
    Some(e)
}

pub fn parse(text: &str) -> Playlist {
    let mut out = Playlist::default();
    let mut p = Pending::default();
    let text = text.trim_start_matches('\u{feff}');
    for raw in text.split(['\n', '\r']) {
        let line = raw.trim().trim_start_matches('\u{feff}');
        if line.is_empty() {
            continue;
        }
        if let Some(rest) = line.strip_prefix('#') {
            let upper: String = rest
                .chars()
                .take(12)
                .collect::<String>()
                .to_ascii_uppercase();
            if upper.starts_with("EXTM3U") {
                let (attrs, _) = parse_attrs(&rest[6..]);
                for (k, v) in attrs {
                    if matches!(k.as_str(), "url-tvg" | "x-tvg-url" | "tvg-url") {
                        for u in v.split(',').map(str::trim).filter(|u| !u.is_empty()) {
                            if !out.epg_urls.iter().any(|x| x == u) {
                                out.epg_urls.push(u.to_string());
                            }
                        }
                    }
                }
            } else if upper.starts_with("EXTINF:") {
                if p.has_info {
                    p = Pending {
                        opts: std::mem::take(&mut p.opts),
                        group: p.group.take(),
                        ..Pending::default()
                    };
                }
                let body = &rest[7..];
                let dur_end = body
                    .find(|c: char| c.is_whitespace() || c == ',')
                    .unwrap_or(body.len());
                p.duration = body[..dur_end].trim().parse::<f64>().ok();
                let (attrs, name) = parse_attrs(&body[dur_end..]);
                p.attrs = attrs;
                p.name = name.unwrap_or_default();
                p.has_info = true;
            } else if upper.starts_with("EXTGRP:") {
                let g = rest[7..].trim();
                if !g.is_empty() {
                    p.group = Some(g.to_string());
                }
            } else if upper.starts_with("EXTVLCOPT:") || upper.starts_with("KODIPROP:") {
                let body = &rest[rest.find(':').unwrap() + 1..];
                if let Some((k, v)) = body.split_once('=') {
                    p.opts.push((k.to_string(), v.to_string()));
                }
            } else if upper.starts_with("EXTHTTP:") {
                p.opts
                    .push(("exthttp".into(), rest[8..].trim().to_string()));
            }
            continue;
        }
        if let Some(e) = finish(std::mem::take(&mut p), line) {
            out.entries.push(e);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn header_and_basic_entries() {
        let src = "\u{feff}#EXTM3U url-tvg=\"http://a/epg.xml.gz,http://b/epg.xml\" x-tvg-url='http://a/epg.xml.gz'\r\n#EXTINF:-1 tvg-id=\"news.br\" tvg-name=\"News\" tvg-logo=\"http://l/n.png\" group-title=\"Notícias\",News HD\r\nhttp://s/news.m3u8\r\n";
        let pl = parse(src);
        assert_eq!(pl.epg_urls, vec!["http://a/epg.xml.gz", "http://b/epg.xml"]);
        assert_eq!(pl.entries.len(), 1);
        let e = &pl.entries[0];
        assert_eq!(e.name, "News HD");
        assert_eq!(e.url, "http://s/news.m3u8");
        assert_eq!(e.tvg_id.as_deref(), Some("news.br"));
        assert_eq!(e.tvg_logo.as_deref(), Some("http://l/n.png"));
        assert_eq!(e.group.as_deref(), Some("Notícias"));
        assert_eq!(e.duration, None);
    }

    #[test]
    fn unquoted_single_quoted_and_comma_in_title() {
        let src = "#EXTM3U\n#EXTINF:0 tvg-id=abc group-title='Filmes, Séries' tvg-name=\"A, B\",Title, with comma\nhttp://x/1.ts\n#EXTINF:120.5,Plain\nhttp://x/2.mp4";
        let pl = parse(src);
        let e = &pl.entries[0];
        assert_eq!(e.tvg_id.as_deref(), Some("abc"));
        assert_eq!(e.group.as_deref(), Some("Filmes, Séries"));
        assert_eq!(e.tvg_name.as_deref(), Some("A, B"));
        assert_eq!(e.name, "Title, with comma");
        assert_eq!(pl.entries[1].duration, Some(120.5));
        assert_eq!(pl.entries[1].name, "Plain");
    }

    #[test]
    fn options_before_and_after_extinf() {
        let src = "#EXTM3U\n#EXTVLCOPT:http-user-agent=VLC/3.0\n#EXTINF:-1,Before\n#EXTVLCOPT:http-referrer=http://ref/\nhttp://x/a.m3u8\n#EXTINF:-1,After\n#EXTGRP:Esportes\n#KODIPROP:inputstream.adaptive.stream_headers=User-Agent=Kodi&Origin=http%3A%2F%2Fo\nhttp://x/b.m3u8\n#EXTINF:-1,Third\nhttp://x/c.m3u8";
        let pl = parse(src);
        assert_eq!(pl.entries.len(), 3);
        let a = &pl.entries[0];
        assert_eq!(a.user_agent.as_deref(), Some("VLC/3.0"));
        assert_eq!(a.referrer.as_deref(), Some("http://ref/"));
        let b = &pl.entries[1];
        assert_eq!(b.group.as_deref(), Some("Esportes"));
        assert_eq!(b.user_agent.as_deref(), Some("Kodi"));
        assert_eq!(
            b.headers,
            vec![("Origin".to_string(), "http://o".to_string())]
        );
        let c = &pl.entries[2];
        assert_eq!(c.user_agent, None);
        assert_eq!(c.group, None);
    }

    #[test]
    fn pipe_headers() {
        let src = "#EXTINF:-1 user-agent=\"Attr\",Pipe\nhttp://x/a.m3u8|User-Agent=Mozilla%2F5.0&Referer=https://site/&X-Token=abc\n";
        let e = &parse(src).entries[0];
        assert_eq!(e.url, "http://x/a.m3u8");
        assert_eq!(e.user_agent.as_deref(), Some("Mozilla/5.0"));
        assert_eq!(e.referrer.as_deref(), Some("https://site/"));
        assert_eq!(e.headers, vec![("X-Token".to_string(), "abc".to_string())]);
    }

    #[test]
    fn exthttp_catchup_and_plain_urls() {
        let src = "#EXTM3U\n#EXTINF:-1 catchup=\"shift\" catchup-days=\"7\" catchup-source=\"?utc={utc}\",Cu\n#EXTHTTP:{\"User-Agent\":\"TiviMate\",\"Cookie\":\"a=b\"}\nhttp://x/c\nhttp://plain/stream/live.ts\n\n#EXTINF:-1,Orphan without url\n#EXTINF:-1,Real\nrtmp://x/y";
        let pl = parse(src);
        assert_eq!(pl.entries.len(), 3);
        let c = &pl.entries[0];
        assert_eq!(c.catchup.as_deref(), Some("shift"));
        assert_eq!(c.catchup_days, Some(7));
        assert_eq!(c.catchup_source.as_deref(), Some("?utc={utc}"));
        assert_eq!(c.user_agent.as_deref(), Some("TiviMate"));
        assert_eq!(c.headers, vec![("Cookie".to_string(), "a=b".to_string())]);
        assert_eq!(pl.entries[1].name, "live.ts");
        assert_eq!(pl.entries[2].name, "Real");
    }

    #[test]
    fn garbage_is_tolerated() {
        for src in [
            "",
            "#EXTM3U",
            "#EXTINF:",
            "#EXTINF:-1 tvg-id=\"unclosed,Name\nhttp://x",
            "#EXTINF:abc =,\n|",
            "#KODIPROP:\n#EXTVLCOPT:x\n",
        ] {
            let _ = parse(src);
        }
        let pl = parse("#EXTINF:-1 tvg-id=\"unclosed,Name\nhttp://x");
        assert_eq!(pl.entries[0].url, "http://x");
    }
}

use base64::Engine;
use serde::Serialize;
use url::Url;

use super::route::{Route, RouteKind, RouteSource};

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ParsedLink {
    Route { route: Route },
    Unsupported { scheme: String, name: String },
}

pub fn parse_many(text: &str, source: RouteSource) -> Vec<ParsedLink> {
    let mut out: Vec<ParsedLink> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for token in tokens(text) {
        if let Some(parsed) = parse_one(&token, source.clone()) {
            if let ParsedLink::Route { route } = &parsed {
                if !seen.insert(route.id.clone()) {
                    continue;
                }
            }
            out.push(parsed);
        }
    }
    out
}

fn tokens(text: &str) -> Vec<String> {
    text.split(|c: char| {
        c.is_whitespace()
            || matches!(
                c,
                '"' | '\''
                    | '<'
                    | '>'
                    | '，'
                    | '：'
                    | '《'
                    | '》'
                    | '。'
                    | '；'
                    | '、'
                    | '「'
                    | '」'
                    | '（'
                    | '）'
                    | '【'
                    | '】'
            )
    })
    .map(|t| t.trim_matches(|c: char| matches!(c, ',' | ';' | ')' | '(' | '[' | ']')))
    .filter(|t| !t.is_empty())
    .map(strip_before_scheme)
    .collect()
}

fn strip_before_scheme(t: &str) -> String {
    let Some(at) = t.find("://") else {
        return t.to_string();
    };
    let start = t[..at]
        .char_indices()
        .rev()
        .take_while(|(_, c)| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
        .last()
        .map(|(i, _)| i)
        .unwrap_or(at);
    t[start..].to_string()
}

pub fn parse_one(raw: &str, source: RouteSource) -> Option<ParsedLink> {
    let s = raw.trim();
    let lower = s.to_ascii_lowercase();
    let route = if lower.starts_with("tg://proxy") || is_tme(&lower, "proxy") {
        parse_tg_proxy(s)
    } else if lower.starts_with("tg://socks") || is_tme(&lower, "socks") {
        parse_tg_socks(s)
    } else if lower.starts_with("socks5://")
        || lower.starts_with("socks5h://")
        || lower.starts_with("socks://")
    {
        parse_userinfo_url(s, RouteKind::Socks5)
    } else if lower.starts_with("http://") || lower.starts_with("https://") {
        let u = Url::parse(s).ok()?;
        if u.port().is_none() || (u.path() != "/" && !u.path().is_empty()) || u.query().is_some() {
            return None;
        }
        parse_userinfo_url(s, RouteKind::Http)
    } else if lower.starts_with("ss://") {
        parse_ss(s)
    } else if lower.starts_with("trojan://") {
        parse_trojan(s)
    } else if let Some(scheme) = [
        "vmess",
        "vless",
        "hysteria2",
        "hy2",
        "hysteria",
        "tuic",
        "ssr",
        "wireguard",
        "anytls",
    ]
    .iter()
    .find(|p| lower.starts_with(&format!("{p}://")))
    {
        return Some(ParsedLink::Unsupported {
            scheme: scheme.to_string(),
            name: fragment_name(s).unwrap_or_default(),
        });
    } else {
        parse_bare(s)
    }?;
    let mut route = route;
    route.source = source;
    route.finish();
    Some(ParsedLink::Route { route })
}

fn is_tme(lower: &str, what: &str) -> bool {
    let rest = lower
        .trim_start_matches("https://")
        .trim_start_matches("http://")
        .trim_start_matches("www.");
    ["t.me/", "telegram.me/", "telegram.dog/"]
        .iter()
        .any(|d| rest.starts_with(&format!("{d}{what}?")))
}

fn query_map(s: &str) -> std::collections::HashMap<String, String> {
    let q = s.split_once('?').map(|(_, q)| q).unwrap_or("");
    let q = q.split('#').next().unwrap_or("");
    url::form_urlencoded::parse(q.as_bytes())
        .map(|(k, v)| (k.to_ascii_lowercase(), v.into_owned()))
        .collect()
}

fn parse_tg_proxy(s: &str) -> Option<Route> {
    let q = query_map(s);
    let host = q.get("server")?.trim().to_string();
    let port: u16 = q.get("port")?.trim().parse().ok()?;
    let secret = normalize_secret(q.get("secret")?)?;
    let mut r = Route::new(RouteKind::MtProxy, host, port, RouteSource::Link);
    r.secret = Some(secret);
    Some(r)
}

fn parse_tg_socks(s: &str) -> Option<Route> {
    let q = query_map(s);
    let host = q.get("server")?.trim().to_string();
    let port: u16 = q.get("port")?.trim().parse().ok()?;
    let mut r = Route::new(RouteKind::Socks5, host, port, RouteSource::Link);
    r.username = q.get("user").filter(|v| !v.is_empty()).cloned();
    r.password = q.get("pass").filter(|v| !v.is_empty()).cloned();
    Some(r)
}

fn parse_userinfo_url(s: &str, kind: RouteKind) -> Option<Route> {
    let u = Url::parse(s).ok()?;
    let host = u.host_str()?.trim_matches(['[', ']']).to_string();
    let port = u
        .port()
        .unwrap_or(if kind == RouteKind::Http { 8080 } else { 1080 });
    let mut r = Route::new(kind, host, port, RouteSource::Link);
    if !u.username().is_empty() {
        r.username = Some(urlencoding::decode(u.username()).ok()?.into_owned());
    }
    if let Some(p) = u.password() {
        r.password = Some(urlencoding::decode(p).ok()?.into_owned());
    }
    if let Some(name) = fragment_name(s) {
        r.name = name;
    }
    Some(r)
}

fn parse_ss(s: &str) -> Option<Route> {
    let body = &s[5..];
    let (body, name) = match body.split_once('#') {
        Some((b, n)) => (b, urlencoding::decode(n).ok().map(|c| c.into_owned())),
        None => (body, None),
    };
    let body = body.split('?').next().unwrap_or(body).trim_end_matches('/');
    let (userinfo, hostport) = match body.rsplit_once('@') {
        Some((u, h)) => {
            let u = urlencoding::decode(u).ok()?.into_owned();
            let u = if u.contains(':') {
                u
            } else {
                String::from_utf8(b64_any(&u)?).ok()?
            };
            (u, h.to_string())
        }
        None => {
            let decoded = String::from_utf8(b64_any(body)?).ok()?;
            let (u, h) = decoded.rsplit_once('@')?;
            (u.to_string(), h.to_string())
        }
    };
    let (method, password) = userinfo.split_once(':')?;
    let (host, port) = split_host_port(&hostport)?;
    let mut r = Route::new(RouteKind::Shadowsocks, host, port, RouteSource::Link);
    r.cipher = Some(method.to_ascii_lowercase());
    r.password = Some(password.to_string());
    if let Some(n) = name.filter(|n| !n.is_empty()) {
        r.name = n;
    }
    Some(r)
}

fn parse_trojan(s: &str) -> Option<Route> {
    let u = Url::parse(s).ok()?;
    let host = u.host_str()?.trim_matches(['[', ']']).to_string();
    let port = u.port().unwrap_or(443);
    let q = query_map(s);
    if let Some(t) = q.get("type") {
        if !t.is_empty() && t != "tcp" {
            return None;
        }
    }
    let mut r = Route::new(RouteKind::Trojan, host.clone(), port, RouteSource::Link);
    r.password = Some(urlencoding::decode(u.username()).ok()?.into_owned());
    r.sni = q
        .get("sni")
        .or_else(|| q.get("peer"))
        .filter(|v| !v.is_empty())
        .cloned()
        .or(Some(host));
    r.insecure = matches!(
        q.get("allowinsecure").map(String::as_str),
        Some("1") | Some("true")
    );
    if let Some(n) = fragment_name(s) {
        r.name = n;
    }
    Some(r)
}

fn parse_bare(s: &str) -> Option<Route> {
    let parts: Vec<&str> = s.split(':').collect();
    match parts.as_slice() {
        [host, port, secret] if looks_like_host(host) => {
            let port: u16 = port.parse().ok()?;
            let secret = normalize_secret(secret)?;
            let mut r = Route::new(RouteKind::MtProxy, *host, port, RouteSource::Link);
            r.secret = Some(secret);
            Some(r)
        }
        [host, port] if looks_like_host(host) => {
            let port: u16 = port.parse().ok()?;
            Some(Route::new(
                RouteKind::Socks5,
                *host,
                port,
                RouteSource::Link,
            ))
        }
        _ => None,
    }
}

fn looks_like_host(h: &str) -> bool {
    !h.is_empty()
        && h.len() <= 253
        && h.contains('.')
        && h.chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-')
}

fn split_host_port(s: &str) -> Option<(String, u16)> {
    if let Some(rest) = s.strip_prefix('[') {
        let (h, p) = rest.split_once("]:")?;
        return Some((h.to_string(), p.parse().ok()?));
    }
    let (h, p) = s.rsplit_once(':')?;
    Some((h.to_string(), p.trim_end_matches('/').parse().ok()?))
}

fn fragment_name(s: &str) -> Option<String> {
    let (_, frag) = s.split_once('#')?;
    let n = urlencoding::decode(frag).ok()?.into_owned();
    (!n.is_empty()).then_some(n)
}

fn b64_any(s: &str) -> Option<Vec<u8>> {
    let t = s.trim().trim_end_matches('=');
    let engines = [
        base64::engine::general_purpose::STANDARD_NO_PAD,
        base64::engine::general_purpose::URL_SAFE_NO_PAD,
    ];
    engines.iter().find_map(|e| e.decode(t).ok())
}

pub fn normalize_secret(raw: &str) -> Option<String> {
    let t = raw.trim();
    if t.len() >= 32 && t.len().is_multiple_of(2) && t.chars().all(|c| c.is_ascii_hexdigit()) {
        return Some(t.to_ascii_lowercase());
    }
    let bytes = b64_any(t)?;
    (bytes.len() >= 16).then(|| hex::encode(bytes))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SecretMode {
    Plain([u8; 16]),
    Padded([u8; 16]),
    FakeTls { key: [u8; 16], domain: String },
}

pub fn secret_mode(hex_secret: &str) -> Option<SecretMode> {
    let bytes = hex::decode(hex_secret).ok()?;
    let key_at = |off: usize| -> Option<[u8; 16]> { bytes.get(off..off + 16)?.try_into().ok() };
    match bytes.first()? {
        0xee if bytes.len() > 17 => Some(SecretMode::FakeTls {
            key: key_at(1)?,
            domain: String::from_utf8(bytes[17..].to_vec()).ok()?,
        }),
        0xdd if bytes.len() >= 17 => Some(SecretMode::Padded(key_at(1)?)),
        _ if bytes.len() == 16 => Some(SecretMode::Plain(key_at(0)?)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn one(s: &str) -> Route {
        match parse_one(s, RouteSource::Link) {
            Some(ParsedLink::Route { route }) => route,
            other => panic!("{s} -> {other:?}"),
        }
    }

    #[test]
    fn tg_proxy_hex_and_base64() {
        let r = one("tg://proxy?server=1.2.3.4&port=443&secret=ee0123456789abcdef0123456789abcdef7777772e636c6f7564666c6172652e636f6d");
        assert_eq!(r.kind, RouteKind::MtProxy);
        assert!(matches!(
            secret_mode(r.secret.as_deref().unwrap()),
            Some(SecretMode::FakeTls { ref domain, .. }) if domain == "www.cloudflare.com"
        ));
        let r = one("https://t.me/proxy?server=p.example.com&port=8443&secret=7gEjRWeJq83vASNFZ4mrze93d3cuYmluZy5jb20");
        assert_eq!(r.port, 8443);
        assert!(r.secret.unwrap().starts_with("ee"));
    }

    #[test]
    fn socks_and_http() {
        let r = one("tg://socks?server=10.0.0.1&port=1080&user=a&pass=b");
        assert_eq!(
            (r.kind, r.username.as_deref()),
            (RouteKind::Socks5, Some("a"))
        );
        let r = one("socks5://127.0.0.1:7890");
        assert_eq!(r.port, 7890);
        let r = one("http://u:p%40ss@127.0.0.1:7890");
        assert_eq!(r.password.as_deref(), Some("p@ss"));
        assert!(parse_one("https://example.com/page", RouteSource::Link).is_none());
    }

    #[test]
    fn shadowsocks_forms() {
        let r = one("ss://YWVzLTI1Ni1nY206cGFzcw@1.2.3.4:8388#HK%2001");
        assert_eq!(r.cipher.as_deref(), Some("aes-256-gcm"));
        assert_eq!(r.password.as_deref(), Some("pass"));
        assert_eq!(r.name, "HK 01");
        let r = one("ss://YWVzLTEyOC1nY206cGFzc0AxLjIuMy40Ojg0NDM#x");
        assert_eq!((r.host.as_str(), r.port), ("1.2.3.4", 8443));
        let r = one("ss://2022-blake3-aes-128-gcm:a2V5@h.example:9000");
        assert_eq!(r.cipher.as_deref(), Some("2022-blake3-aes-128-gcm"));
    }

    #[test]
    fn trojan_and_unsupported() {
        let r = one("trojan://pw@t.example.com:443?sni=cdn.example.com&allowInsecure=1#JP");
        assert_eq!(r.sni.as_deref(), Some("cdn.example.com"));
        assert!(r.insecure);
        assert!(matches!(
            parse_one("vmess://eyJhZGQiOiJ4In0=", RouteSource::Link),
            Some(ParsedLink::Unsupported { .. })
        ));
    }

    #[test]
    fn pasted_block_with_chinese_punctuation() {
        let text = "代理：tg://proxy?server=a.example.com&port=443&secret=dd00112233445566778899aabbccddeeff，备用 b.example.com:8443:00112233445566778899aabbccddeeff";
        let all = parse_many(text, RouteSource::Link);
        assert_eq!(all.len(), 2);
    }
}

use std::time::Duration;

use base64::Engine;
use serde::Serialize;

use super::links::{self, ParsedLink};
use super::route::{Route, RouteKind, RouteSource};

#[derive(Debug, Clone, Serialize)]
pub struct Imported {
    pub routes: Vec<Route>,
    pub unsupported: Vec<(String, usize)>,
}

pub async fn fetch(url: &str, via: Option<&Route>) -> anyhow::Result<Imported> {
    let body = match get(url, None).await {
        Ok(b) => b,
        Err(first) => match via {
            Some(r) => get(url, Some(r)).await.map_err(|_| first)?,
            None => return Err(first),
        },
    };
    let source = RouteSource::Subscription {
        url: url.to_string(),
    };
    let imported = parse(&body, source);
    anyhow::ensure!(
        !imported.routes.is_empty() || !imported.unsupported.is_empty(),
        "the subscription did not contain any server"
    );
    Ok(imported)
}

async fn get(url: &str, via: Option<&Route>) -> anyhow::Result<String> {
    let mut b = reqwest::Client::builder()
        .use_rustls_tls()
        .timeout(Duration::from_secs(20))
        .user_agent("clash.meta");
    b = match via {
        Some(r) if matches!(r.kind, RouteKind::Socks5 | RouteKind::Http) => {
            let scheme = if r.kind == RouteKind::Socks5 {
                "socks5h"
            } else {
                "http"
            };
            b.proxy(reqwest::Proxy::all(format!(
                "{scheme}://{}:{}",
                r.host, r.port
            ))?)
        }
        _ => b.no_proxy(),
    };
    let resp = b.build()?.get(url).send().await?.error_for_status()?;
    Ok(resp.text().await?)
}

pub fn parse(body: &str, source: RouteSource) -> Imported {
    let text = body.trim();
    let mut unsupported: std::collections::BTreeMap<String, usize> = Default::default();
    let mut routes: Vec<Route> = Vec::new();

    let mut push = |parsed: Vec<ParsedLink>| {
        for p in parsed {
            match p {
                ParsedLink::Route { mut route } => {
                    route.source = source.clone();
                    route.finish();
                    if !routes.iter().any(|r| r.id == route.id) {
                        routes.push(route);
                    }
                }
                ParsedLink::Unsupported { scheme, .. } => {
                    *unsupported.entry(scheme).or_default() += 1
                }
            }
        }
    };

    if text.starts_with('{') {
        push(from_sip008(text));
    } else if text.contains("proxies:") {
        let (ok, skipped) = from_clash_yaml(text);
        push(ok);
        for (k, v) in skipped {
            *unsupported.entry(k).or_default() += v;
        }
    } else {
        let decoded = base64_text(text).unwrap_or_else(|| text.to_string());
        push(links::parse_many(&decoded, source.clone()));
    }
    Imported {
        routes,
        unsupported: unsupported.into_iter().collect(),
    }
}

fn base64_text(s: &str) -> Option<String> {
    let compact: String = s.chars().filter(|c| !c.is_whitespace()).collect();
    if compact.contains("://") {
        return None;
    }
    let t = compact.trim_end_matches('=');
    [
        base64::engine::general_purpose::STANDARD_NO_PAD,
        base64::engine::general_purpose::URL_SAFE_NO_PAD,
    ]
    .iter()
    .find_map(|e| e.decode(t).ok())
    .and_then(|b| String::from_utf8(b).ok())
}

fn from_sip008(text: &str) -> Vec<ParsedLink> {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(text) else {
        return Vec::new();
    };
    let servers = v
        .get("servers")
        .and_then(|s| s.as_array())
        .cloned()
        .unwrap_or_default();
    servers
        .iter()
        .filter_map(|s| {
            let host = s.get("server")?.as_str()?.to_string();
            let port = s.get("server_port")?.as_u64()? as u16;
            let mut r = Route::new(RouteKind::Shadowsocks, host, port, RouteSource::Link);
            r.cipher = s.get("method").and_then(|m| m.as_str()).map(str::to_string);
            r.password = s
                .get("password")
                .and_then(|m| m.as_str())
                .map(str::to_string);
            if let Some(n) = s.get("remarks").and_then(|m| m.as_str()) {
                r.name = n.to_string();
            }
            r.finish();
            Some(ParsedLink::Route { route: r })
        })
        .collect()
}

fn from_clash_yaml(text: &str) -> (Vec<ParsedLink>, Vec<(String, usize)>) {
    let Ok(doc) = serde_yaml::from_str::<serde_yaml::Value>(text) else {
        return (Vec::new(), Vec::new());
    };
    let proxies = doc
        .get("proxies")
        .and_then(|p| p.as_sequence())
        .cloned()
        .unwrap_or_default();
    let mut ok = Vec::new();
    let mut skipped: std::collections::BTreeMap<String, usize> = Default::default();
    for p in proxies {
        let get = |k: &str| p.get(k).and_then(|v| v.as_str()).map(str::to_string);
        let port = p
            .get("port")
            .and_then(|v| {
                v.as_u64()
                    .or_else(|| v.as_str().and_then(|s| s.parse().ok()))
            })
            .unwrap_or(0) as u16;
        let (Some(kind), Some(host)) = (get("type"), get("server")) else {
            continue;
        };
        let name = get("name").unwrap_or_default();
        let route = match kind.as_str() {
            "ss" => {
                let cipher = get("cipher").unwrap_or_default();
                if p.get("plugin").is_some() || !super::shadowsocks::supported(&cipher) {
                    *skipped.entry("ss+plugin".into()).or_default() += 1;
                    continue;
                }
                let mut r = Route::new(RouteKind::Shadowsocks, host, port, RouteSource::Link);
                r.cipher = Some(cipher);
                r.password = get("password");
                r
            }
            "trojan" => {
                let network = get("network").unwrap_or_else(|| "tcp".into());
                if network != "tcp" {
                    *skipped.entry(format!("trojan+{network}")).or_default() += 1;
                    continue;
                }
                let mut r = Route::new(RouteKind::Trojan, host.clone(), port, RouteSource::Link);
                r.password = get("password");
                r.sni = get("sni").or(Some(host));
                r.insecure = p
                    .get("skip-cert-verify")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                r
            }
            "socks5" => {
                let mut r = Route::new(RouteKind::Socks5, host, port, RouteSource::Link);
                r.username = get("username");
                r.password = get("password");
                r
            }
            "http" => {
                let mut r = Route::new(RouteKind::Http, host, port, RouteSource::Link);
                r.username = get("username");
                r.password = get("password");
                r
            }
            other => {
                *skipped.entry(other.to_string()).or_default() += 1;
                continue;
            }
        };
        let mut route = route;
        if !name.is_empty() {
            route.name = name;
        }
        route.finish();
        ok.push(ParsedLink::Route { route });
    }
    (ok, skipped.into_iter().collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clash_yaml() {
        let y = r#"
proxies:
  - {name: "香港 01", type: ss, server: hk.example.com, port: 443, cipher: aes-128-gcm, password: pw}
  - {name: JP, type: trojan, server: jp.example.com, port: 443, password: pw, sni: jp.example.com}
  - {name: US, type: vmess, server: us.example.com, port: 443, uuid: x}
  - {name: SG, type: ss, server: sg.example.com, port: 443, cipher: aes-128-gcm, password: pw, plugin: obfs}
"#;
        let i = parse(y, RouteSource::Manual);
        assert_eq!(i.routes.len(), 2);
        assert_eq!(i.routes[0].name, "香港 01");
        assert_eq!(i.unsupported.len(), 2);
    }

    #[test]
    fn base64_list() {
        let list = "ss://YWVzLTI1Ni1nY206cGFzcw@1.2.3.4:8388#A\nvmess://abc\ntrojan://pw@h.example.com:443#B\n";
        let b = base64::engine::general_purpose::STANDARD.encode(list);
        let i = parse(&b, RouteSource::Manual);
        assert_eq!(i.routes.len(), 2);
        assert_eq!(i.unsupported, vec![("vmess".to_string(), 1)]);
    }
}

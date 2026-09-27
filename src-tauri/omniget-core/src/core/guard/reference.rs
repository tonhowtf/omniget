//! Reference validator: URLs and links. No network access, ever.
//!
//! Improvements over the original: loopback in an MCP config is normal and
//! not flagged; XML/JSON-schema namespace URLs (`http://www.w3.org/…`) are
//! not "insecure HTTP"; cloud metadata endpoints, paste sites / request
//! catchers, shorteners and raw public IPs are called out.

use once_cell::sync::Lazy;
use regex::Regex;
use std::collections::BTreeSet;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

use super::model::{Finding, Sink, Validator};
use super::rules::sev;
use super::shell::WEBHOOK_CATCHER;
use super::text::{CodeCtx, CodeMap, LineIndex};

static PLAIN_URL: Lazy<Regex> =
    Lazy::new(|| Regex::new(r#"(?i)\b(https?|ftp|file|sftp|smb)://[^\s<>"'`)\]\}|\\]+"#).unwrap());
static MD_LINK: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(!?)\[([^\]\n]*)\]\(\s*([^)\s]+)").unwrap());

const NAMESPACE_HOSTS: &[&str] = &[
    "www.w3.org",
    "w3.org",
    "json-schema.org",
    "schemas.xmlsoap.org",
    "schemas.microsoft.com",
    "purl.org",
    "xmlns.com",
    "schemas.android.com",
    "ns.adobe.com",
    "www.apache.org",
    "maven.apache.org",
    "java.sun.com",
    "xml.apache.org",
    "schemas.openxmlformats.org",
    "docs.oasis-open.org",
    "www.springframework.org",
];
const PLACEHOLDER_HOSTS: &[&str] = &[
    "example.com",
    "example.org",
    "example.net",
    "your-domain.com",
    "yourdomain.com",
    "domain.com",
];
const SHORTENERS: &[&str] = &[
    "bit.ly",
    "tinyurl.com",
    "t.co",
    "goo.gl",
    "is.gd",
    "ow.ly",
    "cutt.ly",
    "rb.gy",
    "shorturl.at",
    "buff.ly",
    "tiny.cc",
    "rebrand.ly",
];
const SUSPICIOUS_TLDS: &[&str] = &[
    ".tk", ".ml", ".ga", ".cf", ".gq", ".zip", ".mov", ".top", ".click", ".country", ".kim",
    ".work", ".xyz",
];

/// Host (lower-case, no port/userinfo/brackets) of a URL.
pub fn host_of(url: &str) -> Option<String> {
    let rest = url.split_once("://")?.1;
    let auth = rest.split(['/', '?', '#']).next().unwrap_or("");
    let auth = auth.rsplit('@').next().unwrap_or(auth);
    let host = if auth.starts_with('[') {
        auth.trim_start_matches('[')
            .split(']')
            .next()
            .unwrap_or("")
            .to_string()
    } else {
        auth.split(':').next().unwrap_or("").to_string()
    };
    (!host.is_empty()).then(|| host.to_lowercase())
}

pub enum HostClass {
    Loopback,
    Private,
    Metadata,
    PublicIp,
    Name,
}

pub fn classify(host: &str) -> HostClass {
    if host == "localhost" || host.ends_with(".localhost") || host == "0.0.0.0" {
        return HostClass::Loopback;
    }
    if matches!(
        host,
        "metadata.google.internal"
            | "metadata"
            | "instance-data"
            | "169.254.169.254"
            | "100.100.100.200"
            | "fd00:ec2::254"
    ) {
        return HostClass::Metadata;
    }
    if let Ok(ip) = host.parse::<IpAddr>() {
        return match ip {
            IpAddr::V4(v4) => classify_v4(v4),
            IpAddr::V6(v6) => classify_v6(v6),
        };
    }
    if host.ends_with(".local")
        || host.ends_with(".internal")
        || host.ends_with(".lan")
        || host.ends_with(".corp")
    {
        return HostClass::Private;
    }
    HostClass::Name
}

fn classify_v4(ip: Ipv4Addr) -> HostClass {
    if ip.is_loopback() || ip.is_unspecified() {
        HostClass::Loopback
    } else if ip.octets()[0] == 169 && ip.octets()[1] == 254 {
        if ip == Ipv4Addr::new(169, 254, 169, 254) {
            HostClass::Metadata
        } else {
            HostClass::Private
        }
    } else if ip.is_private() || (ip.octets()[0] == 100 && (64..128).contains(&ip.octets()[1])) {
        HostClass::Private
    } else {
        HostClass::PublicIp
    }
}

fn classify_v6(ip: Ipv6Addr) -> HostClass {
    if ip.is_loopback() || ip.is_unspecified() {
        return HostClass::Loopback;
    }
    let s = ip.segments()[0];
    if (s & 0xfe00) == 0xfc00 || (s & 0xffc0) == 0xfe80 {
        HostClass::Private
    } else {
        HostClass::PublicIp
    }
}

/// Check every URL of `text`. `config` = a JSON config (loopback is normal).
pub fn check(file: &str, text: &str, config: bool, markdown: bool, sink: &mut Sink) {
    let idx = LineIndex::new(text);
    let code = markdown.then(|| CodeMap::new(text));
    let mut seen: BTreeSet<(String, String)> = BTreeSet::new();
    let emit = |sink: &mut Sink,
                code_id: &str,
                url: &str,
                start: usize,
                drop: u8,
                seen: &mut BTreeSet<(String, String)>| {
        if !seen.insert((code_id.to_string(), url.to_string())) || seen.len() > 60 {
            return;
        }
        let (line, col) = idx.pos(text, start);
        sink.push(
            Finding::new(
                code_id,
                Validator::Reference,
                sev(code_id),
                file,
                super::text::clip(url, 160),
            )
            .at(line, col)
            .snippet(idx.line_at(text, start))
            .lowered(drop),
        );
    };

    if markdown {
        for c in MD_LINK.captures_iter(text) {
            let target = c.get(3).unwrap();
            let t = target.as_str().to_lowercase();
            let image = &c[1] == "!";
            let in_code = code
                .as_ref()
                .is_some_and(|m| m.ctx(target.start()) != CodeCtx::Prose);
            if ["javascript:", "vbscript:", "data:text/html", "file:"]
                .iter()
                .any(|s| t.starts_with(s))
            {
                emit(
                    sink,
                    "REF_E005",
                    target.as_str(),
                    target.start(),
                    if in_code { 2 } else { 0 },
                    &mut seen,
                );
            }
            if image && t.starts_with("data:") && target.as_str().len() > 10_000 {
                emit(
                    sink,
                    "REF_W006",
                    &format!("data: {} KB", target.as_str().len() / 1024),
                    target.start(),
                    0,
                    &mut seen,
                );
            }
        }
    }

    for m in PLAIN_URL.find_iter(text) {
        let url = m.as_str().trim_end_matches(['.', ',', ';', ':', '*', '_']);
        let lower = url.to_lowercase();
        let in_code = code
            .as_ref()
            .is_some_and(|c| c.ctx(m.start()) != CodeCtx::Prose);
        let drop = if in_code { 1 } else { 0 };
        let scheme = lower.split("://").next().unwrap_or("");
        if matches!(scheme, "ftp" | "file" | "smb") {
            // ftp is only unencrypted; file/smb reach local or LAN files.
            let extra = if scheme == "ftp" { 1 } else { 0 };
            emit(
                sink,
                "REF_E002",
                url,
                m.start(),
                (drop + extra).min(2),
                &mut seen,
            );
            continue;
        }
        let Some(host) = host_of(url) else { continue };
        if host.contains('{')
            || host.contains('$')
            || host.contains('<')
            || PLACEHOLDER_HOSTS
                .iter()
                .any(|h| host == *h || host.ends_with(&format!(".{h}")))
        {
            continue;
        }
        match classify(&host) {
            // In documentation it is usually a pentest example; in a config or
            // script it will be requested.
            HostClass::Metadata => emit(
                sink,
                "REF_E006",
                url,
                m.start(),
                if markdown { 1 + drop } else { 0 },
                &mut seen,
            ),
            HostClass::Private => emit(sink, "REF_E004", url, m.start(), drop, &mut seen),
            HostClass::Loopback => {
                if !config {
                    emit(sink, "REF_W003", url, m.start(), drop, &mut seen)
                }
            }
            HostClass::PublicIp => emit(sink, "REF_W009", url, m.start(), drop, &mut seen),
            HostClass::Name => {
                if scheme == "http" && !NAMESPACE_HOSTS.contains(&host.as_str()) {
                    emit(sink, "REF_W002", url, m.start(), 0, &mut seen);
                }
                if SUSPICIOUS_TLDS.iter().any(|t| host.ends_with(t)) {
                    emit(sink, "REF_W004", url, m.start(), 0, &mut seen);
                }
                if SHORTENERS.contains(&host.as_str()) {
                    emit(sink, "REF_W008", url, m.start(), 0, &mut seen);
                }
                if WEBHOOK_CATCHER.is_match(&lower) {
                    emit(sink, "REF_W007", url, m.start(), 0, &mut seen);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hosts() {
        assert_eq!(
            host_of("https://user:pw@Example.COM:8080/x").as_deref(),
            Some("example.com")
        );
        assert!(matches!(classify("10.0.0.1"), HostClass::Private));
        assert!(matches!(classify("169.254.169.254"), HostClass::Metadata));
        assert!(matches!(classify("::1"), HostClass::Loopback));
    }

    #[test]
    fn flags() {
        let mut s = Sink::default();
        check(
            "a.md",
            "see http://10.0.0.5/admin and [x](javascript:alert(1)) http://www.w3.org/2000/svg",
            false,
            true,
            &mut s,
        );
        let codes: Vec<_> = s.findings.iter().map(|f| f.code.as_str()).collect();
        assert!(codes.contains(&"REF_E004"));
        assert!(codes.contains(&"REF_E005"));
        assert!(!codes.contains(&"REF_W002"));
    }
}

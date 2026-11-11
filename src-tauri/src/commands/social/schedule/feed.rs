use std::net::IpAddr;
use std::time::Duration;

use super::rules::{Entry, Reject};

const MAX_BYTES: usize = 5 * 1024 * 1024;

fn text(n: roxmltree::Node, name: &str) -> Option<String> {
    n.children()
        .find(|c| c.is_element() && c.tag_name().name() == name)
        .and_then(|c| {
            let t: String = c
                .descendants()
                .filter(|d| d.is_text())
                .filter_map(|d| d.text())
                .collect();
            let t = t.split_whitespace().collect::<Vec<_>>().join(" ");
            (!t.is_empty()).then_some(t)
        })
}

fn atom_link(n: roxmltree::Node) -> Option<String> {
    let links: Vec<roxmltree::Node> = n
        .children()
        .filter(|c| c.is_element() && c.tag_name().name() == "link")
        .collect();
    links
        .iter()
        .find(|l| matches!(l.attribute("rel"), None | Some("alternate")))
        .or(links.first())
        .and_then(|l| l.attribute("href"))
        .map(|h| h.trim().to_string())
        .filter(|h| !h.is_empty())
}

pub fn parse(xml: &str) -> Result<Vec<Entry>, String> {
    let options = roxmltree::ParsingOptions {
        allow_dtd: true,
        ..Default::default()
    };
    let doc = roxmltree::Document::parse_with_options(
        xml.trim_start_matches('\u{feff}').trim_start(),
        options,
    )
    .map_err(|_| changed("the address did not return an RSS or Atom document"))?;
    let root = doc.root_element();
    let atom = root.tag_name().name() == "feed";
    if !atom && !matches!(root.tag_name().name(), "rss" | "RDF") {
        return Err(changed(
            "the address did not return an RSS or Atom document",
        ));
    }
    let wanted = if atom { "entry" } else { "item" };
    let mut out = Vec::new();
    for n in root
        .descendants()
        .filter(|n| n.is_element() && n.tag_name().name() == wanted)
    {
        let link = if atom { atom_link(n) } else { text(n, "link") };
        let id = text(n, if atom { "id" } else { "guid" }).or_else(|| link.clone());
        let (Some(id), Some(link)) = (
            id,
            link.filter(|l| l.starts_with("http://") || l.starts_with("https://")),
        ) else {
            continue;
        };
        out.push(Entry {
            id,
            title: text(n, "title").unwrap_or_default(),
            link,
        });
    }
    Ok(out)
}

fn changed(message: &str) -> String {
    format!(
        "FEED_INVALID: {}",
        serde_json::json!({ "message": message, "retryable": false, "next_step": "open the address in a browser and pass the link of the RSS or Atom feed itself" })
    )
}

pub fn check_url(raw: &str) -> Result<String, Reject> {
    let bad = |why: &str| {
        Reject::new(
            "INVALID_URL",
            format!("{raw}: {why}"),
            "pass the public http or https address of an RSS or Atom feed",
        )
    };
    let u = url::Url::parse(raw.trim()).map_err(|_| bad("not an address"))?;
    if !matches!(u.scheme(), "http" | "https") {
        return Err(bad("only http and https feeds are read"));
    }
    let host = u
        .host_str()
        .ok_or_else(|| bad("no host"))?
        .to_ascii_lowercase();
    let local_name =
        host == "localhost" || host.ends_with(".localhost") || host.ends_with(".local");
    let local_ip = host
        .trim_matches(['[', ']'])
        .parse::<IpAddr>()
        .is_ok_and(|ip| match ip {
            IpAddr::V4(v) => {
                v.is_loopback() || v.is_private() || v.is_link_local() || v.is_unspecified()
            }
            IpAddr::V6(v) => {
                v.is_loopback()
                    || v.is_unspecified()
                    || (v.segments()[0] & 0xfe00) == 0xfc00
                    || (v.segments()[0] & 0xffc0) == 0xfe80
            }
        });
    if local_name || local_ip {
        return Err(bad(
            "addresses on this computer or the local network are not read",
        ));
    }
    Ok(u.to_string())
}

pub async fn fetch(url: &str) -> Result<Vec<Entry>, String> {
    let url = check_url(url).map_err(|r| r.wire())?;
    let unreachable = |e: String| {
        format!(
            "FEED_UNREACHABLE: {}",
            serde_json::json!({ "message": e, "retryable": true, "next_step": "the feed is checked again at its next interval" })
        )
    };
    let client = omniget_core::core::http_client::apply_global_proxy(
        reqwest::Client::builder()
            .use_rustls_tls()
            .timeout(Duration::from_secs(20))
            .redirect(reqwest::redirect::Policy::limited(5))
            .user_agent("Mozilla/5.0 (compatible; feed reader)"),
    )
    .build()
    .map_err(|e| unreachable(e.to_string()))?;
    let reply = client
        .get(&url)
        .header(
            "accept",
            "application/rss+xml, application/atom+xml, application/xml, text/xml, */*",
        )
        .send()
        .await
        .map_err(|e| unreachable(e.to_string()))?;
    if !reply.status().is_success() {
        return Err(unreachable(format!(
            "the feed answered {}",
            reply.status().as_u16()
        )));
    }
    let bytes = reply
        .bytes()
        .await
        .map_err(|e| unreachable(e.to_string()))?;
    if bytes.len() > MAX_BYTES {
        return Err(changed("the feed is larger than 5 MB"));
    }
    parse(&String::from_utf8_lossy(&bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_rss() {
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<rss version="2.0" xmlns:content="http://purl.org/rss/1.0/modules/content/"><channel><title>Blog</title><link>https://b.log</link>
<item><title><![CDATA[Second &  post]]></title><link>https://b.log/2</link><guid isPermaLink="false">id-2</guid><content:encoded>x</content:encoded></item>
<item><title>First
 post</title><link> https://b.log/1 </link></item>
<item><title>No link</title></item>
</channel></rss>"#;
        let e = parse(xml).unwrap();
        assert_eq!(e.len(), 2);
        assert_eq!(
            (e[0].id.as_str(), e[0].title.as_str(), e[0].link.as_str()),
            ("id-2", "Second & post", "https://b.log/2")
        );
        assert_eq!(
            (e[1].id.as_str(), e[1].title.as_str()),
            ("https://b.log/1", "First post")
        );
    }

    #[test]
    fn parses_atom() {
        let xml = r#"<feed xmlns="http://www.w3.org/2005/Atom"><title>Blog</title><link href="https://b.log"/>
<entry><id>tag:b.log,2026:2</id><title type="html">Two</title><link rel="self" href="https://b.log/2.atom"/><link rel="alternate" href="https://b.log/2"/></entry>
<entry><id>tag:b.log,2026:1</id><title>One</title><link href="https://b.log/1"/></entry></feed>"#;
        let e = parse(xml).unwrap();
        assert_eq!(e.len(), 2);
        assert_eq!(
            (e[0].id.as_str(), e[0].link.as_str()),
            ("tag:b.log,2026:2", "https://b.log/2")
        );
        assert_eq!(e[1].link, "https://b.log/1");
    }

    #[test]
    fn parses_rdf() {
        let xml = r#"<rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#" xmlns="http://purl.org/rss/1.0/"><channel><title>B</title></channel><item><title>One</title><link>https://b.log/1</link></item></rdf:RDF>"#;
        assert_eq!(parse(xml).unwrap()[0].link, "https://b.log/1");
    }

    #[test]
    fn rejects_what_is_not_a_feed() {
        assert!(parse("<html><body>hi</body></html>")
            .unwrap_err()
            .starts_with("FEED_INVALID"));
        assert!(parse("not xml").unwrap_err().starts_with("FEED_INVALID"));
    }

    #[test]
    fn refuses_local_addresses() {
        assert!(check_url("https://b.log/feed.xml").is_ok());
        for bad in [
            "file:///etc/passwd",
            "http://localhost:8080/feed",
            "http://127.0.0.1/feed",
            "http://192.168.0.1/rss",
            "http://[::1]/rss",
            "nope",
        ] {
            assert_eq!(check_url(bad).unwrap_err().code, "INVALID_URL", "{bad}");
        }
    }
}

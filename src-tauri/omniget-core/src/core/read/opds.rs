use serde::{Deserialize, Serialize};

use super::xml::{self, Node};

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct OpdsLink {
    pub href: String,
    pub rel: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub title: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct OpdsEntry {
    pub id: String,
    pub title: String,
    pub authors: Vec<String>,
    pub summary: String,
    pub updated: String,
    pub language: String,
    pub publisher: String,
    pub categories: Vec<String>,
    pub series: String,
    pub series_index: Option<f64>,
    pub cover: Option<String>,
    pub thumbnail: Option<String>,
    pub acquisitions: Vec<OpdsLink>,
    pub navigation: Option<String>,
    pub stream: Option<String>,
    pub stream_count: Option<u32>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct OpdsFeed {
    pub id: String,
    pub title: String,
    pub icon: Option<String>,
    pub entries: Vec<OpdsEntry>,
    pub next: Option<String>,
    pub prev: Option<String>,
    pub up: Option<String>,
    pub start: Option<String>,
    pub search: Option<String>,
    pub facets: Vec<OpdsLink>,
    pub total: Option<u64>,
}

fn abs(base: &str, href: &str) -> String {
    if href.is_empty() {
        return String::new();
    }
    url::Url::parse(base)
        .and_then(|b| b.join(href))
        .map(|u| u.to_string())
        .unwrap_or_else(|_| href.to_string())
}

fn link_of(n: &Node, base: &str) -> OpdsLink {
    OpdsLink {
        href: abs(base, n.attr("href").unwrap_or("")),
        rel: n.attr("rel").unwrap_or("").to_string(),
        kind: n.attr("type").unwrap_or("").to_string(),
        title: n.attr("title").unwrap_or("").to_string(),
    }
}

pub fn parse(body: &str, base: &str) -> Result<OpdsFeed, String> {
    let t = body.trim_start_matches('\u{feff}').trim_start();
    if t.starts_with('{') {
        parse_v2(t, base)
    } else {
        Ok(parse_v1(t, base))
    }
}

pub fn parse_v1(body: &str, base: &str) -> OpdsFeed {
    let root = xml::parse(body);
    let feed = root
        .find("feed")
        .cloned()
        .or_else(|| {
            root.find("entry").map(|e| Node {
                name: "feed".into(),
                children: vec![e.clone()],
                ..Default::default()
            })
        })
        .unwrap_or_default();
    let mut f = OpdsFeed {
        id: feed.child_text("id").unwrap_or_default(),
        title: feed.child_text("title").unwrap_or_default(),
        icon: feed.child_text("icon").map(|i| abs(base, &i)),
        total: feed.child_text("totalResults").and_then(|v| v.parse().ok()),
        ..Default::default()
    };
    for l in feed.children_named("link") {
        let link = link_of(l, base);
        match link.rel.as_str() {
            "next" => f.next = Some(link.href),
            "previous" | "prev" => f.prev = Some(link.href),
            "up" => f.up = Some(link.href),
            "start" => f.start = Some(link.href),
            "search" => {
                if f.search.is_none() || link.href.contains("{searchTerms}") {
                    f.search = Some(link.href)
                }
            }
            "http://opds-spec.org/facet" => f.facets.push(link),
            _ => {}
        }
    }
    for e in feed.children_named("entry") {
        let mut en = OpdsEntry {
            id: e.child_text("id").unwrap_or_default(),
            title: e.child_text("title").unwrap_or_default(),
            summary: e
                .child_text("summary")
                .or_else(|| e.child_text("content"))
                .unwrap_or_default(),
            updated: e
                .child_text("updated")
                .or_else(|| e.child_text("issued"))
                .unwrap_or_default(),
            language: e.child_text("language").unwrap_or_default(),
            publisher: e.child_text("publisher").unwrap_or_default(),
            ..Default::default()
        };
        en.authors = e
            .children_named("author")
            .filter_map(|a| a.child_text("name"))
            .collect();
        en.categories = e
            .children_named("category")
            .filter_map(|c| c.attr("label").or(c.attr("term")).map(str::to_string))
            .collect();
        for l in e.children_named("link") {
            let link = link_of(l, base);
            let rel = link.rel.as_str();
            if rel.starts_with("http://opds-spec.org/acquisition") {
                en.acquisitions.push(link);
            } else if rel == "http://opds-spec.org/image" || rel == "http://opds-spec.org/cover" {
                en.cover = Some(link.href);
            } else if rel == "http://opds-spec.org/image/thumbnail"
                || rel == "http://opds-spec.org/thumbnail"
            {
                en.thumbnail = Some(link.href);
            } else if rel == "http://vaemendis.net/opds-pse/stream" {
                en.stream_count = l.attr("count").and_then(|c| c.parse().ok());
                en.stream = Some(link.href);
            } else if link.kind.contains("profile=opds-catalog")
                || link.kind.starts_with("application/atom+xml")
                || rel == "subsection"
                || rel.starts_with("http://opds-spec.org/sort")
            {
                en.navigation.get_or_insert(link.href);
            }
        }
        f.entries.push(en);
    }
    f
}

pub fn parse_v2(body: &str, base: &str) -> Result<OpdsFeed, String> {
    let v: serde_json::Value = serde_json::from_str(body).map_err(|e| e.to_string())?;
    let s = |v: &serde_json::Value, k: &str| {
        v.get(k).and_then(|x| x.as_str()).unwrap_or("").to_string()
    };
    let md = &v["metadata"];
    let mut f = OpdsFeed {
        title: s(md, "title"),
        total: md.get("numberOfItems").and_then(|x| x.as_u64()),
        ..Default::default()
    };
    let rels = |l: &serde_json::Value| -> Vec<String> {
        match &l["rel"] {
            serde_json::Value::String(r) => vec![r.clone()],
            serde_json::Value::Array(a) => a
                .iter()
                .filter_map(|x| x.as_str().map(str::to_string))
                .collect(),
            _ => vec![],
        }
    };
    let mk = |l: &serde_json::Value| OpdsLink {
        href: abs(base, &s(l, "href")),
        rel: rels(l).join(" "),
        kind: s(l, "type"),
        title: s(l, "title"),
    };
    for l in v["links"].as_array().into_iter().flatten() {
        let link = mk(l);
        for r in rels(l) {
            match r.as_str() {
                "next" => f.next = Some(link.href.clone()),
                "previous" => f.prev = Some(link.href.clone()),
                "up" => f.up = Some(link.href.clone()),
                "start" => f.start = Some(link.href.clone()),
                "search" => f.search = Some(link.href.clone()),
                _ => {}
            }
        }
    }
    for n in v["navigation"].as_array().into_iter().flatten() {
        f.entries.push(OpdsEntry {
            title: s(n, "title"),
            navigation: Some(abs(base, &s(n, "href"))),
            ..Default::default()
        });
    }
    for fac in v["facets"].as_array().into_iter().flatten() {
        for l in fac["links"].as_array().into_iter().flatten() {
            f.facets.push(mk(l));
        }
    }
    let mut pubs: Vec<&serde_json::Value> =
        v["publications"].as_array().into_iter().flatten().collect();
    for g in v["groups"].as_array().into_iter().flatten() {
        pubs.extend(g["publications"].as_array().into_iter().flatten());
        for n in g["navigation"].as_array().into_iter().flatten() {
            f.entries.push(OpdsEntry {
                title: s(n, "title"),
                navigation: Some(abs(base, &s(n, "href"))),
                ..Default::default()
            });
        }
    }
    for p in pubs {
        let m = &p["metadata"];
        let names = |x: &serde_json::Value| -> Vec<String> {
            match x {
                serde_json::Value::String(s) => vec![s.clone()],
                serde_json::Value::Object(_) => x["name"]
                    .as_str()
                    .map(|s| vec![s.to_string()])
                    .unwrap_or_default(),
                serde_json::Value::Array(a) => a
                    .iter()
                    .filter_map(|i| {
                        i.as_str()
                            .map(str::to_string)
                            .or_else(|| i["name"].as_str().map(str::to_string))
                    })
                    .collect(),
                _ => vec![],
            }
        };
        let mut en = OpdsEntry {
            id: s(m, "identifier"),
            title: s(m, "title"),
            authors: names(&m["author"]),
            summary: s(m, "description"),
            updated: s(m, "modified"),
            language: match &m["language"] {
                serde_json::Value::Array(a) => {
                    a.first().and_then(|x| x.as_str()).unwrap_or("").to_string()
                }
                x => x.as_str().unwrap_or("").to_string(),
            },
            publisher: names(&m["publisher"]).join(", "),
            categories: m["subject"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|x| {
                    x.as_str()
                        .map(str::to_string)
                        .or_else(|| x["name"].as_str().map(str::to_string))
                })
                .collect(),
            ..Default::default()
        };
        if let Some(series) = m["belongsTo"]["series"]
            .as_array()
            .and_then(|a| a.first())
            .or(m["belongsTo"].get("series"))
        {
            en.series = series
                .as_str()
                .map(str::to_string)
                .unwrap_or_else(|| s(series, "name"));
            en.series_index = series["position"].as_f64();
        }
        for l in p["links"].as_array().into_iter().flatten() {
            let link = mk(l);
            if link.rel.contains("http://opds-spec.org/acquisition") {
                en.acquisitions.push(link);
            } else if link.kind.contains("opds") {
                en.navigation.get_or_insert(link.href);
            }
        }
        for i in p["images"].as_array().into_iter().flatten() {
            let href = abs(base, &s(i, "href"));
            if en.cover.is_none() {
                en.cover = Some(href.clone());
            }
            en.thumbnail = Some(href);
        }
        f.entries.push(en);
    }
    Ok(f)
}

pub fn fill_search(template: &str, query: &str) -> String {
    let q = urlencoding::encode(query);
    template
        .replace("{searchTerms}", &q)
        .replace("{?query}", &format!("?query={q}"))
        .replace("{query}", &q)
        .replace("{startPage?}", "1")
        .replace("{startIndex?}", "0")
        .replace("{count?}", "")
        .replace("{language?}", "")
}

pub fn opensearch_template(desc_xml: &str, base: &str) -> Option<String> {
    let root = xml::parse(desc_xml);
    let mut urls = Vec::new();
    root.find_all("Url", &mut urls);
    urls.iter()
        .find(|u| u.attr("type").is_some_and(|t| t.contains("atom")))
        .or_else(|| urls.first())
        .and_then(|u| u.attr("template"))
        .map(|t| abs(base, t))
}

pub struct ServedEntry<'a> {
    pub id: &'a str,
    pub title: &'a str,
    pub authors: &'a [String],
    pub summary: &'a str,
    pub updated: &'a str,
    pub language: &'a str,
    pub download_href: &'a str,
    pub mime: &'a str,
    pub cover_href: Option<&'a str>,
    pub stream: Option<(&'a str, u32)>,
}

pub struct ServedNav<'a> {
    pub id: &'a str,
    pub title: &'a str,
    pub href: &'a str,
    pub content: &'a str,
}

pub fn render_feed(
    id: &str,
    title: &str,
    self_href: &str,
    search_href: &str,
    navs: &[ServedNav<'_>],
    entries: &[ServedEntry<'_>],
    next: Option<&str>,
) -> String {
    let now = chrono::Utc::now().to_rfc3339();
    let kind = if entries.is_empty() {
        "navigation"
    } else {
        "acquisition"
    };
    let mut o = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<feed xmlns="http://www.w3.org/2005/Atom" xmlns:dc="http://purl.org/dc/terms/" xmlns:opds="http://opds-spec.org/2010/catalog" xmlns:pse="http://vaemendis.net/opds-pse/ns" xmlns:opensearch="http://a9.com/-/spec/opensearch/1.1/">
  <id>{}</id>
  <title>{}</title>
  <updated>{now}</updated>
  <author><name>OmniGet</name></author>
  <link rel="self" href="{}" type="application/atom+xml;profile=opds-catalog;kind={kind}"/>
  <link rel="start" href="/opds" type="application/atom+xml;profile=opds-catalog;kind=navigation"/>
  <link rel="search" href="{}" type="application/opensearchdescription+xml"/>
"#,
        xml::escape(id),
        xml::escape(title),
        xml::escape(self_href),
        xml::escape(search_href),
    );
    if let Some(n) = next {
        o.push_str(&format!(
            "  <link rel=\"next\" href=\"{}\" type=\"application/atom+xml;profile=opds-catalog;kind={kind}\"/>\n",
            xml::escape(n)
        ));
    }
    for n in navs {
        o.push_str(&format!(
            "  <entry>\n    <id>{}</id>\n    <title>{}</title>\n    <updated>{now}</updated>\n    <content type=\"text\">{}</content>\n    <link rel=\"subsection\" href=\"{}\" type=\"application/atom+xml;profile=opds-catalog;kind=acquisition\"/>\n  </entry>\n",
            xml::escape(n.id),
            xml::escape(n.title),
            xml::escape(n.content),
            xml::escape(n.href)
        ));
    }
    for e in entries {
        o.push_str(&format!(
            "  <entry>\n    <id>{}</id>\n    <title>{}</title>\n    <updated>{}</updated>\n",
            xml::escape(e.id),
            xml::escape(e.title),
            xml::escape(if e.updated.is_empty() {
                &now
            } else {
                e.updated
            })
        ));
        for a in e.authors {
            o.push_str(&format!(
                "    <author><name>{}</name></author>\n",
                xml::escape(a)
            ));
        }
        if !e.language.is_empty() {
            o.push_str(&format!(
                "    <dc:language>{}</dc:language>\n",
                xml::escape(e.language)
            ));
        }
        if !e.summary.is_empty() {
            o.push_str(&format!(
                "    <summary type=\"text\">{}</summary>\n",
                xml::escape(e.summary)
            ));
        }
        if let Some(c) = e.cover_href {
            o.push_str(&format!(
                "    <link rel=\"http://opds-spec.org/image\" href=\"{0}\" type=\"image/jpeg\"/>\n    <link rel=\"http://opds-spec.org/image/thumbnail\" href=\"{0}&amp;t=1\" type=\"image/jpeg\"/>\n",
                xml::escape(c)
            ));
        }
        o.push_str(&format!(
            "    <link rel=\"http://opds-spec.org/acquisition\" href=\"{}\" type=\"{}\"/>\n",
            xml::escape(e.download_href),
            xml::escape(e.mime)
        ));
        if let Some((tpl, count)) = e.stream {
            o.push_str(&format!(
                "    <link rel=\"http://vaemendis.net/opds-pse/stream\" href=\"{}\" type=\"image/jpeg\" pse:count=\"{count}\"/>\n",
                xml::escape(tpl)
            ));
        }
        o.push_str("  </entry>\n");
    }
    o.push_str("</feed>\n");
    o
}

pub fn opensearch_description(template: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<OpenSearchDescription xmlns="http://a9.com/-/spec/opensearch/1.1/">
  <ShortName>OmniGet</ShortName>
  <Description>Search the OmniGet library</Description>
  <InputEncoding>UTF-8</InputEncoding>
  <Url type="application/atom+xml;profile=opds-catalog;kind=acquisition" template="{}"/>
</OpenSearchDescription>
"#,
        xml::escape(template)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_atom_feed() {
        let body = r#"<feed xmlns="http://www.w3.org/2005/Atom">
<id>x</id><title>Standard Ebooks</title>
<link rel="next" href="?page=2"/>
<link rel="search" href="/opensearch" type="application/opensearchdescription+xml"/>
<entry><id>e1</id><title>Moby Dick</title><author><name>Herman Melville</name></author>
<link rel="http://opds-spec.org/acquisition/open-access" href="/ebooks/m.epub" type="application/epub+zip"/>
<link rel="http://opds-spec.org/image" href="/c.jpg" type="image/jpeg"/>
</entry>
<entry><id>n</id><title>Subjects</title><link rel="subsection" href="/subjects" type="application/atom+xml;profile=opds-catalog;kind=navigation"/></entry>
</feed>"#;
        let f = parse(body, "https://standardebooks.org/feeds/opds/all").unwrap();
        assert_eq!(f.title, "Standard Ebooks");
        assert_eq!(
            f.next.as_deref(),
            Some("https://standardebooks.org/feeds/opds/all?page=2")
        );
        assert_eq!(
            f.entries[0].acquisitions[0].href,
            "https://standardebooks.org/ebooks/m.epub"
        );
        assert_eq!(f.entries[0].authors, vec!["Herman Melville"]);
        assert_eq!(
            f.entries[1].navigation.as_deref(),
            Some("https://standardebooks.org/subjects")
        );
    }

    #[test]
    fn parses_opds2() {
        let body = r#"{"metadata":{"title":"Cat"},"links":[{"rel":"next","href":"p2"}],
"navigation":[{"title":"New","href":"/new"}],
"publications":[{"metadata":{"title":"B","author":[{"name":"A"}]},"links":[{"rel":"http://opds-spec.org/acquisition","href":"/b.epub","type":"application/epub+zip"}],"images":[{"href":"/b.jpg"}]}]}"#;
        let f = parse(body, "https://x.org/opds/").unwrap();
        assert_eq!(f.next.as_deref(), Some("https://x.org/opds/p2"));
        assert_eq!(f.entries.len(), 2);
        assert_eq!(f.entries[1].authors, vec!["A"]);
        assert_eq!(f.entries[1].cover.as_deref(), Some("https://x.org/b.jpg"));
    }

    #[test]
    fn rendered_feed_parses_back() {
        let authors = vec!["Ana".to_string()];
        let xml = render_feed(
            "urn:omniget",
            "Library",
            "/opds/all",
            "/opds/search.xml",
            &[],
            &[ServedEntry {
                id: "1",
                title: "T & U",
                authors: &authors,
                summary: "",
                updated: "",
                language: "pt",
                download_href: "/opds/file/1",
                mime: "application/epub+zip",
                cover_href: Some("/opds/cover/1?k=a"),
                stream: None,
            }],
            None,
        );
        let f = parse(&xml, "http://127.0.0.1:1/").unwrap();
        assert_eq!(f.entries[0].title, "T & U");
        assert_eq!(f.entries[0].acquisitions.len(), 1);
    }
}

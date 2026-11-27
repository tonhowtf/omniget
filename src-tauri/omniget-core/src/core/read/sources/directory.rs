use scraper::{Html, Selector};

use super::{Download, Hit, Page};

const BOOK_EXTS: &[(&str, &str)] = &[
    ("epub", "EPUB"),
    ("pdf", "PDF"),
    ("mobi", "MOBI"),
    ("azw3", "AZW3"),
    ("cbz", "CBZ"),
    ("cbr", "CBR"),
    ("cb7", "CB7"),
    ("djvu", "DjVu"),
    ("fb2", "FB2"),
    ("fbz", "FBZ"),
    ("txt", "TXT"),
];

pub fn same_origin(a: &str, b: &str) -> bool {
    match (url::Url::parse(a), url::Url::parse(b)) {
        (Ok(a), Ok(b)) => a.origin() == b.origin() && !matches!(a.origin(), url::Origin::Opaque(_)),
        _ => false,
    }
}

pub fn parse_listing(html: &str, base_url: &str, source_id: &str, query: &str) -> Page {
    let Ok(base) = url::Url::parse(base_url) else {
        return Page::default();
    };
    let doc = Html::parse_document(html);
    let sel = Selector::parse("a[href]").expect("static selector");
    let needle = query.trim().to_ascii_lowercase();
    let mut seen = std::collections::HashSet::new();
    let mut dirs = Vec::new();
    let mut files = Vec::new();

    for a in doc.select(&sel) {
        let Some(raw_href) = a.value().attr("href") else {
            continue;
        };
        let href = raw_href.trim();
        if href.is_empty()
            || href.starts_with('#')
            || href.starts_with('?')
            || href == "../"
            || href == "./"
            || href.starts_with("javascript:")
            || href.starts_with("mailto:")
        {
            continue;
        }
        let Ok(resolved) = base.join(href) else {
            continue;
        };
        if resolved == base {
            continue;
        }
        let resolved_str = resolved.to_string();
        if !seen.insert(resolved_str.clone()) {
            continue;
        }

        let link_text = a
            .text()
            .collect::<Vec<_>>()
            .join(" ")
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        let raw_name = resolved
            .path_segments()
            .and_then(|mut segs| segs.rfind(|s| !s.is_empty()))
            .unwrap_or(&link_text);
        let decoded_name = url::form_urlencoded::parse(format!("x={raw_name}").as_bytes())
            .next()
            .map(|(_, v)| v.into_owned())
            .unwrap_or_else(|| raw_name.to_string());

        if href.ends_with('/') {
            // Skip parent directory links
            if resolved.path().len() < base.path().len() && base.path().starts_with(resolved.path())
            {
                continue;
            }
            let title = if !decoded_name.is_empty() {
                decoded_name
            } else {
                link_text
            };
            if title.is_empty() || title.eq_ignore_ascii_case("parent directory") {
                continue;
            }
            if !needle.is_empty() && !title.to_ascii_lowercase().contains(&needle) {
                continue;
            }
            dirs.push(Hit {
                source: source_id.to_string(),
                key: resolved_str.clone(),
                title,
                kind: "folder".into(),
                url: resolved_str,
                ..Default::default()
            });
            continue;
        }

        let ext = resolved
            .path()
            .rsplit('.')
            .next()
            .unwrap_or("")
            .to_ascii_lowercase();
        let Some((fmt, label)) = BOOK_EXTS.iter().find(|(e, _)| *e == ext).copied() else {
            continue;
        };
        let stem = decoded_name
            .rsplit_once('.')
            .map(|(s, _)| s)
            .unwrap_or(&decoded_name)
            .replace('_', " ")
            .trim()
            .to_string();
        let title = if stem.is_empty() { link_text } else { stem };
        if !needle.is_empty() && !title.to_ascii_lowercase().contains(&needle) {
            continue;
        }
        files.push(Hit {
            source: source_id.to_string(),
            key: resolved_str.clone(),
            title,
            kind: "book".into(),
            downloads: vec![Download {
                url: resolved_str.clone(),
                format: fmt.into(),
                label: label.into(),
            }],
            url: resolved_str,
            ..Default::default()
        });
    }

    dirs.extend(files);
    Page {
        total: Some(dirs.len() as u64),
        next: None,
        hits: dirs,
    }
}

pub async fn browse(
    client: &reqwest::Client,
    url: &str,
    source_id: &str,
    query: &str,
    username: &str,
    password: &str,
) -> Result<Page, String> {
    let mut req = client.get(url);
    if !username.is_empty() {
        req = req.basic_auth(username, Some(password));
    }
    let resp = req.send().await.map_err(|e| e.to_string())?;
    if !resp.status().is_success() {
        return Err(format!("{} from directory", resp.status()));
    }
    let final_url = resp.url().to_string();
    let body = resp.text().await.map_err(|e| e.to_string())?;
    Ok(parse_listing(&body, &final_url, source_id, query))
}

#[cfg(test)]
mod tests {
    #[test]
    fn credentials_stay_on_the_catalog_origin() {
        let cat = "https://books.example/library/";
        assert!(super::same_origin(
            cat,
            "https://books.example/library/sci-fi/"
        ));
        assert!(super::same_origin(
            cat,
            "https://books.example/other/a.epub"
        ));
        assert!(!super::same_origin(cat, "https://other.example/books/"));
        assert!(!super::same_origin(cat, "http://books.example/library/"));
        assert!(!super::same_origin(
            cat,
            "https://books.example:8443/library/"
        ));
        assert!(!super::same_origin(
            cat,
            "https://books.example.evil.test/library/"
        ));
        assert!(!super::same_origin(cat, "not a url"));
    }

    use super::*;

    #[test]
    fn parses_apache_directory_index_and_filters_query() {
        let html = r#"
        <html><body><h1>Index of /books/</h1>
        <a href="?C=N;O=D">Name</a>
        <a href="../">Parent Directory</a>
        <a href="SciFi/">SciFi/</a>
        <a href="Clean_Code.pdf">Clean_Code.pdf</a>
        <a href="Dune%20Messiah.epub">Dune Messiah.epub</a>
        <a href="readme.png">readme.png</a>
        </body></html>"#;

        let all = parse_listing(html, "https://example.org/books/", "dir:1", "");
        assert_eq!(all.hits.len(), 3);
        assert_eq!(all.hits[0].kind, "folder");
        assert_eq!(all.hits[0].title, "SciFi");
        assert_eq!(all.hits[1].kind, "book");
        assert_eq!(all.hits[1].title, "Clean Code");
        assert_eq!(all.hits[1].downloads[0].format, "pdf");
        assert_eq!(all.hits[2].title, "Dune Messiah");
        assert_eq!(all.hits[2].downloads[0].format, "epub");

        let filtered = parse_listing(html, "https://example.org/books/", "dir:1", "dune");
        assert_eq!(filtered.hits.len(), 1);
        assert_eq!(filtered.hits[0].title, "Dune Messiah");
    }
}

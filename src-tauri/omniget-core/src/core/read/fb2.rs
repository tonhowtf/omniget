use base64::Engine;

use super::epub::BookMeta;
use super::xml;

pub fn read(text: &str) -> (BookMeta, Option<Vec<u8>>) {
    let root = xml::parse(text);
    let mut m = BookMeta::default();
    let Some(ti) = root.find("title-info") else {
        return (m, None);
    };
    m.title = ti.child_text("book-title").unwrap_or_default();
    for a in ti.children_named("author") {
        let name = ["first-name", "middle-name", "last-name"]
            .iter()
            .filter_map(|k| a.child_text(k))
            .collect::<Vec<_>>()
            .join(" ");
        let name = if name.is_empty() {
            a.child_text("nickname").unwrap_or_default()
        } else {
            name
        };
        if !name.is_empty() {
            m.authors.push(name);
        }
    }
    m.subjects = ti.children_named("genre").map(|g| g.all_text()).collect();
    m.description = ti
        .child("annotation")
        .map(|a| a.all_text())
        .unwrap_or_default();
    m.language = ti.child_text("lang").unwrap_or_default();
    m.published = ti.child_text("date").unwrap_or_default();
    if let Some(seq) = ti.child("sequence") {
        m.series = seq.attr("name").unwrap_or("").to_string();
        m.series_index = seq.attr("number").and_then(|n| n.parse().ok());
    }
    if let Some(pi) = root.find("publish-info") {
        m.publisher = pi.child_text("publisher").unwrap_or_default();
        if let Some(isbn) = pi.child_text("isbn") {
            m.identifiers.push(("isbn".into(), isbn));
        }
    }
    let cover_id = ti
        .child("coverpage")
        .and_then(|c| c.child("image"))
        .and_then(|i| i.attr("href"))
        .map(|h| h.trim_start_matches('#').to_string());
    let cover = cover_id.and_then(|id| {
        let mut bins = Vec::new();
        root.find_all("binary", &mut bins);
        let b = bins
            .into_iter()
            .find(|b| b.attr("id") == Some(id.as_str()))?;
        let clean: String = b.text.chars().filter(|c| !c.is_whitespace()).collect();
        base64::engine::general_purpose::STANDARD.decode(clean).ok()
    });
    (m, cover)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_fb2() {
        let doc = r##"<?xml version="1.0" encoding="utf-8"?>
<FictionBook xmlns:l="http://www.w3.org/1999/xlink"><description><title-info>
<genre>sf</genre><author><first-name>Isaac</first-name><last-name>Asimov</last-name></author>
<book-title>Foundation</book-title><annotation><p>Empire falls.</p></annotation>
<coverpage><image l:href="#c.jpg"/></coverpage><lang>en</lang><sequence name="Foundation" number="1"/>
</title-info></description><binary id="c.jpg" content-type="image/jpeg">/9j/4A==</binary></FictionBook>"##;
        let (m, cover) = read(doc);
        assert_eq!(m.title, "Foundation");
        assert_eq!(m.authors, vec!["Isaac Asimov"]);
        assert_eq!(m.series_index, Some(1.0));
        assert_eq!(m.description, "Empire falls.");
        assert_eq!(cover.unwrap(), vec![0xFF, 0xD8, 0xFF, 0xE0]);
    }
}

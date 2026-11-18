use std::io::{Read, Seek, Write};

use serde::{Deserialize, Serialize};

use super::xml::{self, Node};

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct TocEntry {
    pub title: String,
    pub href: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub children: Vec<TocEntry>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct BookMeta {
    pub title: String,
    pub authors: Vec<String>,
    pub language: String,
    pub publisher: String,
    pub description: String,
    pub subjects: Vec<String>,
    pub identifiers: Vec<(String, String)>,
    pub series: String,
    pub series_index: Option<f64>,
    pub published: String,
    pub rights: String,
    pub cover: Option<String>,
    pub rtl: bool,
    pub fixed_layout: bool,
    pub page_count: Option<u32>,
    pub toc: Vec<TocEntry>,
}

impl BookMeta {
    pub fn isbn(&self) -> Option<&str> {
        self.identifiers
            .iter()
            .find(|(s, v)| s == "isbn" || looks_like_isbn(v))
            .map(|(_, v)| v.as_str())
    }
}

pub fn looks_like_isbn(v: &str) -> bool {
    let digits: String = v
        .chars()
        .filter(|c| c.is_ascii_digit() || *c == 'X')
        .collect();
    (digits.len() == 10 || digits.len() == 13) && v.len() <= 20
}

fn dir_of(path: &str) -> &str {
    match path.rfind('/') {
        Some(i) => &path[..=i],
        None => "",
    }
}

pub fn resolve(base_dir: &str, href: &str) -> String {
    let href = href.split('#').next().unwrap_or("");
    let href = urlencoding::decode(href)
        .map(|c| c.into_owned())
        .unwrap_or_else(|_| href.to_string());
    let mut parts: Vec<&str> = base_dir.split('/').filter(|s| !s.is_empty()).collect();
    for seg in href.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            s => parts.push(s),
        }
    }
    parts.join("/")
}

fn read_entry<R: Read + Seek>(zip: &mut zip::ZipArchive<R>, name: &str) -> Option<String> {
    let mut f = zip.by_name(name).ok()?;
    let mut s = String::new();
    f.read_to_string(&mut s).ok()?;
    Some(s)
}

pub fn read_entry_bytes<R: Read + Seek>(
    zip: &mut zip::ZipArchive<R>,
    name: &str,
) -> Option<Vec<u8>> {
    let mut f = zip.by_name(name).ok()?;
    let mut v = Vec::with_capacity(f.size() as usize);
    f.read_to_end(&mut v).ok()?;
    Some(v)
}

pub fn opf_path<R: Read + Seek>(zip: &mut zip::ZipArchive<R>) -> Option<String> {
    let container = read_entry(zip, "META-INF/container.xml")?;
    let root = xml::parse(&container);
    root.find("rootfile")
        .and_then(|r| r.attr("full-path"))
        .map(str::to_string)
        .or_else(|| {
            (0..zip.len()).find_map(|i| {
                let n = zip.by_index(i).ok()?.name().ok()?.to_string();
                n.ends_with(".opf").then_some(n)
            })
        })
}

pub fn read<R: Read + Seek>(reader: R) -> Result<BookMeta, String> {
    let mut zip = zip::ZipArchive::new(reader).map_err(|e| e.to_string())?;
    let opf = opf_path(&mut zip).ok_or("no OPF in EPUB")?;
    let text = read_entry(&mut zip, &opf).ok_or("unreadable OPF")?;
    let mut meta = parse_opf(&text, &opf);
    let root = xml::parse(&text);
    let base = dir_of(&opf).to_string();
    if let Some(manifest) = root.find("manifest") {
        let nav = manifest.children_named("item").find(|i| {
            i.attr("properties")
                .is_some_and(|p| p.split_whitespace().any(|x| x == "nav"))
        });
        if let Some(href) = nav.and_then(|n| n.attr("href")) {
            let p = resolve(&base, href);
            if let Some(doc) = read_entry(&mut zip, &p) {
                meta.toc = parse_nav(&doc, dir_of(&p));
            }
        }
        if meta.toc.is_empty() {
            let ncx_id = root
                .find("spine")
                .and_then(|s| s.attr("toc"))
                .map(str::to_string);
            let ncx = manifest.children_named("item").find(|i| {
                ncx_id.as_deref().is_some_and(|id| i.attr("id") == Some(id))
                    || i.attr("media-type") == Some("application/x-dtbncx+xml")
            });
            if let Some(href) = ncx.and_then(|n| n.attr("href")) {
                let p = resolve(&base, href);
                if let Some(doc) = read_entry(&mut zip, &p) {
                    meta.toc = parse_ncx(&doc, dir_of(&p));
                }
            }
        }
    }
    if meta.cover.is_none() {
        meta.cover = (0..zip.len()).find_map(|i| {
            let n = zip.by_index(i).ok()?.name().ok()?.to_string();
            (super::format::is_image_name(&n) && n.to_ascii_lowercase().contains("cover"))
                .then_some(n)
        });
    }
    Ok(meta)
}

pub fn parse_opf(text: &str, opf_path: &str) -> BookMeta {
    let root = xml::parse(text);
    let mut m = BookMeta::default();
    let Some(pkg) = root.child("package").or_else(|| root.find("package")) else {
        return m;
    };
    let base = dir_of(opf_path);
    let md = pkg.find("metadata").cloned().unwrap_or_default();
    let refines = |id: &str, prop: &str| -> Option<String> {
        let want = format!("#{id}");
        md.children_named("meta")
            .find(|x| x.attr("refines") == Some(want.as_str()) && x.attr("property") == Some(prop))
            .map(|x| x.all_text())
    };
    for c in &md.children {
        let text = c.all_text();
        match c.name.as_str() {
            "title" if m.title.is_empty() => m.title = text,
            "creator" => {
                let role = c
                    .attr("role")
                    .map(str::to_string)
                    .or_else(|| c.attr("id").and_then(|id| refines(id, "role")));
                if role.as_deref().is_none_or(|r| r == "aut") && !text.is_empty() {
                    m.authors.push(text);
                }
            }
            "language" if m.language.is_empty() => m.language = text,
            "publisher" if m.publisher.is_empty() => m.publisher = text,
            "description" if m.description.is_empty() => m.description = text,
            "subject" if !text.is_empty() => m.subjects.push(text),
            "rights" if m.rights.is_empty() => m.rights = text,
            "date" if m.published.is_empty() => m.published = text,
            "identifier" if !text.is_empty() => {
                let scheme = c
                    .attr("scheme")
                    .map(|s| s.to_ascii_lowercase())
                    .or_else(|| {
                        let lower = text.to_ascii_lowercase();
                        lower
                            .strip_prefix("urn:")
                            .and_then(|r| r.split(':').next())
                            .map(str::to_string)
                    })
                    .unwrap_or_else(|| {
                        if looks_like_isbn(&text) {
                            "isbn".into()
                        } else {
                            String::new()
                        }
                    });
                let value = text
                    .rsplit_once(':')
                    .filter(|_| text.to_ascii_lowercase().starts_with("urn:"))
                    .map(|(_, v)| v.to_string())
                    .unwrap_or(text);
                m.identifiers.push((scheme, value));
            }
            "meta" => {
                let name = c.attr("name").unwrap_or("");
                let prop = c.attr("property").unwrap_or("");
                let content = c.attr("content").map(str::to_string).unwrap_or(text);
                match (name, prop) {
                    ("calibre:series", _) => m.series = content,
                    ("calibre:series_index", _) => m.series_index = content.parse().ok(),
                    (_, "belongs-to-collection") if m.series.is_empty() => {
                        m.series = content;
                        if let Some(id) = c.attr("id") {
                            m.series_index =
                                refines(id, "group-position").and_then(|v| v.parse().ok());
                        }
                    }
                    (_, "rendition:layout") => m.fixed_layout = content == "pre-paginated",
                    ("fixed-layout", _) => m.fixed_layout = content == "true",
                    _ => {}
                }
            }
            _ => {}
        }
    }
    if let Some(spine) = pkg.find("spine") {
        m.rtl = spine.attr("page-progression-direction") == Some("rtl");
    }
    if let Some(manifest) = pkg.find("manifest") {
        let items: Vec<&Node> = manifest.children_named("item").collect();
        let by_prop = items.iter().find(|i| {
            i.attr("properties")
                .is_some_and(|p| p.split_whitespace().any(|x| x == "cover-image"))
        });
        let by_meta = md
            .children_named("meta")
            .find(|x| x.attr("name") == Some("cover"))
            .and_then(|x| x.attr("content"))
            .and_then(|id| items.iter().find(|i| i.attr("id") == Some(id)));
        let by_id = items.iter().find(|i| {
            i.attr("id")
                .is_some_and(|id| id.eq_ignore_ascii_case("cover"))
                && i.attr("media-type")
                    .is_some_and(|t| t.starts_with("image/"))
        });
        m.cover = by_prop
            .or(by_meta)
            .or(by_id)
            .and_then(|i| i.attr("href"))
            .map(|h| resolve(base, h));
    }
    m
}

fn parse_nav(doc: &str, base: &str) -> Vec<TocEntry> {
    let root = xml::parse(doc);
    let mut navs = Vec::new();
    root.find_all("nav", &mut navs);
    let nav = navs
        .iter()
        .find(|n| n.attr("type") == Some("toc"))
        .or_else(|| navs.first());
    fn walk_ol(ol: &Node, base: &str) -> Vec<TocEntry> {
        ol.children_named("li")
            .filter_map(|li| {
                let a = li.child("a").or_else(|| li.child("span"))?;
                let children = li.child("ol").map(|o| walk_ol(o, base)).unwrap_or_default();
                let href = a
                    .attr("href")
                    .map(|h| resolve_keep_fragment(base, h))
                    .unwrap_or_default();
                Some(TocEntry {
                    title: a.all_text(),
                    href,
                    children,
                })
            })
            .collect()
    }
    nav.and_then(|n| n.find("ol"))
        .map(|ol| walk_ol(ol, base))
        .unwrap_or_default()
}

fn parse_ncx(doc: &str, base: &str) -> Vec<TocEntry> {
    let root = xml::parse(doc);
    fn walk(n: &Node, base: &str) -> Vec<TocEntry> {
        n.children_named("navPoint")
            .map(|p| TocEntry {
                title: p
                    .child("navLabel")
                    .map(|l| l.all_text())
                    .unwrap_or_default(),
                href: p
                    .child("content")
                    .and_then(|c| c.attr("src"))
                    .map(|h| resolve_keep_fragment(base, h))
                    .unwrap_or_default(),
                children: walk(p, base),
            })
            .collect()
    }
    root.find("navMap")
        .map(|m| walk(m, base))
        .unwrap_or_default()
}

fn resolve_keep_fragment(base: &str, href: &str) -> String {
    let (path, frag) = match href.split_once('#') {
        Some((p, f)) => (p, Some(f)),
        None => (href, None),
    };
    let mut out = resolve(base, path);
    if let Some(f) = frag {
        out.push('#');
        out.push_str(f);
    }
    out
}

pub fn spine_text<R: Read + Seek>(reader: R) -> Result<Vec<(String, String)>, String> {
    let mut zip = zip::ZipArchive::new(reader).map_err(|e| e.to_string())?;
    let opf = opf_path(&mut zip).ok_or("no OPF in EPUB")?;
    let text = read_entry(&mut zip, &opf).ok_or("unreadable OPF")?;
    let root = xml::parse(&text);
    let base = dir_of(&opf).to_string();
    let manifest = root.find("manifest").cloned().unwrap_or_default();
    let spine = root.find("spine").cloned().unwrap_or_default();
    let mut out = Vec::new();
    for item in spine.children_named("itemref") {
        let Some(idref) = item.attr("idref") else {
            continue;
        };
        let Some(href) = manifest
            .children_named("item")
            .find(|i| i.attr("id") == Some(idref))
            .and_then(|i| i.attr("href"))
        else {
            continue;
        };
        let path = resolve(&base, href);
        let Some(doc) = read_entry(&mut zip, &path) else {
            continue;
        };
        let html = scraper::Html::parse_document(&doc);
        let title = scraper::Selector::parse("h1, h2, h3, title")
            .ok()
            .and_then(|s| html.select(&s).next().map(|e| e.text().collect::<String>()))
            .map(|t| t.split_whitespace().collect::<Vec<_>>().join(" "))
            .unwrap_or_default();
        let body = scraper::Selector::parse("body")
            .ok()
            .and_then(|s| {
                html.select(&s).next().map(|b| {
                    b.text()
                        .collect::<Vec<_>>()
                        .join(" ")
                        .split_whitespace()
                        .collect::<Vec<_>>()
                        .join(" ")
                })
            })
            .unwrap_or_default();
        if !body.is_empty() {
            out.push((title, body));
        }
    }
    Ok(out)
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MetaEdit {
    pub title: Option<String>,
    pub authors: Option<Vec<String>>,
    pub language: Option<String>,
    pub publisher: Option<String>,
    pub description: Option<String>,
    pub subjects: Option<Vec<String>>,
    pub series: Option<String>,
    pub series_index: Option<f64>,
    pub published: Option<String>,
    pub isbn: Option<String>,
}

pub fn write_metadata(path: &std::path::Path, edit: &MetaEdit) -> Result<(), String> {
    let src = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let mut zip = zip::ZipArchive::new(src).map_err(|e| e.to_string())?;
    let opf = opf_path(&mut zip).ok_or("no OPF in EPUB")?;
    let text = read_entry(&mut zip, &opf).ok_or("unreadable OPF")?;
    let new_opf = rewrite_opf(&text, edit)?;
    let tmp = path.with_extension("omniget-tmp");
    {
        let out = std::fs::File::create(&tmp).map_err(|e| e.to_string())?;
        let mut w = zip::ZipWriter::new(out);
        let stored = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);
        let deflated = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);
        w.start_file("mimetype", stored)
            .map_err(|e| e.to_string())?;
        w.write_all(b"application/epub+zip")
            .map_err(|e| e.to_string())?;
        for i in 0..zip.len() {
            let entry = zip.by_index_raw(i).map_err(|e| e.to_string())?;
            let name = entry.name().map_err(|e| e.to_string())?.to_string();
            if name == "mimetype" {
                continue;
            }
            if name == opf {
                drop(entry);
                w.start_file(name, deflated).map_err(|e| e.to_string())?;
                w.write_all(new_opf.as_bytes()).map_err(|e| e.to_string())?;
            } else {
                w.raw_copy_file(entry).map_err(|e| e.to_string())?;
            }
        }
        w.finish().map_err(|e| e.to_string())?;
    }
    std::fs::rename(&tmp, path).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        e.to_string()
    })
}

pub fn rewrite_opf(text: &str, edit: &MetaEdit) -> Result<String, String> {
    let open_re = regex::Regex::new(r"<(?:\w+:)?metadata\b[^>]*>").expect("static regex");
    let close_re = regex::Regex::new(r"</(?:\w+:)?metadata>").expect("static regex");
    let open = open_re.find(text).ok_or("no <metadata> in OPF")?;
    let body_start = open.end();
    let close = body_start
        + close_re
            .find(&text[body_start..])
            .ok_or("unclosed <metadata>")?
            .start();
    let mut body = text[body_start..close].to_string();
    let drop_tag = |body: &mut String, tag: &str| {
        let re = regex::Regex::new(&format!(
            r"(?s)\s*<(?:dc:)?{tag}\b[^>]*?(?:/>|>.*?</(?:dc:)?{tag}>)"
        ))
        .expect("static regex");
        *body = re.replace_all(body, "").to_string();
    };
    let drop_meta = |body: &mut String, name: &str| {
        let re = regex::Regex::new(&format!(
            r#"\s*<meta\b[^>]*name="{}"[^>]*/?>"#,
            regex::escape(name)
        ))
        .expect("static regex");
        *body = re.replace_all(body, "").to_string();
    };
    let mut add = String::new();
    let mut push = |tag: &str, v: &str, extra: &str| {
        add.push_str(&format!(
            "\n    <dc:{tag}{extra}>{}</dc:{tag}>",
            xml::escape(v)
        ));
    };
    if let Some(v) = &edit.title {
        drop_tag(&mut body, "title");
        push("title", v, "");
    }
    if let Some(v) = &edit.authors {
        drop_tag(&mut body, "creator");
        for a in v {
            push("creator", a, " opf:role=\"aut\"");
        }
    }
    if let Some(v) = &edit.language {
        drop_tag(&mut body, "language");
        push("language", v, "");
    }
    if let Some(v) = &edit.publisher {
        drop_tag(&mut body, "publisher");
        push("publisher", v, "");
    }
    if let Some(v) = &edit.description {
        drop_tag(&mut body, "description");
        push("description", v, "");
    }
    if let Some(v) = &edit.subjects {
        drop_tag(&mut body, "subject");
        for s in v {
            push("subject", s, "");
        }
    }
    if let Some(v) = &edit.published {
        drop_tag(&mut body, "date");
        push("date", v, "");
    }
    if let Some(v) = &edit.isbn {
        push("identifier", v, " opf:scheme=\"ISBN\"");
    }
    if let Some(v) = &edit.series {
        drop_meta(&mut body, "calibre:series");
        drop_meta(&mut body, "calibre:series_index");
        if !v.is_empty() {
            add.push_str(&format!(
                "\n    <meta name=\"calibre:series\" content=\"{}\"/>",
                xml::escape(v)
            ));
            if let Some(i) = edit.series_index {
                add.push_str(&format!(
                    "\n    <meta name=\"calibre:series_index\" content=\"{i}\"/>"
                ));
            }
        }
    }
    let mut out = String::with_capacity(text.len() + add.len());
    out.push_str(&text[..body_start]);
    out.push_str(add.trim_end_matches('\n'));
    out.push_str(&body);
    out.push_str(&text[close..]);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    const OPF: &str = r##"<?xml version="1.0"?>
<package xmlns="http://www.idpf.org/2007/opf" version="3.0">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
    <dc:title>Dom Casmurro</dc:title>
    <dc:creator id="c1">Machado de Assis</dc:creator>
    <meta refines="#c1" property="role">aut</meta>
    <dc:creator opf:role="edt">Someone</dc:creator>
    <dc:language>pt</dc:language>
    <dc:identifier>urn:isbn:9788535910663</dc:identifier>
    <meta name="calibre:series" content="Romances"/>
    <meta name="calibre:series_index" content="3"/>
    <meta name="cover" content="img"/>
  </metadata>
  <manifest>
    <item id="img" href="../images/c.jpg" media-type="image/jpeg"/>
  </manifest>
  <spine page-progression-direction="rtl"/>
</package>"##;

    #[test]
    fn parses_opf() {
        let m = parse_opf(OPF, "OEBPS/content.opf");
        assert_eq!(m.title, "Dom Casmurro");
        assert_eq!(m.authors, vec!["Machado de Assis"]);
        assert_eq!(m.series, "Romances");
        assert_eq!(m.series_index, Some(3.0));
        assert_eq!(m.cover.as_deref(), Some("images/c.jpg"));
        assert_eq!(m.isbn(), Some("9788535910663"));
        assert!(m.rtl);
    }

    #[test]
    fn rewrites_opf_keeps_rest() {
        let out = rewrite_opf(
            OPF,
            &MetaEdit {
                title: Some("Novo & título".into()),
                series: Some("S".into()),
                series_index: Some(1.5),
                ..Default::default()
            },
        )
        .unwrap();
        let m = parse_opf(&out, "OEBPS/content.opf");
        assert_eq!(m.title, "Novo & título");
        assert_eq!(m.series, "S");
        assert_eq!(m.series_index, Some(1.5));
        assert_eq!(m.authors, vec!["Machado de Assis"]);
        assert!(out.contains("<manifest>"));
    }

    #[test]
    fn resolves_relative() {
        assert_eq!(
            resolve("OEBPS/text/", "../img/a%20b.png#x"),
            "OEBPS/img/a b.png"
        );
    }
}

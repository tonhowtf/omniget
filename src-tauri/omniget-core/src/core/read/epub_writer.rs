use std::io::Write;
use std::path::Path;

use super::xml::escape;

#[derive(Debug, Clone, Default)]
pub struct Chapter {
    pub title: String,
    pub body: String,
}

#[derive(Debug, Clone, Default)]
pub struct EpubMeta {
    pub title: String,
    pub authors: Vec<String>,
    pub language: String,
    pub description: String,
    pub publisher: String,
    pub subjects: Vec<String>,
    pub series: String,
    pub series_index: Option<f64>,
    pub source_url: String,
    pub rtl: bool,
}

const CSS: &str = "body{margin:0 5%;line-height:1.5;font-family:serif}\nh1,h2,h3{line-height:1.2;text-align:left}\nh1{margin-top:2em}\np{margin:0;text-indent:1.2em}\np+p{margin-top:.2em}\nblockquote{margin:1em 2em;font-style:italic}\nimg{max-width:100%}\npre,code{font-family:monospace;white-space:pre-wrap}\n.center{text-align:center;text-indent:0}\n";

fn uid() -> String {
    format!("urn:uuid:{}", uuid::Uuid::new_v4())
}

fn opf(
    meta: &EpubMeta,
    manifest: &str,
    spine: &str,
    extra_meta: &str,
    cover_id: Option<&str>,
) -> String {
    let mut md = format!(
        "    <dc:identifier id=\"bookid\">{}</dc:identifier>\n    <dc:title>{}</dc:title>\n    <dc:language>{}</dc:language>\n    <meta property=\"dcterms:modified\">{}</meta>\n",
        uid(),
        escape(if meta.title.is_empty() { "Untitled" } else { &meta.title }),
        escape(if meta.language.is_empty() { "en" } else { &meta.language }),
        chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ")
    );
    for (i, a) in meta.authors.iter().enumerate() {
        md.push_str(&format!(
            "    <dc:creator id=\"a{i}\">{}</dc:creator>\n    <meta refines=\"#a{i}\" property=\"role\" scheme=\"marc:relators\">aut</meta>\n",
            escape(a)
        ));
    }
    if !meta.description.is_empty() {
        md.push_str(&format!(
            "    <dc:description>{}</dc:description>\n",
            escape(&meta.description)
        ));
    }
    if !meta.publisher.is_empty() {
        md.push_str(&format!(
            "    <dc:publisher>{}</dc:publisher>\n",
            escape(&meta.publisher)
        ));
    }
    for s in &meta.subjects {
        md.push_str(&format!("    <dc:subject>{}</dc:subject>\n", escape(s)));
    }
    if !meta.source_url.is_empty() {
        md.push_str(&format!(
            "    <dc:source>{}</dc:source>\n",
            escape(&meta.source_url)
        ));
    }
    if !meta.series.is_empty() {
        md.push_str(&format!(
            "    <meta property=\"belongs-to-collection\" id=\"series\">{}</meta>\n    <meta refines=\"#series\" property=\"collection-type\">series</meta>\n    <meta name=\"calibre:series\" content=\"{}\"/>\n",
            escape(&meta.series),
            escape(&meta.series)
        ));
        if let Some(i) = meta.series_index {
            md.push_str(&format!(
                "    <meta refines=\"#series\" property=\"group-position\">{i}</meta>\n    <meta name=\"calibre:series_index\" content=\"{i}\"/>\n"
            ));
        }
    }
    if let Some(c) = cover_id {
        md.push_str(&format!("    <meta name=\"cover\" content=\"{c}\"/>\n"));
    }
    md.push_str(extra_meta);
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<package xmlns=\"http://www.idpf.org/2007/opf\" version=\"3.0\" unique-identifier=\"bookid\" prefix=\"rendition: http://www.idpf.org/vocab/rendition/#\">\n  <metadata xmlns:dc=\"http://purl.org/dc/elements/1.1/\">\n{md}  </metadata>\n  <manifest>\n{manifest}  </manifest>\n  <spine toc=\"ncx\"{}>\n{spine}  </spine>\n</package>\n",
        if meta.rtl { " page-progression-direction=\"rtl\"" } else { "" }
    )
}

fn xhtml(title: &str, lang: &str, body: &str, head_extra: &str) -> String {
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE html>\n<html xmlns=\"http://www.w3.org/1999/xhtml\" xmlns:epub=\"http://www.idpf.org/2007/ops\" xml:lang=\"{lang}\" lang=\"{lang}\">\n<head>\n<meta charset=\"UTF-8\"/>\n<title>{}</title>\n{head_extra}</head>\n<body>\n{body}\n</body>\n</html>\n",
        escape(title)
    )
}

fn nav(titles: &[(String, String)], lang: &str) -> String {
    let items: String = titles
        .iter()
        .map(|(href, t)| format!("      <li><a href=\"{href}\">{}</a></li>\n", escape(t)))
        .collect();
    xhtml(
        "Contents",
        lang,
        &format!("<nav epub:type=\"toc\" id=\"toc\">\n  <h1>Contents</h1>\n  <ol>\n{items}  </ol>\n</nav>"),
        "",
    )
}

fn ncx(titles: &[(String, String)], title: &str) -> String {
    let points: String = titles
        .iter()
        .enumerate()
        .map(|(i, (href, t))| {
            format!(
                "    <navPoint id=\"p{i}\" playOrder=\"{}\"><navLabel><text>{}</text></navLabel><content src=\"{href}\"/></navPoint>\n",
                i + 1,
                escape(t)
            )
        })
        .collect();
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<ncx xmlns=\"http://www.daisy.org/z3986/2005/ncx/\" version=\"2005-1\">\n  <head><meta name=\"dtb:uid\" content=\"omniget\"/></head>\n  <docTitle><text>{}</text></docTitle>\n  <navMap>\n{points}  </navMap>\n</ncx>\n",
        escape(title)
    )
}

struct Writer {
    zip: zip::ZipWriter<std::fs::File>,
}

impl Writer {
    fn new(out: &Path) -> Result<Self, String> {
        if let Some(d) = out.parent() {
            std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
        }
        let f = std::fs::File::create(out).map_err(|e| e.to_string())?;
        let mut zip = zip::ZipWriter::new(f);
        let stored = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);
        zip.start_file("mimetype", stored)
            .map_err(|e| e.to_string())?;
        zip.write_all(b"application/epub+zip")
            .map_err(|e| e.to_string())?;
        let mut w = Self { zip };
        w.add(
            "META-INF/container.xml",
            b"<?xml version=\"1.0\"?>\n<container version=\"1.0\" xmlns=\"urn:oasis:names:tc:opendocument:xmlns:container\">\n  <rootfiles><rootfile full-path=\"OEBPS/content.opf\" media-type=\"application/oebps-package+xml\"/></rootfiles>\n</container>\n",
            true,
        )?;
        Ok(w)
    }

    fn add(&mut self, name: &str, data: &[u8], deflate: bool) -> Result<(), String> {
        let opts = zip::write::SimpleFileOptions::default().compression_method(if deflate {
            zip::CompressionMethod::Deflated
        } else {
            zip::CompressionMethod::Stored
        });
        self.zip.start_file(name, opts).map_err(|e| e.to_string())?;
        self.zip.write_all(data).map_err(|e| e.to_string())
    }

    fn finish(self) -> Result<(), String> {
        self.zip.finish().map(|_| ()).map_err(|e| e.to_string())
    }
}

fn image_ext_mime(data: &[u8]) -> (&'static str, &'static str) {
    if data.starts_with(b"\x89PNG") {
        ("png", "image/png")
    } else if data.get(8..12) == Some(b"WEBP") {
        ("webp", "image/webp")
    } else if data.starts_with(b"GIF8") {
        ("gif", "image/gif")
    } else {
        ("jpg", "image/jpeg")
    }
}

pub fn write_reflowable(
    out: &Path,
    meta: &EpubMeta,
    chapters: &[Chapter],
    cover: Option<&[u8]>,
) -> Result<(), String> {
    let tmp = out.with_extension("epub.part");
    let mut w = Writer::new(&tmp)?;
    let lang = if meta.language.is_empty() {
        "en"
    } else {
        &meta.language
    };
    let mut manifest = String::from(
        "    <item id=\"nav\" href=\"nav.xhtml\" media-type=\"application/xhtml+xml\" properties=\"nav\"/>\n    <item id=\"ncx\" href=\"toc.ncx\" media-type=\"application/x-dtbncx+xml\"/>\n    <item id=\"css\" href=\"style.css\" media-type=\"text/css\"/>\n",
    );
    let mut spine = String::new();
    let mut toc = Vec::new();
    let link = "<link rel=\"stylesheet\" type=\"text/css\" href=\"style.css\"/>\n";
    let mut cover_id = None;
    if let Some(c) = cover {
        let (ext, mime) = image_ext_mime(c);
        w.add(&format!("OEBPS/cover.{ext}"), c, false)?;
        manifest.push_str(&format!(
            "    <item id=\"cover-image\" href=\"cover.{ext}\" media-type=\"{mime}\" properties=\"cover-image\"/>\n    <item id=\"cover\" href=\"cover.xhtml\" media-type=\"application/xhtml+xml\"/>\n"
        ));
        w.add(
            "OEBPS/cover.xhtml",
            xhtml(
                &meta.title,
                lang,
                &format!("<div style=\"text-align:center\"><img src=\"cover.{ext}\" alt=\"Cover\" style=\"max-height:100%\"/></div>"),
                link,
            )
            .as_bytes(),
            true,
        )?;
        spine.push_str("    <itemref idref=\"cover\" linear=\"no\"/>\n");
        cover_id = Some("cover-image");
    }
    for (i, ch) in chapters.iter().enumerate() {
        let name = format!("ch{:04}.xhtml", i + 1);
        let body = if ch.body.contains("<h1") || ch.body.contains("<h2") || ch.title.is_empty() {
            ch.body.clone()
        } else {
            format!("<h2>{}</h2>\n{}", escape(&ch.title), ch.body)
        };
        w.add(
            &format!("OEBPS/{name}"),
            xhtml(&ch.title, lang, &body, link).as_bytes(),
            true,
        )?;
        manifest.push_str(&format!(
            "    <item id=\"c{i}\" href=\"{name}\" media-type=\"application/xhtml+xml\"/>\n"
        ));
        spine.push_str(&format!("    <itemref idref=\"c{i}\"/>\n"));
        toc.push((
            name,
            if ch.title.is_empty() {
                format!("{}", i + 1)
            } else {
                ch.title.clone()
            },
        ));
    }
    w.add("OEBPS/style.css", CSS.as_bytes(), true)?;
    w.add("OEBPS/nav.xhtml", nav(&toc, lang).as_bytes(), true)?;
    w.add("OEBPS/toc.ncx", ncx(&toc, &meta.title).as_bytes(), true)?;
    w.add(
        "OEBPS/content.opf",
        opf(meta, &manifest, &spine, "", cover_id).as_bytes(),
        true,
    )?;
    w.finish()?;
    std::fs::rename(&tmp, out).map_err(|e| e.to_string())
}

pub fn write_fixed_layout(
    out: &Path,
    meta: &EpubMeta,
    pages: &[(Vec<u8>, u32, u32)],
    chapter_marks: &[(usize, String)],
) -> Result<(), String> {
    if pages.is_empty() {
        return Err("no pages".into());
    }
    let tmp = out.with_extension("epub.part");
    let mut w = Writer::new(&tmp)?;
    let lang = if meta.language.is_empty() {
        "en"
    } else {
        &meta.language
    };
    let mut manifest = String::from(
        "    <item id=\"nav\" href=\"nav.xhtml\" media-type=\"application/xhtml+xml\" properties=\"nav\"/>\n    <item id=\"ncx\" href=\"toc.ncx\" media-type=\"application/x-dtbncx+xml\"/>\n",
    );
    let mut spine = String::new();
    let (vw, vh) = (pages[0].1, pages[0].2);
    for (i, (data, pw, ph)) in pages.iter().enumerate() {
        let (ext, mime) = image_ext_mime(data);
        let img = format!("img/{:04}.{ext}", i + 1);
        let page = format!("p{:04}.xhtml", i + 1);
        w.add(&format!("OEBPS/{img}"), data, false)?;
        let head = format!("<meta name=\"viewport\" content=\"width={pw}, height={ph}\"/>\n<style>html,body{{margin:0;padding:0;width:{pw}px;height:{ph}px}}img{{width:100%;height:100%;display:block}}</style>\n");
        w.add(
            &format!("OEBPS/{page}"),
            xhtml(
                &format!("{} {}", meta.title, i + 1),
                lang,
                &format!("<img src=\"{img}\" alt=\"\"/>"),
                &head,
            )
            .as_bytes(),
            true,
        )?;
        let props = if i == 0 {
            " properties=\"cover-image\""
        } else {
            ""
        };
        manifest.push_str(&format!(
            "    <item id=\"i{i}\" href=\"{img}\" media-type=\"{mime}\"{props}/>\n    <item id=\"p{i}\" href=\"{page}\" media-type=\"application/xhtml+xml\"/>\n"
        ));
        let side = if (i % 2 == 0) != meta.rtl {
            "page-spread-right"
        } else {
            "page-spread-left"
        };
        spine.push_str(&format!(
            "    <itemref idref=\"p{i}\" properties=\"{}\"/>\n",
            if i == 0 {
                "rendition:page-spread-center"
            } else {
                side
            }
        ));
    }
    let mut toc: Vec<(String, String)> = chapter_marks
        .iter()
        .map(|(i, t)| (format!("p{:04}.xhtml", i + 1), t.clone()))
        .collect();
    if toc.is_empty() {
        toc.push(("p0001.xhtml".into(), meta.title.clone()));
    }
    w.add("OEBPS/nav.xhtml", nav(&toc, lang).as_bytes(), true)?;
    w.add("OEBPS/toc.ncx", ncx(&toc, &meta.title).as_bytes(), true)?;
    let extra = format!(
        "    <meta property=\"rendition:layout\">pre-paginated</meta>\n    <meta property=\"rendition:spread\">landscape</meta>\n    <meta name=\"fixed-layout\" content=\"true\"/>\n    <meta name=\"original-resolution\" content=\"{vw}x{vh}\"/>\n    <meta name=\"book-type\" content=\"comic\"/>\n    <meta name=\"primary-writing-mode\" content=\"{}\"/>\n",
        if meta.rtl { "horizontal-rl" } else { "horizontal-lr" }
    );
    w.add(
        "OEBPS/content.opf",
        opf(meta, &manifest, &spine, &extra, Some("i0")).as_bytes(),
        true,
    )?;
    w.finish()?;
    std::fs::rename(&tmp, out).map_err(|e| e.to_string())
}

pub fn decode_text(bytes: &[u8]) -> String {
    if let Some((enc, bom)) = encoding_rs::Encoding::for_bom(bytes) {
        return enc
            .decode_without_bom_handling(&bytes[bom..])
            .0
            .into_owned();
    }
    if let Ok(s) = std::str::from_utf8(bytes) {
        return s.to_string();
    }
    let mut det = chardetng::EncodingDetector::new();
    det.feed(bytes, true);
    det.guess(None, true).decode(bytes).0.into_owned()
}

fn chapter_re() -> &'static regex::Regex {
    static RE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    RE.get_or_init(|| {
        regex::Regex::new(
            r"(?im)^[ \t]*((?:chapter|cap[ií]tulo|chapitre|kapitel|capitolo|глава|part|parte|book|livro|prologue|pr[oó]logo|epilogue|ep[ií]logo)\b[^\n]{0,80}|第[0-9一二三四五六七八九十百千零〇]+[章回節节][^\n]{0,40}|[IVXLC]{1,7}\.?)[ \t]*$",
        )
        .expect("static regex")
    })
}

fn paragraphs(text: &str) -> String {
    let mut out = String::new();
    let blocks: Vec<&str> = if text.contains("\n\n") {
        text.split("\n\n").collect()
    } else {
        text.lines().collect()
    };
    for b in blocks {
        let p = b.split_whitespace().collect::<Vec<_>>().join(" ");
        if !p.is_empty() {
            out.push_str(&format!("<p>{}</p>\n", escape(&p)));
        }
    }
    out
}

pub fn text_chapters(text: &str, fallback_title: &str) -> Vec<Chapter> {
    let text = text.replace("\r\n", "\n").replace('\r', "\n");
    let marks: Vec<(usize, usize, String)> = chapter_re()
        .captures_iter(&text)
        .filter_map(|c| {
            let m = c.get(0)?;
            Some((m.start(), m.end(), c.get(1)?.as_str().trim().to_string()))
        })
        .collect();
    if marks.len() < 2 {
        return vec![Chapter {
            title: fallback_title.to_string(),
            body: paragraphs(&text),
        }];
    }
    let mut out = Vec::new();
    let front = text[..marks[0].0].trim();
    if front.len() > 80 {
        out.push(Chapter {
            title: fallback_title.to_string(),
            body: paragraphs(front),
        });
    }
    for (i, (_, end, title)) in marks.iter().enumerate() {
        let stop = marks.get(i + 1).map(|m| m.0).unwrap_or(text.len());
        out.push(Chapter {
            title: title.clone(),
            body: paragraphs(&text[*end..stop]),
        });
    }
    out
}

pub fn markdown_chapters(md: &str, fallback_title: &str) -> Vec<Chapter> {
    use pulldown_cmark::{html, Options, Parser};
    let md = md
        .strip_prefix("---\n")
        .and_then(|r| r.split_once("\n---\n").map(|(_, b)| b))
        .unwrap_or(md);
    let mut parts: Vec<(String, String)> = Vec::new();
    let mut cur_title = fallback_title.to_string();
    let mut cur = String::new();
    let mut fence = false;
    for line in md.lines() {
        if line.trim_start().starts_with("```") {
            fence = !fence;
        }
        let heading = (!fence)
            .then(|| line.strip_prefix("# ").or_else(|| line.strip_prefix("## ")))
            .flatten();
        if let Some(h) = heading {
            if !cur.trim().is_empty() {
                parts.push((cur_title.clone(), cur.clone()));
            }
            cur_title = h.trim().to_string();
            cur = format!("{line}\n");
        } else {
            cur.push_str(line);
            cur.push('\n');
        }
    }
    if !cur.trim().is_empty() {
        parts.push((cur_title, cur));
    }
    let opts = Options::ENABLE_TABLES | Options::ENABLE_FOOTNOTES | Options::ENABLE_STRIKETHROUGH;
    parts
        .into_iter()
        .map(|(title, body)| {
            let mut out = String::new();
            html::push_html(&mut out, Parser::new_ext(&body, opts));
            Chapter {
                title,
                body: to_xhtml(&out),
            }
        })
        .collect()
}

pub fn to_xhtml(html: &str) -> String {
    static RES: std::sync::OnceLock<Vec<(regex::Regex, &'static str)>> = std::sync::OnceLock::new();
    let res = RES.get_or_init(|| {
        [
            (r"(?is)<script\b.*?</script\s*>", ""),
            (r"(?is)<style\b.*?</style\s*>", ""),
            (r"(?is)<iframe\b.*?</iframe\s*>", ""),
            (r"(?is)<object\b.*?</object\s*>", ""),
            (r"(?is)<embed\b.*?</embed\s*>", ""),
            (r"(?is)<form\b.*?</form\s*>", ""),
            (r"(?is)<noscript\b.*?</noscript\s*>", ""),
            (
                r"(?is)<(script|iframe|object|embed|link|meta|base)\b[^>]*/?>",
                "",
            ),
            (r#"(?i)\s+on[a-z]+\s*=\s*("[^"]*"|'[^']*'|[^\s>]+)"#, ""),
            (r#"(?i)(href|src)\s*=\s*"\s*javascript:[^"]*""#, "$1=\"#\""),
            (
                r"(?i)<(br|hr|img|input|col|source|wbr)\b([^>]*?)\s*/?>",
                "<$1$2/>",
            ),
            (r"&nbsp;", "&#160;"),
        ]
        .into_iter()
        .map(|(p, r)| (regex::Regex::new(p).expect("static regex"), r))
        .collect()
    });
    let mut s = html.to_string();
    for (re, rep) in res {
        s = re.replace_all(&s, *rep).to_string();
    }
    let amp = regex::Regex::new(r"&([^#a-zA-Z]|$)").expect("static regex");
    amp.replace_all(&s, "&amp;$1").to_string()
}

pub fn html_chapter(html: &str, fallback_title: &str) -> Chapter {
    let lower = html.to_lowercase();
    let body = match (lower.find("<body"), lower.rfind("</body>")) {
        (Some(a), Some(b)) if b > a => {
            let start = a + lower[a..].find('>').map(|x| x + 1).unwrap_or(0);
            &html[start..b]
        }
        _ => html,
    };
    let tags = regex::Regex::new(r"<[^>]+>").expect("static regex");
    let title = [
        r"(?is)<title[^>]*>(.*?)</title>",
        r"(?is)<h1[^>]*>(.*?)</h1>",
    ]
    .into_iter()
    .filter_map(|re| {
        regex::Regex::new(re)
            .expect("static regex")
            .captures(html)
            .map(|c| tags.replace_all(&c[1], "").trim().to_string())
    })
    .find(|t| !t.is_empty())
    .unwrap_or_else(|| fallback_title.to_string());
    Chapter {
        title: super::xml::unescape(&title),
        body: to_xhtml(body),
    }
}

pub fn article_from_html(html: &str, fallback_title: &str) -> Chapter {
    use scraper::{Html, Selector};
    let doc = Html::parse_document(html);
    let sel = |s: &str| Selector::parse(s).ok();
    let title = sel("meta[property='og:title']")
        .and_then(|s| doc.select(&s).next())
        .and_then(|e| e.value().attr("content").map(str::to_string))
        .or_else(|| {
            sel("h1")
                .and_then(|s| doc.select(&s).next())
                .map(|e| e.text().collect::<String>())
        })
        .or_else(|| {
            sel("title")
                .and_then(|s| doc.select(&s).next())
                .map(|e| e.text().collect::<String>())
        })
        .map(|t| t.split_whitespace().collect::<Vec<_>>().join(" "))
        .filter(|t| !t.is_empty())
        .unwrap_or_else(|| fallback_title.to_string());
    let body = [
        "article",
        "main",
        "[role=main]",
        ".post-content",
        ".entry-content",
        ".article-body",
    ]
    .iter()
    .filter_map(|q| sel(q))
    .find_map(|s| {
        doc.select(&s)
            .max_by_key(|e| e.text().map(str::len).sum::<usize>())
    })
    .map(|e| e.html())
    .or_else(|| {
        let p = sel("p")?;
        let mut best: std::collections::HashMap<String, (usize, String)> = Default::default();
        for el in doc.select(&p) {
            if let Some(parent) = el.parent().and_then(scraper::ElementRef::wrap) {
                let len: usize = el.text().map(str::len).sum();
                let entry = best
                    .entry(format!("{:?}", parent.id()))
                    .or_insert((0, parent.html()));
                entry.0 += len;
            }
        }
        best.into_values().max_by_key(|(n, _)| *n).map(|(_, h)| h)
    })
    .unwrap_or_default();
    let mut clean = body;
    for tag in ["nav", "aside", "footer", "header", "button", "svg", "form"] {
        let re = regex::Regex::new(&format!(r"(?is)<{tag}\b.*?</{tag}>")).expect("static regex");
        clean = re.replace_all(&clean, "").to_string();
    }
    Chapter {
        title: title.clone(),
        body: format!("<h1>{}</h1>\n{}", escape(&title), to_xhtml(&clean)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_text_chapters() {
        let t = "Some front matter that is long enough to keep as its own chapter, really it is quite long.\n\nChapter 1\n\nIt was a dark night.\nStill dark.\n\nChapter 2: Morning\n\nThe sun rose.";
        let ch = text_chapters(t, "Book");
        assert_eq!(ch.len(), 3);
        assert_eq!(ch[1].title, "Chapter 1");
        assert_eq!(ch[2].title, "Chapter 2: Morning");
        assert!(ch[1]
            .body
            .contains("<p>It was a dark night. Still dark.</p>"));
    }

    #[test]
    fn decodes_latin1() {
        let bytes = encoding_rs::WINDOWS_1252
            .encode("Capítulo único, ação e coração.")
            .0
            .into_owned();
        assert_eq!(decode_text(&bytes), "Capítulo único, ação e coração.");
    }

    #[test]
    fn extracts_articles() {
        let html = r#"<html><head><title>T</title><meta property="og:title" content="Big News"></head><body><nav>menu</nav><article><p>One two three.</p><p>Four.</p><script>x()</script></article><footer>f</footer></body></html>"#;
        let c = article_from_html(html, "x");
        assert_eq!(c.title, "Big News");
        assert!(c.body.contains("<p>One two three.</p>"));
        assert!(!c.body.contains("script") && !c.body.contains("menu"));
    }

    #[test]
    fn sanitizes_html() {
        let x = to_xhtml(
            "<p onclick=\"x()\">a<br>b & c</p><script>alert(1)</script><img src=\"a.png\">",
        );
        assert_eq!(x, "<p>a<br/>b &amp; c</p><img src=\"a.png\"/>");
    }

    #[test]
    fn titles_html_chapters() {
        let c = html_chapter(
            "<html><head><title>A &amp; B</title></head><body><p>x</p></body></html>",
            "f",
        );
        assert_eq!(c.title, "A & B");
        let c = html_chapter("<body><h1>The <em>Start</em></h1><p>x</p></body>", "f");
        assert_eq!(c.title, "The Start");
        assert_eq!(html_chapter("<p>x</p>", "Fallback").title, "Fallback");
    }

    #[test]
    fn writes_valid_epub() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("b.epub");
        let meta = EpubMeta {
            title: "Livro".into(),
            authors: vec!["Ana".into()],
            language: "pt".into(),
            series: "S".into(),
            series_index: Some(2.0),
            ..Default::default()
        };
        write_reflowable(
            &out,
            &meta,
            &markdown_chapters("# Um\n\ntexto *bom*\n\n# Dois\n\nmais", "x"),
            None,
        )
        .unwrap();
        let m = super::super::epub::read(std::fs::File::open(&out).unwrap()).unwrap();
        assert_eq!(m.title, "Livro");
        assert_eq!(m.series, "S");
        assert_eq!(m.series_index, Some(2.0));
        assert_eq!(m.toc.len(), 2);
        assert_eq!(m.toc[1].title, "Dois");
        let fl = dir.path().join("c.epub");
        write_fixed_layout(
            &fl,
            &EpubMeta {
                title: "C".into(),
                rtl: true,
                ..Default::default()
            },
            &[
                (vec![0xFF, 0xD8, 0xFF], 100, 150),
                (vec![0xFF, 0xD8, 0xFF], 100, 150),
            ],
            &[],
        )
        .unwrap();
        let m = super::super::epub::read(std::fs::File::open(&fl).unwrap()).unwrap();
        assert!(m.fixed_layout && m.rtl);
    }
}

use std::path::Path;

use pulldown_cmark::{html, Event, Options, Parser, Tag, TagEnd};
use scraper::{ElementRef, Html, Node, Selector};

use crate::core::read::epub_writer::{self, EpubMeta};
use crate::core::read::xml::escape;

const STYLE: &str = "body{max-width:46rem;margin:2rem auto;padding:0 1rem;font-family:system-ui,-apple-system,Segoe UI,Roboto,sans-serif;line-height:1.6;color:#1f2328}pre,code{font-family:ui-monospace,Menlo,Consolas,monospace;background:#f6f8fa;border-radius:4px}pre{padding:.8rem;overflow:auto}code{padding:.1rem .3rem}pre code{padding:0}table{border-collapse:collapse}th,td{border:1px solid #d0d7de;padding:.3rem .6rem}blockquote{margin:0;padding-left:1rem;border-left:3px solid #d0d7de;color:#59636e}img{max-width:100%}";

fn md_options() -> Options {
    Options::ENABLE_TABLES
        | Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_TASKLISTS
        | Options::ENABLE_FOOTNOTES
}

pub fn standalone_html(title: &str, body: &str) -> String {
    format!(
        "<!DOCTYPE html>\n<html>\n<head>\n<meta charset=\"utf-8\">\n<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n<title>{}</title>\n<style>{STYLE}</style>\n</head>\n<body>\n{body}</body>\n</html>\n",
        escape(title)
    )
}

fn first_heading(md: &str) -> Option<String> {
    let mut inside = false;
    let mut text = String::new();
    for ev in Parser::new_ext(md, md_options()) {
        match ev {
            Event::Start(Tag::Heading { .. }) => inside = true,
            Event::End(TagEnd::Heading(_)) => {
                if !text.trim().is_empty() {
                    return Some(text.trim().to_string());
                }
                inside = false;
            }
            Event::Text(t) | Event::Code(t) if inside => text.push_str(&t),
            _ => {}
        }
    }
    None
}

pub fn md_to_html(md: &str, fallback_title: &str) -> String {
    let mut body = String::new();
    html::push_html(&mut body, Parser::new_ext(md, md_options()));
    let title = first_heading(md).unwrap_or_else(|| fallback_title.to_string());
    standalone_html(&title, &body)
}

pub fn md_to_txt(md: &str) -> String {
    let mut out = String::new();
    let mut lists: Vec<Option<u64>> = Vec::new();
    for ev in Parser::new_ext(md, md_options()) {
        match ev {
            Event::Text(t) | Event::Code(t) => out.push_str(&t),
            Event::SoftBreak => out.push(' '),
            Event::HardBreak => out.push('\n'),
            Event::Rule => out.push_str("\n\n"),
            Event::Start(Tag::List(start)) => lists.push(start),
            Event::End(TagEnd::List(_)) => {
                lists.pop();
                if lists.is_empty() {
                    out.push('\n');
                }
            }
            Event::Start(Tag::Item) => {
                if !out.is_empty() && !out.ends_with('\n') {
                    out.push('\n');
                }
                out.push_str(&"  ".repeat(lists.len().saturating_sub(1)));
                match lists.last_mut() {
                    Some(Some(n)) => {
                        out.push_str(&format!("{n}. "));
                        *n += 1;
                    }
                    _ => out.push_str("- "),
                }
            }
            Event::End(TagEnd::Item) => {
                if !out.ends_with('\n') {
                    out.push('\n');
                }
            }
            Event::End(TagEnd::TableCell) => out.push('\t'),
            Event::End(TagEnd::TableRow) | Event::End(TagEnd::TableHead) => out.push('\n'),
            Event::End(TagEnd::Paragraph)
            | Event::End(TagEnd::Heading(_))
            | Event::End(TagEnd::CodeBlock)
            | Event::End(TagEnd::BlockQuote(_))
            | Event::End(TagEnd::Table) => {
                if lists.is_empty() {
                    out.push_str("\n\n");
                } else if !out.ends_with('\n') {
                    out.push('\n');
                }
            }
            _ => {}
        }
    }
    tidy(&out)
}

fn tidy(s: &str) -> String {
    let mut out = String::new();
    let mut blank = 0;
    for line in s.lines() {
        let line = line.trim_end();
        if line.is_empty() {
            blank += 1;
            if blank > 1 {
                continue;
            }
        } else {
            blank = 0;
        }
        out.push_str(line);
        out.push('\n');
    }
    let t = out.trim_matches('\n');
    format!("{t}\n")
}

pub fn escape_md(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        if matches!(
            c,
            '\\' | '`' | '*' | '_' | '[' | ']' | '<' | '>' | '#' | '|'
        ) {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

fn longest_run(s: &str, ch: char) -> usize {
    let mut best = 0;
    let mut cur = 0;
    for c in s.chars() {
        if c == ch {
            cur += 1;
            best = best.max(cur);
        } else {
            cur = 0;
        }
    }
    best
}

pub fn code_span(code: &str) -> String {
    let fence = "`".repeat(longest_run(code, '`') + 1);
    let pad = if code.starts_with('`') || code.ends_with('`') {
        " "
    } else {
        ""
    };
    format!("{fence}{pad}{code}{pad}{fence}")
}

pub fn escape_line_start(line: &str) -> String {
    if line.starts_with(['-', '+', '=', '~']) {
        return format!("\\{line}");
    }
    let digits = line.chars().take_while(char::is_ascii_digit).count();
    if digits > 0 {
        let rest = &line[digits..];
        if rest.starts_with(['.', ')']) && (rest.len() == 1 || rest[1..].starts_with(' ')) {
            return format!("{}\\{rest}", &line[..digits]);
        }
    }
    line.to_string()
}

pub fn txt_to_md(text: &str) -> String {
    let text = text.replace("\r\n", "\n");
    let mut out = String::new();
    for para in text.split("\n\n") {
        let lines: Vec<String> = para
            .lines()
            .map(|l| escape_md(l.trim_end()))
            .filter(|l| !l.is_empty())
            .collect();
        if lines.is_empty() {
            continue;
        }
        out.push_str(&lines.join("  \n"));
        out.push_str("\n\n");
    }
    tidy(&out)
}

pub fn txt_to_html(text: &str, title: &str) -> String {
    let text = text.replace("\r\n", "\n");
    let mut body = String::new();
    for para in text.split("\n\n") {
        let lines: Vec<String> = para
            .lines()
            .map(|l| escape(l.trim_end()))
            .filter(|l| !l.is_empty())
            .collect();
        if !lines.is_empty() {
            body.push_str(&format!("<p>{}</p>\n", lines.join("<br>\n")));
        }
    }
    standalone_html(title, &body)
}

const SKIP: &[&str] = &[
    "script", "style", "head", "noscript", "template", "svg", "iframe", "title",
];
const BLOCKS: &[&str] = &[
    "p",
    "div",
    "section",
    "article",
    "main",
    "header",
    "footer",
    "nav",
    "aside",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "ul",
    "ol",
    "li",
    "blockquote",
    "pre",
    "hr",
    "table",
    "figure",
    "figcaption",
    "dl",
    "dt",
    "dd",
    "body",
    "html",
    "form",
    "address",
];

struct Renderer {
    plain: bool,
}

fn collapse(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut space = false;
    for c in s.chars() {
        if c.is_whitespace() {
            space = true;
        } else {
            if space {
                out.push(' ');
            }
            space = false;
            out.push(c);
        }
    }
    if space {
        out.push(' ');
    }
    out
}

impl Renderer {
    fn inline(&self, el: ElementRef<'_>) -> String {
        let mut out = String::new();
        for child in el.children() {
            match child.value() {
                Node::Text(t) => {
                    let c = collapse(t);
                    if self.plain {
                        out.push_str(&c);
                    } else {
                        out.push_str(&escape_md(&c));
                    }
                }
                Node::Element(_) => {
                    if let Some(e) = ElementRef::wrap(child) {
                        out.push_str(&self.inline_element(e));
                    }
                }
                _ => {}
            }
        }
        out
    }

    fn inline_element(&self, el: ElementRef<'_>) -> String {
        let name = el.value().name();
        if SKIP.contains(&name) {
            return String::new();
        }
        let inner = || self.inline(el);
        let wrap = |mark: &str| {
            let s = inner();
            let t = s.trim();
            if self.plain || t.is_empty() {
                s
            } else {
                let lead = if s.starts_with(' ') { " " } else { "" };
                let trail = if s.ends_with(' ') { " " } else { "" };
                format!("{lead}{mark}{t}{mark}{trail}")
            }
        };
        match name {
            "br" => {
                if self.plain {
                    "\n".to_string()
                } else {
                    "  \n".to_string()
                }
            }
            "strong" | "b" => wrap("**"),
            "em" | "i" => wrap("*"),
            "del" | "s" | "strike" => wrap("~~"),
            "code" | "kbd" | "samp" => {
                let t: String = el.text().collect();
                if self.plain {
                    t
                } else {
                    code_span(&t)
                }
            }
            "a" => {
                let text = inner();
                match el.value().attr("href") {
                    Some(href) if !self.plain && !href.starts_with("javascript:") => {
                        format!("[{}]({})", text.trim(), href.replace(' ', "%20"))
                    }
                    _ => text,
                }
            }
            "img" => {
                let alt = el.value().attr("alt").unwrap_or("");
                match el.value().attr("src") {
                    Some(src) if !self.plain => format!("![{}]({})", escape_md(alt), src),
                    _ => alt.to_string(),
                }
            }
            _ if BLOCKS.contains(&name) => {
                let mut s = String::new();
                self.block(el, &mut s, 0);
                s.trim().replace('\n', " ")
            }
            _ => inner(),
        }
    }

    fn lead(&self, line: &str) -> String {
        if self.plain {
            line.to_string()
        } else {
            escape_line_start(line)
        }
    }

    fn flush(&self, buf: &mut String, out: &mut String) {
        let t = buf.trim();
        if !t.is_empty() {
            for line in t.lines() {
                out.push_str(&self.lead(line.trim_start()));
                out.push('\n');
            }
            out.push('\n');
        }
        buf.clear();
    }

    fn children_blocks(&self, el: ElementRef<'_>, out: &mut String, depth: usize) {
        let mut buf = String::new();
        for child in el.children() {
            match child.value() {
                Node::Text(t) => {
                    let c = collapse(t);
                    if self.plain {
                        buf.push_str(&c);
                    } else {
                        buf.push_str(&escape_md(&c));
                    }
                }
                Node::Element(e) => {
                    let Some(ce) = ElementRef::wrap(child) else {
                        continue;
                    };
                    if BLOCKS.contains(&e.name()) {
                        self.flush(&mut buf, out);
                        self.block(ce, out, depth);
                    } else {
                        buf.push_str(&self.inline_element(ce));
                    }
                }
                _ => {}
            }
        }
        self.flush(&mut buf, out);
    }

    fn list(&self, el: ElementRef<'_>, out: &mut String, depth: usize, ordered: bool) {
        let mut n: u64 = el
            .value()
            .attr("start")
            .and_then(|s| s.parse().ok())
            .unwrap_or(1);
        let indent = "  ".repeat(depth);
        for child in el.children().filter_map(ElementRef::wrap) {
            if child.value().name() != "li" {
                continue;
            }
            let marker = if ordered {
                let m = format!("{n}. ");
                n += 1;
                m
            } else {
                "- ".to_string()
            };
            let mut head = String::new();
            let mut nested = String::new();
            for c in child.children() {
                match c.value() {
                    Node::Text(t) => {
                        let s = collapse(t);
                        head.push_str(&if self.plain { s } else { escape_md(&s) });
                    }
                    Node::Element(e) => {
                        let Some(ce) = ElementRef::wrap(c) else {
                            continue;
                        };
                        match e.name() {
                            "ul" => self.list(ce, &mut nested, depth + 1, false),
                            "ol" => self.list(ce, &mut nested, depth + 1, true),
                            _ => head.push_str(&self.inline_element(ce)),
                        }
                    }
                    _ => {}
                }
            }
            out.push_str(&indent);
            out.push_str(&marker);
            out.push_str(&self.lead(head.trim()));
            out.push('\n');
            out.push_str(&nested);
        }
        if depth == 0 {
            out.push('\n');
        }
    }

    fn table(&self, el: ElementRef<'_>, out: &mut String) {
        let Ok(row_sel) = Selector::parse("tr") else {
            return;
        };
        let rows: Vec<Vec<String>> = el
            .select(&row_sel)
            .map(|tr| {
                tr.children()
                    .filter_map(ElementRef::wrap)
                    .filter(|c| matches!(c.value().name(), "td" | "th"))
                    .map(|c| self.inline(c).trim().to_string())
                    .collect()
            })
            .filter(|r: &Vec<String>| !r.is_empty())
            .collect();
        let Some(width) = rows.iter().map(Vec::len).max() else {
            return;
        };
        for (i, row) in rows.iter().enumerate() {
            let mut cells = row.clone();
            cells.resize(width, String::new());
            if self.plain {
                out.push_str(&cells.join("\t"));
            } else {
                out.push_str(&format!("| {} |", cells.join(" | ")));
            }
            out.push('\n');
            if i == 0 && !self.plain {
                out.push_str(&format!("|{}\n", " --- |".repeat(width)));
            }
        }
        out.push('\n');
    }

    fn block(&self, el: ElementRef<'_>, out: &mut String, depth: usize) {
        let name = el.value().name();
        if SKIP.contains(&name) {
            return;
        }
        match name {
            "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
                let level: usize = name[1..].parse().unwrap_or(1);
                let text = self.inline(el);
                let text = text.trim();
                if !text.is_empty() {
                    if !self.plain {
                        out.push_str(&"#".repeat(level));
                        out.push(' ');
                    }
                    out.push_str(text);
                    out.push_str("\n\n");
                }
            }
            "p" | "dt" | "dd" | "figcaption" | "address" => {
                let mut buf = self.inline(el);
                self.flush(&mut buf, out);
            }
            "ul" => self.list(el, out, depth, false),
            "ol" => self.list(el, out, depth, true),
            "li" => {
                let text = self.lead(self.inline(el).trim());
                out.push_str(&format!("- {text}\n\n"));
            }
            "hr" => out.push_str(if self.plain { "\n" } else { "---\n\n" }),
            "pre" => {
                let code: String = el.text().collect();
                if self.plain {
                    out.push_str(code.trim_end());
                    out.push_str("\n\n");
                } else {
                    let lang = el
                        .children()
                        .filter_map(ElementRef::wrap)
                        .find(|c| c.value().name() == "code")
                        .and_then(|c| c.value().attr("class"))
                        .and_then(|cls| {
                            cls.split_whitespace()
                                .find_map(|c| c.strip_prefix("language-"))
                        })
                        .unwrap_or("");
                    let fence = "`".repeat(longest_run(&code, '`').max(2) + 1);
                    out.push_str(&format!("{fence}{lang}\n{}\n{fence}\n\n", code.trim_end()));
                }
            }
            "blockquote" => {
                let mut inner = String::new();
                self.children_blocks(el, &mut inner, depth);
                let prefix = if self.plain { "  " } else { "> " };
                for line in inner.trim().lines() {
                    out.push_str(prefix.trim_end());
                    if !line.is_empty() {
                        out.push_str(&prefix[prefix.trim_end().len()..]);
                        out.push_str(line);
                    }
                    out.push('\n');
                }
                out.push('\n');
            }
            "table" => self.table(el, out),
            _ => self.children_blocks(el, out, depth),
        }
    }
}

fn render_html(source: &str, plain: bool) -> String {
    let doc = Html::parse_document(source);
    let r = Renderer { plain };
    let mut out = String::new();
    let body = Selector::parse("body")
        .ok()
        .and_then(|s| doc.select(&s).next());
    match body {
        Some(b) => r.block(b, &mut out, 0),
        None => r.block(doc.root_element(), &mut out, 0),
    }
    tidy(&out)
}

pub fn html_to_md(html: &str) -> String {
    render_html(html, false)
}

pub fn html_to_txt(html: &str) -> String {
    render_html(html, true)
}

fn read_text(path: &Path) -> Result<String, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("Could not read the file: {e}."))?;
    Ok(epub_writer::decode_text(&bytes))
}

fn stem(path: &Path) -> String {
    path.file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("Document")
        .to_string()
}

fn epub_text(path: &Path, markdown: bool) -> Result<String, String> {
    use crate::core::read::{epub, xml};
    let file = std::fs::File::open(path).map_err(|e| format!("Could not read the file: {e}."))?;
    let unreadable =
        || "This EPUB could not be read: it has no table of contents (OPF).".to_string();
    let mut zip = zip::ZipArchive::new(std::io::BufReader::new(file))
        .map_err(|e| format!("This EPUB could not be read: {e}."))?;
    let opf = epub::opf_path(&mut zip).ok_or_else(unreadable)?;
    let opf_text = epub::read_entry_bytes(&mut zip, &opf)
        .map(|b| String::from_utf8_lossy(&b).into_owned())
        .ok_or_else(unreadable)?;
    let root = xml::parse(&opf_text);
    let base = opf
        .rsplit_once('/')
        .map(|(d, _)| d)
        .unwrap_or("")
        .to_string();
    let manifest = root.find("manifest").cloned().unwrap_or_default();
    let spine = root.find("spine").cloned().unwrap_or_default();
    let mut out = String::new();
    for item in spine.children_named("itemref") {
        let Some(href) = item.attr("idref").and_then(|id| {
            manifest
                .children_named("item")
                .find(|i| i.attr("id") == Some(id))
                .and_then(|i| i.attr("href"))
        }) else {
            continue;
        };
        let Some(bytes) = epub::read_entry_bytes(&mut zip, &epub::resolve(&base, href)) else {
            continue;
        };
        let html = String::from_utf8_lossy(&bytes);
        let part = if markdown {
            html_to_md(&html)
        } else {
            html_to_txt(&html)
        };
        if !part.trim().is_empty() {
            out.push_str(part.trim());
            out.push_str("\n\n");
        }
    }
    if out.trim().is_empty() {
        return Err(
            "This EPUB has no readable text. It may contain only images or be protected (DRM)."
                .to_string(),
        );
    }
    Ok(tidy(&out))
}

pub fn convert_file(input: &Path, from: &str, output: &Path, to: &str) -> Result<(), String> {
    let title = stem(input);
    if to == "epub" {
        let chapters = match from {
            "md" => epub_writer::markdown_chapters(&read_text(input)?, &title),
            "html" => vec![epub_writer::html_chapter(&read_text(input)?, &title)],
            "txt" => epub_writer::text_chapters(&read_text(input)?, &title),
            _ => return Err(format!("OmniGet can't turn {from} into EPUB by itself.")),
        };
        let meta = EpubMeta {
            title,
            ..Default::default()
        };
        return epub_writer::write_reflowable(output, &meta, &chapters, None);
    }
    let text = match (from, to) {
        ("epub", "txt") => epub_text(input, false)?,
        ("epub", "md") => epub_text(input, true)?,
        ("md", "html") => md_to_html(&read_text(input)?, &title),
        ("md", "txt") => md_to_txt(&read_text(input)?),
        ("html", "md") => html_to_md(&read_text(input)?),
        ("html", "txt") => html_to_txt(&read_text(input)?),
        ("txt", "md") => txt_to_md(&read_text(input)?),
        ("txt", "html") => txt_to_html(&read_text(input)?, &title),
        _ => return Err(format!("OmniGet can't convert {from} to {to} by itself.")),
    };
    std::fs::write(output, text).map_err(|e| format!("Could not write the output: {e}."))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn md_to_html_is_standalone() {
        let html = md_to_html("# Title\n\nHello **world**", "fallback");
        assert!(html.contains("<meta charset=\"utf-8\">"));
        assert!(html.contains("<title>Title</title>"));
        assert!(html.contains("<strong>world</strong>"));
    }

    #[test]
    fn html_to_md_basic() {
        let html = r#"<html><head><style>x{}</style></head><body><h2>Hi</h2><p>Some <strong>bold</strong> and <a href="https://x.y">link</a>.</p><ul><li>one</li><li>two<ul><li>deep</li></ul></li></ul><pre><code class="language-rs">fn main() {}</code></pre><table><tr><th>a</th><th>b</th></tr><tr><td>1</td><td>2</td></tr></table><blockquote><p>quote</p></blockquote></body></html>"#;
        let md = html_to_md(html);
        assert!(md.contains("## Hi"));
        assert!(
            md.contains("Some **bold** and [link](https://x.y)."),
            "{md}"
        );
        assert!(md.contains("- one\n- two\n  - deep"));
        assert!(md.contains("```rs\nfn main() {}\n```"));
        assert!(md.contains("| a | b |\n| --- | --- |\n| 1 | 2 |"));
        assert!(md.contains("> quote"));
        assert!(!md.contains("x{}"));
    }

    #[test]
    fn html_to_md_escapes_line_starts_pipes_and_backticks() {
        let html = "<p>1. not a list</p><p>- not a bullet</p><p>+ plus</p><table><tr><th>h</th></tr><tr><td>a|b</td></tr></table><p>run <code>a`b</code> now</p><pre><code>```inner```</code></pre>";
        let md = html_to_md(html);
        assert!(md.contains("1\\. not a list"), "{md}");
        assert!(md.contains("\\- not a bullet"), "{md}");
        assert!(md.contains("\\+ plus"), "{md}");
        assert!(md.contains("| a\\|b |"), "{md}");
        assert!(!md.contains("\\\\|"), "{md}");
        assert!(md.contains("``a`b``"), "{md}");
        assert!(md.contains("````\n```inner```\n````"), "{md}");
        let html_back = md_to_html(&md, "t");
        assert!(html_back.contains("<p>1. not a list</p>"), "{html_back}");
        assert!(html_back.contains("<code>a`b</code>"), "{html_back}");
        assert!(html_back.contains("<td>a|b</td>"), "{html_back}");
    }

    #[test]
    fn md_html_md_round_trip_keeps_structure() {
        let src = "# Title\n\nA paragraph with *em*.\n\n- a\n- b\n";
        let html = md_to_html(src, "t");
        let back = html_to_md(&html);
        assert!(back.contains("# Title"));
        assert!(back.contains("A paragraph with *em*."));
        assert!(back.contains("- a\n- b"));
    }

    #[test]
    fn text_conversions() {
        assert_eq!(
            md_to_txt("# T\n\n**b** `c`\n\n1. x\n2. y\n"),
            "T\n\nb c\n\n1. x\n2. y\n"
        );
        assert!(txt_to_html("a < b\n\nnext", "t").contains("<p>a &lt; b</p>"));
        assert_eq!(txt_to_md("a*b\n\nc"), "a\\*b\n\nc\n");
        assert_eq!(
            html_to_txt("<p>Hello<br>there</p><p>x</p>"),
            "Hello\nthere\n\nx\n"
        );
    }

    #[test]
    fn md_to_epub_and_back() {
        let dir = tempfile::tempdir().unwrap();
        let md = dir.path().join("book.md");
        std::fs::write(&md, "# One\n\nFirst chapter.\n\n# Two\n\nSecond chapter.\n").unwrap();
        let epub = dir.path().join("book.epub");
        convert_file(&md, "md", &epub, "epub").unwrap();
        let txt = dir.path().join("book.txt");
        convert_file(&epub, "epub", &txt, "txt").unwrap();
        let text = std::fs::read_to_string(&txt).unwrap();
        assert!(text.contains("First chapter."));
        assert!(text.contains("Second chapter."));
    }
}

use std::io::{Read, Write};
use std::path::Path;

const BLOCKS: &[&str] = &[
    "p",
    "div",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "li",
    "blockquote",
    "td",
    "th",
    "dd",
    "dt",
    "figcaption",
    "caption",
    "pre",
    "section",
    "article",
    "aside",
];

const SKIP: &[&str] = &["script", "style", "svg", "math", "head", "title"];

const STYLE: &str = "<style type=\"text/css\" class=\"kobostylehacks\">div#book-inner { margin-top: 0; margin-bottom: 0; }</style>";

fn is_content(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    n.ends_with(".xhtml") || n.ends_with(".html") || n.ends_with(".htm")
}

fn sentences(text: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut start = 0;
    let mut chars = text.char_indices().peekable();
    let mut after_stop = false;
    while let Some((i, c)) = chars.next() {
        if matches!(c, '.' | '!' | '?' | '…' | '。' | '！' | '？') {
            after_stop = true;
            continue;
        }
        if after_stop && matches!(c, '"' | '\'' | '”' | '’' | ')' | '»') {
            continue;
        }
        if after_stop && c.is_whitespace() {
            let mut end = i + c.len_utf8();
            while let Some(&(j, w)) = chars.peek() {
                if !w.is_whitespace() {
                    break;
                }
                end = j + w.len_utf8();
                chars.next();
            }
            out.push(&text[start..end]);
            start = end;
        }
        after_stop = false;
    }
    if start < text.len() {
        out.push(&text[start..]);
    }
    out
}

fn tag_name(tag: &str) -> (bool, String) {
    let t = tag.trim_start_matches('<');
    let closing = t.starts_with('/');
    let name: String = t
        .trim_start_matches('/')
        .chars()
        .take_while(|c| c.is_ascii_alphanumeric() || *c == ':' || *c == '-')
        .collect();
    (closing, name.to_ascii_lowercase())
}

pub fn kepubify_html(doc: &str) -> String {
    if doc.contains("koboSpan") {
        return doc.to_string();
    }
    let lower = doc.to_ascii_lowercase();
    let Some(body_open) = lower.find("<body") else {
        return doc.to_string();
    };
    let Some(body_start) = lower[body_open..].find('>').map(|i| body_open + i + 1) else {
        return doc.to_string();
    };
    let Some(body_end) = lower.rfind("</body>").filter(|e| *e >= body_start) else {
        return doc.to_string();
    };

    let body = &doc[body_start..body_end];
    let mut out = String::with_capacity(body.len() * 2);
    let (mut para, mut seg) = (0u32, 0u32);
    let mut skip = 0usize;
    let mut rest = body;
    while !rest.is_empty() {
        if rest.starts_with("<!--") {
            let end = rest.find("-->").map(|i| i + 3).unwrap_or(rest.len());
            out.push_str(&rest[..end]);
            rest = &rest[end..];
        } else if rest.starts_with("<![CDATA[") {
            let end = rest.find("]]>").map(|i| i + 3).unwrap_or(rest.len());
            out.push_str(&rest[..end]);
            rest = &rest[end..];
        } else if rest.starts_with('<') {
            let end = rest.find('>').map(|i| i + 1).unwrap_or(rest.len());
            let tag = &rest[..end];
            let self_closing = tag.ends_with("/>");
            let (closing, name) = tag_name(tag);
            if SKIP.contains(&name.as_str()) && !self_closing {
                if closing {
                    skip = skip.saturating_sub(1);
                } else {
                    skip += 1;
                }
            } else if !closing && BLOCKS.contains(&name.as_str()) {
                para += 1;
                seg = 0;
            }
            out.push_str(tag);
            rest = &rest[end..];
        } else {
            let end = rest.find('<').unwrap_or(rest.len());
            let text = &rest[..end];
            if skip > 0 || text.trim().is_empty() {
                out.push_str(text);
            } else {
                let lead = text.len() - text.trim_start().len();
                out.push_str(&text[..lead]);
                for s in sentences(&text[lead..]) {
                    para = para.max(1);
                    seg += 1;
                    out.push_str(&format!(
                        "<span class=\"koboSpan\" id=\"kobo.{para}.{seg}\">{s}</span>"
                    ));
                }
            }
            rest = &rest[end..];
        }
    }

    let head = &doc[..body_start];
    let head = match head.to_ascii_lowercase().rfind("</head>") {
        Some(i) => format!("{}{STYLE}{}", &head[..i], &head[i..]),
        None => head.to_string(),
    };
    format!(
        "{head}<div id=\"book-columns\"><div id=\"book-inner\">{out}</div></div>{}",
        &doc[body_end..]
    )
}

pub fn convert(input: &Path, out: &Path) -> Result<(), String> {
    let file = std::fs::File::open(input).map_err(|e| e.to_string())?;
    let mut zin = zip::ZipArchive::new(file).map_err(|e| e.to_string())?;
    let tmp = out.with_extension("part");
    let res = (|| -> Result<(), String> {
        let f = std::fs::File::create(&tmp).map_err(|e| e.to_string())?;
        let mut zout = zip::ZipWriter::new(f);
        for i in 0..zin.len() {
            let mut entry = zin.by_index(i).map_err(|e| e.to_string())?;
            if entry.is_dir() || !is_content(entry.name()) {
                zout.raw_copy_file(entry).map_err(|e| e.to_string())?;
                continue;
            }
            let name = entry.name().to_string();
            let mut bytes = Vec::new();
            entry.read_to_end(&mut bytes).map_err(|e| e.to_string())?;
            let text = String::from_utf8_lossy(&bytes);
            let opts = zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Deflated);
            zout.start_file(name, opts).map_err(|e| e.to_string())?;
            zout.write_all(kepubify_html(&text).as_bytes())
                .map_err(|e| e.to_string())?;
        }
        zout.finish().map_err(|e| e.to_string())?;
        Ok(())
    })();
    match res {
        Ok(()) => std::fs::rename(&tmp, out).map_err(|e| e.to_string()),
        Err(e) => {
            let _ = std::fs::remove_file(&tmp);
            Err(e)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_sentences() {
        assert_eq!(
            sentences("One. Two! \"Three?\" Four"),
            vec!["One. ", "Two! ", "\"Three?\" ", "Four"]
        );
        assert_eq!(sentences("3.14 is pi"), vec!["3.14 is pi"]);
    }

    #[test]
    fn spans_and_wraps() {
        let doc = "<html><head><title>T</title></head><body class=\"x\"><h1>Title</h1>\n<p>A b. <em>C</em> d.</p><script>var a = 1;</script><p> </p></body></html>";
        let k = kepubify_html(doc);
        assert!(k.contains("<title>T</title><style"), "{k}");
        assert!(k.contains("<body class=\"x\"><div id=\"book-columns\"><div id=\"book-inner\"><h1><span class=\"koboSpan\" id=\"kobo.1.1\">Title</span></h1>"), "{k}");
        assert!(k.contains("<p><span class=\"koboSpan\" id=\"kobo.2.1\">A b. </span><em><span class=\"koboSpan\" id=\"kobo.2.2\">C</span></em> <span class=\"koboSpan\" id=\"kobo.2.3\">d.</span></p>"), "{k}");
        assert!(k.contains("<script>var a = 1;</script>"));
        assert!(k.ends_with("</div></div></body></html>"));
        assert_eq!(kepubify_html(&k), k);
    }

    #[test]
    fn converts_an_epub() {
        let dir = std::env::temp_dir().join(format!("kepub-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let src = dir.join("a.epub");
        {
            let mut z = zip::ZipWriter::new(std::fs::File::create(&src).unwrap());
            let stored = zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Stored);
            z.start_file("mimetype", stored).unwrap();
            z.write_all(b"application/epub+zip").unwrap();
            z.start_file("OEBPS/c.xhtml", zip::write::SimpleFileOptions::default())
                .unwrap();
            z.write_all(b"<html><head></head><body><p>Hi.</p></body></html>")
                .unwrap();
            z.finish().unwrap();
        }
        let out = dir.join("a.kepub.epub");
        convert(&src, &out).unwrap();
        let mut z = zip::ZipArchive::new(std::fs::File::open(&out).unwrap()).unwrap();
        assert_eq!(z.by_index(0).unwrap().name(), "mimetype");
        assert_eq!(
            z.by_index(0).unwrap().compression(),
            zip::CompressionMethod::Stored
        );
        let mut s = String::new();
        z.by_name("OEBPS/c.xhtml")
            .unwrap()
            .read_to_string(&mut s)
            .unwrap();
        assert!(s.contains("kobo.1.1") && s.contains("book-inner"));
        let _ = std::fs::remove_dir_all(dir);
    }
}

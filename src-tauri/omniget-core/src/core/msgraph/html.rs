//! Teams message bodies are HTML. For an export an LLM reads, plain text with
//! line breaks, list bullets, links and mentions kept is what matters.

/// HTML → readable text: block tags become line breaks, `<li>` a bullet,
/// `<a href>` keeps its URL when the text differs, entities are decoded,
/// `<attachment>` and images leave a marker.
pub fn to_text(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut chars = html.char_indices().peekable();
    let mut href: Option<String> = None;
    let mut link_text = String::new();
    while let Some((i, c)) = chars.next() {
        if c != '<' {
            if href.is_some() {
                link_text.push(c);
            } else {
                out.push(c);
            }
            continue;
        }
        let end = match html[i..].find('>') {
            Some(e) => i + e,
            None => {
                out.push_str(&html[i..]);
                break;
            }
        };
        let tag = &html[i + 1..end];
        while chars.peek().is_some_and(|(j, _)| *j <= end) {
            chars.next();
        }
        let lower = tag.trim().to_ascii_lowercase();
        let name = lower
            .trim_start_matches('/')
            .split(|c: char| c.is_whitespace() || c == '/')
            .next()
            .unwrap_or("");
        let closing = lower.starts_with('/');
        match name {
            "br" => out.push('\n'),
            "p" | "div" | "blockquote" | "tr" | "h1" | "h2" | "h3" | "h4" | "pre" => {
                if !out.ends_with('\n') && !out.is_empty() {
                    out.push('\n');
                }
            }
            "li" if !closing => {
                if !out.ends_with('\n') && !out.is_empty() {
                    out.push('\n');
                }
                out.push_str("- ");
            }
            "td" | "th" if closing => out.push_str(" | "),
            "a" if !closing => {
                href = attr(tag, "href");
                link_text.clear();
            }
            "a" => {
                let text = decode(link_text.trim());
                match href.take() {
                    Some(h) if !h.is_empty() && h != text => out.push_str(&format!("{text} ({h})")),
                    _ => out.push_str(&text),
                }
                link_text.clear();
            }
            "img" => {
                let alt = attr(tag, "alt").unwrap_or_default();
                out.push_str(&format!(
                    "[image{}]",
                    if alt.is_empty() {
                        String::new()
                    } else {
                        format!(": {alt}")
                    }
                ));
            }
            "attachment" if !closing => out.push_str("[attachment]"),
            _ => {}
        }
    }
    let text = decode(&out);
    // Collapse runs of blank lines and trailing spaces.
    let mut lines: Vec<String> = Vec::new();
    for l in text.lines().map(|l| l.trim_end().to_string()) {
        if l.trim().is_empty() && lines.last().is_some_and(|p: &String| p.trim().is_empty()) {
            continue;
        }
        lines.push(l);
    }
    lines.join("\n").trim().to_string()
}

fn attr(tag: &str, name: &str) -> Option<String> {
    let lower = tag.to_ascii_lowercase();
    let key = format!("{name}=");
    let pos = lower.find(&key)? + key.len();
    let rest = &tag[pos..];
    let (quote, body) = match rest.chars().next()? {
        q @ ('"' | '\'') => (q, &rest[1..]),
        _ => (' ', rest),
    };
    let end = body.find(quote).unwrap_or(body.len());
    Some(decode(&body[..end]))
}

pub fn decode(s: &str) -> String {
    if !s.contains('&') {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        let tail = &rest[i..];
        let Some(semi) = tail[..tail.len().min(12)].find(';') else {
            out.push('&');
            rest = &tail[1..];
            continue;
        };
        let ent = &tail[1..semi];
        let ch = match ent {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            "nbsp" => Some(' '),
            _ if ent.starts_with("#x") || ent.starts_with("#X") => {
                u32::from_str_radix(&ent[2..], 16)
                    .ok()
                    .and_then(char::from_u32)
            }
            _ if ent.starts_with('#') => ent[1..].parse::<u32>().ok().and_then(char::from_u32),
            _ => None,
        };
        match ch {
            Some(c) => {
                out.push(c);
                rest = &tail[semi + 1..];
            }
            None => {
                out.push('&');
                rest = &tail[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_teams_html() {
        let h = r#"<p>Oi <at id="0">Ana</at>,&nbsp;veja:</p><ul><li>item &amp; 1</li><li>dois</li></ul><p><a href="https://x.com/a">https://x.com/a</a> e <a href="https://y.com">doc</a></p><attachment id="abc"></attachment><img alt="print" src="z">"#;
        let t = to_text(h);
        assert_eq!(
            t,
            "Oi Ana, veja:\n- item & 1\n- dois\nhttps://x.com/a e doc (https://y.com)\n[attachment][image: print]"
        );
    }

    #[test]
    fn decodes_numeric_entities_and_keeps_stray_ampersand() {
        assert_eq!(decode("a &#233; b &#x1F600; c & d"), "a é b 😀 c & d");
        assert_eq!(
            to_text("line1<br>line2<br/><br/><br/>line3"),
            "line1\nline2\n\nline3"
        );
    }
}

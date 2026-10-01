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

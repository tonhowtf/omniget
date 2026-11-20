use quick_xml::events::Event;
use quick_xml::Reader;

#[derive(Debug, Clone, Default)]
pub struct Node {
    pub name: String,
    pub prefix: String,
    pub attrs: Vec<(String, String)>,
    pub children: Vec<Node>,
    pub text: String,
}

fn split_name(raw: &str) -> (String, String) {
    match raw.split_once(':') {
        Some((p, l)) => (p.to_string(), l.to_string()),
        None => (String::new(), raw.to_string()),
    }
}

fn node_from(e: &quick_xml::events::BytesStart<'_>) -> Node {
    let (prefix, name) = split_name(e.name().as_ref());
    let attrs = e
        .attributes()
        .with_checks(false)
        .filter_map(|a| a.ok())
        .map(|a| {
            let key = a.key.as_ref().to_string();
            let val = unescape(&a.value);
            (key, val)
        })
        .collect();
    Node {
        name,
        prefix,
        attrs,
        ..Default::default()
    }
}

pub fn parse(xml: &str) -> Node {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(false);
    reader.config_mut().check_end_names = false;
    let mut stack: Vec<Node> = vec![Node::default()];
    loop {
        match reader.read_event() {
            Ok(Event::Start(e)) => stack.push(node_from(&e)),
            Ok(Event::Empty(e)) => {
                let n = node_from(&e);
                if let Some(top) = stack.last_mut() {
                    top.children.push(n);
                }
            }
            Ok(Event::End(_)) => {
                if stack.len() > 1 {
                    let mut n = stack.pop().unwrap_or_default();
                    n.text = n.text.trim().to_string();
                    if let Some(top) = stack.last_mut() {
                        top.children.push(n);
                    }
                }
            }
            Ok(Event::Text(t)) => {
                if let Some(top) = stack.last_mut() {
                    top.text.push_str(&unescape(&t));
                }
            }
            Ok(Event::GeneralRef(r)) => {
                if let Some(top) = stack.last_mut() {
                    top.text.push_str(&unescape(&format!("&{};", &*r)));
                }
            }
            Ok(Event::CData(t)) => {
                if let Some(top) = stack.last_mut() {
                    top.text.push_str(&t);
                }
            }
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
    }
    while stack.len() > 1 {
        let n = stack.pop().unwrap_or_default();
        if let Some(top) = stack.last_mut() {
            top.children.push(n);
        }
    }
    stack.pop().unwrap_or_default()
}

pub fn unescape(s: &str) -> String {
    if !s.contains('&') {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        let tail = &rest[i..];
        let Some(end) = tail.find(';').filter(|&e| e <= 12) else {
            out.push('&');
            rest = &tail[1..];
            continue;
        };
        let ent = &tail[1..end];
        let rep = match ent {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            "nbsp" => Some('\u{a0}'),
            _ if ent.starts_with("#x") || ent.starts_with("#X") => {
                u32::from_str_radix(&ent[2..], 16)
                    .ok()
                    .and_then(char::from_u32)
            }
            _ if ent.starts_with('#') => ent[1..].parse().ok().and_then(char::from_u32),
            _ => None,
        };
        match rep {
            Some(c) => out.push(c),
            None => out.push_str(&tail[..=end]),
        }
        rest = &tail[end + 1..];
    }
    out.push_str(rest);
    out
}

pub fn escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

impl Node {
    pub fn attr(&self, key: &str) -> Option<&str> {
        self.attrs
            .iter()
            .find(|(k, _)| k == key || k.rsplit(':').next() == Some(key))
            .map(|(_, v)| v.as_str())
    }

    pub fn child(&self, name: &str) -> Option<&Node> {
        self.children.iter().find(|c| c.name == name)
    }

    pub fn children_named<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a Node> + 'a {
        self.children.iter().filter(move |c| c.name == name)
    }

    pub fn find(&self, name: &str) -> Option<&Node> {
        for c in &self.children {
            if c.name == name {
                return Some(c);
            }
            if let Some(n) = c.find(name) {
                return Some(n);
            }
        }
        None
    }

    pub fn find_all<'a>(&'a self, name: &str, out: &mut Vec<&'a Node>) {
        for c in &self.children {
            if c.name == name {
                out.push(c);
            }
            c.find_all(name, out);
        }
    }

    pub fn child_text(&self, name: &str) -> Option<String> {
        self.child(name)
            .map(|c| c.all_text())
            .filter(|t| !t.is_empty())
    }

    pub fn all_text(&self) -> String {
        let mut s = String::new();
        self.collect_text(&mut s);
        s.split_whitespace().collect::<Vec<_>>().join(" ")
    }

    fn collect_text(&self, out: &mut String) {
        out.push_str(&self.text);
        out.push(' ');
        for c in &self.children {
            c.collect_text(out);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_prefixed_names_and_entities() {
        let root = parse(
            r#"<?xml version="1.0"?><package><metadata><dc:title>A &amp; B &#233;</dc:title><meta name="x" content="y"/></metadata></package>"#,
        );
        let pkg = root.child("package").unwrap();
        let md = pkg.child("metadata").unwrap();
        assert_eq!(md.child_text("title").unwrap(), "A & B é");
        assert_eq!(md.child("meta").unwrap().attr("content"), Some("y"));
    }

    #[test]
    fn tolerates_unclosed_tail() {
        let root = parse("<a><b>hi</b><c>");
        assert!(root.find("b").is_some());
    }
}

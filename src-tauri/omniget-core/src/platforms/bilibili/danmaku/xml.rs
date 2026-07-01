use super::proto::DanmakuElem;

pub fn render(elems: &[DanmakuElem]) -> String {
    let mut out = String::new();
    out.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    out.push_str("<i>\n");
    out.push_str("  <chatserver>chat.bilibili.com</chatserver>\n");
    out.push_str("  <chatid>0</chatid>\n");
    out.push_str("  <mission>0</mission>\n");
    out.push_str("  <maxlimit>1500</maxlimit>\n");
    out.push_str("  <state>0</state>\n");
    out.push_str("  <real_name>0</real_name>\n");
    out.push_str("  <source>k-v</source>\n");
    for e in elems {
        let stime = (e.progress_ms as f64) / 1000.0;
        let mode = e.mode.max(1);
        let size = if e.fontsize > 0 { e.fontsize } else { 25 };
        let color = e.color;
        let date = e.ctime;
        let pool = e.pool;
        let mid_hash = escape_xml_attr(&e.mid_hash);
        let dmid = e.id;
        let text = escape_xml_text(&e.content);
        out.push_str(&format!(
            "  <d p=\"{:.3},{},{},{},{},{},{},{}\">{}</d>\n",
            stime, mode, size, color, date, pool, mid_hash, dmid, text
        ));
    }
    out.push_str("</i>\n");
    out
}

fn escape_xml_text(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            _ => out.push(c),
        }
    }
    out
}

fn escape_xml_attr(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            _ => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_minimal_xml() {
        let elem = DanmakuElem {
            id: 1,
            progress_ms: 1500,
            mode: 1,
            fontsize: 25,
            color: 0xFFFFFF,
            content: "hello & world".to_string(),
            ..DanmakuElem::default()
        };
        let xml = render(&[elem]);
        assert!(xml.contains("<i>"));
        assert!(xml.contains("hello &amp; world"));
        assert!(xml.contains("1.500,1,25,16777215"));
    }
}

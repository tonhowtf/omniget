use flate2::bufread::MultiGzDecoder;
use quick_xml::events::{BytesStart, Event};
use quick_xml::Reader;
use serde::{Deserialize, Serialize};
use std::io::{self, BufRead, BufReader, Read};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum XmlEvent {
    Channel {
        id: String,
        names: Vec<String>,
        icon: Option<String>,
    },
    Programme {
        channel: String,
        start_ms: i64,
        stop_ms: Option<i64>,
        title: String,
        desc: Option<String>,
        category: Option<String>,
    },
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct XmltvStats {
    pub channels: usize,
    pub programmes: usize,
    pub error: Option<String>,
}

fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

pub fn parse_time(s: &str) -> Option<i64> {
    let s = s.trim();
    let digits: String = s.chars().take_while(|c| c.is_ascii_digit()).collect();
    if digits.len() < 8 {
        return None;
    }
    let digits = &digits[..digits.len().min(14)];
    let field = |a: usize, b: usize| -> Option<i64> {
        if digits.len() >= b {
            digits[a..b].parse().ok()
        } else {
            Some(0)
        }
    };
    let (y, mo, d) = (field(0, 4)?, field(4, 6)?, field(6, 8)?);
    let (h, mi, sec) = (field(8, 10)?, field(10, 12)?, field(12, 14)?);
    if !(1..=12).contains(&mo) || !(1..=31).contains(&d) || h > 24 || mi > 59 || sec > 60 {
        return None;
    }
    let rest = s[s
        .char_indices()
        .nth(digits.len())
        .map(|(i, _)| i)
        .unwrap_or(s.len())..]
        .trim();
    let rest = rest
        .trim_start_matches(|c: char| c == '.' || c.is_ascii_digit())
        .trim();
    let offset_s: i64 = if rest.is_empty()
        || rest.eq_ignore_ascii_case("z")
        || rest.eq_ignore_ascii_case("utc")
        || rest.eq_ignore_ascii_case("gmt")
    {
        0
    } else {
        let sign = match rest.chars().next()? {
            '+' => 1,
            '-' => -1,
            _ => return Some(civil_ms(y, mo, d, h, mi, sec, 0)),
        };
        let tz: String = rest[1..].chars().filter(|c| c.is_ascii_digit()).collect();
        let (th, tm) = match tz.len() {
            1 | 2 => (tz.parse::<i64>().ok()?, 0),
            3 => (tz[..1].parse::<i64>().ok()?, tz[1..].parse::<i64>().ok()?),
            n if n >= 4 => (tz[..2].parse::<i64>().ok()?, tz[2..4].parse::<i64>().ok()?),
            _ => (0, 0),
        };
        sign * (th * 3600 + tm * 60)
    };
    Some(civil_ms(y, mo, d, h, mi, sec, offset_s))
}

fn civil_ms(y: i64, mo: i64, d: i64, h: i64, mi: i64, sec: i64, offset_s: i64) -> i64 {
    let days = days_from_civil(y, mo, d);
    (days * 86_400 + h * 3600 + mi * 60 + sec - offset_s) * 1000
}

#[derive(Default)]
struct ProgBuilder {
    channel: String,
    start: Option<i64>,
    stop: Option<i64>,
    title: Option<String>,
    desc: Option<String>,
    category: Option<String>,
}

#[derive(Default)]
struct ChanBuilder {
    id: String,
    names: Vec<String>,
    icon: Option<String>,
}

#[derive(Clone, Copy, PartialEq)]
enum Field {
    None,
    Title,
    Desc,
    Category,
    DisplayName,
}

fn attr(e: &BytesStart, name: &str) -> Option<String> {
    e.attributes().flatten().find_map(|a| {
        (a.key.local_name().as_ref() == name).then(|| match quick_xml::escape::unescape(&a.value) {
            Ok(v) => v.into_owned(),
            Err(_) => a.value.to_string(),
        })
    })
}

fn set_first(slot: &mut Option<String>, text: &str) {
    let t = text.trim();
    if slot.is_none() && !t.is_empty() {
        *slot = Some(t.to_string());
    }
}

struct LossyUtf8<R> {
    inner: R,
    out: Vec<u8>,
    pos: usize,
    tail: Vec<u8>,
}

impl<R: BufRead> LossyUtf8<R> {
    fn new(inner: R) -> Self {
        Self {
            inner,
            out: Vec::new(),
            pos: 0,
            tail: Vec::new(),
        }
    }
}

impl<R: BufRead> BufRead for LossyUtf8<R> {
    fn fill_buf(&mut self) -> io::Result<&[u8]> {
        while self.pos == self.out.len() {
            self.out.clear();
            self.pos = 0;
            let chunk = self.inner.fill_buf()?;
            if chunk.is_empty() {
                if !std::mem::take(&mut self.tail).is_empty() {
                    self.out.extend_from_slice("\u{FFFD}".as_bytes());
                }
                break;
            }
            let mut data = std::mem::take(&mut self.tail);
            data.extend_from_slice(chunk);
            let read = chunk.len();
            self.inner.consume(read);
            let mut parts = data.utf8_chunks().peekable();
            while let Some(part) = parts.next() {
                self.out.extend_from_slice(part.valid().as_bytes());
                if part.invalid().is_empty() {
                    continue;
                }
                if parts.peek().is_none() {
                    // may be a character cut in half by the chunk boundary
                    self.tail.extend_from_slice(part.invalid());
                } else {
                    self.out.extend_from_slice("\u{FFFD}".as_bytes());
                }
            }
        }
        Ok(&self.out[self.pos..])
    }

    fn consume(&mut self, amt: usize) {
        self.pos = (self.pos + amt).min(self.out.len());
    }
}

impl<R: BufRead> Read for LossyUtf8<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let avail = self.fill_buf()?;
        let n = avail.len().min(buf.len());
        buf[..n].copy_from_slice(&avail[..n]);
        self.consume(n);
        Ok(n)
    }
}

pub fn parse<R: BufRead>(mut reader: R, sink: impl FnMut(XmlEvent)) -> XmltvStats {
    let gz = match reader.fill_buf() {
        Ok(b) => b.len() >= 2 && b[0] == 0x1f && b[1] == 0x8b,
        Err(e) => {
            return XmltvStats {
                error: Some(e.to_string()),
                ..XmltvStats::default()
            }
        }
    };
    if gz {
        parse_plain(BufReader::new(MultiGzDecoder::new(reader)), sink)
    } else {
        parse_plain(reader, sink)
    }
}

fn parse_plain<R: BufRead>(input: R, mut sink: impl FnMut(XmlEvent)) -> XmltvStats {
    let mut reader = Reader::from_reader(LossyUtf8::new(input));
    {
        let cfg = reader.config_mut();
        cfg.expand_empty_elements = true;
        cfg.check_end_names = false;
    }
    let mut stats = XmltvStats::default();
    let mut buf = Vec::new();
    let mut prog: Option<ProgBuilder> = None;
    let mut chan: Option<ChanBuilder> = None;
    let mut field = Field::None;
    let mut text = String::new();
    loop {
        let ev = match reader.read_event_into(&mut buf) {
            Ok(ev) => ev,
            Err(e) => {
                stats.error = Some(format!(
                    "xml error at byte {}: {e}",
                    reader.buffer_position()
                ));
                break;
            }
        };
        match ev {
            Event::Start(e) => {
                let name = e.local_name();
                match name.as_ref() {
                    "programme" => {
                        prog = Some(ProgBuilder {
                            channel: attr(&e, "channel").unwrap_or_default(),
                            start: attr(&e, "start").and_then(|s| parse_time(&s)),
                            stop: attr(&e, "stop").and_then(|s| parse_time(&s)),
                            ..ProgBuilder::default()
                        });
                        chan = None;
                    }
                    "channel" if prog.is_none() => {
                        chan = Some(ChanBuilder {
                            id: attr(&e, "id").unwrap_or_default(),
                            ..ChanBuilder::default()
                        });
                    }
                    "title" if prog.is_some() => field = Field::Title,
                    "desc" if prog.is_some() => field = Field::Desc,
                    "category" if prog.is_some() => field = Field::Category,
                    "display-name" if chan.is_some() => field = Field::DisplayName,
                    "icon" => {
                        if let Some(c) = chan.as_mut() {
                            if c.icon.is_none() {
                                c.icon = attr(&e, "src").filter(|s| !s.trim().is_empty());
                            }
                        }
                    }
                    _ => {}
                }
                if field != Field::None {
                    text.clear();
                }
            }
            Event::Text(t) => {
                if field != Field::None {
                    match quick_xml::escape::unescape(&t) {
                        Ok(s) => text.push_str(&s),
                        Err(_) => text.push_str(&t),
                    }
                }
            }
            Event::GeneralRef(r) => {
                if field != Field::None {
                    let entity = format!("&{};", &*r);
                    match quick_xml::escape::unescape(&entity) {
                        Ok(s) => text.push_str(&s),
                        Err(_) => text.push_str(&entity),
                    }
                }
            }
            Event::CData(t) => {
                if field != Field::None {
                    text.push_str(&t);
                }
            }
            Event::End(e) => {
                let name = e.local_name();
                match name.as_ref() {
                    "title" | "desc" | "category" | "display-name" => {
                        match (field, prog.as_mut(), chan.as_mut()) {
                            (Field::Title, Some(p), _) => set_first(&mut p.title, &text),
                            (Field::Desc, Some(p), _) => set_first(&mut p.desc, &text),
                            (Field::Category, Some(p), _) => set_first(&mut p.category, &text),
                            (Field::DisplayName, _, Some(c)) => {
                                let t = text.trim();
                                if !t.is_empty() && !c.names.iter().any(|n| n == t) {
                                    c.names.push(t.to_string());
                                }
                            }
                            _ => {}
                        }
                        field = Field::None;
                        text.clear();
                    }
                    "programme" => {
                        if let Some(p) = prog.take() {
                            if let (false, Some(start)) = (p.channel.is_empty(), p.start) {
                                stats.programmes += 1;
                                sink(XmlEvent::Programme {
                                    channel: p.channel,
                                    start_ms: start,
                                    stop_ms: p.stop.filter(|s| *s > start),
                                    title: p.title.unwrap_or_default(),
                                    desc: p.desc,
                                    category: p.category,
                                });
                            }
                        }
                        field = Field::None;
                    }
                    "channel" => {
                        if let Some(c) = chan.take() {
                            if !c.id.is_empty() {
                                stats.channels += 1;
                                sink(XmlEvent::Channel {
                                    id: c.id,
                                    names: c.names,
                                    icon: c.icon,
                                });
                            }
                        }
                        field = Field::None;
                    }
                    _ => {}
                }
            }
            Event::Eof => break,
            _ => {}
        }
        buf.clear();
    }
    stats
}

#[cfg(test)]
mod tests {
    use super::*;
    use flate2::write::GzEncoder;
    use flate2::Compression;
    use std::io::Write;

    const DOC: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE tv SYSTEM "xmltv.dtd">
<tv generator-info-name="x">
  <channel id="globo.br">
    <display-name lang="pt">Globo</display-name>
    <display-name>Globo HD</display-name>
    <icon src="http://img/globo.png"/>
  </channel>
  <channel id="empty.br"/>
  <programme start="20240101120000 +0200" stop="20240101130000 +0200" channel="globo.br">
    <title lang="pt">Jornal &amp; Notícias</title>
    <title lang="en">News</title>
    <desc><![CDATA[Descrição <b>longa</b>]]></desc>
    <category>News</category>
    <category>Talk</category>
  </programme>
  <programme start="20240101130000" channel="globo.br"><title>Sem fim</title></programme>
  <programme start="garbage" channel="globo.br"><title>Bad</title></programme>
</tv>"#;

    fn collect(bytes: &[u8]) -> (Vec<XmlEvent>, XmltvStats) {
        let mut v = Vec::new();
        let stats = parse(bytes, |e| v.push(e));
        (v, stats)
    }

    #[test]
    fn plain_document() {
        let (ev, stats) = collect(DOC.as_bytes());
        assert_eq!(stats.error, None);
        assert_eq!((stats.channels, stats.programmes), (2, 2));
        assert_eq!(
            ev[0],
            XmlEvent::Channel {
                id: "globo.br".into(),
                names: vec!["Globo".into(), "Globo HD".into()],
                icon: Some("http://img/globo.png".into()),
            }
        );
        assert_eq!(
            ev[2],
            XmlEvent::Programme {
                channel: "globo.br".into(),
                start_ms: 1_704_103_200_000,
                stop_ms: Some(1_704_106_800_000),
                title: "Jornal & Notícias".into(),
                desc: Some("Descrição <b>longa</b>".into()),
                category: Some("News".into()),
            }
        );
        match &ev[3] {
            XmlEvent::Programme {
                start_ms, stop_ms, ..
            } => {
                assert_eq!(*start_ms, 1_704_114_000_000);
                assert_eq!(*stop_ms, None);
            }
            _ => panic!(),
        }
    }

    #[test]
    fn gzip_document() {
        let mut enc = GzEncoder::new(Vec::new(), Compression::default());
        enc.write_all(DOC.as_bytes()).unwrap();
        let gz = enc.finish().unwrap();
        let (ev, stats) = collect(&gz);
        assert_eq!(stats.error, None);
        assert_eq!(ev.len(), 4);
    }

    #[test]
    fn truncated_document_keeps_events() {
        let cut = &DOC[..DOC.find("<programme start=\"20240101130000\"").unwrap() + 20];
        let (ev, stats) = collect(cut.as_bytes());
        assert_eq!(stats.programmes, 1);
        assert_eq!(ev.len(), 3);
        let (ev, _) = collect(b"not xml at all <<<");
        assert!(ev.is_empty());
        let (ev, stats) = collect(b"");
        assert!(ev.is_empty() && stats.error.is_none());
    }

    #[test]
    fn invalid_utf8_becomes_replacement_char() {
        let mut doc = Vec::new();
        doc.extend_from_slice(
            b"<tv><channel id=\"tv.br\"><display-name>Canal S\xe3o</display-name></channel>",
        );
        doc.extend_from_slice(b"<programme start=\"20240101120000\" channel=\"tv.br\"><title>Not\xedcias</title></programme></tv>");
        let (ev, stats) = collect(&doc);
        assert_eq!(stats.error, None);
        assert_eq!((stats.channels, stats.programmes), (1, 1));
        match (&ev[0], &ev[1]) {
            (XmlEvent::Channel { names, .. }, XmlEvent::Programme { title, .. }) => {
                assert_eq!(names, &["Canal S\u{FFFD}o"]);
                assert_eq!(title, "Not\u{FFFD}cias");
            }
            _ => panic!(),
        }
    }

    #[test]
    fn characters_split_across_reads() {
        let (whole, _) = collect(DOC.as_bytes());
        let mut split = Vec::new();
        let stats = parse(BufReader::with_capacity(1, DOC.as_bytes()), |e| {
            split.push(e)
        });
        assert_eq!(stats.error, None);
        assert_eq!(split, whole);
    }

    #[test]
    fn times() {
        assert_eq!(parse_time("19700101000000 +0000"), Some(0));
        assert_eq!(parse_time("19700101000000"), Some(0));
        assert_eq!(parse_time("19700101010000 +0100"), Some(0));
        assert_eq!(parse_time("19691231230000 -0100"), Some(0));
        assert_eq!(parse_time("19700101053000 +05:30"), Some(0));
        assert_eq!(parse_time("197001010000"), Some(0));
        assert_eq!(parse_time("20240229"), Some(1_709_164_800_000));
        assert_eq!(parse_time("20241301000000"), None);
        assert_eq!(parse_time("2024"), None);
        assert_eq!(parse_time(""), None);
    }
}

//! Streaming XMLTV guide parser. Handles gzip transparently (by magic bytes)
//! and converts `YYYYMMDDhhmmss ±zzzz` times to unix milliseconds.

use flate2::bufread::MultiGzDecoder;
use quick_xml::events::{BytesStart, Event};
use quick_xml::Reader;
use serde::{Deserialize, Serialize};
use std::io::{BufRead, BufReader};

/// One item of the guide, emitted as soon as its element closes.
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

/// What a parse produced. `error` is set when the document broke off; the
/// events before it were still delivered.
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

/// Parses an XMLTV time (`20240101120000 +0200`, `202401011200`, `20240101`,
/// with `±hhmm`, `±hh:mm`, `Z`/`UTC`/`GMT` or no zone = UTC) to unix ms.
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
    // Skip fractional seconds some generators add ("…00.000 +0000").
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

fn attr(e: &BytesStart, name: &[u8]) -> Option<String> {
    e.attributes().flatten().find_map(|a| {
        (a.key.local_name().as_ref() == name).then(|| match a.unescape_value() {
            Ok(v) => v.into_owned(),
            Err(_) => String::from_utf8_lossy(&a.value).into_owned(),
        })
    })
}

fn set_first(slot: &mut Option<String>, text: &str) {
    let t = text.trim();
    if slot.is_none() && !t.is_empty() {
        *slot = Some(t.to_string());
    }
}

/// Parses an XMLTV document (plain or gzip) and calls `sink` for every
/// channel and programme. Never panics on bad input.
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
    let mut reader = Reader::from_reader(input);
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
                    b"programme" => {
                        prog = Some(ProgBuilder {
                            channel: attr(&e, b"channel").unwrap_or_default(),
                            start: attr(&e, b"start").and_then(|s| parse_time(&s)),
                            stop: attr(&e, b"stop").and_then(|s| parse_time(&s)),
                            ..ProgBuilder::default()
                        });
                        chan = None;
                    }
                    b"channel" if prog.is_none() => {
                        chan = Some(ChanBuilder {
                            id: attr(&e, b"id").unwrap_or_default(),
                            ..ChanBuilder::default()
                        });
                    }
                    b"title" if prog.is_some() => field = Field::Title,
                    b"desc" if prog.is_some() => field = Field::Desc,
                    b"category" if prog.is_some() => field = Field::Category,
                    b"display-name" if chan.is_some() => field = Field::DisplayName,
                    b"icon" => {
                        if let Some(c) = chan.as_mut() {
                            if c.icon.is_none() {
                                c.icon = attr(&e, b"src").filter(|s| !s.trim().is_empty());
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
                    let raw = String::from_utf8_lossy(&t);
                    match quick_xml::escape::unescape(&raw) {
                        Ok(s) => text.push_str(&s),
                        Err(_) => text.push_str(&raw),
                    }
                }
            }
            // quick-xml 0.38 reports `&amp;`, `&#233;`… as their own events.
            Event::GeneralRef(r) => {
                if field != Field::None {
                    let entity = format!("&{};", String::from_utf8_lossy(&r));
                    match quick_xml::escape::unescape(&entity) {
                        Ok(s) => text.push_str(&s),
                        Err(_) => text.push_str(&entity),
                    }
                }
            }
            Event::CData(t) => {
                if field != Field::None {
                    text.push_str(&String::from_utf8_lossy(&t));
                }
            }
            Event::End(e) => {
                let name = e.local_name();
                match name.as_ref() {
                    b"title" | b"desc" | b"category" | b"display-name" => {
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
                    b"programme" => {
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
                    b"channel" => {
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
        // 2024-01-01T10:00:00Z
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

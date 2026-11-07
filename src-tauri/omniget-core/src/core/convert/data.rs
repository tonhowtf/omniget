use std::fmt;

use quick_xml::events::{BytesDecl, BytesEnd, BytesStart, BytesText, Event};
use quick_xml::{Reader, Writer};
use serde::de::{Deserialize, Deserializer, MapAccess, SeqAccess, Visitor};
use serde::ser::{Serialize, SerializeMap, SerializeSeq, Serializer};
use serde_json::Number;

use super::ConvertOptions;
use crate::core::read::xml::unescape;

#[derive(Clone, Debug, PartialEq, Default)]
pub enum Value {
    #[default]
    Null,
    Bool(bool),
    Number(Number),
    String(String),
    Array(Vec<Value>),
    Object(Vec<(String, Value)>),
}

static NULL: Value = Value::Null;

impl Value {
    pub fn get(&self, key: &str) -> Option<&Value> {
        match self {
            Value::Object(m) => m.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }

    pub fn is_array(&self) -> bool {
        matches!(self, Value::Array(_))
    }

    pub fn str(s: &str) -> Value {
        Value::String(s.to_string())
    }

    fn insert(map: &mut Vec<(String, Value)>, key: String, value: Value) {
        match map.iter_mut().find(|(k, _)| *k == key) {
            Some(slot) => slot.1 = value,
            None => map.push((key, value)),
        }
    }
}

impl std::ops::Index<&str> for Value {
    type Output = Value;
    fn index(&self, key: &str) -> &Value {
        self.get(key).unwrap_or(&NULL)
    }
}

impl std::ops::Index<usize> for Value {
    type Output = Value;
    fn index(&self, i: usize) -> &Value {
        match self {
            Value::Array(a) => a.get(i).unwrap_or(&NULL),
            _ => &NULL,
        }
    }
}

impl From<i64> for Value {
    fn from(i: i64) -> Self {
        Value::Number(i.into())
    }
}

impl Serialize for Value {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        match self {
            Value::Null => s.serialize_unit(),
            Value::Bool(b) => s.serialize_bool(*b),
            Value::Number(n) => n.serialize(s),
            Value::String(v) => s.serialize_str(v),
            Value::Array(a) => {
                let mut seq = s.serialize_seq(Some(a.len()))?;
                for v in a {
                    seq.serialize_element(v)?;
                }
                seq.end()
            }
            Value::Object(m) => {
                let mut map = s.serialize_map(Some(m.len()))?;
                for (k, v) in m {
                    map.serialize_entry(k, v)?;
                }
                map.end()
            }
        }
    }
}

struct ValueVisitor;

impl<'de> Visitor<'de> for ValueVisitor {
    type Value = Value;

    fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str("any JSON value")
    }
    fn visit_bool<E>(self, v: bool) -> Result<Value, E> {
        Ok(Value::Bool(v))
    }
    fn visit_i64<E>(self, v: i64) -> Result<Value, E> {
        Ok(Value::Number(v.into()))
    }
    fn visit_u64<E>(self, v: u64) -> Result<Value, E> {
        Ok(Value::Number(v.into()))
    }
    fn visit_f64<E>(self, v: f64) -> Result<Value, E> {
        Ok(Number::from_f64(v)
            .map(Value::Number)
            .unwrap_or(Value::Null))
    }
    fn visit_str<E>(self, v: &str) -> Result<Value, E> {
        Ok(Value::String(v.to_string()))
    }
    fn visit_string<E>(self, v: String) -> Result<Value, E> {
        Ok(Value::String(v))
    }
    fn visit_unit<E>(self) -> Result<Value, E> {
        Ok(Value::Null)
    }
    fn visit_none<E>(self) -> Result<Value, E> {
        Ok(Value::Null)
    }
    fn visit_some<D: Deserializer<'de>>(self, d: D) -> Result<Value, D::Error> {
        Value::deserialize(d)
    }
    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Value, A::Error> {
        let mut out = Vec::new();
        while let Some(v) = seq.next_element()? {
            out.push(v);
        }
        Ok(Value::Array(out))
    }
    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Value, A::Error> {
        let mut out: Vec<(String, Value)> = Vec::new();
        while let Some((k, v)) = map.next_entry::<String, Value>()? {
            Value::insert(&mut out, k, v);
        }
        Ok(Value::Object(out))
    }
}

impl<'de> Deserialize<'de> for Value {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Value, D::Error> {
        d.deserialize_any(ValueVisitor)
    }
}

pub fn convert_bytes(
    bytes: &[u8],
    from: &str,
    to: &str,
    opts: &ConvertOptions,
) -> Result<Vec<u8>, String> {
    let value = read_value(bytes, from)?;
    write_value(&value, to, opts)
}

pub fn read_value(bytes: &[u8], format: &str) -> Result<Value, String> {
    let text = || crate::core::read::epub_writer::decode_text(bytes);
    let label = format.to_ascii_uppercase();
    let bad = |e: String| format!("This file is not valid {label}: {e}");
    match format {
        "json" => serde_json::from_str::<Value>(text().trim_start_matches('\u{feff}'))
            .map_err(|e| bad(e.to_string())),
        "jsonl" => {
            let mut out = Vec::new();
            for (i, line) in text().lines().enumerate() {
                let line = line.trim();
                if line.is_empty() {
                    continue;
                }
                out.push(
                    serde_json::from_str::<Value>(line)
                        .map_err(|e| bad(format!("line {}: {e}", i + 1)))?,
                );
            }
            Ok(Value::Array(out))
        }
        "yaml" => serde_yaml::from_str::<serde_yaml::Value>(&text())
            .map(yaml_to_value)
            .map_err(|e| bad(e.to_string())),
        "toml" => toml::from_str::<toml::Table>(&text())
            .map(|t| toml_to_value(toml::Value::Table(t)))
            .map_err(|e| bad(e.to_string())),
        "csv" => read_csv(&text(), b','),
        "tsv" => read_csv(&text(), b'\t'),
        "xml" => read_xml(&text()).map_err(bad),
        "plist" => plist::Value::from_reader(std::io::Cursor::new(bytes))
            .map(plist_to_value)
            .map_err(|e| bad(e.to_string())),
        other => Err(format!("{other} is not a data format.")),
    }
}

pub fn write_value(value: &Value, format: &str, opts: &ConvertOptions) -> Result<Vec<u8>, String> {
    let pretty = opts.pretty.unwrap_or(true);
    let sorted;
    let value = if opts.sort_keys.unwrap_or(false) {
        sorted = sort_keys(value);
        &sorted
    } else {
        value
    };
    let fail = |e: String| format!("Could not write {}: {e}", format.to_ascii_uppercase());
    match format {
        "json" => {
            let mut out = if pretty {
                serde_json::to_vec_pretty(value)
            } else {
                serde_json::to_vec(value)
            }
            .map_err(|e| fail(e.to_string()))?;
            out.push(b'\n');
            Ok(out)
        }
        "jsonl" => {
            let items: Vec<&Value> = match value {
                Value::Array(a) => a.iter().collect(),
                v => vec![v],
            };
            let mut out = Vec::new();
            for item in items {
                out.extend(serde_json::to_vec(item).map_err(|e| fail(e.to_string()))?);
                out.push(b'\n');
            }
            Ok(out)
        }
        "yaml" => serde_yaml::to_string(value)
            .map(String::into_bytes)
            .map_err(|e| fail(e.to_string())),
        "toml" => {
            let table = match value_to_toml(value) {
                Some(toml::Value::Table(t)) => t,
                Some(other) => {
                    let mut t = toml::Table::new();
                    t.insert("items".to_string(), other);
                    t
                }
                None => toml::Table::new(),
            };
            let s = if pretty {
                toml::to_string_pretty(&table)
            } else {
                toml::to_string(&table)
            };
            s.map(String::into_bytes).map_err(|e| fail(e.to_string()))
        }
        "csv" => write_csv(value, b','),
        "tsv" => write_csv(value, b'\t'),
        "xml" => write_xml(value, pretty).map_err(fail),
        "plist" => {
            let v = value_to_plist(value).unwrap_or(plist::Value::Dictionary(Default::default()));
            let mut out = Vec::new();
            v.to_writer_xml(&mut out).map_err(|e| fail(e.to_string()))?;
            Ok(out)
        }
        other => Err(format!("{other} is not a data format.")),
    }
}

pub fn sort_keys(value: &Value) -> Value {
    match value {
        Value::Object(m) => {
            let mut out: Vec<(String, Value)> =
                m.iter().map(|(k, v)| (k.clone(), sort_keys(v))).collect();
            out.sort_by(|a, b| a.0.cmp(&b.0));
            Value::Object(out)
        }
        Value::Array(a) => Value::Array(a.iter().map(sort_keys).collect()),
        v => v.clone(),
    }
}

fn yaml_key(k: serde_yaml::Value) -> String {
    match k {
        serde_yaml::Value::String(s) => s,
        serde_yaml::Value::Null => "null".to_string(),
        serde_yaml::Value::Bool(b) => b.to_string(),
        serde_yaml::Value::Number(n) => n.to_string(),
        other => serde_yaml::to_string(&other)
            .unwrap_or_default()
            .trim()
            .to_string(),
    }
}

fn float(f: f64) -> Value {
    Number::from_f64(f)
        .map(Value::Number)
        .unwrap_or(Value::Null)
}

fn yaml_to_value(v: serde_yaml::Value) -> Value {
    match v {
        serde_yaml::Value::Null => Value::Null,
        serde_yaml::Value::Bool(b) => Value::Bool(b),
        serde_yaml::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                Value::Number(i.into())
            } else if let Some(u) = n.as_u64() {
                Value::Number(u.into())
            } else {
                n.as_f64().map(float).unwrap_or(Value::Null)
            }
        }
        serde_yaml::Value::String(s) => Value::String(s),
        serde_yaml::Value::Sequence(s) => Value::Array(s.into_iter().map(yaml_to_value).collect()),
        serde_yaml::Value::Mapping(m) => {
            let mut out = Vec::new();
            for (k, v) in m {
                Value::insert(&mut out, yaml_key(k), yaml_to_value(v));
            }
            Value::Object(out)
        }
        serde_yaml::Value::Tagged(t) => yaml_to_value(t.value),
    }
}

fn toml_to_value(v: toml::Value) -> Value {
    match v {
        toml::Value::String(s) => Value::String(s),
        toml::Value::Integer(i) => Value::from(i),
        toml::Value::Float(f) => float(f),
        toml::Value::Boolean(b) => Value::Bool(b),
        toml::Value::Datetime(d) => Value::String(d.to_string()),
        toml::Value::Array(a) => Value::Array(a.into_iter().map(toml_to_value).collect()),
        toml::Value::Table(t) => {
            Value::Object(t.into_iter().map(|(k, v)| (k, toml_to_value(v))).collect())
        }
    }
}

fn value_to_toml(v: &Value) -> Option<toml::Value> {
    match v {
        Value::Null => None,
        Value::Bool(b) => Some(toml::Value::Boolean(*b)),
        Value::Number(n) => n
            .as_i64()
            .map(toml::Value::Integer)
            .or_else(|| n.as_f64().map(toml::Value::Float)),
        Value::String(s) => Some(toml::Value::String(s.clone())),
        Value::Array(a) => Some(toml::Value::Array(
            a.iter().filter_map(value_to_toml).collect(),
        )),
        Value::Object(m) => Some(toml::Value::Table(
            m.iter()
                .filter_map(|(k, v)| value_to_toml(v).map(|v| (k.clone(), v)))
                .collect(),
        )),
    }
}

fn plist_to_value(v: plist::Value) -> Value {
    use base64::Engine as _;
    match v {
        plist::Value::Array(a) => Value::Array(a.into_iter().map(plist_to_value).collect()),
        plist::Value::Dictionary(d) => {
            Value::Object(d.into_iter().map(|(k, v)| (k, plist_to_value(v))).collect())
        }
        plist::Value::Boolean(b) => Value::Bool(b),
        plist::Value::Data(d) => Value::String(base64::engine::general_purpose::STANDARD.encode(d)),
        plist::Value::Date(d) => Value::String(d.to_xml_format()),
        plist::Value::Real(f) => float(f),
        plist::Value::Integer(i) => i
            .as_signed()
            .map(Value::from)
            .or_else(|| i.as_unsigned().map(|u| Value::Number(u.into())))
            .unwrap_or(Value::Null),
        plist::Value::String(s) => Value::String(s),
        plist::Value::Uid(u) => Value::Number(u.get().into()),
        _ => Value::Null,
    }
}

fn value_to_plist(v: &Value) -> Option<plist::Value> {
    match v {
        Value::Null => None,
        Value::Bool(b) => Some(plist::Value::Boolean(*b)),
        Value::Number(n) => n
            .as_i64()
            .map(|i| plist::Value::Integer(i.into()))
            .or_else(|| n.as_u64().map(|u| plist::Value::Integer(u.into())))
            .or_else(|| n.as_f64().map(plist::Value::Real)),
        Value::String(s) => Some(plist::Value::String(s.clone())),
        Value::Array(a) => Some(plist::Value::Array(
            a.iter().filter_map(value_to_plist).collect(),
        )),
        Value::Object(m) => Some(plist::Value::Dictionary(
            m.iter()
                .filter_map(|(k, v)| value_to_plist(v).map(|v| (k.clone(), v)))
                .collect(),
        )),
    }
}

fn infer_scalar(s: &str) -> Value {
    match s {
        "true" => return Value::Bool(true),
        "false" => return Value::Bool(false),
        _ => {}
    }
    if let Ok(i) = s.parse::<i64>() {
        if i.to_string() == s {
            return Value::from(i);
        }
    }
    if s.contains('.') && !s.starts_with('.') && !s.ends_with('.') {
        if let Ok(f) = s.parse::<f64>() {
            if f.is_finite() && f.to_string() == s {
                return float(f);
            }
        }
    }
    Value::str(s)
}

fn unique_headers(raw: impl Iterator<Item = String>) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for h in raw {
        let mut name = h.clone();
        let mut n = 2;
        while out.contains(&name) {
            name = format!("{h}_{n}");
            n += 1;
        }
        out.push(name);
    }
    out
}

fn read_csv(text: &str, delimiter: u8) -> Result<Value, String> {
    let label = if delimiter == b'\t' { "TSV" } else { "CSV" };
    let mut rdr = csv::ReaderBuilder::new()
        .delimiter(delimiter)
        .flexible(true)
        .from_reader(text.trim_start_matches('\u{feff}').as_bytes());
    let headers = unique_headers(
        rdr.headers()
            .map_err(|e| format!("This file is not valid {label}: {e}"))?
            .iter()
            .map(str::to_string),
    );
    let mut rows = Vec::new();
    for rec in rdr.records() {
        let rec = rec.map_err(|e| format!("This file is not valid {label}: {e}"))?;
        let mut row = Vec::new();
        for (i, field) in rec.iter().enumerate() {
            let key = headers
                .get(i)
                .cloned()
                .unwrap_or_else(|| format!("column{}", i + 1));
            Value::insert(&mut row, key, infer_scalar(field));
        }
        rows.push(Value::Object(row));
    }
    Ok(Value::Array(rows))
}

fn cell(v: &Value) -> String {
    match v {
        Value::Null => String::new(),
        Value::String(s) => s.clone(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => n.to_string(),
        other => serde_json::to_string(other).unwrap_or_default(),
    }
}

fn table_root(mut value: &Value) -> &Value {
    while let Value::Object(m) = value {
        match m.as_slice() {
            [(_, inner @ (Value::Array(_) | Value::Object(_)))] => value = inner,
            _ => break,
        }
    }
    value
}

fn single(key: &str, v: &Value) -> Vec<(String, Value)> {
    vec![(key.to_string(), v.clone())]
}

fn write_csv(value: &Value, delimiter: u8) -> Result<Vec<u8>, String> {
    let rows: Vec<Vec<(String, Value)>> = match table_root(value) {
        Value::Array(a) => a
            .iter()
            .map(|v| match v {
                Value::Object(m) => m.clone(),
                other => single("value", other),
            })
            .collect(),
        Value::Object(m) => vec![m.clone()],
        other => vec![single("value", other)],
    };
    let mut header: Vec<String> = Vec::new();
    for r in &rows {
        for (k, _) in r {
            if !header.contains(k) {
                header.push(k.clone());
            }
        }
    }
    let mut w = csv::WriterBuilder::new()
        .delimiter(delimiter)
        .from_writer(Vec::new());
    w.write_record(&header).map_err(|e| e.to_string())?;
    for r in &rows {
        let rec: Vec<String> = header
            .iter()
            .map(|k| {
                r.iter()
                    .find(|(rk, _)| rk == k)
                    .map(|(_, v)| cell(v))
                    .unwrap_or_default()
            })
            .collect();
        w.write_record(&rec).map_err(|e| e.to_string())?;
    }
    w.into_inner().map_err(|e| e.to_string())
}

struct Frame {
    name: String,
    map: Vec<(String, Value)>,
    text: String,
}

fn insert_child(map: &mut Vec<(String, Value)>, key: String, value: Value) {
    match map.iter_mut().find(|(k, _)| *k == key) {
        Some((_, Value::Array(a))) => a.push(value),
        Some((_, existing)) => {
            let prev = std::mem::take(existing);
            *existing = Value::Array(vec![prev, value]);
        }
        None => map.push((key, value)),
    }
}

fn frame_from(e: &BytesStart<'_>) -> Result<Frame, String> {
    let name = String::from_utf8_lossy(e.name().as_ref()).to_string();
    let mut map = Vec::new();
    for a in e.attributes() {
        let a = a.map_err(|e| e.to_string())?;
        let key = format!("@{}", String::from_utf8_lossy(a.key.as_ref()));
        let val = unescape(&String::from_utf8_lossy(&a.value));
        Value::insert(&mut map, key, Value::String(val));
    }
    Ok(Frame {
        name,
        map,
        text: String::new(),
    })
}

fn finish(frame: Frame) -> (String, Value) {
    let text = frame.text.trim().to_string();
    let value = if frame.map.is_empty() {
        if text.is_empty() {
            Value::Null
        } else {
            Value::String(text)
        }
    } else {
        let mut map = frame.map;
        if !text.is_empty() {
            map.push(("#text".to_string(), Value::String(text)));
        }
        Value::Object(map)
    };
    (frame.name, value)
}

fn read_xml(text: &str) -> Result<Value, String> {
    let mut reader = Reader::from_str(text);
    let mut stack: Vec<Frame> = Vec::new();
    let mut root: Option<(String, Value)> = None;
    loop {
        let ev = reader.read_event().map_err(|e| e.to_string())?;
        match ev {
            Event::Start(e) => stack.push(frame_from(&e)?),
            Event::Empty(e) => {
                let (k, v) = finish(frame_from(&e)?);
                match stack.last_mut() {
                    Some(top) => insert_child(&mut top.map, k, v),
                    None => root = Some((k, v)),
                }
            }
            Event::End(_) => {
                let frame = stack.pop().ok_or("unexpected closing tag")?;
                let (k, v) = finish(frame);
                match stack.last_mut() {
                    Some(top) => insert_child(&mut top.map, k, v),
                    None => root = Some((k, v)),
                }
            }
            Event::Text(t) => {
                if let Some(top) = stack.last_mut() {
                    top.text
                        .push_str(&unescape(&String::from_utf8_lossy(t.as_ref())));
                }
            }
            Event::GeneralRef(r) => {
                if let Some(top) = stack.last_mut() {
                    let name = String::from_utf8_lossy(r.as_ref()).to_string();
                    top.text.push_str(&unescape(&format!("&{name};")));
                }
            }
            Event::CData(t) => {
                if let Some(top) = stack.last_mut() {
                    top.text.push_str(&String::from_utf8_lossy(t.as_ref()));
                }
            }
            Event::Eof => break,
            _ => {}
        }
    }
    if !stack.is_empty() {
        return Err("the document ends before all tags are closed".to_string());
    }
    let (k, v) = root.ok_or("there is no root element")?;
    Ok(Value::Object(vec![(k, v)]))
}

fn xml_allowed(c: char) -> bool {
    matches!(c, '\t' | '\n' | '\r' | '\u{20}'..='\u{D7FF}' | '\u{E000}'..='\u{FFFD}' | '\u{10000}'..)
}

fn xml_clean(s: &str) -> String {
    s.chars().filter(|c| xml_allowed(*c)).collect()
}

fn xml_name(raw: &str) -> String {
    let mut out: String = raw
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || matches!(c, '_' | '-' | '.' | ':') {
                c
            } else {
                '_'
            }
        })
        .collect();
    if out
        .chars()
        .next()
        .is_none_or(|c| !(c.is_alphabetic() || c == '_'))
    {
        out.insert(0, '_');
    }
    out
}

type XmlW = Writer<Vec<u8>>;

fn write_element(w: &mut XmlW, name: &str, value: &Value) -> Result<(), String> {
    let name = xml_name(name);
    let io = |e: std::io::Error| e.to_string();
    match value {
        Value::Array(items) => {
            for item in items {
                write_element(w, &name, item)?;
            }
            Ok(())
        }
        Value::Object(m) => {
            let mut start = BytesStart::new(name.as_str());
            for (k, v) in m.iter().filter(|(k, _)| k.starts_with('@')) {
                start.push_attribute((xml_name(&k[1..]).as_str(), xml_clean(&cell(v)).as_str()));
            }
            let children: Vec<&(String, Value)> = m
                .iter()
                .filter(|(k, _)| !k.starts_with('@') && k.as_str() != "#text")
                .collect();
            let text = m
                .iter()
                .find(|(k, _)| k == "#text")
                .map(|(_, v)| xml_clean(&cell(v)));
            if children.is_empty() && text.is_none() {
                return w.write_event(Event::Empty(start)).map_err(io);
            }
            w.write_event(Event::Start(start)).map_err(io)?;
            if let Some(t) = text {
                w.write_event(Event::Text(BytesText::new(&t))).map_err(io)?;
            }
            for (k, v) in children {
                write_element(w, k, v)?;
            }
            w.write_event(Event::End(BytesEnd::new(name.as_str())))
                .map_err(io)
        }
        Value::Null => w
            .write_event(Event::Empty(BytesStart::new(name.as_str())))
            .map_err(io),
        scalar => {
            w.write_event(Event::Start(BytesStart::new(name.as_str())))
                .map_err(io)?;
            w.write_event(Event::Text(BytesText::new(&xml_clean(&cell(scalar)))))
                .map_err(io)?;
            w.write_event(Event::End(BytesEnd::new(name.as_str())))
                .map_err(io)
        }
    }
}

fn write_xml(value: &Value, pretty: bool) -> Result<Vec<u8>, String> {
    let mut w = if pretty {
        Writer::new_with_indent(Vec::new(), b' ', 2)
    } else {
        Writer::new(Vec::new())
    };
    w.write_event(Event::Decl(BytesDecl::new("1.0", Some("UTF-8"), None)))
        .map_err(|e| e.to_string())?;
    match value {
        Value::Object(m) if m.len() == 1 && !m[0].1.is_array() => {
            write_element(&mut w, &m[0].0, &m[0].1)?;
        }
        Value::Array(items) => {
            let wrapped = Value::Object(vec![("item".to_string(), Value::Array(items.clone()))]);
            write_element(&mut w, "root", &wrapped)?;
        }
        other => write_element(&mut w, "root", other)?,
    }
    let mut out = w.into_inner();
    out.push(b'\n');
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opts() -> ConvertOptions {
        ConvertOptions::default()
    }

    fn j(s: &str) -> Value {
        read_value(s.as_bytes(), "json").unwrap()
    }

    fn keys(v: &Value) -> Vec<&str> {
        match v {
            Value::Object(m) => m.iter().map(|(k, _)| k.as_str()).collect(),
            _ => vec![],
        }
    }

    #[test]
    fn json_yaml_json_round_trip() {
        let src = j(
            r#"{"name": "omni", "n": 3, "f": 1.5, "tags": ["a", "b"], "nested": {"ok": true, "none": null}}"#,
        );
        let yaml = write_value(&src, "yaml", &opts()).unwrap();
        let back = read_value(&yaml, "yaml").unwrap();
        assert_eq!(back, src);
        let out = write_value(&back, "json", &opts()).unwrap();
        assert_eq!(read_value(&out, "json").unwrap(), src);
    }

    #[test]
    fn keeps_source_key_order_unless_sorting() {
        let src = j(r#"{"zeta": 1, "alpha": 2, "mid": {"z": 1, "a": 2}}"#);
        let yaml = String::from_utf8(write_value(&src, "yaml", &opts()).unwrap()).unwrap();
        assert!(yaml.find("zeta").unwrap() < yaml.find("alpha").unwrap());
        let back = read_value(yaml.as_bytes(), "yaml").unwrap();
        assert_eq!(keys(&back), vec!["zeta", "alpha", "mid"]);
        let toml = write_value(&src, "toml", &opts()).unwrap();
        assert_eq!(
            keys(&read_value(&toml, "toml").unwrap()),
            vec!["zeta", "alpha", "mid"]
        );
        let o = ConvertOptions {
            sort_keys: Some(true),
            ..Default::default()
        };
        let sorted = String::from_utf8(write_value(&src, "yaml", &o).unwrap()).unwrap();
        assert!(sorted.find("alpha").unwrap() < sorted.find("zeta").unwrap());
        let csv = b"name,age,city\nAna,30,Rio\n";
        let tsv = String::from_utf8(
            write_value(&read_value(csv, "csv").unwrap(), "tsv", &opts()).unwrap(),
        )
        .unwrap();
        assert_eq!(tsv.lines().next().unwrap(), "name\tage\tcity");
    }

    #[test]
    fn csv_json_round_trip() {
        let csv = b"name,age,city\nAna,30,\"Rio, RJ\"\nBo,007,SP\n";
        let v = read_value(csv, "csv").unwrap();
        assert_eq!(v[0]["age"], Value::from(30));
        assert_eq!(v[1]["age"], Value::str("007"));
        assert_eq!(v[0]["city"], Value::str("Rio, RJ"));
        let out = write_value(&v, "csv", &opts()).unwrap();
        assert_eq!(read_value(&out, "csv").unwrap(), v);
    }

    #[test]
    fn csv_duplicate_headers_are_suffixed() {
        let v = read_value(b"a,b,a,a\n1,2,3,4\n", "csv").unwrap();
        assert_eq!(keys(&v[0]), vec!["a", "b", "a_2", "a_3"]);
        assert_eq!(v[0]["a_3"], Value::from(4));
    }

    #[test]
    fn csv_header_union_and_nested() {
        let v = j(r#"[{"a": 1}, {"b": {"x": 1}, "a": 2}]"#);
        let out = String::from_utf8(write_value(&v, "tsv", &opts()).unwrap()).unwrap();
        assert_eq!(out.lines().next().unwrap(), "a\tb");
        assert!(out.contains("{\"\"x\"\":1}") || out.contains("{\"x\":1}"));
    }

    #[test]
    fn tabular_output_descends_wrappers() {
        let xml =
            b"<rows><row><id>1</id><name>a</name></row><row><id>2</id><name>b</name></row></rows>";
        let out = String::from_utf8(
            write_value(&read_value(xml, "xml").unwrap(), "csv", &opts()).unwrap(),
        )
        .unwrap();
        assert_eq!(out, "id,name\n1,a\n2,b\n");
        let nested = j(r#"{"data": {"items": [{"k": 1}, {"k": 2}]}}"#);
        let out = String::from_utf8(write_value(&nested, "csv", &opts()).unwrap()).unwrap();
        assert_eq!(out, "k\n1\n2\n");
    }

    #[test]
    fn xml_json_round_trip() {
        let xml = br#"<?xml version="1.0"?><lib id="1"><book lang="pt">Dom &amp; Casmurro</book><book>Iracema</book><empty/></lib>"#;
        let v = read_value(xml, "xml").unwrap();
        assert_eq!(v["lib"]["@id"], Value::str("1"));
        assert_eq!(v["lib"]["book"][0]["#text"], Value::str("Dom & Casmurro"));
        assert_eq!(v["lib"]["book"][1], Value::str("Iracema"));
        assert_eq!(v["lib"]["empty"], Value::Null);
        let out = write_value(&v, "xml", &opts()).unwrap();
        assert_eq!(read_value(&out, "xml").unwrap(), v);
    }

    #[test]
    fn xml_output_strips_invalid_control_chars() {
        let v = j(r#"{"a": {"@k": "x\u0001y", "b": "line\u0000\tok\u000b"}}"#);
        let out = String::from_utf8(write_value(&v, "xml", &opts()).unwrap()).unwrap();
        assert!(!out.contains('\u{0}') && !out.contains('\u{1}') && !out.contains('\u{b}'));
        let back = read_value(out.as_bytes(), "xml").unwrap();
        assert_eq!(back["a"]["@k"], Value::str("xy"));
        assert_eq!(back["a"]["b"], Value::str("line\tok"));
    }

    #[test]
    fn toml_wraps_arrays_and_drops_nulls() {
        let v = j(r#"[{"a": 1, "b": null}]"#);
        let out = write_value(&v, "toml", &opts()).unwrap();
        let back = read_value(&out, "toml").unwrap();
        assert_eq!(back, j(r#"{"items": [{"a": 1}]}"#));
        let src = j(r#"{"title": "x", "server": {"port": 80}, "list": [1, 2]}"#);
        let t = write_value(&src, "toml", &opts()).unwrap();
        let back = read_value(&t, "toml").unwrap();
        assert_eq!(back["server"]["port"], Value::from(80));
        assert_eq!(back["list"], j("[1, 2]"));
        assert_eq!(back["title"], Value::str("x"));
    }

    #[test]
    fn plist_and_jsonl_round_trip() {
        let src = j(r#"{"b": [1, 2], "a": "c", "d": true}"#);
        let p = write_value(&src, "plist", &opts()).unwrap();
        assert_eq!(read_value(&p, "plist").unwrap(), src);
        let lines = j(r#"[{"a": 1}, {"a": 2}]"#);
        let l = write_value(&lines, "jsonl", &opts()).unwrap();
        assert_eq!(String::from_utf8(l.clone()).unwrap().lines().count(), 2);
        assert_eq!(read_value(&l, "jsonl").unwrap(), lines);
    }

    #[test]
    fn sort_keys_and_compact() {
        let src = j(r#"{"b": 1, "a": {"d": 1, "c": 2}}"#);
        let compact = ConvertOptions {
            pretty: Some(false),
            ..Default::default()
        };
        let out = String::from_utf8(write_value(&src, "json", &compact).unwrap()).unwrap();
        assert_eq!(out.trim(), r#"{"b":1,"a":{"d":1,"c":2}}"#);
        let o = ConvertOptions {
            pretty: Some(false),
            sort_keys: Some(true),
            ..Default::default()
        };
        let out = String::from_utf8(write_value(&src, "json", &o).unwrap()).unwrap();
        assert_eq!(out.trim(), r#"{"a":{"c":2,"d":1},"b":1}"#);
    }

    #[test]
    fn invalid_input_says_why() {
        let err = read_value(b"{nope", "json").unwrap_err();
        assert!(err.starts_with("This file is not valid JSON"));
    }
}

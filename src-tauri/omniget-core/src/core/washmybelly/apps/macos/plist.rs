//! Property lists, XML and binary (`bplist00`), read into `serde_json::Value`
//! without spawning `plutil`. Pure functions, compiled and tested everywhere.
//!
//! Mapping: dict → object, array/set → array, string → string, integer and
//! uid → number, real → number, true/false → bool, date → ISO 8601 string,
//! data → `{"$data": <length>}` (nothing here needs the bytes).

use serde_json::{Map, Number, Value};

const MAX_DEPTH: usize = 64;

/// Parse either format, by the magic at the start.
pub fn parse(bytes: &[u8]) -> Result<Value, String> {
    if bytes.starts_with(b"bplist00") {
        parse_binary(bytes)
    } else {
        let text = std::str::from_utf8(bytes).map_err(|_| "plist: not UTF-8".to_string())?;
        parse_xml(text)
    }
}

/// Read and parse a plist file. A missing or unreadable file is an error.
pub fn read(path: &std::path::Path) -> Result<Value, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    parse(&bytes).map_err(|e| format!("{}: {e}", path.display()))
}

// ── XML ──────────────────────────────────────────────────────────────

pub fn parse_xml(text: &str) -> Result<Value, String> {
    let opts = roxmltree::ParsingOptions {
        allow_dtd: true,
        ..Default::default()
    };
    let doc =
        roxmltree::Document::parse_with_options(text, opts).map_err(|e| format!("plist: {e}"))?;
    let root = doc.root_element();
    let node = if root.has_tag_name("plist") {
        root.children()
            .find(|n| n.is_element())
            .ok_or("plist: empty")?
    } else {
        root
    };
    xml_value(node, 0)
}

fn xml_value(n: roxmltree::Node, depth: usize) -> Result<Value, String> {
    if depth > MAX_DEPTH {
        return Err("plist: too deep".into());
    }
    let text = || n.text().unwrap_or("").to_string();
    Ok(match n.tag_name().name() {
        "dict" => {
            let mut map = Map::new();
            let mut key: Option<String> = None;
            for c in n.children().filter(|c| c.is_element()) {
                if c.has_tag_name("key") {
                    key = Some(c.text().unwrap_or("").to_string());
                } else if let Some(k) = key.take() {
                    map.insert(k, xml_value(c, depth + 1)?);
                }
            }
            Value::Object(map)
        }
        "array" => Value::Array(
            n.children()
                .filter(|c| c.is_element())
                .map(|c| xml_value(c, depth + 1))
                .collect::<Result<_, _>>()?,
        ),
        "string" => Value::String(text()),
        "integer" => {
            let t = text();
            let t = t.trim();
            t.parse::<i64>()
                .map(Value::from)
                .or_else(|_| t.parse::<u64>().map(Value::from))
                .unwrap_or(Value::Null)
        }
        "real" => text()
            .trim()
            .parse::<f64>()
            .ok()
            .and_then(Number::from_f64)
            .map(Value::Number)
            .unwrap_or(Value::Null),
        "true" => Value::Bool(true),
        "false" => Value::Bool(false),
        "date" => Value::String(text().trim().to_string()),
        "data" => {
            let len = text().chars().filter(|c| !c.is_whitespace()).count() * 3 / 4;
            serde_json::json!({ "$data": len })
        }
        _ => Value::Null,
    })
}

// ── Binary ───────────────────────────────────────────────────────────

struct Bin<'a> {
    b: &'a [u8],
    offsets: Vec<usize>,
    ref_size: usize,
}

fn be(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0u64, |acc, &x| (acc << 8) | x as u64)
}

pub fn parse_binary(b: &[u8]) -> Result<Value, String> {
    if b.len() < 40 || !b.starts_with(b"bplist00") {
        return Err("bplist: too short".into());
    }
    let t = &b[b.len() - 32..];
    let off_size = t[6] as usize;
    let ref_size = t[7] as usize;
    let count = be(&t[8..16]) as usize;
    let top = be(&t[16..24]) as usize;
    let table = be(&t[24..32]) as usize;
    if !(1..=8).contains(&off_size) || !(1..=8).contains(&ref_size) {
        return Err("bplist: bad trailer".into());
    }
    let end = count
        .checked_mul(off_size)
        .and_then(|n| n.checked_add(table))
        .ok_or("bplist: bad table")?;
    if end > b.len() - 32 || top >= count {
        return Err("bplist: bad table".into());
    }
    let offsets = (0..count)
        .map(|i| be(&b[table + i * off_size..table + (i + 1) * off_size]) as usize)
        .collect();
    let p = Bin {
        b,
        offsets,
        ref_size,
    };
    p.object(top, 0)
}

impl Bin<'_> {
    fn slice(&self, at: usize, len: usize) -> Result<&[u8], String> {
        self.b
            .get(at..at.checked_add(len).ok_or("bplist: overflow")?)
            .ok_or_else(|| "bplist: out of bounds".to_string())
    }

    /// Length of a collection/string and where its payload starts.
    fn len_at(&self, at: usize, lo: u8) -> Result<(usize, usize), String> {
        if lo != 0xF {
            return Ok((lo as usize, at + 1));
        }
        let m = *self.b.get(at + 1).ok_or("bplist: out of bounds")?;
        if m >> 4 != 0x1 {
            return Err("bplist: bad length".into());
        }
        let n = 1usize << (m & 0xF);
        if n > 8 {
            return Err("bplist: bad length".into());
        }
        Ok((be(self.slice(at + 2, n)?) as usize, at + 2 + n))
    }

    fn reference(&self, at: usize, i: usize) -> Result<usize, String> {
        Ok(be(self.slice(at + i * self.ref_size, self.ref_size)?) as usize)
    }

    fn object(&self, idx: usize, depth: usize) -> Result<Value, String> {
        if depth > MAX_DEPTH {
            return Err("bplist: too deep".into());
        }
        let at = *self.offsets.get(idx).ok_or("bplist: bad reference")?;
        let marker = *self.b.get(at).ok_or("bplist: out of bounds")?;
        let (hi, lo) = (marker >> 4, marker & 0xF);
        Ok(match hi {
            0x0 => match lo {
                0x8 => Value::Bool(false),
                0x9 => Value::Bool(true),
                _ => Value::Null,
            },
            0x1 => {
                let n = 1usize << lo;
                let raw = self.slice(at + 1, n)?;
                // 16-byte integers keep their low 8 bytes.
                let raw = &raw[raw.len().saturating_sub(8)..];
                let v = be(raw);
                if n >= 8 {
                    Value::from(v as i64)
                } else {
                    Value::from(v)
                }
            }
            0x2 => {
                let n = 1usize << lo;
                let raw = self.slice(at + 1, n)?;
                let f = match n {
                    4 => f32::from_be_bytes(raw.try_into().map_err(|_| "bplist: real")?) as f64,
                    8 => f64::from_be_bytes(raw.try_into().map_err(|_| "bplist: real")?),
                    _ => return Err("bplist: bad real".into()),
                };
                Number::from_f64(f)
                    .map(Value::Number)
                    .unwrap_or(Value::Null)
            }
            0x3 => {
                let raw = self.slice(at + 1, 8)?;
                let secs = f64::from_be_bytes(raw.try_into().map_err(|_| "bplist: date")?);
                // Seconds since 2001-01-01T00:00:00Z.
                let unix = 978_307_200i64 + secs.floor() as i64;
                chrono::DateTime::from_timestamp(unix, 0)
                    .map(|d| Value::String(d.format("%Y-%m-%dT%H:%M:%SZ").to_string()))
                    .unwrap_or(Value::Null)
            }
            0x4 => {
                let (len, _) = self.len_at(at, lo)?;
                serde_json::json!({ "$data": len })
            }
            0x5 => {
                let (len, start) = self.len_at(at, lo)?;
                Value::String(String::from_utf8_lossy(self.slice(start, len)?).into_owned())
            }
            0x6 => {
                let (len, start) = self.len_at(at, lo)?;
                let raw = self.slice(start, len.checked_mul(2).ok_or("bplist: overflow")?)?;
                let units: Vec<u16> = raw
                    .chunks_exact(2)
                    .map(|c| u16::from_be_bytes([c[0], c[1]]))
                    .collect();
                Value::String(String::from_utf16_lossy(&units))
            }
            0x8 => Value::from(be(self.slice(at + 1, lo as usize + 1)?)),
            0xA | 0xC => {
                let (len, start) = self.len_at(at, lo)?;
                let mut out = Vec::with_capacity(len.min(4096));
                for i in 0..len {
                    out.push(self.object(self.reference(start, i)?, depth + 1)?);
                }
                Value::Array(out)
            }
            0xD => {
                let (len, start) = self.len_at(at, lo)?;
                let mut map = Map::new();
                for i in 0..len {
                    let k = self.object(self.reference(start, i)?, depth + 1)?;
                    let v = self.object(self.reference(start, len + i)?, depth + 1)?;
                    let key = match k {
                        Value::String(s) => s,
                        other => other.to_string(),
                    };
                    map.insert(key, v);
                }
                Value::Object(map)
            }
            _ => Value::Null,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FX: &str = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/washmybelly_fixtures/apps_macos/"
    );

    fn fixture(name: &str) -> Vec<u8> {
        std::fs::read(format!("{FX}{name}")).unwrap()
    }

    #[test]
    fn binary_and_xml_give_the_same_value() {
        let bin = parse(&fixture("info_spotify.bplist")).unwrap();
        let xml = parse(&fixture("info_spotify.xml")).unwrap();
        assert_eq!(bin["CFBundleIdentifier"], "com.spotify.client");
        assert_eq!(bin, xml);
    }

    #[test]
    fn xml_info_plist_fields() {
        let v = parse(&fixture("info_mullvad.plist")).unwrap();
        assert_eq!(v["CFBundleIdentifier"], "net.mullvad.vpn");
        assert!(v["CFBundleShortVersionString"].is_string());
    }

    #[test]
    fn garbage_is_an_error_not_a_panic() {
        assert!(parse(b"bplist00\x00\x01").is_err());
        let mut junk = b"bplist00".to_vec();
        junk.extend(std::iter::repeat_n(0xFFu8, 64));
        assert!(parse(&junk).is_err());
        assert!(parse(b"<plist><dict><key>a</key>").is_err());
    }
}

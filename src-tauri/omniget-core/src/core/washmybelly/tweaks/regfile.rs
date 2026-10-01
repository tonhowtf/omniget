use super::catalog::{Hive, TweakOp};

pub fn decode(bytes: &[u8]) -> Result<String, String> {
    if let Some(rest) = bytes.strip_prefix(&[0xFF, 0xFE]) {
        return utf16(rest, u16::from_le_bytes);
    }
    if let Some(rest) = bytes.strip_prefix(&[0xFE, 0xFF]) {
        return utf16(rest, u16::from_be_bytes);
    }
    let rest = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(bytes);
    String::from_utf8(rest.to_vec()).map_err(|e| format!(".reg: not UTF-8 or UTF-16: {e}"))
}

fn utf16(bytes: &[u8], f: fn([u8; 2]) -> u16) -> Result<String, String> {
    if !bytes.len().is_multiple_of(2) {
        return Err(".reg: odd UTF-16 length".into());
    }
    let units: Vec<u16> = bytes.chunks_exact(2).map(|c| f([c[0], c[1]])).collect();
    String::from_utf16(&units).map_err(|e| format!(".reg: bad UTF-16: {e}"))
}

pub fn split_key(full: &str) -> Result<(Hive, String), String> {
    let (root, rest) = full
        .split_once('\\')
        .ok_or_else(|| format!(".reg: key without hive: {full}"))?;
    let rest = rest.trim_end_matches('\\').to_string();
    match root.to_ascii_uppercase().as_str() {
        "HKEY_CURRENT_USER" | "HKCU" => Ok((Hive::Hkcu, rest)),
        "HKEY_LOCAL_MACHINE" | "HKLM" => Ok((Hive::Hklm, rest)),
        "HKEY_CLASSES_ROOT" | "HKCR" => Ok((Hive::Hklm, format!("SOFTWARE\\Classes\\{rest}"))),
        "HKEY_USERS" | "HKU" => Ok((Hive::Hku, rest)),
        other => Err(format!(".reg: unsupported hive {other}")),
    }
}

fn lines(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    for raw in text.lines() {
        let l = raw.trim();
        if cur.is_empty() && (l.is_empty() || l.starts_with(';')) {
            continue;
        }
        if let Some(head) = l.strip_suffix('\\') {
            if !l.starts_with('[') {
                cur.push_str(head);
                continue;
            }
        }
        cur.push_str(l);
        out.push(std::mem::take(&mut cur));
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

fn quoted(s: &str) -> Result<(String, &str), String> {
    let body = s.strip_prefix('"').ok_or(".reg: expected a quote")?;
    let mut out = String::new();
    let mut chars = body.char_indices();
    while let Some((i, c)) = chars.next() {
        match c {
            '\\' => match chars.next() {
                Some((_, n)) => out.push(n),
                None => return Err(".reg: dangling escape".into()),
            },
            '"' => return Ok((out, &body[i + 1..])),
            c => out.push(c),
        }
    }
    Err(".reg: unterminated string".into())
}

fn hex_bytes(list: &str) -> Result<Vec<u8>, String> {
    list.split(',')
        .map(str::trim)
        .filter(|b| !b.is_empty())
        .map(|b| u8::from_str_radix(b, 16).map_err(|_| format!(".reg: bad hex byte {b}")))
        .collect()
}

fn utf16le_text(bytes: &[u8]) -> Result<String, String> {
    let s = utf16(bytes, u16::from_le_bytes)?;
    Ok(s.trim_end_matches('\0').to_string())
}

fn value(rhs: &str) -> Result<Option<(String, String)>, String> {
    let rhs = rhs.trim();
    if rhs == "-" {
        return Ok(None);
    }
    if rhs.starts_with('"') {
        let (s, rest) = quoted(rhs)?;
        if !rest.trim().is_empty() {
            return Err(format!(".reg: text after a string: {rest}"));
        }
        return Ok(Some(("REG_SZ".into(), s)));
    }
    if let Some(h) = rhs.strip_prefix("dword:") {
        let v = u32::from_str_radix(h.trim(), 16).map_err(|_| format!(".reg: bad dword {h}"))?;
        return Ok(Some(("REG_DWORD".into(), v.to_string())));
    }
    if let Some(h) = rhs.strip_prefix("hex:") {
        let b = hex_bytes(h)?;
        return Ok(Some((
            "REG_BINARY".into(),
            b.iter().map(|x| format!("{x:02X}")).collect(),
        )));
    }
    if let Some(rest) = rhs.strip_prefix("hex(") {
        let (t, list) = rest.split_once("):").ok_or(".reg: bad hex() value")?;
        let b = hex_bytes(list)?;
        return match t.to_ascii_lowercase().as_str() {
            "2" => Ok(Some(("REG_EXPAND_SZ".into(), utf16le_text(&b)?))),
            "7" => {
                let s = utf16le_text(&b)?;
                let parts: Vec<&str> = s.split('\0').filter(|p| !p.is_empty()).collect();
                Ok(Some(("REG_MULTI_SZ".into(), parts.join("\\0"))))
            }
            "b" => {
                if b.len() != 8 {
                    return Err(".reg: qword needs 8 bytes".into());
                }
                let mut a = [0u8; 8];
                a.copy_from_slice(&b);
                Ok(Some((
                    "REG_QWORD".into(),
                    u64::from_le_bytes(a).to_string(),
                )))
            }
            "4" => {
                if b.len() != 4 {
                    return Err(".reg: dword needs 4 bytes".into());
                }
                Ok(Some((
                    "REG_DWORD".into(),
                    u32::from_le_bytes([b[0], b[1], b[2], b[3]]).to_string(),
                )))
            }
            other => Err(format!(".reg: unsupported value type hex({other})")),
        };
    }
    Err(format!(".reg: unsupported value {rhs}"))
}

pub fn parse(text: &str) -> Result<Vec<TweakOp>, String> {
    let mut ops = Vec::new();
    let mut current: Option<(Hive, String)> = None;
    for l in lines(text) {
        if l.starts_with("Windows Registry Editor") || l == "REGEDIT4" {
            continue;
        }
        if let Some(inner) = l.strip_prefix('[').and_then(|x| x.strip_suffix(']')) {
            if let Some(del) = inner.strip_prefix('-') {
                let (hive, key) = split_key(del)?;
                ops.push(TweakOp::RegDelete {
                    hive,
                    key,
                    name: None,
                });
                current = None;
            } else {
                current = Some(split_key(inner)?);
            }
            continue;
        }
        let (hive, key) = current
            .clone()
            .ok_or_else(|| format!(".reg: value outside a key: {l}"))?;
        let (name, rhs) = if let Some(rest) = l.strip_prefix('@') {
            (
                String::new(),
                rest.trim_start()
                    .strip_prefix('=')
                    .ok_or(".reg: expected = after @")?,
            )
        } else {
            let (n, rest) = quoted(&l)?;
            (
                n,
                rest.trim_start()
                    .strip_prefix('=')
                    .ok_or(".reg: expected = after a name")?,
            )
        };
        match value(rhs)? {
            None => ops.push(TweakOp::RegDelete {
                hive,
                key,
                name: Some(name),
            }),
            Some((kind, data)) => ops.push(TweakOp::RegSet {
                hive,
                key,
                name,
                kind,
                data,
                absent_means: None,
            }),
        }
    }
    Ok(ops)
}

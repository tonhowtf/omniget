//! `.reg` files (Windows Registry Editor 5.00 / REGEDIT4) as typed ops.
//!
//! Win11Debloat ships its apply and undo files as they are: some in UTF-16 LE
//! with a BOM, others in UTF-8. [`decode`] takes both; [`parse`] turns the
//! text into [`TweakOp`]s. `HKEY_CLASSES_ROOT` becomes
//! `HKLM\SOFTWARE\Classes`, where an administrator's write lands anyway.

use super::catalog::{Hive, TweakOp};

/// Bytes of a `.reg` file as text: UTF-16 LE/BE with BOM, UTF-8 with or
/// without BOM.
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

/// `HKEY_*\rest` → our hive and the key under it.
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

/// Logical lines: comments dropped, `\` continuations joined.
fn lines(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    for raw in text.lines() {
        let l = raw.trim();
        if cur.is_empty() && (l.is_empty() || l.starts_with(';')) {
            continue;
        }
        if let Some(head) = l.strip_suffix('\\') {
            // A key line may end in `\` only as part of a name; values use
            // it for continuation (hex lists).
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

/// A quoted `.reg` string at the start of `s`: returns the text and the rest.
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

/// A value's right-hand side as (kind, data in our text form).
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

/// Typed ops, in file order. `[-KEY]` deletes a key, `"name"=-` a value.
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

#[cfg(test)]
mod tests {
    use super::*;

    const FIX: &str = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/washmybelly_fixtures/tweaks/"
    );

    fn fixture(name: &str) -> Vec<u8> {
        std::fs::read(format!("{FIX}{name}")).unwrap()
    }

    #[test]
    fn utf16_with_bom_and_utf8_read_the_same() {
        let a = decode(&fixture("utf16le_bom.reg")).unwrap();
        let b = decode(&fixture("utf8.reg")).unwrap();
        let c = decode(&fixture("utf8_bom.reg")).unwrap();
        let pa = parse(&a).unwrap();
        assert_eq!(pa, parse(&b).unwrap());
        assert_eq!(pa, parse(&c).unwrap());
        assert_eq!(pa.len(), 4);
    }

    #[test]
    fn every_value_form_becomes_a_typed_op() {
        let ops = parse(&decode(&fixture("all_forms.reg")).unwrap()).unwrap();
        let find = |n: &str| {
            ops.iter().find(|o| matches!(o, TweakOp::RegSet { name, .. } | TweakOp::RegDelete { name: Some(name), .. } if name == n)).cloned()
        };
        assert!(
            matches!(find("Dw"), Some(TweakOp::RegSet { kind, data, .. }) if kind == "REG_DWORD" && data == "4294967295")
        );
        assert!(
            matches!(find("Bin"), Some(TweakOp::RegSet { kind, data, .. }) if kind == "REG_BINARY" && data == "9012078010000000")
        );
        assert!(
            matches!(find("Exp"), Some(TweakOp::RegSet { kind, data, .. }) if kind == "REG_EXPAND_SZ" && data == "%SystemRoot%")
        );
        assert!(
            matches!(find("Multi"), Some(TweakOp::RegSet { kind, data, .. }) if kind == "REG_MULTI_SZ" && data == "a\\0b")
        );
        assert!(
            matches!(find("Q"), Some(TweakOp::RegSet { kind, data, .. }) if kind == "REG_QWORD" && data == "1")
        );
        assert!(
            matches!(find("Quote"), Some(TweakOp::RegSet { data, .. }) if data == "say \"hi\" C:\\x")
        );
        assert!(matches!(find("Gone"), Some(TweakOp::RegDelete { .. })));
        // The default value, and a key delete of HKCR mapped under HKLM.
        assert!(ops.iter().any(|o| matches!(o, TweakOp::RegSet { name, data, .. } if name.is_empty() && data == "{f81e9010-6ea4-11ce-a7ff-00aa003ca9f6}")));
        assert!(ops.iter().any(|o| matches!(o, TweakOp::RegDelete { hive: Hive::Hklm, key, name: None } if key == "SOFTWARE\\Classes\\*\\shellex\\ContextMenuHandlers\\Sharing")));
    }

    #[test]
    fn broken_files_are_errors() {
        assert!(
            parse("Windows Registry Editor Version 5.00\n\"x\"=dword:1\n").is_err(),
            "value outside a key"
        );
        assert!(parse("[HKEY_FOO\\x]\n").is_err(), "unknown hive");
        assert!(
            parse("[HKEY_CURRENT_USER\\x]\n\"a\"=hex(0):00\n").is_err(),
            "REG_NONE is not supported"
        );
        assert!(decode(&[0xFF, 0xFE, 0x41]).is_err(), "odd UTF-16");
    }
}

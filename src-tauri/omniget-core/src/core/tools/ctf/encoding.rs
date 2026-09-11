//! Codificações que aparecem o tempo todo: base64/32/16, URL, entidades
//! HTML, binário, decimal e morse. Tudo com ida e volta, e um "adivinhe o
//! que é isso" que tenta reconhecer antes de decodificar.

use serde::{Deserialize, Serialize};

const B32: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";

pub fn b32_encode(data: &[u8]) -> String {
    let mut out = String::new();
    for chunk in data.chunks(5) {
        let mut buf = [0u8; 5];
        buf[..chunk.len()].copy_from_slice(chunk);
        let n = u64::from_be_bytes([0, 0, 0, buf[0], buf[1], buf[2], buf[3], buf[4]]);
        let chars = (chunk.len() * 8).div_ceil(5);
        for i in 0..8 {
            if i < chars {
                out.push(B32[((n >> (35 - i * 5)) & 0x1F) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

pub fn b32_decode(text: &str) -> Option<Vec<u8>> {
    let clean: Vec<u8> = text
        .trim()
        .bytes()
        .filter(|b| !b.is_ascii_whitespace() && *b != b'=')
        .map(|b| b.to_ascii_uppercase())
        .collect();
    let mut bits = 0u32;
    let mut acc = 0u64;
    let mut out = Vec::new();
    for b in clean {
        let v = B32.iter().position(|c| *c == b)? as u64;
        acc = (acc << 5) | v;
        bits += 5;
        if bits >= 8 {
            bits -= 8;
            out.push(((acc >> bits) & 0xFF) as u8);
        }
    }
    Some(out)
}

const MORSE: &[(char, &str)] = &[
    ('a', ".-"),
    ('b', "-..."),
    ('c', "-.-."),
    ('d', "-.."),
    ('e', "."),
    ('f', "..-."),
    ('g', "--."),
    ('h', "...."),
    ('i', ".."),
    ('j', ".---"),
    ('k', "-.-"),
    ('l', ".-.."),
    ('m', "--"),
    ('n', "-."),
    ('o', "---"),
    ('p', ".--."),
    ('q', "--.-"),
    ('r', ".-."),
    ('s', "..."),
    ('t', "-"),
    ('u', "..-"),
    ('v', "...-"),
    ('w', ".--"),
    ('x', "-..-"),
    ('y', "-.--"),
    ('z', "--.."),
    ('0', "-----"),
    ('1', ".----"),
    ('2', "..---"),
    ('3', "...--"),
    ('4', "....-"),
    ('5', "....."),
    ('6', "-...."),
    ('7', "--..."),
    ('8', "---.."),
    ('9', "----."),
    ('.', ".-.-.-"),
    (',', "--..--"),
    ('?', "..--.."),
    ('!', "-.-.--"),
    ('/', "-..-."),
    ('-', "-....-"),
    ('(', "-.--."),
    (')', "-.--.-"),
];

pub fn morse_encode(text: &str) -> String {
    text.to_lowercase()
        .split_whitespace()
        .map(|word| {
            word.chars()
                .filter_map(|c| MORSE.iter().find(|(k, _)| *k == c).map(|(_, v)| *v))
                .collect::<Vec<_>>()
                .join(" ")
        })
        .collect::<Vec<_>>()
        .join(" / ")
}

pub fn morse_decode(text: &str) -> String {
    text.split('/')
        .map(|word| {
            word.split_whitespace()
                .filter_map(|code| MORSE.iter().find(|(_, v)| *v == code).map(|(k, _)| *k))
                .collect::<String>()
        })
        .filter(|w| !w.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

pub fn html_escape(text: &str) -> String {
    text.chars()
        .map(|c| match c {
            '&' => "&amp;".to_string(),
            '<' => "&lt;".to_string(),
            '>' => "&gt;".to_string(),
            '"' => "&quot;".to_string(),
            '\'' => "&#39;".to_string(),
            c => c.to_string(),
        })
        .collect()
}

pub fn html_unescape(text: &str) -> String {
    let mut s = text
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&apos;", "'")
        .replace("&nbsp;", " ");
    // Entidades numéricas: &#65; e &#x41;
    if let Ok(re) = regex::Regex::new(r"&#(x?)([0-9A-Fa-f]+);") {
        s = re
            .replace_all(&s, |c: &regex::Captures| {
                let radix = if &c[1] == "x" { 16 } else { 10 };
                u32::from_str_radix(&c[2], radix)
                    .ok()
                    .and_then(char::from_u32)
                    .map(|ch| ch.to_string())
                    .unwrap_or_else(|| c[0].to_string())
            })
            .to_string();
    }
    // O & vem por último para não desfazer as outras entidades.
    s.replace("&amp;", "&")
}

#[derive(Debug, Clone, Deserialize)]
pub struct EncodeOptions {
    pub input: String,
    /// base64 | base32 | hex | url | html | binary | decimal | morse
    pub format: String,
    #[serde(default)]
    pub decode: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct EncodeResult {
    pub output: String,
    pub bytes: usize,
    pub error: Option<String>,
}

pub fn convert(opts: &EncodeOptions) -> EncodeResult {
    use base64::Engine;
    let engine = base64::engine::general_purpose::STANDARD;
    let input = &opts.input;
    let fail = |e: &str| EncodeResult {
        output: String::new(),
        bytes: 0,
        error: Some(e.to_string()),
    };
    let ok = |s: String| EncodeResult {
        bytes: s.len(),
        output: s,
        error: None,
    };

    if opts.decode {
        let bytes: Result<Vec<u8>, String> = match opts.format.as_str() {
            "base64" => engine
                .decode(input.trim())
                .map_err(|e| format!("base64 inválido: {}", e)),
            "base32" => b32_decode(input).ok_or_else(|| "base32 inválido".into()),
            "hex" => hex::decode(input.trim().replace([' ', '\n', ':'], ""))
                .map_err(|e| format!("hex inválido: {}", e)),
            "url" => Ok(urlencoding::decode(input.trim())
                .map(|c| c.into_owned())
                .unwrap_or_else(|_| input.clone())
                .into_bytes()),
            "html" => Ok(html_unescape(input).into_bytes()),
            "binary" => input
                .split_whitespace()
                .map(|b| u8::from_str_radix(b, 2).map_err(|_| format!("binário inválido: {}", b)))
                .collect(),
            "decimal" => input
                .split(|c: char| !c.is_ascii_digit())
                .filter(|s| !s.is_empty())
                .map(|b| {
                    b.parse::<u8>()
                        .map_err(|_| format!("decimal inválido: {}", b))
                })
                .collect(),
            "morse" => Ok(morse_decode(input).into_bytes()),
            other => Err(format!("formato desconhecido: {}", other)),
        };
        match bytes {
            Ok(b) => ok(String::from_utf8_lossy(&b).to_string()),
            Err(e) => fail(&e),
        }
    } else {
        let data = input.as_bytes();
        ok(match opts.format.as_str() {
            "base64" => engine.encode(data),
            "base32" => b32_encode(data),
            "hex" => hex::encode(data),
            "url" => urlencoding::encode(input).to_string(),
            "html" => html_escape(input),
            "binary" => data
                .iter()
                .map(|b| format!("{:08b}", b))
                .collect::<Vec<_>>()
                .join(" "),
            "decimal" => data
                .iter()
                .map(|b| b.to_string())
                .collect::<Vec<_>>()
                .join(" "),
            "morse" => morse_encode(input),
            _ => return fail("formato desconhecido"),
        })
    }
}

/// Palpite de qual codificação é o texto, para não ter que tentar uma a uma.
pub fn detect(input: &str) -> Vec<String> {
    let s = input.trim();
    let mut out = Vec::new();
    if s.is_empty() {
        return out;
    }
    let no_ws: String = s.chars().filter(|c| !c.is_whitespace()).collect();
    if no_ws.chars().all(|c| c == '.' || c == '-' || c == '/') {
        out.push("morse".into());
    }
    if no_ws.chars().all(|c| c == '0' || c == '1') && no_ws.len().is_multiple_of(8) {
        out.push("binary".into());
    }
    if no_ws.chars().all(|c| c.is_ascii_hexdigit())
        && no_ws.len().is_multiple_of(2)
        && no_ws.len() >= 4
    {
        out.push("hex".into());
    }
    if no_ws.len() >= 8
        && no_ws
            .trim_end_matches('=')
            .chars()
            .all(|c| c.is_ascii_uppercase() || ('2'..='7').contains(&c))
    {
        out.push("base32".into());
    }
    if no_ws.len() >= 4
        && no_ws.len().is_multiple_of(4)
        && no_ws
            .trim_end_matches('=')
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '+' || c == '/')
    {
        out.push("base64".into());
    }
    if s.contains('%')
        && regex::Regex::new(r"%[0-9A-Fa-f]{2}")
            .map(|r| r.is_match(s))
            .unwrap_or(false)
    {
        out.push("url".into());
    }
    if s.contains('&') && s.contains(';') {
        out.push("html".into());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn round(format: &str, text: &str) -> String {
        let enc = convert(&EncodeOptions {
            input: text.into(),
            format: format.into(),
            decode: false,
        });
        assert!(enc.error.is_none(), "{}: {:?}", format, enc.error);
        let dec = convert(&EncodeOptions {
            input: enc.output,
            format: format.into(),
            decode: true,
        });
        assert!(dec.error.is_none(), "{}: {:?}", format, dec.error);
        dec.output
    }

    #[test]
    fn every_format_round_trips() {
        for f in [
            "base64", "base32", "hex", "url", "html", "binary", "decimal",
        ] {
            assert_eq!(round(f, "OmniGet 2026!"), "OmniGet 2026!", "formato {}", f);
        }
    }

    #[test]
    fn base32_matches_rfc_4648() {
        let e = |s: &str| b32_encode(s.as_bytes());
        assert_eq!(e(""), "");
        assert_eq!(e("f"), "MY======");
        assert_eq!(e("fo"), "MZXQ====");
        assert_eq!(e("foo"), "MZXW6===");
        assert_eq!(e("foob"), "MZXW6YQ=");
        assert_eq!(e("fooba"), "MZXW6YTB");
        assert_eq!(e("foobar"), "MZXW6YTBOI======");
        assert_eq!(b32_decode("MZXW6YTBOI======").unwrap(), b"foobar");
    }

    #[test]
    fn morse_round_trips_words() {
        let m = morse_encode("SOS agora");
        assert!(m.starts_with("... --- ..."), "{}", m);
        assert_eq!(morse_decode(&m), "sos agora");
    }

    #[test]
    fn html_entities_survive_the_ampersand() {
        let s = html_escape("<a href=\"x\">tom & jerry</a>");
        assert!(s.contains("&amp;") && s.contains("&lt;"));
        assert_eq!(html_unescape(&s), "<a href=\"x\">tom & jerry</a>");
        assert_eq!(html_unescape("&#65;&#x42;&amp;"), "AB&");
    }

    #[test]
    fn broken_input_reports_instead_of_panicking() {
        let r = convert(&EncodeOptions {
            input: "não é base64 ###".into(),
            format: "base64".into(),
            decode: true,
        });
        assert!(r.error.is_some());
        let r = convert(&EncodeOptions {
            input: "abc".into(),
            format: "esperanto".into(),
            decode: false,
        });
        assert!(r.error.is_some());
    }

    #[test]
    fn detection_narrows_the_field() {
        assert_eq!(detect("... --- ..."), vec!["morse"]);
        assert!(detect("01001000 01101001").contains(&"binary".to_string()));
        assert!(detect("MZXW6YTBOI======").contains(&"base32".to_string()));
        assert!(detect("SGVsbG8gbXVuZG8=").contains(&"base64".to_string()));
        assert!(detect("a%20b%21").contains(&"url".to_string()));
        assert!(detect("tom &amp; jerry").contains(&"html".to_string()));
        assert!(detect("").is_empty());
    }
}

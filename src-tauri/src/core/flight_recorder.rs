//! Caixa-preta: os ultimos N eventos do app, redigidos, para anexar a um bug
//! report.
//!
//! O `download_log` que ja existe e por download e some no restart. Isto e
//! global e atravessa downloads, plugins e bootstrap — o que falta quando
//! alguem diz "parou de funcionar" e nao sabe dizer quando comecou.
//!
//! O item aqui e a **redacao**, nao o buffer. Um despejo de log que vaza cookie
//! ou token e pior que nenhum despejo, porque o usuario cola em issue publica
//! achando que e seguro. Por isso a redacao e funcao pura e testada por classe
//! de segredo, e o buffer e so um `VecDeque`.

use std::collections::VecDeque;
use std::sync::{Mutex, OnceLock};

/// Quantos eventos ficam retidos. Alto o bastante para cobrir uma sessao de
/// download, baixo o bastante para caber num comentario de issue.
pub const CAPACITY: usize = 500;

/// Substitui todo segredo conhecido por um marcador estavel.
///
/// Blocklist e o inverso do que se quer para seguranca, mas aqui e inevitavel:
/// a entrada e log arbitrario de yt-dlp, ffmpeg e plugins, e nao existe forma de
/// enumerar o que e seguro. A mitigacao e cobrir por *classe* de segredo e
/// testar cada uma, em vez de por padrao literal.
pub fn redact(line: &str) -> String {
    // URLs first, by allowlist: a parameter name nobody predicted (`igsh=`,
    // `hdnts=`, `e=`) must not survive just because the blocklist below never
    // heard of it.
    redact_keyed(&redact_urls_in_text(line))
}

const REDACTED: &str = "[REDACTED]";

/// Query parameters that identify public content and never carry a
/// credential. Everything else keeps its name but loses its value.
const SAFE_QUERY_KEYS: &[&str] = &[
    // YouTube
    "v",
    "list",
    "t",
    "index",
    "start",
    "end",
    "time_continue",
    // generic public ids / pagination
    "id",
    "p",
    "page",
    "vid",
    "video_id",
    "playlist",
    "pl",
    "item_id",
    // Bilibili
    "bvid",
    "aid",
    "cid",
    "ep_id",
    "season_id",
    // Facebook
    "story_fbid",
    "fbid",
    // locale
    "hl",
    "lang",
];

/// Longest value kept even for a safe key: public ids are short, a value this
/// long under `id=` is a bearer blob wearing a harmless name.
const SAFE_VALUE_MAX: usize = 64;

/// Redacts one URL for persistence or display, by allowlist.
///
/// Keeps scheme, host, port, non-secret path segments and the values of
/// [`SAFE_QUERY_KEYS`]. Replaces userinfo with `[REDACTED]@`, replaces every other query value and
/// the whole fragment with `[REDACTED]`, and replaces path segments that look
/// like tokens (JWTs, long hex/base64 runs). The result is for humans and
/// logs: it is not executable and must never be fed back to a downloader.
///
/// Input that is not an absolute URL goes through [`redact`]-style keyed
/// redaction and loses anything after `?` or `#`.
pub fn redact_url(raw: &str) -> String {
    let trimmed = raw.trim();
    let Ok(parsed) = url::Url::parse(trimmed) else {
        return redact_non_url(trimmed);
    };
    if parsed.cannot_be_a_base() {
        return redact_non_url(trimmed);
    }
    let mut out = String::with_capacity(trimmed.len());
    out.push_str(parsed.scheme());
    out.push_str("://");
    // Credentials are gone, but say so: a retry from this string must know
    // it lost something (see `is_redacted_url`).
    if !parsed.username().is_empty() || parsed.password().is_some() {
        out.push_str(REDACTED);
        out.push('@');
    }
    if let Some(host) = parsed.host_str() {
        out.push_str(host);
    }
    if let Some(port) = parsed.port() {
        out.push(':');
        out.push_str(&port.to_string());
    }
    let path = parsed.path();
    let segments: Vec<String> = path
        .split('/')
        .map(|seg| {
            if segment_looks_like_token(seg) {
                REDACTED.to_string()
            } else {
                seg.to_string()
            }
        })
        .collect();
    out.push_str(&segments.join("/"));
    if let Some(query) = parsed.query() {
        out.push('?');
        out.push_str(&redact_query(query));
    }
    if parsed.fragment().is_some_and(|f| !f.is_empty()) {
        out.push('#');
        out.push_str(REDACTED);
    }
    out
}

fn redact_non_url(raw: &str) -> String {
    let cut = raw.find(['?', '#']).unwrap_or(raw.len());
    let mut out = redact_keyed(&raw[..cut]);
    if cut < raw.len() {
        out.push(raw.as_bytes()[cut] as char);
        out.push_str(REDACTED);
    }
    out
}

fn redact_query(query: &str) -> String {
    query
        .split('&')
        .filter(|pair| !pair.is_empty())
        .map(|pair| {
            let (raw_key, value) = match pair.split_once('=') {
                Some((k, v)) => (k, Some(v)),
                None => (pair, None),
            };
            let key = urlencoding::decode(raw_key)
                .map(|k| k.into_owned())
                .unwrap_or_else(|_| raw_key.to_string());
            let key_is_plain = !key.is_empty()
                && key.len() <= 40
                && key
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.' | '[' | ']'));
            if !key_is_plain {
                return REDACTED.to_string();
            }
            let safe = SAFE_QUERY_KEYS.iter().any(|s| s.eq_ignore_ascii_case(&key));
            match value {
                Some(v) if safe && v.len() <= SAFE_VALUE_MAX && !segment_looks_like_token(v) => {
                    format!("{raw_key}={v}")
                }
                Some(_) => format!("{raw_key}={REDACTED}"),
                // A bare key is either a flag or an opaque token; only
                // allowlisted flags survive.
                None if safe => raw_key.to_string(),
                None => REDACTED.to_string(),
            }
        })
        .collect::<Vec<_>>()
        .join("&")
}

/// A path segment or value that reads like a credential: a JWT, or an
/// unbroken alphanumeric run long enough to be a signature/hash. Public ids
/// (11-char YouTube ids, numeric snowflakes, UUIDs, slugs) stay.
fn segment_looks_like_token(seg: &str) -> bool {
    let decoded = urlencoding::decode(seg)
        .map(|s| s.into_owned())
        .unwrap_or_else(|_| seg.to_string());
    if decoded.starts_with("eyJ") && decoded.len() >= 20 {
        return true;
    }
    let mut run = 0usize;
    let mut has_digit = false;
    let mut has_alpha = false;
    for c in decoded.chars() {
        if c.is_ascii_alphanumeric() {
            run += 1;
            has_digit |= c.is_ascii_digit();
            has_alpha |= c.is_ascii_alphabetic();
            if run >= 24 && has_digit && has_alpha {
                return true;
            }
        } else {
            run = 0;
            has_digit = false;
            has_alpha = false;
        }
    }
    false
}

/// True when `url` came out of [`redact_url`] with something removed: it is
/// display-only and a download from it would fail. Callers that re-run a
/// download from a stored URL must refuse it with "link expired, paste again"
/// instead of sending `[REDACTED]` to a server.
pub fn is_redacted_url(url: &str) -> bool {
    url.contains(REDACTED)
}

/// Only the URL pass of [`redact`]: URLs inside `text` are redacted, the rest
/// (paths, flags) is left byte-identical. For shell commands shown to the user.
pub fn redact_urls(text: &str) -> String {
    redact_urls_in_text(text)
}

/// Rewrites every `http(s)://` URL inside free text with [`redact_url`].
fn redact_urls_in_text(text: &str) -> String {
    let lower = text.to_ascii_lowercase();
    let mut out = String::with_capacity(text.len());
    let mut cursor = 0usize;
    loop {
        let next = [
            lower[cursor..].find("https://"),
            lower[cursor..].find("http://"),
        ]
        .into_iter()
        .flatten()
        .min();
        let Some(rel) = next else { break };
        let start = cursor + rel;
        let mut end = text[start..]
            .find(|c: char| c.is_whitespace() || matches!(c, '"' | '\'' | '<' | '>' | '`'))
            .map(|n| start + n)
            .unwrap_or(text.len());
        // Sentence punctuation glued to the URL is not part of it, but the
        // `]` closing an earlier `[REDACTED]` is: trimming it would redact the
        // marker again and leave `[REDACTED]]` (N-1).
        while end > start
            && !text[start..end].ends_with(REDACTED)
            && matches!(
                text.as_bytes()[end - 1],
                b'.' | b',' | b';' | b':' | b')' | b']' | b'}'
            )
        {
            end -= 1;
        }
        out.push_str(&text[cursor..start]);
        out.push_str(&redact_url(&text[start..end]));
        cursor = end;
    }
    out.push_str(&text[cursor..]);
    out
}

/// The keyed blocklist pass alone, without the URL pass (avoids recursion
/// from [`redact_non_url`]).
fn redact_keyed(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut rest = line;
    while !rest.is_empty() {
        match next_secret(rest) {
            Some((start, len, replacement)) => {
                out.push_str(&rest[..start]);
                out.push_str(replacement);
                rest = &rest[start + len..];
            }
            None => {
                out.push_str(rest);
                break;
            }
        }
    }
    redact_home_paths(&out)
}

/// Acha o proximo segredo: `(offset, comprimento, substituto)`.
fn next_secret(hay: &str) -> Option<(usize, usize, &'static str)> {
    let lower = hay.to_ascii_lowercase();
    let mut best: Option<(usize, usize, &'static str)> = None;

    // Chave=valor, em query string, header ou linha de config.
    const KEYED: &[&str] = &[
        "cookie=",
        "cookies=",
        "token=",
        "access_token=",
        "refresh_token=",
        "api_key=",
        "apikey=",
        "key=",
        "password=",
        "passwd=",
        "secret=",
        "authorization=",
        "sessdata=",
        "bili_jct=",
        "auth=",
        "sig=",
        "signature=",
        "x-amz-credential=",
        "x-amz-security-token=",
        "x-goog-credential=",
        "policy=",
        "po_token=",
    ];
    for k in KEYED {
        if let Some(i) = lower.find(k) {
            let value_start = i + k.len();
            let value_len = hay[value_start..]
                .find(|c: char| c.is_whitespace() || c == '&' || c == ';' || c == '"' || c == '\'')
                .unwrap_or(hay.len() - value_start);
            if value_len > 0 {
                let cand = (value_start, value_len, "[REDACTED]");
                if best.is_none_or(|(b, _, _)| cand.0 < b) {
                    best = Some(cand);
                }
            }
        }
    }

    // Headers can contain several credentials; redact the complete value.
    for key in [
        "cookie:",
        "set-cookie:",
        "authorization:",
        "proxy-authorization:",
    ] {
        if let Some(i) = lower.find(key) {
            let start = i + key.len();
            let len = hay[start..].find(['\r', '\n']).unwrap_or(hay.len() - start);
            if len > 0 && best.is_none_or(|(b, _, _)| start < b) {
                best = Some((start, len, "[REDACTED]"));
            }
        }
    }
    for key in [
        "--password ",
        "--username ",
        "--proxy ",
        "--cookies ",
        "--add-header ",
    ] {
        if let Some(i) = lower.find(key) {
            let start = i + key.len();
            // Quoted shell arguments may contain spaces: discard the remaining
            // diagnostic command rather than retaining part of a credential.
            if start < hay.len() && best.is_none_or(|(b, _, _)| start < b) {
                best = Some((start, hay.len() - start, "[REDACTED]"));
            }
        }
    }
    // URL userinfo (including proxy passwords) is never diagnostic evidence.
    for (i, _) in lower.match_indices("://") {
        let start = i + 3;
        let end = hay[start..]
            .find(|c: char| c == '/' || c.is_whitespace())
            .map(|n| start + n)
            .unwrap_or(hay.len());
        if let Some(at) = hay[start..end].rfind('@') {
            if best.is_none_or(|(b, _, _)| start < b) {
                best = Some((start, at + 1, "[REDACTED]@"));
            }
        }
    }

    // Header `Authorization: Bearer <token>`.
    if let Some(i) = lower.find("bearer ") {
        let value_start = i + "bearer ".len();
        let value_len = hay[value_start..]
            .find(char::is_whitespace)
            .unwrap_or(hay.len() - value_start);
        if value_len > 0 {
            let cand = (value_start, value_len, "[REDACTED]");
            if best.is_none_or(|(b, _, _)| cand.0 < b) {
                best = Some(cand);
            }
        }
    }

    best
}

/// Troca o diretorio pessoal por `~`, nas tres convencoes.
///
/// Nome de usuario num path e identificavel e vaza sozinho — muita gente cola o
/// log sem olhar.
fn redact_home_paths(line: &str) -> String {
    let mut out = line.to_string();
    for prefix in ["/Users/", "/home/", "\\Users\\"] {
        while let Some(i) = out.find(prefix) {
            let after = i + prefix.len();
            let sep = if prefix.contains('\\') { '\\' } else { '/' };
            let end = out[after..]
                .find(|c: char| c == sep || c.is_whitespace())
                .map(|e| after + e)
                .unwrap_or(out.len());
            if end == after {
                break; // prefixo sem nome de usuario depois; nada a redigir
            }
            out.replace_range(i..end, "~");
        }
    }
    out
}

/// Buffer circular, sem estado global.
///
/// Extraido de proposito: com o buffer atras de um `static`, dois testes que
/// gravam eventos disputam a mesma estrutura e um deles falha de forma
/// intermitente. Testar a estrutura direto elimina a corrida em vez de
/// contorna-la com serializacao de teste.
#[derive(Debug)]
pub struct Recorder {
    events: VecDeque<String>,
    capacity: usize,
}

impl Recorder {
    pub fn new(capacity: usize) -> Self {
        Self {
            events: VecDeque::with_capacity(capacity.min(1024)),
            capacity: capacity.max(1),
        }
    }

    /// Grava um evento. A redacao acontece **na entrada**, nao no despejo: um
    /// segredo que entra em claro ja vazou para quem tiver acesso a memoria, e
    /// o despejo pode ser esquecido.
    pub fn record(&mut self, event: &str) {
        if self.events.len() >= self.capacity {
            self.events.pop_front();
        }
        self.events.push_back(redact(event));
    }

    /// Do mais antigo ao mais recente.
    pub fn dump(&self) -> Vec<String> {
        self.events.iter().cloned().collect()
    }

    pub fn clear(&mut self) {
        self.events.clear();
    }

    pub fn len(&self) -> usize {
        self.events.len()
    }

    pub fn is_empty(&self) -> bool {
        self.events.is_empty()
    }
}

static BUFFER: OnceLock<Mutex<Recorder>> = OnceLock::new();

fn buffer() -> &'static Mutex<Recorder> {
    BUFFER.get_or_init(|| Mutex::new(Recorder::new(CAPACITY)))
}

pub fn record(event: &str) {
    if let Ok(mut buf) = buffer().lock() {
        buf.record(event);
    }
}

pub fn dump() -> Vec<String> {
    buffer().lock().map(|b| b.dump()).unwrap_or_default()
}

pub fn clear() {
    if let Ok(mut buf) = buffer().lock() {
        buf.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_synthetic_credentials_at_ingress() {
        for input in [
            "Cookie: session=SYNTHETIC_SECRET; csrf=SYNTHETIC_SECRET",
            "Set-Cookie: session=SYNTHETIC_SECRET; HttpOnly",
            "Authorization: Basic SYNTHETIC_SECRET",
            "https://user:SYNTHETIC_SECRET@example.com/video",
            "--password SYNTHETIC_SECRET",
            "https://example.com/video?X-Amz-Credential=SYNTHETIC_SECRET",
            "İ ?token=SYNTHETIC_SECRET",
            "https://example.com then https://user:SYNTHETIC_SECRET@other.example/video",
        ] {
            let output = redact(input);
            assert!(!output.contains("SYNTHETIC_SECRET"), "{output}");
        }
    }

    #[test]
    fn redige_cookie_em_query_string() {
        let s = redact("GET /video?id=42&cookie=abc123def&x=1");
        assert!(!s.contains("abc123def"), "{s}");
        assert!(s.contains("cookie=[REDACTED]"), "{s}");
        // O resto da linha precisa sobreviver, senao o log perde utilidade.
        assert!(s.contains("id=42"), "{s}");
        assert!(s.contains("x=1"), "{s}");
    }

    #[test]
    fn redige_cada_classe_de_segredo() {
        let casos = [
            ("SESSDATA=deadbeef", "deadbeef"),
            ("bili_jct=abc99", "abc99"),
            ("Authorization: Bearer eyJhbGciOi", "eyJhbGciOi"),
            ("--api_key=sk-live-1234", "sk-live-1234"),
            ("password=hunter2", "hunter2"),
            ("access_token=ya29.a0Af", "ya29.a0Af"),
            ("?sig=9f8e7d6c", "9f8e7d6c"),
        ];
        for (entrada, segredo) in casos {
            let s = redact(entrada);
            assert!(!s.contains(segredo), "vazou {segredo:?} em {s:?}");
            assert!(s.contains("[REDACTED]"), "{s}");
        }
    }

    #[test]
    fn redige_o_nome_de_usuario_do_caminho() {
        // Path pessoal identifica a pessoa sozinho, e e o que mais aparece em
        // log colado em issue publica.
        for (entrada, proibido) in [
            ("/Users/tonho/Downloads/x.mp4", "tonho"),
            ("/home/maria/videos", "maria"),
            ("C:\\Users\\Joao\\AppData", "Joao"),
        ] {
            let s = redact(entrada);
            assert!(!s.contains(proibido), "vazou usuario em {s:?}");
            assert!(s.contains('~'), "{s}");
        }
    }

    #[test]
    fn linha_sem_segredo_atravessa_intacta() {
        // Redacao agressiva demais destroi o valor do log.
        let limpa = "[ytdlp] merging formats 137+140 into out.mp4";
        assert_eq!(redact(limpa), limpa);
    }

    #[test]
    fn multiplos_segredos_na_mesma_linha() {
        let s = redact("cookie=aaa&token=bbb&keep=ok");
        assert!(!s.contains("aaa") && !s.contains("bbb"), "{s}");
        assert!(s.contains("keep=ok"), "{s}");
        assert_eq!(s.matches("[REDACTED]").count(), 2, "{s}");
    }

    #[test]
    fn o_buffer_descarta_o_mais_antigo_e_preserva_a_ordem() {
        let mut r = Recorder::new(5);
        for i in 0..15 {
            r.record(&format!("evento {i}"));
        }
        let d = r.dump();
        assert_eq!(d.len(), 5);
        assert_eq!(d.first().unwrap(), "evento 10");
        assert_eq!(d.last().unwrap(), "evento 14");
    }

    #[test]
    fn o_buffer_guarda_redigido_e_nao_o_original() {
        // Redigir so no despejo deixaria o segredo em memoria e dependeria de
        // ninguem esquecer de chamar a redacao no caminho de saida.
        let mut r = Recorder::new(4);
        r.record("cookie=segredo_absoluto");
        assert!(!r.dump()[0].contains("segredo_absoluto"), "{:?}", r.dump());
    }

    const S: &str = "SYNTHETIC_SECRET_9q8w";

    #[test]
    fn redact_url_keeps_only_allowlisted_query_values() {
        for input in [
            format!("https://cdn.test/v.mp4?token={S}"),
            format!("https://cdn.test/v.mp4?sig={S}&expires=1700000000"),
            format!("https://s3.test/o?X-Amz-Signature={S}&X-Amz-Credential={S}&Policy={S}"),
            format!("https://www.instagram.com/reel/Cabc123/?igsh={S}"),
            format!("https://user:{S}@host.test/file.mp4"),
            format!("https://host.test/cb#access_token={S}"),
            format!("https://host.test/x?hdnts=exp=1~acl=/*~hmac={S}"),
            format!("https://host.test/x?{S}"),
            format!("https://host.test/x?v={S}{S}{S}{S}"),
        ] {
            let out = redact_url(&input);
            assert!(!out.contains(S), "{input} -> {out}");
            assert!(is_redacted_url(&out), "{out}");
        }
        let igsh = redact_url(&format!("https://www.instagram.com/reel/Cabc123/?igsh={S}"));
        assert_eq!(
            igsh,
            "https://www.instagram.com/reel/Cabc123/?igsh=[REDACTED]"
        );
        let ui = redact_url(&format!("https://user:{S}@host.test/file.mp4"));
        assert_eq!(ui, "https://[REDACTED]@host.test/file.mp4");
    }

    #[test]
    fn redact_url_leaves_public_links_executable() {
        for url in [
            "https://www.youtube.com/watch?v=dQw4w9WgXcQ&list=PL123&t=42",
            "https://youtu.be/dQw4w9WgXcQ",
            "https://x.com/user/status/1790000000000000000",
            "https://www.bilibili.com/video/BV1xx411c7mD?p=2",
            "https://vimeo.com/123456789",
            "https://example.com/a/0f8fad5b-d9cb-469f-a165-70867728950e/file.mp4",
        ] {
            let out = redact_url(url);
            assert_eq!(out, url);
            assert!(!is_redacted_url(&out));
        }
    }

    #[test]
    fn redact_url_hides_token_looking_path_segments() {
        let jwt = "eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxIn0.c2lnbmF0dXJl";
        let hex = "9f86d081884c7d659a2feaa0c55ad015a3bf4f1b2b0b822cd15d6c15b0f00a08";
        let out = redact_url(&format!("https://cdn.test/{jwt}/hls/{hex}/index.m3u8"));
        assert!(!out.contains(jwt) && !out.contains(hex), "{out}");
        assert!(out.ends_with("/hls/[REDACTED]/index.m3u8"), "{out}");
    }

    #[test]
    fn redact_url_of_non_url_drops_query() {
        assert_eq!(
            redact_url(&format!("host.test/x?igsh={S}")),
            "host.test/x?[REDACTED]"
        );
        assert_eq!(redact_url("ytsearch:cats"), "ytsearch:cats");
    }

    #[test]
    fn journal_lines_never_carry_unknown_param_secrets() {
        // D-12: `igsh` is in no blocklist; the URL pass must catch it anyway.
        for line in [
            format!("[direct] GET https://www.instagram.com/reel/Cabc123/?igsh={S} -> 200"),
            format!("status url=\"https://cdn.test/v.mp4?e=1&hmac={S}\""),
            format!("fetch (https://host.test/cb#id_token={S})."),
            format!("HTTP 403 downloading https://s3.test/o?X-Amz-Signature={S}"),
        ] {
            let out = redact(&line);
            assert!(!out.contains(S), "{line} -> {out}");
        }
        let out = redact(&format!("fetch (https://host.test/x?v=abc&q={S})."));
        assert!(out.ends_with("?v=abc&q=[REDACTED])."), "{out}");
    }

    #[test]
    fn redaction_is_idempotent_on_already_redacted_urls() {
        // N-1: MCP `sanitize` runs `redact` over URLs `redact_url` already
        // redacted; the marker must come back as is, never `[REDACTED]]`.
        for input in [
            format!("https://cdn.test/v.mp4?token={S}"),
            format!("https://cdn.test/v.mp4?g4={S}"),
            format!("https://s3.test/o?X-Amz-Signature={S}&X-Amz-Credential={S}&Policy={S}"),
            format!("https://www.instagram.com/reel/Cabc123/?igsh={S}"),
            format!("https://user:{S}@host.test/file.mp4"),
            format!("https://host.test/cb#access_token={S}"),
            format!("https://host.test/x?{S}"),
            format!("https://cdn.test/eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxIn0.c2lnbmF0dXJl/i.m3u8"),
            format!("host.test/x?igsh={S}"),
        ] {
            let once = redact_url(&input);
            assert_eq!(redact_url(&once), once, "redact_url twice: {input}");
            assert_eq!(redact(&once), once, "redact over redact_url: {input}");
            assert_eq!(
                redact_urls(&once),
                once,
                "redact_urls over redact_url: {input}"
            );
            assert!(!once.contains("]]"), "{once}");
            let line = format!("title {once} (queued).");
            assert_eq!(redact(&line), line, "{line}");
        }
    }

    #[test]
    fn redact_urls_leaves_non_url_text_alone() {
        let cmd = format!("yt-dlp -o /Users/me/out.mp4 'https://cdn.test/v?token={S}'");
        let out = redact_urls(&cmd);
        assert!(!out.contains(S), "{out}");
        assert!(out.contains("/Users/me/out.mp4"), "{out}");
    }

    #[test]
    fn capacidade_zero_nao_vira_buffer_que_engole_tudo() {
        let mut r = Recorder::new(0);
        r.record("a");
        r.record("b");
        assert_eq!(r.len(), 1);
        assert_eq!(r.dump(), vec!["b"]);
    }
}

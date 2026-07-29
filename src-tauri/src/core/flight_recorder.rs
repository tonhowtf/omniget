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
    let lower = hay.to_lowercase();
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

    #[test]
    fn capacidade_zero_nao_vira_buffer_que_engole_tudo() {
        let mut r = Recorder::new(0);
        r.record("a");
        r.record("b");
        assert_eq!(r.len(), 1);
        assert_eq!(r.dump(), vec!["b"]);
    }
}

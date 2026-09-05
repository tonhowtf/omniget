//! Texto para voz pelo serviço de leitura em voz alta do Microsoft Edge,
//! reimplementação em Rust do protocolo documentado no estudo 09 (rany2/edge-tts).
//! Grátis, sem chave, precisa de internet. Devolve MP3 24 kHz e os limites
//! de palavra (WordBoundary), que viram legenda sincronizada.

use std::sync::Mutex;

use anyhow::anyhow;
use futures::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio_tungstenite::tungstenite::Message;

use crate::core::subtitle_merge::{cues_to_srt, Cue};

const TRUSTED_CLIENT_TOKEN: &str = "6A5AA1D4EAFF4E9FB37E23D68491D6F4";
const BASE_URL: &str = "speech.platform.bing.com/consumer/speech/synthesize/readaloud";
const CHROMIUM_FULL_VERSION: &str = "130.0.2849.68";
const WIN_EPOCH: u64 = 11_644_473_600;
/// Limite conservador de bytes de texto por requisição (o edge-tts calcula
/// a partir do cabeçalho; 3 000 bytes fica bem abaixo do teto).
const CHUNK_BYTES: usize = 3000;

fn chromium_major() -> &'static str {
    CHROMIUM_FULL_VERSION.split('.').next().unwrap_or("130")
}

/// `Sec-MS-GEC`: SHA-256 de (ticks Windows arredondados a 5 min + token).
fn sec_ms_gec(clock_skew_secs: i64) -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
        + clock_skew_secs;
    let mut ticks = (now.max(0) as u64) + WIN_EPOCH;
    ticks -= ticks % 300;
    let ticks = (ticks as u128) * 10_000_000u128;
    let mut h = Sha256::new();
    h.update(format!("{}{}", ticks, TRUSTED_CLIENT_TOKEN).as_bytes());
    hex::encode_upper(h.finalize())
}

fn connect_id() -> String {
    uuid::Uuid::new_v4().simple().to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Voice {
    #[serde(rename = "ShortName")]
    pub short_name: String,
    #[serde(rename = "Gender", default)]
    pub gender: String,
    #[serde(rename = "Locale", default)]
    pub locale: String,
    #[serde(rename = "FriendlyName", default)]
    pub friendly_name: String,
}

static VOICES: Mutex<Option<(std::time::Instant, Vec<Voice>)>> = Mutex::new(None);

pub async fn list_voices() -> anyhow::Result<Vec<Voice>> {
    if let Ok(g) = VOICES.lock() {
        if let Some((at, v)) = g.as_ref() {
            if at.elapsed() < std::time::Duration::from_secs(6 * 3600) {
                return Ok(v.clone());
            }
        }
    }
    let url = format!(
        "https://{}/voices/list?trustedclienttoken={}&Sec-MS-GEC={}&Sec-MS-GEC-Version=1-{}",
        BASE_URL,
        TRUSTED_CLIENT_TOKEN,
        sec_ms_gec(0),
        CHROMIUM_FULL_VERSION
    );
    let client = super::client()?;
    let resp = client
        .get(&url)
        .header("Authority", "speech.platform.bing.com")
        .header(
            "Sec-CH-UA",
            format!(
                "\" Not;A Brand\";v=\"99\", \"Microsoft Edge\";v=\"{0}\", \"Chromium\";v=\"{0}\"",
                chromium_major()
            ),
        )
        .header("Sec-CH-UA-Mobile", "?0")
        .header("Accept", "*/*")
        .send()
        .await?;
    if !resp.status().is_success() {
        return Err(anyhow!(
            "lista de vozes do Edge indisponivel: HTTP {}",
            resp.status()
        ));
    }
    let mut voices: Vec<Voice> = resp.json().await?;
    voices.sort_by(|a, b| {
        a.locale
            .cmp(&b.locale)
            .then(a.short_name.cmp(&b.short_name))
    });
    if let Ok(mut g) = VOICES.lock() {
        *g = Some((std::time::Instant::now(), voices.clone()));
    }
    Ok(voices)
}

#[derive(Debug, Clone, Serialize)]
pub struct WordBoundary {
    pub text: String,
    pub start_ms: u64,
    pub duration_ms: u64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TtsOptions {
    pub text: String,
    pub voice: String,
    /// "+0%", "-10%", "+25%"
    #[serde(default = "default_pct")]
    pub rate: String,
    #[serde(default = "default_hz")]
    pub pitch: String,
    #[serde(default = "default_pct")]
    pub volume: String,
}

fn default_pct() -> String {
    "+0%".into()
}
fn default_hz() -> String {
    "+0Hz".into()
}

#[derive(Debug, Clone, Serialize)]
pub struct TtsResult {
    pub audio_path: String,
    pub srt_path: String,
    pub words: usize,
    pub duration_ms: u64,
}

fn escape_xml(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn mkssml(text: &str, voice: &str, rate: &str, pitch: &str, volume: &str) -> String {
    format!(
        "<speak version='1.0' xmlns='http://www.w3.org/2001/10/synthesis' xml:lang='en-US'><voice name='{}'><prosody pitch='{}' rate='{}' volume='{}'>{}</prosody></voice></speak>",
        voice, pitch, rate, volume, text
    )
}

fn date_string() -> String {
    chrono::Utc::now()
        .format("%a %b %d %Y %H:%M:%S GMT+0000 (Coordinated Universal Time)")
        .to_string()
}

/// Divide por frases sem passar de `CHUNK_BYTES`, sem cortar UTF-8.
pub fn split_text(text: &str) -> Vec<String> {
    let mut chunks = Vec::new();
    let mut current = String::new();
    for sentence in text.split_inclusive(['.', '!', '?', '\n', ';']) {
        if current.len() + sentence.len() > CHUNK_BYTES && !current.trim().is_empty() {
            chunks.push(std::mem::take(&mut current));
        }
        if sentence.len() > CHUNK_BYTES {
            // frase gigante: corta em espaços
            for word in sentence.split(' ') {
                if current.len() + word.len() + 1 > CHUNK_BYTES && !current.trim().is_empty() {
                    chunks.push(std::mem::take(&mut current));
                }
                current.push_str(word);
                current.push(' ');
            }
        } else {
            current.push_str(sentence);
        }
    }
    if !current.trim().is_empty() {
        chunks.push(current);
    }
    chunks
        .into_iter()
        .map(|c| c.trim().to_string())
        .filter(|c| !c.is_empty())
        .collect()
}

struct ChunkOut {
    audio: Vec<u8>,
    words: Vec<WordBoundary>,
}

fn parse_metadata(json: &str) -> Vec<WordBoundary> {
    let mut out = Vec::new();
    let Ok(v) = serde_json::from_str::<serde_json::Value>(json) else {
        return out;
    };
    if let Some(items) = v["Metadata"].as_array() {
        for it in items {
            if it["Type"].as_str() != Some("WordBoundary") {
                continue;
            }
            let d = &it["Data"];
            let offset = d["Offset"].as_u64().unwrap_or(0);
            let dur = d["Duration"].as_u64().unwrap_or(0);
            let text = d["text"]["Text"].as_str().unwrap_or("").to_string();
            out.push(WordBoundary {
                text,
                start_ms: offset / 10_000,
                duration_ms: dur / 10_000,
            });
        }
    }
    out
}

async fn synth_chunk(text: &str, opts: &TtsOptions, skew: i64) -> anyhow::Result<ChunkOut> {
    use tokio_tungstenite::tungstenite::client::IntoClientRequest;

    let url = format!(
        "wss://{}/edge/v1?TrustedClientToken={}&Sec-MS-GEC={}&Sec-MS-GEC-Version=1-{}&ConnectionId={}",
        BASE_URL,
        TRUSTED_CLIENT_TOKEN,
        sec_ms_gec(skew),
        CHROMIUM_FULL_VERSION,
        connect_id()
    );
    let mut req = url.into_client_request()?;
    let h = req.headers_mut();
    h.insert("Pragma", "no-cache".parse()?);
    h.insert("Cache-Control", "no-cache".parse()?);
    h.insert(
        "Origin",
        "chrome-extension://jdiccldimpdaibmpdkjnbmckianbfold".parse()?,
    );
    h.insert("Accept-Encoding", "gzip, deflate, br".parse()?);
    h.insert("Accept-Language", "en-US,en;q=0.9".parse()?);
    h.insert(
        "User-Agent",
        format!(
            "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/{}.0.0.0 Safari/537.36 Edg/{}.0.0.0",
            chromium_major(),
            chromium_major()
        )
        .parse()?,
    );
    let (mut ws, _) = tokio_tungstenite::connect_async(req)
        .await
        .map_err(|e| anyhow!("Edge TTS: conexao recusada ({})", e))?;

    let cfg = format!(
        "X-Timestamp:{}\r\nContent-Type:application/json; charset=utf-8\r\nPath:speech.config\r\n\r\n{{\"context\":{{\"synthesis\":{{\"audio\":{{\"metadataoptions\":{{\"sentenceBoundaryEnabled\":\"false\",\"wordBoundaryEnabled\":\"true\"}},\"outputFormat\":\"audio-24khz-48kbitrate-mono-mp3\"}}}}}}}}\r\n",
        date_string()
    );
    ws.send(Message::Text(cfg.into())).await?;
    let ssml = mkssml(
        &escape_xml(text),
        &opts.voice,
        &opts.rate,
        &opts.pitch,
        &opts.volume,
    );
    let req_id = connect_id();
    let msg = format!(
        "X-RequestId:{}\r\nContent-Type:application/ssml+xml\r\nX-Timestamp:{}Z\r\nPath:ssml\r\n\r\n{}",
        req_id,
        date_string(),
        ssml
    );
    ws.send(Message::Text(msg.into())).await?;

    let mut audio = Vec::new();
    let mut words = Vec::new();
    let mut got_audio = false;
    while let Some(frame) = ws.next().await {
        match frame? {
            Message::Text(t) => {
                let t: &str = &t;
                let (headers, body) = t.split_once("\r\n\r\n").unwrap_or((t, ""));
                let path = headers
                    .lines()
                    .find_map(|l| l.strip_prefix("Path:"))
                    .unwrap_or("")
                    .trim();
                match path {
                    "audio.metadata" => words.extend(parse_metadata(body)),
                    "turn.end" => break,
                    _ => {}
                }
            }
            Message::Binary(b) => {
                if b.len() < 2 {
                    continue;
                }
                let header_len = u16::from_be_bytes([b[0], b[1]]) as usize;
                if b.len() < header_len + 2 {
                    continue;
                }
                let header = String::from_utf8_lossy(&b[2..2 + header_len]);
                if header.contains("Path:audio") {
                    audio.extend_from_slice(&b[2 + header_len..]);
                    got_audio = true;
                }
            }
            Message::Close(_) => break,
            _ => {}
        }
    }
    let _ = ws.close(None).await;
    if !got_audio {
        return Err(anyhow!("Edge TTS nao devolveu audio (o servico pode ter mudado o token; tente de novo mais tarde)"));
    }
    Ok(ChunkOut { audio, words })
}

/// Agrupa palavras em legendas curtas (até 8 palavras, corta em pausas).
pub fn words_to_cues(words: &[WordBoundary]) -> Vec<Cue> {
    let mut cues = Vec::new();
    let mut buf: Vec<&WordBoundary> = Vec::new();
    let flush = |buf: &mut Vec<&WordBoundary>, cues: &mut Vec<Cue>| {
        if buf.is_empty() {
            return;
        }
        let start = buf[0].start_ms;
        let last = buf[buf.len() - 1];
        let end = last.start_ms + last.duration_ms.max(200);
        let text = buf
            .iter()
            .map(|w| w.text.as_str())
            .collect::<Vec<_>>()
            .join(" ");
        cues.push(Cue {
            start_ms: start,
            end_ms: end.max(start + 1),
            text,
        });
        buf.clear();
    };
    for w in words {
        if let Some(last) = buf.last() {
            let gap = w.start_ms.saturating_sub(last.start_ms + last.duration_ms);
            if buf.len() >= 8 || gap > 600 {
                flush(&mut buf, &mut cues);
            }
        }
        buf.push(w);
    }
    flush(&mut buf, &mut cues);
    cues
}

/// Sintetiza `opts.text` inteiro em `audio_path` (MP3) e escreve o SRT ao lado.
pub async fn synthesize(
    opts: TtsOptions,
    audio_path: &std::path::Path,
    progress: super::ProgressFn,
) -> anyhow::Result<TtsResult> {
    let chunks = split_text(&opts.text);
    if chunks.is_empty() {
        return Err(anyhow!("texto vazio"));
    }
    let id = format!("tts:{}", audio_path.display());
    let mut audio = Vec::new();
    let mut words: Vec<WordBoundary> = Vec::new();
    let mut offset_ms: u64 = 0;
    let mut skew: i64 = 0;
    for (i, chunk) in chunks.iter().enumerate() {
        super::report(
            &progress,
            &id,
            "synthesize",
            i as u64,
            Some(chunks.len() as u64),
            None,
        );
        let out = match synth_chunk(chunk, &opts, skew).await {
            Ok(o) => o,
            Err(e) => {
                // 403 por relógio fora: o edge-tts corrige lendo o header Date; aqui
                // tentamos uma vez com deslocamento de 5 min para frente e para trás.
                let msg = e.to_string();
                if msg.contains("403") || msg.contains("recusada") {
                    skew = if skew == 0 { 300 } else { -300 };
                    synth_chunk(chunk, &opts, skew).await?
                } else {
                    return Err(e);
                }
            }
        };
        let last_end = out
            .words
            .iter()
            .map(|w| w.start_ms + w.duration_ms)
            .max()
            .unwrap_or(0);
        for mut w in out.words {
            w.start_ms += offset_ms;
            words.push(w);
        }
        audio.extend_from_slice(&out.audio);
        // O edge-tts soma o fim da última palavra mais ~0,9 s entre pedaços.
        offset_ms += last_end + 875;
    }
    if let Some(parent) = audio_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    tokio::fs::write(audio_path, &audio).await?;
    let cues = words_to_cues(&words);
    let srt_path = audio_path.with_extension("srt");
    tokio::fs::write(&srt_path, cues_to_srt(&cues)).await?;
    let duration_ms = words
        .iter()
        .map(|w| w.start_ms + w.duration_ms)
        .max()
        .unwrap_or(0);
    super::report(
        &progress,
        &id,
        "done",
        chunks.len() as u64,
        Some(chunks.len() as u64),
        None,
    );
    Ok(TtsResult {
        audio_path: audio_path.to_string_lossy().to_string(),
        srt_path: srt_path.to_string_lossy().to_string(),
        words: words.len(),
        duration_ms,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gec_is_64_hex_upper() {
        let g = sec_ms_gec(0);
        assert_eq!(g.len(), 64);
        assert!(g
            .chars()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_lowercase()));
    }

    #[test]
    fn splits_long_text_by_sentences() {
        let text = "Frase um. Frase dois! ".repeat(400);
        let chunks = split_text(&text);
        assert!(chunks.len() > 1);
        assert!(chunks.iter().all(|c| c.len() <= CHUNK_BYTES + 16));
    }

    #[test]
    fn groups_words_into_cues() {
        let words: Vec<WordBoundary> = (0..20)
            .map(|i| WordBoundary {
                text: format!("w{}", i),
                start_ms: i * 300,
                duration_ms: 250,
            })
            .collect();
        let cues = words_to_cues(&words);
        assert_eq!(cues.len(), 3);
        assert_eq!(cues[0].start_ms, 0);
    }

    #[test]
    fn parses_word_boundaries() {
        let j = r#"{"Metadata":[{"Type":"WordBoundary","Data":{"Offset":1000000,"Duration":500000,"text":{"Text":"Olá","Length":3,"BoundaryType":"WordBoundary"}}}]}"#;
        let w = parse_metadata(j);
        assert_eq!(w.len(), 1);
        assert_eq!(w[0].start_ms, 100);
        assert_eq!(w[0].duration_ms, 50);
    }
}

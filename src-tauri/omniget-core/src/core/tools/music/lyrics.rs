//! Letra sincronizada (`.lrc`) via LRCLIB — API pública, sem chave, que já
//! devolve o LRC pronto quando alguém sincronizou a faixa. Quando só existe
//! letra corrida, gravamos o `.txt` e, se o usuário pedir, um `.lrc`
//! estimado (tempos divididos pela duração, marcado como estimativa).
//!
//! Artista e título saem do ID3v2 lido na mão quando o MP3 tem tag, e do
//! nome do arquivo caso contrário. Não há crate de tag aqui de propósito.

use std::path::{Path, PathBuf};

use anyhow::Result;
use serde::{Deserialize, Serialize};

use super::norm::{norm_artist, norm_title, similarity, strip_track_number};
use crate::core::tools::{client, report, ProgressFn};

const TOOL_ID: &str = "music-lyrics";
const AUDIO_EXTS: &[&str] = &[
    "mp3", "m4a", "flac", "wav", "ogg", "opus", "aac", "wma", "aiff", "aif", "alac",
];

#[derive(Debug, Clone, Deserialize)]
pub struct LyricsOptions {
    /// Arquivos de áudio escolhidos a dedo.
    #[serde(default)]
    pub files: Vec<String>,
    /// Ou uma pasta inteira (lote).
    #[serde(default)]
    pub dir: Option<String>,
    /// Onde gravar. Vazio = ao lado do arquivo de áudio.
    #[serde(default)]
    pub out_dir: Option<String>,
    /// Gerar `.lrc` estimado quando só houver letra corrida.
    #[serde(default)]
    pub estimate: bool,
    /// Regravar quando já existe `.lrc` ao lado.
    #[serde(default)]
    pub overwrite: bool,
    /// Gravar também a letra corrida em `.txt`.
    #[serde(default = "default_true")]
    pub write_plain: bool,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Serialize)]
pub struct LyricsTrack {
    pub path: String,
    pub artist: String,
    pub title: String,
    /// "synced" | "plain" | "estimated" | "not_found" | "skipped" | "error"
    pub status: String,
    pub lrc_path: Option<String>,
    pub txt_path: Option<String>,
    pub lines: usize,
    pub message: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct LyricsResult {
    pub total: usize,
    pub synced: usize,
    pub plain: usize,
    pub estimated: usize,
    pub not_found: usize,
    pub tracks: Vec<LyricsTrack>,
}

// ── ID3v2 na unha ───────────────────────────────────────────────────────

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Id3Tag {
    pub artist: String,
    pub title: String,
    pub album: String,
    pub duration_secs: Option<u64>,
}

fn syncsafe(b: &[u8]) -> usize {
    ((b[0] as usize & 0x7f) << 21)
        | ((b[1] as usize & 0x7f) << 14)
        | ((b[2] as usize & 0x7f) << 7)
        | (b[3] as usize & 0x7f)
}

fn plain_u32(b: &[u8]) -> usize {
    ((b[0] as usize) << 24) | ((b[1] as usize) << 16) | ((b[2] as usize) << 8) | (b[3] as usize)
}

fn decode_text(data: &[u8]) -> String {
    if data.is_empty() {
        return String::new();
    }
    let enc = data[0];
    let body = &data[1..];
    let s = match enc {
        0 => body.iter().map(|&b| b as char).collect::<String>(),
        1 => {
            if body.len() < 2 {
                String::new()
            } else {
                let (be, rest) = match (body[0], body[1]) {
                    (0xFF, 0xFE) => (false, &body[2..]),
                    (0xFE, 0xFF) => (true, &body[2..]),
                    _ => (false, body),
                };
                let units: Vec<u16> = rest
                    .chunks_exact(2)
                    .map(|c| {
                        if be {
                            u16::from_be_bytes([c[0], c[1]])
                        } else {
                            u16::from_le_bytes([c[0], c[1]])
                        }
                    })
                    .collect();
                String::from_utf16_lossy(&units)
            }
        }
        2 => {
            let units: Vec<u16> = body
                .chunks_exact(2)
                .map(|c| u16::from_be_bytes([c[0], c[1]]))
                .collect();
            String::from_utf16_lossy(&units)
        }
        _ => String::from_utf8_lossy(body).to_string(),
    };
    s.split('\u{0}').next().unwrap_or("").trim().to_string()
}

/// Lê os campos que interessam de uma tag ID3v2.2/2.3/2.4 no começo do
/// arquivo. Devolve `None` quando não há tag ou quando ela não tem nada útil.
pub fn read_id3(bytes: &[u8]) -> Option<Id3Tag> {
    if bytes.len() < 10 || &bytes[0..3] != b"ID3" {
        return None;
    }
    let ver = bytes[3];
    let flags = bytes[5];
    let size = syncsafe(&bytes[6..10]);
    let end = (10 + size).min(bytes.len());
    let mut pos = 10usize;
    if flags & 0x40 != 0 && pos + 4 <= end {
        let ext = if ver >= 4 {
            syncsafe(&bytes[pos..pos + 4])
        } else {
            plain_u32(&bytes[pos..pos + 4]) + 4
        };
        pos += ext.max(4);
    }

    let id_len = if ver <= 2 { 3 } else { 4 };
    let head_len = if ver <= 2 { 6 } else { 10 };
    let mut tag = Id3Tag::default();
    while pos + head_len <= end {
        let id = &bytes[pos..pos + id_len];
        if id[0] == 0 {
            break;
        }
        let (fsize, data_start) = if ver <= 2 {
            (
                ((bytes[pos + 3] as usize) << 16)
                    | ((bytes[pos + 4] as usize) << 8)
                    | bytes[pos + 5] as usize,
                pos + 6,
            )
        } else if ver >= 4 {
            (syncsafe(&bytes[pos + 4..pos + 8]), pos + 10)
        } else {
            (plain_u32(&bytes[pos + 4..pos + 8]), pos + 10)
        };
        let data_end = data_start.saturating_add(fsize).min(end);
        if fsize == 0 || data_start >= data_end {
            pos = data_end.max(pos + head_len);
            continue;
        }
        let data = &bytes[data_start..data_end];
        match id {
            b"TPE1" | b"TP1" => tag.artist = decode_text(data),
            b"TIT2" | b"TT2" => tag.title = decode_text(data),
            b"TALB" | b"TAL" => tag.album = decode_text(data),
            b"TLEN" | b"TLE" => {
                if let Ok(ms) = decode_text(data).parse::<u64>() {
                    if ms > 0 {
                        tag.duration_secs = Some(ms / 1000);
                    }
                }
            }
            _ => {}
        }
        pos = data_end;
    }
    if tag.title.is_empty() && tag.artist.is_empty() {
        None
    } else {
        Some(tag)
    }
}

/// Artista/título de "01 - Artista - Título.mp3". Sem traço, tudo vira
/// título e a busca vai só pelo nome da faixa.
pub fn from_filename(stem: &str) -> (String, String) {
    let s = strip_track_number(stem)
        .replace(" – ", " - ")
        .replace(" — ", " - ");
    match s.split_once(" - ") {
        Some((a, t)) if !t.trim().is_empty() => (a.trim().to_string(), t.trim().to_string()),
        _ => (String::new(), s.trim().to_string()),
    }
}

/// Primeiro a tag, depois o nome do arquivo. Só lê o começo do arquivo.
pub fn guess_meta(path: &Path) -> Id3Tag {
    let stem = path
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();
    let (fa, ft) = from_filename(&stem);
    let mut tag = read_head(path)
        .and_then(|b| read_id3(&b))
        .unwrap_or_default();
    if tag.artist.is_empty() {
        tag.artist = fa;
    }
    if tag.title.is_empty() {
        tag.title = ft;
    }
    tag
}

fn read_head(path: &Path) -> Option<Vec<u8>> {
    use std::io::Read;
    let mut f = std::fs::File::open(path).ok()?;
    let mut buf = vec![0u8; 512 * 1024];
    let n = f.read(&mut buf).ok()?;
    buf.truncate(n);
    Some(buf)
}

// ── LRC ─────────────────────────────────────────────────────────────────

/// `[mm:ss.xx]`, o formato que todo tocador entende.
pub fn fmt_ts(secs: f64) -> String {
    let cs = (secs.max(0.0) * 100.0).round() as u64;
    format!(
        "[{:02}:{:02}.{:02}]",
        cs / 6000,
        (cs % 6000) / 100,
        cs % 100
    )
}

/// Lê um LRC e devolve (segundos, texto) por linha, já ordenado. Ignora as
/// tags de cabeçalho (`[ar:]`, `[ti:]`…).
pub fn parse_lrc(text: &str) -> Vec<(f64, String)> {
    let mut out: Vec<(f64, String)> = Vec::new();
    for line in text.lines() {
        let mut rest = line.trim();
        let mut stamps: Vec<f64> = Vec::new();
        while rest.starts_with('[') {
            let Some(close) = rest.find(']') else { break };
            let inner = &rest[1..close];
            let Some((m, s)) = inner.split_once(':') else {
                break;
            };
            let Ok(m) = m.trim().parse::<f64>() else {
                break;
            };
            let Ok(s) = s.trim().replace(',', ".").parse::<f64>() else {
                break;
            };
            stamps.push(m * 60.0 + s);
            rest = rest[close + 1..].trim_start();
        }
        for t in stamps {
            out.push((t, rest.to_string()));
        }
    }
    out.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
    out
}

/// LRC estimado: as linhas divididas igualmente pela duração. Vem marcado
/// no cabeçalho, para ninguém achar que é sincronia de verdade.
pub fn estimate_lrc(plain: &str, duration_secs: f64, artist: &str, title: &str) -> String {
    let lines: Vec<&str> = plain
        .lines()
        .map(|l| l.trim())
        .filter(|l| !l.is_empty())
        .collect();
    let mut out = String::new();
    if !artist.is_empty() {
        out.push_str(&format!("[ar:{artist}]\n"));
    }
    if !title.is_empty() {
        out.push_str(&format!("[ti:{title}]\n"));
    }
    out.push_str("[by:OmniGet - tempos estimados]\n");
    if lines.is_empty() || duration_secs <= 0.0 {
        for l in lines {
            out.push_str(&format!("{}{}\n", fmt_ts(0.0), l));
        }
        return out;
    }
    let step = duration_secs / lines.len() as f64;
    for (i, l) in lines.iter().enumerate() {
        out.push_str(&format!("{}{}\n", fmt_ts(i as f64 * step), l));
    }
    out
}

// ── LRCLIB ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Deserialize)]
pub struct LrcLibTrack {
    #[serde(rename = "trackName", default)]
    pub track_name: String,
    #[serde(rename = "artistName", default)]
    pub artist_name: String,
    #[serde(rename = "albumName", default)]
    pub album_name: Option<String>,
    #[serde(default)]
    pub duration: Option<f64>,
    #[serde(default)]
    pub instrumental: bool,
    #[serde(rename = "plainLyrics", default)]
    pub plain_lyrics: Option<String>,
    #[serde(rename = "syncedLyrics", default)]
    pub synced_lyrics: Option<String>,
}

/// Nota de um candidato da busca: título e artista pesam, duração parecida
/// desempata.
pub fn rank(candidate: &LrcLibTrack, artist: &str, title: &str, duration: Option<u64>) -> u32 {
    let t = similarity(&norm_title(title), &norm_title(&candidate.track_name));
    let a = if artist.is_empty() {
        t
    } else {
        similarity(&norm_artist(artist), &norm_artist(&candidate.artist_name))
    };
    let mut s = (t * 2 + a) / 3;
    if let (Some(d), Some(cd)) = (duration, candidate.duration) {
        if (cd - d as f64).abs() <= 3.0 {
            s = (s + 5).min(100);
        }
    }
    if candidate.synced_lyrics.is_some() {
        s = (s + 3).min(100);
    }
    s
}

/// `/api/get` primeiro (é o casamento exato do LRCLIB); `/api/search` como
/// rede de segurança.
pub async fn fetch_lyrics(
    http: &reqwest::Client,
    artist: &str,
    title: &str,
    album: &str,
    duration: Option<u64>,
) -> Result<Option<LrcLibTrack>> {
    let enc = urlencoding::encode;
    if !artist.is_empty() {
        let mut url = format!(
            "https://lrclib.net/api/get?artist_name={}&track_name={}",
            enc(artist),
            enc(title)
        );
        if !album.is_empty() {
            url.push_str(&format!("&album_name={}", enc(album)));
        }
        if let Some(d) = duration {
            url.push_str(&format!("&duration={d}"));
        }
        let resp = http.get(&url).send().await?;
        if resp.status().is_success() {
            let t: LrcLibTrack = resp.json().await?;
            return Ok(Some(t));
        }
    }

    let mut url = format!("https://lrclib.net/api/search?track_name={}", enc(title));
    if !artist.is_empty() {
        url.push_str(&format!("&artist_name={}", enc(artist)));
    }
    let resp = http.get(&url).send().await?;
    if !resp.status().is_success() {
        return Ok(None);
    }
    let list: Vec<LrcLibTrack> = resp.json().await.unwrap_or_default();
    Ok(list
        .into_iter()
        .map(|c| {
            let s = rank(&c, artist, title, duration);
            (s, c)
        })
        .filter(|(s, _)| *s >= 70)
        .max_by_key(|(s, _)| *s)
        .map(|(_, c)| c))
}

// ── Execução ────────────────────────────────────────────────────────────

fn collect_inputs(opts: &LyricsOptions) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = opts.files.iter().map(PathBuf::from).collect();
    if let Some(dir) = &opts.dir {
        for entry in walkdir::WalkDir::new(dir)
            .follow_links(false)
            .into_iter()
            .filter_map(|e| e.ok())
        {
            let p = entry.path();
            let audio = p
                .extension()
                .map(|e| AUDIO_EXTS.contains(&e.to_string_lossy().to_lowercase().as_str()))
                .unwrap_or(false);
            if entry.file_type().is_file() && audio {
                out.push(p.to_path_buf());
            }
        }
    }
    out.sort();
    out.dedup();
    out
}

pub async fn run(opts: LyricsOptions, p: ProgressFn) -> Result<LyricsResult> {
    let inputs = collect_inputs(&opts);
    if inputs.is_empty() {
        anyhow::bail!("nenhum arquivo de áudio para procurar letra");
    }
    let http = client()?;
    let total = inputs.len() as u64;
    report(&p, TOOL_ID, "started", 0, Some(total), None);

    let mut result = LyricsResult {
        total: inputs.len(),
        synced: 0,
        plain: 0,
        estimated: 0,
        not_found: 0,
        tracks: Vec::new(),
    };

    for (i, path) in inputs.iter().enumerate() {
        let meta = guess_meta(path);
        let stem = path
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default();
        let out_dir = match &opts.out_dir {
            Some(d) if !d.is_empty() => PathBuf::from(d),
            _ => path.parent().map(|d| d.to_path_buf()).unwrap_or_default(),
        };
        let _ = std::fs::create_dir_all(&out_dir);
        let lrc_path = out_dir.join(format!("{stem}.lrc"));

        report(
            &p,
            TOOL_ID,
            "progress",
            i as u64,
            Some(total),
            Some(meta.title.clone()),
        );

        let mut track = LyricsTrack {
            path: path.to_string_lossy().to_string(),
            artist: meta.artist.clone(),
            title: meta.title.clone(),
            status: "not_found".to_string(),
            lrc_path: None,
            txt_path: None,
            lines: 0,
            message: None,
        };

        if lrc_path.exists() && !opts.overwrite {
            track.status = "skipped".to_string();
            track.lrc_path = Some(lrc_path.to_string_lossy().to_string());
            result.tracks.push(track);
            continue;
        }
        if meta.title.is_empty() {
            track.status = "error".to_string();
            track.message = Some("não consegui ler artista e título".to_string());
            result.tracks.push(track);
            continue;
        }

        match fetch_lyrics(
            &http,
            &meta.artist,
            &meta.title,
            &meta.album,
            meta.duration_secs,
        )
        .await
        {
            Ok(Some(found)) => {
                let synced = found.synced_lyrics.clone().filter(|s| !s.trim().is_empty());
                let plain = found.plain_lyrics.clone().filter(|s| !s.trim().is_empty());
                if let Some(lrc) = synced {
                    std::fs::write(&lrc_path, &lrc)?;
                    track.lines = parse_lrc(&lrc).len();
                    track.status = "synced".to_string();
                    track.lrc_path = Some(lrc_path.to_string_lossy().to_string());
                    result.synced += 1;
                } else if let Some(text) = plain {
                    if opts.write_plain {
                        let txt = out_dir.join(format!("{stem}.txt"));
                        std::fs::write(&txt, &text)?;
                        track.txt_path = Some(txt.to_string_lossy().to_string());
                    }
                    let dur = meta
                        .duration_secs
                        .map(|d| d as f64)
                        .or(found.duration)
                        .unwrap_or(0.0);
                    if opts.estimate && dur > 0.0 {
                        let lrc = estimate_lrc(&text, dur, &meta.artist, &meta.title);
                        std::fs::write(&lrc_path, &lrc)?;
                        track.lrc_path = Some(lrc_path.to_string_lossy().to_string());
                        track.status = "estimated".to_string();
                        result.estimated += 1;
                    } else {
                        track.status = "plain".to_string();
                        result.plain += 1;
                    }
                    track.lines = text.lines().filter(|l| !l.trim().is_empty()).count();
                } else if found.instrumental {
                    track.status = "not_found".to_string();
                    track.message = Some("instrumental".to_string());
                    result.not_found += 1;
                } else {
                    result.not_found += 1;
                }
            }
            Ok(None) => result.not_found += 1,
            Err(e) => {
                track.status = "error".to_string();
                track.message = Some(e.to_string());
            }
        }
        result.tracks.push(track);
        // LRCLIB é gratuito e sem chave; um respiro entre chamadas.
        tokio::time::sleep(std::time::Duration::from_millis(250)).await;
    }

    report(&p, TOOL_ID, "done", total, Some(total), None);
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text_frame(id: &[u8; 4], value: &str) -> Vec<u8> {
        let mut data = vec![3u8];
        data.extend_from_slice(value.as_bytes());
        let mut f = id.to_vec();
        let n = data.len();
        // ID3v2.4: tamanho syncsafe.
        f.extend_from_slice(&[
            ((n >> 21) & 0x7f) as u8,
            ((n >> 14) & 0x7f) as u8,
            ((n >> 7) & 0x7f) as u8,
            (n & 0x7f) as u8,
        ]);
        f.extend_from_slice(&[0, 0]);
        f.extend_from_slice(&data);
        f
    }

    fn id3v24(frames: Vec<u8>) -> Vec<u8> {
        let mut out = b"ID3".to_vec();
        out.extend_from_slice(&[4, 0, 0]);
        let n = frames.len();
        out.extend_from_slice(&[
            ((n >> 21) & 0x7f) as u8,
            ((n >> 14) & 0x7f) as u8,
            ((n >> 7) & 0x7f) as u8,
            (n & 0x7f) as u8,
        ]);
        out.extend_from_slice(&frames);
        out
    }

    #[test]
    fn le_id3v24_utf8() {
        let mut frames = text_frame(b"TPE1", "Beyoncé");
        frames.extend(text_frame(b"TIT2", "Halo"));
        frames.extend(text_frame(b"TALB", "I Am... Sasha Fierce"));
        frames.extend(text_frame(b"TLEN", "261000"));
        let tag = read_id3(&id3v24(frames)).expect("tag");
        assert_eq!(tag.artist, "Beyoncé");
        assert_eq!(tag.title, "Halo");
        assert_eq!(tag.album, "I Am... Sasha Fierce");
        assert_eq!(tag.duration_secs, Some(261));
    }

    #[test]
    fn sem_tag_devolve_none() {
        assert!(read_id3(b"nao sou um mp3 com tag").is_none());
        assert!(read_id3(&[]).is_none());
    }

    #[test]
    fn artista_e_titulo_do_nome_do_arquivo() {
        assert_eq!(
            from_filename("01 - Daft Punk - Around the World"),
            ("Daft Punk".to_string(), "Around the World".to_string())
        );
        assert_eq!(
            from_filename("Só o Título"),
            (String::new(), "Só o Título".to_string())
        );
    }

    #[test]
    fn timestamp_lrc() {
        assert_eq!(fmt_ts(0.0), "[00:00.00]");
        assert_eq!(fmt_ts(65.5), "[01:05.50]");
        assert_eq!(fmt_ts(3599.99), "[59:59.99]");
        assert_eq!(fmt_ts(9.999), "[00:10.00]");
    }

    #[test]
    fn parse_lrc_com_tags_e_multiplo_timestamp() {
        let lrc = "[ar:Alguém]\n[00:12.30]primeira\n[00:20.00][01:05.10]refrão\n";
        let lines = parse_lrc(lrc);
        assert_eq!(lines.len(), 3);
        assert_eq!(lines[0], (12.30, "primeira".to_string()));
        assert_eq!(lines[1].1, "refrão");
        assert!((lines[2].0 - 65.10).abs() < 0.001);
    }

    #[test]
    fn lrc_estimado_distribui_pela_duracao() {
        let lrc = estimate_lrc("uma\n\ndois\ntres\n", 90.0, "A", "T");
        assert!(lrc.contains("[by:OmniGet - tempos estimados]"));
        let lines = parse_lrc(&lrc);
        assert_eq!(lines.len(), 3);
        assert_eq!(lines[0].0, 0.0);
        assert_eq!(lines[1].0, 30.0);
        assert_eq!(lines[2].0, 60.0);
    }

    #[test]
    fn rank_prefere_o_titulo_certo() {
        let bom = LrcLibTrack {
            track_name: "Believer".into(),
            artist_name: "Imagine Dragons".into(),
            album_name: None,
            duration: Some(204.0),
            instrumental: false,
            plain_lyrics: None,
            synced_lyrics: Some("[00:00.00]x".into()),
        };
        let ruim = LrcLibTrack {
            track_name: "Believe".into(),
            artist_name: "Cher".into(),
            album_name: None,
            duration: None,
            instrumental: false,
            plain_lyrics: None,
            synced_lyrics: None,
        };
        let a = rank(&bom, "Imagine Dragons", "Believer", Some(204));
        let b = rank(&ruim, "Imagine Dragons", "Believer", Some(204));
        assert_eq!(a, 100);
        assert!(b < a);
    }

    #[tokio::test]
    #[ignore = "rede: bate no lrclib.net de verdade"]
    async fn lrclib_devolve_letra_sincronizada() {
        let http = client().expect("client");
        let found = fetch_lyrics(&http, "Imagine Dragons", "Believer", "", Some(204))
            .await
            .expect("fetch")
            .expect("achou a faixa");
        let synced = found.synced_lyrics.as_deref().unwrap_or("");
        println!(
            "lrclib: {} - {} | sincronizada: {} | linhas: {}",
            found.artist_name,
            found.track_name,
            !synced.is_empty(),
            parse_lrc(synced).len()
        );
        assert!(!synced.is_empty(), "veio sem syncedLyrics");
        assert!(parse_lrc(synced).len() > 10);
    }
}

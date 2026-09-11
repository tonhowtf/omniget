//! Replay de chat de VOD/clipe → JSON, CSV e legenda (SRT/ASS).
//!
//! Usa a persisted query pública `VideoCommentsByOffsetOrCursor`, a mesma que
//! o player usa quando você assiste a um VOD gravado. Sem login.
//!
//! Duas formas de paginar: por `cursor` (uma página emenda na outra) e por
//! `contentOffsetSeconds` (a Twitch devolve uma janela em volta do segundo
//! pedido). O cursor é mais barato, mas hoje ele cai em `IntegrityCheckFailed`
//! para cliente anônimo; então a gente tenta cursor, e ao primeiro erro de
//! integridade passa a caminhar por offset e deduplicar por id da mensagem.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use anyhow::{anyhow, bail};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::super::subtitle::{self, Cue};
use super::super::x::export::csv_escape;
use super::super::{report, sanitize_name, ProgressFn};
use super::gql::{first_error, Gql, VideoInfo, COMMENTS_HASH};

const ID: &str = "tw-chat";
const OP: &str = "VideoCommentsByOffsetOrCursor";

#[derive(Debug, Clone, Deserialize)]
pub struct Options {
    /// Link (ou id) de um VOD, ou link de um clipe.
    pub input: String,
    pub out_dir: String,
    /// "json" | "csv" | "srt" | "ass"; vazio exporta JSON e CSV.
    #[serde(default)]
    pub formats: Vec<String>,
    #[serde(default)]
    pub start_seconds: f64,
    /// 0 = até o fim do VOD.
    #[serde(default)]
    pub end_seconds: f64,
    /// Janela de agrupamento da legenda, em segundos.
    #[serde(default = "def_window")]
    pub window_seconds: f64,
    /// Máximo de linhas por bloco de legenda.
    #[serde(default = "def_lines")]
    pub max_lines: usize,
    /// Pausa entre páginas, para não bater rápido demais.
    #[serde(default = "def_delay")]
    pub delay_ms: u64,
    /// 0 = sem limite.
    #[serde(default)]
    pub max_messages: usize,
}

fn def_window() -> f64 {
    5.0
}
fn def_lines() -> usize {
    6
}
fn def_delay() -> u64 {
    350
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Fragment {
    pub text: String,
    /// Id do emote nativo, quando o pedaço é um emote.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub emote: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Message {
    pub id: String,
    /// Segundo do VOD em que a mensagem aparece.
    pub offset: f64,
    pub created_at: String,
    pub author: String,
    pub display: String,
    pub color: String,
    pub badges: Vec<String>,
    pub text: String,
    pub fragments: Vec<Fragment>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Peak {
    pub start_seconds: f64,
    pub clock: String,
    pub count: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct Chatter {
    pub author: String,
    pub count: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct Result {
    pub video_id: String,
    pub title: String,
    pub channel: String,
    pub duration_seconds: f64,
    pub messages: usize,
    pub pages: usize,
    /// Mensagens por minuto, do começo ao fim do trecho baixado.
    pub series: Vec<Peak>,
    /// Os minutos mais movimentados, do maior para o menor.
    pub top_peaks: Vec<Peak>,
    pub top_chatters: Vec<Chatter>,
    pub files: Vec<String>,
    /// Ficou `true` quando a Twitch recusou a paginação por cursor.
    pub offset_fallback: bool,
    /// Amostra para a UI mostrar sem carregar tudo.
    pub sample: Vec<Message>,
}

// ───────────────────────── parsing ─────────────────────────

#[derive(Debug, Clone, Default)]
pub struct Page {
    pub messages: Vec<Message>,
    pub cursor: Option<String>,
    pub has_next: bool,
}

/// Lê um elemento da resposta em lote da persisted query.
pub fn parse_page(raw: &Value) -> anyhow::Result<Page> {
    let comments = raw
        .pointer("/data/video/comments")
        .filter(|v| !v.is_null())
        .ok_or_else(|| anyhow!("resposta sem comentários (o VOD tem replay de chat?)"))?;
    let edges = comments
        .get("edges")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    let mut messages = Vec::with_capacity(edges.len());
    let mut cursor = None;
    for edge in &edges {
        if let Some(c) = edge.get("cursor").and_then(|v| v.as_str()) {
            cursor = Some(c.to_string());
        }
        let Some(node) = edge.get("node") else {
            continue;
        };
        let Some(msg) = parse_message(node) else {
            continue;
        };
        messages.push(msg);
    }
    let has_next = comments
        .pointer("/pageInfo/hasNextPage")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    Ok(Page {
        messages,
        cursor,
        has_next,
    })
}

fn parse_message(node: &Value) -> Option<Message> {
    let id = node.get("id").and_then(|v| v.as_str())?.to_string();
    let offset = node
        .get("contentOffsetSeconds")
        .and_then(|v| v.as_f64())
        .unwrap_or(0.0);
    let author = node
        .pointer("/commenter/login")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let display = node
        .pointer("/commenter/displayName")
        .and_then(|v| v.as_str())
        .unwrap_or(author.as_str())
        .to_string();
    let color = node
        .pointer("/message/userColor")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let badges = node
        .pointer("/message/userBadges")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|b| {
                    let set = b.get("setID").and_then(|v| v.as_str())?;
                    if set.is_empty() {
                        return None;
                    }
                    let version = b.get("version").and_then(|v| v.as_str()).unwrap_or("1");
                    Some(format!("{}/{}", set, version))
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let fragments = node
        .pointer("/message/fragments")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .map(|f| Fragment {
                    text: f
                        .get("text")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string(),
                    emote: f
                        .pointer("/emote/emoteID")
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string()),
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let text = fragments
        .iter()
        .map(|f| f.text.as_str())
        .collect::<Vec<_>>()
        .join("")
        .trim()
        .to_string();
    Some(Message {
        id,
        offset,
        created_at: node
            .get("createdAt")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        author,
        display,
        color,
        badges,
        text,
        fragments,
    })
}

// ───────────────────────── formatação ─────────────────────────

/// Segundos → `HH:MM:SS`.
pub fn fmt_clock(secs: f64) -> String {
    let s = secs.max(0.0).round() as i64;
    format!("{:02}:{:02}:{:02}", s / 3600, (s / 60) % 60, s % 60)
}

pub fn to_csv(msgs: &[Message]) -> String {
    let mut out = String::from("time,offset_seconds,author,message\n");
    for m in msgs {
        out.push_str(&format!(
            "{},{},{},{}\n",
            fmt_clock(m.offset),
            m.offset,
            csv_escape(&m.display),
            csv_escape(&m.text)
        ));
    }
    out
}

/// Agrupa as mensagens em janelas de `window` segundos, guardando as últimas
/// `max_lines` de cada janela (é o que cabe na tela quando queimar).
pub fn to_cues(msgs: &[Message], window: f64, max_lines: usize) -> Vec<Cue> {
    let window = if window > 0.0 { window } else { 5.0 };
    let max_lines = max_lines.max(1);
    let mut buckets: Vec<(i64, Vec<String>)> = Vec::new();
    for m in msgs {
        if m.text.is_empty() {
            continue;
        }
        let bucket = (m.offset / window).floor() as i64;
        let line = if m.display.is_empty() {
            m.text.clone()
        } else {
            format!("{}: {}", m.display, m.text)
        };
        match buckets.last_mut() {
            Some((b, lines)) if *b == bucket => lines.push(line),
            _ => buckets.push((bucket, vec![line])),
        }
    }
    buckets
        .into_iter()
        .map(|(bucket, mut lines)| {
            if lines.len() > max_lines {
                lines = lines.split_off(lines.len() - max_lines);
            }
            let start_ms = (bucket as f64 * window * 1000.0).round() as i64;
            Cue {
                start_ms,
                end_ms: start_ms + (window * 1000.0).round() as i64,
                text: lines.join("\n"),
            }
        })
        .collect()
}

/// Mensagens por janela (por padrão, por minuto): o mapa de melhores momentos.
pub fn peaks(msgs: &[Message], bucket_seconds: f64) -> Vec<Peak> {
    let bucket = if bucket_seconds > 0.0 {
        bucket_seconds
    } else {
        60.0
    };
    let mut counts: HashMap<i64, usize> = HashMap::new();
    for m in msgs {
        *counts
            .entry((m.offset / bucket).floor() as i64)
            .or_insert(0) += 1;
    }
    let mut keys: Vec<i64> = counts.keys().copied().collect();
    keys.sort_unstable();
    keys.into_iter()
        .map(|k| {
            let start = k as f64 * bucket;
            Peak {
                start_seconds: start,
                clock: fmt_clock(start),
                count: counts.get(&k).copied().unwrap_or(0),
            }
        })
        .collect()
}

pub fn top_chatters(msgs: &[Message], n: usize) -> Vec<Chatter> {
    let mut counts: HashMap<&str, usize> = HashMap::new();
    for m in msgs {
        if m.display.is_empty() {
            continue;
        }
        *counts.entry(m.display.as_str()).or_insert(0) += 1;
    }
    let mut list: Vec<Chatter> = counts
        .into_iter()
        .map(|(author, count)| Chatter {
            author: author.to_string(),
            count,
        })
        .collect();
    list.sort_by(|a, b| b.count.cmp(&a.count).then_with(|| a.author.cmp(&b.author)));
    list.truncate(n);
    list
}

// ───────────────────────── coleta ─────────────────────────

/// Puxa a timeline inteira. Devolve as mensagens em ordem e quantas páginas
/// custou; `offset_fallback` diz se a Twitch recusou o cursor no meio.
pub async fn fetch_all(
    gql: &Gql,
    video_id: &str,
    opts: &Options,
    duration: f64,
    p: &ProgressFn,
) -> anyhow::Result<(Vec<Message>, usize, bool)> {
    let start = opts.start_seconds.max(0.0);
    let end = if opts.end_seconds > 0.0 {
        opts.end_seconds
    } else {
        f64::INFINITY
    };
    let total = if duration > 0.0 {
        Some(duration.round() as u64)
    } else {
        None
    };

    let mut out: Vec<Message> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    let mut cursor: Option<String> = None;
    let mut use_cursor = true;
    let mut offset_fallback = false;
    let mut offset = start;
    let mut pages = 0usize;
    let mut stuck = 0u32;

    report(p, ID, "started", 0, total, None);
    loop {
        let vars = match (use_cursor, cursor.as_ref()) {
            (true, Some(c)) => json!({ "videoID": video_id, "cursor": c }),
            _ => json!({ "videoID": video_id, "contentOffsetSeconds": offset }),
        };
        let raw = gql.persisted(OP, COMMENTS_HASH, vars).await?;
        if let Some(msg) = first_error(&raw) {
            let integrity = msg.to_lowercase().contains("integrity");
            if integrity && use_cursor {
                // A Twitch fechou o cursor para cliente anônimo: segue por
                // offset, deduplicando pelas mensagens que já vieram.
                use_cursor = false;
                offset_fallback = true;
                cursor = None;
                continue;
            }
            bail!("Twitch recusou o replay de chat: {}", msg);
        }
        let page = parse_page(&raw)?;
        pages += 1;
        if page.messages.is_empty() {
            break;
        }

        let last_offset = page
            .messages
            .iter()
            .map(|m| m.offset)
            .fold(offset, f64::max);
        let mut fresh = 0usize;
        for m in page.messages {
            if m.offset < start || m.offset > end {
                continue;
            }
            if seen.insert(m.id.clone()) {
                out.push(m);
                fresh += 1;
            }
        }

        report(
            p,
            ID,
            "progress",
            last_offset.max(0.0) as u64,
            total,
            Some(format!("{} · {}", fmt_clock(last_offset), out.len())),
        );

        if last_offset > end {
            break;
        }
        if opts.max_messages > 0 && out.len() >= opts.max_messages {
            break;
        }
        if !page.has_next {
            break;
        }

        if use_cursor && page.cursor.is_some() {
            cursor = page.cursor;
        } else {
            // A janela por offset vem em volta do segundo pedido, então ela
            // repete o que já veio: avança e, se travar, empurra um segundo.
            let next = if fresh == 0 {
                last_offset + 1.0
            } else {
                last_offset
            };
            if next <= offset {
                offset += 1.0;
            } else {
                offset = next;
            }
        }

        if fresh == 0 {
            stuck += 1;
            if stuck >= 8 {
                break;
            }
        } else {
            stuck = 0;
        }

        if opts.delay_ms > 0 {
            tokio::time::sleep(std::time::Duration::from_millis(opts.delay_ms)).await;
        }
    }

    out.sort_by(|a, b| {
        a.offset
            .partial_cmp(&b.offset)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.created_at.cmp(&b.created_at))
    });
    report(p, ID, "done", total.unwrap_or_default(), total, None);
    Ok((out, pages, offset_fallback))
}

pub async fn export(opts: &Options, p: &ProgressFn) -> anyhow::Result<Result> {
    let gql = Gql::new()?;
    report(p, ID, "progress", 0, None, Some("lendo o VOD".into()));
    let info: VideoInfo = gql.resolve_chat_target(&opts.input).await?;

    // Clipe: já limita o trecho à janela do clipe, com uma folga de 5 s.
    let mut opts = opts.clone();
    if let (Some(off), Some(dur)) = (info.clip_offset, info.clip_duration) {
        if opts.start_seconds <= 0.0 {
            opts.start_seconds = (off - 5.0).max(0.0);
        }
        if opts.end_seconds <= 0.0 {
            opts.end_seconds = off + dur + 5.0;
        }
    }

    let (messages, pages, offset_fallback) =
        fetch_all(&gql, &info.id, &opts, info.duration_seconds, p).await?;
    if messages.is_empty() {
        bail!("esse VOD não devolveu nenhuma mensagem de chat");
    }

    let formats: Vec<String> = if opts.formats.is_empty() {
        vec!["json".into(), "csv".into()]
    } else {
        opts.formats.iter().map(|f| f.to_lowercase()).collect()
    };

    let series = peaks(&messages, 60.0);
    let mut top_peaks = series.clone();
    top_peaks.sort_by_key(|p| std::cmp::Reverse(p.count));
    top_peaks.truncate(10);

    let dir = PathBuf::from(&opts.out_dir);
    std::fs::create_dir_all(&dir)?;
    let stem = sanitize_name(&format!(
        "{}-{}-chat",
        if info.channel.is_empty() {
            "twitch"
        } else {
            info.channel.as_str()
        },
        info.id
    ));

    let mut files = Vec::new();
    for format in &formats {
        let (ext, body) = match format.as_str() {
            "json" => (
                "json",
                serde_json::to_string_pretty(&json!({
                    "video": {
                        "id": info.id,
                        "title": info.title,
                        "channel": info.channel,
                        "channel_display": info.channel_display,
                        "created_at": info.created_at,
                        "duration_seconds": info.duration_seconds,
                    },
                    "messages": messages,
                    "peaks_per_minute": series,
                }))?,
            ),
            "csv" => ("csv", to_csv(&messages)),
            "srt" | "ass" | "vtt" => {
                let cues = to_cues(&messages, opts.window_seconds, opts.max_lines);
                (format.as_str(), subtitle::render(&cues, format))
            }
            other => bail!("formato desconhecido: {}", other),
        };
        let path = dir.join(format!("{}.{}", stem, ext));
        std::fs::write(&path, body)?;
        files.push(path.to_string_lossy().to_string());
    }

    let sample = messages.iter().take(30).cloned().collect();
    Ok(Result {
        video_id: info.id,
        title: info.title,
        channel: if info.channel_display.is_empty() {
            info.channel
        } else {
            info.channel_display
        },
        duration_seconds: info.duration_seconds,
        messages: messages.len(),
        pages,
        series,
        top_peaks,
        top_chatters: top_chatters(&messages, 10),
        files,
        offset_fallback,
        sample,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn page_fixture() -> Value {
        json!({
            "data": { "video": {
                "id": "123",
                "comments": {
                    "edges": [
                        { "cursor": "cur-1", "node": {
                            "id": "m1",
                            "commenter": { "login": "mangotree623", "displayName": "mangotree623" },
                            "contentOffsetSeconds": 9,
                            "createdAt": "2026-09-09T16:26:40.987Z",
                            "message": {
                                "fragments": [{ "emote": null, "text": "LIVE  OOOO" }],
                                "userBadges": [
                                    { "id": "Ozs=", "setID": "", "version": "" },
                                    { "id": "cHJlbWl1bTsxOw==", "setID": "premium", "version": "1" }
                                ],
                                "userColor": "#FFBF00"
                            }
                        }},
                        { "cursor": "cur-2", "node": {
                            "id": "m2",
                            "commenter": { "login": "phaerless", "displayName": "Phaerless" },
                            "contentOffsetSeconds": 65,
                            "createdAt": "2026-09-09T16:27:41.132Z",
                            "message": {
                                "fragments": [
                                    { "emote": null, "text": "olha " },
                                    { "emote": { "id": "123171;0;11", "emoteID": "123171", "from": 0 }, "text": "CoolStoryBob" }
                                ],
                                "userBadges": [],
                                "userColor": "#1E90FF"
                            }
                        }}
                    ],
                    "pageInfo": { "hasNextPage": true, "hasPreviousPage": false }
                }
            }}
        })
    }

    fn msgs() -> Vec<Message> {
        parse_page(&page_fixture()).expect("página válida").messages
    }

    #[test]
    fn normaliza_a_pagina_de_comentarios() {
        let page = parse_page(&page_fixture()).expect("página válida");
        assert!(page.has_next);
        assert_eq!(page.cursor.as_deref(), Some("cur-2"));
        assert_eq!(page.messages.len(), 2);

        let m1 = &page.messages[0];
        assert_eq!(m1.id, "m1");
        assert_eq!(m1.offset, 9.0);
        assert_eq!(m1.author, "mangotree623");
        assert_eq!(m1.color, "#FFBF00");
        // O badge sem setID é descartado.
        assert_eq!(m1.badges, vec!["premium/1".to_string()]);
        assert_eq!(m1.text, "LIVE  OOOO");

        let m2 = &page.messages[1];
        assert_eq!(m2.text, "olha CoolStoryBob");
        assert_eq!(m2.fragments.len(), 2);
        assert_eq!(m2.fragments[1].emote.as_deref(), Some("123171"));
        assert_eq!(m2.display, "Phaerless");
    }

    #[test]
    fn recusa_pagina_sem_comentarios() {
        let v = json!({ "data": { "video": { "id": "1", "comments": null } } });
        assert!(parse_page(&v).is_err());
    }

    #[test]
    fn relogio_em_horas_minutos_segundos() {
        assert_eq!(fmt_clock(0.0), "00:00:00");
        assert_eq!(fmt_clock(65.4), "00:01:05");
        assert_eq!(fmt_clock(3661.0), "01:01:01");
        assert_eq!(fmt_clock(-5.0), "00:00:00");
    }

    #[test]
    fn csv_escapa_virgula_e_aspas() {
        let mut m = msgs();
        m[0].text = "oi, \"chat\"".into();
        let csv = to_csv(&m);
        let linhas: Vec<&str> = csv.lines().collect();
        assert_eq!(linhas[0], "time,offset_seconds,author,message");
        assert_eq!(linhas[1], "00:00:09,9,mangotree623,\"oi, \"\"chat\"\"\"");
        assert!(linhas[2].starts_with("00:01:05,65,Phaerless,"));
    }

    #[test]
    fn agrupa_em_janelas_e_corta_no_maximo_de_linhas() {
        let base = msgs();
        let mut lote = Vec::new();
        for i in 0..5 {
            let mut m = base[0].clone();
            m.id = format!("x{}", i);
            m.offset = 1.0 + i as f64 * 0.5;
            m.display = format!("u{}", i);
            m.text = format!("linha {}", i);
            lote.push(m);
        }
        lote.push(base[1].clone());
        let cues = to_cues(&lote, 5.0, 3);
        assert_eq!(cues.len(), 2);
        assert_eq!(cues[0].start_ms, 0);
        assert_eq!(cues[0].end_ms, 5000);
        assert_eq!(cues[0].text, "u2: linha 2\nu3: linha 3\nu4: linha 4");
        assert_eq!(cues[1].start_ms, 65_000);

        let srt = subtitle::to_srt(&cues);
        assert!(srt.starts_with("1\n00:00:00,000 --> 00:00:05,000\n"));
        assert!(srt.contains("00:01:05,000 --> 00:01:10,000"));
    }

    #[test]
    fn picos_por_minuto() {
        let base = msgs();
        let mut lote = Vec::new();
        for (i, offset) in [3.0, 10.0, 59.9, 70.0, 121.0, 122.0, 123.0]
            .iter()
            .enumerate()
        {
            let mut m = base[0].clone();
            m.id = format!("p{}", i);
            m.offset = *offset;
            lote.push(m);
        }
        let series = peaks(&lote, 60.0);
        assert_eq!(series.len(), 3);
        assert_eq!(series[0].count, 3);
        assert_eq!(series[0].clock, "00:00:00");
        assert_eq!(series[1].count, 1);
        assert_eq!(series[2].count, 3);
        assert_eq!(series[2].clock, "00:02:00");
    }

    #[test]
    fn ranking_de_quem_mais_falou() {
        let base = msgs();
        let mut lote = Vec::new();
        for (i, nome) in ["ana", "bia", "ana", "ana", "bia", "caio"]
            .iter()
            .enumerate()
        {
            let mut m = base[0].clone();
            m.id = format!("c{}", i);
            m.display = (*nome).to_string();
            lote.push(m);
        }
        let top = top_chatters(&lote, 2);
        assert_eq!(top.len(), 2);
        assert_eq!(top[0].author, "ana");
        assert_eq!(top[0].count, 3);
        assert_eq!(top[1].author, "bia");
    }

    #[tokio::test]
    #[ignore = "rede: exporta um trecho de chat de verdade em JSON/CSV/SRT"]
    async fn live_exporta_um_trecho() {
        let gql = Gql::new().expect("cliente");
        let data = gql
            .query(r#"{ user(login: "xqc") { videos(first: 1) { edges { node { id } } } } }"#)
            .await
            .expect("lista de VODs");
        let vod = data
            .pointer("/user/videos/edges/0/node/id")
            .and_then(|v| v.as_str())
            .expect("um VOD público")
            .to_string();
        let dir = super::super::super::temp_dir().join("tw-chat-teste");
        let _ = std::fs::remove_dir_all(&dir);
        let opts = Options {
            input: format!("https://www.twitch.tv/videos/{}", vod),
            out_dir: dir.to_string_lossy().to_string(),
            formats: vec!["json".into(), "csv".into(), "srt".into()],
            start_seconds: 0.0,
            end_seconds: 0.0,
            window_seconds: 5.0,
            max_lines: 6,
            delay_ms: 300,
            max_messages: 300,
        };
        let out = export(&opts, &super::super::super::noop_progress())
            .await
            .expect("exportação do chat");
        assert!(out.messages >= 100, "poucas mensagens: {}", out.messages);
        assert_eq!(out.files.len(), 3);
        for f in &out.files {
            let meta = std::fs::metadata(f).expect("arquivo exportado");
            assert!(meta.len() > 0, "arquivo vazio: {}", f);
        }
        println!(
            "VOD {} · {} mensagens em {} páginas · fallback por offset: {} · pico {:?}",
            out.video_id,
            out.messages,
            out.pages,
            out.offset_fallback,
            out.top_peaks.first()
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    #[ignore = "rede: puxa a primeira página de chat de um VOD público"]
    async fn live_primeira_pagina_de_chat() {
        let gql = Gql::new().expect("cliente");
        // Último VOD público de um canal grande, para o teste não envelhecer.
        let data = gql
            .query(r#"{ user(login: "xqc") { videos(first: 1) { edges { node { id } } } } }"#)
            .await
            .expect("lista de VODs");
        let vod = data
            .pointer("/user/videos/edges/0/node/id")
            .and_then(|v| v.as_str())
            .expect("um VOD público")
            .to_string();
        let info = gql.video(&vod).await.expect("metadados do VOD");
        assert!(info.duration_seconds > 0.0);

        let raw = gql
            .persisted(
                OP,
                COMMENTS_HASH,
                json!({ "videoID": vod, "contentOffsetSeconds": 0 }),
            )
            .await
            .expect("primeira página");
        assert!(
            first_error(&raw).is_none(),
            "erro do GQL: {:?}",
            first_error(&raw)
        );
        let page = parse_page(&raw).expect("página normalizada");
        assert!(!page.messages.is_empty(), "primeira página veio vazia");
        println!(
            "VOD {} ({}) · {} mensagens · hasNext={} · primeira: {:?}",
            vod,
            info.title,
            page.messages.len(),
            page.has_next,
            page.messages.first().map(|m| (&m.author, &m.text))
        );
    }
}

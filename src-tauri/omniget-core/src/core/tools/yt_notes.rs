//! Transcrição virando artigo: as linhas da legenda são costuradas em
//! parágrafos e saem como Markdown legível.
//!
//! Legenda não é texto: é uma fila de pedaços de duas linhas cortados pela
//! largura da tela, e a legenda automática do YouTube ainda repete a linha
//! anterior a cada quadro novo. Por isso nada aqui costura por quebra de
//! linha — a costura olha pausa entre falas e pontuação, e a repetição da
//! legenda rolante é desfeita antes.

use anyhow::anyhow;
use serde::{Deserialize, Serialize};

use super::subtitle::Cue;

// ── Limpeza e deduplicação ─────────────────────────────────────────────

/// Tira marcação de karaokê do VTT (`<c>`, `<00:00:01.500>`), entidades e
/// espaço repetido.
pub fn clean_line(line: &str) -> String {
    let mut plain = String::with_capacity(line.len());
    let mut depth = 0usize;
    for ch in line.chars() {
        match ch {
            '<' => depth += 1,
            '>' => {
                if depth > 0 {
                    depth -= 1;
                    plain.push(' ');
                } else {
                    plain.push('>');
                }
            }
            _ => {
                if depth == 0 {
                    plain.push(ch);
                }
            }
        }
    }
    let decoded = plain
        .replace("&nbsp;", " ")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&apos;", "'")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&amp;", "&");
    decoded.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Desfaz a legenda rolante: a mesma frase volta a cada bloco, ora inteira,
/// ora como prefixo do bloco seguinte. Sobra só o que é novo.
pub fn dedup_cues(cues: &[Cue]) -> Vec<Cue> {
    let mut out: Vec<Cue> = Vec::new();
    let mut prev_lines: Vec<String> = Vec::new();
    let mut prev_full = String::new();
    for c in cues {
        let lines: Vec<String> = c
            .text
            .lines()
            .map(clean_line)
            .filter(|l| !l.is_empty())
            .collect();
        let full = lines.join(" ");
        if full.is_empty() {
            continue;
        }
        let fresh: Vec<String> = lines
            .iter()
            .filter(|l| !prev_lines.iter().any(|p| p == *l))
            .cloned()
            .collect();
        let mut text = fresh.join(" ");
        if !prev_full.is_empty() {
            if let Some(rest) = text.strip_prefix(prev_full.as_str()) {
                text = rest.trim_start().to_string();
            }
        }
        prev_lines = lines;
        prev_full = full;
        if text.is_empty() {
            continue;
        }
        out.push(Cue {
            start_ms: c.start_ms,
            end_ms: c.end_ms.max(c.start_ms),
            text,
        });
    }
    out
}

// ── Costura em parágrafos ──────────────────────────────────────────────

#[derive(Debug, Clone, Deserialize)]
pub struct StitchOptions {
    /// Pausa (em segundos) que já vale um parágrafo novo.
    #[serde(default = "default_gap")]
    pub gap_seconds: f64,
    /// A partir daqui o parágrafo fecha na primeira frase que terminar.
    #[serde(default = "default_soft")]
    pub soft_chars: usize,
    /// Limite duro: fecha mesmo sem ponto final.
    #[serde(default = "default_hard")]
    pub hard_chars: usize,
}

fn default_gap() -> f64 {
    2.5
}
fn default_soft() -> usize {
    420
}
fn default_hard() -> usize {
    1200
}

impl Default for StitchOptions {
    fn default() -> Self {
        Self {
            gap_seconds: default_gap(),
            soft_chars: default_soft(),
            hard_chars: default_hard(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Paragraph {
    pub start_ms: i64,
    pub end_ms: i64,
    pub text: String,
}

/// Fim de frase: ponto, exclamação, interrogação ou reticências, aceitando
/// aspas e parênteses fechando depois.
pub fn ends_sentence(text: &str) -> bool {
    let trimmed = text.trim_end_matches([' ', '"', '\'', ')', ']', '”', '’', '»']);
    matches!(
        trimmed.chars().last(),
        Some('.') | Some('!') | Some('?') | Some('…')
    )
}

fn join_text(acc: &str, next: &str) -> String {
    if acc.is_empty() {
        return next.to_string();
    }
    // Palavra partida no fim do bloco ("inte-" + "ressante").
    if acc.ends_with('-') {
        return format!("{}{}", acc.trim_end_matches('-'), next);
    }
    if next.starts_with([',', '.', '!', '?', ';', ':']) {
        return format!("{}{}", acc, next);
    }
    format!("{} {}", acc, next)
}

pub fn stitch(cues: &[Cue], opts: &StitchOptions) -> Vec<Paragraph> {
    stitch_at(cues, &[], opts)
}

/// Igual a `stitch`, mas `boundaries` (em ms, ordenados) também forçam
/// parágrafo novo — é assim que capítulo não fica no meio de um bloco.
pub fn stitch_at(cues: &[Cue], boundaries: &[i64], opts: &StitchOptions) -> Vec<Paragraph> {
    let cues = dedup_cues(cues);
    let gap_ms = (opts.gap_seconds.max(0.0) * 1000.0).round() as i64;
    let soft = opts.soft_chars.max(40);
    let hard = opts.hard_chars.max(soft);
    let mut out: Vec<Paragraph> = Vec::new();
    let mut cur: Option<Paragraph> = None;
    let mut prev_end: i64 = 0;
    for c in &cues {
        let mut split = false;
        if let Some(p) = &cur {
            let len = p.text.chars().count();
            let crossed = boundaries
                .iter()
                .any(|b| *b > p.start_ms && *b <= c.start_ms);
            split = crossed
                || c.start_ms - prev_end > gap_ms
                || (len >= soft && ends_sentence(&p.text))
                || len >= hard;
        }
        if split {
            if let Some(p) = cur.take() {
                out.push(p);
            }
        }
        match cur.as_mut() {
            Some(p) => {
                p.text = join_text(&p.text, &c.text);
                p.end_ms = c.end_ms.max(p.end_ms);
            }
            None => {
                cur = Some(Paragraph {
                    start_ms: c.start_ms,
                    end_ms: c.end_ms,
                    text: c.text.clone(),
                })
            }
        }
        prev_end = c.end_ms;
    }
    if let Some(p) = cur {
        out.push(p);
    }
    out
}

// ── Markdown ───────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Chapter {
    pub start_ms: i64,
    pub title: String,
}

/// `1:02:03` quando passa da hora, `4:05` quando não passa.
pub fn fmt_stamp(ms: i64) -> String {
    let total = (ms.max(0) / 1000) as u64;
    let (h, m, s) = (total / 3600, (total / 60) % 60, total % 60);
    if h > 0 {
        format!("{}:{:02}:{:02}", h, m, s)
    } else {
        format!("{}:{:02}", m, s)
    }
}

/// Link do YouTube com o segundo exato. Sem URL, devolve `None`.
pub fn stamp_url(base: &str, ms: i64) -> Option<String> {
    let base = base.trim();
    if base.is_empty() {
        return None;
    }
    let sep = if base.contains('?') { '&' } else { '?' };
    Some(format!("{}{}t={}s", base, sep, ms.max(0) / 1000))
}

fn stamp_md(base: &str, ms: i64) -> String {
    match stamp_url(base, ms) {
        Some(u) => format!("[{}]({})", fmt_stamp(ms), u),
        None => fmt_stamp(ms),
    }
}

fn yaml_value(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}

#[derive(Debug, Clone, Serialize)]
pub struct NotesDoc {
    pub title: String,
    pub channel: String,
    pub url: String,
    pub duration_seconds: f64,
    pub language: String,
    pub generated: String,
    pub chapters: Vec<Chapter>,
    pub paragraphs: Vec<Paragraph>,
    pub timestamps: bool,
}

pub fn to_markdown(doc: &NotesDoc) -> String {
    let mut out = String::new();
    out.push_str("---\n");
    out.push_str(&format!("title: {}\n", yaml_value(&doc.title)));
    if !doc.channel.is_empty() {
        out.push_str(&format!("channel: {}\n", yaml_value(&doc.channel)));
    }
    if !doc.url.is_empty() {
        out.push_str(&format!("url: {}\n", yaml_value(&doc.url)));
    }
    if doc.duration_seconds > 0.0 {
        out.push_str(&format!(
            "duration: {}\n",
            yaml_value(&fmt_stamp((doc.duration_seconds * 1000.0) as i64))
        ));
    }
    if !doc.language.is_empty() {
        out.push_str(&format!("language: {}\n", yaml_value(&doc.language)));
    }
    if !doc.generated.is_empty() {
        out.push_str(&format!("generated: {}\n", yaml_value(&doc.generated)));
    }
    out.push_str("---\n\n");
    if !doc.title.is_empty() {
        out.push_str(&format!("# {}\n\n", doc.title));
    }

    let mut next = 0usize;
    for p in &doc.paragraphs {
        while next < doc.chapters.len() && doc.chapters[next].start_ms <= p.start_ms {
            let ch = &doc.chapters[next];
            if doc.timestamps {
                out.push_str(&format!(
                    "## {} {}\n\n",
                    stamp_md(&doc.url, ch.start_ms),
                    ch.title
                ));
            } else {
                out.push_str(&format!("## {}\n\n", ch.title));
            }
            next += 1;
        }
        if doc.timestamps {
            out.push_str(&format!("**{}** ", stamp_md(&doc.url, p.start_ms)));
        }
        out.push_str(&p.text);
        out.push_str("\n\n");
    }
    // Capítulo depois do último parágrafo (vídeo que termina em vinheta).
    for ch in doc.chapters.iter().skip(next) {
        out.push_str(&format!("## {}\n\n", ch.title));
    }
    out
}

pub fn word_count(paragraphs: &[Paragraph]) -> usize {
    paragraphs
        .iter()
        .map(|p| p.text.split_whitespace().count())
        .sum()
}

// ── Metadados do yt-dlp ────────────────────────────────────────────────

#[derive(Debug, Clone, Default, Serialize)]
pub struct VideoMeta {
    pub id: String,
    pub title: String,
    pub channel: String,
    pub url: String,
    pub duration_seconds: f64,
    pub language: String,
    pub chapters: Vec<Chapter>,
}

pub fn parse_video_meta(json: &str) -> anyhow::Result<VideoMeta> {
    let v: serde_json::Value = serde_json::from_str(json)?;
    let id = v["id"].as_str().unwrap_or_default().to_string();
    let url = v["webpage_url"]
        .as_str()
        .map(|s| s.to_string())
        .unwrap_or_else(|| {
            if id.is_empty() {
                String::new()
            } else {
                format!("https://www.youtube.com/watch?v={}", id)
            }
        });
    let channel = v["channel"]
        .as_str()
        .or_else(|| v["uploader"].as_str())
        .unwrap_or_default()
        .to_string();
    let mut chapters = Vec::new();
    if let Some(list) = v["chapters"].as_array() {
        for c in list {
            let start = c["start_time"].as_f64().unwrap_or(0.0);
            let title = c["title"].as_str().unwrap_or_default().trim().to_string();
            if title.is_empty() {
                continue;
            }
            chapters.push(Chapter {
                start_ms: (start * 1000.0).round() as i64,
                title,
            });
        }
    }
    chapters.sort_by_key(|c| c.start_ms);
    Ok(VideoMeta {
        id,
        title: v["title"].as_str().unwrap_or_default().to_string(),
        channel,
        url,
        duration_seconds: v["duration"].as_f64().unwrap_or(0.0),
        language: v["language"].as_str().unwrap_or_default().to_string(),
        chapters,
    })
}

// ── Execução ───────────────────────────────────────────────────────────

#[derive(Debug, Clone, Deserialize)]
pub struct Options {
    /// Link do vídeo. Vazio quando a fonte é um arquivo local.
    #[serde(default)]
    pub url: String,
    /// Legenda já baixada (SRT/VTT/ASS). Tem prioridade sobre a rede.
    #[serde(default)]
    pub subtitle_path: String,
    /// Mídia local para transcrever com o Whisper.
    #[serde(default)]
    pub media_path: String,
    #[serde(default = "default_langs")]
    pub sub_langs: String,
    /// Cair no Whisper quando o vídeo não tiver legenda nenhuma.
    #[serde(default)]
    pub allow_whisper: bool,
    #[serde(default = "default_model")]
    pub whisper_model: String,
    #[serde(default = "default_true")]
    pub timestamps: bool,
    #[serde(default = "default_true")]
    pub use_chapters: bool,
    /// Arquivo .md de saída; vazio = não escreve, só devolve o texto.
    #[serde(default)]
    pub output_path: String,
    #[serde(default)]
    pub stitch: StitchOptions,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub account_slug: Option<String>,
    #[serde(default)]
    pub session_netscape: Option<String>,
}

fn default_langs() -> String {
    "pt,pt-BR,pt-PT,en,en-US".to_string()
}
fn default_model() -> String {
    "base".to_string()
}
fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Serialize)]
pub struct NotesResult {
    pub markdown: String,
    pub path: String,
    /// "legenda-local" | "legenda-youtube" | "whisper"
    pub source: String,
    pub title: String,
    pub channel: String,
    pub url: String,
    pub duration_seconds: f64,
    pub language: String,
    pub words: usize,
    pub paragraphs: usize,
    pub chapters: usize,
    pub used_session: bool,
}

const ID: &str = "yt-notes";

pub async fn run(opts: Options, progress: super::ProgressFn) -> anyhow::Result<NotesResult> {
    let used_session = opts
        .session_netscape
        .as_ref()
        .map(|s| !s.trim().is_empty())
        .unwrap_or(false);
    let mut meta = VideoMeta {
        title: opts.title.clone(),
        url: opts.url.trim().to_string(),
        ..Default::default()
    };
    let source;
    let cues: Vec<Cue>;

    if !opts.subtitle_path.trim().is_empty() {
        super::report(&progress, ID, "progress", 1, Some(4), None);
        let text = std::fs::read_to_string(opts.subtitle_path.trim())
            .map_err(|e| anyhow!("não consegui ler a legenda: {}", e))?;
        cues = super::subtitle::parse(&text)?;
        source = "legenda-local".to_string();
        if !opts.url.trim().is_empty() {
            if let Ok(m) = fetch_meta(&opts).await {
                meta = merge_meta(meta, m);
            }
        }
    } else if !opts.url.trim().is_empty() {
        super::report(&progress, ID, "progress", 1, Some(4), None);
        meta = merge_meta(meta, fetch_meta(&opts).await?);
        super::report(&progress, ID, "progress", 2, Some(4), None);
        match fetch_subtitles(&opts).await {
            Ok(Some(text)) => {
                cues = super::subtitle::parse(&text)?;
                source = "legenda-youtube".to_string();
            }
            Ok(None) | Err(_) if opts.allow_whisper => {
                super::report(&progress, ID, "progress", 3, Some(4), None);
                let audio = download_audio(&opts).await?;
                let r = transcribe(&audio, &opts, progress.clone()).await?;
                let _ = std::fs::remove_file(&audio);
                if meta.language.is_empty() {
                    meta.language = r.language.clone();
                }
                cues = r.cues;
                source = "whisper".to_string();
            }
            Ok(None) => {
                return Err(anyhow!(
                    "esse vídeo não tem legenda; ligue o Whisper para transcrever"
                ))
            }
            Err(e) => return Err(e),
        }
    } else if !opts.media_path.trim().is_empty() {
        let path = std::path::PathBuf::from(opts.media_path.trim());
        let r = transcribe(&path, &opts, progress.clone()).await?;
        if meta.title.is_empty() {
            meta.title = path
                .file_stem()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_default();
        }
        meta.language = r.language.clone();
        meta.duration_seconds = r.seconds;
        cues = r.cues;
        source = "whisper".to_string();
    } else {
        return Err(anyhow!(
            "informe um link, uma legenda ou um arquivo de mídia"
        ));
    }

    if cues.is_empty() {
        return Err(anyhow!("a transcrição saiu vazia"));
    }
    super::report(&progress, ID, "progress", 4, Some(4), None);
    let chapters = if opts.use_chapters {
        meta.chapters.clone()
    } else {
        Vec::new()
    };
    let boundaries: Vec<i64> = chapters.iter().map(|c| c.start_ms).collect();
    let paragraphs = stitch_at(&cues, &boundaries, &opts.stitch);
    let doc = NotesDoc {
        title: meta.title.clone(),
        channel: meta.channel.clone(),
        url: meta.url.clone(),
        duration_seconds: meta.duration_seconds,
        language: meta.language.clone(),
        generated: chrono::Local::now().format("%Y-%m-%d").to_string(),
        chapters: chapters.clone(),
        paragraphs: paragraphs.clone(),
        timestamps: opts.timestamps,
    };
    let markdown = to_markdown(&doc);

    let mut path = String::new();
    let target = opts.output_path.trim();
    if !target.is_empty() {
        let p = std::path::PathBuf::from(target);
        if let Some(parent) = p.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent)?;
            }
        }
        std::fs::write(&p, markdown.as_bytes())?;
        path = p.to_string_lossy().to_string();
    }
    super::report(&progress, ID, "done", 4, Some(4), None);

    Ok(NotesResult {
        words: word_count(&paragraphs),
        paragraphs: paragraphs.len(),
        chapters: chapters.len(),
        markdown,
        path,
        source,
        title: meta.title,
        channel: meta.channel,
        url: meta.url,
        duration_seconds: meta.duration_seconds,
        language: meta.language,
        used_session,
    })
}

fn merge_meta(base: VideoMeta, fresh: VideoMeta) -> VideoMeta {
    VideoMeta {
        id: fresh.id,
        title: if base.title.is_empty() {
            fresh.title
        } else {
            base.title
        },
        channel: fresh.channel,
        url: if fresh.url.is_empty() {
            base.url
        } else {
            fresh.url
        },
        duration_seconds: fresh.duration_seconds,
        language: fresh.language,
        chapters: fresh.chapters,
    }
}

async fn fetch_meta(opts: &Options) -> anyhow::Result<VideoMeta> {
    let mut args = vec![
        "-J".to_string(),
        "--no-warnings".to_string(),
        "--no-playlist".to_string(),
        "--skip-download".to_string(),
    ];
    let cookie = super::yt_archive::CookieFile::new(opts.session_netscape.as_deref())?;
    cookie.push_args(&mut args);
    args.push(opts.url.trim().to_string());
    let out = super::yt_archive::run_ytdlp(&args).await?;
    parse_video_meta(&out)
}

/// Baixa a legenda (manual ou automática) em VTT e devolve o conteúdo.
async fn fetch_subtitles(opts: &Options) -> anyhow::Result<Option<String>> {
    let dir = super::temp_dir().join(format!("yt-notes-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir)?;
    let template = dir.join("legenda.%(ext)s");
    let mut args = vec![
        "--skip-download".to_string(),
        "--no-warnings".to_string(),
        "--no-playlist".to_string(),
        "--write-subs".to_string(),
        "--write-auto-subs".to_string(),
        "--sub-format".to_string(),
        "vtt/best".to_string(),
        "--sub-langs".to_string(),
        opts.sub_langs.clone(),
        "-o".to_string(),
        template.to_string_lossy().to_string(),
    ];
    let cookie = super::yt_archive::CookieFile::new(opts.session_netscape.as_deref())?;
    cookie.push_args(&mut args);
    args.push(opts.url.trim().to_string());
    let _ = super::yt_archive::run_ytdlp(&args).await;

    let mut found: Option<String> = None;
    if let Ok(entries) = std::fs::read_dir(&dir) {
        for e in entries.flatten() {
            let p = e.path();
            let ext = p
                .extension()
                .map(|x| x.to_string_lossy().to_lowercase())
                .unwrap_or_default();
            if matches!(ext.as_str(), "vtt" | "srt" | "ass" | "srv3" | "json3") {
                if let Ok(text) = std::fs::read_to_string(&p) {
                    if text.contains("-->") {
                        found = Some(text);
                        break;
                    }
                }
            }
        }
    }
    let _ = std::fs::remove_dir_all(&dir);
    Ok(found)
}

async fn download_audio(opts: &Options) -> anyhow::Result<std::path::PathBuf> {
    let out = super::temp_dir().join(format!("yt-notes-{}.m4a", uuid::Uuid::new_v4()));
    let mut args = vec![
        "-f".to_string(),
        "bestaudio/best".to_string(),
        "--no-warnings".to_string(),
        "--no-playlist".to_string(),
        "-o".to_string(),
        out.to_string_lossy().to_string(),
    ];
    let cookie = super::yt_archive::CookieFile::new(opts.session_netscape.as_deref())?;
    cookie.push_args(&mut args);
    args.push(opts.url.trim().to_string());
    super::yt_archive::run_ytdlp(&args).await?;
    if !out.exists() {
        return Err(anyhow!("não consegui baixar o áudio para transcrever"));
    }
    Ok(out)
}

/// O Whisper devolve o `Cue` do `subtitle_merge` (ms sem sinal); aqui tudo
/// trabalha com o `Cue` do módulo de legenda.
struct Transcript {
    cues: Vec<Cue>,
    language: String,
    seconds: f64,
}

async fn transcribe(
    input: &std::path::Path,
    opts: &Options,
    progress: super::ProgressFn,
) -> anyhow::Result<Transcript> {
    let r = super::whisper::transcribe(
        super::whisper::TranscribeOptions {
            input: input.to_string_lossy().to_string(),
            model: opts.whisper_model.clone(),
            language: "auto".to_string(),
            translate: false,
            max_len: 0,
            prompt: String::new(),
            output_dir: super::temp_dir().to_string_lossy().to_string(),
            threads: 0,
        },
        progress,
    )
    .await?;
    Ok(Transcript {
        cues: r
            .cues
            .into_iter()
            .map(|c| Cue {
                start_ms: c.start_ms as i64,
                end_ms: c.end_ms as i64,
                text: c.text,
            })
            .collect(),
        language: r.language,
        seconds: r.seconds,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const VTT_ROLANTE: &str = "WEBVTT

00:00:01.000 --> 00:00:03.000
o que a gente vai ver hoje

00:00:03.000 --> 00:00:05.000
o que a gente vai ver hoje
é como cortar silêncio

00:00:05.000 --> 00:00:07.000
é como cortar silêncio
de uma gravação longa.

00:00:12.000 --> 00:00:15.000
Depois da pausa, o assunto muda.
";

    const SRT_SIMPLES: &str = "1
00:00:00,000 --> 00:00:02,000
Primeira frase.

2
00:00:02,000 --> 00:00:04,000
Segunda frase, sem pausa.

3
00:00:20,000 --> 00:00:22,000
Já isto é outro assunto.
";

    #[test]
    fn cleans_karaoke_tags_and_entities() {
        assert_eq!(
            clean_line("<00:00:01.500><c>bom</c>  dia &amp; boa   noite"),
            "bom dia & boa noite"
        );
        assert_eq!(clean_line("   "), "");
    }

    #[test]
    fn rolling_captions_are_deduped() {
        let cues = super::super::subtitle::parse(VTT_ROLANTE).expect("vtt válido");
        let deduped = dedup_cues(&cues);
        let text: Vec<&str> = deduped.iter().map(|c| c.text.as_str()).collect();
        assert_eq!(
            text,
            vec![
                "o que a gente vai ver hoje",
                "é como cortar silêncio",
                "de uma gravação longa.",
                "Depois da pausa, o assunto muda.",
            ]
        );
    }

    #[test]
    fn stitching_joins_lines_and_breaks_on_the_pause() {
        let cues = super::super::subtitle::parse(VTT_ROLANTE).expect("vtt válido");
        let paras = stitch(&cues, &StitchOptions::default());
        assert_eq!(paras.len(), 2, "a pausa de 5 s abre parágrafo novo");
        assert_eq!(
            paras[0].text,
            "o que a gente vai ver hoje é como cortar silêncio de uma gravação longa."
        );
        assert_eq!(paras[0].start_ms, 1000);
        assert_eq!(paras[0].end_ms, 7000);
        assert_eq!(paras[1].text, "Depois da pausa, o assunto muda.");
    }

    #[test]
    fn srt_and_vtt_stitch_the_same_way() {
        let cues = super::super::subtitle::parse(SRT_SIMPLES).expect("srt válido");
        let paras = stitch(&cues, &StitchOptions::default());
        assert_eq!(paras.len(), 2);
        assert_eq!(paras[0].text, "Primeira frase. Segunda frase, sem pausa.");
    }

    #[test]
    fn long_paragraph_closes_at_the_first_full_stop() {
        let mut cues = Vec::new();
        for i in 0..40 {
            cues.push(Cue {
                start_ms: i * 1000,
                end_ms: i * 1000 + 900,
                text: if i % 5 == 4 {
                    format!("e a frase {} termina.", i)
                } else {
                    format!("palavra {} comprida sem fim", i)
                },
            });
        }
        let paras = stitch(&cues, &StitchOptions::default());
        assert!(
            paras.len() > 1,
            "devia ter quebrado em mais de um parágrafo"
        );
        for p in paras.iter().take(paras.len() - 1) {
            assert!(
                ends_sentence(&p.text),
                "parágrafo cortado no meio da frase: {}",
                p.text
            );
        }
    }

    #[test]
    fn hard_limit_breaks_even_without_punctuation() {
        let cues: Vec<Cue> = (0..80)
            .map(|i| Cue {
                start_ms: i * 500,
                end_ms: i * 500 + 400,
                text: format!("sem pontuação {} nenhuma aqui", i),
            })
            .collect();
        let opts = StitchOptions {
            gap_seconds: 10.0,
            soft_chars: 100,
            hard_chars: 200,
        };
        let paras = stitch(&cues, &opts);
        assert!(paras.len() >= 5, "saíram {} parágrafos", paras.len());
        assert!(paras.iter().all(|p| p.text.chars().count() < 260));
    }

    #[test]
    fn chapter_boundary_forces_a_new_paragraph() {
        let cues = super::super::subtitle::parse(SRT_SIMPLES).expect("srt válido");
        let paras = stitch_at(&cues, &[2000], &StitchOptions::default());
        assert_eq!(paras.len(), 3, "o capítulo em 2 s parte o primeiro bloco");
        assert_eq!(paras[0].text, "Primeira frase.");
        assert_eq!(paras[1].text, "Segunda frase, sem pausa.");
    }

    #[test]
    fn an_identical_line_repeated_is_the_same_line() {
        // Legenda automática repete o bloco inteiro; texto igual colado é
        // repetição, não fala nova.
        let cues: Vec<Cue> = (0..4)
            .map(|i| Cue {
                start_ms: i * 1000,
                end_ms: i * 1000 + 900,
                text: "a mesma coisa".to_string(),
            })
            .collect();
        assert_eq!(dedup_cues(&cues).len(), 1);
    }

    #[test]
    fn hyphen_at_the_end_glues_the_word() {
        let cues = vec![
            Cue {
                start_ms: 0,
                end_ms: 500,
                text: "inte-".into(),
            },
            Cue {
                start_ms: 500,
                end_ms: 1000,
                text: "ressante".into(),
            },
        ];
        let paras = stitch(&cues, &StitchOptions::default());
        assert_eq!(paras[0].text, "interessante");
    }

    #[test]
    fn timestamps_are_formatted_and_linked() {
        assert_eq!(fmt_stamp(0), "0:00");
        assert_eq!(fmt_stamp(65_000), "1:05");
        assert_eq!(fmt_stamp(3_725_000), "1:02:05");
        assert_eq!(fmt_stamp(-10), "0:00");
        assert_eq!(
            stamp_url("https://youtu.be/abc", 65_000).as_deref(),
            Some("https://youtu.be/abc?t=65s")
        );
        assert_eq!(
            stamp_url("https://www.youtube.com/watch?v=abc", 65_000).as_deref(),
            Some("https://www.youtube.com/watch?v=abc&t=65s")
        );
        assert_eq!(stamp_url("", 1000), None);
    }

    fn doc_de_teste(timestamps: bool) -> NotesDoc {
        NotesDoc {
            title: "Aula de \"corte\"".into(),
            channel: "Canal".into(),
            url: "https://www.youtube.com/watch?v=abc".into(),
            duration_seconds: 754.0,
            language: "pt".into(),
            generated: "2026-09-09".into(),
            chapters: vec![
                Chapter {
                    start_ms: 0,
                    title: "Abertura".into(),
                },
                Chapter {
                    start_ms: 20_000,
                    title: "Outro assunto".into(),
                },
            ],
            paragraphs: vec![
                Paragraph {
                    start_ms: 0,
                    end_ms: 4000,
                    text: "Primeira frase. Segunda frase.".into(),
                },
                Paragraph {
                    start_ms: 20_000,
                    end_ms: 22_000,
                    text: "Já isto é outro assunto.".into(),
                },
            ],
            timestamps,
        }
    }

    #[test]
    fn markdown_has_front_matter_headings_and_links() {
        let md = to_markdown(&doc_de_teste(true));
        assert!(md.starts_with("---\n"), "sem front-matter:\n{}", md);
        assert!(md.contains("title: \"Aula de \\\"corte\\\"\""), "{}", md);
        assert!(md.contains("channel: \"Canal\""));
        assert!(md.contains("duration: \"12:34\""));
        assert!(md.contains("generated: \"2026-09-09\""));
        assert!(md.contains("# Aula de \"corte\""));
        assert!(md.contains("## [0:00](https://www.youtube.com/watch?v=abc&t=0s) Abertura"));
        assert!(md.contains("## [0:20](https://www.youtube.com/watch?v=abc&t=20s) Outro assunto"));
        assert!(md.contains("**[0:00](https://www.youtube.com/watch?v=abc&t=0s)** Primeira frase."));
    }

    #[test]
    fn markdown_without_timestamps_has_no_links() {
        let md = to_markdown(&doc_de_teste(false));
        assert!(!md.contains("&t="), "{}", md);
        assert!(md.contains("## Abertura"));
        assert!(md.contains("\nPrimeira frase. Segunda frase.\n"));
    }

    #[test]
    fn word_count_counts_the_paragraphs() {
        assert_eq!(word_count(&doc_de_teste(true).paragraphs), 9);
    }

    #[test]
    fn parses_the_metadata_of_ytdlp() {
        let json = r#"{
            "id": "abc12345678",
            "title": "Como cortar silêncio",
            "uploader": "Fulano",
            "channel": "Canal do Fulano",
            "duration": 754.2,
            "language": "pt",
            "webpage_url": "https://www.youtube.com/watch?v=abc12345678",
            "chapters": [
                {"start_time": 0.0, "title": "Intro"},
                {"start_time": 120.5, "title": "Prática"},
                {"start_time": 300.0, "title": "   "}
            ]
        }"#;
        let m = parse_video_meta(json).expect("json válido");
        assert_eq!(m.id, "abc12345678");
        assert_eq!(m.channel, "Canal do Fulano");
        assert_eq!(m.title, "Como cortar silêncio");
        assert!((m.duration_seconds - 754.2).abs() < 1e-9);
        assert_eq!(m.chapters.len(), 2, "capítulo sem título não entra");
        assert_eq!(m.chapters[1].start_ms, 120_500);
    }

    #[test]
    fn metadata_without_webpage_url_builds_one() {
        let m = parse_video_meta(r#"{"id":"abc12345678","title":"x"}"#).expect("json válido");
        assert_eq!(m.url, "https://www.youtube.com/watch?v=abc12345678");
        assert!(m.chapters.is_empty());
    }

    #[test]
    fn empty_transcript_is_an_error_not_an_empty_file() {
        assert!(stitch(&[], &StitchOptions::default()).is_empty());
    }

    /// Precisa de rede e do yt-dlp.
    /// `cargo test -p omniget-core --lib -- --ignored live_yt_notes`
    #[tokio::test]
    #[ignore]
    async fn live_yt_notes_builds_markdown_from_a_real_video() {
        let opts = Options {
            url: "https://www.youtube.com/watch?v=jNQXAC9IVRw".into(),
            subtitle_path: String::new(),
            media_path: String::new(),
            sub_langs: default_langs(),
            allow_whisper: false,
            whisper_model: default_model(),
            timestamps: true,
            use_chapters: true,
            output_path: String::new(),
            stitch: StitchOptions::default(),
            title: String::new(),
            account_slug: None,
            session_netscape: None,
        };
        let r = run(opts, super::super::noop_progress())
            .await
            .expect("o vídeo devia render notas");
        assert!(r.markdown.contains("---"));
        eprintln!("{} palavras, {} parágrafos", r.words, r.paragraphs);
    }
}

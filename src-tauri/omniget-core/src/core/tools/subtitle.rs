//! Legenda: converter entre SRT/VTT/ASS, ressincronizar e queimar no vídeo.
//!
//! O parser é próprio (Rust puro) porque o que se precisa aqui é pouco e o
//! FFmpeg não sabe deslocar tempo nem esticar por âncora. A queima usa o
//! filtro `subtitles`/`ass` do FFmpeg gerido, que já traz libass.

use std::path::{Path, PathBuf};

use anyhow::anyhow;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Cue {
    pub start_ms: i64,
    pub end_ms: i64,
    pub text: String,
}

// ── Leitura ────────────────────────────────────────────────────────────

/// `00:01:02,500` (SRT), `00:01:02.500` (VTT) ou `0:01:02.50` (ASS).
pub fn parse_time(s: &str) -> Option<i64> {
    let s = s.trim().replace(',', ".");
    let mut parts = s.split(':').collect::<Vec<_>>();
    if parts.is_empty() || parts.len() > 3 {
        return None;
    }
    let secs: f64 = parts.pop()?.parse().ok()?;
    let mins: i64 = parts.pop().map(|m| m.parse().ok()).unwrap_or(Some(0))?;
    let hours: i64 = parts.pop().map(|h| h.parse().ok()).unwrap_or(Some(0))?;
    Some((hours * 3600 + mins * 60) * 1000 + (secs * 1000.0).round() as i64)
}

pub fn fmt_srt_time(ms: i64) -> String {
    let ms = ms.max(0);
    format!(
        "{:02}:{:02}:{:02},{:03}",
        ms / 3_600_000,
        (ms / 60_000) % 60,
        (ms / 1000) % 60,
        ms % 1000
    )
}

pub fn fmt_vtt_time(ms: i64) -> String {
    fmt_srt_time(ms).replace(',', ".")
}

/// ASS conta centésimos, não milésimos, e a hora não tem zero à esquerda.
pub fn fmt_ass_time(ms: i64) -> String {
    let ms = ms.max(0);
    format!(
        "{}:{:02}:{:02}.{:02}",
        ms / 3_600_000,
        (ms / 60_000) % 60,
        (ms / 1000) % 60,
        (ms % 1000) / 10
    )
}

fn parse_timed_blocks(text: &str, arrow: &str) -> Vec<Cue> {
    let mut cues = Vec::new();
    let mut pending: Option<(i64, i64)> = None;
    let mut buffer: Vec<String> = Vec::new();
    let flush = |pending: &mut Option<(i64, i64)>, buffer: &mut Vec<String>, out: &mut Vec<Cue>| {
        if let Some((start, end)) = pending.take() {
            let body = buffer.join("\n").trim().to_string();
            if !body.is_empty() {
                out.push(Cue {
                    start_ms: start,
                    end_ms: end,
                    text: body,
                });
            }
        }
        buffer.clear();
    };

    for line in text.lines() {
        let line = line.trim_end_matches('\r');
        if let Some((a, b)) = line.split_once(arrow) {
            // Uma linha de tempo fecha o bloco anterior.
            flush(&mut pending, &mut buffer, &mut cues);
            let start = parse_time(a);
            // O VTT pode ter posição depois do tempo final ("line:90%").
            let end = b.split_whitespace().next().and_then(parse_time);
            if let (Some(s), Some(e)) = (start, end) {
                pending = Some((s, e));
            }
        } else if line.trim().is_empty() {
            flush(&mut pending, &mut buffer, &mut cues);
        } else if pending.is_some() {
            buffer.push(line.to_string());
        }
    }
    flush(&mut pending, &mut buffer, &mut cues);
    cues
}

fn parse_ass(text: &str) -> Vec<Cue> {
    let mut cues = Vec::new();
    let mut fields: Vec<String> = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("Format:") {
            if !fields.is_empty() && !line.contains("Start") {
                continue;
            }
            fields = rest
                .split(',')
                .map(|f| f.trim().to_lowercase())
                .collect::<Vec<_>>();
        } else if let Some(rest) = line.strip_prefix("Dialogue:") {
            let want = fields.len().max(10);
            let parts: Vec<&str> = rest.splitn(want, ',').collect();
            let idx = |name: &str| fields.iter().position(|f| f == name);
            let (si, ei, ti) = (
                idx("start").unwrap_or(1),
                idx("end").unwrap_or(2),
                idx("text").unwrap_or(9),
            );
            let (start, end) = match (
                parts.get(si).and_then(|s| parse_time(s)),
                parts.get(ei).and_then(|s| parse_time(s)),
            ) {
                (Some(s), Some(e)) => (s, e),
                _ => continue,
            };
            let body = parts.get(ti).copied().unwrap_or("");
            // Tira as marcas de override ({\an8}, {\i1}) e as quebras do ASS.
            let mut clean = String::new();
            let mut depth = 0;
            for c in body.chars() {
                match c {
                    '{' => depth += 1,
                    '}' => depth = (depth - 1).max(0),
                    _ if depth == 0 => clean.push(c),
                    _ => {}
                }
            }
            let clean = clean.replace("\\N", "\n").replace("\\n", "\n");
            if !clean.trim().is_empty() {
                cues.push(Cue {
                    start_ms: start,
                    end_ms: end,
                    text: clean.trim().to_string(),
                });
            }
        }
    }
    cues
}

/// Descobre o formato pelo conteúdo, não pela extensão — arquivo de legenda
/// vem com o nome errado o tempo todo.
pub fn parse(text: &str) -> anyhow::Result<Vec<Cue>> {
    let head = text.trim_start_matches('\u{feff}');
    let cues = if head.contains("[Script Info]") || head.contains("Dialogue:") {
        parse_ass(head)
    } else if head.contains("-->") {
        parse_timed_blocks(head, "-->")
    } else {
        Vec::new()
    };
    if cues.is_empty() {
        return Err(anyhow!("nenhuma legenda reconhecida no arquivo"));
    }
    Ok(cues)
}

// ── Escrita ────────────────────────────────────────────────────────────

pub fn to_srt(cues: &[Cue]) -> String {
    let mut out = String::new();
    for (i, c) in cues.iter().enumerate() {
        out.push_str(&format!(
            "{}\n{} --> {}\n{}\n\n",
            i + 1,
            fmt_srt_time(c.start_ms),
            fmt_srt_time(c.end_ms),
            c.text
        ));
    }
    out
}

pub fn to_vtt(cues: &[Cue]) -> String {
    let mut out = String::from("WEBVTT\n\n");
    for c in cues {
        out.push_str(&format!(
            "{} --> {}\n{}\n\n",
            fmt_vtt_time(c.start_ms),
            fmt_vtt_time(c.end_ms),
            c.text
        ));
    }
    out
}

pub fn to_ass(cues: &[Cue]) -> String {
    let mut out = String::from(
        "[Script Info]\n\
ScriptType: v4.00+\n\
WrapStyle: 0\n\
ScaledBorderAndShadow: yes\n\
YCbCr Matrix: None\n\
PlayResX: 1920\n\
PlayResY: 1080\n\n\
[V4+ Styles]\n\
Format: Name, Fontname, Fontsize, PrimaryColour, SecondaryColour, OutlineColour, BackColour, Bold, Italic, Underline, StrikeOut, ScaleX, ScaleY, Spacing, Angle, BorderStyle, Outline, Shadow, Alignment, MarginL, MarginR, MarginV, Encoding\n\
Style: Default,Arial,54,&H00FFFFFF,&H000000FF,&H00000000,&H80000000,0,0,0,0,100,100,0,0,1,3,1,2,60,60,50,1\n\n\
[Events]\n\
Format: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\n",
    );
    for c in cues {
        out.push_str(&format!(
            "Dialogue: 0,{},{},Default,,0,0,0,,{}\n",
            fmt_ass_time(c.start_ms),
            fmt_ass_time(c.end_ms),
            c.text.replace('\n', "\\N")
        ));
    }
    out
}

pub fn render(cues: &[Cue], format: &str) -> String {
    match format {
        "vtt" => to_vtt(cues),
        "ass" => to_ass(cues),
        _ => to_srt(cues),
    }
}

// ── Sincronia ──────────────────────────────────────────────────────────

pub fn shift(cues: &mut [Cue], ms: i64) {
    for c in cues.iter_mut() {
        c.start_ms = (c.start_ms + ms).max(0);
        c.end_ms = (c.end_ms + ms).max(0);
    }
}

/// Estica ou encolhe a linha do tempo inteira — é o conserto de legenda
/// pega de um vídeo com outro frame rate (23,976 vs 25 é o caso clássico).
pub fn scale(cues: &mut [Cue], factor: f64) {
    for c in cues.iter_mut() {
        c.start_ms = ((c.start_ms as f64) * factor).round() as i64;
        c.end_ms = ((c.end_ms as f64) * factor).round() as i64;
    }
}

/// Sincronia por duas âncoras: o usuário diz onde a primeira e a última fala
/// caem de verdade no áudio, e o resto é interpolado. Resolve de uma vez o
/// caso "começa certo e vai atrasando".
pub fn anchor_resync(cues: &mut [Cue], from_a: i64, to_a: i64, from_b: i64, to_b: i64) -> f64 {
    let span = (from_b - from_a) as f64;
    if span.abs() < 1.0 {
        shift(cues, to_a - from_a);
        return 1.0;
    }
    let factor = (to_b - to_a) as f64 / span;
    for c in cues.iter_mut() {
        c.start_ms = to_a + (((c.start_ms - from_a) as f64) * factor).round() as i64;
        c.end_ms = to_a + (((c.end_ms - from_a) as f64) * factor).round() as i64;
        c.start_ms = c.start_ms.max(0);
        c.end_ms = c.end_ms.max(c.start_ms);
    }
    factor
}

pub const FPS_PAIRS: &[(&str, f64)] = &[
    ("23.976→25", 23.976 / 25.0),
    ("25→23.976", 25.0 / 23.976),
    ("24→25", 24.0 / 25.0),
    ("25→24", 25.0 / 24.0),
    ("29.97→30", 29.97 / 30.0),
    ("30→29.97", 30.0 / 29.97),
];

// ── Comandos ───────────────────────────────────────────────────────────

#[derive(Debug, Clone, Deserialize)]
pub struct ConvertOptions {
    pub inputs: Vec<String>,
    /// "srt" | "vtt" | "ass"
    #[serde(default = "default_format")]
    pub format: String,
    /// Deslocamento em milissegundos (pode ser negativo).
    #[serde(default)]
    pub shift_ms: i64,
    /// Fator de escala do tempo; 0 ou 1 não mexe.
    #[serde(default)]
    pub scale: f64,
    /// Âncoras em milissegundos: [de_a, para_a, de_b, para_b]. Vazio = não usa.
    #[serde(default)]
    pub anchors: Vec<i64>,
    #[serde(default)]
    pub output_dir: String,
    #[serde(default)]
    pub suffix: String,
}

fn default_format() -> String {
    "srt".into()
}

#[derive(Debug, Clone, Serialize)]
pub struct SubtitleItem {
    pub input: String,
    pub output: Option<String>,
    pub cues: usize,
    pub first_ms: i64,
    pub last_ms: i64,
    pub applied_factor: f64,
    pub ok: bool,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SubtitleResult {
    pub items: Vec<SubtitleItem>,
}

/// Lê um arquivo respeitando UTF-8 com BOM e caindo para Latin-1 quando o
/// arquivo não é UTF-8 (legenda antiga em português é cheia disso).
fn read_text(path: &Path) -> anyhow::Result<String> {
    let bytes = std::fs::read(path)?;
    Ok(match String::from_utf8(bytes.clone()) {
        Ok(s) => s,
        Err(_) => bytes.iter().map(|b| *b as char).collect(),
    })
}

pub fn convert(opts: &ConvertOptions, progress: &super::ProgressFn) -> SubtitleResult {
    let total = opts.inputs.len() as u64;
    let mut items = Vec::new();
    for (i, input) in opts.inputs.iter().enumerate() {
        super::report(
            progress,
            "subtitle",
            "progress",
            i as u64,
            Some(total),
            Some(input.clone()),
        );
        items.push(convert_one(opts, input));
    }
    super::report(progress, "subtitle", "done", total, Some(total), None);
    SubtitleResult { items }
}

fn convert_one(opts: &ConvertOptions, input: &str) -> SubtitleItem {
    let inp = Path::new(input);
    let fail = |e: String| SubtitleItem {
        input: input.to_string(),
        output: None,
        cues: 0,
        first_ms: 0,
        last_ms: 0,
        applied_factor: 1.0,
        ok: false,
        error: Some(e),
    };
    let text = match read_text(inp) {
        Ok(t) => t,
        Err(e) => return fail(e.to_string()),
    };
    let mut cues = match parse(&text) {
        Ok(c) => c,
        Err(e) => return fail(e.to_string()),
    };

    let mut factor = 1.0;
    if opts.anchors.len() == 4 {
        factor = anchor_resync(
            &mut cues,
            opts.anchors[0],
            opts.anchors[1],
            opts.anchors[2],
            opts.anchors[3],
        );
    } else {
        if opts.scale > 0.0 && (opts.scale - 1.0).abs() > f64::EPSILON {
            scale(&mut cues, opts.scale);
            factor = opts.scale;
        }
        if opts.shift_ms != 0 {
            shift(&mut cues, opts.shift_ms);
        }
    }
    cues.sort_by_key(|c| c.start_ms);

    let dir = if opts.output_dir.trim().is_empty() {
        inp.parent().map(|p| p.to_path_buf()).unwrap_or_default()
    } else {
        PathBuf::from(opts.output_dir.trim())
    };
    if let Err(e) = std::fs::create_dir_all(&dir) {
        return fail(e.to_string());
    }
    let stem = inp
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "legenda".into());
    let ext = match opts.format.as_str() {
        "vtt" => "vtt",
        "ass" => "ass",
        _ => "srt",
    };
    // Sem sufixo e mesmo formato, não sobrescreve o original em silêncio.
    let suffix = if opts.suffix.is_empty() {
        let same = inp
            .extension()
            .map(|e| e.to_string_lossy().to_lowercase() == ext)
            .unwrap_or(false);
        if same {
            "-sync"
        } else {
            ""
        }
    } else {
        opts.suffix.as_str()
    };
    let out = dir.join(format!("{}{}.{}", stem, suffix, ext));
    if let Err(e) = std::fs::write(&out, render(&cues, ext)) {
        return fail(e.to_string());
    }
    SubtitleItem {
        input: input.to_string(),
        output: Some(out.to_string_lossy().to_string()),
        cues: cues.len(),
        first_ms: cues.first().map(|c| c.start_ms).unwrap_or(0),
        last_ms: cues.last().map(|c| c.end_ms).unwrap_or(0),
        applied_factor: factor,
        ok: true,
        error: None,
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct BurnOptions {
    pub video: String,
    pub subtitle: String,
    /// Tamanho da fonte quando a legenda não é ASS (o ASS traz o estilo dele).
    #[serde(default = "default_font_size")]
    pub font_size: u32,
    #[serde(default)]
    pub output_dir: String,
    #[serde(default)]
    pub suffix: String,
}

fn default_font_size() -> u32 {
    24
}

/// O filtro `subtitles` recebe o caminho dentro de uma string do FFmpeg:
/// dois-pontos separam opções e a barra invertida do Windows é escape.
pub fn escape_filter_path(path: &str) -> String {
    path.replace('\\', "/").replace(':', "\\:")
}

pub async fn burn(opts: BurnOptions, progress: super::ProgressFn) -> anyhow::Result<String> {
    let ffmpeg = crate::core::dependencies::ensure_ffmpeg().await?;
    let video = Path::new(&opts.video);
    let sub = Path::new(&opts.subtitle);
    if !sub.exists() {
        return Err(anyhow!("legenda não encontrada"));
    }
    let dir = if opts.output_dir.trim().is_empty() {
        video.parent().map(|p| p.to_path_buf()).unwrap_or_default()
    } else {
        PathBuf::from(opts.output_dir.trim())
    };
    std::fs::create_dir_all(&dir)?;
    let stem = video
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "video".into());
    let suffix = if opts.suffix.is_empty() {
        "-legendado"
    } else {
        opts.suffix.as_str()
    };
    let output = dir.join(format!("{}{}.mp4", stem, suffix));

    let is_ass = sub
        .extension()
        .map(|e| {
            let e = e.to_string_lossy().to_lowercase();
            e == "ass" || e == "ssa"
        })
        .unwrap_or(false);
    let path = escape_filter_path(&sub.to_string_lossy());
    let filter = if is_ass {
        format!("ass='{}'", path)
    } else {
        format!(
            "subtitles='{}':force_style='FontSize={},Outline=1,Shadow=0'",
            path,
            opts.font_size.clamp(8, 96)
        )
    };

    super::report(&progress, "subtitle", "progress", 0, Some(1), None);
    let out = crate::core::process::command(&ffmpeg)
        .args(["-y", "-hide_banner", "-loglevel", "error", "-i"])
        .arg(video)
        .args(["-vf", &filter])
        .args([
            "-c:v",
            "libx264",
            "-preset",
            "veryfast",
            "-crf",
            "20",
            "-c:a",
            "copy",
            "-movflags",
            "+faststart",
        ])
        .arg(&output)
        .output()
        .await
        .map_err(|e| anyhow!("ffmpeg nao iniciou: {}", e))?;
    if !out.status.success() {
        return Err(anyhow!("{}", String::from_utf8_lossy(&out.stderr).trim()));
    }
    super::report(&progress, "subtitle", "done", 1, Some(1), None);
    Ok(output.to_string_lossy().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    const SRT: &str = "1\n00:00:01,000 --> 00:00:03,500\nPrimeira fala\n\n2\n00:00:05,000 --> 00:00:06,000\nSegunda\nem duas linhas\n\n";

    #[test]
    fn reads_srt_with_multiline_text() {
        let cues = parse(SRT).unwrap();
        assert_eq!(cues.len(), 2);
        assert_eq!(cues[0].start_ms, 1000);
        assert_eq!(cues[0].end_ms, 3500);
        assert_eq!(cues[1].text, "Segunda\nem duas linhas");
    }

    #[test]
    fn reads_vtt_with_cue_settings() {
        let vtt = "WEBVTT\n\n00:00:02.250 --> 00:00:04.000 line:90% align:middle\nOi\n";
        let cues = parse(vtt).unwrap();
        assert_eq!(cues.len(), 1);
        assert_eq!(cues[0].start_ms, 2250);
        assert_eq!(cues[0].end_ms, 4000);
    }

    #[test]
    fn reads_ass_and_drops_override_tags() {
        let ass = "[Script Info]\nScriptType: v4.00+\n\n[Events]\nFormat: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\nDialogue: 0,0:00:01.50,0:00:03.00,Default,,0,0,0,,{\\an8}Legenda\\Nem duas\n";
        let cues = parse(ass).unwrap();
        assert_eq!(cues.len(), 1);
        assert_eq!(cues[0].start_ms, 1500);
        assert_eq!(cues[0].text, "Legenda\nem duas");
    }

    #[test]
    fn round_trip_keeps_the_timing() {
        let cues = parse(SRT).unwrap();
        for format in ["srt", "vtt", "ass"] {
            let back = parse(&render(&cues, format)).unwrap();
            assert_eq!(back.len(), cues.len(), "{}", format);
            assert_eq!(back[0].start_ms, cues[0].start_ms, "{}", format);
            // O ASS só tem centésimos: 3500 ms continua 3500, mas 3505 vira 3500.
            assert_eq!(back[1].end_ms, cues[1].end_ms, "{}", format);
        }
    }

    #[test]
    fn shift_never_goes_negative() {
        let mut cues = parse(SRT).unwrap();
        shift(&mut cues, -5000);
        assert_eq!(cues[0].start_ms, 0);
        assert_eq!(cues[1].start_ms, 0);
    }

    #[test]
    fn fps_scale_matches_the_classic_pair() {
        let mut cues = vec![Cue {
            start_ms: 25_000,
            end_ms: 26_000,
            text: "x".into(),
        }];
        let (_, factor) = FPS_PAIRS[1];
        scale(&mut cues, factor);
        // 25 → 23,976: a legenda tem que ficar mais longa em tempo.
        assert!(cues[0].start_ms > 26_000, "{}", cues[0].start_ms);
        assert_eq!(cues[0].start_ms, 26_068);
    }

    #[test]
    fn anchors_fix_drift_at_both_ends() {
        let mut cues = vec![
            Cue {
                start_ms: 1_000,
                end_ms: 2_000,
                text: "a".into(),
            },
            Cue {
                start_ms: 50_000,
                end_ms: 51_000,
                text: "b".into(),
            },
            Cue {
                start_ms: 100_000,
                end_ms: 101_000,
                text: "c".into(),
            },
        ];
        // A primeira fala cai em 2 s e a última em 105 s.
        let factor = anchor_resync(&mut cues, 1_000, 2_000, 100_000, 105_000);
        assert!((factor - 1.0404).abs() < 1e-3, "fator: {}", factor);
        assert_eq!(cues[0].start_ms, 2_000);
        assert_eq!(cues[2].start_ms, 105_000);
        // O do meio interpolado, sem inversão de ordem.
        assert!(cues[1].start_ms > cues[0].start_ms && cues[1].start_ms < cues[2].start_ms);
    }

    #[test]
    fn anchors_with_no_span_fall_back_to_a_shift() {
        let mut cues = vec![Cue {
            start_ms: 1_000,
            end_ms: 2_000,
            text: "a".into(),
        }];
        let factor = anchor_resync(&mut cues, 1_000, 3_000, 1_000, 9_999);
        assert_eq!(factor, 1.0);
        assert_eq!(cues[0].start_ms, 3_000);
        assert_eq!(cues[0].end_ms, 4_000);
    }

    #[test]
    fn garbage_is_refused() {
        assert!(parse("isso aqui não é legenda nenhuma").is_err());
        assert!(parse("").is_err());
    }

    #[test]
    fn windows_paths_are_escaped_for_the_filter() {
        let e = escape_filter_path(r"C:\Users\tonho\aula.srt");
        assert_eq!(e, "C\\:/Users/tonho/aula.srt");
        assert!(!e.contains('\\') || e.contains("\\:"));
    }

    #[test]
    fn time_parser_accepts_every_shape() {
        assert_eq!(parse_time("00:00:01,500"), Some(1500));
        assert_eq!(parse_time("0:00:01.50"), Some(1500));
        assert_eq!(parse_time("01:02.500"), Some(62_500));
        assert_eq!(parse_time("2.5"), Some(2500));
        assert_eq!(parse_time("não"), None);
    }
}

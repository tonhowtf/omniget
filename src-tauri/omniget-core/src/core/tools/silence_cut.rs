//! Cortar silêncio (dead air) de gravação de aula, podcast e vídeo de tela.
//!
//! `silenceremove` do FFmpeg é filtro de áudio: usar direto num vídeo
//! dessincroniza a imagem. Aqui o silêncio é só **detectado** (`silencedetect`),
//! invertido em trechos de fala e cortado com `trim`/`atrim` + `concat`, que
//! corta as duas trilhas na mesma marca. Sobra um `padding` de cada lado para
//! não decepar o ataque da palavra.

use std::path::{Path, PathBuf};

use anyhow::anyhow;
use serde::{Deserialize, Serialize};

/// Filtro grande demais trava o FFmpeg (e a linha de comando do Windows).
/// Acima disso os silêncios mais curtos são devolvidos ao vídeo.
const MAX_SEGMENTS: usize = 150;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub struct Span {
    pub start: f64,
    pub end: f64,
}

impl Span {
    pub fn duration(&self) -> f64 {
        (self.end - self.start).max(0.0)
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct SilenceOptions {
    pub inputs: Vec<String>,
    /// Limiar em dB abaixo do qual é silêncio (-30 é conversa normal).
    #[serde(default = "default_threshold")]
    pub threshold_db: f64,
    /// Silêncio mais curto que isso não vale corte, em segundos.
    #[serde(default = "default_min_silence")]
    pub min_silence: f64,
    /// Quanto de silêncio fica de cada lado da fala, em segundos.
    #[serde(default = "default_padding")]
    pub padding: f64,
    /// Só medir e relatar, sem escrever arquivo novo.
    #[serde(default)]
    pub dry_run: bool,
    #[serde(default)]
    pub output_dir: String,
    #[serde(default)]
    pub suffix: String,
}

fn default_threshold() -> f64 {
    -30.0
}
fn default_min_silence() -> f64 {
    0.6
}
fn default_padding() -> f64 {
    0.1
}

#[derive(Debug, Clone, Serialize)]
pub struct SilenceItem {
    pub input: String,
    pub output: Option<String>,
    pub duration_before: f64,
    pub duration_after: f64,
    pub seconds_removed: f64,
    pub cuts: usize,
    /// Silêncios que ficaram no vídeo por causa do limite de segmentos.
    pub skipped_short: usize,
    pub ok: bool,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SilenceResult {
    pub items: Vec<SilenceItem>,
}

/// Lê `silence_start` / `silence_end` do stderr do FFmpeg.
///
/// Um `silence_start` sem par significa que o arquivo termina em silêncio: com
/// a duração total em mãos, o par fecha no fim do arquivo.
pub fn parse_spans(stderr: &str, total: f64) -> Vec<Span> {
    let mut spans = Vec::new();
    let mut open: Option<f64> = None;
    for line in stderr.lines() {
        if let Some(v) = field_after(line, "silence_start:") {
            open = Some(v);
        } else if let Some(end) = field_after(line, "silence_end:") {
            if let Some(start) = open.take() {
                if end > start {
                    spans.push(Span { start, end });
                }
            }
        }
    }
    if let Some(start) = open {
        if total > start {
            spans.push(Span { start, end: total });
        }
    }
    spans
}

fn field_after(line: &str, key: &str) -> Option<f64> {
    let idx = line.find(key)?;
    line[idx + key.len()..]
        .split_whitespace()
        .next()?
        .parse()
        .ok()
}

/// Devolve os silêncios em ordem de duração, cortando os mais curtos até
/// caber no limite de segmentos. O segundo valor é quantos ficaram de fora.
pub fn cap_spans(mut spans: Vec<Span>, max_segments: usize) -> (Vec<Span>, usize) {
    // Cada silêncio no meio gera no máximo um segmento a mais de fala.
    if spans.len() < max_segments {
        return (spans, 0);
    }
    spans.sort_by(|a, b| b.duration().total_cmp(&a.duration()));
    let keep = max_segments.saturating_sub(1);
    let dropped = spans.len() - keep;
    spans.truncate(keep);
    spans.sort_by(|a, b| a.start.total_cmp(&b.start));
    (spans, dropped)
}

/// Inverte os silêncios em trechos de fala, devolvendo o padding às pontas.
pub fn keep_ranges(spans: &[Span], total: f64, padding: f64) -> Vec<Span> {
    let mut keeps: Vec<Span> = Vec::new();
    let mut cursor = 0.0f64;
    for s in spans {
        let cut_start = (s.start + padding).min(total);
        let cut_end = (s.end - padding).max(cut_start);
        // Só vira trecho se houver fala entre o cursor e este silêncio; um
        // silêncio que começa no cursor (arquivo que abre em silêncio, ou dois
        // silêncios colados) só renderia o próprio padding.
        if s.start > cursor {
            keeps.push(Span {
                start: cursor,
                end: cut_start,
            });
        }
        cursor = cursor.max(cut_end);
    }
    if cursor < total {
        keeps.push(Span {
            start: cursor,
            end: total,
        });
    }
    // Trecho microscópico só engorda o filtro sem virar som audível.
    keeps.retain(|k| k.duration() >= 0.05);
    keeps
}

/// Monta o `filter_complex` que corta e emenda as duas trilhas.
pub fn build_filter(keeps: &[Span], has_audio: bool) -> String {
    let mut parts = Vec::new();
    for (i, k) in keeps.iter().enumerate() {
        parts.push(format!(
            "[0:v]trim=start={:.3}:end={:.3},setpts=PTS-STARTPTS[v{}]",
            k.start, k.end, i
        ));
        if has_audio {
            parts.push(format!(
                "[0:a]atrim=start={:.3}:end={:.3},asetpts=PTS-STARTPTS[a{}]",
                k.start, k.end, i
            ));
        }
    }
    let labels: String = (0..keeps.len())
        .map(|i| {
            if has_audio {
                format!("[v{}][a{}]", i, i)
            } else {
                format!("[v{}]", i)
            }
        })
        .collect();
    parts.push(format!(
        "{}concat=n={}:v=1:a={}[outv]{}",
        labels,
        keeps.len(),
        if has_audio { 1 } else { 0 },
        if has_audio { "[outa]" } else { "" }
    ));
    parts.join(";")
}

async fn detect(
    ffmpeg: &Path,
    input: &Path,
    threshold_db: f64,
    min_silence: f64,
) -> anyhow::Result<String> {
    let out = crate::core::process::command(ffmpeg)
        .args(["-hide_banner", "-nostats", "-i"])
        .arg(input)
        .args([
            "-af",
            &format!(
                "silencedetect=noise={}dB:d={}",
                threshold_db,
                min_silence.max(0.05)
            ),
            "-f",
            "null",
            "-",
        ])
        .output()
        .await
        .map_err(|e| anyhow!("ffmpeg nao iniciou: {}", e))?;
    Ok(String::from_utf8_lossy(&out.stderr).to_string())
}

async fn cut_one(ffmpeg: &Path, opts: &SilenceOptions, input: &str) -> anyhow::Result<SilenceItem> {
    let inp = Path::new(input);
    let probe = crate::core::ffmpeg::probe(inp).await?;
    let total = probe.duration_seconds;
    if total <= 0.0 {
        return Err(anyhow!("não consegui medir a duração"));
    }
    let has_audio = probe.streams.iter().any(|s| s.codec_type == "audio");
    if !has_audio {
        return Err(anyhow!("o arquivo não tem trilha de áudio"));
    }

    let stderr = detect(ffmpeg, inp, opts.threshold_db, opts.min_silence).await?;
    let spans = parse_spans(&stderr, total);
    let (spans, skipped) = cap_spans(spans, MAX_SEGMENTS);
    let keeps = keep_ranges(&spans, total, opts.padding.max(0.0));
    let kept: f64 = keeps.iter().map(|k| k.duration()).sum();

    let mut item = SilenceItem {
        input: input.to_string(),
        output: None,
        duration_before: total,
        duration_after: kept,
        seconds_removed: (total - kept).max(0.0),
        cuts: spans.len(),
        skipped_short: skipped,
        ok: true,
        error: None,
    };
    if opts.dry_run || spans.is_empty() || keeps.is_empty() {
        item.duration_after = if spans.is_empty() { total } else { kept };
        return Ok(item);
    }

    let out_dir = if opts.output_dir.trim().is_empty() {
        inp.parent().map(|p| p.to_path_buf()).unwrap_or_default()
    } else {
        PathBuf::from(opts.output_dir.trim())
    };
    std::fs::create_dir_all(&out_dir)?;
    let stem = inp
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "gravacao".into());
    let suffix = if opts.suffix.is_empty() {
        "-sem-silencio"
    } else {
        opts.suffix.as_str()
    };
    let output = out_dir.join(format!("{}{}.mp4", stem, suffix));

    let filter = build_filter(&keeps, true);
    let out = crate::core::process::command(ffmpeg)
        .args(["-y", "-hide_banner", "-loglevel", "error", "-i"])
        .arg(inp)
        .args(["-filter_complex", &filter])
        .args([
            "-map",
            "[outv]",
            "-map",
            "[outa]",
            "-c:v",
            "libx264",
            "-preset",
            "veryfast",
            "-crf",
            "20",
            "-c:a",
            "aac",
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
    item.output = Some(output.to_string_lossy().to_string());
    Ok(item)
}

pub async fn run(
    opts: SilenceOptions,
    progress: super::ProgressFn,
) -> anyhow::Result<SilenceResult> {
    let ffmpeg = crate::core::dependencies::ensure_ffmpeg().await?;
    let total = opts.inputs.len() as u64;
    let mut items = Vec::new();
    for (i, input) in opts.inputs.iter().enumerate() {
        super::report(
            &progress,
            "video-silence",
            "progress",
            i as u64,
            Some(total),
            Some(input.clone()),
        );
        match cut_one(&ffmpeg, &opts, input).await {
            Ok(item) => items.push(item),
            Err(e) => {
                tracing::warn!("[video-silence] {}: {}", input, e);
                items.push(SilenceItem {
                    input: input.clone(),
                    output: None,
                    duration_before: 0.0,
                    duration_after: 0.0,
                    seconds_removed: 0.0,
                    cuts: 0,
                    skipped_short: 0,
                    ok: false,
                    error: Some(e.to_string()),
                });
            }
        }
    }
    super::report(&progress, "video-silence", "done", total, Some(total), None);
    Ok(SilenceResult { items })
}

#[cfg(test)]
mod tests {
    use super::*;

    const LOG: &str = "\
[silencedetect @ 0x1] silence_start: 2.5
[silencedetect @ 0x1] silence_end: 5.0 | silence_duration: 2.5
[silencedetect @ 0x1] silence_start: 8.0
[silencedetect @ 0x1] silence_end: 9.2 | silence_duration: 1.2
";

    #[test]
    fn reads_pairs_from_the_log() {
        let spans = parse_spans(LOG, 12.0);
        assert_eq!(spans.len(), 2);
        assert_eq!(
            spans[0],
            Span {
                start: 2.5,
                end: 5.0
            }
        );
        assert_eq!(
            spans[1],
            Span {
                start: 8.0,
                end: 9.2
            }
        );
    }

    #[test]
    fn a_file_ending_in_silence_closes_the_pair() {
        let log = "silence_start: 30.0\n";
        let spans = parse_spans(log, 42.0);
        assert_eq!(
            spans,
            vec![Span {
                start: 30.0,
                end: 42.0
            }]
        );
        // Sem saber a duração não dá para fechar o par.
        assert!(parse_spans(log, 0.0).is_empty());
    }

    #[test]
    fn keeps_are_the_inverse_plus_padding() {
        let spans = parse_spans(LOG, 12.0);
        let keeps = keep_ranges(&spans, 12.0, 0.1);
        assert_eq!(keeps.len(), 3);
        assert!((keeps[0].end - 2.6).abs() < 1e-9, "padding do começo");
        assert!((keeps[1].start - 4.9).abs() < 1e-9, "padding do fim");
        assert!((keeps[2].end - 12.0).abs() < 1e-9);
        let kept: f64 = keeps.iter().map(|k| k.duration()).sum();
        assert!(kept < 12.0 && kept > 8.0, "sobrou {}", kept);
    }

    #[test]
    fn no_silence_keeps_the_whole_file() {
        let keeps = keep_ranges(&[], 10.0, 0.1);
        assert_eq!(
            keeps,
            vec![Span {
                start: 0.0,
                end: 10.0
            }]
        );
    }

    #[test]
    fn silence_at_the_very_start_does_not_make_an_empty_segment() {
        let spans = vec![Span {
            start: 0.0,
            end: 4.0,
        }];
        let keeps = keep_ranges(&spans, 10.0, 0.1);
        assert_eq!(keeps.len(), 1);
        assert!((keeps[0].start - 3.9).abs() < 1e-9);
    }

    #[test]
    fn cap_drops_the_shortest_silences_first() {
        let spans: Vec<Span> = (0..10)
            .map(|i| Span {
                start: i as f64 * 10.0,
                end: i as f64 * 10.0 + (i as f64 + 1.0),
            })
            .collect();
        let (kept, dropped) = cap_spans(spans, 4);
        assert_eq!(dropped, 7);
        assert_eq!(kept.len(), 3);
        // Sobraram os três mais longos, de volta em ordem de tempo.
        assert!(kept[0].start < kept[1].start && kept[1].start < kept[2].start);
        assert!(kept.iter().all(|s| s.duration() >= 8.0));
    }

    #[test]
    fn cap_is_a_no_op_when_it_already_fits() {
        let spans = vec![Span {
            start: 1.0,
            end: 2.0,
        }];
        let (kept, dropped) = cap_spans(spans.clone(), 150);
        assert_eq!(kept, spans);
        assert_eq!(dropped, 0);
    }

    #[test]
    fn filter_trims_both_tracks_and_concats() {
        let keeps = vec![
            Span {
                start: 0.0,
                end: 2.5,
            },
            Span {
                start: 5.0,
                end: 8.0,
            },
        ];
        let f = build_filter(&keeps, true);
        assert!(f.contains("[0:v]trim=start=0.000:end=2.500"));
        assert!(f.contains("[0:a]atrim=start=5.000:end=8.000"));
        assert!(f.contains("[v0][a0][v1][a1]concat=n=2:v=1:a=1[outv][outa]"));
    }

    /// Gera um clipe com um buraco de silêncio conhecido (3 s a 6 s) e
    /// confere que o corte acha e remove exatamente aquilo.
    /// `cargo test -p omniget-core --lib -- --ignored live_silence`
    #[tokio::test]
    #[ignore]
    async fn live_silence_cut_removes_the_known_gap() {
        let ffmpeg = crate::core::dependencies::ensure_ffmpeg().await.unwrap();
        let dir = std::env::temp_dir().join("omniget-silence-live");
        std::fs::create_dir_all(&dir).unwrap();
        let src = dir.join("fonte.mp4");
        let gen = crate::core::process::command(&ffmpeg)
            .args([
                "-y",
                "-hide_banner",
                "-loglevel",
                "error",
                "-f",
                "lavfi",
                "-i",
                "testsrc2=size=320x240:rate=25:duration=10",
                "-f",
                "lavfi",
                "-i",
                "sine=frequency=440:duration=10,volume=enable='between(t,3,6)':volume=0",
                "-c:v",
                "libx264",
                "-preset",
                "ultrafast",
                "-pix_fmt",
                "yuv420p",
                "-c:a",
                "aac",
                "-shortest",
            ])
            .arg(&src)
            .output()
            .await
            .unwrap();
        assert!(gen.status.success(), "não gerei o clipe de teste");

        let opts = SilenceOptions {
            inputs: vec![src.to_string_lossy().to_string()],
            threshold_db: -30.0,
            min_silence: 0.5,
            padding: 0.1,
            dry_run: false,
            output_dir: dir.to_string_lossy().to_string(),
            suffix: String::new(),
        };
        let res = run(opts, crate::core::tools::noop_progress())
            .await
            .unwrap();
        let item = &res.items[0];
        assert!(item.ok, "falhou: {:?}", item.error);
        assert_eq!(item.cuts, 1, "era para achar um silêncio só");
        assert!(
            (item.seconds_removed - 2.8).abs() < 0.4,
            "removeu {} s, esperado ~2,8",
            item.seconds_removed
        );

        let out = item.output.as_ref().unwrap();
        let probe = crate::core::ffmpeg::probe(std::path::Path::new(out))
            .await
            .unwrap();
        assert!(
            (probe.duration_seconds - item.duration_after).abs() < 0.5,
            "duração do arquivo ({}) não bate com o relatado ({})",
            probe.duration_seconds,
            item.duration_after
        );
        assert!(
            probe.streams.iter().any(|s| s.codec_type == "audio"),
            "perdeu o áudio"
        );
        assert!(
            probe.streams.iter().any(|s| s.codec_type == "video"),
            "perdeu o vídeo"
        );
        eprintln!(
            "{:.2}s → {:.2}s ({} corte, {:.2}s fora)",
            item.duration_before, probe.duration_seconds, item.cuts, item.seconds_removed
        );
    }

    #[test]
    fn filter_without_audio_has_no_audio_labels() {
        let f = build_filter(
            &[Span {
                start: 0.0,
                end: 1.0,
            }],
            false,
        );
        assert!(!f.contains("atrim"));
        assert!(f.ends_with("concat=n=1:v=1:a=0[outv]"));
    }
}

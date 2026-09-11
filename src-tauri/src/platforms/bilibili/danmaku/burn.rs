//! Queimar o ASS de danmaku num arquivo de vídeo local, com corte opcional.
//!
//! Cortar e queimar ao mesmo tempo é onde o FFmpeg sai de sincronia: com `-ss`
//! antes de `-i` os quadros chegam ao filtro já rebobinados para zero, e o
//! libass desenha em cima do clipe os comentários do começo do vídeo. Em vez
//! de brigar com `-copyts` (que resolve o desenho e desarruma a saída), o
//! corte é feito na legenda: uma cópia temporária do ASS já sai deslocada e
//! recortada para a janela pedida, e aí `-ss`/`-t` viram um corte comum.
//!
//! A cópia temporária também tira o caminho original do caminho: nome ASCII
//! curto em pasta nossa, sem apóstrofo nem acento para o parser de filtro do
//! FFmpeg tropeçar.

use std::path::{Path, PathBuf};

use anyhow::{anyhow, Result};
use omniget_core::core::tools::{report, subtitle, video_compress, ProgressFn};
use serde::{Deserialize, Serialize};

/// Id da tool no evento `tool-progress`.
pub const ID: &str = "bili-danmaku-burn";

#[derive(Debug, Clone, Deserialize)]
pub struct BurnOptions {
    pub video: String,
    /// O `.ass` do danmaku (aceita `.ssa`, `.srt` e `.vtt` também).
    pub subtitle: String,
    /// Início do recorte em segundos; 0 pega do começo.
    #[serde(default)]
    pub start: f64,
    /// Duração do recorte em segundos; 0 vai até o fim.
    #[serde(default)]
    pub duration: f64,
    /// Alvo de tamanho em MB; 0 codifica por qualidade fixa (CRF).
    #[serde(default)]
    pub target_mb: f64,
    #[serde(default = "default_crf")]
    pub crf: u32,
    #[serde(default = "default_preset")]
    pub preset: String,
    /// Altura máxima da saída; 0 mantém a original.
    #[serde(default)]
    pub max_height: u32,
    /// Pasta com as fontes do ASS (o danmaku pede uma CJK que raramente está
    /// instalada). Vazia usa as fontes do sistema.
    #[serde(default)]
    pub fonts_dir: String,
    #[serde(default)]
    pub output_dir: String,
    #[serde(default)]
    pub suffix: String,
}

fn default_crf() -> u32 {
    20
}

fn default_preset() -> String {
    "veryfast".into()
}

#[derive(Debug, Clone, Serialize)]
pub struct BurnResult {
    pub output: String,
    pub bytes: u64,
    pub duration_secs: f64,
    /// Quantos comentários sobraram dentro da janela.
    pub dialogues: usize,
    /// 0 quando a codificação foi por CRF.
    pub video_kbps: u32,
    pub audio_kbps: u32,
}

pub async fn run(opts: &BurnOptions, progress: &ProgressFn) -> Result<BurnResult> {
    let video = PathBuf::from(opts.video.trim());
    if !video.is_file() {
        return Err(anyhow!("vídeo não encontrado: {}", opts.video));
    }
    let sub = PathBuf::from(opts.subtitle.trim());
    if !sub.is_file() {
        return Err(anyhow!("legenda não encontrada: {}", opts.subtitle));
    }
    let ffmpeg = omniget_core::core::dependencies::ensure_ffmpeg().await?;

    report(
        progress,
        ID,
        "progress",
        0,
        Some(3),
        Some("lendo o vídeo".into()),
    );
    let probe = omniget_core::core::ffmpeg::probe(&video).await?;
    let has_audio = probe.streams.iter().any(|s| s.codec_type == "audio");
    let start = opts.start.max(0.0);
    let window = if opts.duration > 0.0 {
        opts.duration
    } else {
        (probe.duration_seconds - start).max(0.0)
    };
    if window <= 0.0 {
        return Err(anyhow!(
            "a janela de corte ficou vazia — o vídeo tem {:.1}s",
            probe.duration_seconds
        ));
    }

    let text = read_text(&sub)?;
    let (body, dialogues) = if is_ass(&sub) {
        shift_ass(&text, start, window)
    } else {
        shift_other(&text, start, window)?
    };
    if dialogues == 0 {
        return Err(anyhow!(
            "nenhum comentário cai entre {:.1}s e {:.1}s",
            start,
            start + window
        ));
    }
    let work_dir = omniget_core::core::tools::temp_dir();
    std::fs::create_dir_all(&work_dir)?;
    let work = work_dir.join(format!("danmaku-{}.ass", uuid::Uuid::new_v4()));
    std::fs::write(&work, &body)?;

    let out_dir = if opts.output_dir.trim().is_empty() {
        video.parent().map(|p| p.to_path_buf()).unwrap_or_default()
    } else {
        PathBuf::from(opts.output_dir.trim())
    };
    std::fs::create_dir_all(&out_dir)?;
    let stem = video
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "clipe".into());
    let suffix = if opts.suffix.trim().is_empty() {
        "-danmaku"
    } else {
        opts.suffix.trim()
    };
    let output = out_dir.join(format!("{}{}.mp4", stem, suffix));

    let filters = filter_chain(&work, opts.fonts_dir.trim(), opts.max_height);
    let cut = cut_args(start, window);
    let audio = audio_args(has_audio, 0);

    let result = if opts.target_mb > 0.0 {
        let target_bytes = (opts.target_mb * 1024.0 * 1024.0) as u64;
        let (video_kbps, audio_kbps) =
            video_compress::plan_bitrates(window, target_bytes, 128, has_audio, 0.97).ok_or_else(
                || {
                    anyhow!(
                        "{:.0} MB não cabem {:.0}s de vídeo — aumente o alvo ou encurte o clipe",
                        opts.target_mb,
                        window
                    )
                },
            )?;
        let log_dir = work_dir.join(format!("danmaku-2pass-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&log_dir)?;
        let log = log_dir.join("pass");
        let common = bitrate_args(video_kbps, &opts.preset);

        report(
            progress,
            ID,
            "progress",
            1,
            Some(3),
            Some("passagem 1 de 2".into()),
        );
        run_ffmpeg(
            &ffmpeg,
            &video,
            &cut,
            &filters,
            &common,
            Some((1, &log)),
            &["-an".into(), "-f".into(), "mp4".into(), null_sink().into()],
        )
        .await?;

        report(
            progress,
            ID,
            "progress",
            2,
            Some(3),
            Some("passagem 2 de 2".into()),
        );
        let mut tail = audio_args(has_audio, audio_kbps);
        tail.extend(["-movflags".into(), "+faststart".into()]);
        tail.push(output.to_string_lossy().to_string());
        run_ffmpeg(
            &ffmpeg,
            &video,
            &cut,
            &filters,
            &common,
            Some((2, &log)),
            &tail,
        )
        .await?;
        let _ = std::fs::remove_dir_all(&log_dir);
        (video_kbps, if has_audio { audio_kbps } else { 0 })
    } else {
        report(
            progress,
            ID,
            "progress",
            1,
            Some(3),
            Some("queimando".into()),
        );
        let common = crf_args(opts.crf, &opts.preset);
        let mut tail = audio;
        tail.extend(["-movflags".into(), "+faststart".into()]);
        tail.push(output.to_string_lossy().to_string());
        run_ffmpeg(&ffmpeg, &video, &cut, &filters, &common, None, &tail).await?;
        (0, 0)
    };

    let _ = std::fs::remove_file(&work);
    let bytes = std::fs::metadata(&output).map(|m| m.len()).unwrap_or(0);
    if bytes == 0 {
        return Err(anyhow!("o FFmpeg terminou sem escrever nada"));
    }
    report(progress, ID, "done", 3, Some(3), None);
    Ok(BurnResult {
        output: output.to_string_lossy().to_string(),
        bytes,
        duration_secs: window,
        dialogues,
        video_kbps: result.0,
        audio_kbps: result.1,
    })
}

// ── Montagem do comando ────────────────────────────────────────────────

fn null_sink() -> &'static str {
    if cfg!(windows) {
        "NUL"
    } else {
        "/dev/null"
    }
}

/// `-ss`/`-t` antes do `-i`: o FFmpeg busca no contêiner em vez de decodificar
/// o vídeo inteiro, e o ASS já vem deslocado, então a conta bate.
pub fn cut_args(start: f64, window: f64) -> Vec<String> {
    let mut a = Vec::new();
    if start > 0.0 {
        a.push("-ss".into());
        a.push(format!("{:.3}", start));
    }
    if window > 0.0 {
        a.push("-t".into());
        a.push(format!("{:.3}", window));
    }
    a
}

/// Escala antes do `ass`: o libass desenha na resolução de saída e o texto sai
/// nítido, em vez de ser desenhado grande e depois reduzido junto com o vídeo.
pub fn filter_chain(subtitle: &Path, fonts_dir: &str, max_height: u32) -> String {
    let mut chain = String::new();
    if max_height > 0 {
        chain.push_str(&format!(
            "scale=-2:'min({h},ih)':flags=lanczos,",
            h = max_height - max_height % 2
        ));
    }
    chain.push_str(&format!(
        "ass='{}'",
        subtitle::escape_filter_path(&subtitle.to_string_lossy())
    ));
    if !fonts_dir.is_empty() {
        chain.push_str(&format!(
            ":fontsdir='{}'",
            subtitle::escape_filter_path(fonts_dir)
        ));
    }
    chain
}

fn crf_args(crf: u32, preset: &str) -> Vec<String> {
    vec![
        "-c:v".into(),
        "libx264".into(),
        "-preset".into(),
        sane_preset(preset),
        "-crf".into(),
        crf.clamp(0, 51).to_string(),
        "-pix_fmt".into(),
        "yuv420p".into(),
    ]
}

fn bitrate_args(video_kbps: u32, preset: &str) -> Vec<String> {
    vec![
        "-c:v".into(),
        "libx264".into(),
        "-preset".into(),
        sane_preset(preset),
        "-b:v".into(),
        format!("{}k", video_kbps),
        "-pix_fmt".into(),
        "yuv420p".into(),
    ]
}

fn sane_preset(preset: &str) -> String {
    const KNOWN: [&str; 9] = [
        "ultrafast",
        "superfast",
        "veryfast",
        "faster",
        "fast",
        "medium",
        "slow",
        "slower",
        "veryslow",
    ];
    let p = preset.trim();
    if KNOWN.contains(&p) {
        p.to_string()
    } else {
        "veryfast".to_string()
    }
}

/// Recodifica o áudio em vez de copiar: cópia de trecho começa num pacote que
/// não é o primeiro quadro e o clipe sai com o som adiantado.
fn audio_args(has_audio: bool, kbps: u32) -> Vec<String> {
    if !has_audio {
        return vec!["-an".into()];
    }
    vec![
        "-c:a".into(),
        "aac".into(),
        "-b:a".into(),
        format!("{}k", if kbps > 0 { kbps } else { 128 }),
    ]
}

async fn run_ffmpeg(
    ffmpeg: &Path,
    input: &Path,
    cut: &[String],
    filters: &str,
    common: &[String],
    pass: Option<(u8, &Path)>,
    tail: &[String],
) -> Result<()> {
    let mut cmd = omniget_core::core::process::command(ffmpeg);
    cmd.args(["-y", "-hide_banner", "-loglevel", "error"])
        .args(cut)
        .arg("-i")
        .arg(input)
        .args(["-vf", filters])
        .args(common);
    if let Some((n, log)) = pass {
        cmd.args(["-pass", &n.to_string()])
            .arg("-passlogfile")
            .arg(log);
    }
    let out = cmd
        .args(tail)
        .output()
        .await
        .map_err(|e| anyhow!("ffmpeg nao iniciou: {}", e))?;
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        return Err(anyhow!("{}", friendly_ffmpeg(stderr.trim())));
    }
    Ok(())
}

/// O erro cru do libass não diz nada para quem não conhece FFmpeg.
fn friendly_ffmpeg(stderr: &str) -> String {
    if stderr.contains("fontselect") || stderr.contains("Glyph") {
        return format!(
            "a fonte do ASS não está instalada — aponte a pasta de fontes. ({})",
            stderr
        );
    }
    if stderr.is_empty() {
        return "o ffmpeg falhou sem dizer por quê".to_string();
    }
    stderr.to_string()
}

// ── Recorte da legenda ─────────────────────────────────────────────────

fn is_ass(path: &Path) -> bool {
    path.extension()
        .map(|e| {
            let e = e.to_string_lossy().to_lowercase();
            e == "ass" || e == "ssa"
        })
        .unwrap_or(false)
}

/// Legenda antiga em português vem em Latin-1 mais vezes do que se admite.
fn read_text(path: &Path) -> Result<String> {
    let bytes = std::fs::read(path)?;
    Ok(match String::from_utf8(bytes.clone()) {
        Ok(s) => s,
        Err(_) => bytes.iter().map(|b| *b as char).collect(),
    })
}

pub fn parse_ass_time(s: &str) -> Option<f64> {
    let mut parts = s.trim().split(':');
    let h: f64 = parts.next()?.trim().parse().ok()?;
    let m: f64 = parts.next()?.trim().parse().ok()?;
    let sec: f64 = parts.next()?.trim().parse().ok()?;
    if parts.next().is_some() {
        return None;
    }
    Some(h * 3600.0 + m * 60.0 + sec)
}

/// Desloca e recorta um ASS mexendo só nas colunas de tempo. As tags de
/// posição do danmaku (`\move`, `\pos`) e as vírgulas dentro delas têm que
/// sair intactas, então o texto é o resto da linha e não é tocado.
///
/// Devolve o arquivo novo e quantos diálogos sobraram.
pub fn shift_ass(text: &str, start_secs: f64, window_secs: f64) -> (String, usize) {
    let mut out = String::with_capacity(text.len());
    let mut kept = 0usize;
    for line in text.trim_start_matches('\u{feff}').lines() {
        let trimmed = line.trim_start();
        if !trimmed.starts_with("Dialogue:") && !trimmed.starts_with("Comment:") {
            out.push_str(line);
            out.push('\n');
            continue;
        }
        if let Some(shifted) = shift_event(trimmed, start_secs, window_secs) {
            out.push_str(&shifted);
            out.push('\n');
            kept += 1;
        }
    }
    (out, kept)
}

fn shift_event(line: &str, start_secs: f64, window_secs: f64) -> Option<String> {
    let (head, rest) = line.split_once(':')?;
    // Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text.
    let fields: Vec<&str> = rest.splitn(10, ',').collect();
    if fields.len() < 10 {
        return None;
    }
    let s = parse_ass_time(fields[1])? - start_secs;
    let e = parse_ass_time(fields[2])? - start_secs;
    if e <= 0.0 || s >= window_secs {
        return None;
    }
    Some(format!(
        "{}:{},{},{},{}",
        head,
        fields[0],
        super::ass::ass_time(s.max(0.0)),
        super::ass::ass_time(e.min(window_secs)),
        fields[3..].join(",")
    ))
}

/// SRT/VTT: passa pelo parser da seção Tools, corta a janela e volta em ASS.
fn shift_other(text: &str, start_secs: f64, window_secs: f64) -> Result<(String, usize)> {
    let mut cues = subtitle::parse(text)?;
    let offset = (start_secs * 1000.0).round() as i64;
    let limit = (window_secs * 1000.0).round() as i64;
    for c in cues.iter_mut() {
        c.start_ms -= offset;
        c.end_ms -= offset;
    }
    cues.retain(|c| c.end_ms > 0 && c.start_ms < limit);
    for c in cues.iter_mut() {
        c.start_ms = c.start_ms.max(0);
        c.end_ms = c.end_ms.min(limit);
    }
    let n = cues.len();
    Ok((subtitle::to_ass(&cues), n))
}

#[cfg(test)]
mod tests {
    use super::*;

    const ASS: &str = "[Script Info]\nScriptType: v4.00+\nPlayResX: 1920\n\n[Events]\nFormat: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\nDialogue: 0,0:00:01.00,0:00:13.00,Danmaku,,0,0,0,,{\\move(1920,50,-200,50)}comeco\nDialogue: 0,0:00:10.00,0:00:22.00,Danmaku,,0,0,0,,{\\move(1920,100,-200,100)}meio, com virgula\nDialogue: 0,0:01:00.00,0:01:12.00,Danmaku,,0,0,0,,{\\an8\\pos(960,50)}fim\n";

    #[test]
    fn time_parsing_round_trips() {
        assert_eq!(parse_ass_time("0:00:00.00"), Some(0.0));
        assert_eq!(parse_ass_time("0:01:05.50"), Some(65.5));
        assert_eq!(parse_ass_time("1:01:01.25"), Some(3661.25));
        assert_eq!(parse_ass_time("00:01:05,500"), None);
        assert_eq!(parse_ass_time("bagunca"), None);
    }

    #[test]
    fn shift_keeps_the_header_and_the_position_tags() {
        let (out, kept) = shift_ass(ASS, 8.0, 20.0);
        assert_eq!(kept, 2, "{}", out);
        assert!(out.contains("[Script Info]"));
        assert!(out.contains("Format: Layer, Start, End"));
        // A tag de movimento e a vírgula do texto não podem ser tocadas.
        assert!(out.contains("{\\move(1920,100,-200,100)}meio, com virgula"));
        // O que começava em 10s passa a começar em 2s.
        assert!(
            out.contains("Dialogue: 0,0:00:02.00,0:00:14.00,Danmaku"),
            "{}",
            out
        );
    }

    #[test]
    fn shift_clamps_to_the_window() {
        // O primeiro comentário abre antes da janela e fecha dentro dela.
        let (out, kept) = shift_ass(ASS, 8.0, 3.0);
        assert_eq!(kept, 2);
        // Começo negativo vira zero e nada passa do fim da janela.
        for line in out.lines().filter(|l| l.starts_with("Dialogue:")) {
            let rest = line.split_once(':').unwrap().1;
            let f: Vec<&str> = rest.splitn(10, ',').collect();
            let s = parse_ass_time(f[1]).unwrap();
            let e = parse_ass_time(f[2]).unwrap();
            assert!(s >= 0.0, "{}", line);
            assert!(e <= 3.0, "{}", line);
        }
    }

    #[test]
    fn shift_drops_what_falls_outside() {
        let (out, kept) = shift_ass(ASS, 30.0, 10.0);
        assert_eq!(kept, 0, "{}", out);
        // Sem diálogo, mas o cabeçalho continua de pé.
        assert!(out.contains("[Events]"));
    }

    #[test]
    fn shift_without_cut_is_a_copy_of_the_events() {
        let (out, kept) = shift_ass(ASS, 0.0, 3600.0);
        assert_eq!(kept, 3);
        assert!(out.contains("0:00:01.00,0:00:13.00"));
        assert!(out.contains("0:01:00.00,0:01:12.00"));
    }

    #[test]
    fn srt_goes_through_the_tools_parser() {
        let srt = "1\n00:00:05,000 --> 00:00:07,000\nprimeira\n\n2\n00:01:00,000 --> 00:01:02,000\nsegunda\n\n";
        let (out, kept) = shift_other(srt, 4.0, 10.0).unwrap();
        assert_eq!(kept, 1);
        assert!(out.contains("[Events]"));
        assert!(out.contains("Dialogue: 0,0:00:01.00,0:00:03.00"), "{}", out);
    }

    #[test]
    fn cut_args_are_omitted_when_zero() {
        assert!(cut_args(0.0, 0.0).is_empty());
        assert_eq!(cut_args(2.5, 0.0), vec!["-ss", "2.500"]);
        assert_eq!(cut_args(0.0, 4.0), vec!["-t", "4.000"]);
    }

    #[test]
    fn filter_escapes_the_path_and_scales_before_drawing() {
        let chain = filter_chain(Path::new("C:\\temp\\a.ass"), "", 0);
        assert_eq!(chain, "ass='C\\:/temp/a.ass'");

        let scaled = filter_chain(Path::new("/tmp/a.ass"), "/tmp/fontes", 720);
        assert!(scaled.starts_with("scale=-2:'min(720,ih)'"), "{}", scaled);
        assert!(
            scaled.contains("ass='/tmp/a.ass':fontsdir='/tmp/fontes'"),
            "{}",
            scaled
        );
        // A escala tem que vir antes do ass, senão o texto é reduzido junto.
        assert!(scaled.find("scale=").unwrap() < scaled.find("ass=").unwrap());
    }

    #[test]
    fn odd_heights_are_rounded_down_for_h264() {
        assert!(filter_chain(Path::new("/tmp/a.ass"), "", 721).contains("min(720,ih)"));
    }

    #[test]
    fn unknown_preset_falls_back() {
        assert_eq!(sane_preset("slow"), "slow");
        assert_eq!(sane_preset("turbo"), "veryfast");
        assert_eq!(sane_preset(" medium "), "medium");
    }

    #[test]
    fn audio_is_reencoded_or_dropped() {
        assert_eq!(audio_args(false, 96), vec!["-an"]);
        assert_eq!(audio_args(true, 0), vec!["-c:a", "aac", "-b:a", "128k"]);
        assert_eq!(audio_args(true, 96), vec!["-c:a", "aac", "-b:a", "96k"]);
    }

    /// Ponta a ponta com o FFmpeg gerido: gera um clipe, recorta e confere que
    /// o comentário aparece na hora certa (o quadro fica mais claro quando o
    /// texto branco entra em cena).
    /// `cargo test -p omniget --lib -- --ignored live_danmaku_burn`
    #[tokio::test]
    #[ignore]
    async fn live_danmaku_burn_keeps_the_timing() {
        let ffmpeg = omniget_core::core::dependencies::ensure_ffmpeg()
            .await
            .unwrap();
        let dir = std::env::temp_dir().join("omniget-danmaku-burn-live");
        std::fs::create_dir_all(&dir).unwrap();
        let src = dir.join("fonte.mp4");
        let gen = omniget_core::core::process::command(&ffmpeg)
            .args([
                "-y",
                "-hide_banner",
                "-loglevel",
                "error",
                "-f",
                "lavfi",
                "-i",
                "color=c=black:size=640x360:rate=25:duration=20",
                "-f",
                "lavfi",
                "-i",
                "sine=frequency=440:duration=20",
                "-c:v",
                "libx264",
                "-preset",
                "ultrafast",
                "-pix_fmt",
                "yuv420p",
                "-c:a",
                "aac",
            ])
            .arg(&src)
            .output()
            .await
            .unwrap();
        assert!(gen.status.success());

        // Um comentário só, de 12s a 16s no vídeo original.
        let ass = dir.join("danmaku.ass");
        std::fs::write(
            &ass,
            "[Script Info]\nScriptType: v4.00+\nPlayResX: 640\nPlayResY: 360\n\n[V4+ Styles]\nFormat: Name, Fontname, Fontsize, PrimaryColour, SecondaryColour, OutlineColour, BackColour, Bold, Italic, Underline, StrikeOut, ScaleX, ScaleY, Spacing, Angle, BorderStyle, Outline, Shadow, Alignment, MarginL, MarginR, MarginV, Encoding\nStyle: Danmaku,Arial,72,&H00FFFFFF,&H00FFFFFF,&H00000000,&H00000000,0,0,0,0,100,100,0,0,1,1.0,0.0,7,0,0,0,1\n\n[Events]\nFormat: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\nDialogue: 0,0:00:12.00,0:00:16.00,Danmaku,,0,0,0,,{\\pos(320,160)}WWWWWWWW\n",
        )
        .unwrap();

        let opts = BurnOptions {
            video: src.to_string_lossy().to_string(),
            subtitle: ass.to_string_lossy().to_string(),
            start: 10.0,
            duration: 8.0,
            target_mb: 0.0,
            crf: 20,
            preset: "ultrafast".into(),
            max_height: 0,
            fonts_dir: String::new(),
            output_dir: dir.to_string_lossy().to_string(),
            suffix: String::new(),
        };
        let res = run(&opts, &omniget_core::core::tools::noop_progress())
            .await
            .unwrap();
        assert_eq!(res.dialogues, 1);
        assert!(res.bytes > 1000, "saiu pequeno demais: {}", res.bytes);
        let probe = omniget_core::core::ffmpeg::probe(Path::new(&res.output))
            .await
            .unwrap();
        assert!(
            (probe.duration_seconds - 8.0).abs() < 0.5,
            "duração: {}",
            probe.duration_seconds
        );
        assert!(probe.streams.iter().any(|s| s.codec_type == "audio"));

        // O texto tem que estar por volta de 2s-6s do clipe, não no começo.
        let brightness = |at: f64| {
            let ffmpeg = ffmpeg.clone();
            let out = res.output.clone();
            let dir = dir.clone();
            async move {
                let png = dir.join(format!("f{:.0}.png", at * 10.0));
                let o = omniget_core::core::process::command(&ffmpeg)
                    .args(["-y", "-hide_banner", "-loglevel", "error", "-ss"])
                    .arg(format!("{:.2}", at))
                    .args(["-i", &out, "-frames:v", "1"])
                    .arg(&png)
                    .output()
                    .await
                    .unwrap();
                assert!(o.status.success());
                std::fs::metadata(&png).unwrap().len()
            }
        };
        let antes = brightness(0.5).await;
        let durante = brightness(3.0).await;
        assert!(
            durante > antes,
            "o comentário não apareceu na janela certa: {} vs {}",
            antes,
            durante
        );
        eprintln!("clipe: {} bytes, {} diálogo(s)", res.bytes, res.dialogues);
    }

    /// Mesmo clipe, agora mirando um tamanho de arquivo.
    /// `cargo test -p omniget --lib -- --ignored live_danmaku_burn_target`
    #[tokio::test]
    #[ignore]
    async fn live_danmaku_burn_hits_the_size_target() {
        let ffmpeg = omniget_core::core::dependencies::ensure_ffmpeg()
            .await
            .unwrap();
        let dir = std::env::temp_dir().join("omniget-danmaku-target-live");
        std::fs::create_dir_all(&dir).unwrap();
        let src = dir.join("fonte.mp4");
        let gen = omniget_core::core::process::command(&ffmpeg)
            .args([
                "-y",
                "-hide_banner",
                "-loglevel",
                "error",
                "-f",
                "lavfi",
                "-i",
                "testsrc2=size=1280x720:rate=30:duration=15",
                "-c:v",
                "libx264",
                "-preset",
                "ultrafast",
                "-crf",
                "18",
                "-pix_fmt",
                "yuv420p",
            ])
            .arg(&src)
            .output()
            .await
            .unwrap();
        assert!(gen.status.success());

        let ass = dir.join("danmaku.ass");
        std::fs::write(
            &ass,
            "[Script Info]\nScriptType: v4.00+\nPlayResX: 1280\nPlayResY: 720\n\n[Events]\nFormat: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\nDialogue: 0,0:00:00.00,0:00:15.00,Default,,0,0,0,,{\\pos(640,360)}danmaku\n",
        )
        .unwrap();

        let opts = BurnOptions {
            video: src.to_string_lossy().to_string(),
            subtitle: ass.to_string_lossy().to_string(),
            start: 0.0,
            duration: 0.0,
            target_mb: 1.0,
            crf: 20,
            preset: "veryfast".into(),
            max_height: 480,
            fonts_dir: String::new(),
            output_dir: dir.to_string_lossy().to_string(),
            suffix: "-alvo".into(),
        };
        let res = run(&opts, &omniget_core::core::tools::noop_progress())
            .await
            .unwrap();
        assert!(res.video_kbps > 0);
        assert!(
            res.bytes <= 1024 * 1024,
            "{} bytes passaram de 1 MB",
            res.bytes
        );
        // O log de duas passagens não pode ficar para trás.
        assert!(
            !std::fs::read_dir(&dir)
                .unwrap()
                .flatten()
                .any(|e| e.file_name().to_string_lossy().contains("pass")),
            "sobrou log de passagem"
        );
        eprintln!("alvo 1 MB → {} bytes a {} kbps", res.bytes, res.video_kbps);
    }
}

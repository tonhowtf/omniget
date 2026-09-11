//! Comprimir vídeo para um alvo exato de tamanho (o limite de anexo do
//! Discord, do e-mail, do WhatsApp) com duas passagens do FFmpeg já gerido.
//!
//! O padrão é o mesmo do ffmpeg4discord: mede a duração, divide o orçamento
//! de bits pelo tempo, tira o áudio da conta e roda `-pass 1` / `-pass 2`.
//! Uma passagem só erra o alvo para cima ou para baixo; duas acertam.

use std::path::{Path, PathBuf};

use anyhow::anyhow;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize)]
pub struct CompressOptions {
    pub inputs: Vec<String>,
    /// Alvo em MB (base 1024, que é o que o Discord conta).
    pub target_mb: f64,
    /// "h264" | "h265" | "vp9"
    #[serde(default = "default_codec")]
    pub codec: String,
    /// 0 = escolher sozinho (128k estéreo, 96k mono); o áudio sai do orçamento.
    #[serde(default)]
    pub audio_kbps: u32,
    /// 0 = manter a resolução original.
    #[serde(default)]
    pub max_height: u32,
    /// 0 = manter o frame rate original.
    #[serde(default)]
    pub fps: u32,
    #[serde(default = "default_preset")]
    pub preset: String,
    #[serde(default)]
    pub output_dir: String,
    #[serde(default)]
    pub suffix: String,
}

fn default_codec() -> String {
    "h264".into()
}

fn default_preset() -> String {
    "medium".into()
}

#[derive(Debug, Clone, Serialize)]
pub struct CompressItem {
    pub input: String,
    pub output: Option<String>,
    pub bytes_before: u64,
    pub bytes_after: u64,
    pub target_bytes: u64,
    pub video_kbps: u32,
    pub audio_kbps: u32,
    pub duration_s: f64,
    pub ok: bool,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CompressResult {
    pub items: Vec<CompressItem>,
}

/// Divide o orçamento de bits entre vídeo e áudio. Devolve `(vídeo, áudio)`
/// em kbps, ou `None` quando nem cortando o áudio o alvo cabe.
///
/// `headroom` desconta o overhead do container (moov box, índices): mirar
/// 100% do alvo estoura por alguns por cento em arquivo curto.
pub fn plan_bitrates(
    duration_s: f64,
    target_bytes: u64,
    audio_kbps: u32,
    has_audio: bool,
    headroom: f64,
) -> Option<(u32, u32)> {
    if duration_s <= 0.0 || target_bytes == 0 {
        return None;
    }
    let total_kbps = (target_bytes as f64 * 8.0 * headroom) / (duration_s * 1000.0);
    if !has_audio {
        let v = total_kbps.floor() as u32;
        return (v >= 60).then_some((v, 0));
    }
    // Vai cedendo o áudio antes de desistir: 96k já é aceitável para fala.
    for a in [audio_kbps, 96, 64, 48, 32] {
        if a == 0 {
            continue;
        }
        let v = total_kbps - a as f64;
        if v >= 60.0 {
            return Some((v.floor() as u32, a));
        }
    }
    None
}

fn null_sink() -> &'static str {
    if cfg!(windows) {
        "NUL"
    } else {
        "/dev/null"
    }
}

fn container(codec: &str) -> &'static str {
    if codec == "vp9" {
        "webm"
    } else {
        "mp4"
    }
}

fn video_encoder(codec: &str) -> &'static str {
    match codec {
        "h265" | "hevc" => "libx265",
        "vp9" => "libvpx-vp9",
        _ => "libx264",
    }
}

fn audio_encoder(codec: &str) -> &'static str {
    if codec == "vp9" {
        "libopus"
    } else {
        "aac"
    }
}

fn scale_filter(max_height: u32) -> Option<String> {
    (max_height > 0).then(|| {
        format!(
            "scale=-2:'min({h},ih)':flags=lanczos",
            h = max_height - max_height % 2
        )
    })
}

/// Argumentos comuns às duas passagens (entrada, filtros, bitrate de vídeo).
fn common_args(opts: &CompressOptions, video_kbps: u32) -> Vec<String> {
    let mut a: Vec<String> = vec![
        "-c:v".into(),
        video_encoder(&opts.codec).into(),
        "-b:v".into(),
        format!("{}k", video_kbps),
    ];
    if opts.codec != "vp9" {
        a.push("-preset".into());
        a.push(opts.preset.clone());
    }
    if let Some(f) = scale_filter(opts.max_height) {
        a.push("-vf".into());
        a.push(f);
    }
    if opts.fps > 0 {
        a.push("-r".into());
        a.push(opts.fps.to_string());
    }
    a
}

async fn run_pass(
    ffmpeg: &Path,
    input: &Path,
    args: &[String],
    pass: u8,
    log: &Path,
    tail: &[String],
) -> anyhow::Result<()> {
    let mut cmd = crate::core::process::command(ffmpeg);
    cmd.args(["-y", "-hide_banner", "-loglevel", "error", "-i"])
        .arg(input)
        .args(args)
        .args(["-pass", &pass.to_string()])
        .arg("-passlogfile")
        .arg(log)
        .args(tail);
    let out = cmd
        .output()
        .await
        .map_err(|e| anyhow!("ffmpeg nao iniciou: {}", e))?;
    if !out.status.success() {
        return Err(anyhow!(
            "ffmpeg passagem {}: {}",
            pass,
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    Ok(())
}

fn clean_logs(log: &Path) {
    for suffix in ["-0.log", "-0.log.mbtree", ".log", ".log.mbtree"] {
        let p = PathBuf::from(format!("{}{}", log.display(), suffix));
        let _ = std::fs::remove_file(p);
    }
    if let Some(dir) = log.parent() {
        let _ = std::fs::remove_dir(dir);
    }
}

async fn compress_one(
    ffmpeg: &Path,
    opts: &CompressOptions,
    input: &str,
) -> anyhow::Result<CompressItem> {
    let inp = Path::new(input);
    let target_bytes = (opts.target_mb.max(0.1) * 1024.0 * 1024.0) as u64;
    let bytes_before = std::fs::metadata(inp).map(|m| m.len()).unwrap_or(0);

    let probe = crate::core::ffmpeg::probe(inp).await?;
    let has_audio = probe.streams.iter().any(|s| s.codec_type == "audio");
    let channels = probe
        .streams
        .iter()
        .find(|s| s.codec_type == "audio")
        .and_then(|s| s.channels)
        .unwrap_or(2);
    let wanted_audio = if opts.audio_kbps > 0 {
        opts.audio_kbps
    } else if channels <= 1 {
        96
    } else {
        128
    };
    let (video_kbps, audio_kbps) = plan_bitrates(
        probe.duration_seconds,
        target_bytes,
        wanted_audio,
        has_audio,
        0.97,
    )
    .ok_or_else(|| {
        anyhow!(
            "alvo pequeno demais para {:.0}s de vídeo — aumente o tamanho ou corte o clipe",
            probe.duration_seconds
        )
    })?;

    let out_dir = if opts.output_dir.trim().is_empty() {
        inp.parent().map(|p| p.to_path_buf()).unwrap_or_default()
    } else {
        PathBuf::from(opts.output_dir.trim())
    };
    std::fs::create_dir_all(&out_dir)?;
    let stem = inp
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "video".into());
    let suffix = if opts.suffix.is_empty() {
        "-small"
    } else {
        opts.suffix.as_str()
    };
    let output = out_dir.join(format!("{}{}.{}", stem, suffix, container(&opts.codec)));

    let log_dir = std::env::temp_dir().join(format!("omniget-2pass-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&log_dir)?;
    let log = log_dir.join("pass");

    let mut kbps = video_kbps;
    let mut bytes_after = 0u64;
    // Uma correção só: se a primeira tentativa passar do alvo, reencaixa o
    // bitrate na proporção do estouro e refaz a passagem 2 (o log da 1 vale).
    for attempt in 0..2u8 {
        let args = common_args(opts, kbps);
        let mut tail: Vec<String> = Vec::new();
        if has_audio && audio_kbps > 0 {
            tail.push("-c:a".into());
            tail.push(audio_encoder(&opts.codec).into());
            tail.push("-b:a".into());
            tail.push(format!("{}k", audio_kbps));
        } else {
            tail.push("-an".into());
        }
        if container(&opts.codec) == "mp4" {
            tail.push("-movflags".into());
            tail.push("+faststart".into());
        }

        if attempt == 0 {
            let first: Vec<String> = vec![
                "-an".into(),
                "-f".into(),
                if container(&opts.codec) == "webm" {
                    "webm".into()
                } else {
                    "mp4".into()
                },
                null_sink().into(),
            ];
            run_pass(ffmpeg, inp, &args, 1, &log, &first).await?;
        }
        let mut second = tail.clone();
        second.push(output.to_string_lossy().to_string());
        run_pass(ffmpeg, inp, &args, 2, &log, &second).await?;

        bytes_after = std::fs::metadata(&output).map(|m| m.len()).unwrap_or(0);
        if bytes_after <= target_bytes || attempt == 1 {
            break;
        }
        let ratio = target_bytes as f64 / bytes_after as f64;
        let next = (kbps as f64 * ratio * 0.97).floor() as u32;
        if next < 60 || next >= kbps {
            break;
        }
        kbps = next;
    }
    clean_logs(&log);

    Ok(CompressItem {
        input: input.to_string(),
        output: Some(output.to_string_lossy().to_string()),
        bytes_before,
        bytes_after,
        target_bytes,
        video_kbps: kbps,
        audio_kbps: if has_audio { audio_kbps } else { 0 },
        duration_s: probe.duration_seconds,
        ok: true,
        error: None,
    })
}

pub async fn run(
    opts: CompressOptions,
    progress: super::ProgressFn,
) -> anyhow::Result<CompressResult> {
    let ffmpeg = crate::core::dependencies::ensure_ffmpeg().await?;
    let total = opts.inputs.len() as u64;
    let mut items = Vec::new();
    for (i, input) in opts.inputs.iter().enumerate() {
        super::report(
            &progress,
            "video-compress",
            "progress",
            i as u64,
            Some(total),
            Some(input.clone()),
        );
        match compress_one(&ffmpeg, &opts, input).await {
            Ok(item) => items.push(item),
            Err(e) => {
                tracing::warn!("[video-compress] {}: {}", input, e);
                items.push(CompressItem {
                    input: input.clone(),
                    output: None,
                    bytes_before: std::fs::metadata(input).map(|m| m.len()).unwrap_or(0),
                    bytes_after: 0,
                    target_bytes: (opts.target_mb.max(0.1) * 1024.0 * 1024.0) as u64,
                    video_kbps: 0,
                    audio_kbps: 0,
                    duration_s: 0.0,
                    ok: false,
                    error: Some(e.to_string()),
                });
            }
        }
    }
    super::report(
        &progress,
        "video-compress",
        "done",
        total,
        Some(total),
        None,
    );
    Ok(CompressResult { items })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn budget_splits_audio_out() {
        // 10 MB para 60 s: ~1356 kbps no total, menos 128 de áudio.
        let (v, a) = plan_bitrates(60.0, 10 * 1024 * 1024, 128, true, 0.97).unwrap();
        assert_eq!(a, 128);
        assert!((1200..1260).contains(&v), "bitrate fora da faixa: {}", v);
    }

    #[test]
    fn budget_gives_everything_to_video_when_muted() {
        let (v, a) = plan_bitrates(60.0, 10 * 1024 * 1024, 128, false, 0.97).unwrap();
        assert_eq!(a, 0);
        assert!(v > 1300);
    }

    #[test]
    fn budget_degrades_audio_before_failing() {
        // 8 MB para 10 min: com 128k de áudio não sobra vídeo; tem que ceder.
        let (v, a) = plan_bitrates(600.0, 8 * 1024 * 1024, 128, true, 0.97).unwrap();
        assert_eq!(a, 48);
        assert!(v >= 60, "vídeo ficou abaixo do piso: {}", v);
    }

    #[test]
    fn budget_refuses_the_impossible() {
        assert!(plan_bitrates(7200.0, 1024 * 1024, 128, true, 0.97).is_none());
        // 8 MB para 30 min não cabe nem cortando o áudio ao mínimo.
        assert!(plan_bitrates(1800.0, 8 * 1024 * 1024, 128, true, 0.97).is_none());
        assert!(plan_bitrates(0.0, 10 * 1024 * 1024, 128, true, 0.97).is_none());
    }

    #[test]
    fn scale_keeps_even_dimensions() {
        assert_eq!(scale_filter(0), None);
        assert!(scale_filter(721).unwrap().contains("min(720,ih)"));
    }

    /// Ponta a ponta com o FFmpeg gerido: gera um clipe, comprime para 1 MB
    /// e confere que o alvo foi respeitado sem perder áudio nem duração.
    /// `cargo test -p omniget-core --lib -- --ignored live_compress`
    #[tokio::test]
    #[ignore]
    async fn live_compress_hits_the_target() {
        let ffmpeg = crate::core::dependencies::ensure_ffmpeg().await.unwrap();
        let dir = std::env::temp_dir().join("omniget-vcompress-live");
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
                "testsrc2=size=1280x720:rate=30:duration=20",
                "-f",
                "lavfi",
                "-i",
                "sine=frequency=440:duration=20",
                "-c:v",
                "libx264",
                "-preset",
                "ultrafast",
                "-crf",
                "18",
                "-c:a",
                "aac",
            ])
            .arg(&src)
            .output()
            .await
            .unwrap();
        assert!(gen.status.success(), "não consegui gerar o clipe de teste");
        let original = std::fs::metadata(&src).unwrap().len();

        let opts = CompressOptions {
            inputs: vec![src.to_string_lossy().to_string()],
            target_mb: 1.0,
            codec: "h264".into(),
            audio_kbps: 0,
            max_height: 0,
            fps: 0,
            preset: "veryfast".into(),
            output_dir: dir.to_string_lossy().to_string(),
            suffix: String::new(),
        };
        let res = run(opts, crate::core::tools::noop_progress())
            .await
            .unwrap();
        let item = &res.items[0];
        assert!(item.ok, "falhou: {:?}", item.error);
        assert!(
            item.bytes_after <= item.target_bytes,
            "{} bytes passaram do alvo de {}",
            item.bytes_after,
            item.target_bytes
        );
        assert!(item.bytes_after < original, "não comprimiu nada");

        let out = item.output.as_ref().unwrap();
        let probe = crate::core::ffmpeg::probe(std::path::Path::new(out))
            .await
            .unwrap();
        assert!(
            (probe.duration_seconds - 20.0).abs() < 1.0,
            "duração mudou: {}",
            probe.duration_seconds
        );
        assert!(
            probe.streams.iter().any(|s| s.codec_type == "audio"),
            "perdeu o áudio"
        );
        eprintln!(
            "{} → {} bytes (alvo {}), vídeo {} kbps",
            original, item.bytes_after, item.target_bytes, item.video_kbps
        );
        // O log de duas passagens não pode ficar para trás.
        assert!(
            !std::fs::read_dir(&dir)
                .unwrap()
                .flatten()
                .any(|e| e.file_name().to_string_lossy().contains("pass")),
            "sobrou log de passagem na pasta de saída"
        );
    }

    #[test]
    fn containers_follow_the_codec() {
        assert_eq!(container("vp9"), "webm");
        assert_eq!(container("h265"), "mp4");
        assert_eq!(video_encoder("h265"), "libx265");
        assert_eq!(audio_encoder("vp9"), "libopus");
    }
}

//! Limpar áudio: normalizar loudness (EBU R128) e tirar ruído de fundo.
//!
//! Normalizar com `volume` ou pico não serve para voz: o que o ouvido (e o
//! YouTube, e o Spotify) medem é loudness integrado. O `loudnorm` faz isso,
//! mas numa passagem só ele trabalha em modo dinâmico e "bombeia". Duas
//! passagens — medir, depois aplicar as medidas — dão ganho linear.
//!
//! O ruído sai com filtros nativos (`afftdn`, `anlmdn`), sem baixar modelo. O
//! `arnndn` (RNNoise) só entra se o usuário já tiver um `.rnnn` na máquina.

use std::path::{Path, PathBuf};

use anyhow::anyhow;
use serde::{Deserialize, Serialize};

// ── Loudness ───────────────────────────────────────────────────────────

/// Alvos de LUFS integrados que as plataformas usam.
pub const TARGETS: &[(&str, f64, f64)] = &[
    ("streaming", -14.0, -1.0),
    ("podcast", -16.0, -1.5),
    ("broadcast", -23.0, -2.0),
];

pub fn target_for(name: &str) -> (f64, f64) {
    TARGETS
        .iter()
        .find(|(n, _, _)| *n == name)
        .map(|(_, i, tp)| (*i, *tp))
        .unwrap_or((-16.0, -1.5))
}

#[derive(Debug, Clone, Copy, Serialize)]
pub struct Loudness {
    pub input_i: f64,
    pub input_tp: f64,
    pub input_lra: f64,
    pub input_thresh: f64,
    pub target_offset: f64,
}

/// O `loudnorm` imprime um JSON no stderr, junto com o resto do log.
pub fn parse_measure(stderr: &str) -> Option<Loudness> {
    let start = stderr.rfind('{')?;
    let end = stderr[start..].find('}')? + start + 1;
    let v: serde_json::Value = serde_json::from_str(&stderr[start..end]).ok()?;
    let num = |k: &str| -> Option<f64> {
        let s = v.get(k)?.as_str()?;
        // Silêncio total devolve "-inf", e `parse` aceita isso: infinito na
        // linha de comando do ffmpeg quebra a segunda passagem. Vira -99.
        let n: f64 = s.parse().unwrap_or(-99.0);
        Some(if n.is_finite() { n } else { -99.0 })
    };
    Some(Loudness {
        input_i: num("input_i")?,
        input_tp: num("input_tp")?,
        input_lra: num("input_lra")?,
        input_thresh: num("input_thresh")?,
        target_offset: num("target_offset").unwrap_or(0.0),
    })
}

pub fn measure_filter(target_i: f64, target_tp: f64) -> String {
    format!(
        "loudnorm=I={}:TP={}:LRA=11:print_format=json",
        target_i, target_tp
    )
}

pub fn apply_filter(m: &Loudness, target_i: f64, target_tp: f64) -> String {
    format!(
        "loudnorm=I={}:TP={}:LRA=11:measured_I={:.2}:measured_TP={:.2}:measured_LRA={:.2}:measured_thresh={:.2}:offset={:.2}:linear=true:print_format=summary",
        target_i, target_tp, m.input_i, m.input_tp, m.input_lra, m.input_thresh, m.target_offset
    )
}

// ── Ruído ──────────────────────────────────────────────────────────────

/// Cadeia de filtros por modo. `voice` corta o que não é voz antes de
/// denoiser nenhum: quase todo ruído de sala mora fora de 80 Hz-12 kHz.
pub fn denoise_filter(mode: &str, strength: u32, rnnn: Option<&str>) -> String {
    let s = strength.clamp(1, 100) as f64;
    match mode {
        "nlm" => format!("anlmdn=s={:.5}:p=0.002:r=0.006", 0.00001 + s * 0.0001),
        "voice" => format!(
            "highpass=f=80,lowpass=f=12000,afftdn=nr={:.0}:nf=-25:tn=1",
            6.0 + s * 0.3
        ),
        "rnnoise" => match rnnn {
            Some(path) => format!("arnndn=m='{}'", path.replace('\\', "/").replace(':', "\\:")),
            None => format!("afftdn=nr={:.0}:nf=-25:tn=1", 6.0 + s * 0.3),
        },
        _ => format!("afftdn=nr={:.0}:nf=-25:tn=1", 6.0 + s * 0.3),
    }
}

// ── Execução ───────────────────────────────────────────────────────────

#[derive(Debug, Clone, Deserialize)]
#[cfg_attr(test, derive(Default))]
pub struct CleanOptions {
    pub inputs: Vec<String>,
    /// "loudness" | "denoise" | "both"
    #[serde(default = "default_mode")]
    pub mode: String,
    /// Alvo de loudness: "streaming" | "podcast" | "broadcast".
    #[serde(default = "default_target")]
    pub target: String,
    /// "fft" | "nlm" | "voice" | "rnnoise"
    #[serde(default = "default_denoise")]
    pub denoise_mode: String,
    #[serde(default = "default_strength")]
    pub strength: u32,
    /// Caminho de um modelo `.rnnn`, quando o usuário tiver um.
    #[serde(default)]
    pub rnnn_path: String,
    /// Extensão de saída para arquivo de áudio; vídeo mantém o container.
    #[serde(default)]
    pub audio_format: String,
    #[serde(default)]
    pub output_dir: String,
    #[serde(default)]
    pub suffix: String,
}

fn default_mode() -> String {
    "loudness".into()
}
fn default_target() -> String {
    "podcast".into()
}
fn default_denoise() -> String {
    "fft".into()
}
fn default_strength() -> u32 {
    50
}

#[derive(Debug, Clone, Serialize)]
pub struct CleanItem {
    pub input: String,
    pub output: Option<String>,
    pub measured: Option<Loudness>,
    pub target_i: f64,
    pub gain_db: f64,
    pub filter: String,
    pub ok: bool,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CleanResult {
    pub items: Vec<CleanItem>,
}

async fn measure(ffmpeg: &Path, input: &Path, filter: &str) -> anyhow::Result<Loudness> {
    let out = crate::core::process::command(ffmpeg)
        .args(["-hide_banner", "-nostats", "-i"])
        .arg(input)
        .args(["-af", filter, "-f", "null", "-"])
        .output()
        .await
        .map_err(|e| anyhow!("ffmpeg nao iniciou: {}", e))?;
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    parse_measure(&stderr).ok_or_else(|| {
        anyhow!(
            "não consegui medir o loudness: {}",
            stderr.lines().last().unwrap_or("").trim()
        )
    })
}

async fn clean_one(ffmpeg: &Path, opts: &CleanOptions, input: &str) -> anyhow::Result<CleanItem> {
    let inp = Path::new(input);
    let probe = crate::core::ffmpeg::probe(inp).await?;
    if !probe.streams.iter().any(|s| s.codec_type == "audio") {
        return Err(anyhow!("o arquivo não tem trilha de áudio"));
    }
    let has_video = probe
        .streams
        .iter()
        .any(|s| s.codec_type == "video" && s.codec_name != "mjpeg" && s.codec_name != "png");
    let (target_i, target_tp) = target_for(&opts.target);

    let mut chain: Vec<String> = Vec::new();
    let mut measured = None;
    if opts.mode == "denoise" || opts.mode == "both" {
        chain.push(denoise_filter(
            &opts.denoise_mode,
            opts.strength,
            (!opts.rnnn_path.trim().is_empty()).then_some(opts.rnnn_path.trim()),
        ));
    }
    if opts.mode == "loudness" || opts.mode == "both" {
        let m = measure(ffmpeg, inp, &measure_filter(target_i, target_tp)).await?;
        chain.push(apply_filter(&m, target_i, target_tp));
        measured = Some(m);
    }
    if chain.is_empty() {
        return Err(anyhow!("nenhuma operação escolhida"));
    }
    let filter = chain.join(",");

    let out_dir = if opts.output_dir.trim().is_empty() {
        inp.parent().map(|p| p.to_path_buf()).unwrap_or_default()
    } else {
        PathBuf::from(opts.output_dir.trim())
    };
    std::fs::create_dir_all(&out_dir)?;
    let stem = inp
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "audio".into());
    let ext = if has_video {
        inp.extension()
            .map(|e| e.to_string_lossy().to_string())
            .unwrap_or_else(|| "mp4".into())
    } else if opts.audio_format.trim().is_empty() {
        inp.extension()
            .map(|e| e.to_string_lossy().to_string())
            .unwrap_or_else(|| "m4a".into())
    } else {
        opts.audio_format.trim().to_string()
    };
    let suffix = if opts.suffix.is_empty() {
        "-limpo"
    } else {
        opts.suffix.as_str()
    };
    let output = out_dir.join(format!("{}{}.{}", stem, suffix, ext));

    let mut cmd = crate::core::process::command(ffmpeg);
    cmd.args(["-y", "-hide_banner", "-loglevel", "error", "-i"])
        .arg(inp)
        .args(["-af", &filter]);
    if has_video {
        // O vídeo não é tocado: só a trilha de áudio é reescrita.
        cmd.args(["-c:v", "copy", "-map", "0"]);
    } else {
        cmd.args(["-vn"]);
    }
    match ext.as_str() {
        "wav" => cmd.args(["-c:a", "pcm_s16le"]),
        "flac" => cmd.args(["-c:a", "flac"]),
        "mp3" => cmd.args(["-c:a", "libmp3lame", "-b:a", "192k"]),
        _ => cmd.args(["-c:a", "aac", "-b:a", "192k"]),
    };
    let out = cmd
        .arg(&output)
        .output()
        .await
        .map_err(|e| anyhow!("ffmpeg nao iniciou: {}", e))?;
    if !out.status.success() {
        return Err(anyhow!("{}", String::from_utf8_lossy(&out.stderr).trim()));
    }

    Ok(CleanItem {
        input: input.to_string(),
        output: Some(output.to_string_lossy().to_string()),
        gain_db: measured.map(|m| target_i - m.input_i).unwrap_or(0.0),
        measured,
        target_i,
        filter,
        ok: true,
        error: None,
    })
}

pub async fn run(opts: CleanOptions, progress: super::ProgressFn) -> anyhow::Result<CleanResult> {
    let ffmpeg = crate::core::dependencies::ensure_ffmpeg().await?;
    let total = opts.inputs.len() as u64;
    let mut items = Vec::new();
    for (i, input) in opts.inputs.iter().enumerate() {
        super::report(
            &progress,
            "audio-clean",
            "progress",
            i as u64,
            Some(total),
            Some(input.clone()),
        );
        match clean_one(&ffmpeg, &opts, input).await {
            Ok(item) => items.push(item),
            Err(e) => {
                tracing::warn!("[audio-clean] {}: {}", input, e);
                items.push(CleanItem {
                    input: input.clone(),
                    output: None,
                    measured: None,
                    target_i: target_for(&opts.target).0,
                    gain_db: 0.0,
                    filter: String::new(),
                    ok: false,
                    error: Some(e.to_string()),
                });
            }
        }
    }
    super::report(&progress, "audio-clean", "done", total, Some(total), None);
    Ok(CleanResult { items })
}

#[cfg(test)]
mod tests {
    use super::*;

    const MEASURE_LOG: &str = r#"
[Parsed_loudnorm_0 @ 0x14] 
{
	"input_i" : "-27.61",
	"input_tp" : "-9.61",
	"input_lra" : "5.20",
	"input_thresh" : "-37.85",
	"output_i" : "-16.02",
	"target_offset" : "0.02"
}
"#;

    #[test]
    fn reads_the_measurement_json_out_of_the_log() {
        let m = parse_measure(MEASURE_LOG).unwrap();
        assert_eq!(m.input_i, -27.61);
        assert_eq!(m.input_tp, -9.61);
        assert_eq!(m.input_lra, 5.20);
        assert_eq!(m.input_thresh, -37.85);
        assert_eq!(m.target_offset, 0.02);
    }

    #[test]
    fn silence_measures_as_very_low_instead_of_failing() {
        let log = r#"{"input_i":"-inf","input_tp":"-inf","input_lra":"0.00","input_thresh":"-inf","target_offset":"0.00"}"#;
        let m = parse_measure(log).unwrap();
        assert_eq!(m.input_i, -99.0);
    }

    #[test]
    fn a_log_without_json_is_none() {
        assert!(parse_measure("ffmpeg falhou feio").is_none());
    }

    #[test]
    fn apply_filter_carries_every_measurement() {
        let m = parse_measure(MEASURE_LOG).unwrap();
        let f = apply_filter(&m, -16.0, -1.5);
        for needle in [
            "measured_I=-27.61",
            "measured_TP=-9.61",
            "measured_LRA=5.20",
            "measured_thresh=-37.85",
            "linear=true",
            "I=-16",
        ] {
            assert!(f.contains(needle), "faltou {} em {}", needle, f);
        }
    }

    #[test]
    fn targets_match_the_platforms() {
        assert_eq!(target_for("streaming").0, -14.0);
        assert_eq!(target_for("broadcast").0, -23.0);
        // Nome desconhecido cai no podcast, não em zero.
        assert_eq!(target_for("nada disso").0, -16.0);
    }

    #[test]
    fn denoise_modes_pick_different_filters() {
        assert!(denoise_filter("fft", 50, None).starts_with("afftdn=nr=21"));
        assert!(denoise_filter("nlm", 50, None).starts_with("anlmdn"));
        let voice = denoise_filter("voice", 50, None);
        assert!(voice.starts_with("highpass=f=80,lowpass=f=12000,afftdn"));
    }

    #[test]
    fn rnnoise_needs_a_model_and_falls_back_without_one() {
        let with = denoise_filter("rnnoise", 50, Some("/modelos/bd.rnnn"));
        assert!(with.contains("arnndn=m='/modelos/bd.rnnn'"));
        assert!(denoise_filter("rnnoise", 50, None).starts_with("afftdn"));
    }

    /// Mede e normaliza um tom baixo de verdade, e limpa ruído branco.
    /// `cargo test -p omniget-core --lib -- --ignored live_audio`
    #[tokio::test]
    #[ignore]
    async fn live_audio_normalizes_and_denoises() {
        let ffmpeg = crate::core::dependencies::ensure_ffmpeg().await.unwrap();
        let dir = std::env::temp_dir().join("omniget-audio-live");
        std::fs::create_dir_all(&dir).unwrap();
        let src = dir.join("baixo.wav");
        let gen = crate::core::process::command(&ffmpeg)
            .args([
                "-y",
                "-hide_banner",
                "-loglevel",
                "error",
                "-f",
                "lavfi",
                "-i",
                "sine=frequency=440:duration=6,volume=-24dB",
                "-c:a",
                "pcm_s16le",
            ])
            .arg(&src)
            .output()
            .await
            .unwrap();
        assert!(gen.status.success());

        let base = CleanOptions {
            inputs: vec![src.to_string_lossy().to_string()],
            mode: "loudness".into(),
            target: "podcast".into(),
            denoise_mode: "fft".into(),
            strength: 50,
            rnnn_path: String::new(),
            audio_format: "wav".into(),
            output_dir: dir.to_string_lossy().to_string(),
            suffix: "-norm".into(),
        };
        let res = run(base.clone(), crate::core::tools::noop_progress())
            .await
            .unwrap();
        let item = &res.items[0];
        assert!(item.ok, "falhou: {:?}", item.error);
        let m = item.measured.expect("sem medição");
        assert!(m.input_i.is_finite(), "medida infinita: {}", m.input_i);
        assert!(
            m.input_i < -20.0,
            "o tom era para estar baixo: {}",
            m.input_i
        );
        assert!(
            item.gain_db > 0.0,
            "tinha que ganhar volume: {}",
            item.gain_db
        );

        // Confere no arquivo de saída: medir de novo tem que dar perto do alvo.
        let out = item.output.clone().unwrap();
        let after = measure(
            &ffmpeg,
            std::path::Path::new(&out),
            &measure_filter(-16.0, -1.5),
        )
        .await
        .unwrap();
        assert!(
            (after.input_i + 16.0).abs() < 1.5,
            "saiu em {} LUFS, alvo -16",
            after.input_i
        );
        eprintln!("{:.1} → {:.1} LUFS", m.input_i, after.input_i);

        let denoise = CleanOptions {
            mode: "denoise".into(),
            suffix: "-clean".into(),
            ..base
        };
        let res = run(denoise, crate::core::tools::noop_progress())
            .await
            .unwrap();
        assert!(res.items[0].ok, "denoise falhou: {:?}", res.items[0].error);
        assert!(res.items[0].filter.starts_with("afftdn"));
    }

    #[test]
    fn strength_is_clamped() {
        assert_eq!(
            denoise_filter("fft", 0, None),
            denoise_filter("fft", 1, None)
        );
        assert_eq!(
            denoise_filter("fft", 999, None),
            denoise_filter("fft", 100, None)
        );
    }
}

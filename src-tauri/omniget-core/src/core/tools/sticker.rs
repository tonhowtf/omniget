//! Foto ou vídeo → figurinha de WhatsApp e Telegram.
//!
//! Os dois apps aceitam WebP quadrado de 512 px, e é aí que a coisa aperta:
//! o limite não é de resolução, é de **bytes**. WhatsApp quer 100 KB no
//! estático e 500 KB no animado (até 10 s); o Telegram quer 512 KB no
//! estático e, no animado, um `.webm` VP9 de até 3 s e 256 KB.
//!
//! Como não dá para calcular o peso do WebP de cabeça, o módulo faz o mesmo
//! que o `image_compress`: desce uma escada de qualidade/fps até caber e, se
//! nem no degrau mais baixo couber, devolve `hit_target: false` em vez de
//! fingir que deu certo. A transparência é preservada em todos os caminhos
//! (`yuva420p`), e o enquadramento é `contain` (cabe inteiro, com folga em
//! volta) ou `cover` (preenche e corta).

use std::path::{Path, PathBuf};

use anyhow::anyhow;
use serde::{Deserialize, Serialize};

const ID: &str = "wa-sticker";

/// Extensões que valem como fonte animada quando o modo é "auto".
const ANIMATED_EXT: &[&str] = &[
    "mp4", "mov", "mkv", "webm", "avi", "m4v", "gif", "apng", "flv", "ts",
];

// ── Limites de cada alvo ──

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Spec {
    /// Lado do quadro final, em pixels.
    pub size: u32,
    pub max_bytes: u64,
    /// 0 no estático.
    pub max_seconds: f64,
    pub ext: &'static str,
    pub encoder: &'static str,
}

/// Limites oficiais. `target` é "whatsapp" ou "telegram"; qualquer outra
/// coisa cai no WhatsApp, que é o mais apertado dos dois.
pub fn spec(target: &str, animated: bool) -> Spec {
    match (target, animated) {
        ("telegram", false) => Spec {
            size: 512,
            max_bytes: 512 * 1024,
            max_seconds: 0.0,
            ext: "webp",
            encoder: "libwebp",
        },
        ("telegram", true) => Spec {
            size: 512,
            max_bytes: 256 * 1024,
            max_seconds: 3.0,
            ext: "webm",
            encoder: "libvpx-vp9",
        },
        (_, true) => Spec {
            size: 512,
            max_bytes: 500 * 1024,
            max_seconds: 10.0,
            ext: "webp",
            encoder: "libwebp_anim",
        },
        (_, false) => Spec {
            size: 512,
            max_bytes: 100 * 1024,
            max_seconds: 0.0,
            ext: "webp",
            encoder: "libwebp",
        },
    }
}

pub fn is_animated_source(path: &str) -> bool {
    Path::new(path)
        .extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase())
        .is_some_and(|e| ANIMATED_EXT.contains(&e.as_str()))
}

// ── Enquadramento ──

/// Lado útil dentro do quadro depois da margem. Sempre par (o VP9 exige) e
/// nunca menor que 16 px, senão uma margem absurda zeraria a figurinha.
pub fn inner_size(size: u32, padding: u32) -> u32 {
    let size = size.max(16);
    let max_pad = (size - 16) / 2;
    let pad = padding.min(max_pad);
    let inner = size - pad * 2;
    inner - inner % 2
}

/// Cadeia `-vf` do enquadramento. `contain` encaixa a imagem inteira e
/// preenche o resto com transparente; `cover` preenche o quadro e corta o
/// que sobra. Nos dois casos o resultado é `size`×`size` com alfa.
pub fn fit_filter(fit: &str, size: u32, padding: u32) -> String {
    let inner = inner_size(size, padding);
    let core = if fit == "cover" {
        format!(
            "scale={inner}:{inner}:force_original_aspect_ratio=increase:flags=lanczos,crop={inner}:{inner}",
            inner = inner
        )
    } else {
        format!(
            "scale={inner}:{inner}:force_original_aspect_ratio=decrease:flags=lanczos",
            inner = inner
        )
    };
    format!(
        "format=rgba,{},pad={size}:{size}:(ow-iw)/2:(oh-ih)/2:color=#00000000",
        core,
        size = size
    )
}

// ── Escada de tentativas ──

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Attempt {
    /// Qualidade do WebP (1-100); no VP9 vira o fator do bitrate.
    pub quality: u32,
    /// 0 no estático.
    pub fps: u32,
}

/// Degraus da busca, do melhor para o pior. Poucos e bem espaçados: cada
/// degrau é uma chamada de ffmpeg, e vinte tentativas fariam a figurinha
/// demorar mais que o vídeo.
pub fn attempts(animated: bool) -> Vec<Attempt> {
    if animated {
        vec![
            Attempt {
                quality: 75,
                fps: 20,
            },
            Attempt {
                quality: 60,
                fps: 15,
            },
            Attempt {
                quality: 50,
                fps: 15,
            },
            Attempt {
                quality: 40,
                fps: 12,
            },
            Attempt {
                quality: 30,
                fps: 12,
            },
            Attempt {
                quality: 20,
                fps: 10,
            },
            Attempt {
                quality: 12,
                fps: 8,
            },
            Attempt { quality: 6, fps: 6 },
        ]
    } else {
        vec![
            Attempt {
                quality: 92,
                fps: 0,
            },
            Attempt {
                quality: 80,
                fps: 0,
            },
            Attempt {
                quality: 68,
                fps: 0,
            },
            Attempt {
                quality: 55,
                fps: 0,
            },
            Attempt {
                quality: 42,
                fps: 0,
            },
            Attempt {
                quality: 30,
                fps: 0,
            },
            Attempt {
                quality: 18,
                fps: 0,
            },
            Attempt { quality: 8, fps: 0 },
        ]
    }
}

/// Bitrate de vídeo para caber no alvo, em kbps. O `headroom` desconta o
/// overhead do container WebM; mirar 100% estoura por alguns por cento.
pub fn plan_bitrate(target_bytes: u64, duration_s: f64, quality: u32) -> u32 {
    if duration_s <= 0.0 {
        return 256;
    }
    let headroom = 0.90 * (quality.clamp(1, 100) as f64 / 75.0).min(1.0);
    let kbps = (target_bytes as f64 * 8.0 * headroom) / (duration_s * 1000.0);
    (kbps.floor() as u32).clamp(24, 4000)
}

// ── Montagem do comando ──

#[derive(Debug, Clone)]
pub struct Job {
    pub target: String,
    pub animated: bool,
    pub fit: String,
    pub padding: u32,
    pub start: f64,
    /// Duração pedida; 0 = o que couber no limite do alvo.
    pub duration: f64,
}

/// Argumentos completos do ffmpeg para uma tentativa. Função pura: o teste
/// confere a linha inteira sem rodar nada.
pub fn build_args(
    job: &Job,
    spec: &Spec,
    attempt: &Attempt,
    bitrate_kbps: u32,
    input: &str,
    output: &str,
) -> Vec<String> {
    let mut a: Vec<String> = ["-y", "-hide_banner", "-loglevel", "error"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    if job.animated && job.start > 0.0 {
        a.push("-ss".into());
        a.push(format!("{:.3}", job.start));
    }
    a.push("-i".into());
    a.push(input.to_string());

    let mut chain = fit_filter(&job.fit, spec.size, job.padding);
    if job.animated && attempt.fps > 0 {
        chain = format!("fps={},{}", attempt.fps.clamp(1, 30), chain);
    }
    a.push("-vf".into());
    a.push(chain);

    if job.animated {
        let secs = clamp_duration(job.duration, spec.max_seconds);
        a.push("-t".into());
        a.push(format!("{:.3}", secs));
        a.push("-an".into());
        if spec.encoder == "libvpx-vp9" {
            a.push("-c:v".into());
            a.push("libvpx-vp9".into());
            a.push("-pix_fmt".into());
            a.push("yuva420p".into());
            // Alfa em VP9 só funciona com alt-ref desligado.
            a.push("-auto-alt-ref".into());
            a.push("0".into());
            a.push("-b:v".into());
            a.push(format!("{}k", bitrate_kbps));
            a.push("-crf".into());
            a.push("40".into());
            a.push("-deadline".into());
            a.push("good".into());
            a.push("-cpu-used".into());
            a.push("4".into());
        } else {
            a.push("-c:v".into());
            a.push("libwebp_anim".into());
            a.push("-pix_fmt".into());
            a.push("yuva420p".into());
            a.push("-lossless".into());
            a.push("0".into());
            a.push("-q:v".into());
            a.push(attempt.quality.clamp(1, 100).to_string());
            a.push("-compression_level".into());
            a.push("6".into());
            a.push("-loop".into());
            a.push("0".into());
        }
    } else {
        a.push("-frames:v".into());
        a.push("1".into());
        a.push("-an".into());
        a.push("-c:v".into());
        a.push("libwebp".into());
        a.push("-pix_fmt".into());
        a.push("yuva420p".into());
        a.push("-lossless".into());
        a.push("0".into());
        a.push("-q:v".into());
        a.push(attempt.quality.clamp(1, 100).to_string());
        a.push("-compression_level".into());
        a.push("6".into());
    }
    a.push(output.to_string());
    a
}

/// Duração final: o pedido do usuário, cortado no teto do alvo.
pub fn clamp_duration(wanted: f64, max_seconds: f64) -> f64 {
    let teto = if max_seconds > 0.0 { max_seconds } else { 10.0 };
    if wanted <= 0.0 {
        teto
    } else {
        wanted.min(teto)
    }
}

// ── Entrada e saída ──

fn default_target() -> String {
    "whatsapp".into()
}
fn default_kind() -> String {
    "auto".into()
}
fn default_fit() -> String {
    "contain".into()
}
fn default_padding() -> u32 {
    8
}

#[derive(Debug, Clone, Deserialize)]
pub struct StickerOptions {
    pub inputs: Vec<String>,
    /// "whatsapp" | "telegram"
    #[serde(default = "default_target")]
    pub target: String,
    /// "auto" | "static" | "animated"
    #[serde(default = "default_kind")]
    pub kind: String,
    /// "contain" | "cover"
    #[serde(default = "default_fit")]
    pub fit: String,
    /// Margem em pixels dentro do quadro de 512.
    #[serde(default = "default_padding")]
    pub padding: u32,
    #[serde(default)]
    pub start: f64,
    /// 0 = o máximo que o alvo permite.
    #[serde(default)]
    pub duration: f64,
    /// 0 = o limite oficial do alvo.
    #[serde(default)]
    pub max_kb: u64,
    #[serde(default)]
    pub output_dir: String,
    /// Gera o `pack.json` (e a bandeja) ao lado das figurinhas.
    #[serde(default)]
    pub pack: bool,
    #[serde(default)]
    pub pack_name: String,
    #[serde(default)]
    pub pack_author: String,
    /// Emojis padrão de cada figurinha, separados por espaço.
    #[serde(default)]
    pub emoji: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct StickerItem {
    pub input: String,
    pub output: Option<String>,
    pub animated: bool,
    pub bytes: u64,
    pub max_bytes: u64,
    pub size: u32,
    pub duration_s: f64,
    pub quality: u32,
    pub fps: u32,
    pub tries: u32,
    pub hit_target: bool,
    pub ok: bool,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct StickerResult {
    pub items: Vec<StickerItem>,
    pub target: String,
    /// Caminho do `pack.json`, quando pedido.
    pub pack: Option<String>,
    pub tray: Option<String>,
}

fn out_dir_for(opts: &StickerOptions, input: &Path) -> PathBuf {
    if opts.output_dir.trim().is_empty() {
        input.parent().map(Path::to_path_buf).unwrap_or_default()
    } else {
        PathBuf::from(opts.output_dir.trim())
    }
}

fn wants_animated(opts: &StickerOptions, input: &str) -> bool {
    match opts.kind.as_str() {
        "animated" => true,
        "static" => false,
        _ => is_animated_source(input),
    }
}

async fn source_seconds(input: &Path) -> f64 {
    crate::core::ffmpeg::probe(input)
        .await
        .map(|p| p.duration_seconds)
        .unwrap_or(0.0)
}

async fn convert_one(
    ffmpeg: &Path,
    opts: &StickerOptions,
    input: &str,
    progress: &super::ProgressFn,
) -> anyhow::Result<StickerItem> {
    let inp = Path::new(input);
    if !inp.exists() {
        return Err(anyhow!("arquivo não existe"));
    }
    let animated = wants_animated(opts, input);
    let mut spec = spec(&opts.target, animated);
    if opts.max_kb > 0 {
        spec.max_bytes = opts.max_kb * 1024;
    }

    let mut duration = opts.duration;
    if animated {
        let real = source_seconds(inp).await;
        let sobra = (real - opts.start).max(0.0);
        if sobra > 0.0 {
            duration = if duration > 0.0 {
                duration.min(sobra)
            } else {
                sobra
            };
        }
    }
    let job = Job {
        target: opts.target.clone(),
        animated,
        fit: opts.fit.clone(),
        padding: opts.padding,
        start: opts.start,
        duration,
    };
    let secs = clamp_duration(job.duration, spec.max_seconds);

    let out_dir = out_dir_for(opts, inp);
    std::fs::create_dir_all(&out_dir)?;
    let stem = super::sanitize_name(
        &inp.file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| "figurinha".into()),
    );
    let output = out_dir.join(format!("{}.{}", stem, spec.ext));

    let mut best: Option<(u64, Attempt)> = None;
    let mut tries = 0u32;
    for attempt in attempts(animated) {
        tries += 1;
        let bitrate = plan_bitrate(spec.max_bytes, secs, attempt.quality);
        let args = build_args(
            &job,
            &spec,
            &attempt,
            bitrate,
            &inp.to_string_lossy(),
            &output.to_string_lossy(),
        );
        super::report(
            progress,
            ID,
            "progress",
            tries as u64,
            None,
            Some(format!("{} · q{}", stem, attempt.quality)),
        );
        let out = crate::core::process::command(ffmpeg)
            .args(&args)
            .output()
            .await
            .map_err(|e| anyhow!("ffmpeg nao iniciou: {}", e))?;
        if !out.status.success() {
            let msg = String::from_utf8_lossy(&out.stderr).trim().to_string();
            // Falhar já no primeiro degrau é erro de verdade (codec ausente,
            // arquivo corrompido); nos degraus seguintes já existe um arquivo
            // bom na mão, então vale ficar com ele.
            if best.is_none() {
                return Err(anyhow!("{}", msg));
            }
            tracing::warn!(
                "[wa-sticker] tentativa q{} falhou: {}",
                attempt.quality,
                msg
            );
            break;
        }
        let bytes = std::fs::metadata(&output).map(|m| m.len()).unwrap_or(0);
        best = Some((bytes, attempt));
        if bytes <= spec.max_bytes {
            break;
        }
    }

    let (bytes, attempt) = best.ok_or_else(|| anyhow!("nenhuma tentativa rodou"))?;
    Ok(StickerItem {
        input: input.to_string(),
        output: Some(output.to_string_lossy().to_string()),
        animated,
        bytes,
        max_bytes: spec.max_bytes,
        size: spec.size,
        duration_s: if animated { secs } else { 0.0 },
        quality: attempt.quality,
        fps: attempt.fps,
        tries,
        hit_target: bytes <= spec.max_bytes,
        ok: true,
        error: None,
    })
}

/// Bandeja do pacote: 96×96 PNG, o que o WhatsApp pede.
async fn make_tray(ffmpeg: &Path, source: &str, dir: &Path) -> Option<String> {
    let dest = dir.join("tray.png");
    let out = crate::core::process::command(ffmpeg)
        .args(["-y", "-hide_banner", "-loglevel", "error", "-i", source])
        .args(["-vf", &fit_filter("contain", 96, 4), "-frames:v", "1"])
        .arg(&dest)
        .output()
        .await
        .ok()?;
    out.status
        .success()
        .then(|| dest.to_string_lossy().to_string())
}

/// `pack.json` no formato que os apps de pacote leem (o mesmo `contents.json`
/// do app de exemplo do WhatsApp).
pub fn pack_json(
    name: &str,
    author: &str,
    animated: bool,
    files: &[String],
    emojis: &[String],
) -> serde_json::Value {
    let stickers: Vec<serde_json::Value> = files
        .iter()
        .enumerate()
        .map(|(i, f)| {
            let emoji = emojis
                .get(i)
                .or_else(|| emojis.first())
                .cloned()
                .unwrap_or_else(|| "😀".to_string());
            serde_json::json!({ "image_file": f, "emojis": [emoji] })
        })
        .collect();
    serde_json::json!({
        "android_play_store_link": "",
        "ios_app_store_link": "",
        "sticker_packs": [{
            "identifier": super::sanitize_name(name).to_lowercase().replace(' ', "-"),
            "name": name,
            "publisher": author,
            "tray_image_file": "tray.png",
            "publisher_email": "",
            "publisher_website": "",
            "privacy_policy_website": "",
            "license_agreement_website": "",
            "image_data_version": "1",
            "avoid_cache": false,
            "animated_sticker_pack": animated,
            "stickers": stickers,
        }]
    })
}

pub async fn run(
    opts: StickerOptions,
    progress: super::ProgressFn,
) -> anyhow::Result<StickerResult> {
    let ffmpeg = crate::core::dependencies::ensure_ffmpeg().await?;
    let total = opts.inputs.len() as u64;
    let mut items = Vec::new();
    for (i, input) in opts.inputs.iter().enumerate() {
        super::report(
            &progress,
            ID,
            "progress",
            i as u64,
            Some(total),
            Some(input.clone()),
        );
        match convert_one(&ffmpeg, &opts, input, &progress).await {
            Ok(item) => items.push(item),
            Err(e) => {
                tracing::warn!("[wa-sticker] {}: {}", input, e);
                items.push(StickerItem {
                    input: input.clone(),
                    output: None,
                    animated: wants_animated(&opts, input),
                    bytes: 0,
                    max_bytes: 0,
                    size: 512,
                    duration_s: 0.0,
                    quality: 0,
                    fps: 0,
                    tries: 0,
                    hit_target: false,
                    ok: false,
                    error: Some(e.to_string()),
                });
            }
        }
    }

    let mut pack = None;
    let mut tray = None;
    let feitas: Vec<&StickerItem> = items.iter().filter(|i| i.ok).collect();
    if opts.pack && !feitas.is_empty() {
        let dir = feitas
            .first()
            .and_then(|i| i.output.as_ref())
            .map(|o| {
                Path::new(o)
                    .parent()
                    .map(Path::to_path_buf)
                    .unwrap_or_default()
            })
            .unwrap_or_default();
        let files: Vec<String> = feitas
            .iter()
            .filter_map(|i| i.output.as_ref())
            .map(|o| {
                Path::new(o)
                    .file_name()
                    .map(|f| f.to_string_lossy().to_string())
                    .unwrap_or_default()
            })
            .collect();
        let emojis: Vec<String> = opts.emoji.split_whitespace().map(str::to_string).collect();
        let name = if opts.pack_name.trim().is_empty() {
            "OmniGet".to_string()
        } else {
            opts.pack_name.trim().to_string()
        };
        let author = if opts.pack_author.trim().is_empty() {
            "OmniGet".to_string()
        } else {
            opts.pack_author.trim().to_string()
        };
        if let Some(src) = feitas.first().map(|i| i.input.clone()) {
            tray = make_tray(&ffmpeg, &src, &dir).await;
        }
        let value = pack_json(
            &name,
            &author,
            feitas.iter().any(|i| i.animated),
            &files,
            &emojis,
        );
        let dest = dir.join("pack.json");
        std::fs::write(&dest, serde_json::to_string_pretty(&value)?)?;
        pack = Some(dest.to_string_lossy().to_string());
    }

    super::report(&progress, ID, "done", total, Some(total), None);
    Ok(StickerResult {
        items,
        target: opts.target.clone(),
        pack,
        tray,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_target_carries_its_own_limits() {
        assert_eq!(spec("whatsapp", false).max_bytes, 100 * 1024);
        assert_eq!(spec("whatsapp", true).max_bytes, 500 * 1024);
        assert_eq!(spec("whatsapp", true).max_seconds, 10.0);
        assert_eq!(spec("telegram", false).max_bytes, 512 * 1024);
        let tg = spec("telegram", true);
        assert_eq!(
            (tg.max_bytes, tg.max_seconds, tg.ext),
            (256 * 1024, 3.0, "webm")
        );
        // Alvo desconhecido cai no mais apertado.
        assert_eq!(spec("qualquer", false), spec("whatsapp", false));
    }

    #[test]
    fn padding_never_eats_the_whole_frame() {
        assert_eq!(inner_size(512, 8), 496);
        assert_eq!(inner_size(512, 0), 512);
        assert_eq!(inner_size(512, 9999), 16, "margem absurda para no mínimo");
        assert_eq!(inner_size(512, 7) % 2, 0, "o VP9 exige lado par");
    }

    #[test]
    fn contain_fits_and_cover_crops() {
        let contain = fit_filter("contain", 512, 8);
        assert!(contain.contains("force_original_aspect_ratio=decrease"));
        assert!(!contain.contains("crop="), "contain não corta");
        assert!(contain.contains("scale=496:496"));
        assert!(contain.ends_with("pad=512:512:(ow-iw)/2:(oh-ih)/2:color=#00000000"));

        let cover = fit_filter("cover", 512, 8);
        assert!(cover.contains("force_original_aspect_ratio=increase"));
        assert!(cover.contains("crop=496:496"));
    }

    #[test]
    fn the_chain_always_keeps_alpha() {
        for fit in ["contain", "cover"] {
            let c = fit_filter(fit, 512, 8);
            assert!(c.starts_with("format=rgba,"), "{}", c);
            assert!(c.contains("color=#00000000"), "{}", c);
        }
    }

    #[test]
    fn duration_is_clamped_to_the_target_ceiling() {
        assert_eq!(clamp_duration(0.0, 3.0), 3.0, "0 = o máximo que cabe");
        assert_eq!(clamp_duration(1.5, 3.0), 1.5);
        assert_eq!(clamp_duration(30.0, 3.0), 3.0);
        assert_eq!(clamp_duration(30.0, 10.0), 10.0);
    }

    fn args_de(target: &str, animated: bool, fit: &str) -> Vec<String> {
        let s = spec(target, animated);
        let job = Job {
            target: target.into(),
            animated,
            fit: fit.into(),
            padding: 8,
            start: 1.25,
            duration: 0.0,
        };
        let a = attempts(animated)[0];
        build_args(
            &job,
            &s,
            &a,
            plan_bitrate(s.max_bytes, clamp_duration(0.0, s.max_seconds), a.quality),
            "entrada.mp4",
            "saida",
        )
    }

    fn valor(args: &[String], flag: &str) -> Option<String> {
        args.iter()
            .position(|a| a == flag)
            .and_then(|i| args.get(i + 1))
            .cloned()
    }

    #[test]
    fn static_args_ask_for_one_frame_of_webp() {
        let a = args_de("whatsapp", false, "contain");
        assert_eq!(valor(&a, "-c:v").as_deref(), Some("libwebp"));
        assert_eq!(valor(&a, "-frames:v").as_deref(), Some("1"));
        assert_eq!(valor(&a, "-pix_fmt").as_deref(), Some("yuva420p"));
        assert_eq!(valor(&a, "-q:v").as_deref(), Some("92"));
        assert!(a.contains(&"-an".to_string()));
        assert!(!a.contains(&"-t".to_string()), "estático não tem duração");
        assert!(
            !a.contains(&"-ss".to_string()),
            "estático não busca no tempo"
        );
        assert_eq!(a.last().map(String::as_str), Some("saida"));
    }

    #[test]
    fn whatsapp_animated_is_a_looping_webp() {
        let a = args_de("whatsapp", true, "contain");
        assert_eq!(valor(&a, "-c:v").as_deref(), Some("libwebp_anim"));
        assert_eq!(valor(&a, "-loop").as_deref(), Some("0"));
        assert_eq!(valor(&a, "-t").as_deref(), Some("10.000"));
        assert_eq!(valor(&a, "-ss").as_deref(), Some("1.250"));
        assert!(valor(&a, "-vf").unwrap_or_default().starts_with("fps=20,"));
    }

    #[test]
    fn telegram_animated_is_vp9_webm_with_alpha() {
        let a = args_de("telegram", true, "cover");
        assert_eq!(valor(&a, "-c:v").as_deref(), Some("libvpx-vp9"));
        assert_eq!(valor(&a, "-pix_fmt").as_deref(), Some("yuva420p"));
        assert_eq!(
            valor(&a, "-auto-alt-ref").as_deref(),
            Some("0"),
            "alfa precisa de alt-ref desligado"
        );
        assert_eq!(valor(&a, "-t").as_deref(), Some("3.000"));
        assert!(valor(&a, "-b:v").unwrap_or_default().ends_with('k'));
        assert!(a.contains(&"-an".to_string()));
    }

    #[test]
    fn the_ladder_only_goes_down() {
        for animated in [true, false] {
            let l = attempts(animated);
            assert!(l.len() >= 6);
            for pair in l.windows(2) {
                assert!(
                    pair[0].quality > pair[1].quality,
                    "a escada tem que descer: {:?}",
                    pair
                );
                assert!(
                    pair[0].fps >= pair[1].fps,
                    "o fps não pode subir: {:?}",
                    pair
                );
            }
            assert_eq!(l.iter().all(|a| a.fps == 0), !animated);
        }
    }

    #[test]
    fn bitrate_follows_the_budget() {
        // 256 KB em 3 s dá algo perto de 600 kbps com folga de container.
        let b = plan_bitrate(256 * 1024, 3.0, 75);
        assert!((550..=640).contains(&b), "bitrate fora da conta: {}", b);
        // Metade do tempo, o dobro do bitrate.
        assert!(plan_bitrate(256 * 1024, 1.5, 75) > b);
        // Qualidade menor pede menos bits.
        assert!(plan_bitrate(256 * 1024, 3.0, 30) < b);
        // Nunca zero, nunca absurdo.
        assert_eq!(plan_bitrate(1, 100.0, 75), 24);
        assert_eq!(plan_bitrate(u64::MAX / 8, 0.001, 75), 4000);
    }

    #[test]
    fn auto_reads_the_extension() {
        assert!(is_animated_source("/tmp/clipe.MP4"));
        assert!(is_animated_source("/tmp/meme.gif"));
        assert!(!is_animated_source("/tmp/foto.png"));
        assert!(!is_animated_source("/tmp/sem-extensao"));
    }

    #[test]
    fn the_pack_manifest_has_what_the_apps_read() {
        let v = pack_json(
            "Meu Pack",
            "Tonho",
            true,
            &["a.webp".into(), "b.webp".into()],
            &["😀".into()],
        );
        let pack = &v["sticker_packs"][0];
        assert_eq!(pack["name"], "Meu Pack");
        assert_eq!(pack["publisher"], "Tonho");
        assert_eq!(pack["identifier"], "meu-pack");
        assert_eq!(pack["tray_image_file"], "tray.png");
        assert_eq!(pack["animated_sticker_pack"], true);
        assert_eq!(pack["stickers"].as_array().map(Vec::len), Some(2));
        // Só um emoji foi dado: vale para todas.
        assert_eq!(pack["stickers"][1]["emojis"][0], "😀");
    }

    /// Quadro declarado no cabeçalho WebP: `VP8X` traz o tamanho da tela em
    /// dois inteiros de 24 bits little-endian, menos um.
    fn webp_canvas(bytes: &[u8]) -> Option<(u32, u32)> {
        if bytes.len() < 30 || &bytes[0..4] != b"RIFF" || &bytes[8..12] != b"WEBP" {
            return None;
        }
        if &bytes[12..16] != b"VP8X" {
            return None;
        }
        let u24 = |s: &[u8]| u32::from(s[0]) | (u32::from(s[1]) << 8) | (u32::from(s[2]) << 16);
        Some((u24(&bytes[24..27]) + 1, u24(&bytes[27..30]) + 1))
    }

    /// Gera figurinha de verdade a partir de uma imagem e de um clipe curtos.
    /// `cargo test -p omniget-core --lib -- --ignored live_sticker`
    #[tokio::test]
    #[ignore]
    async fn live_sticker_fits_the_limits() {
        let ffmpeg = crate::core::dependencies::ensure_ffmpeg()
            .await
            .expect("ffmpeg gerido");
        let dir = std::env::temp_dir().join(format!("omniget-sticker-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).expect("tempdir");

        // Fonte estática: PNG com alfa e uma forma colorida.
        let png = dir.join("foto.png");
        let mut img = image::RgbaImage::from_pixel(900, 600, image::Rgba([0, 0, 0, 0]));
        for (x, y, p) in img.enumerate_pixels_mut() {
            if (x as i64 - 450).pow(2) + (y as i64 - 300).pow(2) < 250_000 {
                *p = image::Rgba([(x % 255) as u8, (y % 255) as u8, 200, 255]);
            }
        }
        image::DynamicImage::ImageRgba8(img)
            .save(&png)
            .expect("png");

        // Fonte animada: clipe de 4 s.
        let mp4 = dir.join("clipe.mp4");
        let gen = crate::core::process::command(&ffmpeg)
            .args([
                "-y",
                "-hide_banner",
                "-loglevel",
                "error",
                "-f",
                "lavfi",
                "-i",
                "testsrc2=size=480x360:rate=24:duration=4",
                "-c:v",
                "libx264",
                "-preset",
                "ultrafast",
                "-pix_fmt",
                "yuv420p",
            ])
            .arg(&mp4)
            .output()
            .await
            .expect("gerar clipe");
        assert!(gen.status.success());

        for (target, kind, fonte) in [
            ("whatsapp", "static", &png),
            ("telegram", "static", &png),
            ("whatsapp", "animated", &mp4),
            ("telegram", "animated", &mp4),
        ] {
            let saida = dir.join(format!("{}-{}", target, kind));
            let res = run(
                StickerOptions {
                    inputs: vec![fonte.to_string_lossy().to_string()],
                    target: target.into(),
                    kind: kind.into(),
                    fit: "contain".into(),
                    padding: 8,
                    start: 0.0,
                    duration: 2.0,
                    max_kb: 0,
                    output_dir: saida.to_string_lossy().to_string(),
                    pack: true,
                    pack_name: "Teste".into(),
                    pack_author: "OmniGet".into(),
                    emoji: "😀".into(),
                },
                crate::core::tools::noop_progress(),
            )
            .await
            .expect("run");
            let item = &res.items[0];
            assert!(item.ok, "{} {} falhou: {:?}", target, kind, item.error);
            eprintln!(
                "{} {}: {} bytes (limite {}), q{} fps{} em {} tentativa(s) · {}",
                target,
                kind,
                item.bytes,
                item.max_bytes,
                item.quality,
                item.fps,
                item.tries,
                item.output.clone().unwrap_or_default()
            );
            assert!(
                item.hit_target,
                "{} {} ficou com {} bytes, acima do limite de {}",
                target, kind, item.bytes, item.max_bytes
            );
            let saida_path = item.output.clone().expect("saída");
            // O ffprobe não sabe dizer o tamanho de um WebP animado; nesse
            // caso o quadro sai do cabeçalho VP8X, que é a fonte da verdade.
            let dims = if saida_path.ends_with(".webp") {
                webp_canvas(&std::fs::read(&saida_path).expect("ler webp"))
            } else {
                let probe = crate::core::ffmpeg::probe(Path::new(&saida_path))
                    .await
                    .expect("probe da figurinha");
                probe
                    .streams
                    .iter()
                    .find(|s| s.codec_type == "video")
                    .and_then(|v| v.width.zip(v.height))
            };
            assert_eq!(
                dims,
                Some((512, 512)),
                "{} {} não saiu 512×512",
                target,
                kind
            );
            assert!(res.pack.is_some(), "pack.json não saiu");
        }
        eprintln!("saída em {}", dir.display());
    }
}

//! Remover o fundo de uma imagem (pessoa, produto, o que for), uma ou em lote.
//!
//! Roda **local**: nenhuma imagem sai da máquina. A segmentação é um modelo
//! ONNX gerido (`core/tools/onnx.rs`) rodando na runtime gerida
//! (`core/onnxrt.rs`). O pré e o pós-processamento seguem o que o rembg
//! (danielgatis/rembg, MIT) faz com os mesmos pesos — o mesmo `resize` para a
//! entrada do modelo, a mesma normalização por média/desvio, e a máscara de
//! saída normalizada por mín-máx antes de virar alfa.
//!
//! O que sai é um PNG RGBA com o fundo transparente, ou um JPEG/PNG sobre uma
//! cor sólida escolhida pelo usuário. A máscara sozinha também pode ser salva.

use std::path::{Path, PathBuf};

use anyhow::{anyhow, Context};
use image::{imageops::FilterType, DynamicImage, GrayImage, Rgba, RgbaImage};
use ndarray::Array4;
use serde::{Deserialize, Serialize};

use super::ProgressFn;

const TOOL_ID: &str = "img-bg";
const DEFAULT_SUFFIX: &str = "-nobg";

/// Pré-processamento de cada modelo de segmentação. Vem do rembg: cada família
/// de pesos foi treinada com um tamanho de entrada e uma normalização próprios,
/// e trocar isso estraga a máscara sem dar erro nenhum.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BgParams {
    pub model_id: &'static str,
    pub size: u32,
    pub mean: [f32; 3],
    pub std: [f32; 3],
}

const IMAGENET_MEAN: [f32; 3] = [0.485, 0.456, 0.406];
const IMAGENET_STD: [f32; 3] = [0.229, 0.224, 0.225];

const PARAMS: &[BgParams] = &[
    BgParams {
        model_id: "u2netp",
        size: 320,
        mean: IMAGENET_MEAN,
        std: IMAGENET_STD,
    },
    BgParams {
        model_id: "u2net",
        size: 320,
        mean: IMAGENET_MEAN,
        std: IMAGENET_STD,
    },
    BgParams {
        model_id: "silueta",
        size: 320,
        mean: IMAGENET_MEAN,
        std: IMAGENET_STD,
    },
    BgParams {
        model_id: "isnet-general-use",
        size: 1024,
        mean: [0.5, 0.5, 0.5],
        std: [1.0, 1.0, 1.0],
    },
];

pub fn params_for(model_id: &str) -> Option<&'static BgParams> {
    PARAMS.iter().find(|p| p.model_id == model_id)
}

/// Modelo padrão: o menor da família, para o primeiro uso não pedir 175 MB.
pub fn default_model() -> &'static str {
    "u2netp"
}

#[derive(Debug, Clone, Deserialize)]
pub struct BgOptions {
    /// Arquivos escolhidos um a um.
    #[serde(default)]
    pub inputs: Vec<String>,
    /// Pasta inteira (não entra em subpastas).
    #[serde(default)]
    pub input_dir: String,
    /// Id do modelo do catálogo ONNX. Vazio = o padrão.
    #[serde(default)]
    pub model: String,
    /// Vazio = mesma pasta do arquivo de entrada.
    #[serde(default)]
    pub output_dir: String,
    /// Vazio = `-nobg`.
    #[serde(default)]
    pub suffix: String,
    /// Vazio = fundo transparente. `#RRGGBB` ou `#RRGGBBAA` = cor sólida.
    #[serde(default)]
    pub background: String,
    /// "png" | "jpg"
    #[serde(default = "default_format")]
    pub format: String,
    #[serde(default = "default_quality")]
    pub quality: u8,
    /// Alfa abaixo disso vira zero, para matar a poeira do fundo. 0 = desligado.
    #[serde(default)]
    pub alpha_threshold: u8,
    /// Borda dura: alfa vira 0 ou 255. Bom para logotipo, ruim para cabelo.
    #[serde(default)]
    pub hard_edges: bool,
    /// Salva só a máscara em tons de cinza, sem cortar a imagem.
    #[serde(default)]
    pub mask_only: bool,
}

fn default_format() -> String {
    "png".into()
}
fn default_quality() -> u8 {
    92
}

#[derive(Debug, Clone, Serialize)]
pub struct BgItem {
    pub input: String,
    pub output: Option<String>,
    pub width: u32,
    pub height: u32,
    pub bytes: u64,
    pub ok: bool,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct BgResult {
    pub model: String,
    pub items: Vec<BgItem>,
    pub done: usize,
    pub failed: usize,
}

// ── Peças puras (testadas sem modelo nem runtime) ──────────────────────

/// `#RGB`, `#RRGGBB` ou `#RRGGBBAA`. Vazio devolve `None` (= transparente).
pub fn parse_hex_color(raw: &str) -> anyhow::Result<Option<Rgba<u8>>> {
    let s = raw.trim().trim_start_matches('#');
    if s.is_empty() {
        return Ok(None);
    }
    let byte = |i: usize| -> anyhow::Result<u8> {
        u8::from_str_radix(&s[i..i + 2], 16).map_err(|_| anyhow!("cor inválida: {raw}"))
    };
    let px = match s.len() {
        3 => {
            let mut v = [0u8; 3];
            for (i, c) in s.chars().enumerate() {
                let d = c
                    .to_digit(16)
                    .ok_or_else(|| anyhow!("cor inválida: {raw}"))? as u8;
                v[i] = d * 17;
            }
            Rgba([v[0], v[1], v[2], 255])
        }
        6 => Rgba([byte(0)?, byte(2)?, byte(4)?, 255]),
        8 => Rgba([byte(0)?, byte(2)?, byte(4)?, byte(6)?]),
        _ => return Err(anyhow!("cor inválida: {raw}")),
    };
    Ok(Some(px))
}

/// Imagem → tensor `NCHW` normalizado, do jeito que os pesos esperam.
///
/// O rembg divide o array pelo **maior valor do próprio array** e só depois
/// aplica média/desvio; manter isso é o que faz a máscara sair igual à dele.
pub fn normalize_input(img: &DynamicImage, p: &BgParams) -> Array4<f32> {
    let size = p.size as usize;
    let rgb = image::imageops::resize(&img.to_rgb8(), p.size, p.size, FilterType::Lanczos3);
    let max = rgb.iter().copied().max().unwrap_or(0) as f32;
    let scale = 1.0 / max.max(1e-6);
    let mut t = Array4::<f32>::zeros((1, 3, size, size));
    for y in 0..size {
        for x in 0..size {
            let px = rgb.get_pixel(x as u32, y as u32).0;
            for c in 0..3 {
                t[[0, c, y, x]] = (px[c] as f32 * scale - p.mean[c]) / p.std[c];
            }
        }
    }
    t
}

/// Saída bruta do modelo → máscara em tons de cinza, normalizada por mín-máx.
/// Saída constante (imagem toda fundo ou toda objeto) vira máscara zerada em
/// vez de dividir por zero.
pub fn mask_from_raw(raw: &[f32], w: u32, h: u32) -> anyhow::Result<GrayImage> {
    let n = (w as usize) * (h as usize);
    if raw.len() < n {
        return Err(anyhow!(
            "a saída do modelo tem {} valores, esperava {}",
            raw.len(),
            n
        ));
    }
    let slice = &raw[..n];
    let mut mi = f32::INFINITY;
    let mut ma = f32::NEG_INFINITY;
    for v in slice {
        if v.is_finite() {
            mi = mi.min(*v);
            ma = ma.max(*v);
        }
    }
    let span = ma - mi;
    let buf: Vec<u8> = if !span.is_finite() || span <= f32::EPSILON {
        vec![0u8; n]
    } else {
        slice
            .iter()
            .map(|v| (((v - mi) / span) * 255.0).round().clamp(0.0, 255.0) as u8)
            .collect()
    };
    GrayImage::from_raw(w, h, buf).ok_or_else(|| anyhow!("máscara com tamanho inconsistente"))
}

/// Limpeza opcional da máscara: corte de alfa fraco e/ou borda dura.
pub fn clean_mask(mask: &GrayImage, alpha_threshold: u8, hard_edges: bool) -> GrayImage {
    let mut out = mask.clone();
    for px in out.pixels_mut() {
        let mut v = px.0[0];
        if alpha_threshold > 0 && v < alpha_threshold {
            v = 0;
        }
        if hard_edges {
            v = if v >= 128 { 255 } else { 0 };
        }
        px.0[0] = v;
    }
    out
}

/// Aplica a máscara como alfa e, se houver cor, compõe sobre ela.
///
/// O alfa da imagem original é preservado (multiplicado pela máscara), então
/// um PNG que já tinha transparência não a perde.
pub fn compose(base: &RgbaImage, mask: &GrayImage, background: Option<Rgba<u8>>) -> RgbaImage {
    let (w, h) = base.dimensions();
    let mut out = RgbaImage::new(w, h);
    for y in 0..h {
        for x in 0..w {
            let src = base.get_pixel(x, y).0;
            let m = mask.get_pixel(x, y).0[0] as u32;
            let a = ((src[3] as u32 * m) / 255) as u8;
            out.put_pixel(x, y, Rgba([src[0], src[1], src[2], a]));
        }
    }
    match background {
        None => out,
        Some(bg) => {
            let mut flat = RgbaImage::from_pixel(w, h, bg);
            for y in 0..h {
                for x in 0..w {
                    let fg = out.get_pixel(x, y).0;
                    let a = fg[3] as u32;
                    let dst = flat.get_pixel(x, y).0;
                    let mix = |f: u8, b: u8| ((f as u32 * a + b as u32 * (255 - a)) / 255) as u8;
                    let alpha = (a + dst[3] as u32 * (255 - a) / 255).min(255) as u8;
                    flat.put_pixel(
                        x,
                        y,
                        Rgba([
                            mix(fg[0], dst[0]),
                            mix(fg[1], dst[1]),
                            mix(fg[2], dst[2]),
                            alpha,
                        ]),
                    );
                }
            }
            flat
        }
    }
}

fn is_image_file(path: &Path) -> bool {
    const EXT: [&str; 8] = ["png", "jpg", "jpeg", "webp", "bmp", "gif", "tif", "tiff"];
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| EXT.contains(&e.to_ascii_lowercase().as_str()))
        .unwrap_or(false)
}

/// Junta arquivos escolhidos e pasta, sem repetir e em ordem estável.
pub fn collect_inputs(inputs: &[String], input_dir: &str) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();
    for raw in inputs {
        let p = PathBuf::from(raw.trim());
        if p.is_file() {
            out.push(p);
        }
    }
    let dir = input_dir.trim();
    if !dir.is_empty() {
        if let Ok(rd) = std::fs::read_dir(dir) {
            let mut found: Vec<PathBuf> = rd
                .flatten()
                .map(|e| e.path())
                .filter(|p| p.is_file() && is_image_file(p))
                .collect();
            found.sort();
            out.extend(found);
        }
    }
    out.sort();
    out.dedup();
    out
}

/// Caminho de saída de um arquivo, respeitando pasta, sufixo e formato.
pub fn output_path(input: &Path, output_dir: &str, suffix: &str, ext: &str) -> PathBuf {
    let dir = if output_dir.trim().is_empty() {
        input.parent().map(|p| p.to_path_buf()).unwrap_or_default()
    } else {
        PathBuf::from(output_dir.trim())
    };
    let stem = input
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "imagem".into());
    let suffix = if suffix.is_empty() {
        DEFAULT_SUFFIX
    } else {
        suffix
    };
    dir.join(format!("{stem}{suffix}.{ext}"))
}

/// "jpg"/"jpeg" viram JPEG; qualquer outra coisa vira PNG. JPEG não guarda
/// alfa, então sem cor de fundo o corte sairia sobre preto — nesse caso a
/// escolha é ignorada e o arquivo sai em PNG.
pub fn resolve_format(format: &str, has_background: bool, mask_only: bool) -> &'static str {
    let f = format.trim().to_ascii_lowercase();
    let wants_jpeg = f == "jpg" || f == "jpeg";
    if wants_jpeg && (has_background || mask_only) {
        "jpg"
    } else {
        "png"
    }
}

fn save_image(img: &DynamicImage, dest: &Path, ext: &str, quality: u8) -> anyhow::Result<u64> {
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent).with_context(|| format!("criando {}", parent.display()))?;
    }
    if ext == "jpg" {
        let file =
            std::fs::File::create(dest).with_context(|| format!("criando {}", dest.display()))?;
        let mut w = std::io::BufWriter::new(file);
        let mut enc =
            image::codecs::jpeg::JpegEncoder::new_with_quality(&mut w, quality.clamp(1, 100));
        enc.encode_image(&img.to_rgb8())
            .map_err(|e| anyhow!("não gravei o JPEG {}: {}", dest.display(), e))?;
    } else {
        img.save_with_format(dest, image::ImageFormat::Png)
            .map_err(|e| anyhow!("não gravei o PNG {}: {}", dest.display(), e))?;
    }
    Ok(std::fs::metadata(dest).map(|m| m.len()).unwrap_or(0))
}

// ── Execução ───────────────────────────────────────────────────────────

/// Roda a inferência num arquivo já aberto e devolve a máscara no tamanho
/// original da imagem.
fn mask_for_image(
    session: &mut ort::session::Session,
    img: &DynamicImage,
    p: &BgParams,
) -> anyhow::Result<GrayImage> {
    let side = p.size as i64;
    // O `ort` traz um `ndarray` próprio (0.17) e o crate usa o 0.16, então o
    // tensor atravessa a fronteira como forma + dados contíguos, que é o que o
    // `Tensor::from_array` aceita sem depender de versão de crate nenhuma.
    let (data, _) = normalize_input(img, p).into_raw_vec_and_offset();
    let tensor = ort::value::Tensor::from_array((vec![1, 3, side, side], data))
        .map_err(|e| anyhow!("não montei o tensor de entrada: {e}"))?;
    let outputs = session
        .run(ort::inputs![tensor])
        .map_err(|e| anyhow!("a inferência falhou: {e}"))?;
    let (shape, raw) = outputs[0]
        .try_extract_tensor::<f32>()
        .map_err(|e| anyhow!("não li a saída do modelo: {e}"))?;
    if shape.len() < 2 {
        return Err(anyhow!(
            "saída do modelo com formato inesperado: {:?}",
            &shape[..]
        ));
    }
    let h = shape[shape.len() - 2].max(0) as u32;
    let w = shape[shape.len() - 1].max(0) as u32;
    let small = mask_from_raw(raw, w, h)?;
    let (ow, oh) = (img.width(), img.height());
    Ok(image::imageops::resize(
        &small,
        ow,
        oh,
        FilterType::Lanczos3,
    ))
}

fn run_blocking(
    opts: &BgOptions,
    model_id: &str,
    progress: &ProgressFn,
) -> anyhow::Result<BgResult> {
    let params = *params_for(model_id)
        .ok_or_else(|| anyhow!("o modelo {model_id} não serve para remover fundo"))?;
    let files = collect_inputs(&opts.inputs, &opts.input_dir);
    if files.is_empty() {
        return Err(anyhow!("escolha ao menos uma imagem"));
    }
    let background = parse_hex_color(&opts.background)?;
    let ext = resolve_format(&opts.format, background.is_some(), opts.mask_only);

    let mut session = super::onnx::session_for(model_id)?;
    let total = files.len() as u64;
    let mut items: Vec<BgItem> = Vec::with_capacity(files.len());
    let mut failed = 0usize;

    for (i, path) in files.iter().enumerate() {
        super::report(
            progress,
            TOOL_ID,
            "progress",
            i as u64,
            Some(total),
            Some(path.to_string_lossy().to_string()),
        );
        let one = (|| -> anyhow::Result<(PathBuf, u32, u32, u64)> {
            let img = image::open(path).with_context(|| format!("abrindo {}", path.display()))?;
            let mask = mask_for_image(&mut session, &img, &params)?;
            let mask = clean_mask(&mask, opts.alpha_threshold, opts.hard_edges);
            let dest = output_path(path, &opts.output_dir, &opts.suffix, ext);
            let out = if opts.mask_only {
                DynamicImage::ImageLuma8(mask)
            } else {
                DynamicImage::ImageRgba8(compose(&img.to_rgba8(), &mask, background))
            };
            let bytes = save_image(&out, &dest, ext, opts.quality)?;
            Ok((dest, out.width(), out.height(), bytes))
        })();
        match one {
            Ok((dest, w, h, bytes)) => items.push(BgItem {
                input: path.to_string_lossy().to_string(),
                output: Some(dest.to_string_lossy().to_string()),
                width: w,
                height: h,
                bytes,
                ok: true,
                error: None,
            }),
            Err(e) => {
                failed += 1;
                tracing::warn!("[img-bg] {} falhou: {}", path.display(), e);
                items.push(BgItem {
                    input: path.to_string_lossy().to_string(),
                    output: None,
                    width: 0,
                    height: 0,
                    bytes: 0,
                    ok: false,
                    error: Some(e.to_string()),
                });
            }
        }
    }

    super::report(progress, TOOL_ID, "done", total, Some(total), None);
    Ok(BgResult {
        model: model_id.to_string(),
        done: items.len() - failed,
        failed,
        items,
    })
}

/// Ponto de entrada da tool. Garante runtime e modelo antes de qualquer
/// inferência, e só então joga o trabalho pesado para uma thread de bloqueio.
pub async fn run(opts: BgOptions, progress: ProgressFn) -> anyhow::Result<BgResult> {
    let model_id = if opts.model.trim().is_empty() {
        default_model().to_string()
    } else {
        opts.model.trim().to_string()
    };
    if params_for(&model_id).is_none() {
        return Err(anyhow!("o modelo {model_id} não serve para remover fundo"));
    }
    // Erro acionável antes de baixar 180 MB à toa.
    crate::core::onnxrt::init()?;
    super::onnx::ensure_model(&model_id, &progress).await?;

    let p = progress.clone();
    tokio::task::spawn_blocking(move || run_blocking(&opts, &model_id, &p))
        .await
        .map_err(|e| anyhow!("tarefa de remoção de fundo falhou: {e}"))?
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gray(w: u32, h: u32, v: u8) -> GrayImage {
        GrayImage::from_pixel(w, h, image::Luma([v]))
    }

    #[test]
    fn cada_modelo_do_catalogo_rembg_tem_pre_processamento() {
        for m in super::super::onnx::family("rembg") {
            let p = params_for(m.id)
                .unwrap_or_else(|| panic!("modelo {} sem parâmetros de entrada", m.id));
            assert!(p.size >= 320);
            assert!(p.std.iter().all(|s| *s > 0.0), "desvio zero em {}", m.id);
        }
    }

    #[test]
    fn o_modelo_padrao_existe_no_catalogo_e_tem_parametros() {
        assert!(super::super::onnx::find(default_model()).is_some());
        assert!(params_for(default_model()).is_some());
        assert!(params_for("nao-existe").is_none());
    }

    #[test]
    fn cores_em_hex_de_tres_seis_e_oito_digitos() {
        assert_eq!(parse_hex_color("").expect("vazio"), None);
        assert_eq!(parse_hex_color("   ").expect("espaço"), None);
        assert_eq!(
            parse_hex_color("#FF8800").expect("6"),
            Some(Rgba([255, 136, 0, 255]))
        );
        assert_eq!(
            parse_hex_color("f80").expect("3"),
            Some(Rgba([255, 136, 0, 255]))
        );
        assert_eq!(
            parse_hex_color("#0080FF40").expect("8"),
            Some(Rgba([0, 128, 255, 64]))
        );
        assert!(parse_hex_color("#12345").is_err());
        assert!(parse_hex_color("#GGGGGG").is_err());
    }

    #[test]
    fn a_normalizacao_da_o_formato_nchw_e_desfaz_media_e_desvio() {
        let p = BgParams {
            model_id: "teste",
            size: 4,
            mean: [0.5, 0.5, 0.5],
            std: [1.0, 1.0, 1.0],
        };
        // Imagem branca: o maior valor é 255, então tudo vira 1.0 antes da média.
        let img = DynamicImage::ImageRgb8(image::RgbImage::from_pixel(
            8,
            8,
            image::Rgb([255, 255, 255]),
        ));
        let t = normalize_input(&img, &p);
        assert_eq!(t.shape(), &[1, 3, 4, 4]);
        for v in t.iter() {
            assert!((v - 0.5).abs() < 1e-5, "esperava 0.5, veio {v}");
        }
    }

    #[test]
    fn a_normalizacao_separa_os_canais_na_ordem_certa() {
        let p = BgParams {
            model_id: "teste",
            size: 2,
            mean: [0.0, 0.0, 0.0],
            std: [1.0, 1.0, 1.0],
        };
        let img =
            DynamicImage::ImageRgb8(image::RgbImage::from_pixel(4, 4, image::Rgb([255, 0, 0])));
        let t = normalize_input(&img, &p);
        assert!((t[[0, 0, 0, 0]] - 1.0).abs() < 1e-5, "canal R");
        assert!(t[[0, 1, 0, 0]].abs() < 1e-5, "canal G");
        assert!(t[[0, 2, 0, 0]].abs() < 1e-5, "canal B");
    }

    #[test]
    fn a_mascara_normaliza_por_min_e_max() {
        let raw = vec![-1.0f32, 0.0, 1.0, 3.0];
        let m = mask_from_raw(&raw, 2, 2).expect("máscara");
        assert_eq!(m.get_pixel(0, 0).0[0], 0);
        assert_eq!(m.get_pixel(1, 1).0[0], 255);
        // 0.0 está a 1/4 do caminho entre -1 e 3.
        assert_eq!(m.get_pixel(1, 0).0[0], 64);
    }

    #[test]
    fn saida_constante_vira_mascara_zerada_em_vez_de_dividir_por_zero() {
        let m = mask_from_raw(&[0.7f32; 4], 2, 2).expect("máscara");
        assert!(m.pixels().all(|p| p.0[0] == 0));
    }

    #[test]
    fn saida_curta_demais_da_erro() {
        assert!(mask_from_raw(&[0.1f32, 0.2], 2, 2).is_err());
    }

    #[test]
    fn a_mascara_ignora_valores_a_mais_do_batch() {
        // (1, 2, 2, 2): só o primeiro canal interessa.
        let raw = vec![0.0f32, 1.0, 0.0, 1.0, 9.0, 9.0, 9.0, 9.0];
        let m = mask_from_raw(&raw, 2, 2).expect("máscara");
        assert_eq!(m.get_pixel(0, 0).0[0], 0);
        assert_eq!(m.get_pixel(1, 0).0[0], 255);
    }

    #[test]
    fn corte_de_alfa_fraco_e_borda_dura() {
        let mut m = gray(3, 1, 0);
        m.put_pixel(0, 0, image::Luma([10]));
        m.put_pixel(1, 0, image::Luma([130]));
        m.put_pixel(2, 0, image::Luma([250]));

        let corte = clean_mask(&m, 32, false);
        assert_eq!(corte.get_pixel(0, 0).0[0], 0);
        assert_eq!(corte.get_pixel(1, 0).0[0], 130);

        let dura = clean_mask(&m, 0, true);
        assert_eq!(dura.get_pixel(0, 0).0[0], 0);
        assert_eq!(dura.get_pixel(1, 0).0[0], 255);
        assert_eq!(dura.get_pixel(2, 0).0[0], 255);
    }

    #[test]
    fn a_composicao_transparente_apaga_o_fundo_e_mantem_o_objeto() {
        let base = RgbaImage::from_pixel(2, 1, Rgba([10, 20, 30, 255]));
        let mut mask = gray(2, 1, 255);
        mask.put_pixel(1, 0, image::Luma([0]));
        let out = compose(&base, &mask, None);
        assert_eq!(out.get_pixel(0, 0).0, [10, 20, 30, 255]);
        assert_eq!(
            out.get_pixel(1, 0).0[3],
            0,
            "fundo tinha que ficar invisível"
        );
    }

    #[test]
    fn a_composicao_respeita_o_alfa_que_a_imagem_ja_tinha() {
        let base = RgbaImage::from_pixel(1, 1, Rgba([10, 20, 30, 128]));
        let out = compose(&base, &gray(1, 1, 255), None);
        assert_eq!(out.get_pixel(0, 0).0[3], 128);
    }

    #[test]
    fn com_cor_de_fundo_o_resultado_fica_opaco_e_meio_a_meio_na_borda() {
        let base = RgbaImage::from_pixel(3, 1, Rgba([0, 0, 0, 255]));
        let mut mask = gray(3, 1, 255);
        mask.put_pixel(1, 0, image::Luma([128]));
        mask.put_pixel(2, 0, image::Luma([0]));
        let out = compose(&base, &mask, Some(Rgba([255, 255, 255, 255])));
        assert_eq!(out.get_pixel(0, 0).0, [0, 0, 0, 255], "objeto puro");
        assert_eq!(out.get_pixel(2, 0).0, [255, 255, 255, 255], "fundo puro");
        let meio = out.get_pixel(1, 0).0;
        assert_eq!(meio[3], 255);
        assert!(
            (100..=160).contains(&meio[0]),
            "borda tinha que misturar, veio {meio:?}"
        );
    }

    #[test]
    fn jpeg_so_quando_nao_precisa_de_alfa() {
        assert_eq!(resolve_format("jpg", true, false), "jpg");
        assert_eq!(resolve_format("jpeg", false, true), "jpg");
        assert_eq!(resolve_format("jpg", false, false), "png");
        assert_eq!(resolve_format("png", true, false), "png");
        assert_eq!(resolve_format("", false, false), "png");
    }

    #[test]
    fn o_nome_de_saida_usa_sufixo_pasta_e_extensao() {
        let inp = PathBuf::from("/tmp/fotos/gato.jpeg");
        let a = output_path(&inp, "", "", "png");
        assert_eq!(a, PathBuf::from("/tmp/fotos/gato-nobg.png"));
        let b = output_path(&inp, "/saida", "_corte", "jpg");
        assert_eq!(b, PathBuf::from("/saida/gato_corte.jpg"));
    }

    #[test]
    fn a_lista_de_entrada_junta_pasta_e_arquivos_sem_repetir() {
        let dir = std::env::temp_dir().join(format!("omniget-imgbg-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("pasta de teste");
        let a = dir.join("a.png");
        let b = dir.join("b.jpg");
        let txt = dir.join("leia.txt");
        image::RgbaImage::from_pixel(2, 2, Rgba([1, 2, 3, 255]))
            .save(&a)
            .expect("png de teste");
        image::RgbImage::from_pixel(2, 2, image::Rgb([1, 2, 3]))
            .save(&b)
            .expect("jpg de teste");
        std::fs::write(&txt, "nada").expect("txt de teste");

        let got = collect_inputs(&[a.to_string_lossy().to_string()], &dir.to_string_lossy());
        assert_eq!(got, vec![a.clone(), b.clone()], "sem repetir e sem o .txt");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn sem_entrada_nenhuma_a_lista_fica_vazia() {
        assert!(collect_inputs(&[], "").is_empty());
        assert!(collect_inputs(&["/nao/existe.png".into()], "/nao/existe/").is_empty());
    }

    /// Precisa de rede e do runtime instalado: baixa o u2netp, roda numa imagem
    /// sintética (quadrado claro no meio de um fundo escuro) e confere que
    /// saiu um PNG RGBA do tamanho certo.
    #[test]
    #[ignore = "rede + runtime: baixa u2netp e remove o fundo de uma imagem sintetica"]
    fn remove_o_fundo_de_verdade_com_o_u2netp() {
        let dir = std::env::temp_dir().join("omniget-imgbg-live");
        std::fs::create_dir_all(&dir).expect("pasta");
        let src = dir.join("alvo.png");
        let mut img = image::RgbImage::from_pixel(256, 256, image::Rgb([12, 14, 18]));
        for y in 80..176 {
            for x in 80..176 {
                img.put_pixel(x, y, image::Rgb([240, 230, 210]));
            }
        }
        img.save(&src).expect("imagem de teste");

        let rt = tokio::runtime::Runtime::new().expect("runtime tokio");
        let opts = BgOptions {
            inputs: vec![src.to_string_lossy().to_string()],
            input_dir: String::new(),
            model: "u2netp".into(),
            output_dir: dir.to_string_lossy().to_string(),
            suffix: String::new(),
            background: String::new(),
            format: "png".into(),
            quality: 92,
            alpha_threshold: 0,
            hard_edges: false,
            mask_only: false,
        };
        let res = rt
            .block_on(run(opts, super::super::noop_progress()))
            .expect("remoção de fundo");
        assert_eq!(res.failed, 0, "itens: {:?}", res.items);
        let out = res.items[0].output.clone().expect("saída");
        let saved = image::open(&out).expect("abrir saída").to_rgba8();
        assert_eq!(saved.dimensions(), (256, 256));
        // O quadrado claro do meio é o objeto; o canto é fundo. Se a
        // normalização estivesse com os eixos ou os canais trocados, a máscara
        // sairia invertida ou embaralhada e uma destas duas falharia.
        assert!(
            saved.get_pixel(128, 128).0[3] > 200,
            "o objeto do meio devia ter ficado opaco"
        );
        assert!(
            saved.get_pixel(4, 4).0[3] < 64,
            "o canto era fundo e devia ter ficado transparente"
        );
    }
}

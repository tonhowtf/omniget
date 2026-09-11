//! Costurar prints numa imagem longa.
//!
//! O modo simples empilha os arquivos com alinhamento, espaçamento e cor de
//! fundo. O modo inteligente é o que dá valor: ele descobre quanto um print
//! repete do anterior — a conversa que rolou meia tela, a página que voltou
//! um pouco — e costura sem repetir conteúdo.
//!
//! A busca é por deslocamento puro: para cada par testamos as sobreposições
//! possíveis e ficamos com a que tem a menor diferença média entre as faixas.
//! Se nem a melhor chega no limiar, aquela junta vira empilhamento normal e o
//! resultado diz isso, junta a junta, para a UI mostrar o que aconteceu.

use std::io::Cursor;
use std::path::{Path, PathBuf};

use anyhow::anyhow;
use image::{imageops, DynamicImage, RgbaImage};
use serde::{Deserialize, Serialize};

use super::ProgressFn;

/// Faixa mínima aceita como sobreposição. Abaixo disso qualquer print casa
/// com qualquer outro por acaso.
const MIN_OVERLAP: u32 = 8;
/// Linhas e colunas amostradas por deslocamento testado. Comparar a faixa
/// inteira a cada candidato custa caro e não muda o vencedor.
const ROW_SAMPLES: u32 = 48;
const COL_SAMPLES: u32 = 128;

#[derive(Debug, Clone, Deserialize)]
pub struct StitchOptions {
    pub inputs: Vec<String>,
    /// "vertical" (padrão) ou "horizontal".
    #[serde(default = "default_direction")]
    pub direction: String,
    /// "start" | "center" | "end" — o eixo curto, quando os prints têm
    /// tamanhos diferentes.
    #[serde(default = "default_align")]
    pub align: String,
    /// Espaço entre prints, em px. Só vale nas juntas empilhadas.
    #[serde(default)]
    pub spacing: u32,
    #[serde(default = "default_bg")]
    pub background: String,
    /// Liga a detecção de sobreposição.
    #[serde(default)]
    pub smart: bool,
    /// Maior sobreposição procurada, em px. 0 = procurar até onde der.
    #[serde(default)]
    pub search: u32,
    /// Diferença média por canal (0-255) ainda aceita como "é a mesma faixa".
    #[serde(default = "default_tolerance")]
    pub tolerance: f32,
    /// Barra fixa no topo do print seguinte, ignorada na comparação e
    /// recortada na costura.
    #[serde(default)]
    pub crop_top: u32,
    /// Barra fixa no rodapé do print anterior, ignorada na comparação.
    #[serde(default)]
    pub crop_bottom: u32,
    /// Maior largura da imagem final; 0 = manter.
    #[serde(default)]
    pub max_width: u32,
    /// "png" (padrão) ou "jpeg".
    #[serde(default = "default_format")]
    pub format: String,
    #[serde(default = "default_quality")]
    pub quality: u8,
    /// Caminho de saída; vazio = ao lado do primeiro print.
    #[serde(default)]
    pub output: String,
}

fn default_direction() -> String {
    "vertical".into()
}
fn default_align() -> String {
    "start".into()
}
fn default_bg() -> String {
    "#FFFFFF".into()
}
fn default_tolerance() -> f32 {
    6.0
}
fn default_format() -> String {
    "png".into()
}
fn default_quality() -> u8 {
    90
}

/// Uma junta entre dois prints consecutivos.
#[derive(Debug, Clone, Serialize)]
pub struct StitchSeam {
    /// Índice do print de baixo (a junta é entre `index - 1` e `index`).
    pub index: u32,
    /// Quantos px de conteúdo repetido foram descartados.
    pub overlap: u32,
    /// Diferença média por canal na faixa escolhida (menor é melhor).
    pub score: f32,
    /// Falso quando a junta caiu para empilhamento puro.
    pub matched: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct StitchResult {
    pub output: String,
    pub width: u32,
    pub height: u32,
    pub bytes: u64,
    pub count: u32,
    /// Total de px de repetição removidos.
    pub saved_px: u32,
    pub seams: Vec<StitchSeam>,
}

/// O que a busca de sobreposição precisa saber, sem arrastar as opções todas.
#[derive(Debug, Clone, Copy)]
pub struct MatchParams {
    pub search: u32,
    pub tolerance: f32,
    pub crop_top: u32,
    pub crop_bottom: u32,
}

impl From<&StitchOptions> for MatchParams {
    fn from(o: &StitchOptions) -> Self {
        MatchParams {
            search: o.search,
            tolerance: o.tolerance,
            crop_top: o.crop_top,
            crop_bottom: o.crop_bottom,
        }
    }
}

/// Diferença média por canal entre a faixa de baixo de `a` e a de cima de `b`
/// para uma sobreposição de `overlap` linhas.
fn band_diff(a: &RgbaImage, b: &RgbaImage, overlap: u32, a_bottom: u32, b_top: u32) -> f32 {
    let w = a.width().min(b.width());
    if w == 0 || overlap == 0 {
        return f32::MAX;
    }
    let a_start = a.height().saturating_sub(a_bottom + overlap);
    let col_step = (w / COL_SAMPLES).max(1);
    let row_step = (overlap / ROW_SAMPLES).max(1);
    let mut sum = 0f64;
    let mut n = 0u64;
    let mut dy = 0;
    while dy < overlap {
        let ay = a_start + dy;
        let by = b_top + dy;
        if ay >= a.height() || by >= b.height() {
            break;
        }
        let mut x = 0;
        while x < w {
            let pa = a.get_pixel(x, ay).0;
            let pb = b.get_pixel(x, by).0;
            sum += (pa[0].abs_diff(pb[0]) as u32
                + pa[1].abs_diff(pb[1]) as u32
                + pa[2].abs_diff(pb[2]) as u32) as f64;
            n += 3;
            x += col_step;
        }
        dy += row_step;
    }
    if n == 0 {
        f32::MAX
    } else {
        (sum / n as f64) as f32
    }
}

/// Melhor sobreposição entre dois prints consecutivos, ou `None` quando nem a
/// melhor candidata chega no limiar. Empates ficam com a maior faixa: repetir
/// menos conteúdo é o que o usuário quer.
pub fn best_overlap(a: &RgbaImage, b: &RgbaImage, p: &MatchParams) -> Option<(u32, f32)> {
    let a_usable = a.height().saturating_sub(p.crop_bottom);
    let b_usable = b.height().saturating_sub(p.crop_top);
    let mut hi = a_usable.min(b_usable);
    if p.search > 0 {
        hi = hi.min(p.search);
    }
    if hi < MIN_OVERLAP {
        return None;
    }
    let mut best: Option<(u32, f32)> = None;
    let mut k = hi;
    while k >= MIN_OVERLAP {
        let d = band_diff(a, b, k, p.crop_bottom, p.crop_top);
        if best.map(|(_, s)| d < s).unwrap_or(true) {
            best = Some((k, d));
        }
        k -= 1;
    }
    best.filter(|(_, s)| *s <= p.tolerance)
}

/// Deslocamento no eixo curto conforme o alinhamento.
fn align_offset(total: u32, item: u32, align: &str) -> i64 {
    let sobra = total.saturating_sub(item);
    match align {
        "center" => (sobra / 2) as i64,
        "end" => sobra as i64,
        _ => 0,
    }
}

/// Costura vertical: é o único caso implementado: o horizontal gira 90°,
/// costura e desgira, para não ter duas versões do mesmo algoritmo.
fn stitch_vertical(
    frames: &[RgbaImage],
    opts: &StitchOptions,
    align: &str,
    progress: &ProgressFn,
) -> (RgbaImage, Vec<StitchSeam>) {
    let params = MatchParams::from(opts);
    let total = frames.len() as u64;

    // Cada print entra na costura inteiro, menos a barra fixa do topo quando
    // a junta casou: aquela faixa já está no print de cima.
    let mut drawn: Vec<RgbaImage> = Vec::with_capacity(frames.len());
    let mut tops: Vec<i64> = Vec::with_capacity(frames.len());
    let mut seams: Vec<StitchSeam> = Vec::new();
    let mut removed_prev = 0u32;

    drawn.push(frames[0].clone());
    tops.push(0);

    for i in 1..frames.len() {
        super::report(
            progress,
            "img-stitch",
            "progress",
            i as u64,
            Some(total),
            None,
        );
        let prev = &frames[i - 1];
        let cur = &frames[i];
        let found = if opts.smart {
            best_overlap(prev, cur, &params)
        } else {
            None
        };
        let (overlap, score, matched) = match found {
            Some((k, s)) => (k, s, true),
            None => (0, 0.0, false),
        };
        // Recorta a barra de topo só quando ela é mesmo repetição do anterior.
        let cut = if matched && opts.crop_top > 0 && opts.crop_top < cur.height() {
            opts.crop_top
        } else {
            0
        };
        let top = if matched {
            // A linha `prev.height - crop_bottom - overlap` do print de cima é
            // a linha `crop_top` do de baixo; `removed_prev` corrige o recorte
            // que o print de cima já sofreu.
            tops[i - 1] + prev.height() as i64
                - opts.crop_bottom.min(prev.height()) as i64
                - overlap as i64
                - removed_prev as i64
        } else {
            tops[i - 1] + drawn[i - 1].height() as i64 + opts.spacing as i64
        };
        drawn.push(if cut > 0 {
            imageops::crop_imm(cur, 0, cut, cur.width(), cur.height() - cut).to_image()
        } else {
            cur.clone()
        });
        tops.push(top);
        removed_prev = cut;
        seams.push(StitchSeam {
            index: i as u32,
            overlap,
            score,
            matched,
        });
    }

    // Sobreposição agressiva pode empurrar um print para antes do zero.
    let shift = tops.iter().copied().min().unwrap_or(0).min(0);
    let height = drawn
        .iter()
        .zip(&tops)
        .map(|(img, t)| t - shift + img.height() as i64)
        .max()
        .unwrap_or(0)
        .max(1) as u32;
    let width = drawn.iter().map(|i| i.width()).max().unwrap_or(1).max(1);

    let bg = super::icon_pack::parse_hex(&opts.background);
    let mut canvas = RgbaImage::from_pixel(width, height, bg);
    for (img, top) in drawn.iter().zip(&tops) {
        let x = align_offset(width, img.width(), align);
        imageops::overlay(&mut canvas, img, x, top - shift);
    }
    (canvas, seams)
}

/// Costura a lista já carregada. Fica separada do `run` para os testes não
/// precisarem de arquivo em disco.
pub fn stitch(
    frames: &[RgbaImage],
    opts: &StitchOptions,
    progress: &ProgressFn,
) -> anyhow::Result<(RgbaImage, Vec<StitchSeam>)> {
    if frames.is_empty() {
        return Err(anyhow!("escolha pelo menos um print"));
    }
    if opts.direction == "horizontal" {
        // Girando 90° no sentido horário, a borda direita vira o rodapé e a
        // esquerda vira o topo — exatamente o que a costura vertical espera.
        // O eixo curto inverte junto, então o alinhamento troca de ponta.
        let turned: Vec<RgbaImage> = frames.iter().map(imageops::rotate90).collect();
        let align = match opts.align.as_str() {
            "start" => "end",
            "end" => "start",
            other => other,
        };
        let (canvas, seams) = stitch_vertical(&turned, opts, align, progress);
        Ok((imageops::rotate270(&canvas), seams))
    } else {
        Ok(stitch_vertical(frames, opts, &opts.align, progress))
    }
}

/// Encolhe proporcionalmente quando passa da largura pedida.
fn cap_width(img: RgbaImage, max_width: u32) -> RgbaImage {
    if max_width == 0 || img.width() <= max_width {
        return img;
    }
    let h = ((img.height() as f64 * max_width as f64 / img.width() as f64).round() as u32).max(1);
    imageops::resize(&img, max_width, h, imageops::FilterType::Lanczos3)
}

fn encode(img: &RgbaImage, format: &str, quality: u8) -> anyhow::Result<Vec<u8>> {
    if format.eq_ignore_ascii_case("jpeg") || format.eq_ignore_ascii_case("jpg") {
        super::image_compress::encode_jpeg(&DynamicImage::ImageRgba8(img.clone()), quality)
    } else {
        let mut buf = Cursor::new(Vec::new());
        DynamicImage::ImageRgba8(img.clone())
            .write_to(&mut buf, image::ImageFormat::Png)
            .map_err(|e| anyhow!("não gravei o PNG: {}", e))?;
        Ok(buf.into_inner())
    }
}

pub fn run(opts: &StitchOptions, progress: &ProgressFn) -> anyhow::Result<StitchResult> {
    if opts.inputs.is_empty() {
        return Err(anyhow!("escolha pelo menos um print"));
    }
    let total = opts.inputs.len() as u64;
    let mut frames: Vec<RgbaImage> = Vec::with_capacity(opts.inputs.len());
    for (i, path) in opts.inputs.iter().enumerate() {
        super::report(
            progress,
            "img-stitch",
            "progress",
            i as u64,
            Some(total),
            Some(path.clone()),
        );
        let img = image::open(Path::new(path))
            .map_err(|e| anyhow!("não abri {}: {}", path, e))?
            .to_rgba8();
        frames.push(img);
    }

    let (canvas, seams) = stitch(&frames, opts, progress)?;
    let canvas = cap_width(canvas, opts.max_width);
    let data = encode(&canvas, &opts.format, opts.quality)?;

    let ext = if opts.format.eq_ignore_ascii_case("jpeg") || opts.format.eq_ignore_ascii_case("jpg")
    {
        "jpg"
    } else {
        "png"
    };
    let output = if opts.output.trim().is_empty() {
        let first = Path::new(&opts.inputs[0]);
        let dir = first.parent().map(|p| p.to_path_buf()).unwrap_or_default();
        dir.join(format!("costura.{}", ext))
    } else {
        PathBuf::from(opts.output.trim())
    };
    if let Some(parent) = output.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)?;
        }
    }
    std::fs::write(&output, &data)?;

    super::report(progress, "img-stitch", "done", total, Some(total), None);
    Ok(StitchResult {
        output: output.to_string_lossy().to_string(),
        width: canvas.width(),
        height: canvas.height(),
        bytes: data.len() as u64,
        count: frames.len() as u32,
        saved_px: seams.iter().map(|s| s.overlap).sum(),
        seams,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Conteúdo com linha a linha diferente: é o que um print de conversa tem
    /// e o que faz o deslocamento certo ser um mínimo único.
    fn source(w: u32, h: u32) -> RgbaImage {
        let mut img = RgbaImage::new(w, h);
        for (x, y, p) in img.enumerate_pixels_mut() {
            *p = image::Rgba([
                ((y * 37 + x * 3) % 251) as u8,
                ((y * 91 + 17) % 253) as u8,
                ((x * 13 + y * 5) % 249) as u8,
                255,
            ]);
        }
        img
    }

    fn rows(src: &RgbaImage, from: u32, to: u32) -> RgbaImage {
        imageops::crop_imm(src, 0, from, src.width(), to - from).to_image()
    }

    fn cols(src: &RgbaImage, from: u32, to: u32) -> RgbaImage {
        imageops::crop_imm(src, from, 0, to - from, src.height()).to_image()
    }

    fn base(smart: bool) -> StitchOptions {
        StitchOptions {
            inputs: vec![],
            direction: "vertical".into(),
            align: "start".into(),
            spacing: 0,
            background: "#FFFFFF".into(),
            smart,
            search: 0,
            tolerance: 6.0,
            crop_top: 0,
            crop_bottom: 0,
            max_width: 0,
            format: "png".into(),
            quality: 90,
            output: String::new(),
        }
    }

    fn params(o: &StitchOptions) -> MatchParams {
        MatchParams::from(o)
    }

    fn same(a: &RgbaImage, b: &RgbaImage) -> bool {
        a.dimensions() == b.dimensions() && a.as_raw() == b.as_raw()
    }

    #[test]
    fn overlap_is_recovered_exactly() {
        let src = source(120, 400);
        let a = rows(&src, 0, 250);
        let b = rows(&src, 200, 400);
        let found = best_overlap(&a, &b, &params(&base(true)));
        assert_eq!(found.map(|(k, _)| k), Some(50), "deslocamento errado");
        assert!(
            found.unwrap().1 < 0.001,
            "faixa idêntica tinha que dar zero"
        );
    }

    #[test]
    fn a_partial_overlap_still_lands_on_the_right_offset() {
        let src = source(90, 600);
        let a = rows(&src, 0, 400);
        let b = rows(&src, 380, 600);
        assert_eq!(
            best_overlap(&a, &b, &params(&base(true))).map(|(k, _)| k),
            Some(20)
        );
    }

    #[test]
    fn unrelated_prints_are_refused() {
        let a = source(100, 150);
        let mut b = RgbaImage::new(100, 150);
        for (x, y, p) in b.enumerate_pixels_mut() {
            *p = image::Rgba([250 - (x % 30) as u8, 10, 200 - (y % 40) as u8, 255]);
        }
        assert!(
            best_overlap(&a, &b, &params(&base(true))).is_none(),
            "print sem relação não pode virar sobreposição"
        );
    }

    #[test]
    fn the_search_range_caps_the_offset() {
        let src = source(80, 400);
        let a = rows(&src, 0, 250);
        let b = rows(&src, 200, 400);
        let mut o = base(true);
        o.search = 30;
        assert!(
            best_overlap(&a, &b, &params(&o)).is_none(),
            "a sobreposição real é 50; com busca de 30 não podia achar nada bom"
        );
    }

    #[test]
    fn smart_stitch_rebuilds_the_original() {
        let src = source(120, 400);
        let frames = vec![rows(&src, 0, 250), rows(&src, 200, 400)];
        let o = base(true);
        let (canvas, seams) = stitch(&frames, &o, &crate::core::tools::noop_progress()).unwrap();
        assert_eq!(seams.len(), 1);
        assert!(seams[0].matched);
        assert_eq!(seams[0].overlap, 50);
        assert!(
            same(&canvas, &src),
            "a costura tinha que devolver o original"
        );
    }

    #[test]
    fn without_a_match_it_falls_back_to_stacking() {
        let src = source(100, 300);
        let a = rows(&src, 0, 150);
        let mut b = RgbaImage::new(100, 120);
        for (x, y, p) in b.enumerate_pixels_mut() {
            *p = image::Rgba([9, (x % 7) as u8, (y % 5) as u8, 255]);
        }
        let mut o = base(true);
        o.spacing = 10;
        let (canvas, seams) = stitch(&[a, b], &o, &crate::core::tools::noop_progress()).unwrap();
        assert!(!seams[0].matched, "não podia ter casado");
        assert_eq!(seams[0].overlap, 0);
        assert_eq!(canvas.height(), 150 + 10 + 120);
        assert_eq!(
            canvas.get_pixel(0, 155),
            &image::Rgba([255, 255, 255, 255]),
            "o espaçamento tem que ficar com a cor de fundo"
        );
    }

    #[test]
    fn plain_mode_never_looks_for_overlap() {
        let src = source(100, 400);
        let frames = vec![rows(&src, 0, 250), rows(&src, 200, 400)];
        let (canvas, seams) =
            stitch(&frames, &base(false), &crate::core::tools::noop_progress()).unwrap();
        assert!(!seams[0].matched);
        assert_eq!(canvas.height(), 450, "no modo simples nada é descartado");
    }

    #[test]
    fn alignment_moves_the_narrow_print() {
        let a = source(100, 40);
        let b = source(60, 40);
        let mut o = base(false);
        o.align = "center".into();
        let (canvas, _) =
            stitch(&[a, b.clone()], &o, &crate::core::tools::noop_progress()).unwrap();
        assert_eq!(canvas.width(), 100);
        assert_eq!(canvas.get_pixel(20, 40), b.get_pixel(0, 0));
        assert_eq!(canvas.get_pixel(0, 40), &image::Rgba([255, 255, 255, 255]));

        o.align = "end".into();
        let (canvas, _) = stitch(
            &[source(100, 40), b.clone()],
            &o,
            &crate::core::tools::noop_progress(),
        )
        .unwrap();
        assert_eq!(canvas.get_pixel(40, 40), b.get_pixel(0, 0));
    }

    #[test]
    fn horizontal_stitching_works_the_same() {
        let src = source(400, 120);
        let frames = vec![cols(&src, 0, 250), cols(&src, 200, 400)];
        let mut o = base(true);
        o.direction = "horizontal".into();
        let (canvas, seams) = stitch(&frames, &o, &crate::core::tools::noop_progress()).unwrap();
        assert_eq!(seams[0].overlap, 50);
        assert!(
            same(&canvas, &src),
            "o horizontal tem que refazer o original"
        );
    }

    #[test]
    fn fixed_bars_are_ignored_and_cut() {
        let src = source(100, 400);
        let bar = image::Rgba([20u8, 20, 20, 255]);
        let mut a = rows(&src, 0, 250);
        for y in 230..250 {
            for x in 0..100 {
                a.put_pixel(x, y, bar);
            }
        }
        let mut b = rows(&src, 200, 400);
        for y in 0..20 {
            for x in 0..100 {
                b.put_pixel(x, y, bar);
            }
        }
        let mut o = base(true);
        o.crop_top = 20;
        o.crop_bottom = 20;
        let (canvas, seams) = stitch(&[a, b], &o, &crate::core::tools::noop_progress()).unwrap();
        assert!(seams[0].matched, "as barras não podiam atrapalhar a busca");
        assert_eq!(canvas.height(), 400);
        assert!(
            same(&canvas, &src),
            "a barra fixa tinha que sumir da costura"
        );
    }

    #[test]
    fn the_width_cap_keeps_the_proportion() {
        let img = source(1000, 400);
        let small = cap_width(img, 250);
        assert_eq!(small.dimensions(), (250, 100));
        assert_eq!(
            cap_width(source(100, 50), 400).dimensions(),
            (100, 50),
            "não amplia"
        );
    }

    #[test]
    fn end_to_end_writes_the_long_image() {
        let dir = std::env::temp_dir().join("omniget-stitch-test");
        std::fs::create_dir_all(&dir).unwrap();
        let src = source(120, 400);
        let p1 = dir.join("a.png");
        let p2 = dir.join("b.png");
        rows(&src, 0, 250).save(&p1).unwrap();
        rows(&src, 200, 400).save(&p2).unwrap();
        let mut o = base(true);
        o.inputs = vec![
            p1.to_string_lossy().to_string(),
            p2.to_string_lossy().to_string(),
        ];
        o.output = dir.join("longo.png").to_string_lossy().to_string();
        let res = run(&o, &crate::core::tools::noop_progress()).unwrap();
        assert_eq!((res.width, res.height), (120, 400));
        assert_eq!(res.saved_px, 50);
        assert_eq!(res.count, 2);
        assert!(std::fs::metadata(&res.output).unwrap().len() > 0);
        let back = image::open(&res.output).unwrap().to_rgba8();
        assert!(same(&back, &src));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_empty_selection_is_an_error_not_a_panic() {
        let o = base(true);
        assert!(run(&o, &crate::core::tools::noop_progress()).is_err());
        assert!(stitch(&[], &o, &crate::core::tools::noop_progress()).is_err());
    }
}

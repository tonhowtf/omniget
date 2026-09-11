//! Uma imagem → o pacote de ícones inteiro: `favicon.ico`, os PNG de web,
//! `apple-touch-icon.png` e `icon.icns` do macOS.
//!
//! Tudo com o crate `image` já pago. O `.icns` é escrito à mão porque o
//! formato é só um cabeçalho e uma lista de PNG — não vale uma dependência.

use std::io::Cursor;
use std::path::{Path, PathBuf};

use anyhow::anyhow;
use image::{imageops::FilterType, ExtendedColorType, ImageFormat, Rgba, RgbaImage};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize)]
pub struct IconOptions {
    pub input: String,
    #[serde(default)]
    pub output_dir: String,
    /// favicon.ico + favicon-16/32.png
    #[serde(default = "yes")]
    pub favicon: bool,
    /// apple-touch-icon.png (180px, sem transparência, como a Apple pede)
    #[serde(default = "yes")]
    pub apple: bool,
    /// icon-192.png e icon-512.png (PWA/Android)
    #[serde(default = "yes")]
    pub android: bool,
    /// icon.icns (app do macOS)
    #[serde(default)]
    pub icns: bool,
    /// app.ico multi-resolução (atalho/executável do Windows)
    #[serde(default)]
    pub win_ico: bool,
    /// Cor de fundo para achatar a transparência do apple-touch: "#RRGGBB".
    #[serde(default = "white")]
    pub background: String,
    /// Margem em volta do desenho, em % do lado (0-40).
    #[serde(default)]
    pub padding_pct: u32,
}

fn yes() -> bool {
    true
}

fn white() -> String {
    "#FFFFFF".into()
}

#[derive(Debug, Clone, Serialize)]
pub struct IconOut {
    pub path: String,
    pub label: String,
    pub bytes: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct IconResult {
    pub folder: String,
    pub outputs: Vec<IconOut>,
    pub source_size: (u32, u32),
    /// Avisa quando a origem é menor que o maior ícone pedido.
    pub upscaled_from: Option<u32>,
}

pub fn parse_hex(color: &str) -> Rgba<u8> {
    let s = color.trim().trim_start_matches('#');
    let n = u32::from_str_radix(s, 16).unwrap_or(0xFF_FF_FF);
    match s.len() {
        3 => {
            let r = ((n >> 8) & 0xF) as u8;
            let g = ((n >> 4) & 0xF) as u8;
            let b = (n & 0xF) as u8;
            Rgba([r * 17, g * 17, b * 17, 255])
        }
        _ => Rgba([
            ((n >> 16) & 0xFF) as u8,
            ((n >> 8) & 0xFF) as u8,
            (n & 0xFF) as u8,
            255,
        ]),
    }
}

/// Deixa a arte quadrada (centralizada, sobra transparente) e aplica a margem.
pub fn squarify(src: &RgbaImage, padding_pct: u32) -> RgbaImage {
    let pad = padding_pct.min(40) as f32 / 100.0;
    let side = src.width().max(src.height()).max(1);
    let inner = ((side as f32) * (1.0 - 2.0 * pad)).round().max(1.0) as u32;
    let scale = (inner as f32 / src.width().max(src.height()) as f32).max(f32::MIN_POSITIVE);
    let w = ((src.width() as f32 * scale).round() as u32).max(1);
    let h = ((src.height() as f32 * scale).round() as u32).max(1);
    let resized = image::imageops::resize(src, w, h, FilterType::Lanczos3);
    let mut canvas = RgbaImage::from_pixel(side, side, Rgba([0, 0, 0, 0]));
    image::imageops::overlay(
        &mut canvas,
        &resized,
        ((side - w) / 2) as i64,
        ((side - h) / 2) as i64,
    );
    canvas
}

fn at(base: &RgbaImage, size: u32) -> RgbaImage {
    image::imageops::resize(base, size, size, FilterType::Lanczos3)
}

fn flatten(img: &RgbaImage, bg: Rgba<u8>) -> RgbaImage {
    let mut out = RgbaImage::from_pixel(img.width(), img.height(), bg);
    image::imageops::overlay(&mut out, img, 0, 0);
    out
}

fn png_bytes(img: &RgbaImage) -> anyhow::Result<Vec<u8>> {
    let mut buf = Cursor::new(Vec::new());
    image::DynamicImage::ImageRgba8(img.clone()).write_to(&mut buf, ImageFormat::Png)?;
    Ok(buf.into_inner())
}

fn ico_bytes(base: &RgbaImage, sizes: &[u32]) -> anyhow::Result<Vec<u8>> {
    use image::codecs::ico::{IcoEncoder, IcoFrame};
    let scaled: Vec<RgbaImage> = sizes.iter().map(|s| at(base, *s)).collect();
    let mut frames = Vec::with_capacity(scaled.len());
    for (img, size) in scaled.iter().zip(sizes) {
        frames.push(IcoFrame::as_png(
            img.as_raw(),
            *size,
            *size,
            ExtendedColorType::Rgba8,
        )?);
    }
    let mut out = Cursor::new(Vec::new());
    IcoEncoder::new(&mut out).encode_images(&frames)?;
    Ok(out.into_inner())
}

/// Tipos do `.icns` que carregam PNG direto. `ic11`..`ic14` são as variantes
/// @2x que o Finder usa em tela Retina.
const ICNS_TYPES: &[(&[u8; 4], u32)] = &[
    (b"ic11", 32),
    (b"ic12", 64),
    (b"ic07", 128),
    (b"ic13", 256),
    (b"ic08", 256),
    (b"ic14", 512),
    (b"ic09", 512),
    (b"ic10", 1024),
];

pub fn icns_bytes(base: &RgbaImage) -> anyhow::Result<Vec<u8>> {
    let mut body: Vec<u8> = Vec::new();
    for (kind, size) in ICNS_TYPES {
        let png = png_bytes(&at(base, *size))?;
        body.extend_from_slice(kind.as_slice());
        body.extend_from_slice(&((png.len() + 8) as u32).to_be_bytes());
        body.extend_from_slice(&png);
    }
    let mut out = Vec::with_capacity(body.len() + 8);
    out.extend_from_slice(b"icns");
    out.extend_from_slice(&((body.len() + 8) as u32).to_be_bytes());
    out.extend_from_slice(&body);
    Ok(out)
}

/// Um arquivo a gerar: o nome final e como produzir os bytes dele.
type IconJob<'a> = (String, Box<dyn Fn() -> anyhow::Result<Vec<u8>> + 'a>);

pub fn run(opts: &IconOptions, progress: &super::ProgressFn) -> anyhow::Result<IconResult> {
    let inp = Path::new(&opts.input);
    let src = image::open(inp)
        .map_err(|e| anyhow!("não consegui abrir a imagem: {}", e))?
        .to_rgba8();
    let source_size = (src.width(), src.height());
    let base = squarify(&src, opts.padding_pct);
    let bg = parse_hex(&opts.background);

    let stem = inp
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "icone".into());
    let parent = if opts.output_dir.trim().is_empty() {
        inp.parent().map(|p| p.to_path_buf()).unwrap_or_default()
    } else {
        PathBuf::from(opts.output_dir.trim())
    };
    let folder = parent.join(format!("{}-icons", stem));
    std::fs::create_dir_all(&folder)?;

    let mut jobs: Vec<IconJob<'_>> = Vec::new();
    if opts.favicon {
        jobs.push((
            "favicon.ico".into(),
            Box::new(|| ico_bytes(&base, &[16, 32, 48, 64])),
        ));
        jobs.push((
            "favicon-16x16.png".into(),
            Box::new(|| png_bytes(&at(&base, 16))),
        ));
        jobs.push((
            "favicon-32x32.png".into(),
            Box::new(|| png_bytes(&at(&base, 32))),
        ));
    }
    if opts.apple {
        jobs.push((
            "apple-touch-icon.png".into(),
            Box::new(|| png_bytes(&flatten(&at(&base, 180), bg))),
        ));
    }
    if opts.android {
        jobs.push((
            "icon-192.png".into(),
            Box::new(|| png_bytes(&at(&base, 192))),
        ));
        jobs.push((
            "icon-512.png".into(),
            Box::new(|| png_bytes(&at(&base, 512))),
        ));
    }
    if opts.win_ico {
        jobs.push((
            "app.ico".into(),
            Box::new(|| ico_bytes(&base, &[16, 24, 32, 48, 64, 128, 256])),
        ));
    }
    if opts.icns {
        jobs.push(("icon.icns".into(), Box::new(|| icns_bytes(&base))));
    }
    if jobs.is_empty() {
        return Err(anyhow!("nenhum formato selecionado"));
    }

    let total = jobs.len() as u64;
    let mut outputs = Vec::new();
    let mut largest = 0u32;
    for (i, (name, make)) in jobs.iter().enumerate() {
        super::report(
            progress,
            "img-icon",
            "progress",
            i as u64,
            Some(total),
            Some(name.clone()),
        );
        let bytes = make()?;
        let path = folder.join(name);
        std::fs::write(&path, &bytes)?;
        outputs.push(IconOut {
            path: path.to_string_lossy().to_string(),
            label: name.clone(),
            bytes: bytes.len() as u64,
        });
        largest = largest.max(match name.as_str() {
            "icon.icns" => 1024,
            "icon-512.png" => 512,
            "app.ico" => 256,
            "apple-touch-icon.png" => 180,
            _ => 64,
        });
    }
    super::report(progress, "img-icon", "done", total, Some(total), None);

    let smallest_side = source_size.0.min(source_size.1);
    Ok(IconResult {
        folder: folder.to_string_lossy().to_string(),
        outputs,
        source_size,
        upscaled_from: (smallest_side < largest).then_some(smallest_side),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn art(w: u32, h: u32) -> RgbaImage {
        RgbaImage::from_pixel(w, h, Rgba([200, 30, 60, 255]))
    }

    /// Gera o pacote inteiro de uma imagem de verdade, para conferir os
    /// arquivos com as ferramentas do sistema (`iconutil`, `sips`).
    /// `OMNIGET_IMG=/caminho/arte.png cargo test -p omniget-core --lib -- --ignored live_icon`
    #[test]
    #[ignore]
    fn live_icon_pack_from_a_real_image() {
        let input = std::env::var("OMNIGET_IMG").expect("defina OMNIGET_IMG");
        let dir = std::env::temp_dir().join("omniget-icon-live");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let opts = IconOptions {
            input,
            output_dir: dir.to_string_lossy().to_string(),
            favicon: true,
            apple: true,
            android: true,
            icns: true,
            win_ico: true,
            background: "#101014".into(),
            padding_pct: 10,
        };
        let res = run(&opts, &crate::core::tools::noop_progress()).unwrap();
        assert_eq!(res.outputs.len(), 8);
        for o in &res.outputs {
            let meta = std::fs::metadata(&o.path).unwrap();
            assert!(meta.len() > 100, "{} saiu vazio", o.label);
        }
        // O apple-touch não pode ter transparência (regra da Apple).
        let apple = res
            .outputs
            .iter()
            .find(|o| o.label == "apple-touch-icon.png")
            .unwrap();
        let img = image::open(&apple.path).unwrap().to_rgba8();
        assert_eq!(img.dimensions(), (180, 180));
        assert!(
            img.pixels().all(|p| p[3] == 255),
            "sobrou alpha no apple-touch"
        );
        eprintln!("{}", res.folder);
    }

    #[test]
    fn hex_parses_both_shapes() {
        assert_eq!(parse_hex("#1E6FE8"), Rgba([0x1E, 0x6F, 0xE8, 255]));
        assert_eq!(parse_hex("fff"), Rgba([255, 255, 255, 255]));
    }

    #[test]
    fn squarify_pads_the_short_side() {
        let sq = squarify(&art(200, 100), 0);
        assert_eq!(sq.dimensions(), (200, 200));
        // Faixa de cima transparente, meio pintado.
        assert_eq!(sq.get_pixel(5, 5)[3], 0);
        assert_eq!(sq.get_pixel(100, 100)[3], 255);
    }

    #[test]
    fn padding_shrinks_the_art() {
        let sq = squarify(&art(100, 100), 20);
        assert_eq!(sq.dimensions(), (100, 100));
        assert_eq!(sq.get_pixel(2, 50)[3], 0, "a margem tem que ficar vazia");
        assert_eq!(sq.get_pixel(50, 50)[3], 255);
    }

    #[test]
    fn flatten_kills_transparency() {
        let mut img = RgbaImage::from_pixel(4, 4, Rgba([0, 0, 0, 0]));
        img.put_pixel(0, 0, Rgba([10, 20, 30, 255]));
        let flat = flatten(&img, Rgba([255, 255, 255, 255]));
        assert_eq!(flat.get_pixel(3, 3), &Rgba([255, 255, 255, 255]));
        assert_eq!(flat.get_pixel(0, 0), &Rgba([10, 20, 30, 255]));
    }

    #[test]
    fn ico_is_readable_back() {
        let bytes = ico_bytes(&art(64, 64), &[16, 32]).unwrap();
        assert_eq!(&bytes[0..4], &[0, 0, 1, 0], "cabeçalho ICONDIR");
        assert_eq!(bytes[4], 2, "duas entradas");
        let decoded = image::load_from_memory_with_format(&bytes, ImageFormat::Ico).unwrap();
        assert_eq!(decoded.width(), 32, "o ICO abre na maior entrada");
    }

    #[test]
    fn icns_header_matches_the_body() {
        let bytes = icns_bytes(&art(64, 64)).unwrap();
        assert_eq!(&bytes[0..4], b"icns");
        let declared = u32::from_be_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]) as usize;
        assert_eq!(declared, bytes.len(), "o tamanho declarado tem que bater");
        // Percorre as entradas: soma dos tamanhos == corpo.
        let mut i = 8usize;
        let mut seen = 0;
        while i + 8 <= bytes.len() {
            let len = u32::from_be_bytes([bytes[i + 4], bytes[i + 5], bytes[i + 6], bytes[i + 7]])
                as usize;
            assert!(len >= 8 && i + len <= bytes.len(), "entrada estourou");
            assert_eq!(&bytes[i + 8..i + 12], b"\x89PNG", "entrada tem que ser PNG");
            i += len;
            seen += 1;
        }
        assert_eq!(i, bytes.len());
        assert_eq!(seen, ICNS_TYPES.len());
    }
}

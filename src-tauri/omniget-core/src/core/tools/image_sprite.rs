//! Spritesheet: fatiar, juntar e redimensionar em lote por regra.
//!
//! Três trabalhos que sempre andam juntos em pasta de arte de jogo e de
//! ícone: quebrar uma folha em quadros, montar a folha de volta (com o
//! atlas JSON ao lado) e passar a régua numa pasta inteira. Tudo com o
//! crate `image`, sem processo externo.

use std::path::{Path, PathBuf};

use anyhow::anyhow;
use image::{imageops, DynamicImage, RgbaImage};
use serde::{Deserialize, Serialize};

use super::ProgressFn;

const IMAGE_EXTS: &[&str] = &["png", "jpg", "jpeg", "webp", "bmp", "gif", "tif", "tiff"];

#[derive(Debug, Clone, Deserialize)]
pub struct SpriteOptions {
    /// "slice" | "pack" | "batch".
    pub mode: String,
    /// Arquivos escolhidos na mão. No modo `slice` só o primeiro vale.
    #[serde(default)]
    pub inputs: Vec<String>,
    /// Pasta inteira, alternativa a `inputs` nos modos `pack` e `batch`.
    #[serde(default)]
    pub input_dir: String,
    #[serde(default)]
    pub output_dir: String,
    /// Prefixo dos quadros / nome da folha.
    #[serde(default)]
    pub name: String,

    // ── fatiar ──────────────────────────────────────────────────────────
    #[serde(default)]
    pub cols: u32,
    #[serde(default)]
    pub rows: u32,
    /// Tamanho da célula; quando é 0, sai de `cols`/`rows`.
    #[serde(default)]
    pub cell_w: u32,
    #[serde(default)]
    pub cell_h: u32,
    /// Borda da folha e espaço entre células.
    #[serde(default)]
    pub margin: u32,
    #[serde(default)]
    pub spacing: u32,
    /// Recorta o transparente de cada quadro.
    #[serde(default)]
    pub trim: bool,

    // ── juntar ──────────────────────────────────────────────────────────
    /// Colunas da folha; 0 = grade quase quadrada.
    #[serde(default)]
    pub pack_cols: u32,
    /// Borda e espaço entre quadros na folha montada.
    #[serde(default)]
    pub padding: u32,
    /// Grava o JSON de atlas ao lado da folha.
    #[serde(default = "default_true")]
    pub atlas: bool,

    // ── lote ────────────────────────────────────────────────────────────
    /// "width" | "height" | "fit" | "scale".
    #[serde(default = "default_rule")]
    pub rule: String,
    /// Px da regra, ou porcentagem quando `rule` é "scale".
    #[serde(default)]
    pub value: u32,
    /// Altura da caixa quando `rule` é "fit" (`value` é a largura).
    #[serde(default)]
    pub box_h: u32,
    #[serde(default)]
    pub suffix: String,
    /// "png" (padrão) ou "jpeg".
    #[serde(default = "default_format")]
    pub format: String,
    #[serde(default = "default_quality")]
    pub quality: u8,
}

fn default_true() -> bool {
    true
}
fn default_rule() -> String {
    "width".into()
}
fn default_format() -> String {
    "png".into()
}
fn default_quality() -> u8 {
    90
}

#[derive(Debug, Clone, Serialize)]
pub struct SpriteFrame {
    pub name: String,
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
}

#[derive(Debug, Clone, Serialize)]
pub struct SpriteResult {
    pub mode: String,
    /// Arquivos gravados (quadros no `slice`, imagens no `batch`).
    pub outputs: Vec<String>,
    pub frames: Vec<SpriteFrame>,
    pub sheet: Option<String>,
    pub atlas: Option<String>,
    pub width: u32,
    pub height: u32,
    pub count: u32,
    pub skipped: u32,
}

/// Retângulo de uma célula da grade.
pub type Cell = (u32, u32, u32, u32);

#[derive(Debug, Clone, Copy)]
pub struct Grid {
    pub cols: u32,
    pub rows: u32,
    pub cell_w: u32,
    pub cell_h: u32,
    pub margin: u32,
    pub spacing: u32,
}

/// Células de uma folha, seja pela grade (colunas × linhas) ou pelo tamanho
/// da célula. Célula que passa da borda da folha é descartada.
pub fn grid_cells(sheet_w: u32, sheet_h: u32, g: &Grid) -> Vec<Cell> {
    let usable_w = sheet_w.saturating_sub(g.margin * 2);
    let usable_h = sheet_h.saturating_sub(g.margin * 2);
    let (cols, rows, cell_w, cell_h) = if g.cell_w > 0 && g.cell_h > 0 {
        let cols = (usable_w + g.spacing) / (g.cell_w + g.spacing);
        let rows = (usable_h + g.spacing) / (g.cell_h + g.spacing);
        (cols, rows, g.cell_w, g.cell_h)
    } else {
        let cols = g.cols.max(1);
        let rows = g.rows.max(1);
        let cw = usable_w.saturating_sub(g.spacing * (cols - 1)) / cols;
        let ch = usable_h.saturating_sub(g.spacing * (rows - 1)) / rows;
        (cols, rows, cw, ch)
    };
    if cell_w == 0 || cell_h == 0 {
        return Vec::new();
    }
    let mut out = Vec::new();
    for r in 0..rows {
        for c in 0..cols {
            let x = g.margin + c * (cell_w + g.spacing);
            let y = g.margin + r * (cell_h + g.spacing);
            if x + cell_w > sheet_w || y + cell_h > sheet_h {
                continue;
            }
            out.push((x, y, cell_w, cell_h));
        }
    }
    out
}

/// Menor retângulo que contém pixel visível. `None` quando o quadro é todo
/// transparente — quadro vazio de spritesheet é comum e não é erro.
pub fn trim_bounds(img: &RgbaImage) -> Option<Cell> {
    let (mut x0, mut y0) = (u32::MAX, u32::MAX);
    let (mut x1, mut y1) = (0u32, 0u32);
    for (x, y, p) in img.enumerate_pixels() {
        if p.0[3] == 0 {
            continue;
        }
        x0 = x0.min(x);
        y0 = y0.min(y);
        x1 = x1.max(x);
        y1 = y1.max(y);
    }
    if x0 == u32::MAX {
        None
    } else {
        Some((x0, y0, x1 - x0 + 1, y1 - y0 + 1))
    }
}

/// Colunas de uma folha "quase quadrada": a raiz arredondada para cima.
pub fn near_square_cols(n: u32) -> u32 {
    if n == 0 {
        return 1;
    }
    let mut c = (n as f64).sqrt().ceil() as u32;
    if c == 0 {
        c = 1;
    }
    c
}

/// Posição de cada quadro na folha montada. A célula é do tamanho do maior
/// quadro, então fatiar a folha de volta pela grade devolve o que entrou.
pub fn pack_layout(sizes: &[(u32, u32)], cols: u32, padding: u32) -> (u32, u32, Vec<(u32, u32)>) {
    if sizes.is_empty() {
        return (1, 1, Vec::new());
    }
    let cols = cols.max(1);
    let rows = sizes.len().div_ceil(cols as usize) as u32;
    let cell_w = sizes.iter().map(|s| s.0).max().unwrap_or(1).max(1);
    let cell_h = sizes.iter().map(|s| s.1).max().unwrap_or(1).max(1);
    let positions = (0..sizes.len())
        .map(|i| {
            let c = i as u32 % cols;
            let r = i as u32 / cols;
            (
                padding + c * (cell_w + padding),
                padding + r * (cell_h + padding),
            )
        })
        .collect();
    (
        padding + cols * (cell_w + padding),
        padding + rows * (cell_h + padding),
        positions,
    )
}

/// Tamanho novo pela regra, sempre com a proporção preservada.
pub fn resize_by_rule(w: u32, h: u32, rule: &str, value: u32, box_h: u32) -> (u32, u32) {
    if w == 0 || h == 0 {
        return (w.max(1), h.max(1));
    }
    let (nw, nh) = match rule {
        "height" => {
            let v = value.max(1) as f64;
            ((w as f64 * v / h as f64).round(), v)
        }
        "fit" => {
            let bw = value.max(1) as f64;
            let bh = box_h.max(1) as f64;
            let s = (bw / w as f64).min(bh / h as f64);
            ((w as f64 * s).round(), (h as f64 * s).round())
        }
        "scale" => {
            let s = value.max(1) as f64 / 100.0;
            ((w as f64 * s).round(), (h as f64 * s).round())
        }
        _ => {
            let v = value.max(1) as f64;
            (v, (h as f64 * v / w as f64).round())
        }
    };
    ((nw as u32).max(1), (nh as u32).max(1))
}

fn is_image(p: &Path) -> bool {
    p.extension()
        .map(|e| IMAGE_EXTS.contains(&e.to_string_lossy().to_lowercase().as_str()))
        .unwrap_or(false)
}

/// Arquivos da entrada: a lista escolhida na mão ou a pasta, em ordem de nome
/// (a numeração do quadro tem que ser previsível).
fn gather(opts: &SpriteOptions) -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = opts.inputs.iter().map(PathBuf::from).collect();
    if !opts.input_dir.trim().is_empty() {
        if let Ok(rd) = std::fs::read_dir(opts.input_dir.trim()) {
            let mut found: Vec<PathBuf> = rd
                .flatten()
                .map(|e| e.path())
                .filter(|p| p.is_file() && is_image(p))
                .collect();
            found.sort();
            files.extend(found);
        }
    }
    files
}

fn out_dir(opts: &SpriteOptions, fallback: &Path) -> PathBuf {
    if opts.output_dir.trim().is_empty() {
        fallback.to_path_buf()
    } else {
        PathBuf::from(opts.output_dir.trim())
    }
}

fn ext_of(format: &str) -> &'static str {
    if format.eq_ignore_ascii_case("jpeg") || format.eq_ignore_ascii_case("jpg") {
        "jpg"
    } else {
        "png"
    }
}

fn write_image(img: &RgbaImage, path: &Path, format: &str, quality: u8) -> anyhow::Result<u64> {
    let data = if ext_of(format) == "jpg" {
        super::image_compress::encode_jpeg(&DynamicImage::ImageRgba8(img.clone()), quality)?
    } else {
        let mut buf = std::io::Cursor::new(Vec::new());
        DynamicImage::ImageRgba8(img.clone())
            .write_to(&mut buf, image::ImageFormat::Png)
            .map_err(|e| anyhow!("não gravei o PNG: {}", e))?;
        buf.into_inner()
    };
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)?;
        }
    }
    std::fs::write(path, &data)?;
    Ok(data.len() as u64)
}

/// JSON de atlas no formato de mapa (o mesmo shape que o TexturePacker usa no
/// preset "JSON (Hash)"), que é o que engine e bundler já sabem ler.
pub fn atlas_json(frames: &[SpriteFrame], sheet: &str, w: u32, h: u32) -> String {
    let mut map = serde_json::Map::new();
    for f in frames {
        map.insert(
            f.name.clone(),
            serde_json::json!({
                "frame": { "x": f.x, "y": f.y, "w": f.w, "h": f.h },
                "rotated": false,
                "trimmed": false,
                "sourceSize": { "w": f.w, "h": f.h }
            }),
        );
    }
    serde_json::to_string_pretty(&serde_json::json!({
        "frames": serde_json::Value::Object(map),
        "meta": {
            "app": "OmniGet",
            "image": sheet,
            "format": "RGBA8888",
            "size": { "w": w, "h": h },
            "scale": "1"
        }
    }))
    .unwrap_or_else(|_| "{}".into())
}

fn slice(opts: &SpriteOptions, progress: &ProgressFn) -> anyhow::Result<SpriteResult> {
    let files = gather(opts);
    let input = files
        .first()
        .ok_or_else(|| anyhow!("escolha o spritesheet"))?;
    let sheet = image::open(input)
        .map_err(|e| anyhow!("não abri {}: {}", input.display(), e))?
        .to_rgba8();
    let grid = Grid {
        cols: opts.cols,
        rows: opts.rows,
        cell_w: opts.cell_w,
        cell_h: opts.cell_h,
        margin: opts.margin,
        spacing: opts.spacing,
    };
    let cells = grid_cells(sheet.width(), sheet.height(), &grid);
    if cells.is_empty() {
        return Err(anyhow!("a grade não cabe nessa folha"));
    }

    let stem = if opts.name.trim().is_empty() {
        input
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| "quadro".into())
    } else {
        super::sanitize_name(&opts.name)
    };
    let dir = out_dir(
        opts,
        &input
            .parent()
            .map(|p| p.join(format!("{}-quadros", stem)))
            .unwrap_or_default(),
    );
    std::fs::create_dir_all(&dir)?;

    let total = cells.len() as u64;
    let mut outputs = Vec::new();
    let mut frames = Vec::new();
    let mut skipped = 0u32;
    for (i, (x, y, w, h)) in cells.iter().enumerate() {
        super::report(
            progress,
            "img-sprite",
            "progress",
            i as u64,
            Some(total),
            None,
        );
        let mut cut = imageops::crop_imm(&sheet, *x, *y, *w, *h).to_image();
        let (mut fx, mut fy, mut fw, mut fh) = (*x, *y, *w, *h);
        if opts.trim {
            match trim_bounds(&cut) {
                Some((tx, ty, tw, th)) => {
                    cut = imageops::crop_imm(&cut, tx, ty, tw, th).to_image();
                    fx += tx;
                    fy += ty;
                    fw = tw;
                    fh = th;
                }
                None => {
                    // Quadro vazio: não vira arquivo, mas conta no relatório.
                    skipped += 1;
                    continue;
                }
            }
        }
        let name = format!("{}_{:03}.{}", stem, i, ext_of(&opts.format));
        let path = dir.join(&name);
        write_image(&cut, &path, &opts.format, opts.quality)?;
        outputs.push(path.to_string_lossy().to_string());
        frames.push(SpriteFrame {
            name,
            x: fx,
            y: fy,
            w: fw,
            h: fh,
        });
    }

    super::report(progress, "img-sprite", "done", total, Some(total), None);
    Ok(SpriteResult {
        mode: "slice".into(),
        count: outputs.len() as u32,
        outputs,
        frames,
        sheet: Some(input.to_string_lossy().to_string()),
        atlas: None,
        width: sheet.width(),
        height: sheet.height(),
        skipped,
    })
}

fn pack(opts: &SpriteOptions, progress: &ProgressFn) -> anyhow::Result<SpriteResult> {
    let files = gather(opts);
    if files.is_empty() {
        return Err(anyhow!("escolha os quadros"));
    }
    let total = files.len() as u64;
    let mut images: Vec<(String, RgbaImage)> = Vec::new();
    let mut skipped = 0u32;
    for (i, path) in files.iter().enumerate() {
        super::report(
            progress,
            "img-sprite",
            "progress",
            i as u64,
            Some(total),
            Some(path.to_string_lossy().to_string()),
        );
        match image::open(path) {
            Ok(img) => {
                let name = path
                    .file_name()
                    .map(|s| s.to_string_lossy().to_string())
                    .unwrap_or_else(|| format!("quadro_{:03}", i));
                images.push((name, img.to_rgba8()));
            }
            Err(e) => {
                tracing::warn!("[img-sprite] {}: {}", path.display(), e);
                skipped += 1;
            }
        }
    }
    if images.is_empty() {
        return Err(anyhow!("nenhum quadro pôde ser aberto"));
    }

    let sizes: Vec<(u32, u32)> = images.iter().map(|(_, i)| i.dimensions()).collect();
    let cols = if opts.pack_cols > 0 {
        opts.pack_cols
    } else {
        near_square_cols(images.len() as u32)
    };
    let (w, h, positions) = pack_layout(&sizes, cols, opts.padding);
    let mut sheet = RgbaImage::from_pixel(w, h, image::Rgba([0, 0, 0, 0]));
    let mut frames = Vec::with_capacity(images.len());
    for ((name, img), (x, y)) in images.iter().zip(&positions) {
        imageops::overlay(&mut sheet, img, *x as i64, *y as i64);
        frames.push(SpriteFrame {
            name: name.clone(),
            x: *x,
            y: *y,
            w: img.width(),
            h: img.height(),
        });
    }

    let stem = if opts.name.trim().is_empty() {
        "spritesheet".to_string()
    } else {
        super::sanitize_name(&opts.name)
    };
    let dir = out_dir(
        opts,
        &files[0]
            .parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_default(),
    );
    // A folha é sempre PNG: JPEG não guarda transparência e o atlas ficaria
    // apontando para quadros com fundo preto.
    let sheet_path = dir.join(format!("{}.png", stem));
    write_image(&sheet, &sheet_path, "png", 100)?;
    let mut outputs = vec![sheet_path.to_string_lossy().to_string()];
    let atlas = if opts.atlas {
        let json = atlas_json(
            &frames,
            &format!("{}.png", stem),
            sheet.width(),
            sheet.height(),
        );
        let atlas_path = dir.join(format!("{}.json", stem));
        std::fs::write(&atlas_path, json)?;
        outputs.push(atlas_path.to_string_lossy().to_string());
        Some(atlas_path.to_string_lossy().to_string())
    } else {
        None
    };

    super::report(progress, "img-sprite", "done", total, Some(total), None);
    Ok(SpriteResult {
        mode: "pack".into(),
        count: frames.len() as u32,
        outputs,
        frames,
        sheet: Some(sheet_path.to_string_lossy().to_string()),
        atlas,
        width: sheet.width(),
        height: sheet.height(),
        skipped,
    })
}

fn batch(opts: &SpriteOptions, progress: &ProgressFn) -> anyhow::Result<SpriteResult> {
    let files = gather(opts);
    if files.is_empty() {
        return Err(anyhow!("escolha as imagens ou a pasta"));
    }
    let total = files.len() as u64;
    let mut outputs = Vec::new();
    let mut frames = Vec::new();
    let mut skipped = 0u32;
    let suffix = if opts.suffix.is_empty() {
        "-lote"
    } else {
        opts.suffix.as_str()
    };
    for (i, path) in files.iter().enumerate() {
        super::report(
            progress,
            "img-sprite",
            "progress",
            i as u64,
            Some(total),
            Some(path.to_string_lossy().to_string()),
        );
        let Ok(img) = image::open(path) else {
            skipped += 1;
            continue;
        };
        let (nw, nh) = resize_by_rule(
            img.width(),
            img.height(),
            &opts.rule,
            opts.value,
            opts.box_h,
        );
        let resized = imageops::resize(&img.to_rgba8(), nw, nh, imageops::FilterType::Lanczos3);
        let dir = out_dir(
            opts,
            &path.parent().map(|p| p.to_path_buf()).unwrap_or_default(),
        );
        let stem = path
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| format!("imagem_{:03}", i));
        let name = format!("{}{}.{}", stem, suffix, ext_of(&opts.format));
        let out = dir.join(&name);
        if let Err(e) = write_image(&resized, &out, &opts.format, opts.quality) {
            tracing::warn!("[img-sprite] {}: {}", out.display(), e);
            skipped += 1;
            continue;
        }
        outputs.push(out.to_string_lossy().to_string());
        frames.push(SpriteFrame {
            name,
            x: 0,
            y: 0,
            w: nw,
            h: nh,
        });
    }

    super::report(progress, "img-sprite", "done", total, Some(total), None);
    Ok(SpriteResult {
        mode: "batch".into(),
        count: outputs.len() as u32,
        outputs,
        frames,
        sheet: None,
        atlas: None,
        width: 0,
        height: 0,
        skipped,
    })
}

pub fn run(opts: &SpriteOptions, progress: &ProgressFn) -> anyhow::Result<SpriteResult> {
    match opts.mode.as_str() {
        "slice" => slice(opts, progress),
        "pack" => pack(opts, progress),
        "batch" => batch(opts, progress),
        other => Err(anyhow!("modo desconhecido: {}", other)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn solid(w: u32, h: u32, c: [u8; 4]) -> RgbaImage {
        RgbaImage::from_pixel(w, h, image::Rgba(c))
    }

    /// Quadro com um pixel diferente em cada canto, para o round-trip pegar
    /// qualquer deslocamento de um px.
    fn frame(i: u32) -> RgbaImage {
        let mut img = solid(16, 16, [(i * 20) as u8, 40, 200 - (i * 7) as u8, 255]);
        img.put_pixel(0, 0, image::Rgba([255, 255, 255, 255]));
        img.put_pixel(15, 15, image::Rgba([0, 0, 0, 255]));
        img
    }

    fn base(mode: &str) -> SpriteOptions {
        SpriteOptions {
            mode: mode.into(),
            inputs: vec![],
            input_dir: String::new(),
            output_dir: String::new(),
            name: String::new(),
            cols: 0,
            rows: 0,
            cell_w: 0,
            cell_h: 0,
            margin: 0,
            spacing: 0,
            trim: false,
            pack_cols: 0,
            padding: 0,
            atlas: true,
            rule: "width".into(),
            value: 0,
            box_h: 0,
            suffix: String::new(),
            format: "png".into(),
            quality: 90,
        }
    }

    fn grid(cols: u32, rows: u32, cell_w: u32, cell_h: u32, margin: u32, spacing: u32) -> Grid {
        Grid {
            cols,
            rows,
            cell_w,
            cell_h,
            margin,
            spacing,
        }
    }

    #[test]
    fn a_four_by_two_grid_gives_eight_cells() {
        let cells = grid_cells(64, 32, &grid(4, 2, 0, 0, 0, 0));
        assert_eq!(cells.len(), 8);
        assert_eq!(cells[0], (0, 0, 16, 16));
        assert_eq!(cells[3], (48, 0, 16, 16));
        assert_eq!(cells[4], (0, 16, 16, 16));
        assert_eq!(cells[7], (48, 16, 16, 16));
    }

    #[test]
    fn cell_size_and_grid_agree_on_the_same_sheet() {
        let by_grid = grid_cells(74, 38, &grid(4, 2, 0, 0, 2, 2));
        let by_cell = grid_cells(74, 38, &grid(0, 0, 16, 16, 2, 2));
        assert_eq!(by_grid, by_cell, "as duas formas descrevem a mesma grade");
        assert_eq!(by_cell[0], (2, 2, 16, 16));
        assert_eq!(by_cell[1], (20, 2, 16, 16));
    }

    #[test]
    fn cells_that_do_not_fit_are_dropped() {
        // 50 px de folha só cabem três células de 16 com 2 de espaço.
        let cells = grid_cells(50, 16, &grid(0, 0, 16, 16, 0, 2));
        assert_eq!(cells.len(), 2);
        assert!(cells.iter().all(|(x, _, w, _)| x + w <= 50));
    }

    #[test]
    fn slicing_a_synthetic_sheet_returns_the_right_pixels() {
        let dir = std::env::temp_dir().join("omniget-sprite-slice");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut sheet = RgbaImage::new(64, 32);
        for i in 0..8u32 {
            let (c, r) = (i % 4, i / 4);
            let cell = solid(16, 16, [(i * 30) as u8, 10, 90, 255]);
            imageops::overlay(&mut sheet, &cell, (c * 16) as i64, (r * 16) as i64);
        }
        let sheet_path = dir.join("folha.png");
        sheet.save(&sheet_path).unwrap();

        let mut o = base("slice");
        o.inputs = vec![sheet_path.to_string_lossy().to_string()];
        o.cols = 4;
        o.rows = 2;
        o.output_dir = dir.to_string_lossy().to_string();
        o.name = "heroi".into();
        let res = run(&o, &crate::core::tools::noop_progress()).unwrap();

        assert_eq!(res.count, 8);
        assert_eq!(res.frames[0].name, "heroi_000.png");
        assert_eq!(res.frames[7].name, "heroi_007.png");
        for i in 0..8u32 {
            let img = image::open(&res.outputs[i as usize]).unwrap().to_rgba8();
            assert_eq!(img.dimensions(), (16, 16));
            assert_eq!(
                img.get_pixel(3, 3),
                &image::Rgba([(i * 30) as u8, 10, 90, 255]),
                "quadro {} veio da célula errada",
                i
            );
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn pack_then_slice_is_a_round_trip() {
        let dir = std::env::temp_dir().join("omniget-sprite-roundtrip");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut inputs = Vec::new();
        for i in 0..8u32 {
            let p = dir.join(format!("q{}.png", i));
            frame(i).save(&p).unwrap();
            inputs.push(p.to_string_lossy().to_string());
        }

        let mut o = base("pack");
        o.inputs = inputs;
        o.pack_cols = 4;
        o.padding = 2;
        o.name = "folha".into();
        o.output_dir = dir.to_string_lossy().to_string();
        let packed = run(&o, &crate::core::tools::noop_progress()).unwrap();
        assert_eq!((packed.width, packed.height), (74, 38));
        assert_eq!(packed.frames.len(), 8);
        assert_eq!((packed.frames[0].x, packed.frames[0].y), (2, 2));
        assert_eq!((packed.frames[5].x, packed.frames[5].y), (20, 20));
        let atlas = std::fs::read_to_string(packed.atlas.as_ref().unwrap()).unwrap();
        assert!(atlas.contains("\"frames\""), "faltou o mapa de quadros");
        assert!(atlas.contains("q5.png"));

        let out = dir.join("de-volta");
        let mut s = base("slice");
        s.inputs = vec![packed.sheet.clone().unwrap()];
        s.cell_w = 16;
        s.cell_h = 16;
        s.margin = 2;
        s.spacing = 2;
        s.name = "q".into();
        s.output_dir = out.to_string_lossy().to_string();
        let sliced = run(&s, &crate::core::tools::noop_progress()).unwrap();
        assert_eq!(sliced.count, 8, "voltaram menos quadros do que entraram");
        for i in 0..8u32 {
            let back = image::open(&sliced.outputs[i as usize]).unwrap().to_rgba8();
            assert_eq!(
                back.as_raw(),
                frame(i).as_raw(),
                "o quadro {} não voltou igual",
                i
            );
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn packing_without_columns_stays_near_square() {
        assert_eq!(near_square_cols(4), 2);
        assert_eq!(near_square_cols(8), 3);
        assert_eq!(near_square_cols(9), 3);
        assert_eq!(near_square_cols(10), 4);
        assert_eq!(near_square_cols(1), 1);
        assert_eq!(
            near_square_cols(0),
            1,
            "lista vazia não pode dar zero coluna"
        );
    }

    #[test]
    fn the_cell_is_the_biggest_frame() {
        let (w, h, pos) = pack_layout(&[(10, 20), (30, 5), (8, 8)], 2, 0);
        assert_eq!((w, h), (60, 40), "célula 30x20, grade 2x2");
        assert_eq!(pos, vec![(0, 0), (30, 0), (0, 20)]);
    }

    #[test]
    fn trim_finds_the_visible_box() {
        let mut img = solid(20, 20, [0, 0, 0, 0]);
        img.put_pixel(5, 7, image::Rgba([255, 0, 0, 255]));
        img.put_pixel(9, 11, image::Rgba([0, 255, 0, 255]));
        assert_eq!(trim_bounds(&img), Some((5, 7, 5, 5)));
        assert_eq!(
            trim_bounds(&solid(4, 4, [0, 0, 0, 0])),
            None,
            "quadro vazio não tem caixa"
        );
    }

    #[test]
    fn every_rule_keeps_the_proportion() {
        assert_eq!(resize_by_rule(1000, 500, "width", 200, 0), (200, 100));
        assert_eq!(resize_by_rule(1000, 500, "height", 100, 0), (200, 100));
        assert_eq!(resize_by_rule(1000, 500, "fit", 300, 300), (300, 150));
        assert_eq!(resize_by_rule(400, 300, "fit", 1000, 150), (200, 150));
        assert_eq!(resize_by_rule(1000, 500, "scale", 25, 0), (250, 125));
        assert_eq!(
            resize_by_rule(3, 1, "scale", 1, 0),
            (1, 1),
            "nunca chega a zero px"
        );
    }

    #[test]
    fn batch_resizes_a_whole_folder() {
        let dir = std::env::temp_dir().join("omniget-sprite-batch");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        for i in 0..3u32 {
            solid(400, 200, [(i * 50) as u8, 20, 20, 255])
                .save(dir.join(format!("foto{}.png", i)))
                .unwrap();
        }
        std::fs::write(dir.join("leiame.txt"), "não é imagem").unwrap();

        let mut o = base("batch");
        o.input_dir = dir.to_string_lossy().to_string();
        o.output_dir = dir.join("saida").to_string_lossy().to_string();
        o.rule = "width".into();
        o.value = 100;
        o.suffix = "@100".into();
        let res = run(&o, &crate::core::tools::noop_progress()).unwrap();
        assert_eq!(res.count, 3, "o txt não podia entrar");
        for out in &res.outputs {
            let img = image::open(out).unwrap().to_rgba8();
            assert_eq!(img.dimensions(), (100, 50));
        }
        assert!(dir.join("saida").join("foto0@100.png").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_unknown_mode_is_an_error() {
        assert!(run(&base("nada"), &crate::core::tools::noop_progress()).is_err());
        assert!(run(&base("pack"), &crate::core::tools::noop_progress()).is_err());
    }
}

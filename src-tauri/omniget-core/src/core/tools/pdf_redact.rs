//! Tarja de verdade: apaga o texto e só depois pinta o retângulo. A tarja que
//! só desenha uma barra preta por cima é o defeito que o `pdf-redaction-check`
//! acusa — aqui o texto sai do content stream antes da tinta entrar.
//!
//! O PDFium entrega a caixa de cada caractere da página (`pdf::read_raw_chars`)
//! e o `lopdf` reescreve os operadores de texto sem os glifos que caem dentro
//! da região. Quando o casamento glifo↔byte não fecha (texto dentro de Form
//! XObject, PDFium velho sem `FPDFText_IsGenerated`, content stream que o lopdf
//! não decodifica), a página cai para pixel: renderiza, pinta a tarja na imagem
//! e troca o conteúdo da página pela imagem. É perda de texto, mas nunca deixa
//! segredo escondido.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::anyhow;
use lopdf::content::{Content, Operation};
use lopdf::{dictionary, Document, Object, Stream};
use serde::{Deserialize, Serialize};

use super::pdf;

/// Região a tarjar, em pontos do PDF e com a origem no canto inferior
/// esquerdo da página — o mesmo sistema do `pdf-redaction-check`.
#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
pub struct Area {
    /// Página 1-based.
    pub page: usize,
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Options {
    pub input: String,
    #[serde(default)]
    pub password: String,
    /// Regiões desenhadas pelo usuário.
    #[serde(default)]
    pub areas: Vec<Area>,
    /// Palavras ou frases a tarjar onde aparecerem (sem caixa, sem acento).
    #[serde(default)]
    pub terms: Vec<String>,
    /// Limita a busca por termo a estas páginas ("1-3, 7"). Vazio = todas.
    #[serde(default)]
    pub pages: String,
    /// "text" (cirúrgico, cai para pixel quando não dá) | "raster" (tudo em pixel).
    #[serde(default)]
    pub mode: String,
    /// Folga em pontos ao redor do termo encontrado.
    #[serde(default)]
    pub padding: f32,
    /// Pinta o retângulo por cima. Desligado deixa o buraco em branco.
    #[serde(default = "yes")]
    pub bar: bool,
    /// DPI da renderização quando a página cai para pixel.
    #[serde(default)]
    pub dpi: u32,
    #[serde(default)]
    pub quality: u8,
    #[serde(default)]
    pub output_dir: String,
    #[serde(default)]
    pub suffix: String,
}

fn yes() -> bool {
    true
}

#[derive(Debug, Clone, Serialize)]
pub struct PageReport {
    pub page: usize,
    pub areas: usize,
    /// Quantos caracteres saíram do content stream.
    pub chars_removed: usize,
    /// "text" quando deu para apagar o texto, "raster" quando virou imagem.
    pub mode: String,
    pub note: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct RedactResult {
    pub input: String,
    pub output: String,
    pub pages: usize,
    pub areas: usize,
    pub chars_removed: usize,
    /// Ocorrências de termo encontradas e tarjadas.
    pub terms_found: usize,
    /// Caracteres que ainda apareciam dentro de alguma região depois da
    /// primeira passada — cada um vira uma página em pixel.
    pub leftovers: usize,
    /// `true` quando a conferência final não achou nada escondido.
    pub verified: bool,
    pub bytes: u64,
    pub by_page: Vec<PageReport>,
}

type Rect = [f32; 4];

// ── Região ─────────────────────────────────────────────────────────────

/// Normaliza a área (largura/altura negativas viram retângulo certo) e já
/// aplica a folga.
pub fn rect_of(a: &Area, padding: f32) -> Rect {
    let x0 = a.x.min(a.x + a.width) - padding;
    let x1 = a.x.max(a.x + a.width) + padding;
    let y0 = a.y.min(a.y + a.height) - padding;
    let y1 = a.y.max(a.y + a.height) + padding;
    [x0, y0, x1, y1]
}

pub fn inside(r: &Rect, x: f32, y: f32) -> bool {
    x >= r[0] && x <= r[2] && y >= r[1] && y <= r[3]
}

fn any_inside(rects: &[Rect], x: f32, y: f32) -> bool {
    rects.iter().any(|r| inside(r, x, y))
}

/// Caixa que cobre todos os retângulos dados.
pub fn union(rects: &[Rect]) -> Option<Rect> {
    rects.iter().copied().reduce(|a, b| {
        [
            a[0].min(b[0]),
            a[1].min(b[1]),
            a[2].max(b[2]),
            a[3].max(b[3]),
        ]
    })
}

/// Minúscula sem acento, para casar termo com o texto da página.
pub fn fold(c: char) -> char {
    let c = c.to_ascii_lowercase();
    match c {
        'á' | 'à' | 'â' | 'ã' | 'ä' | 'Á' | 'À' | 'Â' | 'Ã' | 'Ä' => 'a',
        'é' | 'è' | 'ê' | 'ë' | 'É' | 'È' | 'Ê' | 'Ë' => 'e',
        'í' | 'ì' | 'î' | 'ï' | 'Í' | 'Ì' | 'Î' | 'Ï' => 'i',
        'ó' | 'ò' | 'ô' | 'õ' | 'ö' | 'Ó' | 'Ò' | 'Ô' | 'Õ' | 'Ö' => 'o',
        'ú' | 'ù' | 'û' | 'ü' | 'Ú' | 'Ù' | 'Û' | 'Ü' => 'u',
        'ç' | 'Ç' => 'c',
        'ñ' | 'Ñ' => 'n',
        c if c.is_whitespace() => ' ',
        c => c.to_lowercase().next().unwrap_or(c),
    }
}

pub fn fold_str(s: &str) -> String {
    s.chars().map(fold).collect()
}

/// Acha as ocorrências do termo na sequência de caracteres da página e
/// devolve, para cada uma, os índices dos caracteres que a compõem.
pub fn find_term(chars: &[char], term: &str) -> Vec<(usize, usize)> {
    let needle: Vec<char> = fold_str(term.trim()).chars().collect();
    if needle.is_empty() || needle.len() > chars.len() {
        return Vec::new();
    }
    let hay: Vec<char> = chars.iter().copied().map(fold).collect();
    let mut out = Vec::new();
    let mut i = 0usize;
    while i + needle.len() <= hay.len() {
        if hay[i..i + needle.len()] == needle[..] {
            out.push((i, i + needle.len()));
            i += needle.len();
        } else {
            i += 1;
        }
    }
    out
}

// ── Content stream: tirar glifo ────────────────────────────────────────

/// O que fazer com cada glifo da página, na ordem do content stream.
pub trait GlyphPlan {
    /// `None` mantém o glifo; `Some(avanço em pontos)` tira ele e devolve a
    /// largura que ele ocupava, para o texto seguinte não andar para trás.
    fn drop_at(&mut self, index: usize) -> Option<f32>;
}

impl<F: FnMut(usize) -> Option<f32>> GlyphPlan for F {
    fn drop_at(&mut self, index: usize) -> Option<f32> {
        self(index)
    }
}

fn mul(a: [f64; 6], b: [f64; 6]) -> [f64; 6] {
    [
        a[0] * b[0] + a[1] * b[2],
        a[0] * b[1] + a[1] * b[3],
        a[2] * b[0] + a[3] * b[2],
        a[2] * b[1] + a[3] * b[3],
        a[4] * b[0] + a[5] * b[2] + b[4],
        a[4] * b[1] + a[5] * b[3] + b[5],
    ]
}

fn nums(op: &Operation) -> Vec<f64> {
    op.operands
        .iter()
        .map(|o| match o {
            Object::Integer(i) => *i as f64,
            Object::Real(r) => *r as f64,
            _ => 0.0,
        })
        .collect()
}

const ID: [f64; 6] = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];

/// Estado de texto que interessa para converter "largura em pontos" em
/// número de ajuste do `TJ`.
struct TextState {
    ctm: [f64; 6],
    stack: Vec<[f64; 6]>,
    tm: [f64; 6],
    size: f64,
    /// `Tz` em fração (100% = 1.0).
    hscale: f64,
    font: String,
}

impl TextState {
    fn new() -> Self {
        TextState {
            ctm: ID,
            stack: Vec::new(),
            tm: ID,
            size: 0.0,
            hscale: 1.0,
            font: String::new(),
        }
    }

    /// Escala horizontal de "espaço de texto" para "ponto da página".
    fn scale(&self) -> f64 {
        let m = mul(self.tm, self.ctm);
        (m[0] * m[0] + m[1] * m[1]).sqrt()
    }

    /// Número de `TJ` que devolve exatamente `width_pt` de avanço.
    fn kern(&self, width_pt: f32) -> Option<f64> {
        let scale = self.scale();
        let denom = self.size * self.hscale;
        if !scale.is_finite() || scale <= 1e-6 || denom.abs() <= 1e-6 {
            return None;
        }
        let tx = width_pt as f64 / scale;
        let n = -tx * 1000.0 / denom;
        if n.is_finite() && n.abs() < 100_000.0 {
            Some((n * 100.0).round() / 100.0)
        } else {
            None
        }
    }

    fn apply(&mut self, op: &Operation) {
        match op.operator.as_str() {
            "q" => self.stack.push(self.ctm),
            "Q" => {
                if let Some(m) = self.stack.pop() {
                    self.ctm = m;
                }
            }
            "cm" => {
                let n = nums(op);
                if n.len() == 6 {
                    self.ctm = mul([n[0], n[1], n[2], n[3], n[4], n[5]], self.ctm);
                }
            }
            "BT" => self.tm = ID,
            "Tm" => {
                let n = nums(op);
                if n.len() == 6 {
                    self.tm = [n[0], n[1], n[2], n[3], n[4], n[5]];
                }
            }
            "Tz" => {
                if let Some(v) = nums(op).first() {
                    self.hscale = v / 100.0;
                }
            }
            "Tf" => {
                if let Some(Object::Name(n)) = op.operands.first() {
                    self.font = String::from_utf8_lossy(n).to_string();
                }
                if let Some(v) = nums(op).get(1) {
                    self.size = *v;
                }
            }
            _ => {}
        }
    }
}

/// Bytes de um código de glifo: fonte composta (Type0) usa 2, o resto usa 1.
fn code_len(two_byte: &dyn Fn(&str) -> bool, font: &str) -> usize {
    if two_byte(font) {
        2
    } else {
        1
    }
}

fn as_string(o: &Object) -> Option<&Vec<u8>> {
    match o {
        Object::String(b, _) => Some(b),
        _ => None,
    }
}

fn lit(bytes: Vec<u8>) -> Object {
    Object::String(bytes, lopdf::StringFormat::Literal)
}

/// Reescreve um texto mostrado, tirando os glifos que o plano mandou tirar.
/// Devolve `None` quando nada mudou.
fn strip_string(
    bytes: &[u8],
    step: usize,
    glyph: &mut usize,
    plan: &mut dyn GlyphPlan,
    st: &TextState,
    out: &mut Vec<Object>,
    removed: &mut usize,
) -> bool {
    let mut changed = false;
    let mut keep: Vec<u8> = Vec::new();
    let mut pending = 0f32;
    let mut i = 0usize;
    while i < bytes.len() {
        let end = (i + step).min(bytes.len());
        let code = &bytes[i..end];
        let drop = plan.drop_at(*glyph);
        *glyph += 1;
        match drop {
            Some(width) => {
                changed = true;
                *removed += 1;
                pending += width;
            }
            None => {
                if pending > 0.0 {
                    if !keep.is_empty() {
                        out.push(lit(std::mem::take(&mut keep)));
                    }
                    if let Some(n) = st.kern(pending) {
                        out.push(Object::Real(n as f32));
                    }
                    pending = 0.0;
                }
                keep.extend_from_slice(code);
            }
        }
        i = end;
    }
    if !keep.is_empty() {
        out.push(lit(keep));
    }
    if pending > 0.0 {
        if let Some(n) = st.kern(pending) {
            out.push(Object::Real(n as f32));
        }
    }
    changed
}

/// Passa por todas as operações da página e devolve a lista nova, sem os
/// glifos marcados. O segundo valor é quantos glifos saíram; o terceiro é
/// quantos glifos a página tem no total (para conferir o casamento com o
/// PDFium antes de gravar).
pub fn strip_glyphs(
    ops: &[Operation],
    two_byte: &dyn Fn(&str) -> bool,
    plan: &mut dyn GlyphPlan,
) -> (Vec<Operation>, usize, usize) {
    let mut st = TextState::new();
    let mut out: Vec<Operation> = Vec::with_capacity(ops.len());
    let mut glyph = 0usize;
    let mut removed = 0usize;
    for op in ops {
        st.apply(op);
        let step = code_len(two_byte, &st.font);
        match op.operator.as_str() {
            "Tj" | "'" | "\"" => {
                let (prefix, text): (Vec<Operation>, Option<&Vec<u8>>) = match op.operator.as_str()
                {
                    "Tj" => (Vec::new(), op.operands.first().and_then(as_string)),
                    "'" => (
                        vec![Operation::new("T*", vec![])],
                        op.operands.first().and_then(as_string),
                    ),
                    _ => {
                        let n = nums(op);
                        let mut pre = Vec::new();
                        if n.len() >= 2 {
                            pre.push(Operation::new("Tw", vec![Object::Real(n[0] as f32)]));
                            pre.push(Operation::new("Tc", vec![Object::Real(n[1] as f32)]));
                        }
                        pre.push(Operation::new("T*", vec![]));
                        (pre, op.operands.get(2).and_then(as_string))
                    }
                };
                let Some(bytes) = text else {
                    out.push(op.clone());
                    continue;
                };
                let mut items: Vec<Object> = Vec::new();
                let changed =
                    strip_string(bytes, step, &mut glyph, plan, &st, &mut items, &mut removed);
                if !changed {
                    out.push(op.clone());
                    continue;
                }
                out.extend(prefix);
                if !items.is_empty() {
                    out.push(Operation::new("TJ", vec![Object::Array(items)]));
                }
            }
            "TJ" => {
                let Some(Object::Array(arr)) = op.operands.first() else {
                    out.push(op.clone());
                    continue;
                };
                let mut items: Vec<Object> = Vec::new();
                let mut changed = false;
                for item in arr {
                    match item {
                        Object::String(bytes, _) => {
                            changed |= strip_string(
                                bytes,
                                step,
                                &mut glyph,
                                plan,
                                &st,
                                &mut items,
                                &mut removed,
                            );
                        }
                        other => items.push(other.clone()),
                    }
                }
                if !changed {
                    out.push(op.clone());
                } else if !items.is_empty() {
                    out.push(Operation::new("TJ", vec![Object::Array(items)]));
                }
            }
            _ => out.push(op.clone()),
        }
    }
    (out, removed, glyph)
}

/// Só conta os glifos da página, sem mexer em nada.
pub fn count_glyphs(ops: &[Operation], two_byte: &dyn Fn(&str) -> bool) -> usize {
    let mut plan = |_: usize| None;
    strip_glyphs(ops, two_byte, &mut plan).2
}

// ── Tinta ──────────────────────────────────────────────────────────────

/// Operações que pintam as tarjas por cima do que sobrou.
pub fn bar_ops(rects: &[Rect], color: [f32; 3]) -> Vec<Operation> {
    let mut ops = vec![Operation::new("q", vec![])];
    ops.push(Operation::new(
        "rg",
        color.iter().map(|c| Object::Real(*c)).collect(),
    ));
    for r in rects {
        ops.push(Operation::new(
            "re",
            vec![
                Object::Real(r[0]),
                Object::Real(r[1]),
                Object::Real(r[2] - r[0]),
                Object::Real(r[3] - r[1]),
            ],
        ));
        ops.push(Operation::new("f", vec![]));
    }
    ops.push(Operation::new("Q", vec![]));
    ops
}

// ── Documento ──────────────────────────────────────────────────────────

fn out_path(opts: &Options) -> anyhow::Result<PathBuf> {
    let inp = Path::new(opts.input.trim());
    let dir = if opts.output_dir.trim().is_empty() {
        inp.parent().map(Path::to_path_buf).unwrap_or_default()
    } else {
        PathBuf::from(opts.output_dir.trim())
    };
    std::fs::create_dir_all(&dir)?;
    let stem = inp
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "documento".into());
    let suffix = if opts.suffix.trim().is_empty() {
        "-tarjado"
    } else {
        opts.suffix.trim()
    };
    Ok(dir.join(format!("{}{}.pdf", stem, suffix)))
}

fn load(input: &str, password: &str) -> anyhow::Result<Document> {
    let doc = if password.is_empty() {
        Document::load(input)
    } else {
        let bytes = std::fs::read(input)?;
        Document::load_mem_with_options(&bytes, lopdf::LoadOptions::with_password(password))
    };
    doc.map_err(|e| anyhow!("não abri o PDF: {}", e))
}

/// Fontes da página que usam código de 2 bytes (Type0/Identity).
fn two_byte_fonts(doc: &Document, page_id: lopdf::ObjectId) -> Vec<String> {
    let mut out = Vec::new();
    let Ok((_, res_ids)) = doc.get_page_resources(page_id) else {
        return out;
    };
    let mut dicts: Vec<lopdf::Dictionary> = Vec::new();
    if let Ok(page) = doc.get_dictionary(page_id) {
        if let Ok(r) = page.get(b"Resources") {
            if let Ok(d) = r.as_dict() {
                dicts.push(d.clone());
            }
        }
    }
    for id in res_ids {
        if let Ok(d) = doc.get_dictionary(id) {
            dicts.push(d.clone());
        }
    }
    for d in dicts {
        let Ok(fonts) = d.get(b"Font").and_then(Object::as_dict) else {
            continue;
        };
        for (name, obj) in fonts.iter() {
            let font = match obj {
                Object::Reference(r) => doc.get_dictionary(*r).ok().cloned(),
                Object::Dictionary(fd) => Some(fd.clone()),
                _ => None,
            };
            let Some(font) = font else { continue };
            let subtype = font
                .get(b"Subtype")
                .and_then(Object::as_name)
                .map(|n| String::from_utf8_lossy(n).to_string())
                .unwrap_or_default();
            if subtype == "Type0" {
                out.push(String::from_utf8_lossy(name).to_string());
            }
        }
    }
    out
}

fn media_box(doc: &Document, page_id: lopdf::ObjectId, w: f32, h: f32) -> [f32; 4] {
    let read = |d: &lopdf::Dictionary| -> Option<[f32; 4]> {
        let arr = d.get(b"MediaBox").ok()?.as_array().ok()?;
        if arr.len() != 4 {
            return None;
        }
        let mut v = [0f32; 4];
        for (i, o) in arr.iter().enumerate() {
            v[i] = match o {
                Object::Integer(n) => *n as f32,
                Object::Real(r) => *r,
                _ => return None,
            };
        }
        Some(v)
    };
    let mut id = page_id;
    for _ in 0..8 {
        let Ok(d) = doc.get_dictionary(id) else { break };
        if let Some(v) = read(d) {
            return v;
        }
        match d.get(b"Parent").and_then(Object::as_reference) {
            Ok(p) => id = p,
            Err(_) => break,
        }
    }
    [0.0, 0.0, w, h]
}

/// Troca o conteúdo da página por uma imagem que já vem com a tarja pintada.
fn rasterize_page(
    doc: &mut Document,
    page_id: lopdf::ObjectId,
    jpeg: Vec<u8>,
    media: [f32; 4],
) -> anyhow::Result<()> {
    let info = super::jpeg_pdf::jpeg_info(&jpeg)?;
    let space = if info.components == 1 {
        "DeviceGray"
    } else if info.components == 4 {
        "DeviceCMYK"
    } else {
        "DeviceRGB"
    };
    let img_id = doc.add_object(Stream::new(
        dictionary! {
            "Type" => "XObject",
            "Subtype" => "Image",
            "Width" => info.width as i64,
            "Height" => info.height as i64,
            "ColorSpace" => space,
            "BitsPerComponent" => 8_i64,
            "Filter" => "DCTDecode",
        },
        jpeg,
    ));
    let (w, h) = (media[2] - media[0], media[3] - media[1]);
    let content = Content {
        operations: vec![
            Operation::new("q", vec![]),
            Operation::new(
                "cm",
                vec![
                    Object::Real(w),
                    Object::Real(0.0),
                    Object::Real(0.0),
                    Object::Real(h),
                    Object::Real(media[0]),
                    Object::Real(media[1]),
                ],
            ),
            Operation::new("Do", vec!["OGRXimg".into()]),
            Operation::new("Q", vec![]),
        ],
    };
    let stream_id = doc.add_object(Stream::new(
        dictionary! {},
        content.encode().map_err(|e| anyhow!("conteúdo: {}", e))?,
    ));
    let res_id = doc.add_object(Object::Dictionary(dictionary! {
        "XObject" => dictionary! { "OGRXimg" => Object::Reference(img_id) },
    }));
    let page = doc
        .get_dictionary_mut(page_id)
        .map_err(|e| anyhow!("página: {}", e))?;
    page.set("Contents", Object::Reference(stream_id));
    page.set("Resources", Object::Reference(res_id));
    page.remove(b"Annots");
    Ok(())
}

/// Tira as anotações que encostam na região: um campo de formulário ou uma
/// nota grudada ali carrega o texto de novo.
fn drop_annots(doc: &mut Document, page_id: lopdf::ObjectId, rects: &[Rect]) {
    let annots = match doc.get_dictionary(page_id).and_then(|p| p.get(b"Annots")) {
        Ok(Object::Array(a)) => a.clone(),
        Ok(Object::Reference(r)) => match doc.get_object(*r) {
            Ok(Object::Array(a)) => a.clone(),
            _ => return,
        },
        _ => return,
    };
    let hits = |dict: &lopdf::Dictionary| -> bool {
        let Ok(arr) = dict.get(b"Rect").and_then(Object::as_array) else {
            return false;
        };
        if arr.len() != 4 {
            return false;
        }
        let mut v = [0f32; 4];
        for (i, o) in arr.iter().enumerate() {
            v[i] = match o {
                Object::Integer(n) => *n as f32,
                Object::Real(r) => *r,
                _ => return false,
            };
        }
        let (cx, cy) = ((v[0] + v[2]) / 2.0, (v[1] + v[3]) / 2.0);
        any_inside(rects, cx, cy)
    };
    let kept: Vec<Object> = annots
        .into_iter()
        .filter(|o| {
            let dict = match o {
                Object::Reference(r) => doc.get_dictionary(*r).ok().cloned(),
                Object::Dictionary(d) => Some(d.clone()),
                _ => None,
            };
            match dict {
                Some(d) => !hits(&d),
                None => true,
            }
        })
        .collect();
    if let Ok(page) = doc.get_dictionary_mut(page_id) {
        if kept.is_empty() {
            page.remove(b"Annots");
        } else {
            page.set("Annots", Object::Array(kept));
        }
    }
}

fn set_content(
    doc: &mut Document,
    page_id: lopdf::ObjectId,
    ops: Vec<Operation>,
) -> anyhow::Result<()> {
    let data = Content { operations: ops }
        .encode()
        .map_err(|e| anyhow!("conteúdo: {}", e))?;
    let stream_id = doc.add_object(Stream::new(dictionary! {}, data));
    let page = doc
        .get_dictionary_mut(page_id)
        .map_err(|e| anyhow!("página: {}", e))?;
    page.set("Contents", Object::Reference(stream_id));
    Ok(())
}

fn report(p: &super::ProgressFn, stage: &str, done: u64, total: u64, msg: Option<String>) {
    super::report(p, "pdf-redact", stage, done, Some(total), msg);
}

/// Regiões por página: as áreas pedidas mais o que os termos acharem.
fn plan_rects(
    pages: &[pdf::RawPage],
    opts: &Options,
    wanted: &[usize],
) -> (BTreeMap<usize, Vec<Rect>>, usize) {
    let mut map: BTreeMap<usize, Vec<Rect>> = BTreeMap::new();
    for a in &opts.areas {
        if a.page == 0 {
            continue;
        }
        map.entry(a.page)
            .or_default()
            .push(rect_of(a, opts.padding));
    }
    let mut found = 0usize;
    let terms: Vec<&String> = opts.terms.iter().filter(|t| !t.trim().is_empty()).collect();
    if terms.is_empty() {
        return (map, 0);
    }
    for page in pages {
        if !wanted.contains(&page.number) {
            continue;
        }
        let chars: Vec<char> = page.chars.iter().map(|c| c.ch).collect();
        for term in &terms {
            for (a, b) in find_term(&chars, term) {
                let boxes: Vec<Rect> = page.chars[a..b]
                    .iter()
                    .filter(|c| !c.empty_box())
                    .map(|c| [c.x0, c.y0, c.x1, c.y1])
                    .collect();
                if let Some(r) = union(&boxes) {
                    found += 1;
                    let pad = opts.padding.max(1.0);
                    map.entry(page.number).or_default().push([
                        r[0] - pad,
                        r[1] - pad,
                        r[2] + pad,
                        r[3] + pad,
                    ]);
                }
            }
        }
    }
    (map, found)
}

/// Apaga da página os glifos que caem dentro das regiões e pinta a tarja.
/// Devolve `(deu certo, glifos apagados, nota)`; `false` quer dizer que a
/// página tem que cair para pixel.
fn redact_page(
    doc: &mut Document,
    page_id: lopdf::ObjectId,
    raw: &pdf::RawPage,
    rects: &[Rect],
    bar: bool,
) -> anyhow::Result<(bool, usize, String)> {
    if !raw.generated_known {
        return Ok((false, 0, "PDFium sem FPDFText_IsGenerated".into()));
    }
    // Centro e largura de cada glifo real, na ordem do content stream.
    let visible: Vec<(f32, f32, f32)> = raw
        .chars
        .iter()
        .filter(|c| !c.generated)
        .map(|c| {
            let (cx, cy) = c.center();
            (cx, cy, c.x1 - c.x0)
        })
        .collect();
    let two = two_byte_fonts(doc, page_id);
    let is_two = |name: &str| two.iter().any(|f| f == name);
    let data = doc.get_page_content(page_id);
    let Ok(content) = Content::decode(&data) else {
        return Ok((false, 0, "content stream que o lopdf não decodifica".into()));
    };
    let ops = content.operations;
    let count = count_glyphs(&ops, &is_two);
    if count != visible.len() {
        return Ok((
            false,
            0,
            format!("{} glifos no fluxo x {} no PDFium", count, visible.len()),
        ));
    }
    let mut plan = |i: usize| -> Option<f32> {
        let (cx, cy, w) = *visible.get(i)?;
        if any_inside(rects, cx, cy) {
            Some(w.max(0.0))
        } else {
            None
        }
    };
    let (mut new_ops, removed, _) = strip_glyphs(&ops, &is_two, &mut plan);
    if bar {
        new_ops.extend(bar_ops(rects, [0.0, 0.0, 0.0]));
    }
    set_content(doc, page_id, new_ops)?;
    drop_annots(doc, page_id, rects);
    Ok((true, removed, format!("{} caracteres apagados", removed)))
}

pub fn run(opts: &Options, progress: &super::ProgressFn) -> anyhow::Result<RedactResult> {
    if opts.input.trim().is_empty() {
        return Err(anyhow!("escolha o PDF"));
    }
    let pw = if opts.password.is_empty() {
        None
    } else {
        Some(opts.password.as_str())
    };
    let dpi = if opts.dpi == 0 { 150 } else { opts.dpi };
    let quality = if opts.quality == 0 { 88 } else { opts.quality };

    report(progress, "progress", 0, 4, Some("lendo o PDF".into()));
    let pages = pdf::read_raw_chars(&opts.input, pw, "")?;
    let total_pages = pages.len();
    let wanted = pdf::parse_ranges(&opts.pages, total_pages)?;
    let (rects, terms_found) = plan_rects(&pages, opts, &wanted);
    if rects.is_empty() {
        return Err(anyhow!(
            "nada para tarjar: marque uma região ou digite um termo"
        ));
    }
    let areas_total: usize = rects.values().map(Vec::len).sum();

    let output = out_path(opts)?;
    let mut doc = load(&opts.input, &opts.password)?;
    let ids: BTreeMap<u32, lopdf::ObjectId> = doc.get_pages();
    let mut by_page: Vec<PageReport> = Vec::new();
    let mut chars_removed = 0usize;
    let mut fallback: Vec<usize> = Vec::new();
    let force_raster = opts.mode.trim() == "raster";

    report(progress, "progress", 1, 4, Some("apagando o texto".into()));
    for (no, rs) in &rects {
        let Some(page_id) = ids.get(&(*no as u32)).copied() else {
            continue;
        };
        let raw = pages.iter().find(|p| p.number == *no);
        let (surgical, removed, note) = if force_raster {
            (false, 0, "modo pixel".to_string())
        } else {
            match raw {
                Some(raw) => redact_page(&mut doc, page_id, raw, rs, opts.bar)?,
                None => (false, 0, "página não abriu no PDFium".to_string()),
            }
        };
        if !surgical {
            fallback.push(*no);
        }
        chars_removed += removed;
        by_page.push(PageReport {
            page: *no,
            areas: rs.len(),
            chars_removed: removed,
            mode: if surgical { "text" } else { "raster" }.into(),
            note,
        });
    }

    doc.save(&output)
        .map_err(|e| anyhow!("não gravei {}: {}", output.display(), e))?;

    // Conferência: o que ainda aparece dentro da região tem que virar pixel.
    report(progress, "progress", 2, 4, Some("conferindo".into()));
    let mut leftovers = 0usize;
    let out_s = output.to_string_lossy().to_string();
    if let Ok(after) = pdf::read_raw_chars(&out_s, None, "") {
        for page in &after {
            let Some(rs) = rects.get(&page.number) else {
                continue;
            };
            let n = page
                .chars
                .iter()
                .filter(|c| !c.ch.is_whitespace() && !c.empty_box())
                .filter(|c| {
                    let (cx, cy) = c.center();
                    any_inside(rs, cx, cy)
                })
                .count();
            if n > 0 {
                leftovers += n;
                if !fallback.contains(&page.number) {
                    fallback.push(page.number);
                }
            }
        }
    }

    if !fallback.is_empty() {
        report(
            progress,
            "progress",
            3,
            4,
            Some(format!("{} página(s) em pixel", fallback.len())),
        );
        let mut doc2 = load(&out_s, "")?;
        let ids2: BTreeMap<u32, lopdf::ObjectId> = doc2.get_pages();
        for no in &fallback {
            let Some(page_id) = ids2.get(&(*no as u32)).copied() else {
                continue;
            };
            let raw = pages.iter().find(|p| p.number == *no);
            let (w, h) = raw.map(|p| (p.width, p.height)).unwrap_or((595.0, 842.0));
            let media = media_box(&doc2, page_id, w, h);
            // Renderiza do original: a tarja entra na imagem, então nada do que
            // estava por baixo sobrevive.
            let (jpeg, _, _) = pdf::page_jpeg(&opts.input, pw, *no, dpi, quality)?;
            let jpeg = if opts.bar {
                paint_bars(
                    &jpeg,
                    rects.get(no).map(Vec::as_slice).unwrap_or(&[]),
                    media,
                    quality,
                )?
            } else {
                jpeg
            };
            rasterize_page(&mut doc2, page_id, jpeg, media)?;
            if let Some(r) = by_page.iter_mut().find(|r| r.page == *no) {
                r.mode = "raster".into();
                if r.note.is_empty() {
                    r.note = "página virou imagem".into();
                }
            }
        }
        doc2.save(&output)
            .map_err(|e| anyhow!("não gravei {}: {}", output.display(), e))?;
    }

    let verified = match pdf::read_raw_chars(&out_s, None, "") {
        Ok(after) => !after.iter().any(|page| {
            rects.get(&page.number).is_some_and(|rs| {
                page.chars.iter().any(|c| {
                    let (cx, cy) = c.center();
                    !c.ch.is_whitespace() && !c.empty_box() && any_inside(rs, cx, cy)
                })
            })
        }),
        Err(_) => false,
    };

    let bytes = std::fs::metadata(&output).map(|m| m.len()).unwrap_or(0);
    report(progress, "done", 4, 4, None);
    Ok(RedactResult {
        input: opts.input.clone(),
        output: out_s,
        pages: total_pages,
        areas: areas_total,
        chars_removed,
        terms_found,
        leftovers,
        verified,
        bytes,
        by_page,
    })
}

/// Pinta as tarjas na imagem da página (pixel), convertendo ponto → pixel.
fn paint_bars(
    jpeg: &[u8],
    rects: &[Rect],
    media: [f32; 4],
    quality: u8,
) -> anyhow::Result<Vec<u8>> {
    if rects.is_empty() {
        return Ok(jpeg.to_vec());
    }
    let img = image::load_from_memory(jpeg)?.to_rgb8();
    let (pw, ph) = (img.width() as f32, img.height() as f32);
    let (w, h) = (media[2] - media[0], media[3] - media[1]);
    if w <= 0.0 || h <= 0.0 {
        return Ok(jpeg.to_vec());
    }
    let mut img = img;
    for r in rects {
        let x0 = ((r[0] - media[0]) / w * pw).floor().max(0.0) as u32;
        let x1 = ((r[2] - media[0]) / w * pw).ceil().min(pw) as u32;
        // O eixo Y da página cresce para cima; o da imagem, para baixo.
        let y0 = ((media[3] - r[3]) / h * ph).floor().max(0.0) as u32;
        let y1 = ((media[3] - r[1]) / h * ph).ceil().min(ph) as u32;
        for y in y0..y1 {
            for x in x0..x1 {
                img.put_pixel(x, y, image::Rgb([0, 0, 0]));
            }
        }
    }
    let mut buf = Vec::new();
    let mut cursor = std::io::Cursor::new(&mut buf);
    let mut enc =
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut cursor, quality.clamp(10, 100));
    enc.encode_image(&img)?;
    Ok(buf)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ops_of(src: &str) -> Vec<Operation> {
        Content::decode(src.as_bytes())
            .expect("content de teste")
            .operations
    }

    fn one_byte(_: &str) -> bool {
        false
    }

    #[test]
    fn area_virou_retangulo_com_folga() {
        let a = Area {
            page: 1,
            x: 10.0,
            y: 20.0,
            width: -6.0,
            height: 8.0,
        };
        assert_eq!(rect_of(&a, 0.0), [4.0, 20.0, 10.0, 28.0]);
        assert_eq!(rect_of(&a, 2.0), [2.0, 18.0, 12.0, 30.0]);
        assert!(inside(&rect_of(&a, 0.0), 5.0, 21.0));
        assert!(!inside(&rect_of(&a, 0.0), 11.0, 21.0));
    }

    #[test]
    fn termo_acha_sem_acento_e_sem_caixa() {
        let chars: Vec<char> = "Contrato do JOÃO Silva e do joao Souza".chars().collect();
        let hits = find_term(&chars, "João");
        assert_eq!(hits.len(), 2, "{:?}", hits);
        assert_eq!(
            chars[hits[0].0..hits[0].1].iter().collect::<String>(),
            "JOÃO"
        );
        assert!(find_term(&chars, "  ").is_empty());
    }

    #[test]
    fn conta_glifos_do_fluxo() {
        let ops = ops_of("BT /F1 12 Tf (ABC) Tj [(DE) -120 (F)] TJ ET");
        assert_eq!(count_glyphs(&ops, &one_byte), 6);
        // Fonte composta: dois bytes por glifo.
        let two = |_: &str| true;
        let ops2 = ops_of("BT /F1 12 Tf <00410042> Tj ET");
        assert_eq!(count_glyphs(&ops2, &two), 2);
    }

    #[test]
    fn tira_so_o_glifo_marcado_e_compensa_o_avanco() {
        let ops = ops_of("BT /F1 10 Tf 0 0 Td (SEGREDO ok) Tj ET");
        // Tira "SEGREDO" (7 primeiros glifos), cada um com 6 pt de largura.
        let mut plan = |i: usize| if i < 7 { Some(6.0f32) } else { None };
        let (new, removed, total) = strip_glyphs(&ops, &one_byte, &mut plan);
        assert_eq!((removed, total), (7, 10));
        let text = new
            .iter()
            .filter(|o| o.operator == "TJ")
            .flat_map(|o| o.operands.clone())
            .map(|o| format!("{:?}", o))
            .collect::<String>();
        assert!(text.contains("ok"), "{}", text);
        assert!(!text.contains("SEGREDO"), "{}", text);
        // 7 glifos x 6 pt / 10 pt de corpo = 4.2 em espaço de texto → -4200.
        assert!(text.contains("-4200"), "sem o ajuste de avanço: {}", text);
    }

    #[test]
    fn sem_marca_nao_mexe_no_fluxo() {
        let ops = ops_of("BT /F1 10 Tf (tudo bem) Tj ET");
        let mut plan = |_: usize| None;
        let (new, removed, _) = strip_glyphs(&ops, &one_byte, &mut plan);
        assert_eq!(removed, 0);
        assert_eq!(new.len(), ops.len());
        assert_eq!(new[2].operator, "Tj");
    }

    #[test]
    fn glifo_de_tj_com_kerning_sai_inteiro() {
        let ops = ops_of("BT /F1 10 Tf [(AB) -50 (CD)] TJ ET");
        let mut plan = |i: usize| if i == 2 { Some(5.0) } else { None };
        let (new, removed, total) = strip_glyphs(&ops, &one_byte, &mut plan);
        assert_eq!((removed, total), (1, 4));
        let dump = format!("{:?}", new);
        assert!(dump.contains("AB"), "{}", dump);
        assert!(!dump.contains("CD"), "{}", dump);
        assert!(dump.contains("[68]") || dump.contains("D"), "{}", dump);
    }

    #[test]
    fn a_tarja_vira_retangulo_preto() {
        let ops = bar_ops(&[[10.0, 20.0, 40.0, 30.0]], [0.0, 0.0, 0.0]);
        let dump = format!("{:?}", ops);
        assert!(dump.contains("\"rg\""), "{}", dump);
        assert!(dump.contains("\"re\""), "{}", dump);
        assert!(dump.contains("\"f\""), "{}", dump);
        // largura 30, altura 10
        assert_eq!(
            ops.iter()
                .find(|o| o.operator == "re")
                .map(|o| o.operands.len()),
            Some(4)
        );
    }

    #[test]
    fn uniao_de_caixas() {
        let r = union(&[[1.0, 1.0, 2.0, 2.0], [5.0, 0.0, 6.0, 3.0]]);
        assert_eq!(r, Some([1.0, 0.0, 6.0, 3.0]));
        assert_eq!(union(&[]), None);
    }

    /// PDF montado à mão, tarjado de verdade, e depois passado pelo próprio
    /// `pdf-redaction-check`: ele não pode achar nada escondido.
    /// `cargo test -p omniget-core --lib -- --ignored live_redact`
    #[test]
    #[ignore]
    fn live_redact_deixa_o_check_limpo() {
        let content =
            b"BT /F1 24 Tf 40 150 Td (VISIVEL) Tj ET\nBT /F1 24 Tf 40 100 Td (SEGREDO) Tj ET\n";
        let mut body = String::from("%PDF-1.4\n");
        body.push_str("1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n");
        body.push_str("2 0 obj\n<< /Type /Pages /Kids [3 0 R] /Count 1 >>\nendobj\n");
        body.push_str(
            "3 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 300 200] /Resources << /Font << /F1 4 0 R >> >> /Contents 5 0 R >>\nendobj\n",
        );
        body.push_str("4 0 obj\n<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>\nendobj\n");
        body.push_str(&format!(
            "5 0 obj\n<< /Length {} >>\nstream\n{}endstream\nendobj\n",
            content.len(),
            String::from_utf8_lossy(content)
        ));
        let (bytes, _) = super::super::pdf_repair::rebuild_xref(body.as_bytes()).unwrap();

        let dir = std::env::temp_dir().join(format!("omniget-redact-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("original.pdf");
        std::fs::write(&path, &bytes).unwrap();

        let opts = Options {
            input: path.to_string_lossy().to_string(),
            password: String::new(),
            areas: vec![Area {
                page: 1,
                x: 34.0,
                y: 94.0,
                width: 140.0,
                height: 34.0,
            }],
            terms: Vec::new(),
            pages: String::new(),
            mode: String::new(),
            padding: 0.0,
            bar: true,
            dpi: 120,
            quality: 90,
            output_dir: String::new(),
            suffix: String::new(),
        };
        let out = run(&opts, &super::super::noop_progress()).expect("tarjar");
        eprintln!("{:?}", out);
        assert!(out.verified, "sobrou texto dentro da região: {:?}", out);

        // E o juiz de verdade: o próprio conferidor de tarja.
        let report = pdf::redaction_check(&out.output, 110, "").unwrap();
        let found: Vec<&str> = report.runs.iter().map(|r| r.text.as_str()).collect();
        assert!(
            !found.iter().any(|t| t.contains("SEGREDO")),
            "o check ainda acha o segredo: {:?}",
            found
        );
        assert_eq!(report.chars_hidden, 0, "escondeu {:?}", found);
        // O texto que estava fora da tarja continua lá.
        let txt = pdf::to_text(&out.output, "", false, "").unwrap();
        assert!(txt.text.contains("VISIVEL"), "{}", txt.text);
        assert!(!txt.text.contains("SEGREDO"), "{}", txt.text);
        let _ = std::fs::remove_dir_all(&dir);
    }
}

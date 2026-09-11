//! Escrever estrutura de PDF: senha, marca d'água/numeração, corte de margem
//! e sumário. O PDFium só lê e renderiza — para mexer no arquivo em si quem
//! entra é o `lopdf` (MIT, Rust puro), sem binário externo.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::anyhow;
use lopdf::content::{Content, Operation};
use lopdf::encryption::crypt_filters::{Aes128CryptFilter, CryptFilter};
use lopdf::encryption::{EncryptionState, EncryptionVersion, Permissions};
use lopdf::{dictionary, Document, Object, Stream};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize)]
pub struct WriteItem {
    pub input: String,
    pub output: Option<String>,
    pub pages: usize,
    pub note: String,
    pub ok: bool,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct WriteResult {
    pub items: Vec<WriteItem>,
}

fn out_path(
    input: &str,
    output_dir: &str,
    suffix: &str,
    fallback: &str,
) -> anyhow::Result<PathBuf> {
    let inp = Path::new(input);
    let dir = if output_dir.trim().is_empty() {
        inp.parent().map(|p| p.to_path_buf()).unwrap_or_default()
    } else {
        PathBuf::from(output_dir.trim())
    };
    std::fs::create_dir_all(&dir)?;
    let stem = inp
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "documento".into());
    let suffix = if suffix.is_empty() { fallback } else { suffix };
    Ok(dir.join(format!("{}{}.pdf", stem, suffix)))
}

// ── Senha e permissões ─────────────────────────────────────────────────

#[derive(Debug, Clone, Deserialize)]
pub struct PasswordOptions {
    pub inputs: Vec<String>,
    /// "add" | "remove"
    #[serde(default = "default_add")]
    pub mode: String,
    /// Senha para abrir. Vazia = só trava permissão, abre sem pedir nada.
    #[serde(default)]
    pub user_password: String,
    /// Senha de dono (a que libera as permissões). Vazia = usa a do usuário.
    #[serde(default)]
    pub owner_password: String,
    /// Senha atual, para o modo "remove" ou para reabrir um arquivo cifrado.
    #[serde(default)]
    pub current_password: String,
    #[serde(default = "default_true")]
    pub allow_print: bool,
    #[serde(default = "default_true")]
    pub allow_copy: bool,
    #[serde(default = "default_true")]
    pub allow_modify: bool,
    #[serde(default = "default_true")]
    pub allow_annotate: bool,
    #[serde(default)]
    pub output_dir: String,
    #[serde(default)]
    pub suffix: String,
}

fn default_add() -> String {
    "add".into()
}
fn default_true() -> bool {
    true
}

/// O acesso para leitor de tela é sempre liberado: a partir do PDF 2.0 esse
/// bit tem que ficar ligado, e travar acessibilidade não protege nada.
pub fn permissions_of(opts: &PasswordOptions) -> Permissions {
    let mut p = Permissions::COPYABLE_FOR_ACCESSIBILITY;
    if opts.allow_print {
        p |= Permissions::PRINTABLE | Permissions::PRINTABLE_IN_HIGH_QUALITY;
    }
    if opts.allow_copy {
        p |= Permissions::COPYABLE;
    }
    if opts.allow_modify {
        p |= Permissions::MODIFIABLE | Permissions::ASSEMBLABLE;
    }
    if opts.allow_annotate {
        p |= Permissions::ANNOTABLE | Permissions::FILLABLE;
    }
    p
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

fn password_one(opts: &PasswordOptions, input: &str) -> anyhow::Result<WriteItem> {
    let mut doc = load(input, &opts.current_password)?;
    let pages = doc.get_pages().len();
    let note;
    if opts.mode == "remove" {
        if !doc.is_encrypted() && opts.current_password.is_empty() {
            return Err(anyhow!("esse PDF não tem senha"));
        }
        // `load_mem_with_password` já devolve o documento decifrado; basta
        // apagar o dicionário de cifra para o arquivo sair aberto.
        doc.encryption_state = None;
        doc.trailer.remove(b"Encrypt");
        note = "senha removida".to_string();
    } else {
        if opts.user_password.is_empty() && opts.owner_password.is_empty() {
            return Err(anyhow!("defina pelo menos uma senha"));
        }
        let owner = if opts.owner_password.is_empty() {
            opts.user_password.as_str()
        } else {
            opts.owner_password.as_str()
        };
        // A derivação da chave usa o /ID do arquivo. PDF gerado por
        // ferramenta simples costuma não ter: sem isso a cifra falha com um
        // "decryption error" que não diz nada.
        if doc.trailer.get(b"ID").is_err() {
            use rand::RngExt;
            let mut id = [0u8; 16];
            rand::rng().fill(&mut id);
            let id = hex::encode(id).into_bytes();
            doc.trailer.set(
                "ID",
                Object::Array(vec![
                    Object::string_literal(id.clone()),
                    Object::string_literal(id),
                ]),
            );
        }
        let filter: Arc<dyn CryptFilter> = Arc::new(Aes128CryptFilter);
        let version = EncryptionVersion::V4 {
            document: &doc,
            encrypt_metadata: true,
            crypt_filters: BTreeMap::from([(b"StdCF".to_vec(), filter)]),
            stream_filter: b"StdCF".to_vec(),
            string_filter: b"StdCF".to_vec(),
            owner_password: owner,
            user_password: &opts.user_password,
            permissions: permissions_of(opts),
        };
        let state =
            EncryptionState::try_from(version).map_err(|e| anyhow!("não montei a cifra: {}", e))?;
        doc.encrypt(&state)
            .map_err(|e| anyhow!("não cifrei: {}", e))?;
        note = if opts.user_password.is_empty() {
            "permissões travadas (abre sem senha)".into()
        } else {
            "AES-128, senha para abrir".into()
        };
    }
    let out = out_path(
        input,
        &opts.output_dir,
        &opts.suffix,
        if opts.mode == "remove" {
            "-sem-senha"
        } else {
            "-protegido"
        },
    )?;
    doc.save(&out).map_err(|e| anyhow!("não gravei: {}", e))?;
    Ok(WriteItem {
        input: input.to_string(),
        output: Some(out.to_string_lossy().to_string()),
        pages,
        note,
        ok: true,
        error: None,
    })
}

pub fn password(opts: &PasswordOptions, progress: &super::ProgressFn) -> WriteResult {
    run_each(&opts.inputs, "pdf-password", progress, |input| {
        password_one(opts, input)
    })
}

// ── Marca d'água e numeração ───────────────────────────────────────────

#[derive(Debug, Clone, Deserialize)]
pub struct WatermarkOptions {
    pub inputs: Vec<String>,
    /// "diagonal" | "header" | "footer" | "bates"
    #[serde(default = "default_diagonal")]
    pub mode: String,
    #[serde(default)]
    pub text: String,
    #[serde(default = "default_size")]
    pub font_size: f32,
    /// 0-100; vira o /ca e /CA do ExtGState.
    #[serde(default = "default_opacity")]
    pub opacity: u32,
    /// 0 = preto, 100 = branco.
    #[serde(default = "default_gray")]
    pub gray: u32,
    /// Numeração Bates: primeiro número e prefixo.
    #[serde(default = "default_start")]
    pub start_number: u32,
    #[serde(default)]
    pub prefix: String,
    #[serde(default)]
    pub output_dir: String,
    #[serde(default)]
    pub suffix: String,
}

fn default_diagonal() -> String {
    "diagonal".into()
}
fn default_size() -> f32 {
    48.0
}
fn default_opacity() -> u32 {
    15
}
fn default_gray() -> u32 {
    50
}
fn default_start() -> u32 {
    1
}

/// Largura aproximada de um texto em Helvetica. Sem a tabela de métricas da
/// fonte não dá para acertar na vírgula, e para centralizar carimbo basta.
pub fn text_width(text: &str, font_size: f32) -> f32 {
    text.chars()
        .map(|c| match c {
            'i' | 'l' | 'j' | '.' | ',' | '\'' | '|' | ' ' => 0.28,
            'm' | 'w' | 'M' | 'W' => 0.85,
            c if c.is_ascii_uppercase() => 0.68,
            c if c.is_ascii_digit() => 0.56,
            _ => 0.52,
        })
        .sum::<f32>()
        * font_size
}

/// Onde e como o texto é desenhado. Devolve a matriz de texto `Tm`, que é o
/// que carrega posição e rotação de uma vez.
pub fn placement(mode: &str, w: f32, h: f32, text_w: f32, size: f32) -> [f32; 6] {
    match mode {
        "header" => [1.0, 0.0, 0.0, 1.0, (w - text_w) / 2.0, h - size - 18.0],
        "footer" => [1.0, 0.0, 0.0, 1.0, (w - text_w) / 2.0, 18.0],
        "bates" => [1.0, 0.0, 0.0, 1.0, w - text_w - 24.0, 18.0],
        _ => {
            // Diagonal do canto inferior esquerdo ao superior direito.
            let angle = (h / w).atan();
            let (c, s) = (angle.cos(), angle.sin());
            let cx = (w - text_w * c) / 2.0;
            let cy = (h - text_w * s) / 2.0;
            [c, s, -s, c, cx, cy]
        }
    }
}

fn stamp_ops(text: &str, tm: [f32; 6], size: f32, gray: f32) -> Vec<Operation> {
    vec![
        Operation::new("q", vec![]),
        Operation::new("gs", vec!["OGWMgs".into()]),
        Operation::new("g", vec![gray.into()]),
        Operation::new("BT", vec![]),
        Operation::new("Tf", vec!["OGWMfont".into(), size.into()]),
        Operation::new(
            "Tm",
            tm.iter().map(|v| Object::Real(*v)).collect::<Vec<_>>(),
        ),
        Operation::new("Tj", vec![Object::string_literal(text)]),
        Operation::new("ET", vec![]),
        Operation::new("Q", vec![]),
    ]
}

/// Onde enfiar os recursos novos sem esconder os que a página herda: se a
/// página não tem `/Resources` própria, mexe na herdada em vez de criar uma.
fn resource_dict_id(doc: &mut Document, page_id: lopdf::ObjectId) -> Option<lopdf::ObjectId> {
    let inline = doc
        .get_dictionary(page_id)
        .ok()
        .and_then(|p| p.get(b"Resources").ok())
        .map(|r| r.as_reference().is_ok());
    match inline {
        // Já é referência: mexe no objeto apontado.
        Some(true) => doc
            .get_dictionary(page_id)
            .ok()
            .and_then(|p| p.get(b"Resources").ok())
            .and_then(|r| r.as_reference().ok()),
        // Dicionário inline na página: promove a objeto para poder editar.
        Some(false) => {
            let dict = doc
                .get_dictionary(page_id)
                .ok()
                .and_then(|p| p.get(b"Resources").ok())
                .and_then(|r| r.as_dict().ok())
                .cloned()?;
            let id = doc.add_object(Object::Dictionary(dict));
            if let Ok(page) = doc.get_dictionary_mut(page_id) {
                page.set("Resources", Object::Reference(id));
            }
            Some(id)
        }
        // Herdada do nó pai, ou inexistente.
        None => {
            let inherited = doc
                .get_page_resources(page_id)
                .ok()
                .and_then(|(_, ids)| ids.first().copied());
            match inherited {
                Some(id) => Some(id),
                None => {
                    let id = doc.add_object(Object::Dictionary(dictionary! {}));
                    if let Ok(page) = doc.get_dictionary_mut(page_id) {
                        page.set("Resources", Object::Reference(id));
                    }
                    Some(id)
                }
            }
        }
    }
}

fn watermark_one(opts: &WatermarkOptions, input: &str) -> anyhow::Result<WriteItem> {
    if opts.text.trim().is_empty() && opts.mode != "bates" {
        return Err(anyhow!("escreva o texto da marca d'água"));
    }
    let mut doc = load(input, "")?;
    let pages = doc.get_pages();
    let font_id = doc.add_object(dictionary! {
        "Type" => "Font",
        "Subtype" => "Type1",
        "BaseFont" => "Helvetica",
    });
    let alpha = (opts.opacity.min(100) as f32) / 100.0;
    let gs_id = doc.add_object(dictionary! {
        "Type" => "ExtGState",
        "ca" => alpha,
        "CA" => alpha,
    });
    let gray = (opts.gray.min(100) as f32) / 100.0;

    let ids: Vec<(u32, lopdf::ObjectId)> = pages.iter().map(|(n, id)| (*n, *id)).collect();
    for (index, (_, page_id)) in ids.iter().enumerate() {
        let (w, h) = page_size(&doc, *page_id);
        let text = if opts.mode == "bates" {
            format!("{}{:06}", opts.prefix, opts.start_number as usize + index)
        } else {
            opts.text.clone()
        };
        let size = if opts.mode == "bates" {
            opts.font_size.min(14.0)
        } else {
            opts.font_size
        };
        let tm = placement(&opts.mode, w, h, text_width(&text, size), size);
        let content = Content {
            operations: stamp_ops(&text, tm, size, gray),
        };
        let stream_id = doc.add_object(Stream::new(
            dictionary! {},
            content.encode().map_err(|e| anyhow!("conteúdo: {}", e))?,
        ));

        if let Some(res_id) = resource_dict_id(&mut doc, *page_id) {
            if let Ok(res) = doc.get_dictionary_mut(res_id) {
                let mut fonts = res
                    .get(b"Font")
                    .and_then(Object::as_dict)
                    .cloned()
                    .unwrap_or_default();
                fonts.set("OGWMfont", Object::Reference(font_id));
                res.set("Font", Object::Dictionary(fonts));
                let mut gss = res
                    .get(b"ExtGState")
                    .and_then(Object::as_dict)
                    .cloned()
                    .unwrap_or_default();
                gss.set("OGWMgs", Object::Reference(gs_id));
                res.set("ExtGState", Object::Dictionary(gss));
            }
        }
        // O carimbo entra por último para ficar por cima do conteúdo.
        if let Ok(page) = doc.get_dictionary_mut(*page_id) {
            let mut contents = match page.get(b"Contents") {
                Ok(Object::Reference(r)) => vec![Object::Reference(*r)],
                Ok(Object::Array(a)) => a.clone(),
                _ => vec![],
            };
            contents.push(Object::Reference(stream_id));
            page.set("Contents", Object::Array(contents));
        }
    }

    let out = out_path(
        input,
        &opts.output_dir,
        &opts.suffix,
        if opts.mode == "bates" {
            "-numerado"
        } else {
            "-marca"
        },
    )?;
    doc.save(&out).map_err(|e| anyhow!("não gravei: {}", e))?;
    Ok(WriteItem {
        input: input.to_string(),
        output: Some(out.to_string_lossy().to_string()),
        pages: ids.len(),
        note: format!("{} páginas carimbadas", ids.len()),
        ok: true,
        error: None,
    })
}

pub fn watermark(opts: &WatermarkOptions, progress: &super::ProgressFn) -> WriteResult {
    run_each(&opts.inputs, "pdf-watermark", progress, |input| {
        watermark_one(opts, input)
    })
}

// ── Corte de margem ────────────────────────────────────────────────────

#[derive(Debug, Clone, Deserialize)]
pub struct CropOptions {
    pub inputs: Vec<String>,
    /// "auto" (acha a tinta com o PDFium) | "fixed"
    #[serde(default = "default_auto")]
    pub mode: String,
    /// Respiro em volta do conteúdo, em pontos (modo auto).
    #[serde(default = "default_pad")]
    pub padding: f32,
    /// Corte fixo em pontos, no modo "fixed".
    #[serde(default)]
    pub left: f32,
    #[serde(default)]
    pub right: f32,
    #[serde(default)]
    pub top: f32,
    #[serde(default)]
    pub bottom: f32,
    /// Uma caixa só para o documento inteiro, em vez de uma por página: o
    /// texto não "dança" ao passar página no leitor.
    #[serde(default = "default_true")]
    pub uniform: bool,
    #[serde(default)]
    pub output_dir: String,
    #[serde(default)]
    pub suffix: String,
}

fn default_auto() -> String {
    "auto".into()
}
fn default_pad() -> f32 {
    6.0
}

fn page_size(doc: &Document, page_id: lopdf::ObjectId) -> (f32, f32) {
    let media = doc
        .get_dictionary(page_id)
        .ok()
        .and_then(|p| p.get(b"MediaBox").ok())
        .and_then(|m| m.as_array().ok())
        .map(|a| {
            a.iter()
                .filter_map(|v| {
                    v.as_float()
                        .ok()
                        .or_else(|| v.as_i64().ok().map(|i| i as f32))
                })
                .collect::<Vec<f32>>()
        })
        .filter(|v| v.len() == 4);
    match media {
        Some(v) => ((v[2] - v[0]).abs(), (v[3] - v[1]).abs()),
        // Carta/A4 é o palpite menos errado quando a página herda a MediaBox.
        None => (595.0, 842.0),
    }
}

fn media_box(doc: &Document, page_id: lopdf::ObjectId) -> [f32; 4] {
    doc.get_dictionary(page_id)
        .ok()
        .and_then(|p| p.get(b"MediaBox").ok())
        .and_then(|m| m.as_array().ok())
        .map(|a| {
            let v: Vec<f32> = a
                .iter()
                .filter_map(|x| {
                    x.as_float()
                        .ok()
                        .or_else(|| x.as_i64().ok().map(|i| i as f32))
                })
                .collect();
            if v.len() == 4 {
                [v[0], v[1], v[2], v[3]]
            } else {
                [0.0, 0.0, 595.0, 842.0]
            }
        })
        .unwrap_or([0.0, 0.0, 595.0, 842.0])
}

/// Aplica o respiro e nunca deixa a caixa sair da página.
pub fn clamp_box(ink: [f32; 4], media: [f32; 4], padding: f32) -> [f32; 4] {
    [
        (ink[0] - padding).max(media[0]),
        (ink[1] - padding).max(media[1]),
        (ink[2] + padding).min(media[2]),
        (ink[3] + padding).min(media[3]),
    ]
}

/// Junta as caixas de todas as páginas numa só (a união), para o corte ficar
/// igual do começo ao fim do documento.
pub fn union_box(boxes: &[[f32; 4]]) -> Option<[f32; 4]> {
    boxes.iter().copied().reduce(|a, b| {
        [
            a[0].min(b[0]),
            a[1].min(b[1]),
            a[2].max(b[2]),
            a[3].max(b[3]),
        ]
    })
}

fn crop_one(opts: &CropOptions, input: &str) -> anyhow::Result<WriteItem> {
    let mut doc = load(input, "")?;
    let pages: Vec<lopdf::ObjectId> = doc.get_pages().values().copied().collect();
    let mut boxes: Vec<[f32; 4]> = Vec::with_capacity(pages.len());

    if opts.mode == "auto" {
        let ink = super::pdf::ink_boxes(input, 96, 245)
            .map_err(|e| anyhow!("não consegui medir a tinta ({}); use o corte fixo", e))?;
        for (i, page_id) in pages.iter().enumerate() {
            let media = media_box(&doc, *page_id);
            boxes.push(match ink.get(i).and_then(|b| *b) {
                Some(b) => clamp_box(b, media, opts.padding),
                None => media,
            });
        }
    } else {
        for page_id in &pages {
            let m = media_box(&doc, *page_id);
            boxes.push([
                m[0] + opts.left,
                m[1] + opts.bottom,
                m[2] - opts.right,
                m[3] - opts.top,
            ]);
        }
    }
    if opts.uniform {
        if let Some(u) = union_box(&boxes) {
            boxes = vec![u; boxes.len()];
        }
    }

    let mut cropped = 0usize;
    for (page_id, b) in pages.iter().zip(&boxes) {
        if b[2] - b[0] < 20.0 || b[3] - b[1] < 20.0 {
            continue; // caixa degenerada: melhor não cortar
        }
        if let Ok(page) = doc.get_dictionary_mut(*page_id) {
            page.set(
                "CropBox",
                Object::Array(b.iter().map(|v| Object::Real(*v)).collect()),
            );
            cropped += 1;
        }
    }

    let out = out_path(input, &opts.output_dir, &opts.suffix, "-cortado")?;
    doc.save(&out).map_err(|e| anyhow!("não gravei: {}", e))?;
    Ok(WriteItem {
        input: input.to_string(),
        output: Some(out.to_string_lossy().to_string()),
        pages: pages.len(),
        note: format!("{} páginas cortadas", cropped),
        ok: true,
        error: None,
    })
}

pub fn crop(opts: &CropOptions, progress: &super::ProgressFn) -> WriteResult {
    run_each(&opts.inputs, "pdf-crop", progress, |input| {
        crop_one(opts, input)
    })
}

// ── Sumário (bookmarks) ────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OutlineEntry {
    pub title: String,
    /// Página 1-based.
    pub page: u32,
    /// 0 = raiz, 1 = filho, 2 = neto…
    #[serde(default)]
    pub level: u32,
}

#[derive(Debug, Clone, Deserialize)]
pub struct OutlineOptions {
    pub input: String,
    pub entries: Vec<OutlineEntry>,
    #[serde(default)]
    pub output_dir: String,
    #[serde(default)]
    pub suffix: String,
}

/// Nível maior que "o anterior + 1" não existe em árvore: normaliza para não
/// gerar sumário órfão.
pub fn normalize_levels(entries: &[OutlineEntry]) -> Vec<OutlineEntry> {
    let mut out: Vec<OutlineEntry> = Vec::with_capacity(entries.len());
    let mut previous = 0u32;
    for e in entries {
        let level = if out.is_empty() {
            0
        } else {
            e.level.min(previous + 1)
        };
        previous = level;
        out.push(OutlineEntry {
            title: e.title.clone(),
            page: e.page.max(1),
            level,
        });
    }
    out
}

/// Texto de PDF vem em UTF-16BE com BOM quando tem acento, e em PDFDoc/ASCII
/// quando não tem. Ler tudo como UTF-8 vira mojibake em português.
pub fn decode_pdf_text(bytes: &[u8]) -> String {
    if bytes.starts_with(&[0xFE, 0xFF]) {
        let units: Vec<u16> = bytes[2..]
            .chunks_exact(2)
            .map(|c| u16::from_be_bytes([c[0], c[1]]))
            .collect();
        String::from_utf16_lossy(&units)
    } else if bytes.starts_with(&[0xFF, 0xFE]) {
        let units: Vec<u16> = bytes[2..]
            .chunks_exact(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .collect();
        String::from_utf16_lossy(&units)
    } else {
        String::from_utf8_lossy(bytes).to_string()
    }
}

pub fn read_outline(input: &str) -> anyhow::Result<Vec<OutlineEntry>> {
    let doc = load(input, "")?;
    let pages: BTreeMap<lopdf::ObjectId, u32> =
        doc.get_pages().iter().map(|(n, id)| (*id, *n)).collect();
    let mut out = Vec::new();
    let root = doc
        .catalog()
        .ok()
        .and_then(|c| c.get(b"Outlines").ok())
        .and_then(|o| o.as_reference().ok());
    let Some(root) = root else { return Ok(out) };

    // Percorre a árvore em profundidade seguindo /First e /Next.
    fn walk(
        doc: &Document,
        id: lopdf::ObjectId,
        level: u32,
        pages: &BTreeMap<lopdf::ObjectId, u32>,
        out: &mut Vec<OutlineEntry>,
        depth: u32,
    ) {
        if depth > 32 || out.len() > 5000 {
            return;
        }
        let Ok(dict) = doc.get_dictionary(id) else {
            return;
        };
        if let Ok(title) = dict.get(b"Title") {
            let title = match title {
                Object::String(bytes, _) => decode_pdf_text(bytes),
                other => other.as_str().map(decode_pdf_text).unwrap_or_default(),
            };
            // O destino aparece de duas formas: /Dest direto, ou /A para um
            // dicionário de ação (que quase sempre é uma referência).
            let dest_page = |obj: &Object| -> Option<lopdf::ObjectId> {
                let arr = match obj {
                    Object::Array(a) => a.clone(),
                    Object::Reference(id) => doc.get_object(*id).ok()?.as_array().ok()?.clone(),
                    _ => return None,
                };
                arr.first()?.as_reference().ok()
            };
            let action_page = |obj: &Object| -> Option<lopdf::ObjectId> {
                let d = match obj {
                    Object::Dictionary(d) => d.clone(),
                    Object::Reference(id) => doc.get_dictionary(*id).ok()?.clone(),
                    _ => return None,
                };
                let arr = d.get(b"D").ok()?.as_array().ok()?.clone();
                arr.first()?.as_reference().ok()
            };
            let page = dict
                .get(b"Dest")
                .ok()
                .and_then(dest_page)
                .or_else(|| dict.get(b"A").ok().and_then(action_page))
                .and_then(|pid| pages.get(&pid).copied())
                .unwrap_or(1);
            out.push(OutlineEntry {
                title: title.trim().to_string(),
                page,
                level,
            });
        }
        if let Ok(first) = dict.get(b"First").and_then(Object::as_reference) {
            walk(doc, first, level + 1, pages, out, depth + 1);
        }
        if let Ok(next) = dict.get(b"Next").and_then(Object::as_reference) {
            walk(doc, next, level, pages, out, depth + 1);
        }
    }
    // A raiz /Outlines não tem título: quem é nível 0 é o /First dela.
    if let Ok(first) = doc
        .get_dictionary(root)
        .and_then(|d| d.get(b"First"))
        .and_then(Object::as_reference)
    {
        walk(&doc, first, 0, &pages, &mut out, 0);
    }
    Ok(out)
}

pub fn write_outline(opts: &OutlineOptions) -> anyhow::Result<WriteItem> {
    let mut doc = load(&opts.input, "")?;
    let pages: BTreeMap<u32, lopdf::ObjectId> = doc.get_pages();
    if pages.is_empty() {
        return Err(anyhow!("documento sem páginas"));
    }
    doc.delete_outlines()
        .map_err(|e| anyhow!("não limpei o sumário antigo: {}", e))?;

    let entries = normalize_levels(&opts.entries);
    // Pilha com o id do último item de cada nível, para pendurar os filhos.
    let mut parents: Vec<u32> = Vec::new();
    for e in &entries {
        let page_id = *pages
            .get(&e.page)
            .or_else(|| pages.values().next())
            .ok_or_else(|| anyhow!("página {} não existe", e.page))?;
        let parent = if e.level == 0 {
            None
        } else {
            parents.get(e.level as usize - 1).copied()
        };
        let bookmark = lopdf::Bookmark::new(e.title.clone(), [0.0, 0.0, 0.0], 0, page_id);
        let id = doc.add_bookmark(bookmark, parent);
        parents.truncate(e.level as usize);
        parents.push(id);
    }
    // `build_outline` monta a árvore e devolve o id, mas quem aponta para
    // ela é o catálogo — sem isto o leitor não mostra sumário nenhum.
    if let Some(outline_id) = doc.build_outline() {
        let root = doc
            .trailer
            .get(b"Root")
            .and_then(Object::as_reference)
            .map_err(|e| anyhow!("documento sem catálogo: {}", e))?;
        if let Ok(catalog) = doc.get_dictionary_mut(root) {
            catalog.set("Outlines", Object::Reference(outline_id));
            catalog.set("PageMode", Object::Name(b"UseOutlines".to_vec()));
        }
    }

    let out = out_path(&opts.input, &opts.output_dir, &opts.suffix, "-sumario")?;
    doc.save(&out).map_err(|e| anyhow!("não gravei: {}", e))?;
    Ok(WriteItem {
        input: opts.input.clone(),
        output: Some(out.to_string_lossy().to_string()),
        pages: pages.len(),
        note: format!("{} entradas", entries.len()),
        ok: true,
        error: None,
    })
}

// ── Laço comum ─────────────────────────────────────────────────────────

fn run_each<F>(inputs: &[String], id: &str, progress: &super::ProgressFn, mut f: F) -> WriteResult
where
    F: FnMut(&str) -> anyhow::Result<WriteItem>,
{
    let total = inputs.len() as u64;
    let mut items = Vec::new();
    for (i, input) in inputs.iter().enumerate() {
        super::report(
            progress,
            id,
            "progress",
            i as u64,
            Some(total),
            Some(input.clone()),
        );
        items.push(f(input).unwrap_or_else(|e| {
            tracing::warn!("[{}] {}: {}", id, input, e);
            WriteItem {
                input: input.clone(),
                output: None,
                pages: 0,
                note: String::new(),
                ok: false,
                error: Some(e.to_string()),
            }
        }));
    }
    super::report(progress, id, "done", total, Some(total), None);
    WriteResult { items }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_pdf(path: &Path, pages: usize) {
        let mut doc = Document::with_version("1.5");
        let pages_id = doc.new_object_id();
        let font_id = doc.add_object(dictionary! {
            "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica",
        });
        let resources = doc.add_object(dictionary! { "Font" => dictionary! { "F1" => font_id } });
        let mut kids = Vec::new();
        for i in 0..pages {
            let content = Content {
                operations: vec![
                    Operation::new("BT", vec![]),
                    Operation::new("Tf", vec!["F1".into(), 24.into()]),
                    Operation::new("Td", vec![100.into(), 600.into()]),
                    Operation::new(
                        "Tj",
                        vec![Object::string_literal(format!("Pagina {}", i + 1))],
                    ),
                    Operation::new("ET", vec![]),
                ],
            };
            let stream = doc.add_object(Stream::new(dictionary! {}, content.encode().unwrap()));
            let page = doc.add_object(dictionary! {
                "Type" => "Page",
                "Parent" => pages_id,
                "Contents" => stream,
                "MediaBox" => vec![0.into(), 0.into(), 595.into(), 842.into()],
            });
            kids.push(page.into());
        }
        let count = kids.len() as i64;
        doc.objects.insert(
            pages_id,
            Object::Dictionary(dictionary! {
                "Type" => "Pages", "Kids" => kids, "Count" => count, "Resources" => resources,
            }),
        );
        let catalog = doc.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages_id });
        doc.trailer.set("Root", catalog);
        doc.save(path).unwrap();
    }

    fn tmp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join("omniget-pdfwrite-test");
        std::fs::create_dir_all(&dir).unwrap();
        dir.join(name)
    }

    #[test]
    fn permissions_always_keep_accessibility() {
        let base = PasswordOptions {
            inputs: vec![],
            mode: "add".into(),
            user_password: String::new(),
            owner_password: String::new(),
            current_password: String::new(),
            allow_print: false,
            allow_copy: false,
            allow_modify: false,
            allow_annotate: false,
            output_dir: String::new(),
            suffix: String::new(),
        };
        let locked = permissions_of(&base);
        assert!(locked.contains(Permissions::COPYABLE_FOR_ACCESSIBILITY));
        assert!(!locked.contains(Permissions::PRINTABLE));
        assert!(!locked.contains(Permissions::COPYABLE));

        let open = permissions_of(&PasswordOptions {
            allow_print: true,
            allow_copy: true,
            ..base
        });
        assert!(open.contains(Permissions::PRINTABLE));
        assert!(open.contains(Permissions::COPYABLE));
    }

    #[test]
    fn password_round_trips() {
        let src = tmp("claro.pdf");
        sample_pdf(&src, 2);
        let opts = PasswordOptions {
            inputs: vec![src.to_string_lossy().to_string()],
            mode: "add".into(),
            user_password: "abrir".into(),
            owner_password: "dono".into(),
            current_password: String::new(),
            allow_print: true,
            allow_copy: false,
            allow_modify: false,
            allow_annotate: false,
            output_dir: String::new(),
            suffix: "-cifrado".into(),
        };
        let res = password(&opts, &crate::core::tools::noop_progress());
        let item = &res.items[0];
        assert!(item.ok, "{:?}", item.error);
        let enc = item.output.clone().unwrap();

        // O arquivo tem que estar mesmo cifrado: o `lopdf` até abre a
        // estrutura sem senha, mas o conteúdo não pode sair legível.
        let bytes = std::fs::read(&enc).unwrap();
        assert!(
            !bytes.windows(6).any(|w| w == b"Pagina"),
            "o texto ficou em claro dentro do arquivo"
        );
        let locked = Document::load(&enc).unwrap();
        assert!(locked.is_encrypted(), "faltou o dicionário /Encrypt");
        assert!(
            Document::load_mem_with_options(&bytes, lopdf::LoadOptions::with_password("errada"))
                .is_err(),
            "abriu com a senha errada"
        );
        let doc =
            Document::load_mem_with_options(&bytes, lopdf::LoadOptions::with_password("abrir"))
                .unwrap();
        assert_eq!(doc.get_pages().len(), 2);
        let page = *doc.get_pages().values().next().unwrap();
        let content = String::from_utf8_lossy(&doc.get_page_content(page)).to_string();
        assert!(content.contains("Pagina"), "não decifrou o conteúdo");

        // E dá para tirar a senha de volta.
        let back = password(
            &PasswordOptions {
                inputs: vec![enc.clone()],
                mode: "remove".into(),
                current_password: "abrir".into(),
                suffix: "-aberto".into(),
                ..opts
            },
            &crate::core::tools::noop_progress(),
        );
        assert!(back.items[0].ok, "{:?}", back.items[0].error);
        let plain = Document::load(back.items[0].output.as_ref().unwrap()).unwrap();
        assert_eq!(plain.get_pages().len(), 2);
        assert!(!plain.is_encrypted());
    }

    #[test]
    fn adding_a_password_needs_a_password() {
        let src = tmp("sem-senha.pdf");
        sample_pdf(&src, 1);
        let res = password(
            &PasswordOptions {
                inputs: vec![src.to_string_lossy().to_string()],
                mode: "add".into(),
                user_password: String::new(),
                owner_password: String::new(),
                current_password: String::new(),
                allow_print: true,
                allow_copy: true,
                allow_modify: true,
                allow_annotate: true,
                output_dir: String::new(),
                suffix: String::new(),
            },
            &crate::core::tools::noop_progress(),
        );
        assert!(!res.items[0].ok);
        assert!(res.items[0].error.as_ref().unwrap().contains("senha"));
    }

    #[test]
    fn watermark_lands_on_every_page_and_keeps_the_original_content() {
        let src = tmp("carimbar.pdf");
        sample_pdf(&src, 3);
        let opts = WatermarkOptions {
            inputs: vec![src.to_string_lossy().to_string()],
            mode: "diagonal".into(),
            text: "CONFIDENCIAL".into(),
            font_size: 48.0,
            opacity: 15,
            gray: 50,
            start_number: 1,
            prefix: String::new(),
            output_dir: String::new(),
            suffix: String::new(),
        };
        let res = watermark(&opts, &crate::core::tools::noop_progress());
        assert!(res.items[0].ok, "{:?}", res.items[0].error);

        let doc = Document::load(res.items[0].output.as_ref().unwrap()).unwrap();
        assert_eq!(doc.get_pages().len(), 3);
        for (_, page_id) in doc.get_pages() {
            let streams = doc.get_page_contents(page_id);
            assert_eq!(streams.len(), 2, "conteúdo original + carimbo");
            let data = doc.get_page_content(page_id);
            let text = String::from_utf8_lossy(&data);
            assert!(text.contains("CONFIDENCIAL"), "o carimbo não entrou");
            assert!(text.contains("Pagina"), "o conteúdo original sumiu");
            assert!(text.contains("OGWMgs"), "faltou o ExtGState da opacidade");
        }
    }

    #[test]
    fn bates_numbers_each_page_in_sequence() {
        let src = tmp("bates.pdf");
        sample_pdf(&src, 3);
        let res = watermark(
            &WatermarkOptions {
                inputs: vec![src.to_string_lossy().to_string()],
                mode: "bates".into(),
                text: String::new(),
                font_size: 10.0,
                opacity: 100,
                gray: 0,
                start_number: 42,
                prefix: "OG-".into(),
                output_dir: String::new(),
                suffix: "-num".into(),
            },
            &crate::core::tools::noop_progress(),
        );
        assert!(res.items[0].ok, "{:?}", res.items[0].error);
        let doc = Document::load(res.items[0].output.as_ref().unwrap()).unwrap();
        let mut seen = Vec::new();
        for (_, page_id) in doc.get_pages() {
            let data = doc.get_page_content(page_id);
            let text = String::from_utf8_lossy(&data).to_string();
            for n in [42, 43, 44] {
                if text.contains(&format!("OG-{:06}", n)) {
                    seen.push(n);
                }
            }
        }
        seen.sort_unstable();
        assert_eq!(seen, vec![42, 43, 44]);
    }

    #[test]
    fn placement_puts_things_where_it_says() {
        let (w, h) = (595.0f32, 842.0f32);
        let footer = placement("footer", w, h, 100.0, 12.0);
        assert_eq!(footer[5], 18.0, "rodapé colado embaixo");
        assert!((footer[4] - 247.5).abs() < 0.01, "centralizado");
        let header = placement("header", w, h, 100.0, 12.0);
        assert!(header[5] > h - 40.0, "cabeçalho colado em cima");
        let bates = placement("bates", w, h, 60.0, 10.0);
        assert!(bates[4] > w / 2.0, "Bates fica à direita");
        let diag = placement("diagonal", w, h, 300.0, 48.0);
        assert!(diag[1] > 0.0 && diag[2] < 0.0, "a diagonal tem rotação");
    }

    #[test]
    fn text_width_grows_with_the_string() {
        let a = text_width("iii", 10.0);
        let b = text_width("MMM", 10.0);
        assert!(b > a * 2.0, "M é bem mais largo que i");
        assert!(text_width("", 48.0) == 0.0);
    }

    #[test]
    fn crop_fixed_shrinks_the_visible_box() {
        let src = tmp("cortar.pdf");
        sample_pdf(&src, 2);
        let res = crop(
            &CropOptions {
                inputs: vec![src.to_string_lossy().to_string()],
                mode: "fixed".into(),
                padding: 0.0,
                left: 40.0,
                right: 40.0,
                top: 60.0,
                bottom: 60.0,
                uniform: true,
                output_dir: String::new(),
                suffix: String::new(),
            },
            &crate::core::tools::noop_progress(),
        );
        assert!(res.items[0].ok, "{:?}", res.items[0].error);
        let doc = Document::load(res.items[0].output.as_ref().unwrap()).unwrap();
        for (_, page_id) in doc.get_pages() {
            let page = doc.get_dictionary(page_id).unwrap();
            let b = page.get(b"CropBox").unwrap().as_array().unwrap();
            let v: Vec<f32> = b.iter().map(|x| x.as_float().unwrap()).collect();
            assert_eq!(v, vec![40.0, 60.0, 555.0, 782.0]);
        }
    }

    #[test]
    fn clamp_never_leaves_the_page() {
        let media = [0.0, 0.0, 595.0, 842.0];
        let b = clamp_box([10.0, 10.0, 590.0, 838.0], media, 30.0);
        assert_eq!(b, [0.0, 0.0, 595.0, 842.0], "o respiro para na borda");
    }

    #[test]
    fn union_covers_every_page() {
        let u = union_box(&[[10.0, 20.0, 100.0, 200.0], [5.0, 30.0, 90.0, 300.0]]).unwrap();
        assert_eq!(u, [5.0, 20.0, 100.0, 300.0]);
        assert!(union_box(&[]).is_none());
    }

    #[test]
    fn outline_round_trips_with_nesting() {
        let src = tmp("sumario.pdf");
        sample_pdf(&src, 4);
        let entries = vec![
            OutlineEntry {
                title: "Capítulo 1".into(),
                page: 1,
                level: 0,
            },
            OutlineEntry {
                title: "Seção 1.1".into(),
                page: 2,
                level: 1,
            },
            OutlineEntry {
                title: "Seção 1.2".into(),
                page: 3,
                level: 1,
            },
            OutlineEntry {
                title: "Capítulo 2".into(),
                page: 4,
                level: 0,
            },
        ];
        let item = write_outline(&OutlineOptions {
            input: src.to_string_lossy().to_string(),
            entries: entries.clone(),
            output_dir: String::new(),
            suffix: String::new(),
        })
        .unwrap();
        let back = read_outline(item.output.as_ref().unwrap()).unwrap();
        let titles: Vec<&str> = back.iter().map(|e| e.title.as_str()).collect();
        for e in &entries {
            assert!(titles.contains(&e.title.as_str()), "sumiu: {}", e.title);
        }
        let sec = back.iter().find(|e| e.title == "Seção 1.1").unwrap();
        assert!(sec.level > 0, "a seção tinha que ser filha do capítulo");
        assert_eq!(sec.page, 2, "apontou para a página errada");
    }

    #[test]
    fn levels_are_normalized_so_no_entry_is_orphan() {
        let e = normalize_levels(&[
            OutlineEntry {
                title: "a".into(),
                page: 1,
                level: 3,
            },
            OutlineEntry {
                title: "b".into(),
                page: 2,
                level: 9,
            },
            OutlineEntry {
                title: "c".into(),
                page: 0,
                level: 0,
            },
        ]);
        assert_eq!(e[0].level, 0, "o primeiro sempre é raiz");
        assert_eq!(e[1].level, 1, "no máximo um degrau por vez");
        assert_eq!(e[2].page, 1, "página 0 não existe");
    }
}

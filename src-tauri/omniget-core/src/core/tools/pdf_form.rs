//! Preencher formulário de PDF (AcroForm) e achatar.
//!
//! Escolha de caminho: tudo pelo `lopdf`, sem FFI nova no PDFium. O PDFium
//! tem as `FPDF_Form*`, mas elas exigem um ambiente de formulário vivo
//! (`FPDFDOC_InitFormFillEnvironment`) e não dá para exercitar isso em teste
//! sem baixar a biblioteca. Mexer no dicionário direto é o que o Stirling-PDF
//! faz por baixo, roda em qualquer máquina e — o que mais importa aqui — dá
//! para testar de ponta a ponta com um PDF montado à mão, sem binário externo.
//!
//! Preencher = escrever `/V` (e `/AS` no caso de caixa e rádio) e ligar o
//! `/NeedAppearances`, que manda o leitor redesenhar. Achatar = desenhar o
//! valor no content stream da página com Helvetica e apagar os widgets e o
//! `/AcroForm`, para o texto virar parte do papel.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::anyhow;
use lopdf::content::{Content, Operation};
use lopdf::{dictionary, Document, Object, ObjectId, Stream};
use serde::{Deserialize, Serialize};

use super::pdf_write::{decode_pdf_text, text_width};

const ID: &str = "pdf-form-fill";

// Bits do /Ff (a numeração da especificação começa em 1).
const FF_READONLY: i64 = 1;
const FF_REQUIRED: i64 = 1 << 1;
const FF_MULTILINE: i64 = 1 << 12;
const FF_RADIO: i64 = 1 << 15;
const FF_PUSH: i64 = 1 << 16;

#[derive(Debug, Clone, Serialize)]
pub struct Field {
    /// Nome completo, com o caminho do pai ("endereco.cidade").
    pub name: String,
    /// "text" | "check" | "radio" | "choice" | "push" | "signature"
    pub kind: String,
    pub value: String,
    /// Estados aceitos (caixa/rádio) ou itens da lista.
    pub options: Vec<String>,
    /// Página 1-based onde o campo aparece; 0 quando não tem widget.
    pub page: usize,
    /// Retângulo do widget, em pontos.
    pub rect: [f32; 4],
    pub readonly: bool,
    pub required: bool,
    pub multiline: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct FieldValue {
    pub name: String,
    pub value: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Options {
    pub input: String,
    #[serde(default)]
    pub password: String,
    #[serde(default)]
    pub values: Vec<FieldValue>,
    /// Desenha os valores na página e apaga o formulário.
    #[serde(default)]
    pub flatten: bool,
    /// Corpo da fonte ao achatar. 0 = calcula pela altura do campo.
    #[serde(default)]
    pub font_size: f32,
    #[serde(default)]
    pub output_dir: String,
    #[serde(default)]
    pub suffix: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct FillResult {
    pub input: String,
    pub output: String,
    /// Campos que o documento tem.
    pub fields: usize,
    /// Campos que receberam valor.
    pub filled: usize,
    pub flattened: bool,
    /// Nomes que vieram na lista e não existem no PDF.
    pub missing: Vec<String>,
}

// ── Leitura ────────────────────────────────────────────────────────────

pub fn kind_of(ft: &str, ff: i64) -> &'static str {
    match ft {
        "Tx" => "text",
        "Ch" => "choice",
        "Btn" => {
            if ff & FF_PUSH != 0 {
                "push"
            } else if ff & FF_RADIO != 0 {
                "radio"
            } else {
                "check"
            }
        }
        "Sig" => "signature",
        _ => "text",
    }
}

fn dict_of(doc: &Document, obj: &Object) -> Option<lopdf::Dictionary> {
    match obj {
        Object::Reference(r) => doc.get_dictionary(*r).ok().cloned(),
        Object::Dictionary(d) => Some(d.clone()),
        _ => None,
    }
}

fn as_ref(obj: &Object) -> Option<ObjectId> {
    match obj {
        Object::Reference(r) => Some(*r),
        _ => None,
    }
}

fn text_of(obj: &Object) -> String {
    match obj {
        Object::String(b, _) => decode_pdf_text(b),
        Object::Name(n) => String::from_utf8_lossy(n).to_string(),
        Object::Integer(i) => i.to_string(),
        Object::Real(r) => r.to_string(),
        Object::Boolean(b) => b.to_string(),
        _ => String::new(),
    }
}

fn rect_of(dict: &lopdf::Dictionary) -> [f32; 4] {
    let mut out = [0f32; 4];
    if let Ok(arr) = dict.get(b"Rect").and_then(Object::as_array) {
        for (i, o) in arr.iter().take(4).enumerate() {
            out[i] = match o {
                Object::Integer(n) => *n as f32,
                Object::Real(r) => *r,
                _ => 0.0,
            };
        }
    }
    // Normaliza: alguns geradores escrevem o retângulo de cabeça para baixo.
    [
        out[0].min(out[2]),
        out[1].min(out[3]),
        out[0].max(out[2]),
        out[1].max(out[3]),
    ]
}

/// Estados que a aparência do widget aceita, tirando o "Off".
fn on_states(doc: &Document, widget: &lopdf::Dictionary) -> Vec<String> {
    let Some(ap) = widget.get(b"AP").ok().and_then(|o| dict_of(doc, o)) else {
        return Vec::new();
    };
    let Some(n) = ap.get(b"N").ok().and_then(|o| dict_of(doc, o)) else {
        return Vec::new();
    };
    n.iter()
        .map(|(k, _)| String::from_utf8_lossy(k).to_string())
        .filter(|s| s != "Off")
        .collect()
}

/// Em que página cada anotação está.
fn page_index(doc: &Document) -> HashMap<ObjectId, usize> {
    let mut map = HashMap::new();
    for (no, page_id) in doc.get_pages() {
        let annots = match doc.get_dictionary(page_id).and_then(|p| p.get(b"Annots")) {
            Ok(Object::Array(a)) => a.clone(),
            Ok(Object::Reference(r)) => match doc.get_object(*r) {
                Ok(Object::Array(a)) => a.clone(),
                _ => continue,
            },
            _ => continue,
        };
        for a in annots {
            if let Some(id) = as_ref(&a) {
                map.insert(id, no as usize);
            }
        }
    }
    map
}

fn acroform(doc: &Document) -> Option<lopdf::Dictionary> {
    let root = doc.catalog().ok()?;
    root.get(b"AcroForm").ok().and_then(|o| dict_of(doc, o))
}

/// Percorre a árvore de campos. Um nó com `/Kids` que tenham `/T` é um grupo;
/// sem `/T`, os filhos são só os widgets do mesmo campo.
#[allow(clippy::too_many_arguments)]
fn walk(
    doc: &Document,
    id: ObjectId,
    prefix: &str,
    ft: Option<String>,
    ff: i64,
    pages: &HashMap<ObjectId, usize>,
    out: &mut Vec<Field>,
    depth: usize,
) {
    if depth > 12 {
        return;
    }
    let Ok(dict) = doc.get_dictionary(id).cloned() else {
        return;
    };
    let name = dict.get(b"T").map(text_of).unwrap_or_default();
    let full = if name.is_empty() {
        prefix.to_string()
    } else if prefix.is_empty() {
        name
    } else {
        format!("{}.{}", prefix, name)
    };
    let ft = dict
        .get(b"FT")
        .ok()
        .map(text_of)
        .filter(|s| !s.is_empty())
        .or(ft);
    let ff = dict.get(b"Ff").and_then(Object::as_i64).ok().unwrap_or(ff);

    let kids: Vec<Object> = dict
        .get(b"Kids")
        .and_then(Object::as_array)
        .cloned()
        .unwrap_or_default();
    let child_fields: Vec<ObjectId> = kids
        .iter()
        .filter_map(as_ref)
        .filter(|k| doc.get_dictionary(*k).map(|d| d.has(b"T")).unwrap_or(false))
        .collect();
    if !child_fields.is_empty() {
        for k in child_fields {
            walk(doc, k, &full, ft.clone(), ff, pages, out, depth + 1);
        }
        return;
    }

    let kind = kind_of(ft.as_deref().unwrap_or(""), ff).to_string();
    let value = dict.get(b"V").map(text_of).unwrap_or_default();
    let mut options: Vec<String> = dict
        .get(b"Opt")
        .and_then(Object::as_array)
        .map(|a| {
            a.iter()
                .map(|o| match o {
                    Object::Array(inner) => inner.first().map(text_of).unwrap_or_default(),
                    other => text_of(other),
                })
                .collect()
        })
        .unwrap_or_default();

    // Widgets: o próprio campo (quando é widget mesclado) ou os filhos sem /T.
    let widgets: Vec<ObjectId> = if kids.is_empty() {
        vec![id]
    } else {
        kids.iter().filter_map(as_ref).collect()
    };
    let mut page = 0usize;
    let mut rect = [0f32; 4];
    for w in &widgets {
        let Ok(wd) = doc.get_dictionary(*w) else {
            continue;
        };
        if page == 0 {
            page = pages.get(w).copied().unwrap_or(0);
            rect = rect_of(wd);
        }
        if matches!(kind.as_str(), "check" | "radio") {
            for s in on_states(doc, wd) {
                if !options.contains(&s) {
                    options.push(s);
                }
            }
        }
    }

    out.push(Field {
        name: full,
        kind,
        value,
        options,
        page,
        rect,
        readonly: ff & FF_READONLY != 0,
        required: ff & FF_REQUIRED != 0,
        multiline: ff & FF_MULTILINE != 0,
    });
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

/// Lista os campos do formulário.
pub fn read_fields(input: &str, password: &str) -> anyhow::Result<Vec<Field>> {
    let doc = load(input, password)?;
    Ok(fields_of(&doc))
}

fn fields_of(doc: &Document) -> Vec<Field> {
    let Some(form) = acroform(doc) else {
        return Vec::new();
    };
    let pages = page_index(doc);
    let roots: Vec<ObjectId> = form
        .get(b"Fields")
        .and_then(Object::as_array)
        .map(|a| a.iter().filter_map(as_ref).collect())
        .unwrap_or_default();
    let mut out = Vec::new();
    for r in roots {
        walk(doc, r, "", None, 0, &pages, &mut out, 0);
    }
    out
}

// ── Escrita ────────────────────────────────────────────────────────────

/// Valor de caixa/rádio: o que o usuário digitou vira um estado que o widget
/// conhece. "sim", "1", "x", "true", "on" ligam a caixa.
pub fn button_state(value: &str, options: &[String]) -> String {
    let v = value.trim();
    if let Some(hit) = options.iter().find(|o| o.eq_ignore_ascii_case(v)) {
        return hit.clone();
    }
    let on = matches!(
        v.to_ascii_lowercase().as_str(),
        "1" | "x" | "on" | "yes" | "true" | "sim" | "v" | "check" | "checked"
    );
    if on {
        options.first().cloned().unwrap_or_else(|| "Yes".into())
    } else {
        "Off".into()
    }
}

/// Quebra o valor nas linhas que cabem na largura dada.
pub fn wrap(text: &str, width: f32, size: f32) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for para in text.split(['\n', '\r']) {
        if para.trim().is_empty() {
            out.push(String::new());
            continue;
        }
        let mut line = String::new();
        for word in para.split_whitespace() {
            let cand = if line.is_empty() {
                word.to_string()
            } else {
                format!("{} {}", line, word)
            };
            if !line.is_empty() && text_width(&cand, size) > width {
                out.push(std::mem::take(&mut line));
                line = word.to_string();
            } else {
                line = cand;
            }
        }
        out.push(line);
    }
    while out.last().is_some_and(|l| l.is_empty()) {
        out.pop();
    }
    out
}

/// Operações que desenham o valor dentro do retângulo do campo.
pub fn draw_ops(text: &str, rect: [f32; 4], size: f32, multiline: bool) -> Vec<Operation> {
    let (w, h) = (rect[2] - rect[0], rect[3] - rect[1]);
    if text.trim().is_empty() || w <= 1.0 || h <= 1.0 {
        return Vec::new();
    }
    let size = if size > 0.0 {
        size
    } else {
        (h - 4.0).clamp(6.0, 12.0)
    };
    let pad = 2.0f32;
    let lines = if multiline {
        wrap(text, (w - pad * 2.0).max(1.0), size)
    } else {
        vec![text.replace(['\n', '\r'], " ")]
    };
    let mut ops = vec![
        Operation::new("q", vec![]),
        // Recorta no campo: valor comprido não vaza para o resto da página.
        Operation::new(
            "re",
            vec![
                Object::Real(rect[0]),
                Object::Real(rect[1]),
                Object::Real(w),
                Object::Real(h),
            ],
        ),
        Operation::new("W", vec![]),
        Operation::new("n", vec![]),
        Operation::new("g", vec![Object::Real(0.0)]),
        Operation::new("BT", vec![]),
        Operation::new("Tf", vec!["OGFFfont".into(), Object::Real(size)]),
    ];
    // Uma linha só fica no meio da altura; várias começam no topo.
    let mut y = if multiline && lines.len() > 1 {
        rect[3] - size - pad
    } else {
        rect[1] + (h - size * 0.72) / 2.0
    };
    for line in lines {
        ops.push(Operation::new(
            "Td",
            vec![Object::Real(rect[0] + pad), Object::Real(y)],
        ));
        ops.push(Operation::new("Tj", vec![Object::string_literal(line)]));
        // O `Td` é relativo à linha anterior: volta o deslocamento aplicado.
        ops.push(Operation::new(
            "Td",
            vec![Object::Real(-(rect[0] + pad)), Object::Real(-y)],
        ));
        y -= size * 1.2;
        if y < rect[1] - size {
            break;
        }
    }
    ops.push(Operation::new("ET", vec![]));
    ops.push(Operation::new("Q", vec![]));
    ops
}

/// Onde acrescentar recurso sem esconder o que a página herda.
fn resource_dict_id(doc: &mut Document, page_id: ObjectId) -> Option<ObjectId> {
    let inline = doc
        .get_dictionary(page_id)
        .ok()
        .and_then(|p| p.get(b"Resources").ok())
        .map(|r| r.as_reference().is_ok());
    match inline {
        Some(true) => doc
            .get_dictionary(page_id)
            .ok()
            .and_then(|p| p.get(b"Resources").ok())
            .and_then(|r| r.as_reference().ok()),
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

/// Todos os widgets do campo, com a página de cada um.
fn widgets_of(doc: &Document, id: ObjectId, pages: &HashMap<ObjectId, usize>) -> Vec<ObjectId> {
    let Ok(dict) = doc.get_dictionary(id) else {
        return Vec::new();
    };
    let kids: Vec<ObjectId> = dict
        .get(b"Kids")
        .and_then(Object::as_array)
        .map(|a| a.iter().filter_map(as_ref).collect())
        .unwrap_or_default();
    if kids.is_empty() {
        vec![id]
    } else {
        kids.into_iter().filter(|k| pages.contains_key(k)).collect()
    }
}

/// Índice nome-completo → objeto do campo.
fn field_ids(doc: &Document) -> Vec<(String, ObjectId)> {
    let Some(form) = acroform(doc) else {
        return Vec::new();
    };
    let roots: Vec<ObjectId> = form
        .get(b"Fields")
        .and_then(Object::as_array)
        .map(|a| a.iter().filter_map(as_ref).collect())
        .unwrap_or_default();
    let mut out = Vec::new();
    let mut stack: Vec<(ObjectId, String)> =
        roots.into_iter().map(|r| (r, String::new())).collect();
    let mut guard = 0;
    while let Some((id, prefix)) = stack.pop() {
        guard += 1;
        if guard > 5000 {
            break;
        }
        let Ok(dict) = doc.get_dictionary(id) else {
            continue;
        };
        let name = dict.get(b"T").map(text_of).unwrap_or_default();
        let full = if name.is_empty() {
            prefix.clone()
        } else if prefix.is_empty() {
            name
        } else {
            format!("{}.{}", prefix, name)
        };
        let child_fields: Vec<ObjectId> = dict
            .get(b"Kids")
            .and_then(Object::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(as_ref)
                    .filter(|k| doc.get_dictionary(*k).map(|d| d.has(b"T")).unwrap_or(false))
                    .collect()
            })
            .unwrap_or_default();
        if child_fields.is_empty() {
            out.push((full, id));
        } else {
            for k in child_fields {
                stack.push((k, full.clone()));
            }
        }
    }
    out
}

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
        if opts.flatten {
            "-preenchido"
        } else {
            "-form"
        }
    } else {
        opts.suffix.trim()
    };
    Ok(dir.join(format!("{}{}.pdf", stem, suffix)))
}

pub fn fill(opts: &Options, progress: &super::ProgressFn) -> anyhow::Result<FillResult> {
    if opts.input.trim().is_empty() {
        return Err(anyhow!("escolha o PDF"));
    }
    let mut doc = load(&opts.input, &opts.password)?;
    let known = fields_of(&doc);
    if known.is_empty() {
        return Err(anyhow!("esse PDF não tem formulário (AcroForm)"));
    }
    let index = field_ids(&doc);
    let pages = page_index(&doc);
    let total = opts.values.len().max(1) as u64;

    let mut filled = 0usize;
    let mut missing: Vec<String> = Vec::new();
    // O que desenhar quando achatar: (página, retângulo, texto, multilinha).
    let mut stamps: Vec<(usize, [f32; 4], String, bool)> = Vec::new();

    for (i, v) in opts.values.iter().enumerate() {
        super::report(
            progress,
            ID,
            "progress",
            i as u64,
            Some(total),
            Some(v.name.clone()),
        );
        let wanted = v.name.trim();
        let hit = index
            .iter()
            .find(|(n, _)| n == wanted)
            .or_else(|| index.iter().find(|(n, _)| n.ends_with(wanted)));
        let Some(id) = hit.map(|(_, id)| *id) else {
            missing.push(v.name.clone());
            continue;
        };
        let meta = known
            .iter()
            .find(|f| f.name == wanted || f.name.ends_with(wanted));
        let kind = meta
            .map(|f| f.kind.clone())
            .unwrap_or_else(|| "text".into());
        let multiline = meta.map(|f| f.multiline).unwrap_or(false);
        let options = meta.map(|f| f.options.clone()).unwrap_or_default();
        let widgets = widgets_of(&doc, id, &pages);

        match kind.as_str() {
            "check" | "radio" => {
                let state = button_state(&v.value, &options);
                if let Ok(field) = doc.get_dictionary_mut(id) {
                    field.set("V", Object::Name(state.clone().into_bytes()));
                }
                for w in &widgets {
                    let states = doc
                        .get_dictionary(*w)
                        .map(|d| on_states(&doc, d))
                        .unwrap_or_default();
                    let as_state = if states.contains(&state) {
                        state.clone()
                    } else {
                        "Off".to_string()
                    };
                    if let Ok(wd) = doc.get_dictionary_mut(*w) {
                        wd.set("AS", Object::Name(as_state.into_bytes()));
                    }
                }
                if state != "Off" {
                    if let Some(f) = meta {
                        stamps.push((f.page, f.rect, "X".into(), false));
                    }
                }
            }
            "signature" | "push" => {
                missing.push(v.name.clone());
                continue;
            }
            _ => {
                if let Ok(field) = doc.get_dictionary_mut(id) {
                    field.set("V", Object::string_literal(v.value.as_str()));
                    // A aparência velha mostraria o valor antigo.
                    field.remove(b"AP");
                }
                for w in &widgets {
                    if let Ok(wd) = doc.get_dictionary_mut(*w) {
                        wd.remove(b"AP");
                    }
                }
                if let Some(f) = meta {
                    stamps.push((f.page, f.rect, v.value.clone(), multiline));
                }
            }
        }
        filled += 1;
    }

    // O leitor tem que redesenhar a aparência com o valor novo.
    if let Ok(root) = doc.catalog_mut() {
        if let Ok(Object::Reference(r)) = root.get(b"AcroForm").cloned() {
            if let Ok(Object::Dictionary(form)) = doc.get_object_mut(r) {
                form.set("NeedAppearances", Object::Boolean(true));
            }
        } else if let Ok(Object::Dictionary(form)) = doc.catalog_mut()?.get_mut(b"AcroForm") {
            form.set("NeedAppearances", Object::Boolean(true));
        }
    }

    if opts.flatten {
        flatten(&mut doc, &stamps, opts.font_size)?;
    }

    let output = out_path(opts)?;
    doc.save(&output)
        .map_err(|e| anyhow!("não gravei {}: {}", output.display(), e))?;
    super::report(progress, ID, "done", total, Some(total), None);
    Ok(FillResult {
        input: opts.input.clone(),
        output: output.to_string_lossy().to_string(),
        fields: known.len(),
        filled,
        flattened: opts.flatten,
        missing,
    })
}

/// Desenha os valores nas páginas e apaga o formulário inteiro.
fn flatten(
    doc: &mut Document,
    stamps: &[(usize, [f32; 4], String, bool)],
    font_size: f32,
) -> anyhow::Result<()> {
    let ids: Vec<(u32, ObjectId)> = doc.get_pages().into_iter().collect();
    let font_id = doc.add_object(dictionary! {
        "Type" => "Font",
        "Subtype" => "Type1",
        "BaseFont" => "Helvetica",
        "Encoding" => "WinAnsiEncoding",
    });
    for (no, page_id) in &ids {
        let ops: Vec<Operation> = stamps
            .iter()
            .filter(|(p, ..)| *p == *no as usize)
            .flat_map(|(_, rect, text, multi)| draw_ops(text, *rect, font_size, *multi))
            .collect();
        if !ops.is_empty() {
            let data = Content { operations: ops }
                .encode()
                .map_err(|e| anyhow!("conteúdo: {}", e))?;
            let stream_id = doc.add_object(Stream::new(dictionary! {}, data));
            if let Some(res_id) = resource_dict_id(doc, *page_id) {
                if let Ok(res) = doc.get_dictionary_mut(res_id) {
                    let mut fonts = res
                        .get(b"Font")
                        .and_then(Object::as_dict)
                        .cloned()
                        .unwrap_or_default();
                    fonts.set("OGFFfont", Object::Reference(font_id));
                    res.set("Font", Object::Dictionary(fonts));
                }
            }
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
        // Fora os widgets: o campo virou tinta.
        let annots = match doc.get_dictionary(*page_id).and_then(|p| p.get(b"Annots")) {
            Ok(Object::Array(a)) => a.clone(),
            Ok(Object::Reference(r)) => match doc.get_object(*r) {
                Ok(Object::Array(a)) => a.clone(),
                _ => Vec::new(),
            },
            _ => Vec::new(),
        };
        let kept: Vec<Object> = annots
            .into_iter()
            .filter(|o| {
                let sub = dict_of(doc, o)
                    .and_then(|d| d.get(b"Subtype").ok().map(text_of))
                    .unwrap_or_default();
                sub != "Widget"
            })
            .collect();
        if let Ok(page) = doc.get_dictionary_mut(*page_id) {
            if kept.is_empty() {
                page.remove(b"Annots");
            } else {
                page.set("Annots", Object::Array(kept));
            }
        }
    }
    if let Ok(root) = doc.catalog_mut() {
        root.remove(b"AcroForm");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Formulário de teste: um campo de texto, uma caixa de marcar e um campo
    /// filho dentro de um grupo. Montado com o próprio lopdf, sem arquivo
    /// binário no repositório.
    fn build_form(path: &Path) {
        let mut doc = Document::with_version("1.5");
        let pages_id = doc.new_object_id();
        let font_id = doc.add_object(dictionary! {
            "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica",
        });
        let content = Content {
            operations: vec![
                Operation::new("BT", vec![]),
                Operation::new("Tf", vec!["F1".into(), 12.into()]),
                Operation::new("Td", vec![50.into(), 700.into()]),
                Operation::new("Tj", vec![Object::string_literal("Ficha")]),
                Operation::new("ET", vec![]),
            ],
        };
        let content_id = doc.add_object(Stream::new(dictionary! {}, content.encode().unwrap()));
        let page_id = doc.new_object_id();

        let nome = doc.add_object(dictionary! {
            "Type" => "Annot", "Subtype" => "Widget", "FT" => "Tx",
            "T" => Object::string_literal("nome"),
            "Rect" => vec![50.into(), 640.into(), 300.into(), 660.into()],
            "P" => page_id,
        });
        let aceito = doc.add_object(dictionary! {
            "Type" => "Annot", "Subtype" => "Widget", "FT" => "Btn",
            "T" => Object::string_literal("aceito"),
            "Rect" => vec![50.into(), 600.into(), 66.into(), 616.into()],
            "AP" => dictionary! { "N" => dictionary! { "Yes" => dictionary! {}, "Off" => dictionary! {} } },
            "AS" => Object::Name(b"Off".to_vec()),
            "P" => page_id,
        });
        let cidade = doc.add_object(dictionary! {
            "Type" => "Annot", "Subtype" => "Widget", "FT" => "Tx",
            "T" => Object::string_literal("cidade"),
            "Rect" => vec![50.into(), 560.into(), 300.into(), 580.into()],
            "P" => page_id,
        });
        let endereco = doc.add_object(dictionary! {
            "T" => Object::string_literal("endereco"),
            "Kids" => vec![cidade.into()],
        });
        if let Ok(d) = doc.get_dictionary_mut(cidade) {
            d.set("Parent", endereco);
        }

        doc.objects.insert(
            page_id,
            Object::Dictionary(dictionary! {
                "Type" => "Page",
                "Parent" => pages_id,
                "Contents" => content_id,
                "Annots" => vec![nome.into(), aceito.into(), cidade.into()],
                "Resources" => dictionary! { "Font" => dictionary! { "F1" => font_id } },
                "MediaBox" => vec![0.into(), 0.into(), 595.into(), 842.into()],
            }),
        );
        doc.objects.insert(
            pages_id,
            Object::Dictionary(dictionary! {
                "Type" => "Pages", "Kids" => vec![page_id.into()], "Count" => 1,
            }),
        );
        let catalog_id = doc.add_object(dictionary! {
            "Type" => "Catalog",
            "Pages" => pages_id,
            "AcroForm" => dictionary! {
                "Fields" => vec![nome.into(), aceito.into(), endereco.into()],
            },
        });
        doc.trailer.set("Root", catalog_id);
        doc.save(path).unwrap();
    }

    fn tmp() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("omniget-form-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn tipo_de_campo_sai_do_ft_e_do_ff() {
        assert_eq!(kind_of("Tx", 0), "text");
        assert_eq!(kind_of("Btn", 0), "check");
        assert_eq!(kind_of("Btn", FF_RADIO), "radio");
        assert_eq!(kind_of("Btn", FF_PUSH), "push");
        assert_eq!(kind_of("Ch", 1 << 17), "choice");
        assert_eq!(kind_of("Sig", 0), "signature");
    }

    #[test]
    fn estado_do_botao() {
        let opts = vec!["Yes".to_string()];
        assert_eq!(button_state("sim", &opts), "Yes");
        assert_eq!(button_state("Yes", &opts), "Yes");
        assert_eq!(button_state("nao", &opts), "Off");
        assert_eq!(button_state("1", &[]), "Yes");
        // Estado com nome próprio casa sem olhar a caixa.
        assert_eq!(button_state("opcao2", &["Opcao2".into()]), "Opcao2");
    }

    #[test]
    fn quebra_de_linha_respeita_a_largura() {
        let lines = wrap(
            "um texto bem comprido que nao cabe numa linha so",
            80.0,
            10.0,
        );
        assert!(lines.len() > 2, "{:?}", lines);
        assert!(lines.iter().all(|l| text_width(l, 10.0) <= 80.0 + 1.0));
        assert_eq!(wrap("a\nb", 200.0, 10.0), vec!["a", "b"]);
    }

    #[test]
    fn desenho_cabe_no_retangulo() {
        let ops = draw_ops("Maria", [50.0, 600.0, 200.0, 620.0], 0.0, false);
        let dump = format!("{:?}", ops);
        assert!(dump.contains("Maria"), "{}", dump);
        assert!(dump.contains("\"W\""), "sem recorte: {}", dump);
        assert!(dump.contains("OGFFfont"), "{}", dump);
        // Campo vazio não desenha nada.
        assert!(draw_ops("  ", [50.0, 600.0, 200.0, 620.0], 0.0, false).is_empty());
    }

    #[test]
    fn le_os_campos_do_formulario() {
        let dir = tmp();
        let path = dir.join("ficha.pdf");
        build_form(&path);
        let fields = read_fields(&path.to_string_lossy(), "").unwrap();
        let names: Vec<&str> = fields.iter().map(|f| f.name.as_str()).collect();
        assert_eq!(
            names,
            vec!["nome", "aceito", "endereco.cidade"],
            "{:?}",
            names
        );
        assert_eq!(fields[0].kind, "text");
        assert_eq!(fields[0].page, 1);
        assert_eq!(fields[1].kind, "check");
        assert_eq!(fields[1].options, vec!["Yes"]);
        assert_eq!(fields[2].rect, [50.0, 560.0, 300.0, 580.0]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn preenche_sem_achatar_e_o_valor_fica_no_campo() {
        let dir = tmp();
        let path = dir.join("ficha.pdf");
        build_form(&path);
        let opts = Options {
            input: path.to_string_lossy().to_string(),
            password: String::new(),
            values: vec![
                FieldValue {
                    name: "nome".into(),
                    value: "Maria Souza".into(),
                },
                FieldValue {
                    name: "aceito".into(),
                    value: "sim".into(),
                },
                FieldValue {
                    name: "endereco.cidade".into(),
                    value: "Recife".into(),
                },
                FieldValue {
                    name: "inexistente".into(),
                    value: "x".into(),
                },
            ],
            flatten: false,
            font_size: 0.0,
            output_dir: String::new(),
            suffix: String::new(),
        };
        let out = fill(&opts, &super::super::noop_progress()).unwrap();
        assert_eq!(out.filled, 3);
        assert_eq!(out.missing, vec!["inexistente"]);
        assert!(!out.flattened);

        let fields = read_fields(&out.output, "").unwrap();
        let by = |n: &str| fields.iter().find(|f| f.name == n).cloned().unwrap();
        assert_eq!(by("nome").value, "Maria Souza");
        assert_eq!(by("aceito").value, "Yes");
        assert_eq!(by("endereco.cidade").value, "Recife");
        // O formulário continua lá para o usuário editar.
        let doc = Document::load(&out.output).unwrap();
        assert!(acroform(&doc).is_some());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn achatar_apaga_o_formulario_e_deixa_o_texto_na_pagina() {
        let dir = tmp();
        let path = dir.join("ficha.pdf");
        build_form(&path);
        let opts = Options {
            input: path.to_string_lossy().to_string(),
            password: String::new(),
            values: vec![
                FieldValue {
                    name: "nome".into(),
                    value: "Maria Souza".into(),
                },
                FieldValue {
                    name: "aceito".into(),
                    value: "sim".into(),
                },
            ],
            flatten: true,
            font_size: 10.0,
            output_dir: String::new(),
            suffix: String::new(),
        };
        let out = fill(&opts, &super::super::noop_progress()).unwrap();
        assert!(out.flattened);

        let doc = Document::load(&out.output).unwrap();
        assert!(acroform(&doc).is_none(), "sobrou AcroForm");
        let page_id = doc.get_pages().into_values().next().unwrap();
        assert!(
            doc.get_dictionary(page_id).unwrap().get(b"Annots").is_err(),
            "sobrou widget na página"
        );
        let content = String::from_utf8_lossy(&doc.get_page_content(page_id)).to_string();
        assert!(content.contains("Maria Souza"), "{}", content);
        assert!(content.contains("(X)"), "a caixa marcada não foi desenhada");
        // E não há mais campo nenhum para ler.
        assert!(read_fields(&out.output, "").unwrap().is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn pdf_sem_formulario_da_erro_claro() {
        let dir = tmp();
        let path = dir.join("vazio.pdf");
        let mut doc = Document::with_version("1.5");
        let pages_id = doc.new_object_id();
        let page_id = doc.add_object(dictionary! {
            "Type" => "Page", "Parent" => pages_id,
            "MediaBox" => vec![0.into(), 0.into(), 200.into(), 200.into()],
        });
        doc.objects.insert(
            pages_id,
            Object::Dictionary(
                dictionary! { "Type" => "Pages", "Kids" => vec![page_id.into()], "Count" => 1 },
            ),
        );
        let cat = doc.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages_id });
        doc.trailer.set("Root", cat);
        doc.save(&path).unwrap();
        let opts = Options {
            input: path.to_string_lossy().to_string(),
            password: String::new(),
            values: vec![FieldValue {
                name: "x".into(),
                value: "y".into(),
            }],
            flatten: false,
            font_size: 0.0,
            output_dir: String::new(),
            suffix: String::new(),
        };
        let err = fill(&opts, &super::super::noop_progress())
            .unwrap_err()
            .to_string();
        assert!(err.contains("AcroForm"), "{}", err);
        let _ = std::fs::remove_dir_all(&dir);
    }
}

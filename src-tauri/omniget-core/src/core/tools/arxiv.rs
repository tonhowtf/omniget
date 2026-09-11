//! arXiv → Markdown com a matemática preservada (tool `ai-arxiv-md`).
//! Referência de desenho: timf34/arxiv2md (MIT).
//!
//! Metadados saem da API pública e documentada do arXiv
//! (`http://export.arxiv.org/api/query?id_list=`, Atom XML) — que pede
//! gentileza na frequência, então é uma requisição por chamada.
//! O corpo vem, em ordem: do **source LaTeX** (`arxiv.org/e-print/<id>`, um
//! tar.gz), do HTML do LaTeXML (`arxiv.org/html/<id>`) e, por último, só do
//! resumo. Matemática nunca é renderizada: `$…$` e `$$…$$` saem intactos.

use std::collections::HashMap;

use anyhow::anyhow;
use serde::{Deserialize, Serialize};

use super::{report, ProgressFn};

const ID: &str = "ai-arxiv-md";
const API: &str = "http://export.arxiv.org/api/query?id_list=";

// ── Identificador ──────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ArxivRef {
    /// Sem versão: `2401.12345` ou `hep-th/9901001`.
    pub id: String,
    pub version: Option<u32>,
}

impl ArxivRef {
    /// Com versão quando o usuário pediu uma.
    pub fn full(&self) -> String {
        match self.version {
            Some(v) => format!("{}v{}", self.id, v),
            None => self.id.clone(),
        }
    }
    pub fn abs_url(&self) -> String {
        format!("https://arxiv.org/abs/{}", self.full())
    }
    pub fn pdf_url(&self) -> String {
        format!("https://arxiv.org/pdf/{}", self.full())
    }
    pub fn source_url(&self) -> String {
        format!("https://arxiv.org/e-print/{}", self.full())
    }
    pub fn html_url(&self) -> String {
        format!("https://arxiv.org/html/{}", self.full())
    }
    /// Nome de arquivo seguro.
    pub fn slug(&self) -> String {
        self.full().replace('/', "_")
    }
}

/// Aceita `2401.12345`, `2401.12345v2`, `arXiv:2401.12345`, URLs de
/// `abs/`, `pdf/`, `html/`, `e-print/` e o formato antigo `hep-th/9901001`.
pub fn parse_id(input: &str) -> Option<ArxivRef> {
    static NEW: std::sync::OnceLock<Option<regex::Regex>> = std::sync::OnceLock::new();
    static OLD: std::sync::OnceLock<Option<regex::Regex>> = std::sync::OnceLock::new();
    let new = NEW
        .get_or_init(|| regex::Regex::new(r"(?i)\b(\d{4}\.\d{4,5})(?:v(\d+))?\b").ok())
        .as_ref()?;
    let old = OLD
        .get_or_init(|| {
            regex::Regex::new(r"(?i)\b([a-z][a-z-]{1,15}(?:\.[a-z]{2})?/\d{7})(?:v(\d+))?\b").ok()
        })
        .as_ref()?;

    let raw = input.trim();
    if raw.is_empty() {
        return None;
    }
    let raw = raw.trim_end_matches(".pdf").trim_end_matches('/');
    let take = |c: &regex::Captures| ArxivRef {
        id: c.get(1).map(|m| m.as_str().to_string()).unwrap_or_default(),
        version: c.get(2).and_then(|m| m.as_str().parse().ok()),
    };
    if let Some(c) = new.captures(raw) {
        return Some(take(&c));
    }
    let r = take(&old.captures(raw)?);
    if r.id.is_empty() {
        return None;
    }
    Some(r)
}

// ── XML mínimo (o Atom do arXiv é previsível) ──────────────────────────

fn unescape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        let tail = &rest[i..];
        let Some(end) = tail.find(';').filter(|e| *e <= 10) else {
            out.push('&');
            rest = &tail[1..];
            continue;
        };
        let ent = &tail[1..end];
        let ch = match ent {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            "nbsp" => Some(' '),
            e if e.starts_with("#x") || e.starts_with("#X") => u32::from_str_radix(&e[2..], 16)
                .ok()
                .and_then(char::from_u32),
            e if e.starts_with('#') => e[1..].parse::<u32>().ok().and_then(char::from_u32),
            _ => None,
        };
        match ch {
            Some(c) => {
                out.push(c);
                rest = &tail[end + 1..];
            }
            None => {
                out.push('&');
                rest = &tail[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

fn squeeze(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Conteúdo do primeiro `<tag>` a partir de `from`, com o índice do fim.
fn tag_at(xml: &str, tag: &str, from: usize) -> Option<(String, usize)> {
    let open = format!("<{}", tag);
    let close = format!("</{}>", tag);
    let start = xml[from..].find(&open)? + from;
    let after_name = start + open.len();
    let c = xml.as_bytes().get(after_name)?;
    if c.is_ascii_alphanumeric() || *c == b'-' || *c == b':' {
        // era outra tag com o mesmo prefixo (`<title>` vs `<titlepage>`)
        return tag_at(xml, tag, after_name);
    }
    let gt = xml[start..].find('>')? + start;
    if xml[start..gt].ends_with('/') {
        return Some((String::new(), gt + 1));
    }
    let end = xml[gt..].find(&close)? + gt;
    Some((xml[gt + 1..end].to_string(), end + close.len()))
}

fn tag(xml: &str, name: &str) -> Option<String> {
    tag_at(xml, name, 0).map(|(v, _)| v)
}

fn tags(xml: &str, name: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut i = 0;
    while let Some((v, next)) = tag_at(xml, name, i) {
        out.push(v);
        i = next;
    }
    out
}

/// Valor de um atributo em todas as tags `<name ...>` do XML.
fn attrs(xml: &str, name: &str, attr: &str) -> Vec<String> {
    let open = format!("<{} ", name);
    let needle = format!("{}=\"", attr);
    let mut out = Vec::new();
    let mut i = 0;
    while let Some(rel) = xml[i..].find(&open) {
        let start = i + rel;
        let Some(gt) = xml[start..].find('>').map(|g| start + g) else {
            break;
        };
        let head = &xml[start..gt];
        if let Some(a) = head.find(&needle) {
            let from = a + needle.len();
            if let Some(q) = head[from..].find('"') {
                out.push(unescape(&head[from..from + q]));
            }
        }
        i = gt + 1;
    }
    out
}

// ── Metadados ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, Default, Serialize)]
pub struct Meta {
    pub id: String,
    pub version: Option<u32>,
    pub title: String,
    pub authors: Vec<String>,
    pub summary: String,
    pub categories: Vec<String>,
    pub primary_category: String,
    pub published: String,
    pub updated: String,
    pub doi: Option<String>,
    pub journal_ref: Option<String>,
    pub comment: Option<String>,
    pub abs_url: String,
    pub pdf_url: String,
}

/// Lê o Atom da API do arXiv. Sem crate de XML: o formato é fixo.
pub fn parse_atom(xml: &str) -> anyhow::Result<Meta> {
    let entry = tag(xml, "entry").ok_or_else(|| anyhow!("resposta do arXiv sem <entry>"))?;
    let raw_id = tag(&entry, "id").unwrap_or_default();
    let title = squeeze(&unescape(&tag(&entry, "title").unwrap_or_default()));
    if title.eq_ignore_ascii_case("error") || raw_id.contains("api/errors") {
        let msg = squeeze(&unescape(&tag(&entry, "summary").unwrap_or_default()));
        return Err(anyhow!(
            "arXiv nao reconheceu o identificador: {}",
            if msg.is_empty() { "sem detalhe" } else { &msg }
        ));
    }
    let short = raw_id
        .rsplit("/abs/")
        .next()
        .unwrap_or(&raw_id)
        .trim()
        .to_string();
    let r = parse_id(&short).unwrap_or(ArxivRef {
        id: short.clone(),
        version: None,
    });

    let authors = tags(&entry, "author")
        .iter()
        .filter_map(|a| tag(a, "name"))
        .map(|n| squeeze(&unescape(&n)))
        .filter(|n| !n.is_empty())
        .collect();

    let mut categories: Vec<String> = attrs(&entry, "category", "term");
    categories.dedup();
    let primary = attrs(&entry, "arxiv:primary_category", "term")
        .first()
        .cloned()
        .or_else(|| categories.first().cloned())
        .unwrap_or_default();

    let opt = |v: Option<String>| {
        let v = squeeze(&unescape(&v.unwrap_or_default()));
        if v.is_empty() {
            None
        } else {
            Some(v)
        }
    };

    Ok(Meta {
        abs_url: r.abs_url(),
        pdf_url: r.pdf_url(),
        id: r.id.clone(),
        version: r.version,
        title,
        authors,
        summary: squeeze(&unescape(&tag(&entry, "summary").unwrap_or_default())),
        categories,
        primary_category: primary,
        published: tag(&entry, "published")
            .unwrap_or_default()
            .trim()
            .to_string(),
        updated: tag(&entry, "updated")
            .unwrap_or_default()
            .trim()
            .to_string(),
        doi: opt(tag(&entry, "arxiv:doi")),
        journal_ref: opt(tag(&entry, "arxiv:journal_ref")),
        comment: opt(tag(&entry, "arxiv:comment")),
    })
}

fn yaml_str(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}

/// Front-matter YAML com título, autores, categorias, data, DOI e resumo.
pub fn front_matter(meta: &Meta) -> String {
    let mut out = String::from("---\n");
    out.push_str(&format!("title: {}\n", yaml_str(&meta.title)));
    if !meta.authors.is_empty() {
        out.push_str("authors:\n");
        for a in &meta.authors {
            out.push_str(&format!("  - {}\n", yaml_str(a)));
        }
    }
    let full = match meta.version {
        Some(v) => format!("{}v{}", meta.id, v),
        None => meta.id.clone(),
    };
    out.push_str(&format!("arxiv: {}\n", yaml_str(&full)));
    if !meta.categories.is_empty() {
        out.push_str(&format!("categories: [{}]\n", meta.categories.join(", ")));
    }
    if !meta.primary_category.is_empty() {
        out.push_str(&format!(
            "primary_category: {}\n",
            yaml_str(&meta.primary_category)
        ));
    }
    if !meta.published.is_empty() {
        out.push_str(&format!("published: {}\n", yaml_str(&meta.published)));
    }
    if !meta.updated.is_empty() {
        out.push_str(&format!("updated: {}\n", yaml_str(&meta.updated)));
    }
    if let Some(d) = &meta.doi {
        out.push_str(&format!("doi: {}\n", yaml_str(d)));
    }
    if let Some(j) = &meta.journal_ref {
        out.push_str(&format!("journal_ref: {}\n", yaml_str(j)));
    }
    if let Some(c) = &meta.comment {
        out.push_str(&format!("comment: {}\n", yaml_str(c)));
    }
    out.push_str(&format!("url: {}\n", yaml_str(&meta.abs_url)));
    out.push_str(&format!("pdf: {}\n", yaml_str(&meta.pdf_url)));
    if !meta.summary.is_empty() {
        out.push_str("abstract: |\n");
        for line in meta.summary.split('\n') {
            out.push_str(&format!("  {}\n", line));
        }
    }
    out.push_str("---\n");
    out
}

// ── Source LaTeX (tar.gz) ──────────────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
pub struct SourceBundle {
    /// O `.tex` principal, com os `\input`/`\include` já resolvidos.
    pub main: String,
    pub files: Vec<String>,
}

fn is_gzip(b: &[u8]) -> bool {
    b.len() > 2 && b[0] == 0x1f && b[1] == 0x8b
}

fn is_tar(b: &[u8]) -> bool {
    b.len() > 262 && &b[257..262] == b"ustar"
}

fn gunzip(b: &[u8]) -> anyhow::Result<Vec<u8>> {
    use std::io::Read;
    let mut out = Vec::new();
    flate2::read::GzDecoder::new(b).read_to_end(&mut out)?;
    Ok(out)
}

/// Aceita o tar.gz do `e-print`, um tar cru, um `.tex` gzipado ou o `.tex` solto.
pub fn extract_source(bytes: &[u8]) -> anyhow::Result<SourceBundle> {
    if bytes.starts_with(b"%PDF") {
        return Err(anyhow!("o source deste arXiv e um PDF, nao tem LaTeX"));
    }
    let raw = if is_gzip(bytes) {
        gunzip(bytes)?
    } else {
        bytes.to_vec()
    };
    if !is_tar(&raw) {
        let text = String::from_utf8_lossy(&raw).to_string();
        if !text.contains('\\') {
            return Err(anyhow!("source do arXiv nao parece LaTeX"));
        }
        return Ok(SourceBundle {
            main: text,
            files: vec!["main.tex".to_string()],
        });
    }

    let mut texts: HashMap<String, String> = HashMap::new();
    let mut names: Vec<String> = Vec::new();
    let mut archive = tar::Archive::new(std::io::Cursor::new(&raw));
    for entry in archive.entries()? {
        let mut entry = entry?;
        let path = entry.path()?.to_string_lossy().to_string();
        let lower = path.to_lowercase();
        if !(lower.ends_with(".tex") || lower.ends_with(".bbl") || lower.ends_with(".ltx")) {
            continue;
        }
        let mut buf = Vec::new();
        use std::io::Read;
        entry.read_to_end(&mut buf)?;
        names.push(path.clone());
        texts.insert(path, String::from_utf8_lossy(&buf).to_string());
    }
    if texts.is_empty() {
        return Err(anyhow!("o tar do arXiv nao tem nenhum .tex"));
    }

    let main_key =
        pick_main(&texts).ok_or_else(|| anyhow!("nenhum .tex com \\begin{{document}}"))?;
    let main = texts.get(&main_key).cloned().unwrap_or_default();
    let main = resolve_inputs(&main, &texts, 0);
    names.sort();
    Ok(SourceBundle { main, files: names })
}

fn pick_main(texts: &HashMap<String, String>) -> Option<String> {
    let mut best: Option<(u32, usize, String)> = None;
    for (k, v) in texts {
        if !k.to_lowercase().ends_with(".tex") && !k.to_lowercase().ends_with(".ltx") {
            continue;
        }
        let mut score = 0u32;
        if v.contains("\\begin{document}") {
            score += 4;
        }
        if v.contains("\\documentclass") {
            score += 2;
        }
        let base = k.rsplit('/').next().unwrap_or(k).to_lowercase();
        if base.starts_with("main") || base.starts_with("ms.") || base.starts_with("paper") {
            score += 1;
        }
        if score == 0 {
            continue;
        }
        let better = match &best {
            None => true,
            Some((s, len, _)) => score > *s || (score == *s && v.len() > *len),
        };
        if better {
            best = Some((score, v.len(), k.clone()));
        }
    }
    best.map(|(_, _, k)| k)
}

/// Cola o conteúdo de `\input{}` / `\include{}` no lugar, até 4 níveis.
fn resolve_inputs(tex: &str, texts: &HashMap<String, String>, depth: usize) -> String {
    if depth > 4 {
        return tex.to_string();
    }
    let lookup = |name: &str| -> Option<String> {
        let want = name.trim().trim_matches('"');
        for cand in [want.to_string(), format!("{}.tex", want)] {
            if let Some(v) = texts.get(&cand) {
                return Some(v.clone());
            }
            let tail = cand.rsplit('/').next().unwrap_or(&cand).to_lowercase();
            for (k, v) in texts {
                if k.rsplit('/').next().unwrap_or(k).to_lowercase() == tail {
                    return Some(v.clone());
                }
            }
        }
        None
    };
    let mut out = map_cmd(tex, "input", &|arg| lookup(arg).unwrap_or_default());
    out = map_cmd(&out, "include", &|arg| lookup(arg).unwrap_or_default());
    if out == tex {
        out
    } else {
        resolve_inputs(&out, texts, depth + 1)
    }
}

// ── LaTeX → Markdown ───────────────────────────────────────────────────

const MATH_ENVS: &[&str] = &[
    "equation",
    "equation*",
    "align",
    "align*",
    "alignat",
    "alignat*",
    "flalign",
    "flalign*",
    "gather",
    "gather*",
    "multline",
    "multline*",
    "eqnarray",
    "eqnarray*",
    "displaymath",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Span {
    Text(String),
    Math(String),
}

fn find_from(s: &str, needle: &str, from: usize) -> Option<usize> {
    s.get(from..).and_then(|t| t.find(needle)).map(|i| i + from)
}

/// Igual a `find_from`, mas ignora delimitador escapado com `\`.
fn find_unescaped(s: &str, needle: &str, from: usize) -> Option<usize> {
    let b = s.as_bytes();
    let mut i = from;
    while let Some(at) = find_from(s, needle, i) {
        let mut back = at;
        let mut slashes = 0;
        while back > 0 && b[back - 1] == b'\\' {
            slashes += 1;
            back -= 1;
        }
        if slashes % 2 == 0 {
            return Some(at);
        }
        i = at + needle.len();
    }
    None
}

fn math_env(env: &str, body: &str) -> String {
    if matches!(env, "equation" | "equation*" | "displaymath") {
        format!("$$\n{}\n$$", body.trim())
    } else {
        format!("$$\n\\begin{{{0}}}{1}\\end{{{0}}}\n$$", env, body)
    }
}

/// Separa o texto da matemática. A matemática sai já em Markdown
/// (`$…$` / `$$…$$`) e **não** é tocada por nenhuma outra regra.
pub fn split_math(tex: &str) -> Vec<Span> {
    let b = tex.as_bytes();
    let mut out: Vec<Span> = Vec::new();
    let mut buf = String::new();
    let mut i = 0usize;
    let flush = |buf: &mut String, out: &mut Vec<Span>| {
        if !buf.is_empty() {
            out.push(Span::Text(std::mem::take(buf)));
        }
    };
    while i < b.len() {
        if b[i] == b'\\' {
            if tex[i..].starts_with("\\[") {
                if let Some(end) = find_from(tex, "\\]", i + 2) {
                    flush(&mut buf, &mut out);
                    out.push(Span::Math(format!("$$\n{}\n$$", tex[i + 2..end].trim())));
                    i = end + 2;
                    continue;
                }
            }
            if tex[i..].starts_with("\\(") {
                if let Some(end) = find_from(tex, "\\)", i + 2) {
                    flush(&mut buf, &mut out);
                    out.push(Span::Math(format!("${}$", tex[i + 2..end].trim())));
                    i = end + 2;
                    continue;
                }
            }
            if tex[i..].starts_with("\\begin{") {
                if let Some(close) = find_from(tex, "}", i + 7) {
                    let env = &tex[i + 7..close];
                    if MATH_ENVS.contains(&env) {
                        let endtag = format!("\\end{{{}}}", env);
                        if let Some(e) = find_from(tex, &endtag, close + 1) {
                            flush(&mut buf, &mut out);
                            out.push(Span::Math(math_env(env, &tex[close + 1..e])));
                            i = e + endtag.len();
                            continue;
                        }
                    }
                }
            }
            // Barra escapando alguma coisa: copia sem interpretar.
            buf.push('\\');
            i += 1;
            if let Some(c) = tex[i..].chars().next() {
                buf.push(c);
                i += c.len_utf8();
            }
            continue;
        }
        if b[i] == b'$' {
            let display = tex[i..].starts_with("$$");
            let delim = if display { "$$" } else { "$" };
            let start = i + delim.len();
            if let Some(end) = find_unescaped(tex, delim, start) {
                flush(&mut buf, &mut out);
                let body = &tex[start..end];
                out.push(Span::Math(if display {
                    format!("$$\n{}\n$$", body.trim())
                } else {
                    format!("${}$", body.trim())
                }));
                i = end + delim.len();
                continue;
            }
        }
        let Some(c) = tex[i..].chars().next() else {
            break;
        };
        buf.push(c);
        i += c.len_utf8();
    }
    flush(&mut buf, &mut out);
    out
}

/// Tira os comentários `%` (respeitando `\%`), preservando as quebras de linha.
pub fn strip_comments(tex: &str) -> String {
    let mut out = String::with_capacity(tex.len());
    for line in tex.split('\n') {
        let b = line.as_bytes();
        let mut cut = line.len();
        let mut i = 0;
        while i < b.len() {
            if b[i] == b'\\' {
                i += 2;
                continue;
            }
            if b[i] == b'%' {
                cut = i;
                break;
            }
            i += 1;
        }
        out.push_str(line.get(..cut).unwrap_or(line));
        out.push('\n');
    }
    out
}

/// Só o miolo do documento, quando há preâmbulo.
pub fn document_body(tex: &str) -> String {
    let Some(start) = tex.find("\\begin{document}") else {
        return tex.to_string();
    };
    let from = start + "\\begin{document}".len();
    let end = tex[from..]
        .find("\\end{document}")
        .map(|e| e + from)
        .unwrap_or(tex.len());
    tex[from..end].to_string()
}

/// Conteúdo de `{...}` balanceado começando em `pos`.
fn braced(s: &str, pos: usize) -> Option<(String, usize)> {
    let b = s.as_bytes();
    if b.get(pos) != Some(&b'{') {
        return None;
    }
    let mut depth = 0usize;
    let mut i = pos;
    while i < b.len() {
        match b[i] {
            b'\\' => {
                i += 2;
                continue;
            }
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return s.get(pos + 1..i).map(|v| (v.to_string(), i + 1));
                }
            }
            _ => {}
        }
        i += 1;
    }
    None
}

/// Substitui `\cmd[opt]{arg}` pelo que a função devolver.
fn map_cmd(s: &str, name: &str, f: &dyn Fn(&str) -> String) -> String {
    let pat = format!("\\{}", name);
    let bytes = s.as_bytes();
    let mut out = String::with_capacity(s.len());
    let mut i = 0usize;
    while let Some(rel) = s.get(i..).and_then(|t| t.find(&pat)) {
        let at = i + rel;
        let after = at + pat.len();
        // Não pode ser prefixo de outro comando (`\in` dentro de `\input`).
        if bytes
            .get(after)
            .map(|c| c.is_ascii_alphabetic())
            .unwrap_or(false)
        {
            out.push_str(s.get(i..after).unwrap_or_default());
            i = after;
            continue;
        }
        let mut j = after;
        if bytes.get(j) == Some(&b'*') {
            j += 1;
        }
        while bytes.get(j) == Some(&b'[') {
            match s.get(j..).and_then(|t| t.find(']')) {
                Some(k) => j += k + 1,
                None => break,
            }
        }
        while bytes.get(j) == Some(&b' ') {
            j += 1;
        }
        match braced(s, j) {
            Some((arg, end)) => {
                out.push_str(s.get(i..at).unwrap_or_default());
                out.push_str(&f(&arg));
                i = end;
            }
            None => {
                out.push_str(s.get(i..after).unwrap_or_default());
                i = after;
            }
        }
    }
    out.push_str(s.get(i..).unwrap_or_default());
    out
}

fn drop_cmd(s: &str, name: &str) -> String {
    map_cmd(s, name, &|_| String::new())
}

/// Tira `\cmd` sem argumento (`\noindent`, `\centering`…).
fn drop_bare(s: &str, name: &str) -> String {
    let pat = format!("\\{}", name);
    let mut out = String::with_capacity(s.len());
    let mut i = 0usize;
    while let Some(rel) = s.get(i..).and_then(|t| t.find(&pat)) {
        let at = i + rel;
        let after = at + pat.len();
        let next = s.as_bytes().get(after).copied();
        if next.map(|c| c.is_ascii_alphabetic()).unwrap_or(false) {
            out.push_str(s.get(i..after).unwrap_or_default());
            i = after;
            continue;
        }
        out.push_str(s.get(i..at).unwrap_or_default());
        i = after;
    }
    out.push_str(s.get(i..).unwrap_or_default());
    out
}

fn heading(level: usize) -> impl Fn(&str) -> String {
    move |arg: &str| format!("\n\n{} {}\n\n", "#".repeat(level), squeeze(arg))
}

fn inline_rules(s: &str) -> String {
    let mut t = s.to_string();
    t = map_cmd(&t, "section", &heading(2));
    t = map_cmd(&t, "subsection", &heading(3));
    t = map_cmd(&t, "subsubsection", &heading(4));
    t = map_cmd(&t, "paragraph", &|a| format!("\n\n**{}** ", squeeze(a)));
    t = map_cmd(&t, "subparagraph", &|a| format!("\n\n**{}** ", squeeze(a)));
    t = map_cmd(&t, "title", &heading(1));
    t = map_cmd(&t, "textbf", &|a| format!("**{}**", a));
    t = map_cmd(&t, "textit", &|a| format!("*{}*", a));
    t = map_cmd(&t, "emph", &|a| format!("*{}*", a));
    t = map_cmd(&t, "textsl", &|a| format!("*{}*", a));
    t = map_cmd(&t, "texttt", &|a| format!("`{}`", a));
    t = map_cmd(&t, "textsc", &|a| a.to_string());
    t = map_cmd(&t, "textrm", &|a| a.to_string());
    t = map_cmd(&t, "text", &|a| a.to_string());
    t = map_cmd(&t, "underline", &|a| a.to_string());
    t = map_cmd(&t, "mbox", &|a| a.to_string());
    t = map_cmd(&t, "caption", &|a| format!("\n\n*{}*\n\n", squeeze(a)));
    t = map_cmd(&t, "url", &|a| format!("<{}>", a.trim()));
    // \href{url}{texto}: o primeiro argumento vira o alvo do segundo.
    t = href(&t);
    for c in ["citep", "citet", "citeauthor", "citeyear", "cite"] {
        t = map_cmd(&t, c, &|a| {
            format!(
                "[{}]",
                a.split(',').map(str::trim).collect::<Vec<_>>().join(", ")
            )
        });
    }
    for c in ["eqref", "autoref", "cref", "Cref", "ref"] {
        t = map_cmd(&t, c, &|a| a.trim().to_string());
    }
    t = map_cmd(&t, "footnote", &|a| format!(" ({})", squeeze(a)));
    for c in [
        "label",
        "usepackage",
        "documentclass",
        "bibliographystyle",
        "bibliography",
        "vspace",
        "hspace",
        "setlength",
        "renewcommand",
        "newcommand",
        "def",
        "pagestyle",
        "thispagestyle",
        "graphicspath",
        "includegraphics",
        "index",
        "color",
        "textcolor",
        "affiliation",
        "thanks",
    ] {
        t = drop_cmd(&t, c);
    }
    for c in [
        "maketitle",
        "noindent",
        "centering",
        "clearpage",
        "newpage",
        "linebreak",
        "hline",
        "toprule",
        "midrule",
        "bottomrule",
        "small",
        "footnotesize",
        "scriptsize",
        "large",
        "Large",
        "LARGE",
        "normalsize",
        "bigskip",
        "medskip",
        "smallskip",
        "tableofcontents",
        "protect",
    ] {
        t = drop_bare(&t, c);
    }
    t = t.replace("\\%", "%");
    t = t.replace("\\&", "&");
    t = t.replace("\\_", "_");
    t = t.replace("\\#", "#");
    t = t.replace("\\{", "{");
    t = t.replace("\\}", "}");
    t = t.replace('~', " ");
    t = t.replace("``", "\"").replace("''", "\"");
    t = t.replace("\\\\", "\n");
    t
}

fn href(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut i = 0usize;
    while let Some(rel) = s.get(i..).and_then(|t| t.find("\\href")) {
        let at = i + rel;
        let mut j = at + "\\href".len();
        while s.as_bytes().get(j) == Some(&b' ') {
            j += 1;
        }
        let Some((url, after_url)) = braced(s, j) else {
            out.push_str(s.get(i..j).unwrap_or_default());
            i = j;
            continue;
        };
        match braced(s, after_url) {
            Some((label, end)) => {
                out.push_str(s.get(i..at).unwrap_or_default());
                out.push_str(&format!("[{}]({})", squeeze(&label), url.trim()));
                i = end;
            }
            None => {
                out.push_str(s.get(i..at).unwrap_or_default());
                out.push_str(&format!("<{}>", url.trim()));
                i = after_url;
            }
        }
    }
    out.push_str(s.get(i..).unwrap_or_default());
    out
}

fn env_name(line: &str, kind: &str) -> Option<String> {
    let pat = format!("\\{}{{", kind);
    let at = line.find(&pat)?;
    let from = at + pat.len();
    let end = line[from..].find('}')? + from;
    Some(line[from..end].to_string())
}

/// Listas, ambientes e limpeza final, linha a linha.
fn block_rules(s: &str) -> String {
    let mut out: Vec<String> = Vec::new();
    let mut lists: Vec<&'static str> = Vec::new();
    let mut verbatim: Option<&'static str> = None;
    for raw in s.split('\n') {
        let line = raw.trim_end();
        if let Some(kind) = verbatim {
            if env_name(line, "end").as_deref() == Some(kind) {
                out.push("```".to_string());
                verbatim = None;
            } else {
                out.push(raw.to_string());
            }
            continue;
        }
        if let Some(env) = env_name(line, "begin") {
            match env.as_str() {
                "itemize" => {
                    lists.push("- ");
                    continue;
                }
                "enumerate" => {
                    lists.push("1. ");
                    continue;
                }
                "description" => {
                    lists.push("- ");
                    continue;
                }
                "abstract" => {
                    out.push("\n## Abstract\n".to_string());
                    continue;
                }
                "thebibliography" => {
                    out.push("\n## References\n".to_string());
                    continue;
                }
                "verbatim" | "lstlisting" | "minted" | "algorithmic" | "algorithm" => {
                    out.push("```".to_string());
                    verbatim = Some(match env.as_str() {
                        "verbatim" => "verbatim",
                        "lstlisting" => "lstlisting",
                        "minted" => "minted",
                        "algorithmic" => "algorithmic",
                        _ => "algorithm",
                    });
                    continue;
                }
                "tabular" | "tabularx" | "array" => {
                    out.push("```latex".to_string());
                    verbatim = Some(if env == "tabularx" {
                        "tabularx"
                    } else if env == "array" {
                        "array"
                    } else {
                        "tabular"
                    });
                    continue;
                }
                "quote" | "quotation" => {
                    out.push(">".to_string());
                    continue;
                }
                _ => continue,
            }
        }
        if let Some(env) = env_name(line, "end") {
            if matches!(env.as_str(), "itemize" | "enumerate" | "description") {
                lists.pop();
                out.push(String::new());
            }
            continue;
        }
        if let Some(rest) = line.trim_start().strip_prefix("\\item") {
            let marker = lists.last().copied().unwrap_or("- ");
            let indent = "  ".repeat(lists.len().saturating_sub(1));
            let rest = rest.trim_start().trim_start_matches('[');
            let rest = match rest.find(']') {
                Some(i) if line.contains("\\item[") => &rest[i + 1..],
                _ => rest,
            };
            out.push(format!("{}{}{}", indent, marker, rest.trim()));
            continue;
        }
        if let Some(rest) = line.trim_start().strip_prefix("\\bibitem") {
            let rest = rest.trim_start();
            let rest = match braced(rest, 0) {
                Some((_, end)) => rest.get(end..).unwrap_or("").trim(),
                None => rest,
            };
            out.push(format!("- {}", rest));
            continue;
        }
        out.push(line.to_string());
    }
    let joined = out.join("\n");
    collapse_blanks(&joined)
}

fn collapse_blanks(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut blanks = 0;
    for line in s.split('\n') {
        if line.trim().is_empty() {
            blanks += 1;
            if blanks > 2 {
                continue;
            }
        } else {
            blanks = 0;
        }
        out.push_str(line.trim_end());
        out.push('\n');
    }
    out.trim().to_string()
}

/// LaTeX → Markdown. A matemática é isolada antes de qualquer regra de texto,
/// então `$…$`, `$$…$$`, `\[…\]` e `equation`/`align` chegam intactos no fim.
pub fn latex_to_markdown(tex: &str) -> String {
    let clean = strip_comments(tex);
    let body = document_body(&clean);
    let spans = split_math(&body);
    let mut math: Vec<String> = Vec::new();
    let mut work = String::with_capacity(body.len());
    for span in spans {
        match span {
            Span::Text(t) => work.push_str(&t),
            Span::Math(m) => {
                work.push_str(&format!("\u{1}{}\u{2}", math.len()));
                math.push(m);
            }
        }
    }
    let md = block_rules(&inline_rules(&work));
    restore_math(&md, &math)
}

fn restore_math(md: &str, math: &[String]) -> String {
    let mut out = String::with_capacity(md.len());
    let mut rest = md;
    while let Some(a) = rest.find('\u{1}') {
        out.push_str(&rest[..a]);
        let tail = &rest[a + 1..];
        let Some(b) = tail.find('\u{2}') else {
            out.push_str(tail);
            return out;
        };
        let idx: usize = tail[..b].parse().unwrap_or(usize::MAX);
        match math.get(idx) {
            Some(m) if m.starts_with("$$") => {
                if !out.ends_with('\n') {
                    out.push('\n');
                }
                out.push('\n');
                out.push_str(m);
                out.push('\n');
            }
            Some(m) => out.push_str(m),
            None => {}
        }
        rest = &tail[b + 1..];
    }
    out.push_str(rest);
    collapse_blanks(&out)
}

/// O HTML do LaTeXML guarda o LaTeX original em `alttext`; trocamos o bloco
/// `<math>` por `$…$` antes de converter, senão a fórmula vira sopa de letra.
pub fn mathml_to_tex(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut rest = html;
    while let Some(a) = rest.find("<math") {
        out.push_str(&rest[..a]);
        let tail = &rest[a..];
        let Some(gt) = tail.find('>') else {
            out.push_str(tail);
            return out;
        };
        let head = &tail[..gt];
        let display = head.contains("display=\"block\"");
        let alt = head.find("alttext=\"").and_then(|i| {
            let from = i + "alttext=\"".len();
            head[from..]
                .find('"')
                .map(|q| unescape(&head[from..from + q]))
        });
        let end = match tail.find("</math>") {
            Some(e) => e + "</math>".len(),
            None => {
                out.push_str(tail);
                return out;
            }
        };
        if let Some(tex) = alt {
            let tex = tex.trim();
            let tex = tex.trim_start_matches("\\displaystyle").trim();
            if display {
                out.push_str(&format!("\n\n$$\n{}\n$$\n\n", tex));
            } else {
                out.push_str(&format!("${}$", tex));
            }
        }
        rest = &tail[end..];
    }
    out.push_str(rest);
    out
}

// ── Fluxo ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Default, Deserialize)]
pub struct Options {
    /// ID ou URL do arXiv.
    pub input: String,
    /// Pasta de saída; vazio = não grava.
    #[serde(default)]
    pub output_dir: String,
    /// `auto` (source → html → resumo), `source`, `html` ou `abstract`.
    #[serde(default)]
    pub prefer: String,
    #[serde(default)]
    pub save: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct ArxivDoc {
    pub meta: Meta,
    pub markdown: String,
    /// `latex`, `html` ou `abstract`.
    pub body_source: String,
    pub path: Option<String>,
    pub chars: u64,
    pub math_blocks: u64,
    pub source_files: Vec<String>,
    /// Por que caiu para uma fonte pior, quando caiu.
    pub fallback_reason: Option<String>,
}

fn count_math(md: &str) -> u64 {
    split_math(md)
        .iter()
        .filter(|s| matches!(s, Span::Math(_)))
        .count() as u64
}

pub async fn fetch(opts: Options, p: ProgressFn) -> anyhow::Result<ArxivDoc> {
    let r = parse_id(&opts.input)
        .ok_or_else(|| anyhow!("nao reconheci um id de arXiv em: {}", opts.input.trim()))?;
    report(&p, ID, "started", 0, Some(3), Some(r.full()));

    let client = super::client()?;
    let url = format!("{}{}", API, r.full());
    let xml = client
        .get(&url)
        .send()
        .await?
        .error_for_status()?
        .text()
        .await?;
    let mut meta = parse_atom(&xml)?;
    if meta.version.is_none() {
        meta.version = r.version;
    }
    report(&p, ID, "progress", 1, Some(3), Some(meta.title.clone()));

    let prefer = if opts.prefer.is_empty() {
        "auto"
    } else {
        opts.prefer.as_str()
    };
    let mut fallback: Option<String> = None;
    let mut body_source = "abstract";
    let mut files: Vec<String> = Vec::new();
    let mut body = String::new();

    if matches!(prefer, "auto" | "source") {
        match fetch_source(&client, &r).await {
            Ok(bundle) => {
                body = latex_to_markdown(&bundle.main);
                files = bundle.files;
                body_source = "latex";
            }
            Err(e) => fallback = Some(format!("source LaTeX: {}", e)),
        }
    }
    if body.is_empty() && matches!(prefer, "auto" | "html") {
        report(&p, ID, "progress", 2, Some(3), None);
        match fetch_html(&client, &r).await {
            Ok(md) => {
                body = md;
                body_source = "html";
            }
            Err(e) => {
                let msg = format!("HTML: {}", e);
                fallback = Some(match fallback {
                    Some(prev) => format!("{} · {}", prev, msg),
                    None => msg,
                });
            }
        }
    }
    if body.is_empty() {
        body = format!("## Abstract\n\n{}\n", meta.summary);
        body_source = "abstract";
    }

    let markdown = format!("{}\n# {}\n\n{}\n", front_matter(&meta), meta.title, body);
    let path = if opts.save && !opts.output_dir.is_empty() {
        let dir = std::path::PathBuf::from(&opts.output_dir);
        std::fs::create_dir_all(&dir)?;
        let name = super::sanitize_name(&format!(
            "{} {}.md",
            r.slug(),
            meta.title.chars().take(70).collect::<String>()
        ));
        let file = dir.join(name);
        std::fs::write(&file, &markdown)?;
        Some(file.to_string_lossy().to_string())
    } else {
        None
    };
    report(&p, ID, "done", 3, Some(3), None);

    Ok(ArxivDoc {
        chars: markdown.chars().count() as u64,
        math_blocks: count_math(&markdown),
        meta,
        markdown,
        body_source: body_source.to_string(),
        path,
        source_files: files,
        fallback_reason: fallback,
    })
}

async fn fetch_source(client: &reqwest::Client, r: &ArxivRef) -> anyhow::Result<SourceBundle> {
    let resp = client.get(r.source_url()).send().await?;
    if !resp.status().is_success() {
        return Err(anyhow!("HTTP {}", resp.status()));
    }
    let bytes = resp.bytes().await?;
    extract_source(&bytes)
}

async fn fetch_html(client: &reqwest::Client, r: &ArxivRef) -> anyhow::Result<String> {
    let resp = client.get(r.html_url()).send().await?;
    if !resp.status().is_success() {
        return Err(anyhow!("HTTP {}", resp.status()));
    }
    let html = resp.text().await?;
    let html = mathml_to_tex(&html);
    let md = htmd::convert(&html).map_err(|e| anyhow!("HTML para Markdown: {}", e))?;
    if md.trim().is_empty() {
        return Err(anyhow!("pagina HTML vazia"));
    }
    Ok(md)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reconhece_as_formas_de_id() {
        let cases = [
            ("2401.12345", "2401.12345", None),
            ("2401.12345v2", "2401.12345", Some(2)),
            ("arXiv:2401.12345", "2401.12345", None),
            ("arxiv.org/abs/2401.12345", "2401.12345", None),
            ("https://arxiv.org/abs/2401.12345v3", "2401.12345", Some(3)),
            (
                "https://arxiv.org/pdf/2401.12345v1.pdf",
                "2401.12345",
                Some(1),
            ),
            ("http://arxiv.org/pdf/1706.03762", "1706.03762", None),
            ("https://arxiv.org/html/2401.12345v2", "2401.12345", Some(2)),
            ("  2401.1234 ", "2401.1234", None),
            ("hep-th/9901001", "hep-th/9901001", None),
            (
                "https://arxiv.org/abs/math.GT/0309136v1",
                "math.GT/0309136",
                Some(1),
            ),
        ];
        for (input, id, version) in cases {
            let Some(r) = parse_id(input) else {
                panic!("nao reconheceu {}", input);
            };
            assert_eq!(r.id.to_lowercase(), id.to_lowercase(), "{}", input);
            assert_eq!(r.version, version, "{}", input);
        }
        assert!(parse_id("").is_none());
        assert!(parse_id("um texto qualquer").is_none());
        assert!(parse_id("https://example.com/pagina").is_none());
    }

    #[test]
    fn monta_as_urls() {
        let Some(r) = parse_id("2401.12345v2") else {
            panic!("id");
        };
        assert_eq!(r.full(), "2401.12345v2");
        assert_eq!(r.abs_url(), "https://arxiv.org/abs/2401.12345v2");
        assert_eq!(r.source_url(), "https://arxiv.org/e-print/2401.12345v2");
        assert_eq!(r.slug(), "2401.12345v2");
    }

    const ATOM: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<feed xmlns="http://www.w3.org/2005/Atom">
  <entry>
    <id>http://arxiv.org/abs/1706.03762v7</id>
    <updated>2023-08-02T00:41:18Z</updated>
    <published>2017-06-12T18:44:11Z</published>
    <title>Attention Is All
  You Need</title>
    <summary>  The dominant sequence transduction models are based on complex
recurrent or convolutional neural networks &amp; attention.
</summary>
    <author><name>Ashish Vaswani</name></author>
    <author><name>Noam Shazeer</name></author>
    <arxiv:doi xmlns:arxiv="http://arxiv.org/schemas/atom">10.1000/xyz123</arxiv:doi>
    <arxiv:comment xmlns:arxiv="http://arxiv.org/schemas/atom">15 pages</arxiv:comment>
    <arxiv:primary_category xmlns:arxiv="http://arxiv.org/schemas/atom" term="cs.CL" scheme="http://arxiv.org/schemas/atom"/>
    <category term="cs.CL" scheme="http://arxiv.org/schemas/atom"/>
    <category term="cs.LG" scheme="http://arxiv.org/schemas/atom"/>
  </entry>
</feed>"#;

    #[test]
    fn le_o_atom_da_api() {
        let Ok(m) = parse_atom(ATOM) else {
            panic!("atom");
        };
        assert_eq!(m.title, "Attention Is All You Need");
        assert_eq!(m.id, "1706.03762");
        assert_eq!(m.version, Some(7));
        assert_eq!(m.authors, vec!["Ashish Vaswani", "Noam Shazeer"]);
        assert_eq!(m.categories, vec!["cs.CL", "cs.LG"]);
        assert_eq!(m.primary_category, "cs.CL");
        assert_eq!(m.doi.as_deref(), Some("10.1000/xyz123"));
        assert_eq!(m.comment.as_deref(), Some("15 pages"));
        assert!(m
            .summary
            .contains("recurrent or convolutional neural networks & attention"));
        assert_eq!(m.abs_url, "https://arxiv.org/abs/1706.03762v7");
    }

    #[test]
    fn atom_de_erro_vira_erro() {
        let xml = r#"<feed><entry><id>http://arxiv.org/api/errors#id</id><title>Error</title><summary>incorrect id format</summary></entry></feed>"#;
        assert!(parse_atom(xml).is_err());
        assert!(parse_atom("<feed></feed>").is_err());
    }

    #[test]
    fn front_matter_tem_os_campos() {
        let Ok(m) = parse_atom(ATOM) else {
            panic!("atom");
        };
        let fm = front_matter(&m);
        assert!(fm.starts_with("---\n") && fm.trim_end().ends_with("---"));
        assert!(fm.contains("title: \"Attention Is All You Need\""));
        assert!(fm.contains("  - \"Noam Shazeer\""));
        assert!(fm.contains("categories: [cs.CL, cs.LG]"));
        assert!(fm.contains("doi: \"10.1000/xyz123\""));
        assert!(fm.contains("published: \"2017-06-12T18:44:11Z\""));
        assert!(fm.contains("abstract: |"));
    }

    // ── matemática ──

    #[test]
    fn separa_matematica_do_texto() {
        let spans = split_math("texto $a+b$ meio $$x^2$$ fim");
        assert_eq!(spans.len(), 5);
        assert_eq!(spans[1], Span::Math("$a+b$".to_string()));
        assert_eq!(spans[3], Span::Math("$$\nx^2\n$$".to_string()));
    }

    #[test]
    fn cifrao_escapado_nao_e_matematica() {
        let md = latex_to_markdown("custa \\$5 por mes");
        assert!(md.contains("\\$5"), "{}", md);
    }

    #[test]
    fn preserva_a_matematica_no_markdown() {
        let tex = r"\section{Modelo}
A perda \'e $\mathcal{L} = \sum_i \log p(x_i)$ e a atualiza\c{c}\~ao:
\begin{equation}
\theta_{t+1} = \theta_t - \eta \nabla_\theta \mathcal{L}
\label{eq:sgd}
\end{equation}
Tamb\'em vale \[ e^{i\pi} + 1 = 0 \] e \(x \to \infty\).";
        let md = latex_to_markdown(tex);
        assert!(md.contains("## Modelo"), "{}", md);
        assert!(
            md.contains("$\\mathcal{L} = \\sum_i \\log p(x_i)$"),
            "{}",
            md
        );
        assert!(md.contains("$$\n\\theta_{t+1} = \\theta_t - \\eta \\nabla_\\theta \\mathcal{L}\n\\label{eq:sgd}\n$$"), "{}", md);
        assert!(md.contains("$$\ne^{i\\pi} + 1 = 0\n$$"), "{}", md);
        assert!(md.contains("$x \\to \\infty$"), "{}", md);
    }

    #[test]
    fn ambiente_align_vira_bloco() {
        let md = latex_to_markdown("\\begin{align}\na &= b \\\\\nc &= d\n\\end{align}");
        assert!(md.contains("$$"), "{}", md);
        assert!(md.contains("\\begin{align}"), "{}", md);
        assert!(md.contains("a &= b"), "{}", md);
        // A regra de `\\` -> quebra de linha nao pode entrar na matematica.
        assert!(!md.contains("a &= b  \n"), "{}", md);
    }

    #[test]
    fn comentario_sai_mas_porcento_escapado_fica() {
        let s = strip_comments("texto % comentario\nvalor 50\\% aqui % outro\n");
        assert_eq!(s, "texto \nvalor 50\\% aqui \n\n");
    }

    #[test]
    fn pega_so_o_corpo_do_documento() {
        let tex = "\\documentclass{article}\n\\usepackage{amsmath}\n\\begin{document}\nOi\n\\end{document}\n";
        assert_eq!(document_body(tex).trim(), "Oi");
        assert_eq!(document_body("sem preambulo").trim(), "sem preambulo");
    }

    #[test]
    fn converte_formatacao_e_listas() {
        let tex = r"\begin{document}
\textbf{negrito} e \emph{italico} e \texttt{codigo}.
\begin{itemize}
\item um
\item dois
\end{itemize}
\begin{enumerate}
\item primeiro
\end{enumerate}
Veja \cite{silva2020,souza2021} e \href{https://x.dev}{o site}.
\end{document}";
        let md = latex_to_markdown(tex);
        assert!(md.contains("**negrito**"), "{}", md);
        assert!(md.contains("*italico*"), "{}", md);
        assert!(md.contains("`codigo`"), "{}", md);
        assert!(md.contains("- um\n- dois"), "{}", md);
        assert!(md.contains("1. primeiro"), "{}", md);
        assert!(md.contains("[silva2020, souza2021]"), "{}", md);
        assert!(md.contains("[o site](https://x.dev)"), "{}", md);
    }

    #[test]
    fn mathml_do_latexml_volta_para_tex() {
        let html = r#"<p>a soma <math alttext="\sum_{i=1}^n x_i" display="inline"><mi>x</mi></math> e o bloco <math display="block" alttext="E = mc^2"><mi>E</mi></math>.</p>"#;
        let out = mathml_to_tex(html);
        assert!(out.contains("$\\sum_{i=1}^n x_i$"), "{}", out);
        assert!(out.contains("$$\nE = mc^2\n$$"), "{}", out);
        assert!(!out.contains("<math"), "{}", out);
    }

    // ── tar.gz do source ──

    fn tar_gz(files: &[(&str, &str)]) -> Vec<u8> {
        let mut tar = tar::Builder::new(Vec::new());
        for (name, body) in files {
            let mut header = tar::Header::new_gnu();
            header.set_size(body.len() as u64);
            header.set_mode(0o644);
            header.set_cksum();
            let Ok(()) = tar.append_data(&mut header, name, body.as_bytes()) else {
                panic!("tar");
            };
        }
        let Ok(raw) = tar.into_inner() else {
            panic!("tar");
        };
        use std::io::Write;
        let mut enc = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        let Ok(()) = enc.write_all(&raw) else {
            panic!("gz");
        };
        enc.finish().unwrap_or_default()
    }

    #[test]
    fn extrai_o_tex_principal_do_targz() {
        let gz = tar_gz(&[
            ("figura.eps", "lixo binario"),
            ("secao1.tex", "Conteudo da secao um."),
            (
                "main.tex",
                "\\documentclass{article}\n\\begin{document}\nOla $x$.\n\\input{secao1}\n\\end{document}\n",
            ),
        ]);
        let Ok(b) = extract_source(&gz) else {
            panic!("extrair");
        };
        assert!(b.main.contains("\\begin{document}"));
        assert!(b.main.contains("Conteudo da secao um."), "{}", b.main);
        assert!(b.files.contains(&"main.tex".to_string()));
        assert!(!b.files.contains(&"figura.eps".to_string()));
        let md = latex_to_markdown(&b.main);
        assert!(md.contains("$x$"), "{}", md);
        assert!(md.contains("Conteudo da secao um."), "{}", md);
    }

    #[test]
    fn tex_unico_gzipado_tambem_serve() {
        use std::io::Write;
        let mut enc = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        let Ok(()) = enc.write_all(b"\\begin{document}\nso um arquivo $y$\n\\end{document}") else {
            panic!("gz");
        };
        let gz = enc.finish().unwrap_or_default();
        let Ok(b) = extract_source(&gz) else {
            panic!("extrair");
        };
        assert!(b.main.contains("so um arquivo"));
    }

    #[test]
    fn source_em_pdf_ou_lixo_da_erro() {
        assert!(extract_source(b"%PDF-1.7 blabla").is_err());
        assert!(extract_source(b"texto sem nenhuma barra").is_err());
        let vazio = tar_gz(&[("imagem.png", "abc")]);
        assert!(extract_source(&vazio).is_err());
    }

    #[tokio::test]
    #[ignore = "rede: bate na API publica do arXiv"]
    async fn rede_baixa_o_attention_is_all_you_need() {
        let opts = Options {
            input: "https://arxiv.org/abs/1706.03762".to_string(),
            prefer: "auto".to_string(),
            ..Default::default()
        };
        let Ok(doc) = fetch(opts, super::super::noop_progress()).await else {
            panic!("fetch");
        };
        assert!(doc.meta.title.to_lowercase().contains("attention"));
        assert!(doc.markdown.starts_with("---\n"));
        assert!(doc.chars > 2000, "{}", doc.chars);
    }
}

//! Reparar PDF que não abre. Duas estratégias, na ordem que dá menos dano:
//!
//! 1. **Ghostscript** (quando existe na máquina): interpreta o arquivo inteiro
//!    e reescreve um PDF novo — é o que resolve stream corrompido de verdade.
//! 2. **Reconstrução da xref** (Rust puro, sem dependência): a tabela de
//!    referência cruzada é o que mais quebra (download truncado, edição por
//!    programa ruim, byte a mais no começo). Varre o arquivo procurando os
//!    `N G obj`, monta uma xref nova e um trailer novo no fim. O corpo do
//!    documento não é tocado. PDF 1.5+ guarda a maioria dos objetos
//!    comprimidos dentro de `/Type /ObjStm`: esses são descompactados e
//!    reescritos como objetos normais no fim do arquivo, senão o catálogo
//!    aponta para objetos que a xref nova não sabe onde estão.
//!
//! No fim o resultado é conferido com o PDFium: se não abrir, não é reparo.

use std::path::{Path, PathBuf};

use anyhow::anyhow;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize)]
pub struct RepairOptions {
    pub inputs: Vec<String>,
    /// "auto" (Ghostscript se existir, senão reconstrução) | "gs" | "rebuild"
    #[serde(default = "default_mode")]
    pub mode: String,
    #[serde(default)]
    pub output_dir: String,
    #[serde(default)]
    pub suffix: String,
}

fn default_mode() -> String {
    "auto".into()
}

#[derive(Debug, Clone, Serialize)]
pub struct RepairItem {
    pub input: String,
    pub output: Option<String>,
    /// "ghostscript" | "xref-rebuild"
    pub method: Option<String>,
    pub opened_before: bool,
    pub pages_before: Option<usize>,
    pub pages_after: Option<usize>,
    pub objects_found: usize,
    pub bytes_before: u64,
    pub bytes_after: u64,
    pub ok: bool,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RepairResult {
    pub items: Vec<RepairItem>,
    pub ghostscript: Option<String>,
}

// ── Reconstrução da xref ───────────────────────────────────────────────

fn find_all(hay: &[u8], needle: &[u8]) -> Vec<usize> {
    if needle.is_empty() || hay.len() < needle.len() {
        return Vec::new();
    }
    let mut out = Vec::new();
    let mut i = 0usize;
    while i + needle.len() <= hay.len() {
        if &hay[i..i + needle.len()] == needle {
            out.push(i);
            i += needle.len();
        } else {
            i += 1;
        }
    }
    out
}

/// Faixas `stream …ency endstream`: qualquer "N G obj" aí dentro é conteúdo
/// binário comprimido, não um objeto de verdade.
fn stream_ranges(data: &[u8]) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut i = 0usize;
    while i + 6 <= data.len() {
        if &data[i..i + 6] == b"stream" && !(i >= 3 && &data[i - 3..i] == b"end") {
            let rest = &data[i..];
            match find_first(rest, b"endstream") {
                Some(rel) => {
                    out.push((i, i + rel + 9));
                    i += rel + 9;
                }
                None => {
                    out.push((i, data.len()));
                    break;
                }
            }
        } else {
            i += 1;
        }
    }
    out
}

fn find_first(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w == needle)
}

fn is_ws(b: u8) -> bool {
    matches!(b, b' ' | b'\t' | b'\r' | b'\n' | b'\x0c' | b'\0')
}

fn is_delim(b: u8) -> bool {
    matches!(
        b,
        b'(' | b')' | b'<' | b'>' | b'[' | b']' | b'{' | b'}' | b'/' | b'%'
    )
}

/// Varre o arquivo e devolve `(número do objeto, offset, geração)`, com a
/// última definição de cada número vencendo (é assim que o PDF atualiza).
pub fn scan_objects(data: &[u8]) -> Vec<(u32, usize, u16)> {
    let skip = stream_ranges(data);
    let mut found: std::collections::BTreeMap<u32, (usize, u16)> = Default::default();
    for pos in find_all(data, b"obj") {
        if skip.iter().any(|(a, b)| pos > *a && pos < *b) {
            continue;
        }
        let after = data.get(pos + 3).copied();
        if let Some(c) = after {
            if !is_ws(c) && !is_delim(c) {
                continue;
            }
        }
        // Anda para trás: espaço, geração, espaço, número.
        let mut i = pos;
        while i > 0 && is_ws(data[i - 1]) {
            i -= 1;
        }
        let gen_end = i;
        while i > 0 && data[i - 1].is_ascii_digit() {
            i -= 1;
        }
        let gen_start = i;
        if gen_start == gen_end {
            continue;
        }
        while i > 0 && is_ws(data[i - 1]) {
            i -= 1;
        }
        if i == gen_start {
            continue;
        }
        let num_end = i;
        while i > 0 && data[i - 1].is_ascii_digit() {
            i -= 1;
        }
        let num_start = i;
        if num_start == num_end {
            continue;
        }
        if num_start > 0 && !is_ws(data[num_start - 1]) && !is_delim(data[num_start - 1]) {
            continue;
        }
        let num: u32 = match std::str::from_utf8(&data[num_start..num_end])
            .ok()
            .and_then(|s| s.parse().ok())
        {
            Some(n) => n,
            None => continue,
        };
        let gen: u16 = std::str::from_utf8(&data[gen_start..gen_end])
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(0);
        found.insert(num, (num_start, gen));
    }
    found.into_iter().map(|(n, (o, g))| (n, o, g)).collect()
}

/// Dicionário que vem logo depois da última palavra `trailer`.
fn last_trailer_dict(data: &[u8]) -> Option<String> {
    let start = *find_all(data, b"trailer").last()?;
    let open = find_first(&data[start..], b"<<")? + start;
    let mut depth = 0i32;
    let mut i = open;
    while i + 1 < data.len() {
        if &data[i..i + 2] == b"<<" {
            depth += 1;
            i += 2;
        } else if &data[i..i + 2] == b">>" {
            depth -= 1;
            i += 2;
            if depth == 0 {
                return Some(String::from_utf8_lossy(&data[open..i]).to_string());
            }
        } else {
            i += 1;
        }
    }
    None
}

/// `/Root 12 0 R` — a última menção no arquivo é a que vale.
fn find_root_ref(data: &[u8]) -> Option<String> {
    let text = String::from_utf8_lossy(data);
    let re = regex::Regex::new(r"/Root\s+(\d+)\s+(\d+)\s+R").ok()?;
    let m = re.captures_iter(&text).last()?;
    Some(format!("{} {} R", &m[1], &m[2]))
}

/// Sem trailer nenhum: acha o objeto que é o catálogo do documento.
fn find_catalog(data: &[u8], objects: &[(u32, usize, u16)]) -> Option<String> {
    let re = regex::Regex::new(r"/Type\s*/Catalog").ok()?;
    for (num, off, gen) in objects {
        let end = objects
            .iter()
            .map(|(_, o, _)| *o)
            .filter(|o| o > off)
            .min()
            .unwrap_or(data.len());
        let chunk = String::from_utf8_lossy(&data[*off..end]);
        if re.is_match(&chunk) {
            return Some(format!("{} {} R", num, gen));
        }
    }
    None
}

fn patch_trailer(dict: &str, size: u32) -> String {
    let no_prev = regex::Regex::new(r"/(Prev|XRefStm)\s+\d+")
        .map(|re| re.replace_all(dict, "").to_string())
        .unwrap_or_else(|_| dict.to_string());
    let re_size = regex::Regex::new(r"/Size\s+\d+").ok();
    match re_size {
        Some(re) if re.is_match(&no_prev) => re
            .replace(&no_prev, format!("/Size {}", size).as_str())
            .to_string(),
        _ => no_prev.replacen("<<", &format!("<< /Size {}", size), 1),
    }
}

/// Faixa de bytes de um objeto: do `N G obj` até o começo do próximo (ou o
/// fim do arquivo).
fn object_body(data: &[u8], objects: &[(u32, usize, u16)], idx: usize) -> (usize, usize) {
    let start = objects[idx].1;
    let end = objects
        .iter()
        .map(|(_, o, _)| *o)
        .filter(|o| *o > start)
        .min()
        .unwrap_or(data.len());
    (start, end)
}

fn inflate(data: &[u8]) -> Option<Vec<u8>> {
    use std::io::Read;
    let mut out = Vec::new();
    if flate2::read::ZlibDecoder::new(data)
        .read_to_end(&mut out)
        .is_ok()
        && !out.is_empty()
    {
        return Some(out);
    }
    out.clear();
    // Alguns geradores escrevem deflate cru, sem o cabeçalho zlib.
    flate2::read::DeflateDecoder::new(data)
        .read_to_end(&mut out)
        .ok()
        .filter(|_| !out.is_empty())
        .map(|_| out)
}

fn dict_int(dict: &str, key: &str) -> Option<usize> {
    regex::Regex::new(&format!(r"/{}\s+(\d+)", key))
        .ok()?
        .captures(dict)?
        .get(1)?
        .as_str()
        .parse()
        .ok()
}

/// Abre um `/Type /ObjStm` e devolve `(número, corpo)` de cada objeto que
/// estava comprimido lá dentro.
pub fn expand_object_stream(chunk: &[u8]) -> Vec<(u32, Vec<u8>)> {
    let head = match find_first(chunk, b"stream") {
        Some(p) => p,
        None => return Vec::new(),
    };
    let dict = String::from_utf8_lossy(&chunk[..head]).to_string();
    if !regex::Regex::new(r"/Type\s*/ObjStm")
        .map(|re| re.is_match(&dict))
        .unwrap_or(false)
    {
        return Vec::new();
    }
    if !dict.contains("/FlateDecode") || dict.contains("/Predictor") {
        return Vec::new();
    }
    let (n, first) = match (dict_int(&dict, "N"), dict_int(&dict, "First")) {
        (Some(n), Some(f)) => (n, f),
        _ => return Vec::new(),
    };
    // Pula "stream" e a quebra de linha obrigatória depois dele.
    let mut body = head + 6;
    if chunk.get(body) == Some(&b'\r') {
        body += 1;
    }
    if chunk.get(body) == Some(&b'\n') {
        body += 1;
    }
    let end = match find_first(&chunk[body..], b"endstream") {
        Some(p) => body + p,
        None => chunk.len(),
    };
    let decoded = match inflate(&chunk[body..end]) {
        Some(d) => d,
        None => return Vec::new(),
    };
    if first > decoded.len() {
        return Vec::new();
    }
    let header: Vec<usize> = String::from_utf8_lossy(&decoded[..first])
        .split_whitespace()
        .filter_map(|t| t.parse().ok())
        .collect();
    let mut out = Vec::new();
    for i in 0..n.min(header.len() / 2) {
        let num = header[i * 2] as u32;
        let start = first + header[i * 2 + 1];
        let stop = if i + 1 < n.min(header.len() / 2) {
            first + header[i * 2 + 3]
        } else {
            decoded.len()
        };
        if start <= stop && stop <= decoded.len() {
            out.push((num, decoded[start..stop].to_vec()));
        }
    }
    out
}

/// Reescreve o arquivo com uma xref nova no fim. Devolve `(bytes, objetos)`.
pub fn rebuild_xref(data: &[u8]) -> anyhow::Result<(Vec<u8>, usize)> {
    let head = find_first(data, b"%PDF-")
        .ok_or_else(|| anyhow!("não achei a assinatura %PDF- — o arquivo não parece um PDF"))?;
    // Lixo antes do cabeçalho desloca todos os offsets: corta fora.
    let body = &data[head..];
    let objects = scan_objects(body);
    if objects.is_empty() {
        return Err(anyhow!(
            "nenhum objeto encontrado — arquivo vazio ou cifrado"
        ));
    }

    let mut out = body.to_vec();
    if !out.ends_with(b"\n") {
        out.push(b'\n');
    }
    let mut by_num: std::collections::BTreeMap<u32, (usize, u16)> =
        objects.iter().map(|(n, o, g)| (*n, (*o, *g))).collect();

    // PDF 1.5+ comprime objeto dentro de objeto. Reescreve cada um solto no
    // fim do arquivo — é o que deixa a xref plana voltar a fazer sentido.
    let mut unpacked = 0usize;
    for idx in 0..objects.len() {
        let (a, b) = object_body(body, &objects, idx);
        for (num, content) in expand_object_stream(&body[a..b]) {
            if by_num.contains_key(&num) {
                continue;
            }
            let off = out.len();
            out.extend_from_slice(format!("{} 0 obj\n", num).as_bytes());
            out.extend_from_slice(&content);
            out.extend_from_slice(b"\nendobj\n");
            by_num.insert(num, (off, 0));
            unpacked += 1;
        }
    }
    tracing::debug!("[pdf-repair] {} objetos soltos de object stream", unpacked);

    let max = by_num.keys().copied().max().unwrap_or(0);
    let size = max + 1;
    let all: Vec<(u32, usize, u16)> = by_num.iter().map(|(n, (o, g))| (*n, *o, *g)).collect();
    let trailer = match last_trailer_dict(body) {
        Some(d) if d.contains("/Root") => patch_trailer(&d, size),
        _ => {
            let root = find_root_ref(&out)
                .or_else(|| find_catalog(&out, &all))
                .ok_or_else(|| anyhow!("não achei o catálogo (/Root) do documento"))?;
            format!("<< /Size {} /Root {} >>", size, root)
        }
    };

    let xref_at = out.len();
    out.extend_from_slice(format!("xref\n0 {}\n", size).as_bytes());
    out.extend_from_slice(b"0000000000 65535 f \n");
    for n in 1..size {
        match by_num.get(&n) {
            Some((off, gen)) => {
                out.extend_from_slice(format!("{:010} {:05} n \n", off, gen).as_bytes())
            }
            None => out.extend_from_slice(b"0000000000 65535 f \n"),
        }
    }
    out.extend_from_slice(b"trailer\n");
    out.extend_from_slice(trailer.trim().as_bytes());
    out.extend_from_slice(format!("\nstartxref\n{}\n%%EOF\n", xref_at).as_bytes());
    Ok((out, by_num.len()))
}

// ── Orquestração ───────────────────────────────────────────────────────

async fn open_pages(path: &str) -> Option<usize> {
    let p = path.to_string();
    tokio::task::spawn_blocking(move || super::pdf::info(&p, None).ok().map(|i| i.pages))
        .await
        .ok()
        .flatten()
}

async fn run_ghostscript(gs: &Path, input: &Path, output: &Path) -> anyhow::Result<()> {
    let out = crate::core::process::command(gs)
        .args(["-q", "-dNOPAUSE", "-dBATCH", "-sDEVICE=pdfwrite"])
        .arg(format!("-sOutputFile={}", output.display()))
        .arg(input)
        .output()
        .await
        .map_err(|e| anyhow!("ghostscript nao iniciou: {}", e))?;
    if !output.exists() || std::fs::metadata(output).map(|m| m.len()).unwrap_or(0) == 0 {
        return Err(anyhow!(
            "ghostscript não gerou nada: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    Ok(())
}

fn output_path(opts: &RepairOptions, inp: &Path) -> anyhow::Result<PathBuf> {
    let dir = if opts.output_dir.trim().is_empty() {
        inp.parent().map(|p| p.to_path_buf()).unwrap_or_default()
    } else {
        PathBuf::from(opts.output_dir.trim())
    };
    std::fs::create_dir_all(&dir)?;
    let stem = inp
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "documento".into());
    let suffix = if opts.suffix.is_empty() {
        "-reparado"
    } else {
        opts.suffix.as_str()
    };
    Ok(dir.join(format!("{}{}.pdf", stem, suffix)))
}

pub async fn run(opts: RepairOptions, progress: super::ProgressFn) -> anyhow::Result<RepairResult> {
    let gs = if opts.mode == "rebuild" {
        None
    } else {
        super::pdf::find_gs().await
    };
    if opts.mode == "gs" && gs.is_none() {
        return Err(anyhow!("Ghostscript não encontrado nesta máquina"));
    }
    let total = opts.inputs.len() as u64;
    let mut items = Vec::new();

    for (i, input) in opts.inputs.iter().enumerate() {
        super::report(
            &progress,
            "pdf-repair",
            "progress",
            i as u64,
            Some(total),
            Some(input.clone()),
        );
        let inp = Path::new(input);
        let bytes_before = std::fs::metadata(inp).map(|m| m.len()).unwrap_or(0);
        let pages_before = open_pages(input).await;
        let mut item = RepairItem {
            input: input.clone(),
            output: None,
            method: None,
            opened_before: pages_before.is_some(),
            pages_before,
            pages_after: None,
            objects_found: 0,
            bytes_before,
            bytes_after: 0,
            ok: false,
            error: None,
        };

        let out = match output_path(&opts, inp) {
            Ok(p) => p,
            Err(e) => {
                item.error = Some(e.to_string());
                items.push(item);
                continue;
            }
        };
        let mut errors: Vec<String> = Vec::new();

        if let Some(gs) = gs.as_ref() {
            match run_ghostscript(gs, inp, &out).await {
                Ok(()) => {
                    item.method = Some("ghostscript".into());
                }
                Err(e) => errors.push(e.to_string()),
            }
        }
        // Confere o que o Ghostscript produziu; se não abrir, tenta a xref.
        if item.method.is_some() {
            item.pages_after = open_pages(&out.to_string_lossy()).await;
            if item.pages_after.is_none() {
                item.method = None;
                errors.push("o PDF do ghostscript não abriu".into());
            }
        }
        if item.method.is_none() && opts.mode != "gs" {
            let attempt = std::fs::read(inp)
                .map_err(|e| anyhow!(e))
                .and_then(|data| rebuild_xref(&data));
            match attempt {
                Ok((bytes, objects)) => match std::fs::write(&out, &bytes) {
                    Ok(()) => {
                        item.objects_found = objects;
                        item.pages_after = open_pages(&out.to_string_lossy()).await;
                        if item.pages_after.is_some() {
                            item.method = Some("xref-rebuild".into());
                        } else {
                            errors.push("a xref foi refeita mas o PDFium ainda não abre".into());
                        }
                    }
                    Err(e) => errors.push(e.to_string()),
                },
                Err(e) => errors.push(e.to_string()),
            }
        }

        if item.method.is_some() {
            item.ok = true;
            item.bytes_after = std::fs::metadata(&out).map(|m| m.len()).unwrap_or(0);
            item.output = Some(out.to_string_lossy().to_string());
        } else {
            let _ = std::fs::remove_file(&out);
            item.error = Some(if errors.is_empty() {
                "não consegui reparar".into()
            } else {
                errors.join(" · ")
            });
        }
        items.push(item);
    }

    super::report(&progress, "pdf-repair", "done", total, Some(total), None);
    Ok(RepairResult {
        items,
        ghostscript: gs.map(|p| p.to_string_lossy().to_string()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// PDF mínimo de uma página, com a xref propositalmente errada.
    fn broken_pdf() -> Vec<u8> {
        let body = "%PDF-1.4\n\
1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n\
2 0 obj\n<< /Type /Pages /Kids [3 0 R] /Count 1 >>\nendobj\n\
3 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 200] >>\nendobj\n\
xref\n0 4\n0000000000 65535 f \n0000009999 00000 n \n0000009999 00000 n \n0000009999 00000 n \n\
trailer\n<< /Size 4 /Root 1 0 R >>\nstartxref\n9999\n%%EOF\n";
        body.as_bytes().to_vec()
    }

    #[test]
    fn scan_finds_every_object() {
        let objs = scan_objects(&broken_pdf());
        assert_eq!(objs.len(), 3);
        assert_eq!(objs[0].0, 1);
        assert_eq!(objs[2].0, 3);
        // Offset tem que apontar mesmo para o "3 0 obj".
        let data = broken_pdf();
        assert!(data[objs[2].1..].starts_with(b"3 0 obj"));
    }

    #[test]
    fn rebuilt_xref_points_at_the_objects() {
        let (out, n) = rebuild_xref(&broken_pdf()).unwrap();
        assert_eq!(n, 3);
        let text = String::from_utf8_lossy(&out).to_string();
        let start = text.rfind("startxref\n").unwrap();
        let at: usize = text[start + 10..]
            .lines()
            .next()
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        assert!(
            out[at..].starts_with(b"xref"),
            "startxref tem que cair na xref"
        );
        // Primeira entrada em uso aponta para "1 0 obj".
        // 0: "xref", 1: "0 4", 2: entrada livre, 3: primeiro objeto.
        let table = &text[at..];
        let line = table.lines().nth(3).unwrap();
        let off: usize = line[..10].parse().unwrap();
        assert!(out[off..].starts_with(b"1 0 obj"));
        assert!(text.contains("/Root 1 0 R"));
        assert!(text.trim_end().ends_with("%%EOF"));
    }

    #[test]
    fn xref_entries_are_exactly_twenty_bytes() {
        let (out, _) = rebuild_xref(&broken_pdf()).unwrap();
        let head = b"xref\n0 4\n";
        let at = find_all(&out, head).last().copied().unwrap();
        let table = &out[at + head.len()..];
        for i in 0..4 {
            let entry = &table[i * 20..(i + 1) * 20];
            assert_eq!(entry.len(), 20);
            assert!(
                entry[18] == b' ' && entry[19] == b'\n',
                "EOL da entrada {}",
                i
            );
        }
    }

    #[test]
    fn leading_junk_is_cut_and_offsets_follow() {
        let mut junk = b"<<<lixo de servidor>>>\n".to_vec();
        junk.extend_from_slice(&broken_pdf());
        let (out, _) = rebuild_xref(&junk).unwrap();
        assert!(
            out.starts_with(b"%PDF-"),
            "o arquivo tem que começar no %PDF-"
        );
        let text = String::from_utf8_lossy(&out).to_string();
        let at: usize = text[text.rfind("startxref\n").unwrap() + 10..]
            .lines()
            .next()
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        let line = text[at..].lines().nth(3).unwrap();
        let off: usize = line[..10].parse().unwrap();
        assert!(out[off..].starts_with(b"1 0 obj"));
    }

    #[test]
    fn objects_inside_streams_are_ignored() {
        let mut data = b"%PDF-1.4\n1 0 obj\n<< /Type /Catalog >>\nendobj\n".to_vec();
        data.extend_from_slice(
            b"2 0 obj\n<< /Length 20 >>\nstream\n7 0 obj fake\nendstream\nendobj\n",
        );
        data.extend_from_slice(b"trailer\n<< /Size 3 /Root 1 0 R >>\n%%EOF\n");
        let objs = scan_objects(&data);
        assert_eq!(objs.len(), 2, "o 7 0 obj do stream não conta");
        assert!(objs.iter().all(|(n, _, _)| *n != 7));
    }

    #[test]
    fn missing_trailer_falls_back_to_the_catalog() {
        let data = b"%PDF-1.5\n1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n\
2 0 obj\n<< /Type /Pages /Count 0 >>\nendobj\n"
            .to_vec();
        let (out, _) = rebuild_xref(&data).unwrap();
        let text = String::from_utf8_lossy(&out).to_string();
        assert!(text.contains("/Root 1 0 R"));
        assert!(text.contains("/Size 3"));
    }

    #[test]
    fn trailer_loses_prev_and_gains_the_new_size() {
        let patched = patch_trailer("<< /Size 9 /Root 1 0 R /Prev 12345 >>", 42);
        assert!(patched.contains("/Size 42"));
        assert!(!patched.contains("/Prev"));
        assert!(patched.contains("/Root 1 0 R"));
    }

    /// Teste com um PDF de verdade, ligado à mão:
    /// `OMNIGET_PDF=/caminho/arquivo.pdf cargo test -p omniget-core --lib -- --ignored live_rebuild`
    /// Quebra a xref como um download truncado quebraria, reconstrói e
    /// confere que toda entrada aponta mesmo para o começo de um objeto.
    #[test]
    #[ignore]
    fn live_rebuild_of_a_real_pdf() {
        let path = std::env::var("OMNIGET_PDF").expect("defina OMNIGET_PDF");
        let original = std::fs::read(&path).unwrap();
        let mut broken = original.clone();
        // Aponta o startxref para o nada — o sintoma mais comum.
        let at = find_all(&broken, b"startxref").last().copied().unwrap();
        for b in broken[at + 10..at + 18].iter_mut() {
            if b.is_ascii_digit() {
                *b = b'9';
            }
        }
        let (fixed, objects) = rebuild_xref(&broken).unwrap();
        assert!(objects > 0, "nenhum objeto achado");

        // Tudo em bytes: um PDF real tem stream binário, e converter para
        // String com `from_utf8_lossy` desloca os offsets.
        let sx = find_all(&fixed, b"startxref\n").last().copied().unwrap() + 10;
        let digits: Vec<u8> = fixed[sx..]
            .iter()
            .copied()
            .take_while(|b| b.is_ascii_digit())
            .collect();
        let start: usize = String::from_utf8(digits).unwrap().parse().unwrap();
        assert!(fixed[start..].starts_with(b"xref"));

        let mut checked = 0;
        // 0: "xref", 1: "0 N"; da terceira linha em diante é entrada, e a
        // n-ésima entrada é o objeto de número n.
        for (num, line) in fixed[start..].split(|b| *b == b'\n').skip(2).enumerate() {
            if line.len() < 18 || line[17] != b'n' {
                continue;
            }
            let off: usize = String::from_utf8_lossy(&line[..10]).parse().unwrap();
            let head =
                String::from_utf8_lossy(&fixed[off..(off + 24).min(fixed.len())]).to_string();
            assert!(
                head.starts_with(&format!("{} ", num)) && head.contains("obj"),
                "entrada {} aponta para {:?}",
                num,
                head
            );
            checked += 1;
        }
        assert!(checked > 0, "nenhuma entrada em uso");
        let out = std::env::temp_dir().join("omniget-repair-live.pdf");
        std::fs::write(&out, &fixed).unwrap();
        eprintln!(
            "{} objetos, {} entradas conferidas → {}",
            objects,
            checked,
            out.display()
        );
    }

    #[test]
    fn a_file_without_the_signature_is_refused() {
        assert!(rebuild_xref(b"isso aqui nao e um pdf").is_err());
    }
}

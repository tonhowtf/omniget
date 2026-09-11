//! Tabela de PDF → CSV e XLSX. O PDF não guarda tabela: guarda texto com
//! coordenada. Então o caminho é o do Camelot (MIT) no modo "stream", refeito
//! em Rust: caractere → linha → célula (o `pdf_markdown::lines_from_chars` já
//! entrega isso) → coluna por corredor de espaço em branco → grade.
//!
//! Não há detecção de linha desenhada (o modo "lattice" do Camelot): o PDFium
//! que o app gerencia não expõe os segmentos de caminho. Na prática o modo por
//! posição pega tabela com e sem borda, porque olha o texto, não o traço.

use std::path::{Path, PathBuf};

use anyhow::anyhow;
use serde::{Deserialize, Serialize};

use super::pdf;
use super::pdf_markdown::{lines_from_chars, Line};

const ID: &str = "pdf-table-extract";

#[derive(Debug, Clone, Deserialize)]
pub struct Options {
    pub input: String,
    #[serde(default)]
    pub password: String,
    /// "1-3, 7". Vazio = todas.
    #[serde(default)]
    pub pages: String,
    #[serde(default)]
    pub output_dir: String,
    /// "csv" | "xlsx" | "both"
    #[serde(default)]
    pub format: String,
    /// Vão mínimo, em pontos, para separar duas colunas.
    #[serde(default)]
    pub min_gap: f32,
    #[serde(default)]
    pub min_rows: usize,
    #[serde(default)]
    pub min_cols: usize,
    /// Emenda tabelas de páginas seguidas que têm o mesmo número de colunas.
    #[serde(default)]
    pub merge_pages: bool,
    /// Só analisa e devolve a prévia, sem gravar arquivo.
    #[serde(default)]
    pub preview_only: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct Table {
    /// Página onde a tabela começa.
    pub page: usize,
    /// Páginas emendadas, quando `merge_pages` juntou.
    pub pages: Vec<usize>,
    pub rows: usize,
    pub cols: usize,
    /// Todas as células, linha a linha.
    pub cells: Vec<Vec<String>>,
    pub csv: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct TableResult {
    pub input: String,
    pub pages: usize,
    pub tables: Vec<Table>,
    pub outputs: Vec<String>,
    pub xlsx: Option<String>,
}

// ── Grade ──────────────────────────────────────────────────────────────

/// Faixa horizontal ocupada por uma célula.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Span {
    pub x0: f32,
    pub x1: f32,
}

/// Corredores verticais de espaço em branco viram as bordas das colunas.
/// Devolve as faixas de coluna, da esquerda para a direita.
pub fn columns_of(spans: &[Span], min_gap: f32) -> Vec<(f32, f32)> {
    if spans.is_empty() {
        return Vec::new();
    }
    let mut sorted: Vec<Span> = spans.to_vec();
    sorted.sort_by(|a, b| a.x0.partial_cmp(&b.x0).unwrap_or(std::cmp::Ordering::Equal));
    let mut cols: Vec<(f32, f32)> = Vec::new();
    let (mut x0, mut x1) = (sorted[0].x0, sorted[0].x1);
    for s in &sorted[1..] {
        if s.x0 - x1 >= min_gap {
            cols.push((x0, x1));
            x0 = s.x0;
            x1 = s.x1;
        } else {
            x1 = x1.max(s.x1);
        }
    }
    cols.push((x0, x1));
    cols
}

/// Em qual coluna o pedaço de texto cai. Usa o centro; se ficar fora de
/// todas, escolhe a mais perto.
pub fn column_of(cols: &[(f32, f32)], x0: f32, x1: f32) -> usize {
    let c = (x0 + x1) / 2.0;
    for (i, (a, b)) in cols.iter().enumerate() {
        if c >= *a && c <= *b {
            return i;
        }
    }
    let mut best = 0usize;
    let mut dist = f32::MAX;
    for (i, (a, b)) in cols.iter().enumerate() {
        let d = if c < *a { *a - c } else { c - *b };
        if d < dist {
            dist = d;
            best = i;
        }
    }
    best
}

/// Blocos de linhas que parecem tabela: linhas seguidas, cada uma com pelo
/// menos duas células, sem salto vertical grande no meio.
pub fn blocks_of(lines: &[Line], min_rows: usize) -> Vec<Vec<Line>> {
    let mut out: Vec<Vec<Line>> = Vec::new();
    let mut cur: Vec<Line> = Vec::new();
    let mut last_bottom = f32::MAX;
    for l in lines {
        let multi = l.cells.len() >= 2;
        let jump = last_bottom - l.top > 2.5 * l.size.max(1.0);
        if !multi || (jump && !cur.is_empty()) {
            if cur.len() >= min_rows {
                out.push(std::mem::take(&mut cur));
            } else {
                cur.clear();
            }
        }
        if multi {
            cur.push(l.clone());
            last_bottom = l.bottom;
        } else {
            last_bottom = f32::MAX;
        }
    }
    if cur.len() >= min_rows {
        out.push(cur);
    }
    out
}

/// Um bloco de linhas vira grade: colunas pelo corredor de branco, células
/// pelo centro de cada pedaço de texto.
pub fn grid_of(block: &[Line], min_gap: f32, min_cols: usize) -> Option<Vec<Vec<String>>> {
    let spans: Vec<Span> = block
        .iter()
        .flat_map(|l| l.cells.iter().map(|c| Span { x0: c.x0, x1: c.x1 }))
        .collect();
    let cols = columns_of(&spans, min_gap);
    if cols.len() < min_cols.max(2) {
        return None;
    }
    let mut rows: Vec<Vec<String>> = Vec::with_capacity(block.len());
    for l in block {
        let mut row = vec![String::new(); cols.len()];
        for cell in &l.cells {
            let i = column_of(&cols, cell.x0, cell.x1);
            if row[i].is_empty() {
                row[i] = cell.text.clone();
            } else {
                row[i].push(' ');
                row[i].push_str(&cell.text);
            }
        }
        rows.push(row);
    }
    // Coluna que ficou vazia em todas as linhas não é coluna.
    let keep: Vec<usize> = (0..cols.len())
        .filter(|i| rows.iter().any(|r| !r[*i].trim().is_empty()))
        .collect();
    if keep.len() < min_cols.max(2) {
        return None;
    }
    Some(
        rows.into_iter()
            .map(|r| keep.iter().map(|i| r[*i].clone()).collect())
            .collect(),
    )
}

// ── CSV ────────────────────────────────────────────────────────────────

pub fn csv_field(s: &str) -> String {
    if s.contains(',') || s.contains('"') || s.contains('\n') || s.contains('\r') {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}

pub fn to_csv(rows: &[Vec<String>]) -> String {
    let mut out = String::new();
    for r in rows {
        out.push_str(
            &r.iter()
                .map(|c| csv_field(c.trim()))
                .collect::<Vec<_>>()
                .join(","),
        );
        out.push('\n');
    }
    out
}

/// Célula que é número puro entra no XLSX como número, não como texto.
pub fn as_number(s: &str) -> Option<f64> {
    let t = s.trim();
    if t.is_empty() || t.len() > 24 {
        return None;
    }
    let ok = t
        .chars()
        .all(|c| c.is_ascii_digit() || matches!(c, '-' | '+' | '.' | 'e' | 'E'));
    if !ok {
        return None;
    }
    t.parse::<f64>().ok().filter(|v| v.is_finite())
}

// ── Execução ───────────────────────────────────────────────────────────

fn out_dir(input: &str, output_dir: &str) -> anyhow::Result<PathBuf> {
    let dir = if output_dir.trim().is_empty() {
        Path::new(input.trim())
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_default()
    } else {
        PathBuf::from(output_dir.trim())
    };
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

fn stem(input: &str) -> String {
    Path::new(input.trim())
        .file_stem()
        .map(|s| super::sanitize_name(&s.to_string_lossy()))
        .unwrap_or_else(|| "documento".into())
}

/// Acha as tabelas do documento sem gravar nada.
pub fn scan(opts: &Options, progress: &super::ProgressFn) -> anyhow::Result<Vec<Table>> {
    if opts.input.trim().is_empty() {
        return Err(anyhow!("escolha o PDF"));
    }
    let password = if opts.password.is_empty() {
        None
    } else {
        Some(opts.password.as_str())
    };
    let min_gap = if opts.min_gap <= 0.0 {
        5.0
    } else {
        opts.min_gap
    };
    let min_rows = opts.min_rows.max(2);
    let min_cols = opts.min_cols.max(2);

    let progress2 = progress.clone();
    let pages = pdf::read_pages(
        &opts.input,
        password,
        &opts.pages,
        false,
        move |done, total| {
            super::report(
                &progress2,
                ID,
                "progress",
                done as u64,
                Some(total as u64),
                Some(format!("página {}", done + 1)),
            );
        },
    )?;

    let mut tables: Vec<Table> = Vec::new();
    for page in &pages {
        let lines = lines_from_chars(&page.chars, page.number);
        for block in blocks_of(&lines, min_rows) {
            let Some(cells) = grid_of(&block, min_gap, min_cols) else {
                continue;
            };
            let cols = cells.first().map(Vec::len).unwrap_or(0);
            // Emenda com a tabela anterior quando ela vem da página de trás e
            // tem o mesmo número de colunas.
            if opts.merge_pages {
                if let Some(prev) = tables.last_mut() {
                    let prev_page = prev.pages.last().copied().unwrap_or(prev.page);
                    if prev.cols == cols && page.number == prev_page + 1 {
                        prev.cells.extend(cells);
                        prev.rows = prev.cells.len();
                        prev.pages.push(page.number);
                        continue;
                    }
                }
            }
            tables.push(Table {
                page: page.number,
                pages: vec![page.number],
                rows: cells.len(),
                cols,
                cells,
                csv: None,
            });
        }
    }
    Ok(tables)
}

pub fn run(opts: &Options, progress: &super::ProgressFn) -> anyhow::Result<TableResult> {
    let mut tables = scan(opts, progress)?;
    let total_pages = pdf::info(&opts.input, None).map(|i| i.pages).unwrap_or(0);
    let mut outputs: Vec<String> = Vec::new();
    let mut xlsx_path: Option<String> = None;

    if tables.is_empty() {
        return Ok(TableResult {
            input: opts.input.clone(),
            pages: total_pages,
            tables,
            outputs,
            xlsx: None,
        });
    }

    let format = match opts.format.trim() {
        "" | "csv" => "csv",
        "xlsx" => "xlsx",
        _ => "both",
    };
    for t in tables.iter_mut() {
        t.csv = Some(to_csv(&t.cells));
    }
    if opts.preview_only {
        return Ok(TableResult {
            input: opts.input.clone(),
            pages: total_pages,
            tables,
            outputs,
            xlsx: None,
        });
    }

    let dir = out_dir(&opts.input, &opts.output_dir)?;
    let base = stem(&opts.input);
    if format != "xlsx" {
        for (i, t) in tables.iter().enumerate() {
            let name = format!("{} - tabela {} (p{}).csv", base, i + 1, t.page);
            let path = dir.join(super::sanitize_name(&name));
            std::fs::write(&path, t.csv.clone().unwrap_or_default())?;
            outputs.push(path.to_string_lossy().to_string());
        }
    }
    if format != "csv" {
        let path = dir.join(super::sanitize_name(&format!("{} - tabelas.xlsx", base)));
        write_xlsx(&tables, &path)?;
        xlsx_path = Some(path.to_string_lossy().to_string());
        outputs.push(path.to_string_lossy().to_string());
    }
    super::report(
        progress,
        ID,
        "done",
        tables.len() as u64,
        Some(tables.len() as u64),
        None,
    );
    Ok(TableResult {
        input: opts.input.clone(),
        pages: total_pages,
        tables,
        outputs,
        xlsx: xlsx_path,
    })
}

fn write_xlsx(tables: &[Table], path: &Path) -> anyhow::Result<()> {
    use rust_xlsxwriter::Workbook;
    let mut book = Workbook::new();
    for (i, t) in tables.iter().enumerate() {
        let sheet = book.add_worksheet();
        let name = format!("Tabela {} (p{})", i + 1, t.page);
        // O Excel só aceita 31 caracteres e nada de : \ / ? * [ ].
        let name: String = name
            .chars()
            .filter(|c| !matches!(c, ':' | '\\' | '/' | '?' | '*' | '[' | ']'))
            .take(31)
            .collect();
        sheet.set_name(&name)?;
        for (r, row) in t.cells.iter().enumerate() {
            for (c, cell) in row.iter().enumerate() {
                let (r, c) = (r as u32, c as u16);
                match as_number(cell) {
                    Some(v) => {
                        sheet.write_number(r, c, v)?;
                    }
                    None => {
                        sheet.write_string(r, c, cell.trim())?;
                    }
                }
            }
        }
    }
    book.save(path)
        .map_err(|e| anyhow!("não gravei o xlsx: {}", e))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::tools::pdf::TextChar;

    fn ch(c: char, x: f32, y: f32, size: f32) -> TextChar {
        TextChar {
            ch: c,
            x0: x,
            x1: x + size * 0.5,
            y0: y,
            y1: y + size,
            size,
            mono: false,
            bold: false,
            space_before: false,
        }
    }

    /// Escreve uma palavra a partir de `x`, na linha `y`.
    fn word(text: &str, x: f32, y: f32, size: f32) -> Vec<TextChar> {
        text.chars()
            .enumerate()
            .map(|(i, c)| ch(c, x + i as f32 * size * 0.55, y, size))
            .collect()
    }

    /// Três colunas bem separadas, quatro linhas.
    fn tabela() -> Vec<TextChar> {
        let mut cs = Vec::new();
        let rows = [
            ["Produto", "Qtd", "Preco"],
            ["Cafe", "2", "19.90"],
            ["Cha", "10", "4.50"],
            ["Acucar", "1", "7.25"],
        ];
        for (r, row) in rows.iter().enumerate() {
            let y = 700.0 - r as f32 * 20.0;
            cs.extend(word(row[0], 50.0, y, 10.0));
            cs.extend(word(row[1], 200.0, y, 10.0));
            cs.extend(word(row[2], 300.0, y, 10.0));
        }
        cs
    }

    #[test]
    fn corredor_de_branco_vira_coluna() {
        let spans = vec![
            Span { x0: 50.0, x1: 90.0 },
            Span {
                x0: 200.0,
                x1: 215.0,
            },
            Span {
                x0: 205.0,
                x1: 230.0,
            },
            Span {
                x0: 300.0,
                x1: 340.0,
            },
        ];
        let cols = columns_of(&spans, 5.0);
        assert_eq!(cols.len(), 3, "{:?}", cols);
        assert_eq!(cols[1], (200.0, 230.0));
        assert_eq!(column_of(&cols, 302.0, 320.0), 2);
        // Fora de todas: escolhe a mais perto.
        assert_eq!(column_of(&cols, 95.0, 99.0), 0);
    }

    #[test]
    fn tabela_de_tres_colunas_vira_grade() {
        let lines = lines_from_chars(&tabela(), 1);
        let blocks = blocks_of(&lines, 2);
        assert_eq!(blocks.len(), 1, "{:?}", blocks.len());
        let grid = grid_of(&blocks[0], 5.0, 2).expect("grade");
        assert_eq!(grid.len(), 4);
        assert_eq!(grid[0], vec!["Produto", "Qtd", "Preco"]);
        assert_eq!(grid[2], vec!["Cha", "10", "4.50"]);
    }

    #[test]
    fn linha_de_uma_celula_so_nao_e_tabela() {
        let mut cs = word("Relatorio anual", 50.0, 760.0, 12.0);
        cs.extend(tabela());
        let lines = lines_from_chars(&cs, 1);
        let blocks = blocks_of(&lines, 2);
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].len(), 4, "o título ficou de fora");
    }

    #[test]
    fn texto_corrido_nao_vira_tabela() {
        let mut cs = Vec::new();
        for r in 0..5 {
            cs.extend(word(
                "uma frase comum sem colunas",
                50.0,
                700.0 - r as f32 * 14.0,
                10.0,
            ));
        }
        let lines = lines_from_chars(&cs, 1);
        let grids: Vec<_> = blocks_of(&lines, 2)
            .iter()
            .filter_map(|b| grid_of(b, 5.0, 2))
            .collect();
        assert!(grids.is_empty(), "achou tabela onde não tem: {:?}", grids);
    }

    #[test]
    fn csv_escapa_virgula_e_aspas() {
        assert_eq!(csv_field("simples"), "simples");
        assert_eq!(csv_field("a,b"), "\"a,b\"");
        assert_eq!(csv_field("diz \"oi\""), "\"diz \"\"oi\"\"\"");
        let csv = to_csv(&[vec!["a".into(), "b,c".into()], vec!["1".into(), "2".into()]]);
        assert_eq!(csv, "a,\"b,c\"\n1,2\n");
    }

    #[test]
    fn numero_puro_vira_numero() {
        assert_eq!(as_number(" 19.90 "), Some(19.90));
        assert_eq!(as_number("-3"), Some(-3.0));
        assert_eq!(as_number("R$ 10"), None);
        assert_eq!(as_number("2024-01"), None);
        assert_eq!(as_number(""), None);
    }

    #[test]
    fn xlsx_sai_com_uma_aba_por_tabela() {
        let t = Table {
            page: 1,
            pages: vec![1],
            rows: 2,
            cols: 2,
            cells: vec![
                vec!["Produto".into(), "Preco".into()],
                vec!["Cafe".into(), "19.90".into()],
            ],
            csv: None,
        };
        let dir = std::env::temp_dir().join(format!("omniget-xlsx-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("t.xlsx");
        write_xlsx(&[t], &path).expect("gravar xlsx");
        let bytes = std::fs::read(&path).unwrap();
        // XLSX é um zip: tem que começar com PK.
        assert!(bytes.len() > 1000, "{} bytes", bytes.len());
        assert_eq!(&bytes[..2], b"PK");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// PDF de verdade, montado à mão, com uma tabela de três colunas.
    /// `cargo test -p omniget-core --lib -- --ignored live_table`
    #[test]
    #[ignore]
    fn live_table_extract() {
        let mut content = String::new();
        let rows = [
            ("Produto", "Qtd", "Preco"),
            ("Cafe", "2", "19.90"),
            ("Cha", "10", "4.50"),
            ("Acucar", "1", "7.25"),
        ];
        for (i, (a, b, c)) in rows.iter().enumerate() {
            let y = 700 - i * 24;
            content.push_str(&format!("BT /F1 12 Tf 60 {} Td ({}) Tj ET\n", y, a));
            content.push_str(&format!("BT /F1 12 Tf 220 {} Td ({}) Tj ET\n", y, b));
            content.push_str(&format!("BT /F1 12 Tf 340 {} Td ({}) Tj ET\n", y, c));
        }
        let mut body = String::from("%PDF-1.4\n");
        body.push_str("1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n");
        body.push_str("2 0 obj\n<< /Type /Pages /Kids [3 0 R] /Count 1 >>\nendobj\n");
        body.push_str(
            "3 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 595 792] /Resources << /Font << /F1 4 0 R >> >> /Contents 5 0 R >>\nendobj\n",
        );
        body.push_str("4 0 obj\n<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>\nendobj\n");
        body.push_str(&format!(
            "5 0 obj\n<< /Length {} >>\nstream\n{}endstream\nendobj\n",
            content.len(),
            content
        ));
        let (bytes, _) = super::super::pdf_repair::rebuild_xref(body.as_bytes()).unwrap();
        let dir = std::env::temp_dir().join(format!("omniget-table-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("tabela.pdf");
        std::fs::write(&path, &bytes).unwrap();

        let opts = Options {
            input: path.to_string_lossy().to_string(),
            password: String::new(),
            pages: String::new(),
            output_dir: String::new(),
            format: "both".into(),
            min_gap: 5.0,
            min_rows: 2,
            min_cols: 2,
            merge_pages: false,
            preview_only: false,
        };
        let out = run(&opts, &super::super::noop_progress()).expect("extrair");
        eprintln!("{:?}", out.outputs);
        assert_eq!(out.tables.len(), 1, "{:?}", out.tables);
        let t = &out.tables[0];
        assert_eq!(t.cols, 3, "{:?}", t.cells);
        assert_eq!(t.rows, 4, "{:?}", t.cells);
        assert_eq!(t.cells[0], vec!["Produto", "Qtd", "Preco"]);
        assert!(out.xlsx.is_some());
        let _ = std::fs::remove_dir_all(&dir);
    }
}

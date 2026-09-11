//! PDF → Markdown limpo, offline, para jogar num LLM. Sai texto com títulos,
//! parágrafos remontados, listas, tabelas simples e blocos de código — sem
//! serviço na nuvem e sem crate de PDF: a posição de cada caractere vem do
//! PDFium que o app já gerencia (`pdf::read_pages`).
//!
//! O caminho é sempre o mesmo: caractere → linha (com células separadas por
//! espaço grande) → coluna (histograma de x) → ordem de leitura → bloco →
//! Markdown. As etapas do meio são funções puras sobre `Line`, então dá para
//! testar leitura em duas colunas, hifenização e tabela sem abrir PDF nenhum.
//! Referência de arquitetura: yfedoseev/pdf_oxide (MIT).

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::anyhow;
use serde::{Deserialize, Serialize};

use super::pdf::{self, PageText, TextChar};
use super::ProgressFn;

const ID: &str = "pdf-markdown";

// ── Estruturas intermediárias ──────────────────────────────────────────

/// Um pedaço de linha separado dos vizinhos por um espaço grande. É o que
/// vira célula de tabela e o que preserva o alinhamento de código.
#[derive(Debug, Clone, PartialEq)]
pub struct Cell {
    pub text: String,
    pub x0: f32,
    pub x1: f32,
}

/// Uma linha de texto da página, em pontos do PDF (y cresce para cima).
/// `col0`/`col1` são as bordas da coluna a que a linha pertence — é com elas
/// que se decide se um parágrafo acabou.
#[derive(Debug, Clone, PartialEq)]
pub struct Line {
    pub cells: Vec<Cell>,
    pub x0: f32,
    pub x1: f32,
    pub top: f32,
    pub bottom: f32,
    pub size: f32,
    pub mono: bool,
    pub bold: bool,
    pub col0: f32,
    pub col1: f32,
    pub page: usize,
}

impl Line {
    /// Texto da linha com um espaço entre células.
    pub fn text(&self) -> String {
        let mut out = String::new();
        for c in &self.cells {
            if !out.is_empty() {
                out.push(' ');
            }
            out.push_str(&c.text);
        }
        out
    }

    /// Linha de uma célula só — atalho dos testes.
    #[cfg(test)]
    fn simple(text: &str, x0: f32, x1: f32, top: f32, size: f32) -> Line {
        Line {
            cells: vec![Cell {
                text: text.to_string(),
                x0,
                x1,
            }],
            x0,
            x1,
            top,
            bottom: top - size,
            size,
            mono: false,
            bold: false,
            col0: x0,
            col1: x1,
            page: 1,
        }
    }
}

/// Bloco de Markdown já decidido, antes de virar texto.
#[derive(Debug, Clone, PartialEq)]
pub enum Block {
    Heading { level: u8, text: String },
    Para(String),
    List(Vec<Item>),
    Code(Vec<String>),
    Table(Vec<Vec<String>>),
    Image { src: String, alt: String },
    Rule,
    PageMark(usize),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Item {
    pub level: u8,
    pub ordered: bool,
    /// Marcador como veio do PDF ("3", "a", "•") — a lista numerada sai com o
    /// número do documento, não com uma contagem inventada.
    pub marker: String,
    pub text: String,
}

// ── Caracteres → linhas ────────────────────────────────────────────────

/// Agrupa caracteres em linhas pela altura do centro e corta a linha em
/// células onde o vão horizontal é grande. Espaço em branco do PDF é
/// descartado: o espaçamento sai da geometria, que é mais confiável.
pub fn lines_from_chars(chars: &[TextChar], page: usize) -> Vec<Line> {
    let mut cs: Vec<&TextChar> = chars.iter().filter(|c| !c.ch.is_whitespace()).collect();
    // (o `read_pages` já entrega sem espaço em branco; o filtro é cinto e
    // suspensório para quem montar `TextChar` na mão)
    if cs.is_empty() {
        return Vec::new();
    }
    cs.sort_by(|a, b| {
        let ca = (a.y0 + a.y1) / 2.0;
        let cb = (b.y0 + b.y1) / 2.0;
        cb.partial_cmp(&ca)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(a.x0.partial_cmp(&b.x0).unwrap_or(std::cmp::Ordering::Equal))
    });

    let mut rows: Vec<Vec<&TextChar>> = Vec::new();
    let mut cur: Vec<&TextChar> = Vec::new();
    let mut anchor = 0.0f32;
    let mut anchor_size = 0.0f32;
    for c in cs {
        let center = (c.y0 + c.y1) / 2.0;
        let size = c.size.max(c.y1 - c.y0).max(1.0);
        if cur.is_empty() {
            anchor = center;
            anchor_size = size;
        } else {
            let tol = 0.42 * anchor_size.max(size);
            if (anchor - center).abs() > tol {
                rows.push(std::mem::take(&mut cur));
                anchor = center;
                anchor_size = size;
            }
        }
        anchor_size = anchor_size.max(size);
        cur.push(c);
    }
    if !cur.is_empty() {
        rows.push(cur);
    }

    rows.into_iter()
        .filter_map(|r| row_to_line(r, page))
        .collect()
}

fn row_to_line(mut row: Vec<&TextChar>, page: usize) -> Option<Line> {
    row.sort_by(|a, b| a.x0.partial_cmp(&b.x0).unwrap_or(std::cmp::Ordering::Equal));
    let mut sizes: Vec<f32> = row.iter().map(|c| c.size.max(1.0)).collect();
    sizes.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let size = sizes[sizes.len() / 2];
    let mono = row.iter().filter(|c| c.mono).count() * 10 >= row.len() * 6;
    let bold = row.iter().filter(|c| c.bold).count() * 10 >= row.len() * 6;

    let mut cells: Vec<Cell> = Vec::new();
    let mut text = String::new();
    let (mut cx0, mut cx1) = (row[0].x0, row[0].x1);
    let mut prev_x1 = row[0].x0;
    for c in &row {
        let gap = c.x0 - prev_x1;
        if !text.is_empty() {
            if gap > 1.5 * size {
                cells.push(Cell {
                    text: std::mem::take(&mut text).trim().to_string(),
                    x0: cx0,
                    x1: cx1,
                });
                cx0 = c.x0;
            } else if c.space_before || gap > 0.32 * size {
                text.push(' ');
            }
        } else {
            cx0 = c.x0;
        }
        text.push(c.ch);
        cx1 = c.x1;
        prev_x1 = c.x1;
    }
    if !text.trim().is_empty() {
        cells.push(Cell {
            text: text.trim().to_string(),
            x0: cx0,
            x1: cx1,
        });
    }
    cells.retain(|c| !c.text.is_empty());
    if cells.is_empty() {
        return None;
    }
    let top = row.iter().fold(f32::MIN, |m, c| m.max(c.y1));
    let bottom = row.iter().fold(f32::MAX, |m, c| m.min(c.y0));
    let x0 = cells[0].x0;
    let x1 = cells[cells.len() - 1].x1;
    Some(Line {
        cells,
        x0,
        x1,
        top,
        bottom,
        size,
        mono,
        bold,
        col0: x0,
        col1: x1,
        page,
    })
}

// ── Colunas ────────────────────────────────────────────────────────────

/// Acha as colunas da página por histograma de x: uma faixa vertical quase
/// sem tinta, no meio do conteúdo, é medianiz. Só aceita a divisão se cada
/// coluna tiver conteúdo próprio — página de uma coluna volta como uma.
pub fn detect_columns(lines: &[Line], width: f32) -> Vec<(f32, f32)> {
    let full = |w: f32| vec![(0.0f32, if w > 0.0 { w } else { 1.0 })];
    if lines.len() < 6 || width <= 1.0 {
        return full(width);
    }
    let left = lines.iter().fold(f32::MAX, |m, l| m.min(l.x0));
    let right = lines.iter().fold(f32::MIN, |m, l| m.max(l.x1));
    let span = right - left;
    if span < 0.3 * width {
        return full(width);
    }
    const BINS: usize = 240;
    let mut hist = [0usize; BINS];
    let bin_of = |x: f32| -> usize {
        (((x - left) / span * BINS as f32).floor().max(0.0) as usize).min(BINS - 1)
    };
    for l in lines {
        for c in &l.cells {
            let (a, b) = (bin_of(c.x0), bin_of(c.x1));
            for slot in hist.iter_mut().take(b + 1).skip(a) {
                *slot += 1;
            }
        }
    }
    let noise = (lines.len() / 20).max(1);
    let min_gap = ((0.028 * BINS as f32).ceil() as usize).max(3);
    let guard = (BINS as f32 * 0.15) as usize;

    let mut gaps: Vec<(usize, usize)> = Vec::new();
    let mut start: Option<usize> = None;
    for (b, count) in hist.iter().enumerate().take(BINS - guard).skip(guard) {
        if *count <= noise {
            start.get_or_insert(b);
        } else if let Some(s) = start.take() {
            if b - s >= min_gap {
                gaps.push((s, b));
            }
        }
    }
    if let Some(s) = start {
        if BINS - guard - s >= min_gap {
            gaps.push((s, BINS - guard));
        }
    }
    if gaps.is_empty() {
        return full(width);
    }
    // No máximo três colunas: fica com as maiores medianizes.
    gaps.sort_by_key(|(a, b)| std::cmp::Reverse(b - a));
    gaps.truncate(2);
    gaps.sort_by_key(|(a, _)| *a);

    let to_pt = |b: usize| left + (b as f32 / BINS as f32) * span;
    let mut cols: Vec<(f32, f32)> = Vec::new();
    let mut edge = left - 1.0;
    for (a, b) in &gaps {
        let mid = to_pt((a + b) / 2);
        cols.push((edge, mid));
        edge = mid;
    }
    cols.push((edge, right + 1.0));

    // Só vale se cada coluna se sustenta sozinha e poucas linhas atravessam.
    let mut own = vec![0usize; cols.len()];
    let mut crossing = 0usize;
    for l in lines {
        match split_cols(l, &cols) {
            Some(parts) => {
                for (i, _) in parts {
                    own[i] += 1;
                }
            }
            None => crossing += 1,
        }
    }
    if own.iter().any(|n| *n < 3) || crossing * 4 > lines.len() {
        return full(width);
    }
    cols
}

/// Reparte a linha entre as colunas. `None` quer dizer que uma célula pisa em
/// duas colunas — é linha larga, de título ou de tabela que atravessa a
/// medianiz. Isso é o que permite ler duas colunas mesmo quando as linhas das
/// duas nascem na mesma altura e chegam grudadas do PDFium.
fn split_cols(line: &Line, cols: &[(f32, f32)]) -> Option<Vec<(usize, Line)>> {
    let mut groups: Vec<Vec<Cell>> = vec![Vec::new(); cols.len()];
    for cell in &line.cells {
        let mut hit: Option<usize> = None;
        for (i, (a, b)) in cols.iter().enumerate() {
            if cell.x1.min(*b) - cell.x0.max(*a) > 0.5 {
                if hit.is_some() {
                    return None;
                }
                hit = Some(i);
            }
        }
        groups[hit?].push(cell.clone());
    }
    let mut out = Vec::new();
    for (i, cells) in groups.into_iter().enumerate() {
        if cells.is_empty() {
            continue;
        }
        let mut l = line.clone();
        l.x0 = cells[0].x0;
        l.x1 = cells[cells.len() - 1].x1;
        l.cells = cells;
        l.col0 = cols[i].0;
        l.col1 = cols[i].1;
        out.push((i, l));
    }
    (!out.is_empty()).then_some(out)
}

/// Ordem de leitura: de cima para baixo dentro de cada coluna, coluna a
/// coluna, e uma linha que atravessa as colunas fecha o bloco anterior.
pub fn order_lines(mut lines: Vec<Line>, cols: &[(f32, f32)]) -> Vec<Line> {
    lines.sort_by(|a, b| {
        b.top
            .partial_cmp(&a.top)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    if cols.len() < 2 {
        let (c0, c1) = bounds_of(&lines);
        for l in &mut lines {
            l.col0 = c0;
            l.col1 = c1;
        }
        return lines;
    }
    let mut buckets: Vec<Vec<Line>> = vec![Vec::new(); cols.len()];
    let mut out: Vec<Line> = Vec::new();
    let flush = |buckets: &mut Vec<Vec<Line>>, out: &mut Vec<Line>| {
        for b in buckets.iter_mut() {
            let (c0, c1) = bounds_of(b);
            for mut l in b.drain(..) {
                l.col0 = c0;
                l.col1 = c1;
                out.push(l);
            }
        }
    };
    for line in lines {
        match split_cols(&line, cols) {
            Some(parts) => {
                for (i, l) in parts {
                    buckets[i].push(l);
                }
            }
            None => {
                flush(&mut buckets, &mut out);
                let mut l = line;
                l.col0 = cols[0].0;
                l.col1 = cols[cols.len() - 1].1;
                out.push(l);
            }
        }
    }
    flush(&mut buckets, &mut out);
    out
}

fn bounds_of(lines: &[Line]) -> (f32, f32) {
    let c0 = lines.iter().fold(f32::MAX, |m, l| m.min(l.x0));
    let c1 = lines.iter().fold(f32::MIN, |m, l| m.max(l.x1));
    if c0 > c1 {
        (0.0, 1.0)
    } else {
        (c0, c1)
    }
}

// ── Cabeçalho e rodapé que se repetem ──────────────────────────────────

/// Texto comparável entre páginas: sem caixa, sem espaço duplo e com todo
/// número virando `#`, para "Página 3 de 12" bater com "Página 4 de 12".
fn running_key(text: &str) -> String {
    let mut out = String::new();
    let mut digit = false;
    for ch in text.trim().to_lowercase().chars() {
        if ch.is_ascii_digit() {
            if !digit {
                out.push('#');
                digit = true;
            }
            continue;
        }
        digit = false;
        if ch.is_whitespace() {
            if !out.ends_with(' ') {
                out.push(' ');
            }
        } else {
            out.push(ch);
        }
    }
    out.trim().to_string()
}

/// Tira das faixas de topo e pé as linhas que se repetem na maioria das
/// páginas — cabeçalho corrido e número de página.
pub fn strip_running(pages: &mut [Vec<Line>], heights: &[f32]) {
    if pages.len() < 2 {
        return;
    }
    let band = |l: &Line, h: f32| h > 1.0 && (l.bottom > h * 0.88 || l.top < h * 0.12);
    let mut seen: HashMap<String, Vec<usize>> = HashMap::new();
    for (i, lines) in pages.iter().enumerate() {
        let h = heights.get(i).copied().unwrap_or(0.0);
        let mut here: Vec<String> = Vec::new();
        for l in lines.iter().filter(|l| band(l, h)) {
            let k = running_key(&l.text());
            if k.is_empty() || here.contains(&k) {
                continue;
            }
            here.push(k.clone());
            seen.entry(k).or_default().push(i);
        }
    }
    let need = ((pages.len() * 3) / 5).max(2);
    let repeated: Vec<String> = seen
        .into_iter()
        .filter(|(_, hits)| hits.len() >= need)
        .map(|(k, _)| k)
        .collect();
    for (i, lines) in pages.iter_mut().enumerate() {
        let h = heights.get(i).copied().unwrap_or(0.0);
        lines.retain(|l| {
            if !band(l, h) {
                return true;
            }
            let k = running_key(&l.text());
            // Número de página solto vira "#" e cai fora sem precisar repetir.
            !(repeated.contains(&k) || k == "#" || k.is_empty())
        });
    }
}

// ── Corpo do texto, títulos, listas ────────────────────────────────────

/// Corpo do texto = tamanho de fonte com mais caracteres na obra. É a régua
/// de tudo: título é o que é maior que isso.
pub fn body_size(pages: &[Vec<Line>]) -> f32 {
    let mut hist: HashMap<i32, usize> = HashMap::new();
    for lines in pages {
        for l in lines {
            let weight = l
                .cells
                .iter()
                .map(|c| c.text.chars().count())
                .sum::<usize>();
            *hist.entry((l.size * 2.0).round() as i32).or_default() += weight;
        }
    }
    hist.into_iter()
        .max_by_key(|(size, weight)| (*weight, *size))
        .map(|(size, _)| size as f32 / 2.0)
        .filter(|s| *s > 0.5)
        .unwrap_or(10.0)
}

/// Espaçamento típico entre linhas, para saber quando o salto virou
/// parágrafo novo.
pub fn median_lead(lines: &[Line], body: f32) -> f32 {
    let mut leads: Vec<f32> = lines
        .windows(2)
        .map(|w| w[0].top - w[1].top)
        .filter(|d| *d > 0.1 && *d < 4.0 * body)
        .collect();
    if leads.is_empty() {
        return body * 1.2;
    }
    leads.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    leads[leads.len() / 2]
}

/// Nível de título pela razão entre o corpo da fonte da linha e o do texto.
/// Linha grande é título; linha em negrito, curta e sem ponto final também.
pub fn heading_level(line: &Line, body: f32) -> Option<u8> {
    if body <= 0.0 || line.mono {
        return None;
    }
    let text = line.text();
    let len = text.chars().count();
    if len == 0 || len > 140 {
        return None;
    }
    let ratio = line.size / body;
    if ratio >= 1.6 {
        return Some(1);
    }
    if ratio >= 1.32 {
        return Some(2);
    }
    if ratio >= 1.13 {
        return Some(3);
    }
    let ends_sentence = text.ends_with(['.', ',', ';', '!', '?']);
    if line.bold && ratio >= 0.97 && len <= 80 && !ends_sentence {
        return Some(3);
    }
    None
}

/// Marcador de lista no começo da linha: `(marcador, é numerada, resto)`.
pub fn split_marker(text: &str) -> Option<(String, bool, String)> {
    let t = text.trim_start();
    let mut it = t.chars();
    let first = it.next()?;
    if "•◦‣▪▫·-–—*".contains(first) {
        let rest = it.as_str().trim_start();
        if rest.is_empty() || t.chars().nth(1).is_some_and(|c| !c.is_whitespace()) {
            return None;
        }
        return Some((first.to_string(), false, rest.to_string()));
    }
    // 1. / 1) / (1) / a) / iv.
    let (open, body) = if first == '(' {
        (true, it.as_str())
    } else {
        (false, t)
    };
    let mark: String = body
        .chars()
        .take_while(|c| c.is_ascii_alphanumeric())
        .collect();
    if mark.is_empty() || mark.len() > 4 {
        return None;
    }
    if mark.chars().all(|c| c.is_ascii_digit()) {
        if mark.len() > 3 {
            return None;
        }
    } else if mark.len() > 1 || !mark.chars().all(|c| c.is_ascii_alphabetic()) {
        return None;
    }
    let after = &body[mark.len()..];
    let closer = after.chars().next()?;
    let ok = if open {
        closer == ')'
    } else {
        closer == '.' || closer == ')'
    };
    if !ok {
        return None;
    }
    let rest = after[closer.len_utf8()..].trim_start();
    if rest.is_empty() {
        return None;
    }
    let ordered = mark.chars().all(|c| c.is_ascii_digit());
    Some((mark, ordered, rest.to_string()))
}

/// Junta linhas de um parágrafo desfazendo a hifenização de fim de linha.
pub fn join_lines(parts: &[String]) -> String {
    let mut out = String::new();
    for part in parts {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        if out.is_empty() {
            out.push_str(part);
            continue;
        }
        // Hífen de fim de linha: some antes de minúscula ("legí-vel") e fica
        // quando é composto de verdade ("Rio-Niterói"), mas nunca vira espaço.
        if out.ends_with(['-', '\u{2010}', '\u{00ad}']) {
            if part.chars().next().is_some_and(char::is_lowercase) {
                out.pop();
                while out.ends_with(' ') {
                    out.pop();
                }
            }
            out.push_str(part);
        } else {
            out.push(' ');
            out.push_str(part);
        }
    }
    out
}

// ── Tabela ─────────────────────────────────────────────────────────────

/// Tenta montar uma tabela com as linhas dadas: as células têm que cair em
/// colunas de x alinhadas em quase todas as linhas. Sem confiança devolve
/// `None` — é melhor sair como texto do que sair torta.
pub fn try_table(rows: &[Line]) -> Option<Vec<Vec<String>>> {
    let corpo = rows.iter().filter(|r| r.cells.len() >= 2).count();
    if rows.len() < 2 || corpo < 2 {
        return None;
    }
    let size = rows.iter().map(|r| r.size).fold(0.0f32, f32::max).max(1.0);
    let tol = 1.6 * size;
    let mut anchors: Vec<f32> = Vec::new();
    for r in rows.iter().filter(|r| r.cells.len() >= 2) {
        for c in &r.cells {
            match anchors.iter_mut().find(|a| (**a - c.x0).abs() <= tol) {
                Some(a) => *a = (*a + c.x0) / 2.0,
                None => anchors.push(c.x0),
            }
        }
    }
    anchors.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let widest = rows.iter().map(|r| r.cells.len()).max().unwrap_or(0);
    // Mais colunas do que a linha mais cheia quer dizer célula fora de prumo:
    // é texto com vão grande, não tabela.
    if anchors.len() < 2 || anchors.len() > 12 || anchors.len() > widest {
        return None;
    }
    let mut grid: Vec<Vec<String>> = Vec::with_capacity(rows.len());
    let mut filled = 0usize;
    for r in rows {
        // Linha de uma célula só que cai numa coluna do meio é a continuação
        // da célula de cima, não uma linha nova.
        if r.cells.len() < 2 {
            let cell = &r.cells[0];
            match (nearest(&anchors, cell.x0, tol), grid.last_mut()) {
                (Some(i), Some(last)) if i > 0 => {
                    last[i] = join_lines(&[last[i].clone(), cell.text.clone()]);
                    continue;
                }
                _ => return None,
            }
        }
        let mut cells = vec![String::new(); anchors.len()];
        for c in &r.cells {
            let idx = match nearest(&anchors, c.x0, tol) {
                Some(i) if cells[i].is_empty() => i,
                _ => return None,
            };
            cells[idx] = c.text.clone();
            filled += 1;
        }
        grid.push(cells);
    }
    // Tabela esburacada demais não convence.
    if grid.is_empty() || filled * 10 < grid.len() * anchors.len() * 6 {
        return None;
    }
    Some(grid)
}

/// Índice da coluna mais perto de `x`, dentro da tolerância.
fn nearest(anchors: &[f32], x: f32, tol: f32) -> Option<usize> {
    let (idx, dist) = anchors
        .iter()
        .enumerate()
        .fold((0usize, f32::MAX), |acc, (i, a)| {
            let d = (a - x).abs();
            if d < acc.1 {
                (i, d)
            } else {
                acc
            }
        });
    (dist <= tol).then_some(idx)
}

// ── Linhas → blocos ────────────────────────────────────────────────────

pub struct Shape {
    pub body: f32,
    pub lead: f32,
    pub tables: bool,
    pub code: bool,
}

/// Monta os blocos de uma página já em ordem de leitura. Devolve cada bloco
/// com o `y` de onde ele começa, para intercalar imagem depois.
pub fn assemble(lines: &[Line], shape: &Shape) -> Vec<(f32, Block)> {
    let mut out: Vec<(f32, Block)> = Vec::new();
    let mut i = 0usize;
    while i < lines.len() {
        let l = &lines[i];
        if shape.code && l.mono {
            let mut j = i;
            while j < lines.len() && lines[j].mono {
                j += 1;
            }
            out.push((l.top, code_block(&lines[i..j])));
            i = j;
            continue;
        }
        if let Some(level) = heading_level(l, shape.body) {
            out.push((
                l.top,
                Block::Heading {
                    level,
                    text: l.text(),
                },
            ));
            i += 1;
            continue;
        }
        if split_marker(&l.text()).is_some() {
            let mut j = i + 1;
            let base = l.x0;
            while j < lines.len() {
                let n = &lines[j];
                if n.mono || heading_level(n, shape.body).is_some() {
                    break;
                }
                let gap = lines[j - 1].top - n.top;
                if gap > 2.2 * shape.lead {
                    break;
                }
                if split_marker(&n.text()).is_none() && n.x0 < base + 0.4 * shape.body {
                    break;
                }
                j += 1;
            }
            out.push((l.top, list_block(&lines[i..j], shape)));
            i = j;
            continue;
        }
        // Corrida de texto até o próximo título, código ou marcador.
        let mut j = i + 1;
        while j < lines.len() {
            let n = &lines[j];
            if n.mono || heading_level(n, shape.body).is_some() || split_marker(&n.text()).is_some()
            {
                break;
            }
            j += 1;
        }
        out.extend(text_blocks(&lines[i..j], shape));
        i = j;
    }
    out
}

/// Dentro de uma corrida de texto, o que forma tabela vira tabela e o resto
/// vira parágrafo.
fn text_blocks(run: &[Line], shape: &Shape) -> Vec<(f32, Block)> {
    let mut out = Vec::new();
    let mut i = 0usize;
    while i < run.len() {
        if shape.tables && run[i].cells.len() >= 2 {
            let mut j = i;
            loop {
                let antes = j;
                while j < run.len() && run[j].cells.len() >= 2 {
                    j += 1;
                }
                // Puxa a linha solta logo abaixo quando ela não começa na
                // primeira coluna: é célula que quebrou em duas linhas.
                while j < run.len()
                    && run[j].cells.len() == 1
                    && run[j].x0 > run[i].cells[0].x1 + 0.5 * shape.body
                {
                    j += 1;
                }
                if j == antes {
                    break;
                }
            }
            if let Some(grid) = try_table(&run[i..j]) {
                out.push((run[i].top, Block::Table(grid)));
                i = j;
                continue;
            }
        }
        // Não virou tabela: segue como texto até o começo da próxima tentativa.
        let mut j = i + 1;
        while j < run.len() && !(shape.tables && run[j].cells.len() >= 2) {
            j += 1;
        }
        if shape.tables && j < run.len() {
            let mut k = j;
            while k < run.len() && run[k].cells.len() >= 2 {
                k += 1;
            }
            if try_table(&run[j..k]).is_none() {
                j = k; // a próxima corrida também não é tabela: segue no texto
                while j < run.len() && run[j].cells.len() < 2 {
                    j += 1;
                }
            }
        }
        out.extend(paragraphs(&run[i..j], shape));
        i = j;
    }
    out
}

/// Quebra a corrida em parágrafos e junta as linhas de cada um.
pub fn paragraphs(run: &[Line], shape: &Shape) -> Vec<(f32, Block)> {
    let mut out = Vec::new();
    let mut buf: Vec<String> = Vec::new();
    let mut top = 0.0f32;
    for (i, l) in run.iter().enumerate() {
        if i > 0 && breaks_paragraph(&run[i - 1], l, shape) && !buf.is_empty() {
            out.push((top, Block::Para(join_lines(&buf))));
            buf.clear();
        }
        if buf.is_empty() {
            top = l.top;
        }
        buf.push(l.text());
    }
    if !buf.is_empty() {
        out.push((top, Block::Para(join_lines(&buf))));
    }
    out
}

fn breaks_paragraph(prev: &Line, cur: &Line, shape: &Shape) -> bool {
    let lead = prev.top - cur.top;
    if lead > 1.45 * shape.lead {
        return true;
    }
    if (prev.size - cur.size).abs() > 0.15 * shape.body {
        return true;
    }
    // Primeira linha recuada começa parágrafo.
    if cur.x0 > prev.col0 + 0.9 * shape.body && (prev.x0 - prev.col0).abs() < 0.9 * shape.body {
        return true;
    }
    // Linha anterior terminou a frase e sobrou muito branco à direita.
    let text = prev.text();
    let closed = text.ends_with(['.', '!', '?', '"', '”', '»']);
    let width = (prev.col1 - prev.col0).max(1.0);
    closed && prev.x1 < prev.col1 - 0.3 * width
}

fn list_block(run: &[Line], shape: &Shape) -> Block {
    let base = run.iter().fold(f32::MAX, |m, l| m.min(l.x0));
    let step = (1.6 * shape.body).max(1.0);
    let mut items: Vec<Item> = Vec::new();
    for l in run {
        let text = l.text();
        match split_marker(&text) {
            Some((marker, ordered, rest)) => {
                let level = (((l.x0 - base) / step).round().max(0.0) as u8).min(3);
                items.push(Item {
                    level,
                    ordered,
                    marker,
                    text: rest,
                });
            }
            None => {
                if let Some(last) = items.last_mut() {
                    last.text = join_lines(&[last.text.clone(), text]);
                }
            }
        }
    }
    Block::List(items)
}

fn code_block(run: &[Line]) -> Block {
    let size = run.iter().map(|l| l.size).fold(0.0f32, f32::max).max(1.0);
    let char_w = (0.6 * size).max(1.0);
    let base = run.iter().fold(f32::MAX, |m, l| m.min(l.x0));
    let mut out = Vec::with_capacity(run.len());
    for l in run {
        let indent = (((l.x0 - base) / char_w).round().max(0.0) as usize).min(40);
        let mut text = " ".repeat(indent);
        let mut prev_x1: Option<f32> = None;
        for c in &l.cells {
            if let Some(x1) = prev_x1 {
                let n = (((c.x0 - x1) / char_w).round().max(1.0) as usize).min(40);
                text.push_str(&" ".repeat(n));
            }
            text.push_str(&c.text);
            prev_x1 = Some(c.x1);
        }
        out.push(text.trim_end().to_string());
    }
    Block::Code(out)
}

// ── Blocos → Markdown ──────────────────────────────────────────────────

fn escape_cell(text: &str) -> String {
    text.replace('|', "\\|").replace('\n', " ")
}

pub fn render(blocks: &[Block]) -> String {
    let mut parts: Vec<String> = Vec::new();
    for b in blocks {
        match b {
            Block::Heading { level, text } => {
                parts.push(format!("{} {}", "#".repeat(*level as usize), text.trim()))
            }
            Block::Para(text) => parts.push(text.trim().to_string()),
            Block::List(items) => {
                let mut lines = Vec::with_capacity(items.len());
                for it in items {
                    let pad = "  ".repeat(it.level as usize);
                    if it.ordered {
                        lines.push(format!("{}{}. {}", pad, it.marker, it.text.trim()));
                    } else {
                        lines.push(format!("{}- {}", pad, it.text.trim()));
                    }
                }
                parts.push(lines.join("\n"));
            }
            Block::Code(lines) => parts.push(format!("```\n{}\n```", lines.join("\n"))),
            Block::Table(grid) => {
                let cols = grid.iter().map(Vec::len).max().unwrap_or(0);
                if cols == 0 {
                    continue;
                }
                let row = |cells: &Vec<String>| {
                    let mut c: Vec<String> = cells.iter().map(|s| escape_cell(s.trim())).collect();
                    c.resize(cols, String::new());
                    format!("| {} |", c.join(" | "))
                };
                let mut lines = vec![row(&grid[0]), format!("|{}", " --- |".repeat(cols))];
                for r in &grid[1..] {
                    lines.push(row(r));
                }
                parts.push(lines.join("\n"));
            }
            Block::Image { src, alt } => parts.push(format!("![{}]({})", alt, src)),
            Block::Rule => parts.push("---".into()),
            Block::PageMark(n) => parts.push(format!("<!-- page {} -->", n)),
        }
    }
    let text = parts
        .into_iter()
        .filter(|p| !p.trim().is_empty())
        .collect::<Vec<_>>()
        .join("\n\n");
    format!("{}\n", text.trim_end())
}

// ── Tool ───────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct Options {
    pub input: String,
    /// "1-3, 5" ou vazio para todas.
    pub pages: String,
    /// Caminho do .md; vazio grava ao lado do PDF.
    pub output: String,
    pub save: bool,
    /// "none" | "rule" | "comment"
    pub page_marks: String,
    pub extract_images: bool,
    /// Manter cabeçalho e rodapé repetidos.
    pub keep_running: bool,
    pub tables: bool,
    pub code: bool,
    pub password: Option<String>,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            input: String::new(),
            pages: String::new(),
            output: String::new(),
            save: true,
            page_marks: "comment".into(),
            extract_images: false,
            keep_running: false,
            tables: true,
            code: true,
            password: None,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct PageReport {
    pub page: usize,
    pub chars: usize,
    pub columns: usize,
    pub headings: usize,
    pub tables: usize,
    /// Página sem texto: é digitalização, precisa de OCR.
    pub needs_ocr: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct MdResult {
    pub markdown: String,
    pub output: Option<String>,
    pub images_dir: Option<String>,
    pub images: usize,
    pub pages: Vec<PageReport>,
    pub needs_ocr: usize,
    pub headings: usize,
    pub tables: usize,
    pub words: usize,
    pub bytes: u64,
}

fn stem(path: &Path) -> String {
    path.file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "documento".into())
}

fn unique(path: PathBuf) -> PathBuf {
    if !path.exists() {
        return path;
    }
    let dir = path.parent().map(Path::to_path_buf).unwrap_or_default();
    let base = stem(&path);
    let ext = path
        .extension()
        .map(|e| format!(".{}", e.to_string_lossy()))
        .unwrap_or_default();
    for n in 2..1000 {
        let cand = dir.join(format!("{} ({}){}", base, n, ext));
        if !cand.exists() {
            return cand;
        }
    }
    path
}

pub fn run(opts: &Options, progress: &ProgressFn) -> anyhow::Result<MdResult> {
    let input = opts.input.trim();
    if input.is_empty() {
        return Err(anyhow!("escolha um PDF"));
    }
    let path = PathBuf::from(input);
    super::report(progress, ID, "read", 0, None, Some("PDF".into()));
    let pages: Vec<PageText> = pdf::read_pages(
        input,
        opts.password.as_deref().filter(|p| !p.is_empty()),
        &opts.pages,
        opts.extract_images,
        |done, total| {
            super::report(
                progress,
                ID,
                "read",
                done as u64,
                Some(total as u64),
                Some(format!("{}/{}", done, total)),
            );
        },
    )?;
    if pages.is_empty() {
        return Err(anyhow!("nenhuma pagina selecionada"));
    }

    let mut per_page: Vec<Vec<Line>> = Vec::with_capacity(pages.len());
    let mut columns: Vec<usize> = Vec::with_capacity(pages.len());
    let heights: Vec<f32> = pages.iter().map(|p| p.height).collect();
    for page in &pages {
        let lines = lines_from_chars(&page.chars, page.number);
        let cols = detect_columns(&lines, page.width);
        columns.push(cols.len());
        per_page.push(order_lines(lines, &cols));
    }
    if !opts.keep_running {
        strip_running(&mut per_page, &heights);
    }
    let body = body_size(&per_page);

    // Pasta das imagens, só criada se sair alguma.
    let out_path = if opts.output.trim().is_empty() {
        unique(
            path.parent()
                .unwrap_or(Path::new("."))
                .join(format!("{}.md", stem(&path))),
        )
    } else {
        PathBuf::from(opts.output.trim())
    };
    let img_dir_name = format!("{}-images", stem(&out_path));
    let img_dir = out_path
        .parent()
        .unwrap_or(Path::new("."))
        .join(&img_dir_name);

    let mut blocks: Vec<Block> = Vec::new();
    let mut reports: Vec<PageReport> = Vec::new();
    let mut images = 0usize;
    let (mut headings, mut tables) = (0usize, 0usize);

    for (i, page) in pages.iter().enumerate() {
        super::report(
            progress,
            ID,
            "layout",
            i as u64,
            Some(pages.len() as u64),
            Some(format!("{}/{}", i + 1, pages.len())),
        );
        let lines = &per_page[i];
        let chars = page.chars.iter().filter(|c| !c.ch.is_whitespace()).count();
        let needs_ocr = chars < 12;
        match opts.page_marks.as_str() {
            "rule" => {
                if i > 0 {
                    blocks.push(Block::Rule);
                }
            }
            "none" => {}
            _ => blocks.push(Block::PageMark(page.number)),
        }
        let shape = Shape {
            body,
            lead: median_lead(lines, body),
            tables: opts.tables,
            code: opts.code,
        };
        let mut made = assemble(lines, &shape);

        // Imagens entram na posição delas quando a página é de uma coluna;
        // com duas, o y não é monotônico, então vão para o fim da página.
        let mut shots: Vec<(f32, Block)> = Vec::new();
        for (n, im) in page.images.iter().enumerate() {
            let file = format!("p{}-{}.png", page.number, n + 1);
            std::fs::create_dir_all(&img_dir)?;
            std::fs::write(img_dir.join(&file), &im.png)?;
            images += 1;
            shots.push((
                im.y1,
                Block::Image {
                    src: format!("{}/{}", img_dir_name, file),
                    alt: format!("p{}", page.number),
                },
            ));
        }
        if columns[i] < 2 {
            made.extend(shots);
            made.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
        } else {
            made.extend(shots);
        }

        let mut page_headings = 0usize;
        let mut page_tables = 0usize;
        for (_, b) in &made {
            match b {
                Block::Heading { .. } => page_headings += 1,
                Block::Table(_) => page_tables += 1,
                _ => {}
            }
        }
        headings += page_headings;
        tables += page_tables;
        if needs_ocr {
            blocks.push(Block::Para("<!-- sem texto: precisa de OCR -->".into()));
        }
        blocks.extend(made.into_iter().map(|(_, b)| b));
        reports.push(PageReport {
            page: page.number,
            chars,
            columns: columns[i],
            headings: page_headings,
            tables: page_tables,
            needs_ocr,
        });
    }

    let markdown = render(&blocks);
    let output = if opts.save {
        if let Some(parent) = out_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&out_path, markdown.as_bytes())?;
        Some(out_path.to_string_lossy().to_string())
    } else {
        None
    };
    super::report(
        progress,
        ID,
        "done",
        pages.len() as u64,
        Some(pages.len() as u64),
        None,
    );
    Ok(MdResult {
        needs_ocr: reports.iter().filter(|r| r.needs_ocr).count(),
        words: markdown.split_whitespace().count(),
        bytes: markdown.len() as u64,
        images_dir: (images > 0).then(|| img_dir.to_string_lossy().to_string()),
        images,
        pages: reports,
        headings,
        tables,
        markdown,
        output,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

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

    /// Escreve uma palavra a partir de x, com passo de meia largura.
    fn word(text: &str, x: f32, y: f32, size: f32) -> Vec<TextChar> {
        text.chars()
            .enumerate()
            .map(|(i, c)| ch(c, x + i as f32 * size * 0.5, y, size))
            .collect()
    }

    fn shape(body: f32) -> Shape {
        Shape {
            body,
            lead: body * 1.2,
            tables: true,
            code: true,
        }
    }

    #[test]
    fn linha_junta_caracteres_e_corta_celula_no_vao_grande() {
        let mut cs = word("Nome", 50.0, 700.0, 10.0);
        cs.extend(word("Valor", 200.0, 700.0, 10.0));
        cs.extend(word("Fim", 50.0, 680.0, 10.0));
        let lines = lines_from_chars(&cs, 1);
        assert_eq!(lines.len(), 2, "duas alturas, duas linhas");
        assert_eq!(lines[0].cells.len(), 2, "vão grande virou duas células");
        assert_eq!(lines[0].text(), "Nome Valor");
        assert_eq!(lines[1].text(), "Fim");
        assert!(lines[0].top > lines[1].top, "ordem de cima para baixo");
    }

    #[test]
    fn duas_colunas_saem_na_ordem_de_leitura() {
        // Coluna esquerda em x=50, direita em x=320, mesmas alturas.
        let mut lines = Vec::new();
        for i in 0..6 {
            let y = 700.0 - i as f32 * 14.0;
            lines.push(Line::simple(&format!("E{}", i), 50.0, 240.0, y, 10.0));
            lines.push(Line::simple(&format!("D{}", i), 320.0, 520.0, y, 10.0));
        }
        let cols = detect_columns(&lines, 595.0);
        assert_eq!(cols.len(), 2, "achou a medianiz: {:?}", cols);
        let ordered = order_lines(lines, &cols);
        let seq: Vec<String> = ordered.iter().map(|l| l.text()).collect();
        assert_eq!(
            seq,
            vec!["E0", "E1", "E2", "E3", "E4", "E5", "D0", "D1", "D2", "D3", "D4", "D5"]
        );
    }

    #[test]
    fn titulo_largo_separa_os_blocos_de_coluna() {
        let mut lines = Vec::new();
        for i in 0..5 {
            let y = 700.0 - i as f32 * 14.0;
            lines.push(Line::simple(&format!("E{}", i), 50.0, 240.0, y, 10.0));
            lines.push(Line::simple(&format!("D{}", i), 320.0, 520.0, y, 10.0));
        }
        lines.push(Line::simple("TITULO LARGO", 50.0, 520.0, 620.0, 10.0));
        for i in 0..4 {
            let y = 600.0 - i as f32 * 14.0;
            lines.push(Line::simple(&format!("e{}", i), 50.0, 240.0, y, 10.0));
            lines.push(Line::simple(&format!("d{}", i), 320.0, 520.0, y, 10.0));
        }
        let cols = detect_columns(&lines, 595.0);
        assert_eq!(cols.len(), 2);
        let seq: Vec<String> = order_lines(lines, &cols).iter().map(|l| l.text()).collect();
        let title = seq.iter().position(|s| s == "TITULO LARGO").unwrap();
        assert_eq!(
            &seq[..title],
            &["E0", "E1", "E2", "E3", "E4", "D0", "D1", "D2", "D3", "D4"]
        );
        assert_eq!(
            &seq[title + 1..],
            &["e0", "e1", "e2", "e3", "d0", "d1", "d2", "d3"]
        );
    }

    #[test]
    fn uma_coluna_nao_e_dividida_por_engano() {
        let lines: Vec<Line> = (0..12)
            .map(|i| {
                Line::simple(
                    "linha de texto corrido",
                    50.0,
                    540.0,
                    700.0 - i as f32 * 14.0,
                    10.0,
                )
            })
            .collect();
        assert_eq!(detect_columns(&lines, 595.0).len(), 1);
    }

    #[test]
    fn hifenizacao_desfeita_e_paragrafo_remontado() {
        let joined = join_lines(&[
            "o documento fica muito mais legí-".into(),
            "vel quando o texto é remon-".into(),
            "tado".into(),
        ]);
        assert_eq!(
            joined,
            "o documento fica muito mais legível quando o texto é remontado"
        );
        // Hífen antes de maiúscula é composto e fica.
        assert_eq!(
            join_lines(&["Rio-".into(), "Niterói".into()]),
            "Rio-Niterói"
        );
    }

    #[test]
    fn paragrafo_quebra_no_salto_de_linha_base() {
        let mut a = Line::simple(
            "primeira parte do texto que segue",
            50.0,
            540.0,
            700.0,
            10.0,
        );
        let b = Line::simple("na linha seguinte sem parar", 50.0, 540.0, 688.0, 10.0);
        let mut c = Line::simple("agora um parágrafo novo", 50.0, 540.0, 650.0, 10.0);
        for l in [&mut a, &mut c] {
            l.col0 = 50.0;
            l.col1 = 540.0;
        }
        let mut b2 = b.clone();
        b2.col0 = 50.0;
        b2.col1 = 540.0;
        let out = paragraphs(&[a, b2, c], &shape(10.0));
        assert_eq!(out.len(), 2, "{:?}", out);
        assert_eq!(
            out[0].1,
            Block::Para("primeira parte do texto que segue na linha seguinte sem parar".into())
        );
    }

    #[test]
    fn titulo_sai_do_tamanho_da_fonte() {
        let body = 10.0;
        assert_eq!(
            heading_level(&Line::simple("Capítulo", 50.0, 200.0, 700.0, 20.0), body),
            Some(1)
        );
        assert_eq!(
            heading_level(&Line::simple("Seção", 50.0, 200.0, 700.0, 14.0), body),
            Some(2)
        );
        assert_eq!(
            heading_level(&Line::simple("Item", 50.0, 200.0, 700.0, 11.5), body),
            Some(3)
        );
        assert_eq!(
            heading_level(
                &Line::simple("texto normal", 50.0, 200.0, 700.0, 10.0),
                body
            ),
            None
        );
        let mut bold = Line::simple("Resumo", 50.0, 200.0, 700.0, 10.0);
        bold.bold = true;
        assert_eq!(heading_level(&bold, body), Some(3));
    }

    #[test]
    fn marcador_de_lista_reconhecido() {
        assert_eq!(
            split_marker("• um item"),
            Some(("•".into(), false, "um item".into()))
        );
        assert_eq!(
            split_marker("1. primeiro"),
            Some(("1".into(), true, "primeiro".into()))
        );
        assert_eq!(
            split_marker("a) letra"),
            Some(("a".into(), false, "letra".into()))
        );
        assert_eq!(
            split_marker("- traço"),
            Some(("-".into(), false, "traço".into()))
        );
        assert_eq!(split_marker("2020 foi assim"), None);
        assert_eq!(split_marker("texto comum"), None);
    }

    #[test]
    fn lista_vira_markdown_com_nivel() {
        let mut lines = vec![
            Line::simple("• primeiro item", 50.0, 200.0, 700.0, 10.0),
            Line::simple("que continua aqui", 66.0, 200.0, 688.0, 10.0),
            Line::simple("• segundo item", 50.0, 200.0, 676.0, 10.0),
            Line::simple("• aninhado", 66.0, 200.0, 664.0, 10.0),
        ];
        for l in &mut lines {
            l.col0 = 50.0;
            l.col1 = 200.0;
        }
        let blocks: Vec<Block> = assemble(&lines, &shape(10.0))
            .into_iter()
            .map(|(_, b)| b)
            .collect();
        assert_eq!(blocks.len(), 1, "{:?}", blocks);
        let md = render(&blocks);
        assert_eq!(
            md.trim(),
            "- primeiro item que continua aqui\n- segundo item\n  - aninhado"
        );
    }

    #[test]
    fn tabela_alinhada_vira_tabela_e_torta_vira_texto() {
        let row = |a: &str, b: &str, c: &str, y: f32| Line {
            cells: vec![
                Cell {
                    text: a.into(),
                    x0: 50.0,
                    x1: 100.0,
                },
                Cell {
                    text: b.into(),
                    x0: 200.0,
                    x1: 250.0,
                },
                Cell {
                    text: c.into(),
                    x0: 350.0,
                    x1: 400.0,
                },
            ],
            x0: 50.0,
            x1: 400.0,
            top: y,
            bottom: y - 10.0,
            size: 10.0,
            mono: false,
            bold: false,
            col0: 50.0,
            col1: 400.0,
            page: 1,
        };
        let grid = try_table(&[
            row("Nome", "Preço", "Estoque", 700.0),
            row("Cadeira", "199", "4", 686.0),
            row("Mesa", "499", "2", 672.0),
        ])
        .expect("colunas alinhadas viram tabela");
        assert_eq!(grid.len(), 3);
        assert_eq!(grid[1], vec!["Cadeira", "199", "4"]);
        let md = render(&[Block::Table(grid)]);
        assert!(md.contains("| Nome | Preço | Estoque |"), "{}", md);
        assert!(md.contains("| --- | --- | --- |"), "{}", md);

        // Uma linha com célula fora de qualquer coluna derruba a tabela.
        let mut torta = row("Nome", "Preço", "Estoque", 700.0);
        torta.cells[1].x0 = 127.0;
        assert!(try_table(&[torta, row("Mesa", "499", "2", 686.0)]).is_none());
    }

    #[test]
    fn cabecalho_e_rodape_repetidos_somem() {
        let mut pages: Vec<Vec<Line>> = (0..4)
            .map(|i| {
                vec![
                    Line::simple("Relatório anual", 50.0, 200.0, 780.0, 9.0),
                    Line::simple("conteúdo da página", 50.0, 400.0, 500.0, 10.0),
                    Line::simple(&format!("Página {} de 4", i + 1), 50.0, 120.0, 40.0, 9.0),
                ]
            })
            .collect();
        strip_running(&mut pages, &[792.0; 4]);
        for p in &pages {
            assert_eq!(
                p.len(),
                1,
                "sobrou {:?}",
                p.iter().map(|l| l.text()).collect::<Vec<_>>()
            );
            assert_eq!(p[0].text(), "conteúdo da página");
        }
    }

    #[test]
    fn numero_de_pagina_solto_some() {
        let mut pages: Vec<Vec<Line>> = (0..3)
            .map(|i| {
                vec![
                    Line::simple("miolo", 50.0, 400.0, 500.0, 10.0),
                    Line::simple(&format!("{}", i + 1), 300.0, 310.0, 40.0, 9.0),
                ]
            })
            .collect();
        strip_running(&mut pages, &[792.0; 3]);
        assert!(pages.iter().all(|p| p.len() == 1));
    }

    #[test]
    fn corpo_do_texto_e_a_moda_ponderada() {
        let mut lines = vec![Line::simple("Um título grande", 50.0, 300.0, 700.0, 24.0)];
        for i in 0..10 {
            lines.push(Line::simple(
                "linha de corpo com bastante texto mesmo",
                50.0,
                500.0,
                600.0 - i as f32 * 12.0,
                10.5,
            ));
        }
        assert_eq!(body_size(&[lines]), 10.5);
    }

    #[test]
    fn fonte_mono_vira_bloco_de_codigo() {
        let mut a = Line::simple("fn main() {", 50.0, 200.0, 700.0, 10.0);
        let mut b = Line::simple("println!(\"oi\");", 62.0, 220.0, 688.0, 10.0);
        let mut c = Line::simple("}", 50.0, 60.0, 676.0, 10.0);
        for l in [&mut a, &mut b, &mut c] {
            l.mono = true;
        }
        let blocks: Vec<Block> = assemble(&[a, b, c], &shape(10.0))
            .into_iter()
            .map(|(_, b)| b)
            .collect();
        let md = render(&blocks);
        assert_eq!(md.trim(), "```\nfn main() {\n  println!(\"oi\");\n}\n```");
    }

    #[test]
    fn pagina_inteira_vira_markdown_na_ordem_certa() {
        let mut lines = vec![Line::simple("Guia rápido", 50.0, 200.0, 720.0, 20.0)];
        let mut sub = Line::simple("Como começar", 50.0, 180.0, 690.0, 13.5);
        sub.bold = true;
        lines.push(sub);
        lines.push(Line::simple(
            "Este parágrafo continua na linha de baixo sem",
            50.0,
            540.0,
            660.0,
            10.0,
        ));
        lines.push(Line::simple(
            "nenhuma quebra de verdade.",
            50.0,
            300.0,
            648.0,
            10.0,
        ));
        lines.push(Line::simple("1. abra o app", 50.0, 200.0, 610.0, 10.0));
        lines.push(Line::simple("2. escolha o PDF", 50.0, 200.0, 598.0, 10.0));
        for l in &mut lines {
            l.col0 = 50.0;
            l.col1 = 540.0;
        }
        let md = render(
            &assemble(&lines, &shape(10.0))
                .into_iter()
                .map(|(_, b)| b)
                .collect::<Vec<_>>(),
        );
        assert_eq!(
            md,
            "# Guia rápido\n\n## Como começar\n\nEste parágrafo continua na linha de baixo sem nenhuma quebra de verdade.\n\n1. abra o app\n2. escolha o PDF\n"
        );
    }

    /// Ponta a ponta com um PDF montado à mão (mesma técnica do teste de
    /// tarja em `pdf.rs`): três páginas com cabeçalho e rodapé corridos,
    /// título, parágrafo hifenizado, lista, duas colunas e um bloco em
    /// Courier. `cargo test -p omniget-core --lib -- --ignored live_pdf_markdown`
    #[test]
    #[ignore]
    fn live_pdf_markdown_de_ponta_a_ponta() {
        fn put(out: &mut String, font: &str, size: f32, x: f32, y: f32, text: &str) {
            out.push_str(&format!(
                "BT /{} {} Tf {} {} Td ({}) Tj ET\n",
                font, size, x, y, text
            ));
        }

        let mut pages: Vec<String> = Vec::new();
        for n in 1..=3 {
            let mut c = String::new();
            put(&mut c, "F1", 9.0, 50.0, 760.0, "Manual do OmniGet");
            put(&mut c, "F1", 9.0, 50.0, 40.0, &format!("Pagina {} de 3", n));
            match n {
                1 => {
                    put(&mut c, "F1", 22.0, 50.0, 720.0, "Guia rapido");
                    put(&mut c, "F1", 13.5, 50.0, 690.0, "Como comecar");
                    put(
                        &mut c,
                        "F1",
                        10.0,
                        50.0,
                        660.0,
                        "Este paragrafo continua na linha de bai-",
                    );
                    put(
                        &mut c,
                        "F1",
                        10.0,
                        50.0,
                        648.0,
                        "xo sem nenhuma quebra de verdade.",
                    );
                    put(&mut c, "F1", 10.0, 50.0, 610.0, "1. abra o app");
                    put(&mut c, "F1", 10.0, 50.0, 598.0, "2. escolha o PDF");
                }
                2 => {
                    for i in 1..=6 {
                        let y = 700.0 - (i - 1) as f32 * 14.0;
                        put(
                            &mut c,
                            "F1",
                            10.0,
                            50.0,
                            y,
                            &format!("esquerda linha {}", i),
                        );
                        put(
                            &mut c,
                            "F1",
                            10.0,
                            320.0,
                            y,
                            &format!("direita linha {}", i),
                        );
                    }
                }
                _ => {
                    put(&mut c, "F2", 10.0, 50.0, 700.0, "fn main\\(\\) {");
                    put(&mut c, "F2", 10.0, 62.0, 688.0, "println!");
                    put(&mut c, "F2", 10.0, 50.0, 676.0, "}");
                }
            }
            pages.push(c);
        }

        let mut body = String::from("%PDF-1.4\n");
        body.push_str("1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n");
        let kids: Vec<String> = (0..pages.len())
            .map(|i| format!("{} 0 R", 5 + i * 2))
            .collect();
        body.push_str(&format!(
            "2 0 obj\n<< /Type /Pages /Kids [{}] /Count {} >>\nendobj\n",
            kids.join(" "),
            pages.len()
        ));
        body.push_str("3 0 obj\n<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>\nendobj\n");
        body.push_str("4 0 obj\n<< /Type /Font /Subtype /Type1 /BaseFont /Courier >>\nendobj\n");
        for (i, c) in pages.iter().enumerate() {
            let page_obj = 5 + i * 2;
            body.push_str(&format!(
                "{} 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 595 792] /Resources << /Font << /F1 3 0 R /F2 4 0 R >> >> /Contents {} 0 R >>\nendobj\n",
                page_obj,
                page_obj + 1
            ));
            body.push_str(&format!(
                "{} 0 obj\n<< /Length {} >>\nstream\n{}endstream\nendobj\n",
                page_obj + 1,
                c.len(),
                c
            ));
        }
        let (bytes, _) = super::super::pdf_repair::rebuild_xref(body.as_bytes()).unwrap();

        let dir = std::env::temp_dir().join("omniget-pdfmd-live");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("guia.pdf");
        std::fs::write(&path, &bytes).unwrap();

        let opts = Options {
            input: path.to_string_lossy().to_string(),
            save: true,
            page_marks: "none".into(),
            ..Default::default()
        };
        let out = run(&opts, &super::super::noop_progress()).expect("converteu");
        eprintln!("--- markdown ---\n{}\n----------------", out.markdown);
        let md = &out.markdown;
        assert_eq!(out.pages.len(), 3);
        assert!(md.contains("# Guia rapido"), "sem titulo H1");
        assert!(md.contains("## Como comecar"), "sem titulo H2");
        assert!(
            md.contains("linha de baixo sem nenhuma quebra"),
            "hifenizacao nao foi desfeita"
        );
        assert!(
            md.contains("1. abra o app") && md.contains("2. escolha o PDF"),
            "lista"
        );
        assert!(!md.contains("Manual do OmniGet"), "cabecalho corrido ficou");
        assert!(!md.contains("Pagina 1 de 3"), "rodape corrido ficou");
        assert_eq!(out.pages[1].columns, 2, "nao viu as duas colunas");
        let esq = md.find("esquerda linha 6").expect("coluna esquerda");
        let dir_pos = md.find("direita linha 1").expect("coluna direita");
        assert!(esq < dir_pos, "leu as colunas intercaladas");
        assert!(md.contains("```"), "bloco de codigo nao saiu:\n{}", md);
        assert_eq!(out.needs_ocr, 0);
        assert!(out.output.is_some(), "nao gravou o .md");
        let _ = std::fs::remove_dir_all(&dir);
    }
}

use serde::Serialize;

use super::engine::TermEngine;

#[derive(Debug, Clone, Default, Serialize)]
pub struct ScreenSnapshot {
    pub text: String,
    pub cursor_row: i64,
    pub changed_ms: i64,
    pub alt_screen: bool,
}

pub fn snapshot(engine: &dyn TermEngine, last_lines: usize, changed_ms: i64) -> ScreenSnapshot {
    let (_, rows) = engine.size();
    let rows = rows as i32;
    let screen = engine.lines_text(0, rows);
    let cursor_line = engine.cursor().line;
    let last_text = screen
        .iter()
        .rposition(|l| !l.trim().is_empty())
        .map(|i| i as i32)
        .unwrap_or(0);
    let bottom = last_text.max(cursor_line).min(rows - 1);
    let n = if last_lines == 0 {
        rows
    } else {
        last_lines.min(100_000) as i32
    };
    let from = bottom - n + 1;
    let lines = if from >= 0 {
        screen[from as usize..=bottom as usize].to_vec()
    } else {
        let mut v = engine.lines_text(from, 0);
        v.extend_from_slice(&screen[..=bottom as usize]);
        v
    };
    let first = bottom - lines.len() as i32 + 1;
    let cursor_row = if cursor_line >= first && cursor_line <= bottom {
        (cursor_line - first) as i64
    } else {
        -1
    };
    ScreenSnapshot {
        text: lines.join("\n"),
        cursor_row,
        changed_ms,
        alt_screen: engine.modes().alt_screen,
    }
}

pub fn tail(engine: &dyn TermEngine, lines: usize) -> String {
    let (_, rows) = engine.size();
    let screen = engine.lines_text(0, rows as i32);
    let keep: Vec<&String> = screen
        .iter()
        .rev()
        .filter(|l| !l.trim().is_empty())
        .take(lines)
        .collect();
    keep.into_iter()
        .rev()
        .cloned()
        .collect::<Vec<_>>()
        .join("\n")
}

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use alacritty_terminal::event::{Event, EventListener, WindowSize};
use alacritty_terminal::grid::{Dimensions, Scroll};
use alacritty_terminal::index::{Boundary, Column, Direction, Line, Point, Side};
use alacritty_terminal::selection::{Selection, SelectionType};
use alacritty_terminal::term::cell::{Cell, Flags};
use alacritty_terminal::term::search::{Match, RegexSearch};
use alacritty_terminal::term::{Config, TermMode};
use alacritty_terminal::vte::ansi::{
    Color, CursorShape as ACursorShape, NamedColor, Processor, Rgb, StdSyncHandler,
};
use alacritty_terminal::Term;

use super::{
    color, style_flags as sf, CellView, CursorShape, CursorView, EngineEvent, Modes, ScrollBy,
    SelectionKind, Span, StyleKey, TermEngine,
};

#[derive(Clone)]
struct Proxy(Arc<Mutex<Vec<Event>>>);

impl EventListener for Proxy {
    fn send_event(&self, event: Event) {
        if matches!(
            event,
            Event::Wakeup | Event::MouseCursorDirty | Event::CursorBlinkingChange
        ) {
            return;
        }
        self.0.lock().unwrap_or_else(|e| e.into_inner()).push(event);
    }
}

struct Dims {
    cols: usize,
    rows: usize,
}

impl Dimensions for Dims {
    fn total_lines(&self) -> usize {
        self.rows
    }
    fn screen_lines(&self) -> usize {
        self.rows
    }
    fn columns(&self) -> usize {
        self.cols
    }
}

const LINKS_MAX: usize = 4096;

pub struct AlacrittyEngine {
    term: Term<Proxy>,
    parser: Processor<StdSyncHandler>,
    events: Arc<Mutex<Vec<Event>>>,
    dirty: bool,
    title: Option<String>,
    links: Mutex<HashMap<u32, String>>,
    search: Option<(String, RegexSearch)>,
    last_match: Option<Match>,
}

fn default_rgb(index: usize) -> Rgb {
    const ANSI: [(u8, u8, u8); 16] = [
        (0x1e, 0x1e, 0x1e),
        (0xcd, 0x31, 0x31),
        (0x0d, 0xbc, 0x79),
        (0xe5, 0xe5, 0x10),
        (0x24, 0x72, 0xc8),
        (0xbc, 0x3f, 0xbc),
        (0x11, 0xa8, 0xcd),
        (0xe5, 0xe5, 0xe5),
        (0x66, 0x66, 0x66),
        (0xf1, 0x4c, 0x4c),
        (0x23, 0xd1, 0x8b),
        (0xf5, 0xf5, 0x43),
        (0x3b, 0x8e, 0xea),
        (0xd6, 0x70, 0xd6),
        (0x29, 0xb8, 0xdb),
        (0xff, 0xff, 0xff),
    ];
    let (r, g, b) = match index {
        0..=15 => ANSI[index],
        16..=231 => {
            let i = index - 16;
            let step = |v: usize| if v == 0 { 0 } else { (55 + v * 40) as u8 };
            (step(i / 36), step((i / 6) % 6), step(i % 6))
        }
        232..=255 => {
            let v = (8 + (index - 232) * 10) as u8;
            (v, v, v)
        }
        257 => (0x1e, 0x1e, 0x1e),
        _ => (0xd8, 0xd8, 0xd8),
    };
    Rgb { r, g, b }
}

fn fnv(s: &str) -> u32 {
    let mut h: u32 = 0x811c_9dc5;
    for b in s.bytes() {
        h ^= b as u32;
        h = h.wrapping_mul(0x0100_0193);
    }
    h.max(1)
}

impl AlacrittyEngine {
    pub fn new(cols: u16, rows: u16, scrollback: usize) -> Self {
        let events = Arc::new(Mutex::new(Vec::new()));
        let config = Config {
            scrolling_history: scrollback,
            ..Config::default()
        };
        let dims = Dims {
            cols: cols.max(2) as usize,
            rows: rows.max(1) as usize,
        };
        Self {
            term: Term::new(config, &dims, Proxy(events.clone())),
            parser: Processor::new(),
            events,
            dirty: true,
            title: None,
            links: Mutex::new(HashMap::new()),
            search: None,
            last_match: None,
        }
    }

    fn color(&self, c: Color) -> u32 {
        let colors = self.term.colors();
        match c {
            Color::Spec(rgb) => color::rgb(rgb.r, rgb.g, rgb.b),
            Color::Indexed(i) => match colors[i as usize] {
                Some(rgb) => color::rgb(rgb.r, rgb.g, rgb.b),
                None => color::indexed(i),
            },
            Color::Named(n) => {
                let idx = n as usize;
                if let Some(rgb) = colors[idx] {
                    return color::rgb(rgb.r, rgb.g, rgb.b);
                }
                match n {
                    NamedColor::Foreground
                    | NamedColor::Background
                    | NamedColor::Cursor
                    | NamedColor::BrightForeground
                    | NamedColor::DimForeground => color::DEFAULT,
                    _ if idx < 16 => color::indexed(idx as u8),
                    _ if (259..267).contains(&idx) => color::indexed((idx - 259) as u8),
                    _ => color::DEFAULT,
                }
            }
        }
    }

    fn style_of(&self, cell: &Cell) -> StyleKey {
        let f = cell.flags;
        let mut flags = 0u16;
        if f.contains(Flags::BOLD) {
            flags |= sf::BOLD;
        }
        if f.contains(Flags::DIM) {
            flags |= sf::DIM;
        }
        if f.contains(Flags::ITALIC) {
            flags |= sf::ITALIC;
        }
        if f.contains(Flags::DOUBLE_UNDERLINE) {
            flags |= sf::UL_DOUBLE;
        } else if f.contains(Flags::UNDERCURL) {
            flags |= sf::UL_CURLY;
        } else if f.contains(Flags::DOTTED_UNDERLINE) {
            flags |= sf::UL_DOTTED;
        } else if f.contains(Flags::DASHED_UNDERLINE) {
            flags |= sf::UL_DASHED;
        } else if f.contains(Flags::UNDERLINE) {
            flags |= sf::UL_SINGLE;
        }
        if f.contains(Flags::STRIKEOUT) {
            flags |= sf::STRIKE;
        }
        if f.contains(Flags::HIDDEN) {
            flags |= sf::HIDDEN;
        }
        if f.contains(Flags::WIDE_CHAR) {
            flags |= sf::WIDE;
        }
        let mut fg = self.color(cell.fg);
        let mut bg = self.color(cell.bg);
        if matches!(cell.fg, Color::Named(n) if (259..267).contains(&(n as usize))) {
            flags |= sf::DIM;
        }
        if f.contains(Flags::INVERSE) {
            let nf = if bg == color::DEFAULT {
                color::SWAPPED_DEFAULT
            } else {
                bg
            };
            let nb = if fg == color::DEFAULT {
                color::SWAPPED_DEFAULT
            } else {
                fg
            };
            fg = nf;
            bg = nb;
        }
        let ul = cell
            .underline_color()
            .map(|c| self.color(c))
            .unwrap_or(color::DEFAULT);
        let link = match cell.hyperlink() {
            Some(h) => {
                let id = fnv(&format!("{}\u{0}{}", h.id(), h.uri()));
                let mut links = self.links.lock().unwrap_or_else(|e| e.into_inner());
                if links.len() >= LINKS_MAX && !links.contains_key(&id) {
                    links.clear();
                }
                links.entry(id).or_insert_with(|| h.uri().to_string());
                flags |= sf::LINK;
                id
            }
            None => 0,
        };
        StyleKey {
            fg,
            bg,
            ul,
            flags,
            link,
        }
    }

    fn viewport_point(&self, col: u16, row: u16) -> Point {
        let offset = self.term.grid().display_offset() as i32;
        let cols = self.term.columns();
        let rows = self.term.screen_lines() as i32;
        let line = (row as i32).min(rows - 1) - offset;
        Point::new(
            Line(line),
            Column((col as usize).min(cols.saturating_sub(1))),
        )
    }

    fn to_viewport(&self, p: Point) -> (u16, u16) {
        let offset = self.term.grid().display_offset() as i32;
        let rows = self.term.screen_lines() as i32;
        let y = (p.line.0 + offset).clamp(0, rows - 1);
        (p.column.0 as u16, y as u16)
    }

    fn in_view(&self, start: Point, end: Point) -> bool {
        let offset = self.term.grid().display_offset() as i32;
        let rows = self.term.screen_lines() as i32;
        let top = -offset;
        let bottom = top + rows - 1;
        !(end.line.0 < top || start.line.0 > bottom)
    }

    fn line_text(&self, line: Line) -> String {
        let row = &self.term.grid()[line];
        let mut s = String::new();
        for col in 0..self.term.columns() {
            let cell = &row[Column(col)];
            if cell
                .flags
                .intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER)
            {
                continue;
            }
            s.push(cell.c);
            if let Some(zw) = cell.zerowidth() {
                s.extend(zw.iter());
            }
        }
        s.trim_end().to_string()
    }
}

impl TermEngine for AlacrittyEngine {
    fn advance(&mut self, bytes: &[u8]) {
        self.parser.advance(&mut self.term, bytes);
        self.dirty = true;
    }

    fn resize(&mut self, cols: u16, rows: u16) {
        self.term.resize(Dims {
            cols: cols.max(2) as usize,
            rows: rows.max(1) as usize,
        });
        self.last_match = None;
        self.dirty = true;
    }

    fn size(&self) -> (u16, u16) {
        (self.term.columns() as u16, self.term.screen_lines() as u16)
    }

    fn take_events(&mut self) -> Vec<EngineEvent> {
        let raw: Vec<Event> =
            std::mem::take(&mut *self.events.lock().unwrap_or_else(|e| e.into_inner()));
        let mut out = Vec::with_capacity(raw.len());
        for ev in raw {
            match ev {
                Event::PtyWrite(s) => out.push(EngineEvent::PtyWrite(s.into_bytes())),
                Event::ColorRequest(idx, fmt) => {
                    let rgb = self.term.colors()[idx].unwrap_or_else(|| default_rgb(idx));
                    out.push(EngineEvent::PtyWrite(fmt(rgb).into_bytes()));
                }
                Event::TextAreaSizeRequest(fmt) => {
                    let ws = WindowSize {
                        num_lines: self.term.screen_lines() as u16,
                        num_cols: self.term.columns() as u16,
                        cell_width: 8,
                        cell_height: 16,
                    };
                    out.push(EngineEvent::PtyWrite(fmt(ws).into_bytes()));
                }
                Event::Title(t) => {
                    self.title = Some(t.clone());
                    out.push(EngineEvent::Title(Some(t)));
                }
                Event::ResetTitle => {
                    self.title = None;
                    out.push(EngineEvent::Title(None));
                }
                Event::Bell => out.push(EngineEvent::Bell),
                Event::ClipboardStore(_, s) => out.push(EngineEvent::ClipboardStore(s)),
                _ => {}
            }
        }
        out
    }

    fn take_dirty(&mut self) -> bool {
        std::mem::take(&mut self.dirty)
    }

    fn row(&self, y: u16, out: &mut Vec<CellView>) {
        out.clear();
        let rows = self.term.screen_lines();
        if y as usize >= rows {
            return;
        }
        let offset = self.term.grid().display_offset() as i32;
        let line = Line(y as i32 - offset);
        let row = &self.term.grid()[line];
        for col in 0..self.term.columns() {
            let cell = &row[Column(col)];
            let spacer = cell
                .flags
                .intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER);
            let style = self.style_of(cell);
            let (cp, cluster) = if spacer {
                (0, None)
            } else {
                match cell.zerowidth() {
                    Some(zw) if !zw.is_empty() => {
                        let mut s = String::new();
                        s.push(cell.c);
                        s.extend(zw.iter());
                        (cell.c as u32, Some(s))
                    }
                    _ if cell.c == ' ' || cell.c == '\0' => (0, None),
                    _ => (cell.c as u32, None),
                }
            };
            out.push(CellView { cp, cluster, style });
        }
    }

    fn cursor(&self) -> CursorView {
        let content = self.term.renderable_content();
        let point = content.cursor.point;
        let offset = content.display_offset as i32;
        let y = point.line.0 + offset;
        let rows = self.term.screen_lines() as i32;
        let shape = match content.cursor.shape {
            ACursorShape::Block => CursorShape::Block,
            ACursorShape::Underline => CursorShape::Underline,
            ACursorShape::Beam => CursorShape::Beam,
            ACursorShape::HollowBlock => CursorShape::Hollow,
            ACursorShape::Hidden => CursorShape::Hidden,
        };
        CursorView {
            x: point.column.0 as u16,
            y: y.clamp(0, rows - 1) as u16,
            shape,
            visible: shape != CursorShape::Hidden && (0..rows).contains(&y),
            blink: self.term.cursor_style().blinking,
            line: self.term.grid().cursor.point.line.0,
        }
    }

    fn modes(&self) -> Modes {
        let m = *self.term.mode();
        Modes {
            alt_screen: m.contains(TermMode::ALT_SCREEN),
            bracketed_paste: m.contains(TermMode::BRACKETED_PASTE),
            mouse_click: m.contains(TermMode::MOUSE_REPORT_CLICK),
            mouse_drag: m.contains(TermMode::MOUSE_DRAG),
            mouse_motion: m.contains(TermMode::MOUSE_MOTION),
            sgr_mouse: m.contains(TermMode::SGR_MOUSE),
            utf8_mouse: m.contains(TermMode::UTF8_MOUSE),
            app_cursor: m.contains(TermMode::APP_CURSOR),
            app_keypad: m.contains(TermMode::APP_KEYPAD),
            focus_reporting: m.contains(TermMode::FOCUS_IN_OUT),
            sync_pending: self.parser.sync_timeout().sync_timeout().is_some(),
            alternate_scroll: m.contains(TermMode::ALTERNATE_SCROLL),
            cursor_visible: m.contains(TermMode::SHOW_CURSOR),
            kitty_keyboard: m.intersects(TermMode::KITTY_KEYBOARD_PROTOCOL),
        }
    }

    fn title(&self) -> Option<String> {
        self.title.clone()
    }

    fn display_offset(&self) -> usize {
        self.term.grid().display_offset()
    }

    fn history_len(&self) -> usize {
        self.term.grid().history_size()
    }

    fn scroll_display(&mut self, by: ScrollBy) {
        let s = match by {
            ScrollBy::Lines(n) => Scroll::Delta(n),
            ScrollBy::Pages(n) => Scroll::Delta(n * self.term.screen_lines() as i32),
            ScrollBy::Top => Scroll::Top,
            ScrollBy::Bottom => Scroll::Bottom,
        };
        self.term.scroll_display(s);
        self.dirty = true;
    }

    fn lines_text(&self, from: i32, to: i32) -> Vec<String> {
        let top = self.term.topmost_line().0;
        let bottom = self.term.bottommost_line().0;
        let from = from.max(top);
        let to = to.min(bottom + 1);
        (from..to).map(|l| self.line_text(Line(l))).collect()
    }

    fn selection_start(&mut self, kind: SelectionKind, col: u16, row: u16, right: bool) {
        let ty = match kind {
            SelectionKind::Simple => SelectionType::Simple,
            SelectionKind::Block => SelectionType::Block,
            SelectionKind::Semantic => SelectionType::Semantic,
            SelectionKind::Lines => SelectionType::Lines,
        };
        let p = self.viewport_point(col, row);
        let side = if right { Side::Right } else { Side::Left };
        self.term.selection = Some(Selection::new(ty, p, side));
        self.dirty = true;
    }

    fn selection_update(&mut self, col: u16, row: u16, right: bool) {
        let p = self.viewport_point(col, row);
        let side = if right { Side::Right } else { Side::Left };
        if let Some(sel) = self.term.selection.as_mut() {
            sel.update(p, side);
            self.dirty = true;
        }
    }

    fn selection_clear(&mut self) {
        if self.term.selection.take().is_some() {
            self.dirty = true;
        }
    }

    fn selection_all(&mut self) {
        let top = Point::new(self.term.topmost_line(), Column(0));
        let bottom = Point::new(self.term.bottommost_line(), self.term.last_column());
        let mut sel = Selection::new(SelectionType::Simple, top, Side::Left);
        sel.update(bottom, Side::Right);
        self.term.selection = Some(sel);
        self.dirty = true;
    }

    fn selection_text(&self) -> Option<String> {
        self.term.selection_to_string().filter(|s| !s.is_empty())
    }

    fn highlights(&self) -> Vec<Span> {
        let mut out = Vec::new();
        if let Some(r) = self
            .term
            .selection
            .as_ref()
            .and_then(|s| s.to_range(&self.term))
        {
            if self.in_view(r.start, r.end) {
                out.push(Span {
                    kind: 0,
                    block: r.is_block,
                    start: self.to_viewport(r.start),
                    end: self.to_viewport(r.end),
                });
            }
        }
        if let Some(m) = &self.last_match {
            if self.in_view(*m.start(), *m.end()) {
                out.push(Span {
                    kind: 1,
                    block: false,
                    start: self.to_viewport(*m.start()),
                    end: self.to_viewport(*m.end()),
                });
            }
        }
        out
    }

    fn search(&mut self, query: &str, backwards: bool) -> Option<Span> {
        if query.is_empty() {
            self.search_clear();
            return None;
        }
        if self.search.as_ref().map(|(q, _)| q.as_str()) != Some(query) {
            let regex = RegexSearch::new(query).ok()?;
            self.search = Some((query.to_string(), regex));
            self.last_match = None;
        }
        let dir = if backwards {
            Direction::Left
        } else {
            Direction::Right
        };
        let origin = match (&self.last_match, backwards) {
            (Some(m), false) => m.end().add(&self.term, Boundary::None, 1),
            (Some(m), true) => m.start().sub(&self.term, Boundary::None, 1),
            (None, false) => Point::new(self.term.topmost_line(), Column(0)),
            (None, true) => Point::new(self.term.bottommost_line(), self.term.last_column()),
        };
        let (_, regex) = self.search.as_mut()?;
        let found = self.term.search_next(regex, origin, dir, Side::Left, None);
        self.dirty = true;
        let m = found?;
        self.term.scroll_to_point(*m.start());
        self.last_match = Some(m.clone());
        Some(Span {
            kind: 1,
            block: false,
            start: self.to_viewport(*m.start()),
            end: self.to_viewport(*m.end()),
        })
    }

    fn search_clear(&mut self) {
        self.search = None;
        if self.last_match.take().is_some() {
            self.dirty = true;
        }
    }

    fn link_uri(&self, link_id: u32) -> Option<String> {
        self.links
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(&link_id)
            .cloned()
    }

    fn sync_deadline(&self) -> Option<std::time::Instant> {
        self.parser.sync_timeout().sync_timeout()
    }

    fn flush_sync(&mut self) {
        self.parser.stop_sync(&mut self.term);
        self.dirty = true;
    }
}

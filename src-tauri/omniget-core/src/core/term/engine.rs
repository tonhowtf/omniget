//! Terminal emulation behind a trait. C1 uses `alacritty_terminal`
//! ([`alacritty::AlacrittyEngine`]); `libghostty-vt` can replace it later
//! behind the same trait, checked against the same differential corpus.
//!
//! The engine owns the grid; the session feeds it every byte the program
//! writes, writes the engine's answers to terminal queries (DA, DSR/CPR,
//! DECRQM, colour queries) back to the PTY, and reads rows for the wire and
//! text for `screen`.

pub mod alacritty;

/// Colour as sent on the wire: `kind << 24 | value` (see `wire`).
pub mod color {
    pub const DEFAULT: u32 = 0;
    pub const SWAPPED_DEFAULT: u32 = 3 << 24;

    pub fn indexed(i: u8) -> u32 {
        (1 << 24) | i as u32
    }

    pub fn rgb(r: u8, g: u8, b: u8) -> u32 {
        (2 << 24) | ((r as u32) << 16) | ((g as u32) << 8) | b as u32
    }
}

/// Style flags as sent on the wire.
pub mod style_flags {
    pub const BOLD: u16 = 1;
    pub const DIM: u16 = 1 << 1;
    pub const ITALIC: u16 = 1 << 2;
    pub const UL_SHIFT: u16 = 3;
    pub const UL_SINGLE: u16 = 1 << UL_SHIFT;
    pub const UL_DOUBLE: u16 = 2 << UL_SHIFT;
    pub const UL_CURLY: u16 = 3 << UL_SHIFT;
    pub const UL_DOTTED: u16 = 4 << UL_SHIFT;
    pub const UL_DASHED: u16 = 5 << UL_SHIFT;
    pub const STRIKE: u16 = 1 << 6;
    pub const HIDDEN: u16 = 1 << 7;
    pub const WIDE: u16 = 1 << 8;
    pub const LINK: u16 = 1 << 9;
}

/// Everything that makes two cells look different, besides their text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct StyleKey {
    pub fg: u32,
    pub bg: u32,
    pub ul: u32,
    pub flags: u16,
    pub link: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CellView {
    /// Unicode scalar; 0 = empty cell or wide-char spacer.
    pub cp: u32,
    /// Multi-codepoint cluster (base + combining marks), when there is one.
    pub cluster: Option<String>,
    pub style: StyleKey,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CursorShape {
    #[default]
    Block,
    Underline,
    Beam,
    Hollow,
    Hidden,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CursorView {
    /// Viewport coordinates.
    pub x: u16,
    pub y: u16,
    pub shape: CursorShape,
    /// In the viewport and not hidden by mode 25.
    pub visible: bool,
    pub blink: bool,
    /// Grid line of the cursor (0 = top of the live screen), for `screen`.
    pub line: i32,
}

/// Terminal modes that matter to input, rendering and the wire `META`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Modes {
    pub alt_screen: bool,
    pub bracketed_paste: bool,
    pub mouse_click: bool,
    pub mouse_drag: bool,
    pub mouse_motion: bool,
    pub sgr_mouse: bool,
    pub utf8_mouse: bool,
    pub app_cursor: bool,
    pub app_keypad: bool,
    pub focus_reporting: bool,
    pub sync_pending: bool,
    pub alternate_scroll: bool,
    pub cursor_visible: bool,
    pub kitty_keyboard: bool,
}

impl Modes {
    pub fn mouse_reporting(&self) -> bool {
        self.mouse_click || self.mouse_drag || self.mouse_motion
    }

    /// `META` bitset.
    pub fn bits(&self) -> u32 {
        let flags = [
            self.alt_screen,
            self.bracketed_paste,
            self.mouse_click,
            self.mouse_drag,
            self.mouse_motion,
            self.sgr_mouse,
            self.app_cursor,
            self.app_keypad,
            self.focus_reporting,
            self.sync_pending,
            self.alternate_scroll,
            self.cursor_visible,
            self.kitty_keyboard,
        ];
        flags
            .iter()
            .enumerate()
            .fold(0u32, |acc, (i, on)| if *on { acc | (1 << i) } else { acc })
    }
}

/// Something the program asked of the terminal.
#[derive(Debug, Clone, PartialEq)]
pub enum EngineEvent {
    /// Bytes to write back to the PTY (query answers).
    PtyWrite(Vec<u8>),
    Title(Option<String>),
    Bell,
    /// OSC 52 copy: the app decides whether to honour it.
    ClipboardStore(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelectionKind {
    Simple,
    Block,
    Semantic,
    Lines,
}

/// A selected or highlighted range in viewport coordinates (inclusive).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Span {
    pub kind: u8,
    pub block: bool,
    pub start: (u16, u16),
    pub end: (u16, u16),
}

#[derive(Debug, Clone, Copy)]
pub enum ScrollBy {
    Lines(i32),
    Pages(i32),
    Top,
    Bottom,
}

pub trait TermEngine: Send {
    fn advance(&mut self, bytes: &[u8]);
    fn resize(&mut self, cols: u16, rows: u16);
    fn size(&self) -> (u16, u16);
    fn take_events(&mut self) -> Vec<EngineEvent>;
    /// Something changed since the last call (content, cursor, modes).
    fn take_dirty(&mut self) -> bool;
    /// One viewport row (0 = top of what is displayed).
    fn row(&self, y: u16, out: &mut Vec<CellView>);
    fn cursor(&self) -> CursorView;
    fn modes(&self) -> Modes;
    fn title(&self) -> Option<String>;
    fn display_offset(&self) -> usize;
    fn history_len(&self) -> usize;
    fn scroll_display(&mut self, by: ScrollBy);
    /// Text of grid lines `from..to` (negative = scrollback, 0 = top of the
    /// live screen), trailing blanks trimmed, wrapped lines kept apart.
    fn lines_text(&self, from: i32, to: i32) -> Vec<String>;
    fn selection_start(&mut self, kind: SelectionKind, col: u16, row: u16, right: bool);
    fn selection_update(&mut self, col: u16, row: u16, right: bool);
    fn selection_clear(&mut self);
    fn selection_all(&mut self);
    fn selection_text(&self) -> Option<String>;
    /// Selection plus search highlight, viewport coords.
    fn highlights(&self) -> Vec<Span>;
    /// Finds the next match from the last one (or the bottom) and scrolls to
    /// it. Returns it in viewport coordinates.
    fn search(&mut self, query: &str, backwards: bool) -> Option<Span>;
    fn search_clear(&mut self);
    fn link_uri(&self, link_id: u32) -> Option<String>;
    /// DEC 2026: when the held update must be flushed if nothing ends it.
    fn sync_deadline(&self) -> Option<std::time::Instant>;
    fn flush_sync(&mut self);
}

/// Default engine for new terminals.
pub fn new_engine(cols: u16, rows: u16, scrollback: usize) -> Box<dyn TermEngine> {
    Box::new(alacritty::AlacrittyEngine::new(cols, rows, scrollback))
}

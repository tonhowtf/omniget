pub mod alacritty;

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
    pub cp: u32,
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
    pub x: u16,
    pub y: u16,
    pub shape: CursorShape,
    pub visible: bool,
    pub blink: bool,
    pub line: i32,
}

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

#[derive(Debug, Clone, PartialEq)]
pub enum EngineEvent {
    PtyWrite(Vec<u8>),
    Title(Option<String>),
    Bell,
    ClipboardStore(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelectionKind {
    Simple,
    Block,
    Semantic,
    Lines,
}

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
    fn take_dirty(&mut self) -> bool;
    fn row(&self, y: u16, out: &mut Vec<CellView>);
    fn cursor(&self) -> CursorView;
    fn modes(&self) -> Modes;
    fn title(&self) -> Option<String>;
    fn display_offset(&self) -> usize;
    fn history_len(&self) -> usize;
    fn scroll_display(&mut self, by: ScrollBy);
    fn lines_text(&self, from: i32, to: i32) -> Vec<String>;
    fn selection_start(&mut self, kind: SelectionKind, col: u16, row: u16, right: bool);
    fn selection_update(&mut self, col: u16, row: u16, right: bool);
    fn selection_clear(&mut self);
    fn selection_all(&mut self);
    fn selection_text(&self) -> Option<String>;
    fn highlights(&self) -> Vec<Span>;
    fn search(&mut self, query: &str, backwards: bool) -> Option<Span>;
    fn search_clear(&mut self);
    fn link_uri(&self, link_id: u32) -> Option<String>;
    fn sync_deadline(&self) -> Option<std::time::Instant>;
    fn flush_sync(&mut self);
}

pub fn new_engine(cols: u16, rows: u16, scrollback: usize) -> Box<dyn TermEngine> {
    Box::new(alacritty::AlacrittyEngine::new(cols, rows, scrollback))
}

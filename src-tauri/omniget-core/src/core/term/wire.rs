use std::collections::{HashMap, VecDeque};
use std::hash::{Hash, Hasher};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use super::agent::AgentSignal;
use super::engine::{CellView, CursorShape, CursorView, Span, StyleKey, TermEngine};

pub const MAGIC: u32 = u32::from_le_bytes(*b"OGT1");
pub const HEADER_LEN: usize = 10;
pub const GLOBAL_SLOT: u16 = 0xFFFF;
pub const GRAPHEME_BASE: u32 = 0x11_0000;

pub const FULL: u8 = 0;
pub const ROWS: u8 = 1;
pub const SCROLL: u8 = 2;
pub const STYLES: u8 = 3;
pub const CURSOR: u8 = 4;
pub const META: u8 = 5;
pub const AGENT: u8 = 6;
pub const MINI: u8 = 7;
pub const SEL: u8 = 8;
pub const GRAPHEMES: u8 = 9;

pub const STYLES_RESET: u8 = 1;

pub const MAX_INFLIGHT: usize = 2;
pub const CREDIT_STALL: Duration = Duration::from_secs(3);
pub const MINI_EVERY: Duration = Duration::from_millis(100);
const STYLES_MAX: usize = 60_000;
pub const META_ALIVE: u32 = 1 << 13;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(from = "u8", into = "u8")]
pub enum Lod {
    Text,
    Mini,
    Card,
}

impl From<u8> for Lod {
    fn from(v: u8) -> Self {
        match v {
            0 => Lod::Text,
            1 => Lod::Mini,
            _ => Lod::Card,
        }
    }
}

impl From<Lod> for u8 {
    fn from(l: Lod) -> u8 {
        match l {
            Lod::Text => 0,
            Lod::Mini => 1,
            Lod::Card => 2,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Visible {
    pub id: String,
    pub lod: Lod,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SlotInfo {
    pub id: String,
    pub slot: u16,
}

pub struct SlotInput<'a> {
    pub engine: &'a dyn TermEngine,
    pub content_seq: u64,
    pub generation: u32,
    pub agent: &'a AgentSignal,
    pub agent_seq: u64,
    pub title: &'a str,
    pub cwd: &'a str,
    pub alive: bool,
}

pub trait WireSource {
    fn visit(&self, id: &str, f: &mut dyn FnMut(&SlotInput<'_>)) -> bool;
}

fn put_u16(b: &mut Vec<u8>, v: u16) {
    b.extend_from_slice(&v.to_le_bytes());
}
fn put_u32(b: &mut Vec<u8>, v: u32) {
    b.extend_from_slice(&v.to_le_bytes());
}
fn put_str16(b: &mut Vec<u8>, s: &str) {
    let bytes = s.as_bytes();
    let mut n = bytes.len().min(u16::MAX as usize);
    while n > 0 && !s.is_char_boundary(n) {
        n -= 1;
    }
    put_u16(b, n as u16);
    b.extend_from_slice(&bytes[..n]);
}

#[derive(Default)]
pub struct Sections {
    buf: Vec<u8>,
    count: u16,
}

impl Sections {
    pub fn section(&mut self, slot: u16, kind: u8, flags: u8, payload: impl FnOnce(&mut Vec<u8>)) {
        put_u16(&mut self.buf, slot);
        self.buf.push(kind);
        self.buf.push(flags);
        let len_at = self.buf.len();
        put_u32(&mut self.buf, 0);
        let start = self.buf.len();
        payload(&mut self.buf);
        let len = (self.buf.len() - start) as u32;
        self.buf[len_at..len_at + 4].copy_from_slice(&len.to_le_bytes());
        self.count += 1;
    }

    pub fn is_empty(&self) -> bool {
        self.count == 0
    }

    fn append(&mut self, other: Sections) {
        self.buf.extend_from_slice(&other.buf);
        self.count += other.count;
    }

    pub fn into_frame(self, seq: u32) -> Vec<u8> {
        let mut out = Vec::with_capacity(HEADER_LEN + self.buf.len());
        put_u32(&mut out, MAGIC);
        put_u32(&mut out, seq);
        put_u16(&mut out, self.count);
        out.extend_from_slice(&self.buf);
        out
    }
}

struct SlotState {
    slot: u16,
    lod: Lod,
    visible: bool,
    need_full: bool,
    cols: u16,
    rows: Vec<u64>,
    content_seq: Option<u64>,
    generation: Option<u32>,
    cursor: Option<CursorView>,
    meta: Option<u64>,
    agent_seq: Option<u64>,
    sel: Vec<Span>,
    mini_at: Option<Instant>,
}

impl SlotState {
    fn reset(&mut self) {
        self.need_full = true;
        self.rows.clear();
        self.content_seq = None;
        self.cursor = None;
        self.meta = None;
        self.agent_seq = None;
        self.sel.clear();
        self.mini_at = None;
    }
}

pub struct WireSubscriber {
    next_seq: u32,
    inflight: VecDeque<(u32, Instant)>,
    styles: HashMap<StyleKey, u16>,
    next_style: u16,
    styles_reset: bool,
    slots: HashMap<String, SlotState>,
    next_slot: u16,
    order: Vec<String>,
    scratch: Vec<CellView>,
}

impl Default for WireSubscriber {
    fn default() -> Self {
        Self::new()
    }
}

fn hash_row(cells: &[CellView]) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    cells.hash(&mut h);
    h.finish()
}

fn rgb_to_xterm(r: u8, g: u8, b: u8) -> u8 {
    let q = |v: u8| -> u8 {
        if v < 48 {
            0
        } else if v < 115 {
            1
        } else {
            ((v as u16 - 35) / 40) as u8
        }
    };
    16 + 36 * q(r) + 6 * q(g) + q(b)
}

fn mini_byte(c: &CellView) -> u8 {
    use super::engine::color;
    let pick = if c.cp != 0 {
        c.style.fg
    } else if c.style.bg != color::DEFAULT && c.style.bg != color::SWAPPED_DEFAULT {
        c.style.bg
    } else if c.style.bg == color::SWAPPED_DEFAULT {
        return 255;
    } else {
        return 0;
    };
    match pick >> 24 {
        1 => match (pick & 0xFF) as u8 {
            0 => 16,
            255 => 254,
            i => i,
        },
        2 => rgb_to_xterm((pick >> 16) as u8, (pick >> 8) as u8, pick as u8),
        _ => 255,
    }
}

impl WireSubscriber {
    pub fn new() -> Self {
        Self {
            next_seq: 1,
            inflight: VecDeque::new(),
            styles: HashMap::new(),
            next_style: 1,
            styles_reset: false,
            slots: HashMap::new(),
            next_slot: 0,
            order: Vec::new(),
            scratch: Vec::new(),
        }
    }

    pub fn set_visible(&mut self, visible: &[Visible]) -> Vec<SlotInfo> {
        for st in self.slots.values_mut() {
            st.visible = false;
        }
        self.order.clear();
        let mut out = Vec::with_capacity(visible.len());
        for v in visible {
            if self.order.contains(&v.id) {
                continue;
            }
            if !self.slots.contains_key(&v.id) {
                if self.next_slot == GLOBAL_SLOT {
                    continue;
                }
                let slot = self.next_slot;
                self.next_slot += 1;
                self.slots.insert(
                    v.id.clone(),
                    SlotState {
                        slot,
                        lod: v.lod,
                        visible: false,
                        need_full: true,
                        cols: 0,
                        rows: Vec::new(),
                        content_seq: None,
                        generation: None,
                        cursor: None,
                        meta: None,
                        agent_seq: None,
                        sel: Vec::new(),
                        mini_at: None,
                    },
                );
            }
            let st = self.slots.get_mut(&v.id).expect("inserted above");
            if !st.visible_before_call(v.lod) {
                st.reset();
            }
            st.lod = v.lod;
            st.visible = true;
            self.order.push(v.id.clone());
            out.push(SlotInfo {
                id: v.id.clone(),
                slot: st.slot,
            });
        }
        for st in self.slots.values_mut() {
            if !st.visible {
                st.reset();
            }
        }
        out
    }

    pub fn frame_done(&mut self, seq: u32) {
        while let Some(&(q, _)) = self.inflight.front() {
            if q > seq {
                break;
            }
            self.inflight.pop_front();
        }
    }

    pub fn can_send(&mut self, now: Instant) -> bool {
        if let Some(&(_, at)) = self.inflight.front() {
            if now.duration_since(at) >= CREDIT_STALL {
                self.inflight.clear();
                self.resync();
            }
        }
        self.inflight.len() < MAX_INFLIGHT
    }

    pub fn resync(&mut self) {
        self.styles.clear();
        self.next_style = 1;
        self.styles_reset = true;
        for st in self.slots.values_mut() {
            st.reset();
        }
    }

    fn style_id(&mut self, key: StyleKey, new: &mut Vec<(u16, StyleKey)>) -> u16 {
        if key == StyleKey::default() {
            return 0;
        }
        if let Some(&id) = self.styles.get(&key) {
            return id;
        }
        let id = self.next_style;
        self.next_style = self.next_style.wrapping_add(1).max(1);
        self.styles.insert(key, id);
        new.push((id, key));
        id
    }

    pub fn build(&mut self, src: &dyn WireSource, now: Instant) -> (Option<Vec<u8>>, bool) {
        if self.styles.len() > STYLES_MAX {
            self.resync();
        }
        let mut new_styles: Vec<(u16, StyleKey)> = Vec::new();
        let mut body = Sections::default();
        let mut retry = false;
        let order = self.order.clone();
        for id in &order {
            let Some(mut st) = self.slots.remove(id) else {
                continue;
            };
            let mut secs = Sections::default();
            src.visit(id, &mut |input| {
                retry |= self.encode_slot(&mut st, input, now, &mut secs, &mut new_styles);
            });
            self.slots.insert(id.clone(), st);
            body.append(secs);
        }
        let reset = std::mem::take(&mut self.styles_reset);
        if body.is_empty() && new_styles.is_empty() && !reset {
            return (None, retry);
        }
        let mut frame = Sections::default();
        if !new_styles.is_empty() || reset {
            frame.section(
                GLOBAL_SLOT,
                STYLES,
                if reset { STYLES_RESET } else { 0 },
                |b| {
                    put_u16(b, new_styles.len() as u16);
                    for (id, k) in &new_styles {
                        put_u16(b, *id);
                        put_u32(b, k.fg);
                        put_u32(b, k.bg);
                        put_u32(b, k.ul);
                        put_u16(b, k.flags);
                        put_u32(b, k.link);
                    }
                },
            );
        }
        frame.append(body);
        let seq = self.next_seq;
        self.next_seq = self.next_seq.wrapping_add(1).max(1);
        self.inflight.push_back((seq, now));
        (Some(frame.into_frame(seq)), retry)
    }

    fn encode_row(
        &mut self,
        y: u16,
        cells: &[CellView],
        graphemes: &mut Vec<String>,
        new_styles: &mut Vec<(u16, StyleKey)>,
        out: &mut Vec<u8>,
    ) {
        let blank = StyleKey::default();
        let end = cells
            .iter()
            .rposition(|c| c.cp != 0 || c.style != blank)
            .map(|i| i + 1)
            .unwrap_or(0);
        put_u16(out, y);
        let n_runs_at = out.len();
        put_u16(out, 0);
        let mut n_runs: u16 = 0;
        let mut i = 0;
        while i < end {
            let key = cells[i].style;
            let sid = self.style_id(key, new_styles);
            let mut j = i;
            while j < end && cells[j].style == key {
                j += 1;
            }
            put_u16(out, sid);
            put_u16(out, (j - i) as u16);
            for c in &cells[i..j] {
                let cp = match &c.cluster {
                    Some(s) => {
                        let idx = match graphemes.iter().position(|g| g == s) {
                            Some(p) => p,
                            None => {
                                graphemes.push(s.clone());
                                graphemes.len() - 1
                            }
                        };
                        GRAPHEME_BASE + idx as u32
                    }
                    None => c.cp,
                };
                put_u32(out, cp);
            }
            n_runs += 1;
            i = j;
        }
        out[n_runs_at..n_runs_at + 2].copy_from_slice(&n_runs.to_le_bytes());
    }

    fn encode_slot(
        &mut self,
        st: &mut SlotState,
        input: &SlotInput<'_>,
        now: Instant,
        secs: &mut Sections,
        new_styles: &mut Vec<(u16, StyleKey)>,
    ) -> bool {
        let slot = st.slot;
        let e = input.engine;
        if st.generation != Some(input.generation) {
            st.reset();
            st.generation = Some(input.generation);
        }
        let content_changed = st.content_seq != Some(input.content_seq) || st.need_full;
        let mut deferred = false;

        if st.lod == Lod::Text && content_changed {
            let (cols, nrows) = e.size();
            let mut cells_rows: Vec<Vec<CellView>> = Vec::with_capacity(nrows as usize);
            let mut hashes = Vec::with_capacity(nrows as usize);
            for y in 0..nrows {
                e.row(y, &mut self.scratch);
                hashes.push(hash_row(&self.scratch));
                cells_rows.push(std::mem::take(&mut self.scratch));
            }
            let full = st.need_full || st.cols != cols || st.rows.len() != nrows as usize;
            let mut graphemes: Vec<String> = Vec::new();
            let mut rows_buf: Vec<u8> = Vec::new();
            let mut scroll: Option<i16> = None;
            let mut count: u16 = 0;
            if full {
                for (y, cells) in cells_rows.iter().enumerate() {
                    self.encode_row(y as u16, cells, &mut graphemes, new_styles, &mut rows_buf);
                }
            } else {
                let mut old = st.rows.clone();
                if let Some(d) = detect_scroll(&old, &hashes) {
                    scroll = Some(d as i16);
                    old = shift(&old, d);
                }
                for (y, cells) in cells_rows.iter().enumerate() {
                    if old[y] != hashes[y] {
                        self.encode_row(y as u16, cells, &mut graphemes, new_styles, &mut rows_buf);
                        count += 1;
                    }
                }
            }
            if !graphemes.is_empty() {
                secs.section(slot, GRAPHEMES, 0, |b| {
                    put_u16(b, graphemes.len() as u16);
                    for g in &graphemes {
                        put_str16(b, g);
                    }
                });
            }
            if full {
                secs.section(slot, FULL, 0, |b| {
                    put_u16(b, cols);
                    put_u16(b, nrows);
                    put_u32(b, input.generation);
                    b.extend_from_slice(&rows_buf);
                });
            } else {
                if let Some(d) = scroll {
                    secs.section(slot, SCROLL, 0, |b| {
                        put_u16(b, 0);
                        put_u16(b, nrows);
                        b.extend_from_slice(&d.to_le_bytes());
                    });
                }
                if count > 0 {
                    secs.section(slot, ROWS, 0, |b| {
                        put_u16(b, count);
                        b.extend_from_slice(&rows_buf);
                    });
                }
            }
            st.cols = cols;
            st.rows = hashes;

            let cur = e.cursor();
            if st.cursor != Some(cur) {
                secs.section(slot, CURSOR, 0, |b| {
                    put_u16(b, cur.x);
                    put_u16(b, cur.y);
                    b.push(match cur.shape {
                        CursorShape::Block => 0,
                        CursorShape::Underline => 1,
                        CursorShape::Beam => 2,
                        CursorShape::Hollow => 3,
                        CursorShape::Hidden => 4,
                    });
                    b.push(cur.visible as u8 | (cur.blink as u8) << 1);
                });
                st.cursor = Some(cur);
            }
        }

        if content_changed {
            let modes = e.modes().bits() | if input.alive { META_ALIVE } else { 0 };
            let offset = e.display_offset() as u32;
            let history = e.history_len() as u32;
            let mut h = std::collections::hash_map::DefaultHasher::new();
            (modes, offset, history, input.title, input.cwd).hash(&mut h);
            let mh = h.finish();
            if st.meta != Some(mh) {
                secs.section(slot, META, 0, |b| {
                    put_u32(b, modes);
                    put_u32(b, offset);
                    put_u32(b, history);
                    put_str16(b, input.title);
                    put_str16(b, input.cwd);
                });
                st.meta = Some(mh);
            }
        }

        if st.agent_seq != Some(input.agent_seq) {
            let a = input.agent;
            secs.section(slot, AGENT, 0, |b| {
                b.push(a.state.code());
                b.push(a.source.code());
                b.push(a.progress.unwrap_or(255));
                b.extend_from_slice(&a.since_ms.to_le_bytes());
                put_str16(b, a.message.as_deref().unwrap_or(""));
            });
            st.agent_seq = Some(input.agent_seq);
        }

        if st.lod == Lod::Text && content_changed {
            let sel = e.highlights();
            if sel != st.sel {
                secs.section(slot, SEL, 0, |b| {
                    put_u16(b, sel.len() as u16);
                    for s in &sel {
                        b.push(s.kind);
                        b.push(s.block as u8);
                        put_u16(b, s.start.0);
                        put_u16(b, s.start.1);
                        put_u16(b, s.end.0);
                        put_u16(b, s.end.1);
                    }
                });
                st.sel = sel;
            }
        }

        if st.lod == Lod::Mini && content_changed {
            let due = st
                .mini_at
                .map(|t| now.duration_since(t) >= MINI_EVERY)
                .unwrap_or(true);
            if due {
                let (cols, nrows) = e.size();
                let mut bytes = Vec::with_capacity(cols as usize * nrows as usize);
                for y in 0..nrows {
                    e.row(y, &mut self.scratch);
                    for x in 0..cols as usize {
                        bytes.push(self.scratch.get(x).map(mini_byte).unwrap_or(0));
                    }
                }
                secs.section(slot, MINI, 0, |b| {
                    put_u16(b, cols);
                    put_u16(b, nrows);
                    b.extend_from_slice(&bytes);
                });
                st.mini_at = Some(now);
            } else {
                deferred = true;
            }
        }

        if !deferred {
            st.content_seq = Some(input.content_seq);
            st.need_full = false;
        }
        deferred
    }
}

impl SlotState {
    fn visible_before_call(&self, lod: Lod) -> bool {
        self.lod == lod && (self.meta.is_some() || !self.rows.is_empty())
    }
}

fn detect_scroll(old: &[u64], new: &[u64]) -> Option<i32> {
    let n = old.len();
    if n < 4 || n != new.len() {
        return None;
    }
    let same = (0..n).filter(|&i| old[i] == new[i]).count();
    if same == n {
        return None;
    }
    let mut best = (0usize, 0i32);
    for d in 1..n as i32 {
        let du = d as usize;
        let up = (0..n - du).filter(|&i| new[i] == old[i + du]).count();
        if up > best.0 {
            best = (up, d);
        }
        let down = (du..n).filter(|&i| new[i] == old[i - du]).count();
        if down > best.0 {
            best = (down, -d);
        }
    }
    (best.0 >= 3 && best.0 > same + 2).then_some(best.1)
}

fn shift(old: &[u64], d: i32) -> Vec<u64> {
    let n = old.len() as i32;
    (0..n)
        .map(|i| {
            let src = i + d;
            if (0..n).contains(&src) {
                old[src as usize]
            } else {
                u64::MAX ^ i as u64
            }
        })
        .collect()
}

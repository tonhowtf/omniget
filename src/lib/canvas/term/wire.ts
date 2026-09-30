// OGT1 decoder: one binary frame per tick carrying only what changed in the
// visible terminals (estudo 98/01 §4; exact layout in the term contract §2). Everything reads straight from a
// DataView over the channel's ArrayBuffer; the only copy is the one the grid
// makes into its own typed arrays.
//
// Layout (little-endian):
//   header  u32 magic 'OGT1', u32 frame_seq, u16 n_sections
//   section u16 slot, u8 kind, u8 flags, u32 len, payload[len]

export const MAGIC = 0x3154474f; // bytes 'O','G','T','1' read as LE u32
export const HEADER_LEN = 10;
export const SECTION_HEADER_LEN = 8;

export const KIND_FULL = 0;
export const KIND_ROWS = 1;
export const KIND_SCROLL = 2;
export const KIND_STYLES = 3;
export const KIND_CURSOR = 4;
export const KIND_META = 5;
export const KIND_AGENT = 6;
export const KIND_MINI = 7;
export const KIND_SEL = 8;
export const KIND_GRAPHEMES = 9;

/** First codepoint value that indexes the section's grapheme table. */
export const GRAPHEME_BASE = 0x110000;
/** Slot of subscription-global sections (STYLES). */
export const SLOT_GLOBAL = 0xffff;
/** STYLES section flag: clear the table before adding. */
export const STYLES_RESET = 1;

// Colour u32: kind in the top byte.
export const COLOR_DEFAULT = 0;
export const COLOR_PALETTE = 1;
export const COLOR_RGB = 2;
export const COLOR_SWAPPED = 3;

// Style flags (STYLES.flags).
export const SF_BOLD = 1 << 0;
export const SF_DIM = 1 << 1;
export const SF_ITALIC = 1 << 2;
export const SF_UL_SHIFT = 3; // 3 bits: 0 none, 1 single, 2 double, 3 curly, 4 dotted, 5 dashed
export const SF_UL_MASK = 0b111 << SF_UL_SHIFT;
export const SF_STRIKE = 1 << 6;
export const SF_HIDDEN = 1 << 7;
export const SF_WIDE = 1 << 8;
export const SF_LINK = 1 << 9;

// META.modes bits.
export const MODE_ALT_SCREEN = 1 << 0;
/** META modes bit: the process is running (not a mode of the terminal). */
export const META_ALIVE = 1 << 13;
export const MODE_BRACKETED_PASTE = 1 << 1;
export const MODE_MOUSE_CLICK = 1 << 2;
export const MODE_MOUSE_DRAG = 1 << 3;
export const MODE_MOUSE_MOTION = 1 << 4;
export const MODE_MOUSE_SGR = 1 << 5;
export const MODE_APP_CURSOR = 1 << 6;
export const MODE_APP_KEYPAD = 1 << 7;
export const MODE_FOCUS_REPORT = 1 << 8;
export const MODE_SYNC = 1 << 9;
export const MODE_ALT_SCROLL = 1 << 10;
export const MODE_CURSOR_VISIBLE = 1 << 11;
export const MODE_KITTY_KEYBOARD = 1 << 12;
export const MODE_ALIVE = 1 << 13;
export const MOUSE_MODES = MODE_MOUSE_CLICK | MODE_MOUSE_DRAG | MODE_MOUSE_MOTION;

// CURSOR.flags bits.
export const CF_VISIBLE = 1 << 0;
export const CF_BLINK = 1 << 1;

export const CURSOR_BLOCK = 0;
export const CURSOR_UNDERLINE = 1;
export const CURSOR_BAR = 2;
export const CURSOR_HOLLOW = 3;
export const CURSOR_HIDDEN = 4;

// SEL range kinds.
export const SEL_SELECTION = 0;
export const SEL_MATCH = 1;

export class WireError extends Error {
  constructor(code: string) {
    super(code);
    this.name = 'WireError';
  }
}

export interface WireSection {
  slot: number;
  kind: number;
  flags: number;
  /** Byte offset of the payload in `view`. */
  off: number;
  len: number;
}

export interface WireFrame {
  seq: number;
  view: DataView;
  bytes: Uint8Array;
  sections: WireSection[];
}

export function toArrayBufferView(message: unknown): Uint8Array {
  if (message instanceof Uint8Array) return message;
  if (message instanceof ArrayBuffer) return new Uint8Array(message);
  if (ArrayBuffer.isView(message)) {
    const v = message as ArrayBufferView;
    return new Uint8Array(v.buffer, v.byteOffset, v.byteLength);
  }
  // Small raw payloads (< 1 KiB) may arrive through the eval path as a number array.
  if (Array.isArray(message)) return Uint8Array.from(message as number[]);
  const data = (message as { data?: unknown } | null)?.data;
  if (data) return toArrayBufferView(data);
  throw new WireError('ERR_TERM_BAD_CHANNEL_PAYLOAD');
}

/** Splits a frame into section descriptors. No payload is touched. */
export function decodeFrame(bytes: Uint8Array): WireFrame {
  if (bytes.byteLength < HEADER_LEN) throw new WireError('ERR_TERM_WIRE_SHORT');
  const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  if (view.getUint32(0, true) !== MAGIC) throw new WireError('ERR_TERM_WIRE_MAGIC');
  const seq = view.getUint32(4, true);
  const n = view.getUint16(8, true);
  const sections: WireSection[] = new Array(n);
  let p = HEADER_LEN;
  for (let i = 0; i < n; i++) {
    if (p + SECTION_HEADER_LEN > view.byteLength) throw new WireError('ERR_TERM_WIRE_SECTION');
    const slot = view.getUint16(p, true);
    const kind = view.getUint8(p + 2);
    const flags = view.getUint8(p + 3);
    const len = view.getUint32(p + 4, true);
    const off = p + SECTION_HEADER_LEN;
    if (off + len > view.byteLength) throw new WireError('ERR_TERM_WIRE_OVERRUN');
    sections[i] = { slot, kind, flags, off, len };
    p = off + len;
  }
  return { seq, view, bytes, sections };
}

const utf8 = new TextDecoder('utf-8');

/** Reads `u16 len` + UTF-8 bytes; returns the string and the next offset. */
export function readStr16(frame: WireFrame, off: number, end: number): [string, number] {
  if (off + 2 > end) return ['', end];
  const len = frame.view.getUint16(off, true);
  const start = off + 2;
  const stop = Math.min(start + len, end);
  return [utf8.decode(frame.bytes.subarray(start, stop)), stop];
}

// ---------------------------------------------------------------- rows

/**
 * Receives one decoded row. `runsOff` points at the first run; each run is
 * `u16 style_id, u16 n_cells, u32 cp[n_cells]`.
 */
export type RowVisitor = (y: number, nRuns: number, runsOff: number) => void;

/** Walks `count` ROW records from `off` (bounded by `end`) and returns the end offset. */
export function walkRows(frame: WireFrame, off: number, end: number, count: number, visit: RowVisitor): number {
  const v = frame.view;
  let p = off;
  let i = 0;
  while (p + 4 <= end && i < count) {
    const y = v.getUint16(p, true);
    const nRuns = v.getUint16(p + 2, true);
    const runsOff = p + 4;
    // Skip over the runs to find the next row.
    let q = runsOff;
    for (let r = 0; r < nRuns; r++) {
      if (q + 4 > end) throw new WireError('ERR_TERM_WIRE_RUN');
      const cells = v.getUint16(q + 2, true);
      q += 4 + cells * 4;
    }
    if (q > end) throw new WireError('ERR_TERM_WIRE_RUN');
    visit(y, nRuns, runsOff);
    p = q;
    i++;
  }
  return p;
}

export interface FullHeader {
  cols: number;
  rows: number;
  generation: number;
  /** Offset of the first ROW. */
  rowsOff: number;
}

export function readFull(frame: WireFrame, s: WireSection): FullHeader {
  const v = frame.view;
  if (s.len < 8) throw new WireError('ERR_TERM_WIRE_FULL');
  return {
    cols: v.getUint16(s.off, true),
    rows: v.getUint16(s.off + 2, true),
    generation: v.getUint32(s.off + 4, true),
    rowsOff: s.off + 8,
  };
}

/** ROWS: `u16 count` then `count` ROW records. Returns [count, offset of first row]. */
export function readRowsHeader(frame: WireFrame, s: WireSection): [number, number] {
  if (s.len < 2) return [0, s.off + s.len];
  return [frame.view.getUint16(s.off, true), s.off + 2];
}

export interface ScrollOp {
  top: number;
  /** Exclusive. */
  bottom: number;
  delta: number;
}

export function readScroll(frame: WireFrame, s: WireSection): ScrollOp {
  const v = frame.view;
  return { top: v.getUint16(s.off, true), bottom: v.getUint16(s.off + 2, true), delta: v.getInt16(s.off + 4, true) };
}

export interface WireStyle {
  id: number;
  fg: number;
  bg: number;
  ul: number;
  flags: number;
  link: number;
}

export const STYLE_RECORD_LEN = 20;

/** STYLES (slot 0xFFFF): `u16 count` then `{u16 id, u32 fg, u32 bg, u32 ul_color, u16 flags, u32 link_id}`. */
export function forEachStyle(frame: WireFrame, s: WireSection, fn: (st: WireStyle) => void): void {
  const v = frame.view;
  const end = s.off + s.len;
  const count = v.getUint16(s.off, true);
  let p = s.off + 2;
  const st: WireStyle = { id: 0, fg: 0, bg: 0, ul: 0, flags: 0, link: 0 };
  for (let i = 0; i < count && p + STYLE_RECORD_LEN <= end; i++) {
    st.id = v.getUint16(p, true);
    st.fg = v.getUint32(p + 2, true);
    st.bg = v.getUint32(p + 6, true);
    st.ul = v.getUint32(p + 10, true);
    st.flags = v.getUint16(p + 14, true);
    st.link = v.getUint32(p + 16, true);
    fn(st);
    p += STYLE_RECORD_LEN;
  }
}

export interface CursorInfo {
  x: number;
  y: number;
  shape: number;
  flags: number;
}

export function readCursor(frame: WireFrame, s: WireSection): CursorInfo {
  const v = frame.view;
  return { x: v.getUint16(s.off, true), y: v.getUint16(s.off + 2, true), shape: v.getUint8(s.off + 4), flags: v.getUint8(s.off + 5) };
}

export interface MetaInfo {
  modes: number;
  displayOffset: number;
  historyLen: number;
  title: string;
  cwd: string;
}

/** META: `u32 modes, u32 display_offset, u32 history_len, str16 title, str16 cwd`. */
export function readMeta(frame: WireFrame, s: WireSection): MetaInfo {
  const v = frame.view;
  const end = s.off + s.len;
  const modes = v.getUint32(s.off, true);
  const displayOffset = v.getUint32(s.off + 4, true);
  const historyLen = v.getUint32(s.off + 8, true);
  const [title, p] = readStr16(frame, s.off + 12, end);
  const [cwd] = readStr16(frame, p, end);
  return { modes, displayOffset, historyLen, title, cwd };
}

export interface AgentInfo {
  state: number;
  source: number;
  /** 0..100, 254 = indeterminate, 255 = none. */
  progress: number;
  sinceMs: number;
  message: string;
}

export const PROGRESS_INDETERMINATE = 254;
export const PROGRESS_NONE = 255;

/** AGENT: `u8 state, u8 source, u8 progress, i64 since_ms, u16 msg_len, msg`. */
export function readAgent(frame: WireFrame, s: WireSection): AgentInfo {
  const v = frame.view;
  const end = s.off + s.len;
  const since = v.getBigInt64(s.off + 3, true);
  const [message] = readStr16(frame, s.off + 11, end);
  return {
    state: v.getUint8(s.off),
    source: v.getUint8(s.off + 1),
    progress: v.getUint8(s.off + 2),
    sinceMs: Number(since),
    message,
  };
}

export interface MiniHeader {
  cols: number;
  rows: number;
  /** One byte per cell, row-major, viewing the frame's buffer: 0 empty, 255 default fg, else xterm-256 index. */
  cells: Uint8Array;
}

export function readMini(frame: WireFrame, s: WireSection): MiniHeader {
  const v = frame.view;
  const cols = v.getUint16(s.off, true);
  const rows = v.getUint16(s.off + 2, true);
  const n = Math.min(cols * rows, s.len - 4);
  return { cols, rows, cells: frame.bytes.subarray(s.off + 4, s.off + 4 + Math.max(0, n)) };
}

export interface SelRange {
  kind: number;
  /** Viewport cells; a range covers (x0,y0) through (x1,y1) inclusive, row-major. */
  y0: number;
  x0: number;
  y1: number;
  x1: number;
  /** Rectangular (block) selection. */
  block: boolean;
}

export const SEL_RECORD_LEN = 10; // the contract text says 12; the encoder writes 10

/** SEL: `u16 count` then `{u8 kind, u8 block, u16 start_x, u16 start_y, u16 end_x, u16 end_y}`. */
export function readSel(frame: WireFrame, s: WireSection): SelRange[] {
  const v = frame.view;
  const end = s.off + s.len;
  const count = s.len >= 2 ? v.getUint16(s.off, true) : 0;
  const out: SelRange[] = [];
  let p = s.off + 2;
  for (let i = 0; i < count && p + SEL_RECORD_LEN <= end; i++) {
    out.push({
      kind: v.getUint8(p),
      block: v.getUint8(p + 1) !== 0,
      x0: v.getUint16(p + 2, true),
      y0: v.getUint16(p + 4, true),
      x1: v.getUint16(p + 6, true),
      y1: v.getUint16(p + 8, true),
    });
    p += SEL_RECORD_LEN;
  }
  return out;
}

/**
 * GRAPHEMES: `u16 count` then `count x {u16 len, utf8}`; cells refer to entry
 * `i` as `GRAPHEME_BASE + i`. Frame-local, per slot.
 */
export function readGraphemes(frame: WireFrame, s: WireSection, into: Map<number, string>): void {
  const v = frame.view;
  const end = s.off + s.len;
  const count = s.len >= 2 ? v.getUint16(s.off, true) : 0;
  let p = s.off + 2;
  for (let i = 0; i < count && p + 2 <= end; i++) {
    const [str, next] = readStr16(frame, p, end);
    into.set(GRAPHEME_BASE + i, str);
    p = next;
  }
}

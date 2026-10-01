const MAGIC = 0x3154474f;
const HEADER_LEN = 10;
const SECTION_HEADER_LEN = 8;

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

export const GRAPHEME_BASE = 0x110000;
export const SLOT_GLOBAL = 0xffff;
export const STYLES_RESET = 1;

export const COLOR_PALETTE = 1;
export const COLOR_RGB = 2;
export const COLOR_SWAPPED = 3;

export const SF_BOLD = 1 << 0;
export const SF_DIM = 1 << 1;
export const SF_ITALIC = 1 << 2;
export const SF_UL_SHIFT = 3;
export const SF_UL_MASK = 0b111 << SF_UL_SHIFT;
export const SF_STRIKE = 1 << 6;
export const SF_HIDDEN = 1 << 7;
export const SF_WIDE = 1 << 8;
export const SF_LINK = 1 << 9;

export const MODE_ALT_SCREEN = 1 << 0;
export const META_ALIVE = 1 << 13;
const MODE_MOUSE_CLICK = 1 << 2;
const MODE_MOUSE_DRAG = 1 << 3;
const MODE_MOUSE_MOTION = 1 << 4;
export const MOUSE_MODES = MODE_MOUSE_CLICK | MODE_MOUSE_DRAG | MODE_MOUSE_MOTION;

export const CF_VISIBLE = 1 << 0;
export const CF_BLINK = 1 << 1;

export const CURSOR_HIDDEN = 4;

export const SEL_SELECTION = 0;
export const SEL_MATCH = 1;

class WireError extends Error {
  constructor(code: string) {
    super(code);
    this.name = 'WireError';
  }
}

export interface WireSection {
  slot: number;
  kind: number;
  flags: number;
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
  if (Array.isArray(message)) return Uint8Array.from(message as number[]);
  const data = (message as { data?: unknown } | null)?.data;
  if (data) return toArrayBufferView(data);
  throw new WireError('ERR_TERM_BAD_CHANNEL_PAYLOAD');
}

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

function readStr16(frame: WireFrame, off: number, end: number): [string, number] {
  if (off + 2 > end) return ['', end];
  const len = frame.view.getUint16(off, true);
  const start = off + 2;
  const stop = Math.min(start + len, end);
  return [utf8.decode(frame.bytes.subarray(start, stop)), stop];
}

type RowVisitor = (y: number, nRuns: number, runsOff: number) => void;

export function walkRows(frame: WireFrame, off: number, end: number, count: number, visit: RowVisitor): number {
  const v = frame.view;
  let p = off;
  let i = 0;
  while (p + 4 <= end && i < count) {
    const y = v.getUint16(p, true);
    const nRuns = v.getUint16(p + 2, true);
    const runsOff = p + 4;
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

interface FullHeader {
  cols: number;
  rows: number;
  generation: number;
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

export function readRowsHeader(frame: WireFrame, s: WireSection): [number, number] {
  if (s.len < 2) return [0, s.off + s.len];
  return [frame.view.getUint16(s.off, true), s.off + 2];
}

interface ScrollOp {
  top: number;
  bottom: number;
  delta: number;
}

export function readScroll(frame: WireFrame, s: WireSection): ScrollOp {
  const v = frame.view;
  return { top: v.getUint16(s.off, true), bottom: v.getUint16(s.off + 2, true), delta: v.getInt16(s.off + 4, true) };
}

interface WireStyle {
  id: number;
  fg: number;
  bg: number;
  ul: number;
  flags: number;
  link: number;
}

const STYLE_RECORD_LEN = 20;

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
  progress: number;
  sinceMs: number;
  message: string;
}

export const PROGRESS_INDETERMINATE = 254;
export const PROGRESS_NONE = 255;

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

interface MiniHeader {
  cols: number;
  rows: number;
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
  y0: number;
  x0: number;
  y1: number;
  x1: number;
  block: boolean;
}

const SEL_RECORD_LEN = 10;

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

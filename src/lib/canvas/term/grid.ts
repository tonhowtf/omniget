// Per-terminal mirror of the visible screen in typed arrays. The wire decoder
// hands sections over; the grid copies cells in and remembers which rows are
// dirty so the GL pass re-uploads only those.

import {
  GRAPHEME_BASE,
  KIND_AGENT,
  KIND_CURSOR,
  KIND_FULL,
  KIND_GRAPHEMES,
  KIND_META,
  KIND_MINI,
  KIND_ROWS,
  KIND_SCROLL,
  KIND_SEL,
  KIND_STYLES,
  SEL_SELECTION,
  SLOT_GLOBAL,
  STYLES_RESET,
  CF_VISIBLE,
  forEachStyle,
  readAgent,
  readCursor,
  readFull,
  readGraphemes,
  readMeta,
  readMini,
  readRowsHeader,
  readScroll,
  readSel,
  walkRows,
  type CursorInfo,
  type SelRange,
  type WireFrame,
  type WireSection,
} from './wire';
import type { AgentInfo, MetaInfo } from './wire';

export interface CellStyle {
  fg: number;
  bg: number;
  ul: number;
  flags: number;
  link: number;
}

const DEFAULT_STYLE: CellStyle = { fg: 0, bg: 0, ul: 0, flags: 0, link: 0 };

/** Style table of one subscription (STYLES is global, slot 0xFFFF). */
export class StyleTable {
  private map = new Map<number, CellStyle>();
  /** Bumped when an existing id changed or the table was reset: every texel is stale. */
  epoch = 0;

  get(id: number): CellStyle {
    return this.map.get(id) ?? DEFAULT_STYLE;
  }

  apply(frame: WireFrame, s: WireSection): void {
    if (s.flags & STYLES_RESET) {
      this.map.clear();
      this.epoch++;
    }
    let redefined = false;
    forEachStyle(frame, s, (st) => {
      const prev = this.map.get(st.id);
      if (prev && (prev.fg !== st.fg || prev.bg !== st.bg || prev.flags !== st.flags || prev.ul !== st.ul)) redefined = true;
      this.map.set(st.id, { fg: st.fg, bg: st.bg, ul: st.ul, flags: st.flags, link: st.link });
    });
    if (redefined) this.epoch++;
  }

  clear(): void {
    this.map.clear();
    this.epoch++;
  }
}

/** Upper bound so a corrupt frame cannot allocate gigabytes. */
export const MAX_COLS = 1000;
export const MAX_ROWS = 500;

export class TermGrid {
  cols = 0;
  rows = 0;
  generation = -1;
  /** Codepoint per cell; >= GRAPHEME_BASE indexes `clusters`. 0 = empty/spacer. */
  cp = new Uint32Array(0);
  styleId = new Uint16Array(0);
  /** Persistent grapheme intern table (frame tables are remapped into it). */
  clusters: string[] = [];
  private clusterIds = new Map<string, number>();
  /** Frame-local grapheme table, valid while one frame is being applied. */
  private frameClusters = new Map<number, string>();

  cursor: CursorInfo = { x: 0, y: 0, shape: 0, flags: CF_VISIBLE };
  sel: SelRange[] = [];

  miniCols = 0;
  miniRows = 0;
  mini = new Uint8Array(0);
  miniDirty = false;

  /** Rows whose texels must be rebuilt. */
  dirty = new Uint8Array(0);
  anyDirty = false;
  /** Size changed: the texture must be reallocated. */
  resized = false;
  /** Bumped on every applied change; the pass compares to know when to redraw. */
  version = 0;
  /** Last META/AGENT seen; `liveDirty` tells the hub to publish them to the chrome. */
  meta: MetaInfo | null = null;
  agent: AgentInfo | null = null;
  liveDirty = false;

  constructor(
    readonly id: string,
    readonly styles: StyleTable,
  ) {}

  style(id: number): CellStyle {
    return this.styles.get(id);
  }

  cluster(cp: number): string {
    if (cp >= GRAPHEME_BASE) return this.clusters[cp - GRAPHEME_BASE] ?? '';
    return cp ? String.fromCodePoint(cp) : '';
  }

  hasSelection(): boolean {
    return this.sel.some((r) => r.kind === SEL_SELECTION);
  }

  private resize(cols: number, rows: number): void {
    cols = Math.min(Math.max(0, cols), MAX_COLS);
    rows = Math.min(Math.max(0, rows), MAX_ROWS);
    if (cols === this.cols && rows === this.rows) return;
    const cp = new Uint32Array(cols * rows);
    const st = new Uint16Array(cols * rows);
    const keepR = Math.min(rows, this.rows);
    const keepC = Math.min(cols, this.cols);
    for (let y = 0; y < keepR; y++) {
      cp.set(this.cp.subarray(y * this.cols, y * this.cols + keepC), y * cols);
      st.set(this.styleId.subarray(y * this.cols, y * this.cols + keepC), y * cols);
    }
    this.cp = cp;
    this.styleId = st;
    this.cols = cols;
    this.rows = rows;
    this.dirty = new Uint8Array(rows).fill(1);
    this.anyDirty = rows > 0;
    this.resized = true;
  }

  markAll(): void {
    this.dirty.fill(1);
    this.anyDirty = this.rows > 0;
  }

  private markRow(y: number): void {
    if (y < this.rows) {
      this.dirty[y] = 1;
      this.anyDirty = true;
    }
  }

  private internCp(cp: number): number {
    if (cp < GRAPHEME_BASE) return cp;
    const s = this.frameClusters.get(cp);
    if (s === undefined) return 0xfffd;
    let id = this.clusterIds.get(s);
    if (id === undefined) {
      id = this.clusters.length;
      this.clusters.push(s);
      this.clusterIds.set(s, id);
    }
    return GRAPHEME_BASE + id;
  }

  private writeRow(frame: WireFrame, y: number, nRuns: number, runsOff: number): void {
    if (y >= this.rows) return;
    const v = frame.view;
    const base = y * this.cols;
    let x = 0;
    let p = runsOff;
    for (let r = 0; r < nRuns; r++) {
      const sid = v.getUint16(p, true);
      const n = v.getUint16(p + 2, true);
      p += 4;
      for (let i = 0; i < n; i++, p += 4) {
        if (x >= this.cols) continue;
        this.cp[base + x] = this.internCp(v.getUint32(p, true));
        this.styleId[base + x] = sid;
        x++;
      }
    }
    // A row message describes the whole row: blank the tail.
    if (x < this.cols) {
      this.cp.fill(0, base + x, base + this.cols);
      this.styleId.fill(0, base + x, base + this.cols);
    }
    this.markRow(y);
  }

  /** `bottom` exclusive. */
  private scroll(top: number, bottomEx: number, delta: number): void {
    const c = this.cols;
    const bottom = Math.min(bottomEx, this.rows) - 1;
    if (top > bottom || delta === 0 || c === 0) return;
    const height = bottom - top + 1;
    const d = Math.max(-height, Math.min(height, delta));
    if (d > 0) {
      // Content moves up; `d` new blank rows at the bottom of the region.
      this.cp.copyWithin(top * c, (top + d) * c, (bottom + 1) * c);
      this.styleId.copyWithin(top * c, (top + d) * c, (bottom + 1) * c);
      this.cp.fill(0, (bottom + 1 - d) * c, (bottom + 1) * c);
      this.styleId.fill(0, (bottom + 1 - d) * c, (bottom + 1) * c);
    } else {
      const k = -d;
      this.cp.copyWithin((top + k) * c, top * c, (bottom + 1 - k) * c);
      this.styleId.copyWithin((top + k) * c, top * c, (bottom + 1 - k) * c);
      this.cp.fill(0, top * c, (top + k) * c);
      this.styleId.fill(0, top * c, (top + k) * c);
    }
    for (let y = top; y <= bottom; y++) this.markRow(y);
  }

  /** Grapheme tables go first so ROWS in the same frame can resolve them. */
  preApply(frame: WireFrame, s: WireSection): void {
    if (s.kind === KIND_GRAPHEMES) readGraphemes(frame, s, this.frameClusters);
  }

  apply(frame: WireFrame, s: WireSection): void {
    switch (s.kind) {
      case KIND_FULL: {
        const h = readFull(frame, s);
        this.resize(h.cols, h.rows);
        this.generation = h.generation;
        this.cp.fill(0);
        this.styleId.fill(0);
        walkRows(frame, h.rowsOff, s.off + s.len, h.rows, (y, n, off) => this.writeRow(frame, y, n, off));
        this.markAll();
        break;
      }
      case KIND_ROWS: {
        const [count, off] = readRowsHeader(frame, s);
        walkRows(frame, off, s.off + s.len, count, (y, n, o) => this.writeRow(frame, y, n, o));
        break;
      }
      case KIND_SCROLL: {
        const op = readScroll(frame, s);
        this.scroll(op.top, op.bottom, op.delta);
        break;
      }
      case KIND_CURSOR:
        this.cursor = readCursor(frame, s);
        break;
      case KIND_META:
        this.meta = readMeta(frame, s);
        this.liveDirty = true;
        break;
      case KIND_AGENT:
        this.agent = readAgent(frame, s);
        this.liveDirty = true;
        break;
      case KIND_MINI: {
        const m = readMini(frame, s);
        if (this.mini.length !== m.cols * m.rows) this.mini = new Uint8Array(m.cols * m.rows);
        this.mini.set(m.cells.subarray(0, this.mini.length));
        this.miniCols = m.cols;
        this.miniRows = m.rows;
        this.miniDirty = true;
        break;
      }
      case KIND_SEL: {
        const before = this.sel;
        this.sel = readSel(frame, s);
        for (const r of before) for (let y = r.y0; y <= r.y1; y++) this.markRow(y);
        for (const r of this.sel) for (let y = r.y0; y <= r.y1; y++) this.markRow(y);
        this.liveDirty = true;
        break;
      }
      case KIND_GRAPHEMES:
        break;
      default:
        return;
    }
    this.version++;
  }

  endFrame(): void {
    this.frameClusters.clear();
  }

  /** Selection/match kind for a cell: -1 none, else SEL_* (selection wins over a match). */
  selKind(x: number, y: number): number {
    let best = -1;
    for (const r of this.sel) {
      if (y < r.y0 || y > r.y1) continue;
      let inside: boolean;
      if (r.block) inside = x >= Math.min(r.x0, r.x1) && x <= Math.max(r.x0, r.x1);
      else if (r.y0 === r.y1) inside = x >= r.x0 && x <= r.x1;
      else if (y === r.y0) inside = x >= r.x0;
      else if (y === r.y1) inside = x <= r.x1;
      else inside = true;
      if (inside && (best === -1 || r.kind === SEL_SELECTION)) best = r.kind;
    }
    return best;
  }

  /** Plain text of one row (used for the aria-live region and tests). */
  rowText(y: number): string {
    if (y >= this.rows) return '';
    let s = '';
    const base = y * this.cols;
    for (let x = 0; x < this.cols; x++) {
      const cp = this.cp[base + x];
      s += cp === 0 ? ' ' : this.cluster(cp);
    }
    return s.replace(/\s+$/, '');
  }
}

/**
 * Applies sections of one decoded frame in order. `gridOf` resolves a slot to
 * its grid (null = unknown slot). Returns the sections it could not route, so
 * the caller can replay them once the slot's id is known.
 */
export function applySections(
  frame: WireFrame,
  sections: readonly WireSection[],
  styles: StyleTable,
  gridOf: (slot: number) => TermGrid | null,
  touched: Set<TermGrid>,
): WireSection[] {
  const unrouted: WireSection[] = [];
  for (const s of sections) {
    if (s.slot === SLOT_GLOBAL) {
      if (s.kind === KIND_STYLES) styles.apply(frame, s);
      continue;
    }
    const g = gridOf(s.slot);
    if (!g) {
      unrouted.push(s);
      continue;
    }
    if (s.kind === KIND_GRAPHEMES) g.preApply(frame, s);
    g.apply(frame, s);
    touched.add(g);
  }
  for (const g of touched) g.endFrame();
  return unrouted;
}

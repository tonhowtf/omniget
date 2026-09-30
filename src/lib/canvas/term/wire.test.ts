import { describe, expect, it, vi } from 'vitest';

// The wire pulls in the canvas prefs, which read the settings store: its
// module-level runes need a stand-in outside the Svelte compiler.
vi.hoisted(() => {
  const g = globalThis as Record<string, unknown>;
  g.$state = <T>(value: T) => value;
  g.$derived = <T>(value: T) => value;
});
import {
  KIND_AGENT,
  KIND_CURSOR,
  KIND_FULL,
  KIND_GRAPHEMES,
  KIND_META,
  KIND_ROWS,
  KIND_SCROLL,
  KIND_SEL,
  KIND_STYLES,
  MAGIC,
  SLOT_GLOBAL,
  decodeFrame,
  readAgent,
  readMeta,
} from './wire';
import { StyleTable, TermGrid, applySections } from './grid';
import { nextLod } from './lod';
import { routeKey } from './input';

/** Tiny little-endian writer mirroring the Rust encoder. */
class W {
  bytes: number[] = [];
  u8(v: number) {
    this.bytes.push(v & 255);
    return this;
  }
  u16(v: number) {
    return this.u8(v).u8(v >> 8);
  }
  i16(v: number) {
    return this.u16(v & 0xffff);
  }
  u32(v: number) {
    return this.u16(v & 0xffff).u16(v >>> 16);
  }
  i64(v: number) {
    const b = new DataView(new ArrayBuffer(8));
    b.setBigInt64(0, BigInt(v), true);
    for (let i = 0; i < 8; i++) this.u8(b.getUint8(i));
    return this;
  }
  str(s: string) {
    const b = new TextEncoder().encode(s);
    this.u16(b.length);
    for (const x of b) this.u8(x);
    return this;
  }
}

function row(w: W, y: number, runs: Array<[number, number[]]>) {
  w.u16(y).u16(runs.length);
  for (const [style, cps] of runs) {
    w.u16(style).u16(cps.length);
    for (const cp of cps) w.u32(cp);
  }
}

function frame(seq: number, sections: Array<[number, number, number, W]>): Uint8Array {
  const out = new W();
  out.u32(MAGIC).u32(seq).u16(sections.length);
  for (const [slot, kind, flags, payload] of sections) {
    out.u16(slot).u8(kind).u8(flags).u32(payload.bytes.length);
    out.bytes.push(...payload.bytes);
  }
  return Uint8Array.from(out.bytes);
}

const cps = (s: string) => [...s].map((c) => c.codePointAt(0)!);

function apply(bytes: Uint8Array, grid: TermGrid, styles: StyleTable) {
  const f = decodeFrame(bytes);
  return applySections(f, f.sections, styles, (slot) => (slot === 0 ? grid : null), new Set());
}

describe('OGT1 decoder + grid', () => {
  it('applies FULL, STYLES, ROWS with graphemes, SCROLL, CURSOR, SEL', () => {
    const styles = new StyleTable();
    const grid = new TermGrid('t1', styles);

    const st = new W().u16(1).u16(7).u32(0x02ff0000).u32(0).u32(0).u16(1).u32(0);
    const full = new W().u16(5).u16(3).u32(1);
    row(full, 0, [[0, cps('hi')]]);
    row(full, 1, [[7, cps('abc')]]);
    row(full, 2, []);
    const bytes = frame(1, [
      [SLOT_GLOBAL, KIND_STYLES, 0, st],
      [0, KIND_FULL, 0, full],
    ]);
    expect(apply(bytes, grid, styles)).toEqual([]);
    expect(grid.cols).toBe(5);
    expect(grid.rows).toBe(3);
    expect(grid.rowText(0)).toBe('hi');
    expect(grid.rowText(1)).toBe('abc');
    expect(grid.style(grid.styleId[5]).fg).toBe(0x02ff0000);

    const gr = new W().u16(1).str('👍🏽');
    const rows = new W().u16(1);
    row(rows, 2, [[0, [0x110000, 0, ...cps('x')]]]);
    const cur = new W().u16(3).u16(2).u8(0).u8(3);
    const sel = new W().u16(1).u8(0).u8(0).u16(1).u16(0).u16(2).u16(1);
    apply(
      frame(2, [
        [0, KIND_GRAPHEMES, 0, gr],
        [0, KIND_ROWS, 0, rows],
        [0, KIND_CURSOR, 0, cur],
        [0, KIND_SEL, 0, sel],
      ]),
      grid,
      styles,
    );
    expect(grid.rowText(2)).toBe('👍🏽 x');
    expect(grid.cursor).toEqual({ x: 3, y: 2, shape: 0, flags: 3 });
    expect(grid.selKind(1, 0)).toBe(0);
    expect(grid.selKind(0, 0)).toBe(-1);
    expect(grid.selKind(2, 1)).toBe(0);
    expect(grid.selKind(3, 1)).toBe(-1);

    // Scroll the whole screen up by one: row 1 becomes row 0.
    apply(frame(3, [[0, KIND_SCROLL, 0, new W().u16(0).u16(3).i16(1)]]), grid, styles);
    expect(grid.rowText(0)).toBe('abc');
    expect(grid.rowText(1)).toBe('👍🏽 x');
  });

  it('reports sections for unknown slots so they can be replayed', () => {
    const styles = new StyleTable();
    const grid = new TermGrid('t', styles);
    const f = decodeFrame(frame(1, [[4, KIND_CURSOR, 0, new W().u16(0).u16(0).u8(0).u8(1)]]));
    const rest = applySections(f, f.sections, styles, () => null, new Set());
    expect(rest).toHaveLength(1);
    expect(grid.version).toBe(0);
  });

  it('reads META and AGENT', () => {
    const meta = new W().u32(0b11).u32(4).u32(120).str('claude').str('/tmp');
    const agent = new W().u8(4).u8(1).u8(254).i64(1700000000000).str('needs you');
    const f = decodeFrame(frame(9, [
      [0, KIND_META, 0, meta],
      [0, KIND_AGENT, 0, agent],
    ]));
    expect(f.seq).toBe(9);
    expect(readMeta(f, f.sections[0])).toEqual({ modes: 3, displayOffset: 4, historyLen: 120, title: 'claude', cwd: '/tmp' });
    const a = readAgent(f, f.sections[1]);
    expect(a).toEqual({ state: 4, source: 1, progress: 254, sinceMs: 1700000000000, message: 'needs you' });
  });

  it('rejects a bad magic and an overrun', () => {
    expect(() => decodeFrame(Uint8Array.from([1, 2, 3, 4, 0, 0, 0, 0, 0, 0]))).toThrow();
    const bad = frame(1, [[0, KIND_CURSOR, 0, new W().u16(0)]]);
    bad[14] = 200; // section len far past the end
    expect(() => decodeFrame(bad)).toThrow();
  });
});

describe('LOD hysteresis', () => {
  it('enters text at 7.5 px and leaves below 6.5 px', () => {
    expect(nextLod(null, 7.4, 400, false)).toBe('mini');
    expect(nextLod(null, 7.5, 400, false)).toBe('text');
    expect(nextLod('text', 6.6, 400, false)).toBe('text');
    expect(nextLod('text', 6.4, 400, false)).toBe('mini');
    expect(nextLod(null, 3, 139, false)).toBe('card');
    expect(nextLod('card', 3, 160, false)).toBe('card');
    expect(nextLod('card', 3, 171, false)).toBe('mini');
    expect(nextLod(null, 1, 50, true)).toBe('text');
  });
});

describe('key routing', () => {
  const ev = (key: string, m: Partial<{ shiftKey: boolean; altKey: boolean; ctrlKey: boolean; metaKey: boolean; isComposing: boolean; keyCode: number }> = {}) => ({
    key,
    shiftKey: false,
    altKey: false,
    ctrlKey: false,
    metaKey: false,
    ...m,
  });
  it('keeps text for beforeinput and named keys for Rust', () => {
    expect(routeKey(ev('a'), true, false)).toBe('text');
    expect(routeKey(ev('Enter'), true, false)).toBe('terminal');
    expect(routeKey(ev('Escape'), true, false)).toBe('terminal');
    expect(routeKey(ev('c', { ctrlKey: true }), true, false)).toBe('terminal');
  });
  it('lets dead keys and composition through untouched (ABNT2)', () => {
    expect(routeKey(ev('Dead'), true, false)).toBe('ignore');
    expect(routeKey(ev('a', { isComposing: true }), true, false)).toBe('ignore');
    expect(routeKey(ev('a', { keyCode: 229 }), false, false)).toBe('ignore');
    // AltGr+Q on Windows ABNT2 types '/'
    expect(routeKey(ev('/', { ctrlKey: true, altKey: true }), false, false)).toBe('text');
    // Option composes characters on the Mac by default
    expect(routeKey(ev('œ', { altKey: true }), true, false)).toBe('text');
    expect(routeKey(ev('œ', { altKey: true }), true, false, true)).toBe('terminal');
  });
  it('reserves app shortcuts', () => {
    // ⌘K clears the terminal, as in Terminal.app; other ⌘ keys stay with the app.
    expect(routeKey(ev('k', { metaKey: true }), true, false)).toBe('clear');
    expect(routeKey(ev('t', { metaKey: true }), true, false)).toBe('app');
    expect(routeKey(ev('c', { metaKey: true }), true, true)).toBe('copy');
    expect(routeKey(ev('c', { metaKey: true }), true, false)).toBe('app');
    expect(routeKey(ev('v', { metaKey: true }), true, false)).toBe('paste');
    expect(routeKey(ev('Escape', { metaKey: true }), true, false)).toBe('unfocus');
    expect(routeKey(ev('C', { ctrlKey: true, shiftKey: true }), false, true)).toBe('copy');
  });
});

import { terminalTheme } from '$components/central/terminal/theme';
import { canvasPrefs, faithfulRendering } from '../prefs.svelte';
import { loadUserSchemes, schemeTheme } from './themes';
import { TermAtlas } from './atlas';
import { termHub } from './hub';
import { updateLods, focusedTerm } from './lod';
import { bodyRect, cellMetrics, TERM_FONT_SIZE, TERM_HEADER_H, termFontScale } from './metrics';
import { TERM_FS, TERM_VS, THUMB_FS } from './shaders';
import type { TermGrid } from './grid';
import { isTerminalKind, terminalIdOf, type FrameInfo, type GlPass } from './gl-types';
import { registerPass } from '$lib/canvas/render/gl-layer';
import {
  CF_BLINK,
  CF_VISIBLE,
  COLOR_PALETTE,
  COLOR_RGB,
  COLOR_SWAPPED,
  CURSOR_HIDDEN,
  SEL_MATCH,
  SEL_SELECTION,
  SF_BOLD,
  SF_DIM,
  SF_HIDDEN,
  SF_ITALIC,
  SF_LINK,
  SF_STRIKE,
  SF_UL_MASK,
  SF_UL_SHIFT,
  SF_WIDE,
} from './wire';

const BLINK_MS = 530;
const UPLOAD_UNIT = 4;
const SETTLE_MS = 150;
const THUMB_BELOW_PX = 9;
const THUMB_FONT_PX = 12;
const THUMB_RENDERS_PER_FRAME = 6;
const THUMB_UNIT = 5;

const TA_STRIKE = 8;
const TA_SEL = 16;
const TA_MATCH = 32;
const TA_LINK = 64;
const TA_COLOR = 128;

type Rgba = [number, number, number, number];

interface Theme {
  fg: Rgba;
  bg: Rgba;
  cursor: Rgba;
  sel: Rgba;
  match: Rgba;
  palette: Uint32Array;
}

function parseCss(color: string | undefined, fallback: Rgba): Rgba {
  if (!color) return fallback;
  const m = color.match(/rgba?\(\s*([\d.]+)[,\s]+([\d.]+)[,\s]+([\d.]+)(?:[,\s/]+([\d.]+))?\s*\)/);
  if (m) return [+m[1] / 255, +m[2] / 255, +m[3] / 255, m[4] === undefined ? 1 : +m[4]];
  const h = color.match(/^#([0-9a-f]{6})$/i);
  if (h) {
    const n = parseInt(h[1], 16);
    return [((n >> 16) & 255) / 255, ((n >> 8) & 255) / 255, (n & 255) / 255, 1];
  }
  return fallback;
}

function pack(c: Rgba): number {
  return (
    ((Math.round(c[0] * 255) << 24) |
      (Math.round(c[1] * 255) << 16) |
      (Math.round(c[2] * 255) << 8) |
      Math.max(1, Math.round(c[3] * 255))) >>>
    0
  );
}

interface TermAppearance {
  theme?: string;
  customThemeId?: string;
  fontSize?: number;
  fontFamily?: string;
}

function appearanceOf(node: { data?: unknown } | null | undefined): TermAppearance {
  const a = (node?.data as { appearance?: unknown } | null | undefined)?.appearance;
  return a && typeof a === 'object' ? (a as TermAppearance) : {};
}

function themeIdOf(node: { data?: unknown } | null | undefined): string {
  const a = appearanceOf(node);
  const own = a.theme === 'custom' ? a.customThemeId : a.theme;
  if (own && own !== 'system' && own !== 'default') return own;
  if (a.theme === 'system') return 'system';
  return canvasPrefs().terminal.theme || 'system';
}

function buildTheme(id: string = canvasPrefs().terminal.theme): Theme {
  let t: ReturnType<typeof terminalTheme> | null = null;
  try {
    t = (schemeTheme(id) as ReturnType<typeof terminalTheme> | null) ?? null;
    if (!t && typeof document !== 'undefined') t = terminalTheme();
  } catch {
    t = null;
  }
  const fg = parseCss(t?.foreground, [0.96, 0.96, 0.97, 1]);
  const bg = parseCss(t?.background, [0.11, 0.11, 0.12, 1]);
  const ansi = [
    t?.black,
    t?.red,
    t?.green,
    t?.yellow,
    t?.blue,
    t?.magenta,
    t?.cyan,
    t?.white,
    t?.brightBlack,
    t?.brightRed,
    t?.brightGreen,
    t?.brightYellow,
    t?.brightBlue,
    t?.brightMagenta,
    t?.brightCyan,
    t?.brightWhite,
  ];
  const defaults = [
    '#3a3a3c', '#ff453a', '#30d158', '#ffd60a', '#0a84ff', '#bf5af2', '#40c8e0', '#d1d1d6',
    '#8e8e93', '#ff6961', '#4cd964', '#ffe066', '#409cff', '#da8fff', '#70d7ff', '#ffffff',
  ];
  const palette = new Uint32Array(256);
  for (let i = 0; i < 16; i++) palette[i] = pack(parseCss(ansi[i] ?? defaults[i], parseCss(defaults[i], [1, 1, 1, 1])));
  const steps = [0, 95, 135, 175, 215, 255];
  for (let i = 0; i < 216; i++) {
    const r = steps[Math.floor(i / 36)];
    const g = steps[Math.floor(i / 6) % 6];
    const b = steps[i % 6];
    palette[16 + i] = ((r << 24) | (g << 16) | (b << 8) | 255) >>> 0;
  }
  for (let i = 0; i < 24; i++) {
    const v = 8 + i * 10;
    palette[232 + i] = ((v << 24) | (v << 16) | (v << 8) | 255) >>> 0;
  }
  const cursor = parseCss(t?.cursor, [1, 0.62, 0.04, 1]);
  const sel = parseCss(t?.selectionBackground, [1, 0.62, 0.04, 0.3]);
  return { fg, bg, cursor, sel: [sel[0], sel[1], sel[2], Math.max(0.25, sel[3])], match: [1, 0.84, 0.04, 0.35], palette };
}

interface Thumb {
  tex: WebGLTexture;
  fbo: WebGLFramebuffer;
  w: number;
  h: number;
  key: string;
}

interface GridGpu {
  grid: TermGrid;
  rev: number;
  thumb: Thumb | null;
  cells: WebGLTexture;
  mini: WebGLTexture | null;
  cols: number;
  rows: number;
  texels: Uint32Array;
  styleEpoch: number;
  glyphEpoch: number;
  themeKey: string;
  miniCols: number;
  miniRows: number;
  lastUsed: number;
}

interface TermLayout {
  x: number;
  y: number;
  cellW: number;
  cellH: number;
  cols: number;
  rows: number;
  zoom: number;
}

const layouts = new Map<string, TermLayout>();

export function termLayout(id: string): TermLayout | undefined {
  return layouts.get(id);
}

class TermPass implements GlPass {
  readonly id = 'terminals';
  readonly order = 45;
  private gl: WebGL2RenderingContext | null = null;
  private program: WebGLProgram | null = null;
  private vao: WebGLVertexArrayObject | null = null;
  private vbo: WebGLBuffer | null = null;
  private palette: WebGLTexture | null = null;
  private emptyU8: WebGLTexture | null = null;
  private emptyU32: WebGLTexture | null = null;
  private u: Record<string, WebGLUniformLocation | null> = {};
  private readonly atlas = new TermAtlas();
  private gpus = new Map<string, GridGpu>();
  private theme: Theme = buildTheme();
  private themes = new Map<string, { theme: Theme; palette: WebGLTexture | null }>();
  private lastScale = 0;
  private scaleChangedAt = 0;
  private settleTimer: ReturnType<typeof setTimeout> | null = null;
  private blinkTimer: ReturnType<typeof setTimeout> | null = null;
  private workPending = false;
  private thumbProgram: WebGLProgram | null = null;
  private thumbU: Record<string, WebGLUniformLocation | null> = {};
  private thumbBudget = 0;
  private thumbStale = false;
  requestRedraw: () => void = () => {};

  constructor() {
    termHub.redraw = () => this.requestRedraw();
  }

  refreshTheme(): void {
    this.theme = buildTheme();
    if (this.gl && this.palette) this.uploadPalette(this.gl, this.palette, this.theme);
    this.dropThemes();
    for (const g of this.gpus.values()) g.grid.markAll();
    this.requestRedraw();
  }

  private dropThemes(): void {
    const gl = this.gl;
    for (const t of this.themes.values()) if (gl && t.palette && !gl.isContextLost()) gl.deleteTexture(t.palette);
    this.themes.clear();
  }

  private themeFor(gl: WebGL2RenderingContext, id: string): { theme: Theme; palette: WebGLTexture | null } {
    let t = this.themes.get(id);
    if (!t) {
      const theme = buildTheme(id);
      const palette = gl.createTexture();
      if (palette) this.uploadPalette(gl, palette, theme);
      t = { theme, palette };
      this.themes.set(id, t);
    }
    return t;
  }

  private useTheme(gl: WebGL2RenderingContext, t: { theme: Theme; palette: WebGLTexture | null }): void {
    this.theme = t.theme;
    gl.uniform4fv(this.u.u_defFg, t.theme.fg);
    gl.uniform4fv(this.u.u_defBg, t.theme.bg);
    gl.uniform4fv(this.u.u_cursorColor, t.theme.cursor);
    gl.uniform4fv(this.u.u_selColor, t.theme.sel);
    gl.uniform4fv(this.u.u_matchColor, t.theme.match);
    gl.activeTexture(gl.TEXTURE3);
    gl.bindTexture(gl.TEXTURE_2D, t.palette ?? this.palette);
  }

  private fillBody(gl: WebGL2RenderingContext, grid: TermGrid | undefined, x: number, y: number, w: number, h: number, vw: number, vh: number): void {
    const cx = Math.max(0, x);
    const cy = Math.max(0, y);
    const cw = Math.min(vw, x + w) - cx;
    const ch = Math.min(vh, y + h) - cy;
    if (cw <= 0 || ch <= 0) return;
    let c: Rgba = this.theme.bg;
    if (grid && grid.cols > 0 && grid.rows > 0) {
      const packed = this.color(grid.style(grid.styleId[0]).bg, 'bg');
      if (packed) c = [((packed >>> 24) & 255) / 255, ((packed >>> 16) & 255) / 255, ((packed >>> 8) & 255) / 255, 1];
    }
    gl.scissor(cx, vh - (cy + ch), cw, ch);
    gl.clearColor(c[0], c[1], c[2], 1);
    gl.clear(gl.COLOR_BUFFER_BIT);
  }

  private init(gl: WebGL2RenderingContext): boolean {
    if (this.gl === gl && this.program) return true;
    this.dropGl();
    const vs = gl.createShader(gl.VERTEX_SHADER);
    const fs = gl.createShader(gl.FRAGMENT_SHADER);
    if (!vs || !fs) return false;
    gl.shaderSource(vs, TERM_VS);
    gl.compileShader(vs);
    gl.shaderSource(fs, TERM_FS);
    gl.compileShader(fs);
    const prog = gl.createProgram();
    if (!prog) return false;
    gl.attachShader(prog, vs);
    gl.attachShader(prog, fs);
    gl.bindAttribLocation(prog, 0, 'a_pos');
    gl.linkProgram(prog);
    gl.deleteShader(vs);
    gl.deleteShader(fs);
    if (!gl.getProgramParameter(prog, gl.LINK_STATUS)) {
      console.error('[term] shader link failed', gl.getProgramInfoLog(prog), gl.getShaderInfoLog(fs));
      gl.deleteProgram(prog);
      return false;
    }
    this.program = prog;
    this.thumbProgram = this.link(gl, TERM_VS, THUMB_FS);
    if (this.thumbProgram) {
      for (const name of ['u_rect', 'u_viewport', 'u_tex']) this.thumbU[name] = gl.getUniformLocation(this.thumbProgram, name);
    }
    for (const name of [
      'u_rect', 'u_viewport', 'u_cells', 'u_mini', 'u_atlas', 'u_palette', 'u_mode', 'u_cell', 'u_grid',
      'u_atlasCell', 'u_atlasSize', 'u_defFg', 'u_defBg', 'u_cursorColor', 'u_selColor', 'u_matchColor',
      'u_cursor', 'u_focused',
    ]) {
      this.u[name] = gl.getUniformLocation(prog, name);
    }
    this.vao = gl.createVertexArray();
    this.vbo = gl.createBuffer();
    gl.bindVertexArray(this.vao);
    gl.bindBuffer(gl.ARRAY_BUFFER, this.vbo);
    gl.bufferData(gl.ARRAY_BUFFER, new Float32Array([0, 0, 1, 0, 0, 1, 1, 1]), gl.STATIC_DRAW);
    gl.enableVertexAttribArray(0);
    gl.vertexAttribPointer(0, 2, gl.FLOAT, false, 0, 0);
    gl.bindVertexArray(null);
    this.palette = gl.createTexture();
    if (this.palette) this.uploadPalette(gl, this.palette, this.theme);
    this.emptyU8 = this.intTexture(gl, gl.R8UI, gl.RED_INTEGER, gl.UNSIGNED_BYTE, 1, 1, new Uint8Array(1));
    this.emptyU32 = this.intTexture(gl, gl.RGBA32UI, gl.RGBA_INTEGER, gl.UNSIGNED_INT, 1, 1, new Uint32Array(4));
    this.atlas.bind(gl);
    this.gl = gl;
    return true;
  }

  private link(gl: WebGL2RenderingContext, vsSrc: string, fsSrc: string): WebGLProgram | null {
    const vs = gl.createShader(gl.VERTEX_SHADER);
    const fs = gl.createShader(gl.FRAGMENT_SHADER);
    if (!vs || !fs) return null;
    gl.shaderSource(vs, vsSrc);
    gl.compileShader(vs);
    gl.shaderSource(fs, fsSrc);
    gl.compileShader(fs);
    const prog = gl.createProgram();
    if (!prog) return null;
    gl.attachShader(prog, vs);
    gl.attachShader(prog, fs);
    gl.bindAttribLocation(prog, 0, 'a_pos');
    gl.linkProgram(prog);
    gl.deleteShader(vs);
    gl.deleteShader(fs);
    if (!gl.getProgramParameter(prog, gl.LINK_STATUS)) {
      console.error('[term] thumb shader link failed', gl.getProgramInfoLog(prog));
      gl.deleteProgram(prog);
      return null;
    }
    return prog;
  }

  private uploadPalette(gl: WebGL2RenderingContext, tex: WebGLTexture, theme: Theme): void {
    gl.activeTexture(gl.TEXTURE0 + UPLOAD_UNIT);
    const bytes = new Uint8Array(256 * 4);
    for (let i = 0; i < 256; i++) {
      const c = theme.palette[i];
      bytes[i * 4] = (c >>> 24) & 255;
      bytes[i * 4 + 1] = (c >>> 16) & 255;
      bytes[i * 4 + 2] = (c >>> 8) & 255;
      bytes[i * 4 + 3] = c & 255;
    }
    gl.bindTexture(gl.TEXTURE_2D, tex);
    gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA8, 256, 1, 0, gl.RGBA, gl.UNSIGNED_BYTE, bytes);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.NEAREST);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.NEAREST);
  }

  private intTexture(
    gl: WebGL2RenderingContext,
    internal: number,
    format: number,
    type: number,
    w: number,
    h: number,
    data: ArrayBufferView | null,
  ): WebGLTexture | null {
    gl.activeTexture(gl.TEXTURE0 + UPLOAD_UNIT);
    const tex = gl.createTexture();
    gl.bindTexture(gl.TEXTURE_2D, tex);
    gl.pixelStorei(gl.UNPACK_ALIGNMENT, 1);
    gl.texImage2D(gl.TEXTURE_2D, 0, internal, w, h, 0, format, type, data);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.NEAREST);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.NEAREST);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_S, gl.CLAMP_TO_EDGE);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_T, gl.CLAMP_TO_EDGE);
    return tex;
  }

  private color(c: number, slot: 'fg' | 'bg'): number {
    const kind = c >>> 24;
    if (kind === COLOR_PALETTE) return this.theme.palette[c & 255];
    if (kind === COLOR_RGB) return (((c & 0xffffff) << 8) | 255) >>> 0;
    if (kind === COLOR_SWAPPED) return pack(slot === 'fg' ? this.theme.bg : this.theme.fg);
    return 0;
  }

  private buildRow(g: GridGpu, y: number): void {
    const grid = g.grid;
    const cols = grid.cols;
    const glyphs = this.atlas.glyphs;
    let wideSlot = 0;
    let wideColor = 0;
    for (let x = 0; x < cols; x++) {
      const i = y * cols + x;
      const cp = grid.cp[i];
      const st = grid.style(grid.styleId[i]);
      const f = st.flags;
      let slot = 0;
      let a = 0;
      if (cp === 0) {
        if (wideSlot) {
          slot = wideSlot + 1;
          a |= wideColor;
        }
        wideSlot = 0;
      } else {
        const text = grid.cluster(cp);
        const wide = (f & SF_WIDE) !== 0;
        slot = glyphs.slot(text, (f & SF_BOLD) !== 0, (f & SF_ITALIC) !== 0, wide);
        const info = slot ? glyphs.info(slot) : null;
        if (info?.color) a |= TA_COLOR;
        wideSlot = wide ? slot : 0;
        wideColor = a & TA_COLOR;
      }
      if (f & SF_HIDDEN) slot = 0;
      let fg = this.color(st.fg, 'fg');
      const bg = this.color(st.bg, 'bg');
      if (f & SF_DIM) {
        const base = fg || pack(this.theme.fg);
        fg = ((base & 0xffffff00) | Math.round((base & 255) * 0.6)) >>> 0;
      }
      a |= (f & SF_UL_MASK) >> SF_UL_SHIFT;
      if (f & SF_STRIKE) a |= TA_STRIKE;
      if (f & SF_LINK) a |= TA_LINK;
      if (a & 7) {
        const ul = this.color(st.ul, 'fg');
        if (ul) a |= Math.max(0x010101, ul >>> 8) << 8;
      }
      if (grid.sel.length) {
        const k = grid.selKind(x, y);
        if (k === SEL_SELECTION) a |= TA_SEL;
        else if (k === SEL_MATCH) a |= TA_MATCH;
      }
      const o = i * 4;
      g.texels[o] = slot;
      g.texels[o + 1] = fg;
      g.texels[o + 2] = bg;
      g.texels[o + 3] = a >>> 0;
    }
  }

  private syncCells(gl: WebGL2RenderingContext, g: GridGpu, themeKey: string): void {
    gl.activeTexture(gl.TEXTURE0 + UPLOAD_UNIT);
    const grid = g.grid;
    if (grid.cols === 0 || grid.rows === 0) return;
    if (g.themeKey !== themeKey) {
      g.themeKey = themeKey;
      grid.markAll();
    }
    const glyphEpoch = this.atlas.glyphs.epoch;
    if (g.styleEpoch !== grid.styles.epoch || g.glyphEpoch !== glyphEpoch) {
      g.styleEpoch = grid.styles.epoch;
      g.glyphEpoch = glyphEpoch;
      grid.markAll();
    }
    const resized = grid.resized || g.cols !== grid.cols || g.rows !== grid.rows;
    if (!resized && !grid.anyDirty) return;
    if (resized) {
      g.cols = grid.cols;
      g.rows = grid.rows;
      g.texels = new Uint32Array(grid.cols * grid.rows * 4);
      grid.markAll();
    }
    g.rev++;
    for (let y = 0; y < grid.rows; y++) if (grid.dirty[y]) this.buildRow(g, y);
    if (this.atlas.glyphs.epoch !== glyphEpoch) {
      g.glyphEpoch = this.atlas.glyphs.epoch;
      for (let y = 0; y < grid.rows; y++) this.buildRow(g, y);
      grid.markAll();
    }
    gl.bindTexture(gl.TEXTURE_2D, g.cells);
    gl.pixelStorei(gl.UNPACK_ALIGNMENT, 4);
    if (resized) {
      gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA32UI, g.cols, g.rows, 0, gl.RGBA_INTEGER, gl.UNSIGNED_INT, g.texels);
    } else {
      let y = 0;
      while (y < grid.rows) {
        if (!grid.dirty[y]) {
          y++;
          continue;
        }
        let end = y;
        while (end + 1 < grid.rows && grid.dirty[end + 1]) end++;
        const sub = g.texels.subarray(y * g.cols * 4, (end + 1) * g.cols * 4);
        gl.texSubImage2D(gl.TEXTURE_2D, 0, 0, y, g.cols, end - y + 1, gl.RGBA_INTEGER, gl.UNSIGNED_INT, sub);
        y = end + 1;
      }
    }
    grid.dirty.fill(0);
    grid.anyDirty = false;
    grid.resized = false;
  }

  private syncMini(gl: WebGL2RenderingContext, g: GridGpu): void {
    gl.activeTexture(gl.TEXTURE0 + UPLOAD_UNIT);
    const grid = g.grid;
    if (!grid.miniDirty || grid.miniCols === 0 || grid.miniRows === 0) return;
    if (!g.mini || g.miniCols !== grid.miniCols || g.miniRows !== grid.miniRows) {
      if (g.mini) gl.deleteTexture(g.mini);
      g.mini = this.intTexture(gl, gl.R8UI, gl.RED_INTEGER, gl.UNSIGNED_BYTE, grid.miniCols, grid.miniRows, grid.mini);
      g.miniCols = grid.miniCols;
      g.miniRows = grid.miniRows;
    } else {
      gl.bindTexture(gl.TEXTURE_2D, g.mini);
      gl.pixelStorei(gl.UNPACK_ALIGNMENT, 1);
      gl.texSubImage2D(gl.TEXTURE_2D, 0, 0, 0, g.miniCols, g.miniRows, gl.RED_INTEGER, gl.UNSIGNED_BYTE, grid.mini);
    }
    grid.miniDirty = false;
  }

  private gpuFor(gl: WebGL2RenderingContext, grid: TermGrid, now: number): GridGpu | null {
    gl.activeTexture(gl.TEXTURE0 + UPLOAD_UNIT);
    let g = this.gpus.get(grid.id);
    if (g && g.grid !== grid) {
      this.freeGpu(gl, g);
      this.gpus.delete(grid.id);
      g = undefined;
    }
    if (!g) {
      const cells = gl.createTexture();
      if (!cells) return null;
      gl.bindTexture(gl.TEXTURE_2D, cells);
      gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.NEAREST);
      gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.NEAREST);
      gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_S, gl.CLAMP_TO_EDGE);
      gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_T, gl.CLAMP_TO_EDGE);
      g = {
        grid,
        rev: 0,
        thumb: null,
        cells,
        mini: null,
        cols: 0,
        rows: 0,
        texels: new Uint32Array(0),
        styleEpoch: -1,
        glyphEpoch: -1,
        themeKey: '',
        miniCols: 0,
        miniRows: 0,
        lastUsed: now,
      };
      grid.resized = true;
      grid.miniDirty = grid.miniCols > 0;
      this.gpus.set(grid.id, g);
    }
    g.lastUsed = now;
    return g;
  }

  private freeGpu(gl: WebGL2RenderingContext | null, g: GridGpu): void {
    if (!gl) return;
    gl.deleteTexture(g.cells);
    if (g.mini) gl.deleteTexture(g.mini);
    this.freeThumb(gl, g);
  }

  private freeThumb(gl: WebGL2RenderingContext, g: GridGpu): void {
    if (!g.thumb) return;
    gl.deleteFramebuffer(g.thumb.fbo);
    gl.deleteTexture(g.thumb.tex);
    g.thumb = null;
  }

  private ensureThumb(gl: WebGL2RenderingContext, g: GridGpu, w: number, h: number): Thumb | null {
    if (g.thumb && g.thumb.w === w && g.thumb.h === h) return g.thumb;
    this.freeThumb(gl, g);
    const tex = gl.createTexture();
    const fbo = gl.createFramebuffer();
    if (!tex || !fbo) return null;
    gl.activeTexture(gl.TEXTURE0 + UPLOAD_UNIT);
    gl.bindTexture(gl.TEXTURE_2D, tex);
    gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA8, w, h, 0, gl.RGBA, gl.UNSIGNED_BYTE, null);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.LINEAR_MIPMAP_LINEAR);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.LINEAR);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_S, gl.CLAMP_TO_EDGE);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_T, gl.CLAMP_TO_EDGE);
    gl.bindTexture(gl.TEXTURE_2D, null);
    gl.bindFramebuffer(gl.FRAMEBUFFER, fbo);
    gl.framebufferTexture2D(gl.FRAMEBUFFER, gl.COLOR_ATTACHMENT0, gl.TEXTURE_2D, tex, 0);
    const ok = gl.checkFramebufferStatus(gl.FRAMEBUFFER) === gl.FRAMEBUFFER_COMPLETE;
    gl.bindFramebuffer(gl.FRAMEBUFFER, null);
    if (!ok) {
      gl.deleteFramebuffer(fbo);
      gl.deleteTexture(tex);
      return null;
    }
    g.thumb = { tex, fbo, w, h, key: '' };
    return g.thumb;
  }

  private drawThumb(
    gl: WebGL2RenderingContext,
    g: GridGpu,
    themeKey: string,
    rect: { x: number; y: number; w: number; h: number },
    vw: number,
    vh: number,
  ): boolean {
    if (!this.thumbProgram) return false;
    gl.activeTexture(gl.TEXTURE0 + UPLOAD_UNIT);
    const picked = this.atlas.pick(THUMB_FONT_PX, true);
    if (!picked || !picked.bucket.texture) return false;
    const b = picked.bucket;
    const ready = picked.exact && b.ready(this.atlas.glyphs.count);
    const grid = g.grid;
    const cur = grid.cursor;
    const curOn = (cur.flags & CF_VISIBLE) !== 0 && cur.shape !== CURSOR_HIDDEN;
    const key = `${g.rev}|${themeKey}|${b.index}|${cur.x},${cur.y},${cur.shape},${curOn ? 1 : 0}`;
    const max = Math.min(4096, gl.getParameter(gl.MAX_TEXTURE_SIZE) as number);
    const tw = Math.max(1, Math.min(max, g.cols * b.cw));
    const th = Math.max(1, Math.min(max, g.rows * b.ch));
    let t = g.thumb && g.thumb.w === tw && g.thumb.h === th ? g.thumb : null;
    if (!t || t.key !== key) {
      if (this.thumbBudget <= 0) {
        this.thumbStale = true;
        if (!t) return false;
      } else {
        this.thumbBudget--;
        t = this.ensureThumb(gl, g, tw, th);
        if (!t) return false;
        gl.bindFramebuffer(gl.FRAMEBUFFER, t.fbo);
        gl.viewport(0, 0, tw, th);
        gl.disable(gl.SCISSOR_TEST);
        const bg = this.theme.bg;
        gl.clearColor(bg[0], bg[1], bg[2], 1);
        gl.clear(gl.COLOR_BUFFER_BIT);
        gl.uniform2f(this.u.u_viewport, tw, th);
        gl.uniform4f(this.u.u_rect, 0, 0, tw, th);
        gl.uniform1i(this.u.u_mode, 0);
        gl.uniform2f(this.u.u_cell, b.cw, b.ch);
        gl.uniform2i(this.u.u_grid, g.cols, g.rows);
        gl.uniform3f(this.u.u_atlasCell, b.cw, b.ch, b.perRow);
        gl.uniform2f(this.u.u_atlasSize, b.texW, b.texH);
        gl.uniform4f(this.u.u_cursor, cur.x, cur.y, cur.shape, curOn ? 1 : 0);
        gl.uniform1f(this.u.u_focused, 0);
        gl.activeTexture(gl.TEXTURE0);
        gl.bindTexture(gl.TEXTURE_2D, g.cells);
        gl.activeTexture(gl.TEXTURE1);
        gl.bindTexture(gl.TEXTURE_2D, this.emptyU8);
        gl.activeTexture(gl.TEXTURE2);
        gl.bindTexture(gl.TEXTURE_2D, b.texture);
        gl.drawArrays(gl.TRIANGLE_STRIP, 0, 4);
        gl.bindFramebuffer(gl.FRAMEBUFFER, null);
        gl.viewport(0, 0, gl.drawingBufferWidth, gl.drawingBufferHeight);
        gl.uniform2f(this.u.u_viewport, vw, vh);
        gl.enable(gl.SCISSOR_TEST);
        gl.activeTexture(gl.TEXTURE0 + UPLOAD_UNIT);
        gl.bindTexture(gl.TEXTURE_2D, t.tex);
        gl.generateMipmap(gl.TEXTURE_2D);
        gl.bindTexture(gl.TEXTURE_2D, null);
        t.key = ready ? key : '';
        if (!ready) this.thumbStale = true;
      }
    }
    gl.useProgram(this.thumbProgram);
    gl.uniform2f(this.thumbU.u_viewport, vw, vh);
    gl.uniform4f(this.thumbU.u_rect, rect.x, rect.y, rect.w, rect.h);
    gl.uniform1i(this.thumbU.u_tex, THUMB_UNIT);
    gl.activeTexture(gl.TEXTURE0 + THUMB_UNIT);
    gl.bindTexture(gl.TEXTURE_2D, t.tex);
    gl.drawArrays(gl.TRIANGLE_STRIP, 0, 4);
    gl.bindTexture(gl.TEXTURE_2D, null);
    gl.useProgram(this.program);
    return true;
  }

  private begin(gl: WebGL2RenderingContext, vw: number, vh: number): void {
    gl.useProgram(this.program);
    gl.bindVertexArray(this.vao);
    gl.enable(gl.BLEND);
    gl.blendFunc(gl.ONE, gl.ONE_MINUS_SRC_ALPHA);
    gl.enable(gl.SCISSOR_TEST);
    gl.uniform2f(this.u.u_viewport, vw, vh);
    gl.uniform4fv(this.u.u_defFg, this.theme.fg);
    gl.uniform4fv(this.u.u_defBg, this.theme.bg);
    gl.uniform4fv(this.u.u_cursorColor, this.theme.cursor);
    gl.uniform4fv(this.u.u_selColor, this.theme.sel);
    gl.uniform4fv(this.u.u_matchColor, this.theme.match);
    gl.uniform1i(this.u.u_cells, 0);
    gl.uniform1i(this.u.u_mini, 1);
    gl.uniform1i(this.u.u_atlas, 2);
    gl.uniform1i(this.u.u_palette, 3);
    gl.activeTexture(gl.TEXTURE3);
    gl.bindTexture(gl.TEXTURE_2D, this.palette);
  }

  drawScreen(gl: WebGL2RenderingContext, frame: FrameInfo): void {
    const list = (frame.screenNodes ?? []).filter((n) => n.screen && isTerminalKind(n.kind));
    if (!list.length || gl.isContextLost()) return;
    if (!this.init(gl) || !this.program) return;
    const dpr = frame.viewport.dpr || 1;
    const vw = Math.round(frame.viewport.w * dpr);
    const vh = Math.round(frame.viewport.h * dpr);
    const now = typeof performance !== 'undefined' ? performance.now() : frame.time;
    const m = cellMetrics();
    const scale = dpr;
    const fontDevice = TERM_FONT_SIZE * scale;
    const focus = focusedTerm();
    let blinking = false;
    this.begin(gl, vw, vh);
    for (const node of list) {
      const s = node.screen!;
      const termId = terminalIdOf(node);
      const nx = Math.round(s.x * dpr);
      const ny = Math.round(s.y * dpr);
      const nw = Math.round(s.w * dpr);
      const nh = Math.round(s.h * dpr);
      if (nw <= 0 || nh <= 0) continue;
      const cx = Math.max(0, nx);
      const cy = Math.max(0, ny);
      const cw = Math.min(vw, nx + nw) - cx;
      const ch = Math.min(vh, ny + nh) - cy;
      if (cw <= 0 || ch <= 0) continue;
      const th = this.themeFor(gl, themeIdOf(node));
      this.useTheme(gl, th);
      const fs = termFontScale(node);
      const grid = termHub.grid(termId);
      this.fillBody(gl, grid, nx, ny, nw, nh, vw, vh);
      if (!grid) continue;
      const body = bodyRect(s);
      const bx = Math.round(body.x * dpr);
      const by = Math.round(body.y * dpr);
      const bw = Math.round(body.w * dpr);
      const bh = Math.round(body.h * dpr);
      if (bw <= 0 || bh <= 0) continue;
      const g = this.gpuFor(gl, grid, now);
      if (!g) continue;
      this.syncCells(gl, g, themeIdOf(node));
      if (g.cols === 0) continue;
      gl.activeTexture(gl.TEXTURE0 + UPLOAD_UNIT);
      const picked = this.atlas.pick(fontDevice * fs, true);
      if (!picked || !picked.bucket.texture) continue;
      const b = picked.bucket;
      const cellW = picked.exact ? b.cw : m.w * scale * fs;
      const cellH = picked.exact ? b.ch : m.h * scale * fs;
      const sx = Math.max(0, bx);
      const sy = Math.max(0, by);
      gl.scissor(sx, Math.max(0, vh - (by + bh)), Math.min(vw, bx + bw) - sx, Math.min(vh, by + bh) - sy);
      gl.uniform4f(this.u.u_rect, bx, by, bw, bh);
      const focused = termId === focus;
      const cur = grid.cursor;
      let visible = (cur.flags & CF_VISIBLE) !== 0 && cur.shape !== CURSOR_HIDDEN;
      if (visible && focused && cur.flags & CF_BLINK) {
        blinking = true;
        visible = Math.floor(now / BLINK_MS) % 2 === 0;
      }
      gl.uniform1i(this.u.u_mode, 0);
      gl.uniform2f(this.u.u_cell, cellW, cellH);
      gl.uniform2i(this.u.u_grid, g.cols, g.rows);
      gl.uniform3f(this.u.u_atlasCell, b.cw, b.ch, b.perRow);
      gl.uniform2f(this.u.u_atlasSize, b.texW, b.texH);
      gl.uniform4f(this.u.u_cursor, cur.x, cur.y, cur.shape, visible ? 1 : 0);
      gl.uniform1f(this.u.u_focused, focused ? 1 : 0);
      gl.activeTexture(gl.TEXTURE0);
      gl.bindTexture(gl.TEXTURE_2D, g.cells);
      gl.activeTexture(gl.TEXTURE1);
      gl.bindTexture(gl.TEXTURE_2D, this.emptyU8);
      gl.activeTexture(gl.TEXTURE2);
      gl.bindTexture(gl.TEXTURE_2D, b.texture);
      gl.drawArrays(gl.TRIANGLE_STRIP, 0, 4);
      layouts.set(termId, { x: bx / dpr, y: by / dpr, cellW: cellW / dpr, cellH: cellH / dpr, cols: g.cols, rows: g.rows, zoom: 1 });
    }
    gl.disable(gl.SCISSOR_TEST);
    gl.bindVertexArray(null);
    gl.activeTexture(gl.TEXTURE0);
    gl.activeTexture(gl.TEXTURE0 + UPLOAD_UNIT);
    if (this.atlas.work()) this.requestRedraw();
    gl.activeTexture(gl.TEXTURE0);
    if (blinking && !this.blinkTimer) {
      this.blinkTimer = setTimeout(() => {
        this.blinkTimer = null;
        this.requestRedraw();
      }, BLINK_MS - (now % BLINK_MS) + 5);
    }
  }

  draw(gl: WebGL2RenderingContext, frame: FrameInfo): void {
    if (gl.isContextLost()) return;
    const nodes = updateLods(frame);
    if (!this.init(gl) || !this.program) return;
    const { camera, viewport } = frame;
    const dpr = viewport.dpr || 1;
    const vw = Math.round(viewport.w * dpr);
    const vh = Math.round(viewport.h * dpr);
    const now = typeof performance !== 'undefined' ? performance.now() : frame.time;

    const scale = camera.zoom * dpr;
    if (scale !== this.lastScale) {
      this.lastScale = scale;
      this.scaleChangedAt = now;
      if (this.settleTimer) clearTimeout(this.settleTimer);
      this.settleTimer = setTimeout(() => {
        this.settleTimer = null;
        this.requestRedraw();
      }, SETTLE_MS + 10);
    }
    const settled = now - this.scaleChangedAt >= SETTLE_MS;
    const m = cellMetrics();
    const fontDevice = TERM_FONT_SIZE * scale;
    const focus = focusedTerm();
    const faithful = faithfulRendering();
    let blinking = false;
    this.thumbBudget = THUMB_RENDERS_PER_FRAME;
    this.thumbStale = false;

    this.begin(gl, vw, vh);

    const drawn = new Set<string>();
    for (const { node, termId, lod } of nodes) {
      if (lod === 'card') continue;
      const th = this.themeFor(gl, themeIdOf(node));
      this.useTheme(gl, th);
      const fs = termFontScale(node);
      const grid = termHub.grid(termId);
      this.fillBody(
        gl,
        grid,
        Math.round((node.x - camera.x) * camera.zoom * dpr),
        Math.round((node.y + TERM_HEADER_H - camera.y) * camera.zoom * dpr),
        Math.round(node.w * camera.zoom * dpr),
        Math.round((node.h - TERM_HEADER_H) * camera.zoom * dpr),
        vw,
        vh,
      );
      if (!grid) continue;
      const body = bodyRect(node);
      const bx = Math.round((body.x - camera.x) * camera.zoom * dpr);
      const by = Math.round((body.y - camera.y) * camera.zoom * dpr);
      const bw = Math.round(body.w * camera.zoom * dpr);
      const bh = Math.round(body.h * camera.zoom * dpr);
      if (bw <= 0 || bh <= 0 || bx >= vw || by >= vh || bx + bw <= 0 || by + bh <= 0) continue;
      const g = this.gpuFor(gl, grid, now);
      if (!g) continue;
      drawn.add(termId);

      const sx = Math.max(0, bx);
      const sy = Math.max(0, by);
      gl.scissor(sx, Math.max(0, vh - (by + bh)), Math.min(vw, bx + bw) - sx, Math.min(vh, by + bh) - sy);
      gl.uniform4f(this.u.u_rect, bx, by, bw, bh);

      if (lod === 'mini' && !(faithful && grid.cols > 0 && grid.rows > 0)) {
        this.syncMini(gl, g);
        if (!g.mini) continue;
        gl.uniform1i(this.u.u_mode, 1);
        gl.uniform2f(this.u.u_cell, m.w * scale * fs, m.h * scale * fs);
        gl.uniform2i(this.u.u_grid, g.miniCols, g.miniRows);
        gl.uniform4f(this.u.u_cursor, 0, 0, 0, 0);
        gl.activeTexture(gl.TEXTURE0);
        gl.bindTexture(gl.TEXTURE_2D, this.emptyU32);
        gl.activeTexture(gl.TEXTURE1);
        gl.bindTexture(gl.TEXTURE_2D, g.mini);
        gl.drawArrays(gl.TRIANGLE_STRIP, 0, 4);
        layouts.set(termId, {
          x: bx / dpr,
          y: by / dpr,
          cellW: (m.w * scale * fs) / dpr,
          cellH: (m.h * scale * fs) / dpr,
          cols: g.miniCols,
          rows: g.miniRows,
          zoom: camera.zoom,
        });
        continue;
      }

      this.syncCells(gl, g, themeIdOf(node));
      if (g.cols === 0) continue;
      if (faithful && termId !== focus && fontDevice * fs < THUMB_BELOW_PX) {
        const cw = m.w * scale * fs;
        const ch = m.h * scale * fs;
        if (this.drawThumb(gl, g, themeIdOf(node), { x: bx, y: by, w: g.cols * cw, h: g.rows * ch }, vw, vh)) {
          layouts.set(termId, { x: bx / dpr, y: by / dpr, cellW: cw / dpr, cellH: ch / dpr, cols: g.cols, rows: g.rows, zoom: camera.zoom });
          continue;
        }
      }
      gl.activeTexture(gl.TEXTURE0 + UPLOAD_UNIT);
      const picked = this.atlas.pick(fontDevice * fs, settled);
      if (!picked || !picked.bucket.texture) continue;
      const b = picked.bucket;
      const cellW = picked.exact && settled ? b.cw : m.w * scale * fs;
      const cellH = picked.exact && settled ? b.ch : m.h * scale * fs;

      const focused = termId === focus;
      const cur = grid.cursor;
      let visible = (cur.flags & CF_VISIBLE) !== 0 && cur.shape !== CURSOR_HIDDEN;
      if (visible && focused && cur.flags & CF_BLINK) {
        blinking = true;
        visible = Math.floor(now / BLINK_MS) % 2 === 0;
      }
      gl.uniform1i(this.u.u_mode, 0);
      gl.uniform2f(this.u.u_cell, cellW, cellH);
      gl.uniform2i(this.u.u_grid, g.cols, g.rows);
      gl.uniform3f(this.u.u_atlasCell, b.cw, b.ch, b.perRow);
      gl.uniform2f(this.u.u_atlasSize, b.texW, b.texH);
      gl.uniform4f(this.u.u_cursor, cur.x, cur.y, cur.shape, visible ? 1 : 0);
      gl.uniform1f(this.u.u_focused, focused ? 1 : 0);
      gl.activeTexture(gl.TEXTURE0);
      gl.bindTexture(gl.TEXTURE_2D, g.cells);
      gl.activeTexture(gl.TEXTURE1);
      gl.bindTexture(gl.TEXTURE_2D, this.emptyU8);
      gl.activeTexture(gl.TEXTURE2);
      gl.bindTexture(gl.TEXTURE_2D, b.texture);
      gl.drawArrays(gl.TRIANGLE_STRIP, 0, 4);
      layouts.set(termId, {
        x: bx / dpr,
        y: by / dpr,
        cellW: cellW / dpr,
        cellH: cellH / dpr,
        cols: g.cols,
        rows: g.rows,
        zoom: camera.zoom,
      });
    }

    gl.disable(gl.SCISSOR_TEST);
    gl.bindVertexArray(null);
    gl.activeTexture(gl.TEXTURE0);

    for (const id of layouts.keys()) if (!drawn.has(id)) layouts.delete(id);
    this.evict(gl, now);

    gl.activeTexture(gl.TEXTURE0 + UPLOAD_UNIT);
    this.workPending = this.atlas.work();
    gl.activeTexture(gl.TEXTURE0);
    if (this.workPending || this.thumbStale) this.requestRedraw();
    if (blinking && !this.blinkTimer) {
      this.blinkTimer = setTimeout(() => {
        this.blinkTimer = null;
        this.requestRedraw();
      }, BLINK_MS - (now % BLINK_MS) + 5);
    }
    termHub.afterDraw();
  }

  private evict(gl: WebGL2RenderingContext, now: number): void {
    for (const [id, g] of this.gpus) {
      if (now - g.lastUsed > 10_000 || !termHub.grid(id)) {
        this.freeGpu(gl, g);
        this.gpus.delete(id);
      }
    }
  }

  private dropGl(): void {
    const gl = this.gl;
    if (gl && !gl.isContextLost()) {
      for (const g of this.gpus.values()) this.freeGpu(gl, g);
      if (this.program) gl.deleteProgram(this.program);
      if (this.thumbProgram) gl.deleteProgram(this.thumbProgram);
      if (this.vao) gl.deleteVertexArray(this.vao);
      if (this.vbo) gl.deleteBuffer(this.vbo);
      if (this.palette) gl.deleteTexture(this.palette);
      if (this.emptyU8) gl.deleteTexture(this.emptyU8);
      if (this.emptyU32) gl.deleteTexture(this.emptyU32);
      for (const t of this.themes.values()) if (t.palette) gl.deleteTexture(t.palette);
    }
    this.themes.clear();
    for (const g of this.gpus.values()) g.grid.resized = true;
    this.gpus.clear();
    this.atlas.dropGl();
    this.program = null;
    this.thumbProgram = null;
    this.vao = null;
    this.vbo = null;
    this.palette = null;
    this.emptyU8 = null;
    this.emptyU32 = null;
    this.gl = null;
  }

  dispose(): void {
    if (this.settleTimer) clearTimeout(this.settleTimer);
    if (this.blinkTimer) clearTimeout(this.blinkTimer);
    this.settleTimer = null;
    this.blinkTimer = null;
    this.dropGl();
    layouts.clear();
  }
}

let shared: TermPass | null = null;
let unregister: (() => void) | null = null;

export function registerTermPass(invalidate: () => void): () => void {
  const pass = termPass();
  pass.requestRedraw = invalidate;
  const pollThemes = () => void loadUserSchemes().then((changed) => changed && pass.refreshTheme());
  pollThemes();
  const themeTimer = setInterval(pollThemes, 5000);
  unregister?.();
  const stopMain = registerPass(pass);
  const stopLift = registerPass({ id: 'terminals-lifted', order: 90, draw: (gl, frame) => pass.drawScreen(gl, frame) });
  unregister = () => {
    stopMain();
    stopLift();
  };
  return () => {
    clearInterval(themeTimer);
    unregister?.();
    unregister = null;
    pass.requestRedraw = () => {};
    void termHub.stop();
  };
}

export function termPass(): TermPass {
  if (!shared) shared = new TermPass();
  return shared;
}

let glSupport: boolean | null = null;

export function termGlAvailable(): boolean {
  if (glSupport !== null) return glSupport;
  try {
    const c = typeof document !== 'undefined' ? document.createElement('canvas') : null;
    const gl = c?.getContext('webgl2') ?? null;
    glSupport = !!gl;
    gl?.getExtension('WEBGL_lose_context')?.loseContext();
  } catch {
    glSupport = false;
  }
  return glSupport;
}

export function setTermGlAvailable(ok: boolean): void {
  glSupport = ok;
}

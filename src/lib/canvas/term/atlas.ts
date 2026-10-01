import { Lru } from '$lib/gfx/text';
import { cellMetrics, TERM_FONT_SIZE } from './metrics';

const MAX_SLOTS = 8192;
const RASTER_BUDGET = 192;
const MIN_FONT_PX = 6;
const MAX_FONT_PX = 72;
const SLOT_CHUNK = 512;
const LIVE_BUCKETS = 5;
const ATLAS_WIDTH = 2048;

interface GlyphInfo {
  text: string;
  bold: boolean;
  italic: boolean;
  wide: boolean;
  color: boolean;
}

const EMOJI_RE = /\p{Extended_Pictographic}|\u{FE0F}/u;

function isColorGlyph(text: string): boolean {
  return EMOJI_RE.test(text);
}

function bucketIndex(fontPx: number): number {
  const px = Math.min(MAX_FONT_PX, Math.max(MIN_FONT_PX, fontPx));
  return Math.round(8 * Math.log2(px));
}

function bucketFontPx(index: number): number {
  return Math.pow(2, index / 8);
}

type Ctx2D = CanvasRenderingContext2D | OffscreenCanvasRenderingContext2D;

class GlyphBucket {
  readonly fontPx: number;
  readonly cw: number;
  readonly ch: number;
  readonly perRow: number;
  texture: WebGLTexture | null = null;
  texW = ATLAS_WIDTH;
  texH = 0;
  capacity = 0;
  done = new Uint8Array(0);
  doneUpTo = 1;
  private ctx: Ctx2D | null = null;

  constructor(readonly index: number) {
    const m = cellMetrics();
    this.fontPx = bucketFontPx(index);
    const scale = this.fontPx / TERM_FONT_SIZE;
    this.cw = Math.max(2, Math.ceil(m.w * scale));
    this.ch = Math.max(2, Math.ceil(m.h * scale));
    this.perRow = Math.max(1, Math.floor(ATLAS_WIDTH / this.cw));
  }

  slotOrigin(slot: number): [number, number] {
    return [(slot % this.perRow) * this.cw, Math.floor(slot / this.perRow) * this.ch];
  }

  ready(count: number): boolean {
    return this.texture !== null && this.doneUpTo >= count;
  }

  private ensureCtx(): Ctx2D | null {
    if (this.ctx) return this.ctx;
    const w = this.cw * 2;
    const h = this.ch;
    let ctx: Ctx2D | null = null;
    if (typeof OffscreenCanvas !== 'undefined') {
      ctx = new OffscreenCanvas(w, h).getContext('2d', { willReadFrequently: true }) as OffscreenCanvasRenderingContext2D | null;
    } else if (typeof document !== 'undefined') {
      const c = document.createElement('canvas');
      c.width = w;
      c.height = h;
      ctx = c.getContext('2d', { willReadFrequently: true });
    }
    this.ctx = ctx;
    return ctx;
  }

  private ensureTexture(gl: WebGL2RenderingContext, needed: number): void {
    if (this.texture && needed <= this.capacity) return;
    const capacity = Math.min(MAX_SLOTS, Math.ceil(Math.max(needed, 1) / SLOT_CHUNK) * SLOT_CHUNK);
    const rows = Math.ceil(capacity / this.perRow);
    const maxTex = gl.getParameter(gl.MAX_TEXTURE_SIZE) as number;
    this.texH = Math.min(maxTex, rows * this.ch);
    this.capacity = Math.min(capacity, Math.floor(this.texH / this.ch) * this.perRow);
    if (this.texture) gl.deleteTexture(this.texture);
    const tex = gl.createTexture();
    gl.bindTexture(gl.TEXTURE_2D, tex);
    gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA8, this.texW, this.texH, 0, gl.RGBA, gl.UNSIGNED_BYTE, null);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.LINEAR);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.LINEAR);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_S, gl.CLAMP_TO_EDGE);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_T, gl.CLAMP_TO_EDGE);
    this.texture = tex;
    this.done = new Uint8Array(this.capacity);
    this.done[0] = 1;
    this.doneUpTo = 1;
  }

  fill(gl: WebGL2RenderingContext, glyphs: GlyphTable, budget: number): number {
    const count = glyphs.count;
    this.ensureTexture(gl, count);
    if (this.doneUpTo >= count || budget <= 0) return 0;
    const ctx = this.ensureCtx();
    if (!ctx) return 0;
    const m = cellMetrics();
    gl.bindTexture(gl.TEXTURE_2D, this.texture);
    gl.pixelStorei(gl.UNPACK_PREMULTIPLY_ALPHA_WEBGL, false);
    let spent = 0;
    let slot = this.doneUpTo;
    const limit = Math.min(count, this.capacity);
    while (slot < limit && spent < budget) {
      if (this.done[slot]) {
        slot++;
        continue;
      }
      const g = glyphs.info(slot);
      if (!g) {
        this.done[slot] = 1;
        slot++;
        continue;
      }
      const w = g.wide ? this.cw * 2 : this.cw;
      ctx.clearRect(0, 0, this.cw * 2, this.ch);
      ctx.font = `${g.italic ? 'italic ' : ''}${g.bold ? 'bold ' : ''}${this.fontPx}px ${m.font}`;
      ctx.fillStyle = '#ffffff';
      ctx.textBaseline = 'alphabetic';
      ctx.save();
      ctx.beginPath();
      ctx.rect(0, 0, w, this.ch);
      ctx.clip();
      ctx.fillText(g.text, 0, Math.round(m.ascent * this.ch));
      ctx.restore();
      const halves = g.wide ? 2 : 1;
      for (let h = 0; h < halves && slot + h < limit; h++) {
        const img = ctx.getImageData(h * this.cw, 0, this.cw, this.ch);
        const [ox, oy] = this.slotOrigin(slot + h);
        gl.texSubImage2D(gl.TEXTURE_2D, 0, ox, oy, this.cw, this.ch, gl.RGBA, gl.UNSIGNED_BYTE, img.data);
        this.done[slot + h] = 1;
      }
      slot += halves;
      spent++;
    }
    while (this.doneUpTo < limit && this.done[this.doneUpTo]) this.doneUpTo++;
    return spent;
  }

  dispose(gl: WebGL2RenderingContext | null): void {
    if (gl && this.texture) gl.deleteTexture(this.texture);
    this.texture = null;
    this.capacity = 0;
    this.doneUpTo = 1;
  }
}

class GlyphTable {
  private slots = new Map<string, number>();
  private infos: (GlyphInfo | null)[] = [null];
  epoch = 0;

  get count(): number {
    return this.infos.length;
  }

  info(slot: number): GlyphInfo | null {
    return this.infos[slot] ?? null;
  }

  slot(text: string, bold: boolean, italic: boolean, wide: boolean): number {
    if (!text || text === ' ') return 0;
    const key = `${bold ? 'b' : ''}${italic ? 'i' : ''}${wide ? 'w' : ''}|${text}`;
    const hit = this.slots.get(key);
    if (hit !== undefined) return hit;
    const need = wide ? 2 : 1;
    if (this.infos.length + need > MAX_SLOTS) this.reset();
    const slot = this.infos.length;
    this.infos.push({ text, bold, italic, wide, color: isColorGlyph(text) });
    if (wide) this.infos.push(null);
    this.slots.set(key, slot);
    return slot;
  }

  reset(): void {
    this.slots.clear();
    this.infos = [null];
    this.epoch++;
  }
}

export class TermAtlas {
  readonly glyphs = new GlyphTable();
  private gl: WebGL2RenderingContext | null = null;
  private buckets: Lru<GlyphBucket>;
  private lastEpoch = 0;

  constructor() {
    this.buckets = new Lru<GlyphBucket>(LIVE_BUCKETS, (_k, b) => b.dispose(this.gl));
  }

  bind(gl: WebGL2RenderingContext): void {
    if (this.gl !== gl) {
      this.dropGl();
      this.gl = gl;
    }
  }

  private bucket(index: number): GlyphBucket {
    const key = String(index);
    let b = this.buckets.get(key);
    if (!b) {
      b = new GlyphBucket(index);
      this.buckets.set(key, b);
    }
    return b;
  }

  pick(fontPx: number, settled: boolean): { bucket: GlyphBucket; exact: boolean } | null {
    const gl = this.gl;
    if (!gl) return null;
    if (this.glyphs.epoch !== this.lastEpoch) {
      this.lastEpoch = this.glyphs.epoch;
      for (const k of this.buckets.keys()) this.buckets.get(k)?.dispose(gl);
    }
    const wantIdx = bucketIndex(fontPx);
    const count = this.glyphs.count;
    const existing = this.buckets.get(String(wantIdx));
    if (existing?.ready(count)) return { bucket: existing, exact: true };
    let best: GlyphBucket | null = null;
    for (const k of this.buckets.keys()) {
      const b = this.buckets.get(k);
      if (!b || b.index === wantIdx || !b.texture) continue;
      if (!best || Math.abs(b.index - wantIdx) < Math.abs(best.index - wantIdx)) best = b;
    }
    if (!settled && best) {
      this.moving.add(best.index);
      return { bucket: best, exact: false };
    }
    const want = existing ?? this.bucket(wantIdx);
    this.wanted.add(wantIdx);
    if (best?.ready(count)) return { bucket: best, exact: false };
    return { bucket: want, exact: true };
  }

  private wanted = new Set<number>();
  private moving = new Set<number>();

  work(): boolean {
    const gl = this.gl;
    if (!gl) return false;
    let budget = RASTER_BUDGET;
    for (const idx of [...this.wanted, ...this.moving]) {
      const b = this.buckets.get(String(idx));
      if (b) budget -= b.fill(gl, this.glyphs, budget);
    }
    this.wanted.clear();
    this.moving.clear();
    for (const k of this.buckets.keys()) {
      const b = this.buckets.get(k);
      if (b && b.texture && b.doneUpTo < this.glyphs.count) {
        if (budget > 0) budget -= b.fill(gl, this.glyphs, budget);
        if (b.doneUpTo < Math.min(this.glyphs.count, b.capacity)) return true;
      }
    }
    return false;
  }

  dropGl(): void {
    for (const k of this.buckets.keys()) this.buckets.get(k)?.dispose(this.gl);
    this.buckets.clear();
    this.gl = null;
  }
}

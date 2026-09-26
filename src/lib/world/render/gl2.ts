// WebGL backend. One code path for WebGL2 (floor) and WebGL1 (fallback); `gl1.ts`
// is the thin wrapper that asks for version 1. One VBO, one draw call per atlas page
// run, one draw call per visible chunk, chunks baked into a render texture via FBO.

import { parseAtlas, type AtlasData } from './atlas';
import {
  buildIndices,
  buildSpriteBatches,
  cullSprites,
  growScratch,
  writeQuad,
  FLOATS_PER_QUAD,
  INDICES_PER_QUAD,
  type PageSize,
} from './batcher';
import { chunkLayout, chunkTilePx, sortTiles, ChunkStore } from './chunk';
import { parseChunkId, worldToScreen } from './camera';
import { ContextTracker, waitForRestore } from './context';
import { TextAtlas, textUV } from './text';
import { budgetForTier } from './tier';
import {
  CANVAS_REPLACED_EVENT,
  CHUNK_TILES,
  ERR,
  RenderError,
  type AtlasHandle,
  type Caps,
  type ChunkId,
  type ChunkTile,
  type DynamicFrame,
  type FrameStats,
  type InitOpts,
  type Renderer,
  type Scene,
  type TextHandle,
  type TextStyle,
  type Tier,
} from './types';

type GL = WebGLRenderingContext | WebGL2RenderingContext;

const VS2 = `#version 300 es
in vec2 aPos; in vec2 aUV; in vec4 aColor;
uniform vec2 uRes;
out vec2 vUV; out vec4 vColor;
void main() {
  vec2 p = aPos / uRes * 2.0 - 1.0;
  gl_Position = vec4(p.x, -p.y, 0.0, 1.0);
  vUV = aUV; vColor = aColor;
}`;

const FS2 = `#version 300 es
precision mediump float;
in vec2 vUV; in vec4 vColor;
uniform sampler2D uTex;
out vec4 outColor;
void main() {
  // Premultiplied throughout: the texture is uploaded premultiplied, the tint
  // scales the colour and the alpha scales everything.
  vec4 t = texture(uTex, vUV);
  vec4 c = vec4(t.rgb * vColor.rgb, t.a) * vColor.a;
  if (c.a < 0.004) discard;
  outColor = c;
}`;

const VS1 = `attribute vec2 aPos; attribute vec2 aUV; attribute vec4 aColor;
uniform vec2 uRes;
varying vec2 vUV; varying vec4 vColor;
void main() {
  vec2 p = aPos / uRes * 2.0 - 1.0;
  gl_Position = vec4(p.x, -p.y, 0.0, 1.0);
  vUV = aUV; vColor = aColor;
}`;

const FS1 = `precision mediump float;
varying vec2 vUV; varying vec4 vColor;
uniform sampler2D uTex;
void main() {
  vec4 t = texture2D(uTex, vUV);
  vec4 c = vec4(t.rgb * vColor.rgb, t.a) * vColor.a;
  if (c.a < 0.004) discard;
  gl_FragColor = c;
}`;

export const GL_ATTRIBUTES: WebGLContextAttributes = {
  alpha: true,
  antialias: false,
  depth: false,
  stencil: false,
  // Premultiplied end to end (textures, blending, chunk bakes, the canvas).
  // Straight alpha here double-darkened every anti-aliased edge, because the
  // webview hands ImageBitmaps over already premultiplied (measured, C01
  // acceptance, session 06).
  premultipliedAlpha: true,
  preserveDrawingBuffer: false,
  powerPreference: 'high-performance',
  // Left false on purpose: llvmpipe would be rejected on Linux, where it is the
  // only option. The calibration decides the tier by measuring instead (§1.3).
  failIfMajorPerformanceCaveat: false,
};

export function getGlContext(canvas: HTMLCanvasElement, version: 1 | 2): GL | null {
  if (version === 2) {
    return canvas.getContext('webgl2', GL_ATTRIBUTES) as WebGL2RenderingContext | null;
  }
  return (canvas.getContext('webgl', GL_ATTRIBUTES) ||
    canvas.getContext('experimental-webgl', GL_ATTRIBUTES)) as WebGLRenderingContext | null;
}

function compile(gl: GL, type: number, src: string): WebGLShader {
  const sh = gl.createShader(type);
  if (!sh) throw new RenderError(ERR.SHADER, 'createShader failed');
  gl.shaderSource(sh, src);
  gl.compileShader(sh);
  if (!gl.getShaderParameter(sh, gl.COMPILE_STATUS)) {
    const log = gl.getShaderInfoLog(sh) ?? '';
    gl.deleteShader(sh);
    throw new RenderError(ERR.SHADER, `shader: ${log}`);
  }
  return sh;
}

export function buildProgram(gl: GL, version: 1 | 2): WebGLProgram {
  const vs = compile(gl, gl.VERTEX_SHADER, version === 2 ? VS2 : VS1);
  const fs = compile(gl, gl.FRAGMENT_SHADER, version === 2 ? FS2 : FS1);
  const prog = gl.createProgram();
  if (!prog) throw new RenderError(ERR.SHADER, 'createProgram failed');
  gl.attachShader(prog, vs);
  gl.attachShader(prog, fs);
  gl.linkProgram(prog);
  gl.deleteShader(vs);
  gl.deleteShader(fs);
  if (!gl.getProgramParameter(prog, gl.LINK_STATUS)) {
    const log = gl.getProgramInfoLog(prog) ?? '';
    gl.deleteProgram(prog);
    throw new RenderError(ERR.SHADER, `link: ${log}`);
  }
  return prog;
}

/**
 * Swaps a canvas whose context refused to come back for a fresh element in the
 * same DOM position, copying size and presentation. Returns null when the canvas
 * is detached, in which case the caller has to rebuild the renderer itself.
 */
export function replaceCanvas(old: HTMLCanvasElement): HTMLCanvasElement | null {
  const parent = old.parentNode;
  if (!parent || typeof document === 'undefined') return null;
  const next = document.createElement('canvas');
  // Every attribute (class, tabindex, aria-label, style, id…), so focus and
  // assistive tech see the same element they saw before.
  for (const a of Array.from(old.attributes)) next.setAttribute(a.name, a.value);
  next.width = old.width;
  next.height = old.height;
  parent.replaceChild(next, old);
  old.dispatchEvent(new CustomEvent(CANVAS_REPLACED_EVENT, { detail: next }));
  return next;
}

interface LoadedAtlas {
  data: AtlasData;
  /** Shipped pages, then (at `staticPages`) the dynamic page when there is one. */
  bitmaps: ImageBitmap[];
  textures: WebGLTexture[];
  sizes: PageSize[];
  staticPages: number;
  /** Frame ids that live on the dynamic page. */
  dynamicFrames: string[];
}

export function createGlRenderer(version: 1 | 2): Renderer {
  let canvas: HTMLCanvasElement | null = null;
  let gl: GL | null = null;
  let program: WebGLProgram | null = null;
  let vbo: WebGLBuffer | null = null;
  let ibo: WebGLBuffer | null = null;
  let vao: WebGLVertexArrayObject | null = null;
  let iboQuads = 0;
  let uRes: WebGLUniformLocation | null = null;
  let aPos = 0;
  let aUV = 0;
  let aColor = 0;
  let scratch: Float32Array | null = null;
  let tier: Tier = 1;
  let dpr = 1;
  let width = 1;
  let height = 1;
  let atlas: LoadedAtlas | null = null;
  const chunks = new ChunkStore();
  const chunkTextures = new Map<ChunkId, WebGLTexture>();
  let fbo: WebGLFramebuffer | null = null;
  const textAtlas = new TextAtlas();
  let textTexture: WebGLTexture | null = null;
  let textVersion = -1;
  let caps: Caps | null = null;
  const ctx = new ContextTracker();
  let sleeping = false;

  function need(): GL {
    if (!gl) throw new RenderError(ERR.NOT_INITIALISED, 'renderer not initialised');
    return gl;
  }

  function setupProgram(g: GL): void {
    program = buildProgram(g, version);
    g.useProgram(program);
    uRes = g.getUniformLocation(program, 'uRes');
    aPos = g.getAttribLocation(program, 'aPos');
    aUV = g.getAttribLocation(program, 'aUV');
    aColor = g.getAttribLocation(program, 'aColor');
    const uTex = g.getUniformLocation(program, 'uTex');
    if (uTex) g.uniform1i(uTex, 0);
    vbo = g.createBuffer();
    ibo = g.createBuffer();
    if (version === 2) {
      const g2 = g as WebGL2RenderingContext;
      vao = g2.createVertexArray();
      g2.bindVertexArray(vao);
    }
    bindAttribs(g);
    g.disable(g.DEPTH_TEST);
    g.enable(g.BLEND);
    g.blendFunc(g.ONE, g.ONE_MINUS_SRC_ALPHA);
    g.pixelStorei(g.UNPACK_PREMULTIPLY_ALPHA_WEBGL, true);
    g.clearColor(0, 0, 0, 0);
  }

  function bindAttribs(g: GL): void {
    g.bindBuffer(g.ARRAY_BUFFER, vbo);
    g.bindBuffer(g.ELEMENT_ARRAY_BUFFER, ibo);
    const stride = 8 * 4;
    g.enableVertexAttribArray(aPos);
    g.vertexAttribPointer(aPos, 2, g.FLOAT, false, stride, 0);
    g.enableVertexAttribArray(aUV);
    g.vertexAttribPointer(aUV, 2, g.FLOAT, false, stride, 8);
    g.enableVertexAttribArray(aColor);
    g.vertexAttribPointer(aColor, 4, g.FLOAT, false, stride, 16);
  }

  function ensureIndices(g: GL, quads: number): void {
    if (quads <= iboQuads) return;
    let n = Math.max(256, iboQuads || 0);
    while (n < quads) n *= 2;
    // 16-bit indices cap the buffer at 16384 quads per draw batch.
    n = Math.min(n, 16383);
    g.bindBuffer(g.ELEMENT_ARRAY_BUFFER, ibo);
    g.bufferData(g.ELEMENT_ARRAY_BUFFER, buildIndices(n), g.STATIC_DRAW);
    iboQuads = n;
  }

  function makeTexture(g: GL): WebGLTexture {
    const tex = g.createTexture();
    if (!tex) throw new RenderError(ERR.SHADER, 'createTexture failed');
    g.bindTexture(g.TEXTURE_2D, tex);
    g.texParameteri(g.TEXTURE_2D, g.TEXTURE_MIN_FILTER, g.LINEAR);
    g.texParameteri(g.TEXTURE_2D, g.TEXTURE_MAG_FILTER, g.NEAREST);
    g.texParameteri(g.TEXTURE_2D, g.TEXTURE_WRAP_S, g.CLAMP_TO_EDGE);
    g.texParameteri(g.TEXTURE_2D, g.TEXTURE_WRAP_T, g.CLAMP_TO_EDGE);
    return tex;
  }

  function uploadAtlasPages(g: GL, loaded: LoadedAtlas): void {
    loaded.textures = [];
    loaded.sizes = [];
    for (const bmp of loaded.bitmaps) {
      const tex = makeTexture(g);
      g.texImage2D(g.TEXTURE_2D, 0, g.RGBA, g.RGBA, g.UNSIGNED_BYTE, bmp);
      loaded.textures.push(tex);
      loaded.sizes.push({ w: bmp.width, h: bmp.height });
    }
  }

  function syncTextTexture(g: GL): void {
    const page = textAtlas.page;
    if (!page) return;
    if (!textTexture) textTexture = makeTexture(g);
    if (textVersion === textAtlas.version) return;
    g.bindTexture(g.TEXTURE_2D, textTexture);
    g.texImage2D(
      g.TEXTURE_2D,
      0,
      g.RGBA,
      g.RGBA,
      g.UNSIGNED_BYTE,
      page as unknown as TexImageSource,
    );
    textVersion = textAtlas.version;
  }

  function drawQuads(g: GL, verts: Float32Array, quads: number, tex: WebGLTexture | null): void {
    if (quads === 0 || !tex) return;
    ensureIndices(g, quads);
    g.bindBuffer(g.ARRAY_BUFFER, vbo);
    g.bufferData(g.ARRAY_BUFFER, verts.subarray(0, quads * FLOATS_PER_QUAD), g.DYNAMIC_DRAW);
    g.activeTexture(g.TEXTURE0);
    g.bindTexture(g.TEXTURE_2D, tex);
    g.drawElements(g.TRIANGLES, quads * INDICES_PER_QUAD, g.UNSIGNED_SHORT, 0);
  }

  function bake(g: GL, id: ChunkId): boolean {
    const entry = chunks.get(id);
    if (!entry || !atlas) return false;
    const size = budgetForTier(tier).chunkTexture;
    let tex = chunkTextures.get(id);
    if (!tex) {
      tex = makeTexture(g);
      g.texImage2D(g.TEXTURE_2D, 0, g.RGBA, size, size, 0, g.RGBA, g.UNSIGNED_BYTE, null);
      chunkTextures.set(id, tex);
    }
    if (!fbo) fbo = g.createFramebuffer();
    g.bindFramebuffer(g.FRAMEBUFFER, fbo);
    g.framebufferTexture2D(g.FRAMEBUFFER, g.COLOR_ATTACHMENT0, g.TEXTURE_2D, tex, 0);
    g.viewport(0, 0, size, size);
    if (uRes) g.uniform2f(uRes, size, size);
    g.clearColor(0, 0, 0, 0);
    g.clear(g.COLOR_BUFFER_BIT);

    const layout = chunkLayout(size);
    const tiles = sortTiles(entry.tiles);
    scratch = growScratch(scratch, tiles.length * FLOATS_PER_QUAD);
    // Tiles are grouped per page so a chunk bake is one draw call per page.
    const byPage = new Map<number, number[]>();
    tiles.forEach((t, i) => {
      const f = atlas!.data.frames.get(t.frame);
      if (!f) return;
      const list = byPage.get(f.page);
      if (list) list.push(i);
      else byPage.set(f.page, [i]);
    });
    for (const [page, idxs] of byPage) {
      let floats = 0;
      let quads = 0;
      for (const i of idxs) {
        const t = tiles[i];
        const f = atlas.data.frames.get(t.frame)!;
        const p = chunkTilePx(layout, t.lx, t.ly, t.z ?? 0);
        const sizes = atlas.sizes[page];
        const px = t.flip ? f.w - f.pivotX : f.pivotX;
        floats = writeQuad(scratch, floats, {
          page,
          sx: p.px - px * layout.scale,
          sy: p.py - f.pivotY * layout.scale,
          w: f.w * layout.scale,
          h: f.h * layout.scale,
          u0: f.x / sizes.w,
          v0: f.y / sizes.h,
          u1: (f.x + f.w) / sizes.w,
          v1: (f.y + f.h) / sizes.h,
          flip: t.flip,
          tint: t.tint,
        });
        quads++;
      }
      drawQuads(g, scratch, quads, atlas.textures[page] ?? null);
    }
    g.bindFramebuffer(g.FRAMEBUFFER, null);
    chunks.markBaked(id);
    return true;
  }

  const renderer: Renderer = {
    get tier() {
      return tier;
    },
    setTier(next: Tier) {
      if (next === tier) return;
      tier = next;
      // Chunk textures change size with the tier, so everything is stale.
      for (const [, tex] of chunkTextures) gl?.deleteTexture(tex);
      chunkTextures.clear();
      chunks.invalidateAll();
      if (caps) caps.tier = next;
    },

    async init(target: HTMLCanvasElement, opts: InitOpts): Promise<Caps> {
      canvas = target;
      tier = opts.tier;
      dpr = Math.min(opts.dpr ?? (globalThis.devicePixelRatio || 1), budgetForTier(tier).maxDpr);
      const g = getGlContext(target, version);
      if (!g) throw new RenderError(ERR.NO_WEBGL, `webgl${version} unavailable`);
      gl = g;
      ctx.attach(target, {
        onLost: () => {
          chunkTextures.clear();
          chunks.invalidateAll();
        },
        onRestored: () => {
          void renderer.wake();
        },
      });
      setupProgram(g);
      width = target.width || 1;
      height = target.height || 1;
      g.viewport(0, 0, width, height);
      caps = {
        backend: version === 2 ? 'gl2' : 'gl1',
        tier,
        maxTextureSize: g.getParameter(g.MAX_TEXTURE_SIZE) as number,
        renderTexture: true,
        timerQuery: version === 2 && !!g.getExtension('EXT_disjoint_timer_query_webgl2'),
        dpr,
      };
      sleeping = false;
      return caps;
    },

    loadAtlas(json: unknown, pages: ImageBitmap[]): AtlasHandle {
      const g = need();
      const data = parseAtlas(json);
      if (pages.length < data.pages.length) {
        throw new RenderError(ERR.ATLAS_INVALID, 'fewer bitmaps than atlas pages');
      }
      if (atlas) for (const t of atlas.textures) g.deleteTexture(t);
      const loaded: LoadedAtlas = { data, bitmaps: pages.slice(0, data.pages.length), textures: [], sizes: [], staticPages: data.pages.length, dynamicFrames: [] };
      uploadAtlasPages(g, loaded);
      atlas = loaded;
      chunks.invalidateAll();
      return { id: 1, frames: new Set(data.frames.keys()) };
    },

    frame(scene: Scene, _dt: number): FrameStats {
      const t0 = performance.now();
      const stats: FrameStats = { cpuMs: 0, drawCalls: 0, sprites: 0, chunksBaked: 0 };
      if (!gl || sleeping || !ctx.isLive) {
        stats.cpuMs = performance.now() - t0;
        return stats;
      }
      const g = gl;
      if (!atlas) {
        stats.cpuMs = performance.now() - t0;
        return stats;
      }
      if (vao) (g as WebGL2RenderingContext).bindVertexArray(vao);

      // 1. Bake at most two dirty chunks per frame so a frame never stalls.
      for (const id of chunks.dirtyAmong(scene.chunksVisible, 2)) {
        if (bake(g, id)) stats.chunksBaked++;
      }
      chunks.touch(scene.chunksVisible);

      g.bindFramebuffer(g.FRAMEBUFFER, null);
      g.viewport(0, 0, width, height);
      if (uRes) g.uniform2f(uRes, width, height);
      g.clear(g.COLOR_BUFFER_BIT);

      // 2. One draw call per visible chunk. The FBO texture is y-flipped, so V is
      // swapped instead of re-rendering it.
      scratch = growScratch(scratch, FLOATS_PER_QUAD);
      const chunkSize = budgetForTier(tier).chunkTexture;
      const layout = chunkLayout(chunkSize);
      const drawSide = (chunkSize / layout.scale) * scene.camera.zoom;
      const shiftX = (layout.originX / layout.scale) * scene.camera.zoom;
      const shiftY = (layout.originY / layout.scale) * scene.camera.zoom;
      for (const id of scene.chunksVisible) {
        const tex = chunkTextures.get(id);
        if (!tex) continue;
        const { cx, cy } = parseChunkId(id);
        const origin = worldToScreen(scene.camera, cx * CHUNK_TILES, cy * CHUNK_TILES);
        writeQuad(scratch, 0, {
          page: 0,
          sx: origin.sx - shiftX,
          sy: origin.sy - shiftY,
          w: drawSide,
          h: drawSide,
          u0: 0,
          v0: 1,
          u1: 1,
          v1: 0,
        });
        drawQuads(g, scratch, 1, tex);
        stats.drawCalls++;
      }

      // 3. Sprites, sorted by y + z, one draw call per atlas page run.
      const visible = cullSprites(scene.sprites, atlas.data, scene.camera);
      scratch = growScratch(scratch, visible.length * FLOATS_PER_QUAD);
      const built = buildSpriteBatches(visible, atlas.data, scene.camera, atlas.sizes, scratch);
      if (built.quads > 0) {
        ensureIndices(g, built.quads);
        g.bindBuffer(g.ARRAY_BUFFER, vbo);
        g.bufferData(
          g.ARRAY_BUFFER,
          built.vertices.subarray(0, built.quads * FLOATS_PER_QUAD),
          g.DYNAMIC_DRAW,
        );
        g.activeTexture(g.TEXTURE0);
        for (const b of built.batches) {
          const tex = atlas.textures[b.page];
          if (!tex) continue;
          g.bindTexture(g.TEXTURE_2D, tex);
          g.drawElements(g.TRIANGLES, b.count, g.UNSIGNED_SHORT, b.offset * 2);
          stats.drawCalls++;
        }
        stats.sprites = built.quads;
      }

      // 4. Text: one page, one draw call.
      if (scene.texts.length > 0) {
        syncTextTexture(g);
        scratch = growScratch(scratch, scene.texts.length * FLOATS_PER_QUAD);
        let floats = 0;
        let quads = 0;
        for (const ti of scene.texts) {
          const rect = textAtlas.rect(ti.handle.id);
          if (!rect) continue;
          const uv = textUV(rect, textAtlas.size);
          const p = worldToScreen(scene.camera, ti.x, ti.y, ti.z);
          floats = writeQuad(scratch, floats, {
            page: 0,
            sx: p.sx - rect.w / 2,
            sy: p.sy - rect.h,
            w: rect.w,
            h: rect.h,
            ...uv,
            tint: ti.tint,
            alpha: ti.alpha,
          });
          quads++;
        }
        drawQuads(g, scratch, quads, textTexture);
        if (quads > 0) stats.drawCalls++;
      }

      stats.cpuMs = performance.now() - t0;
      return stats;
    },

    bakeChunk(id: ChunkId, tiles: ChunkTile[]): void {
      chunks.set(id, tiles);
    },

    invalidateChunk(id: ChunkId): void {
      chunks.invalidate(id);
    },

    setDynamicPage(page: ImageBitmap | null, frames: Record<string, DynamicFrame>): void {
      if (!atlas) throw new RenderError(ERR.NOT_INITIALISED, 'dynamic page before the atlas');
      const idx = atlas.staticPages;
      for (const id of atlas.dynamicFrames) atlas.data.frames.delete(id);
      atlas.dynamicFrames = [];
      if (!page) {
        const tex = atlas.textures[idx];
        if (tex && gl) gl.deleteTexture(tex);
        atlas.bitmaps.length = idx;
        atlas.textures.length = Math.min(atlas.textures.length, idx);
        atlas.sizes.length = Math.min(atlas.sizes.length, idx);
        return;
      }
      atlas.bitmaps[idx] = page;
      atlas.sizes[idx] = { w: page.width, h: page.height };
      for (const [id, f] of Object.entries(frames)) {
        atlas.data.frames.set(id, { page: idx, x: f.x, y: f.y, w: f.w, h: f.h, pivotX: f.pivotX, pivotY: f.pivotY, zBase: 0 });
        atlas.dynamicFrames.push(id);
      }
      // One texture for the page, reused: a new picture re-uploads into it.
      const g = gl;
      if (!g || sleeping || !ctx.isLive || atlas.textures.length < idx) return; // wake() uploads it
      let tex = atlas.textures[idx];
      if (!tex) {
        tex = makeTexture(g);
        atlas.textures[idx] = tex;
      }
      g.bindTexture(g.TEXTURE_2D, tex);
      g.texImage2D(g.TEXTURE_2D, 0, g.RGBA, g.RGBA, g.UNSIGNED_BYTE, page);
    },

    memory() {
      let textures = chunkTextures.size + (textTexture ? 1 : 0);
      let bytes = chunkTextures.size * budgetForTier(tier).chunkTexture ** 2 * 4;
      if (atlas) {
        textures += atlas.textures.filter(Boolean).length;
        atlas.textures.forEach((t, i) => {
          if (t && atlas!.sizes[i]) bytes += atlas!.sizes[i].w * atlas!.sizes[i].h * 4;
        });
      }
      const dyn = atlas && atlas.bitmaps.length > atlas.staticPages ? atlas.sizes[atlas.staticPages] : null;
      return { textures, bytes, dynamicPage: dyn ? dyn.w * dyn.h * 4 : 0 };
    },

    text(str: string, style?: TextStyle): TextHandle {
      return textAtlas.get(str, style);
    },

    resize(w: number, h: number, nextDpr: number): void {
      dpr = Math.min(nextDpr, budgetForTier(tier).maxDpr);
      width = Math.max(1, Math.round(w * dpr));
      height = Math.max(1, Math.round(h * dpr));
      if (canvas) {
        canvas.width = width;
        canvas.height = height;
      }
      gl?.viewport(0, 0, width, height);
      if (caps) caps.dpr = dpr;
    },

    sleep(): void {
      if (sleeping) return;
      sleeping = true;
      const g = gl;
      if (g) {
        for (const [, tex] of chunkTextures) g.deleteTexture(tex);
        chunkTextures.clear();
        if (atlas) {
          for (const t of atlas.textures) g.deleteTexture(t);
          atlas.textures = [];
        }
        if (textTexture) {
          g.deleteTexture(textTexture);
          textTexture = null;
          textVersion = -1;
        }
        if (fbo) {
          g.deleteFramebuffer(fbo);
          fbo = null;
        }
      }
      chunks.invalidateAll();
      ctx.loseDeliberately(g);
    },

    async wake(): Promise<void> {
      if (!canvas) throw new RenderError(ERR.NOT_INITIALISED, 'wake before init');
      ctx.restoreDeliberately(gl);
      // restoreContext() is asynchronous and a canvas never hands out a second
      // context, so the same object has to report itself alive again first.
      // Measured on macOS/WKWebView: with the window occluded the driver does
      // not honour restoreContext() at all, so the fallback below swaps the
      // canvas element for a fresh one in the same DOM slot.
      if (gl && gl.isContextLost()) await waitForRestore(gl, 3000);
      let g = gl && !gl.isContextLost() ? gl : getGlContext(canvas, version);
      if (!g || g.isContextLost()) {
        const replacement = replaceCanvas(canvas);
        if (!replacement) throw new RenderError(ERR.CONTEXT_LOST, 'context did not come back');
        canvas = replacement;
        ctx.attach(canvas, {
          onLost: () => {
            chunkTextures.clear();
            chunks.invalidateAll();
          },
          onRestored: () => {
            void renderer.wake();
          },
        });
        g = getGlContext(canvas, version);
        if (!g || g.isContextLost()) {
          throw new RenderError(ERR.CONTEXT_LOST, 'context did not come back');
        }
      }
      gl = g;
      iboQuads = 0;
      setupProgram(g);
      g.viewport(0, 0, width, height);
      if (atlas) uploadAtlasPages(g, atlas); // from the ImageBitmaps kept in memory
      chunks.invalidateAll();
      sleeping = false;
      ctx.apply('restored');
    },

    destroy(): void {
      const g = gl;
      if (g) {
        for (const [, tex] of chunkTextures) g.deleteTexture(tex);
        if (atlas) for (const t of atlas.textures) g.deleteTexture(t);
        if (textTexture) g.deleteTexture(textTexture);
        if (fbo) g.deleteFramebuffer(fbo);
        if (vbo) g.deleteBuffer(vbo);
        if (ibo) g.deleteBuffer(ibo);
        if (program) g.deleteProgram(program);
        if (vao) (g as WebGL2RenderingContext).deleteVertexArray(vao);
      }
      chunkTextures.clear();
      chunks.clear();
      textAtlas.release();
      ctx.apply('destroy');
      ctx.detach();
      atlas = null;
      gl = null;
      canvas = null;
      scratch = null;
    },
  };

  return renderer;
}

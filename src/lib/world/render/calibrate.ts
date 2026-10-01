// Calibration: the machine's tier is MEASURED, never guessed. No GPU string, no
// navigator, no core count, no OS (§1.3 of docs/llm-world-execution-plan.md).
// The scene is identical on every platform: 960x540, 9 chunk-sized quads, 256
// sprite-sized quads over a 2048² texture and 32 text quads, 10 warm-up frames and
// 40 measured frames, each closed by readPixels(1x1).
// I-4: readPixels is compared with gl.finish() and EXT_disjoint_timer_query_webgl2,
// and the largest of the three wins, so the classification errs pessimistic.

import { writeQuad, buildIndices, FLOATS_PER_QUAD, INDICES_PER_QUAD } from './batcher';
import { buildProgram, getGlContext, GL_ATTRIBUTES } from './gl2';
import { tierFromMedian } from './tier';
import type { BackendName, CalibrationResult, Tier } from './types';

export const CAL_WIDTH = 960;
export const CAL_HEIGHT = 540;
export const CAL_CHUNK_QUADS = 9;
export const CAL_SPRITE_QUADS = 256;
export const CAL_TEXT_QUADS = 32;
export const CAL_WARMUP = 10;
export const CAL_FRAMES = 40;
/** Frames of the two comparison passes of I-4; short on purpose. */
export const CAL_COMPARE_FRAMES = 10;
export const CAL_TEXTURE_SIZE = 2048;

export type SyncMode = 'readPixels' | 'finish' | 'timerQuery';

export function median(values: readonly number[]): number {
  if (values.length === 0) return 0;
  const s = Array.from(values).sort((a, b) => a - b);
  const mid = s.length >> 1;
  return s.length % 2 ? s[mid] : (s[mid - 1] + s[mid]) / 2;
}

/**
 * Runs one measuring pass. Injected `draw`, `sync` and `now` so the tests can
 * drive it with a fake GL. Returns the measured frame times, warm-up excluded.
 */
export function runPass(
  draw: () => void,
  sync: () => void,
  now: () => number,
  frames: number,
  warmup: number,
): number[] {
  const out: number[] = [];
  for (let i = 0; i < warmup + frames; i++) {
    const t0 = now();
    draw();
    sync();
    const dt = now() - t0;
    if (i >= warmup) out.push(dt);
  }
  return out;
}

/** Vertices of the calibration scene: chunks, then sprites, then text quads. */
export function buildCalibrationVertices(): { vertices: Float32Array; quads: number } {
  const quads = CAL_CHUNK_QUADS + CAL_SPRITE_QUADS + CAL_TEXT_QUADS;
  const out = new Float32Array(quads * FLOATS_PER_QUAD);
  let o = 0;
  for (let i = 0; i < CAL_CHUNK_QUADS; i++) {
    const col = i % 3;
    const row = Math.floor(i / 3);
    o = writeQuad(out, o, {
      page: 0,
      sx: col * 300,
      sy: row * 170,
      w: 512,
      h: 512,
      u0: 0,
      v0: 0,
      u1: 1,
      v1: 1,
      alpha: 0.5,
    });
  }
  for (let i = 0; i < CAL_SPRITE_QUADS; i++) {
    const col = i % 16;
    const row = Math.floor(i / 16);
    o = writeQuad(out, o, {
      page: 0,
      sx: col * 58,
      sy: row * 32,
      w: 64,
      h: 64,
      u0: (col * 64) / CAL_TEXTURE_SIZE,
      v0: (row * 64) / CAL_TEXTURE_SIZE,
      u1: (col * 64 + 64) / CAL_TEXTURE_SIZE,
      v1: (row * 64 + 64) / CAL_TEXTURE_SIZE,
      tint: 0xffcc88,
      alpha: 0.9,
    });
  }
  for (let i = 0; i < CAL_TEXT_QUADS; i++) {
    o = writeQuad(out, o, {
      page: 0,
      sx: (i % 8) * 110,
      sy: Math.floor(i / 8) * 120,
      w: 96,
      h: 16,
      u0: 0,
      v0: 0,
      u1: 0.05,
      v1: 0.01,
      alpha: 0.8,
    });
  }
  return { vertices: out, quads };
}

/** Deterministic 2048² checkerboard, so the upload cost is the same everywhere. */
export function buildCalibrationTexture(size = CAL_TEXTURE_SIZE): Uint8Array {
  const px = new Uint8Array(size * size * 4);
  for (let y = 0; y < size; y++) {
    for (let x = 0; x < size; x++) {
      const i = (y * size + x) * 4;
      const on = ((x >> 3) + (y >> 3)) & 1;
      px[i] = on ? 220 : 40;
      px[i + 1] = on ? 180 : 60;
      px[i + 2] = on ? 120 : 90;
      px[i + 3] = 255;
    }
  }
  return px;
}

export interface CalibrationDetail extends CalibrationResult {
  /** Median frame time (ms) of each synchronisation method; null = unavailable. */
  readPixelsMs: number;
  finishMs: number;
  timerQueryMs: number | null;
  /**
   * Wall time of a whole pass divided by its frames. WKWebView clamps
   * performance.now() to 1 ms, so the per-frame medians above saturate at 0/1 ms
   * on a fast machine; these two carry the sub-millisecond truth (I-4).
   */
  readPixelsPerFrameMs: number;
  finishPerFrameMs: number;
  /** The one the tier came from: the largest of the three (I-4). */
  usedMethod: SyncMode;
  frames: number;
  durationMs: number;
  maxTextureSize: number;
}

function makeCanvas(): HTMLCanvasElement | null {
  if (typeof document === 'undefined') return null;
  const c = document.createElement('canvas');
  c.width = CAL_WIDTH;
  c.height = CAL_HEIGHT;
  return c;
}

/** Full calibration with the I-4 comparison table. */
export async function calibrateDetailed(
  target?: HTMLCanvasElement | null,
): Promise<CalibrationDetail> {
  const started = typeof performance !== 'undefined' ? performance.now() : 0;
  const now = () => (typeof performance !== 'undefined' ? performance.now() : Date.now());
  const canvas = target ?? makeCanvas();
  const fail = (backend: BackendName | 'none', lost: boolean): CalibrationDetail => ({
    tier: 0 as Tier,
    medianMs: Infinity,
    backend,
    contextLostDuring: lost,
    readPixelsMs: Infinity,
    finishMs: Infinity,
    readPixelsPerFrameMs: Infinity,
    finishPerFrameMs: Infinity,
    timerQueryMs: null,
    usedMethod: 'readPixels',
    frames: 0,
    durationMs: now() - started,
    maxTextureSize: 0,
  });
  if (!canvas) return fail('none', false);

  canvas.width = CAL_WIDTH;
  canvas.height = CAL_HEIGHT;
  let version: 1 | 2 = 2;
  let gl = getGlContext(canvas, 2);
  if (!gl) {
    gl = getGlContext(canvas, 1);
    version = 1;
  }
  if (!gl) return fail('none', false);
  const backend: BackendName = version === 2 ? 'gl2' : 'gl1';

  let contextLost = false;
  const onLost = (e: Event) => {
    e.preventDefault();
    contextLost = true;
  };
  canvas.addEventListener('webglcontextlost', onLost as EventListener, false);

  try {
    const program = buildProgram(gl, version);
    gl.useProgram(program);
    const uRes = gl.getUniformLocation(program, 'uRes');
    const uTex = gl.getUniformLocation(program, 'uTex');
    if (uTex) gl.uniform1i(uTex, 0);
    if (uRes) gl.uniform2f(uRes, CAL_WIDTH, CAL_HEIGHT);

    const { vertices, quads } = buildCalibrationVertices();
    const vbo = gl.createBuffer();
    const ibo = gl.createBuffer();
    gl.bindBuffer(gl.ARRAY_BUFFER, vbo);
    gl.bufferData(gl.ARRAY_BUFFER, vertices, gl.STATIC_DRAW);
    gl.bindBuffer(gl.ELEMENT_ARRAY_BUFFER, ibo);
    gl.bufferData(gl.ELEMENT_ARRAY_BUFFER, buildIndices(quads), gl.STATIC_DRAW);
    const stride = 8 * 4;
    const aPos = gl.getAttribLocation(program, 'aPos');
    const aUV = gl.getAttribLocation(program, 'aUV');
    const aColor = gl.getAttribLocation(program, 'aColor');
    gl.enableVertexAttribArray(aPos);
    gl.vertexAttribPointer(aPos, 2, gl.FLOAT, false, stride, 0);
    gl.enableVertexAttribArray(aUV);
    gl.vertexAttribPointer(aUV, 2, gl.FLOAT, false, stride, 8);
    gl.enableVertexAttribArray(aColor);
    gl.vertexAttribPointer(aColor, 4, gl.FLOAT, false, stride, 16);

    const tex = gl.createTexture();
    gl.bindTexture(gl.TEXTURE_2D, tex);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.LINEAR);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.NEAREST);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_S, gl.CLAMP_TO_EDGE);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_T, gl.CLAMP_TO_EDGE);
    gl.texImage2D(
      gl.TEXTURE_2D,
      0,
      gl.RGBA,
      CAL_TEXTURE_SIZE,
      CAL_TEXTURE_SIZE,
      0,
      gl.RGBA,
      gl.UNSIGNED_BYTE,
      buildCalibrationTexture(),
    );
    gl.activeTexture(gl.TEXTURE0);
    gl.bindTexture(gl.TEXTURE_2D, tex);
    gl.disable(gl.DEPTH_TEST);
    gl.enable(gl.BLEND);
    gl.blendFunc(gl.SRC_ALPHA, gl.ONE_MINUS_SRC_ALPHA);
    gl.viewport(0, 0, CAL_WIDTH, CAL_HEIGHT);
    gl.clearColor(0, 0, 0, 1);

    const pixel = new Uint8Array(4);
    const draw = () => {
      gl.clear(gl.COLOR_BUFFER_BIT);
      gl.drawElements(gl.TRIANGLES, quads * INDICES_PER_QUAD, gl.UNSIGNED_SHORT, 0);
    };
    const syncReadPixels = () => gl.readPixels(0, 0, 1, 1, gl.RGBA, gl.UNSIGNED_BYTE, pixel);
    const syncFinish = () => gl.finish();

    const rpStart = now();
    const readPixelsSamples = runPass(draw, syncReadPixels, now, CAL_FRAMES, CAL_WARMUP);
    const readPixelsPerFrameMs = (now() - rpStart) / (CAL_FRAMES + CAL_WARMUP);
    const fnStart = now();
    const finishSamples = runPass(draw, syncFinish, now, CAL_COMPARE_FRAMES, 2);
    const finishPerFrameMs = (now() - fnStart) / (CAL_COMPARE_FRAMES + 2);

    // GPU time through EXT_disjoint_timer_query_webgl2, when the driver has it.
    let timerQueryMs: number | null = null;
    const ext =
      version === 2
        ? ((gl as WebGL2RenderingContext).getExtension(
            'EXT_disjoint_timer_query_webgl2',
          ) as { TIME_ELAPSED_EXT: number; GPU_DISJOINT_EXT: number } | null)
        : null;
    if (ext) {
      const gl2 = gl as WebGL2RenderingContext;
      const samples: number[] = [];
      for (let i = 0; i < CAL_COMPARE_FRAMES; i++) {
        const q = gl2.createQuery();
        if (!q) break;
        gl2.beginQuery(ext.TIME_ELAPSED_EXT, q);
        draw();
        gl2.endQuery(ext.TIME_ELAPSED_EXT);
        gl2.flush();
        const ns = await waitForQuery(gl2, q, ext.GPU_DISJOINT_EXT);
        gl2.deleteQuery(q);
        if (ns === null) break;
        samples.push(ns / 1e6);
      }
      if (samples.length > 0) timerQueryMs = median(samples);
    }

    gl.deleteTexture(tex);
    gl.deleteBuffer(vbo);
    gl.deleteBuffer(ibo);
    gl.deleteProgram(program);

    const readPixelsMs = median(readPixelsSamples);
    const finishMs = median(finishSamples);
    // The largest of the three wins: a method that does not actually synchronise
    // would measure only JS and classify the machine too high (I-4).
    let usedMethod: SyncMode = 'readPixels';
    let medianMs = readPixelsMs;
    if (finishMs > medianMs) {
      medianMs = finishMs;
      usedMethod = 'finish';
    }
    if (timerQueryMs !== null && timerQueryMs > medianMs) {
      medianMs = timerQueryMs;
      usedMethod = 'timerQuery';
    }
    // Sub-millisecond truth wins over a median flattened by a 1 ms clock.
    if (readPixelsPerFrameMs > medianMs) {
      medianMs = readPixelsPerFrameMs;
      usedMethod = 'readPixels';
    }
    if (finishPerFrameMs > medianMs) {
      medianMs = finishPerFrameMs;
      usedMethod = 'finish';
    }

    const tier = tierFromMedian(medianMs, { webglAvailable: true, contextLost });
    return {
      tier,
      medianMs,
      backend,
      contextLostDuring: contextLost,
      readPixelsMs,
      finishMs,
      readPixelsPerFrameMs,
      finishPerFrameMs,
      timerQueryMs,
      usedMethod,
      frames: readPixelsSamples.length,
      durationMs: now() - started,
      maxTextureSize: gl.getParameter(gl.MAX_TEXTURE_SIZE) as number,
    };
  } catch {
    return fail(backend, contextLost);
  } finally {
    canvas.removeEventListener('webglcontextlost', onLost as EventListener);
    const lose = gl.getExtension('WEBGL_lose_context') as { loseContext(): void } | null;
    try {
      lose?.loseContext();
    } catch {
      // Already gone; nothing to release.
    }
  }
}

async function waitForQuery(
  gl2: WebGL2RenderingContext,
  q: WebGLQuery,
  disjointEnum: number,
): Promise<number | null> {
  for (let i = 0; i < 200; i++) {
    const available = gl2.getQueryParameter(q, gl2.QUERY_RESULT_AVAILABLE) as boolean;
    const disjoint = gl2.getParameter(disjointEnum) as boolean;
    if (disjoint) return null;
    if (available) return gl2.getQueryParameter(q, gl2.QUERY_RESULT) as number;
    await new Promise((r) => setTimeout(r, 1));
  }
  return null;
}

/** Contract entry point (docs/agents/f6-renderer.md §4). */
export async function calibrate(canvas?: HTMLCanvasElement | null): Promise<CalibrationResult> {
  const d = await calibrateDetailed(canvas);
  return {
    tier: d.tier,
    medianMs: d.medianMs,
    backend: d.backend,
    contextLostDuring: d.contextLostDuring,
  };
}

export { GL_ATTRIBUTES };

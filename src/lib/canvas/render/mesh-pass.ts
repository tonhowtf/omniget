// Drawings as one static triangle mesh (rebuilt per changed drawing, cached by
// record identity) plus a dynamic mesh for the stroke or shape in progress
// (02-motor-do-canvas §6.1: "enquanto desenha, buffer dinâmico; ao soltar,
// lote estático"). Vertices are relative to a batch origin; the camera offset
// is subtracted in float64 on the CPU before it reaches the shader.

import { linkProgram } from '$lib/gfx/gl';
import type { CanvasEngine } from '../engine.svelte';
import { buildDrawing, setDefaultInk } from '../draw/build';
import { freehandTriangles, polylineTriangles } from '../draw/stroke';
import { randomSeed } from '../draw/sketch';
import type { Drawing } from '../model/types';
import { parseColor, prefersDark, withAlpha, type Rgba } from './color';
import type { FrameInfo, GlPass } from './gl-layer';

const VS = `#version 300 es
in vec2 aPos;
in vec4 aColor;
uniform vec2 uCam;
uniform float uZoom;
uniform vec2 uRes;
out vec4 vColor;
void main() {
  vec2 s = (aPos - uCam) * uZoom;
  vec2 clip = s / uRes * 2.0 - 1.0;
  gl_Position = vec4(clip.x, -clip.y, 0.0, 1.0);
  vColor = aColor;
}`;

const FS = `#version 300 es
precision mediump float;
in vec4 vColor;
out vec4 outColor;
void main() {
  outColor = vec4(vColor.rgb * vColor.a, vColor.a);
}`;

const STRIDE = 6;

class MeshBuffer {
  data = new Float32Array(STRIDE * 1024);
  verts = 0;
  buf: WebGLBuffer | null = null;
  vao: WebGLVertexArrayObject | null = null;
  uploaded = -1;

  reset(): void {
    this.verts = 0;
  }

  push(tris: ArrayLike<number>, c: Rgba, ox: number, oy: number): void {
    const n = Math.floor(tris.length / 2);
    const need = (this.verts + n) * STRIDE;
    if (need > this.data.length) {
      let len = this.data.length;
      while (len < need) len *= 2;
      const next = new Float32Array(len);
      next.set(this.data.subarray(0, this.verts * STRIDE));
      this.data = next;
    }
    let o = this.verts * STRIDE;
    const d = this.data;
    for (let i = 0; i < n; i++) {
      d[o] = tris[i * 2] - ox;
      d[o + 1] = tris[i * 2 + 1] - oy;
      d[o + 2] = c[0];
      d[o + 3] = c[1];
      d[o + 4] = c[2];
      d[o + 5] = c[3];
      o += STRIDE;
    }
    this.verts += n;
  }
}

export function createMeshPass(engine: CanvasEngine): GlPass {
  let gl: WebGL2RenderingContext | null = null;
  let prog: WebGLProgram | null = null;
  let uCam: WebGLUniformLocation | null = null;
  let uZoom: WebGLUniformLocation | null = null;
  let uRes: WebGLUniformLocation | null = null;
  const statics = new MeshBuffer();
  const dynamic = new MeshBuffer();
  let builtRev = -1;
  let origin = { x: 0, y: 0 };
  const cache = new Map<string, { ref: Drawing; runs: ReturnType<typeof buildDrawing> }>();
  const draftSeed = randomSeed();

  const setup = (g: WebGL2RenderingContext, mb: MeshBuffer) => {
    mb.vao = g.createVertexArray();
    g.bindVertexArray(mb.vao);
    mb.buf = g.createBuffer();
    g.bindBuffer(g.ARRAY_BUFFER, mb.buf);
    const aPos = g.getAttribLocation(prog!, 'aPos');
    const aColor = g.getAttribLocation(prog!, 'aColor');
    g.enableVertexAttribArray(aPos);
    g.vertexAttribPointer(aPos, 2, g.FLOAT, false, STRIDE * 4, 0);
    g.enableVertexAttribArray(aColor);
    g.vertexAttribPointer(aColor, 4, g.FLOAT, false, STRIDE * 4, 8);
    g.bindVertexArray(null);
    mb.uploaded = -1;
  };

  const ensure = (g: WebGL2RenderingContext) => {
    if (gl === g && prog) return;
    gl = g;
    prog = linkProgram(g, VS, FS);
    uCam = g.getUniformLocation(prog, 'uCam');
    uZoom = g.getUniformLocation(prog, 'uZoom');
    uRes = g.getUniformLocation(prog, 'uRes');
    setup(g, statics);
    setup(g, dynamic);
    builtRev = -1;
  };

  let inkDark: boolean | null = null;
  const rebuild = () => {
    // Drawings saved without a colour use the theme ink (dark ink on light).
    const dark = prefersDark();
    if (dark !== inkDark) {
      inkDark = dark;
      setDefaultInk(dark ? [0.91, 0.9, 0.89, 1] : [0.11, 0.11, 0.12, 1]);
      cache.clear();
    }
    const drawings = engine.store.drawings;
    statics.reset();
    const first = drawings[0];
    origin = first ? { x: Math.round(first.x), y: Math.round(first.y) } : { x: 0, y: 0 };
    const alive = new Set<string>();
    for (const d of drawings) {
      alive.add(d.id);
      let entry = cache.get(d.id);
      if (!entry || entry.ref !== d) {
        entry = { ref: d, runs: buildDrawing(d) };
        cache.set(d.id, entry);
      }
      for (const run of entry.runs) statics.push(run.tris, run.color, origin.x, origin.y);
    }
    for (const id of Array.from(cache.keys())) if (!alive.has(id)) cache.delete(id);
  };

  const buildDraft = (): boolean => {
    const draft = engine.draft;
    dynamic.reset();
    if (draft.stroke && draft.stroke.pts.length >= 2) {
      const s = draft.stroke;
      const c = withAlpha(parseColor(s.color), s.highlighter ? 0.4 : 1);
      const real = s.pressures.some((p) => p !== 0.5);
      const tris = s.highlighter
        ? polylineTriangles(s.pts, s.width, false, 4)
        : freehandTriangles(s.pts, {
            size: s.width * 2,
            thinning: real ? 0.6 : 0.5,
            streamline: 0.35,
            taperStart: s.width * 3,
            pressures: real ? s.pressures : undefined,
          });
      dynamic.push(tris, c, origin.x, origin.y);
    }
    if (draft.shape) {
      const s = draft.shape;
      const x = Math.min(s.x0, s.x1);
      const y = Math.min(s.y0, s.y1);
      const d: Drawing = {
        id: 'draft',
        kind: s.kind,
        x,
        y,
        w: Math.abs(s.x1 - s.x0),
        h: Math.abs(s.y1 - s.y0),
        color: s.color,
        strokeWidth: s.width,
        sketch: s.sketch,
        seed: draftSeed,
        fillStyle: s.kind === 'arrow' || s.kind === 'line' ? undefined : s.fillStyle,
        points: s.kind === 'arrow' || s.kind === 'line' ? [s.x0 - x, s.y0 - y, s.x1 - x, s.y1 - y] : undefined,
      };
      for (const run of buildDrawing(d)) dynamic.push(run.tris, run.color, origin.x, origin.y);
    }
    return dynamic.verts > 0;
  };

  /** `rev` null = upload every time (the draft mesh). */
  const drawBuffer = (g: WebGL2RenderingContext, mb: MeshBuffer, rev: number | null) => {
    if (mb.verts === 0) return;
    g.bindVertexArray(mb.vao);
    g.bindBuffer(g.ARRAY_BUFFER, mb.buf);
    if (rev === null || mb.uploaded !== rev) {
      g.bufferData(g.ARRAY_BUFFER, mb.data.subarray(0, mb.verts * STRIDE), rev === null ? g.DYNAMIC_DRAW : g.STATIC_DRAW);
      mb.uploaded = rev ?? -1;
    }
    g.drawArrays(g.TRIANGLES, 0, mb.verts);
  };

  return {
    id: 'canvas.mesh',
    order: 20,
    draw(g: WebGL2RenderingContext, frame: FrameInfo) {
      ensure(g);
      const rev = engine.store.structure;
      if (rev !== builtRev) {
        rebuild();
        builtRev = rev;
        statics.uploaded = -1;
      }
      const hasDraft = buildDraft();
      if (statics.verts === 0 && !hasDraft) return;
      g.useProgram(prog);
      g.uniform2f(uCam, frame.camera.x - origin.x, frame.camera.y - origin.y);
      g.uniform1f(uZoom, frame.camera.zoom);
      g.uniform2f(uRes, frame.viewport.w, frame.viewport.h);
      drawBuffer(g, statics, builtRev);
      if (hasDraft) drawBuffer(g, dynamic, null);
      g.bindVertexArray(null);
    },
    dispose() {
      if (gl && !gl.isContextLost()) {
        if (prog) gl.deleteProgram(prog);
        for (const mb of [statics, dynamic]) {
          if (mb.buf) gl.deleteBuffer(mb.buf);
          if (mb.vao) gl.deleteVertexArray(mb.vao);
        }
      }
      gl = null;
      prog = null;
      cache.clear();
    },
  };
}

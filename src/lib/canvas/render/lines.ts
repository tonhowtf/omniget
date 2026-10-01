import { linkProgram } from '$lib/gfx/gl';
import type { Rgba } from './color';

const VS = `#version 300 es
in vec2 aPos;
in vec4 aColor;
in vec2 aEdge;
uniform vec2 uRes;
out vec4 vColor;
out vec2 vEdge;
void main() {
  vec2 clip = aPos / uRes * 2.0 - 1.0;
  gl_Position = vec4(clip.x, -clip.y, 0.0, 1.0);
  vColor = aColor;
  vEdge = aEdge;
}`;

const FS = `#version 300 es
precision highp float;
in vec4 vColor;
in vec2 vEdge;
out vec4 outColor;
void main() {
  // vEdge.x = signed distance from the centre line (px), vEdge.y = half width.
  float a = clamp(vEdge.y + 0.5 - abs(vEdge.x), 0.0, 1.0) * vColor.a;
  if (a < 0.003) discard;
  outColor = vec4(vColor.rgb * a, a);
}`;

const STRIDE = 8;

export class LineBatch {
  private gl: WebGL2RenderingContext | null = null;
  private prog: WebGLProgram | null = null;
  private vao: WebGLVertexArrayObject | null = null;
  private buf: WebGLBuffer | null = null;
  private uRes: WebGLUniformLocation | null = null;
  private data = new Float32Array(STRIDE * 4096);
  private verts = 0;
  private capacity = 0;

  private ensure(gl: WebGL2RenderingContext): void {
    if (this.gl === gl && this.prog) return;
    this.gl = gl;
    this.prog = linkProgram(gl, VS, FS);
    this.uRes = gl.getUniformLocation(this.prog, 'uRes');
    this.vao = gl.createVertexArray();
    gl.bindVertexArray(this.vao);
    this.buf = gl.createBuffer();
    gl.bindBuffer(gl.ARRAY_BUFFER, this.buf);
    this.capacity = 0;
    const attr = (name: string, size: number, offset: number) => {
      const loc = gl.getAttribLocation(this.prog!, name);
      if (loc < 0) return;
      gl.enableVertexAttribArray(loc);
      gl.vertexAttribPointer(loc, size, gl.FLOAT, false, STRIDE * 4, offset * 4);
    };
    attr('aPos', 2, 0);
    attr('aColor', 4, 2);
    attr('aEdge', 2, 6);
    gl.bindVertexArray(null);
  }

  begin(): void {
    this.verts = 0;
  }

  private vertex(x: number, y: number, c: Rgba, edge: number, half: number): void {
    if ((this.verts + 1) * STRIDE > this.data.length) {
      const next = new Float32Array(this.data.length * 2);
      next.set(this.data);
      this.data = next;
    }
    const o = this.verts * STRIDE;
    const d = this.data;
    d[o] = x;
    d[o + 1] = y;
    d[o + 2] = c[0];
    d[o + 3] = c[1];
    d[o + 4] = c[2];
    d[o + 5] = c[3];
    d[o + 6] = edge;
    d[o + 7] = half;
    this.verts++;
  }

  polyline(pts: ArrayLike<number>, width: number, color: Rgba, dash = 0): void {
    const n = Math.floor(pts.length / 2);
    if (n < 2) return;
    const half = width / 2;
    const ext = half + 1;
    let run = 0;
    for (let i = 0; i + 1 < n; i++) {
      const ax = pts[i * 2];
      const ay = pts[i * 2 + 1];
      const bx = pts[i * 2 + 2];
      const by = pts[i * 2 + 3];
      const dx = bx - ax;
      const dy = by - ay;
      const len = Math.hypot(dx, dy);
      if (len < 1e-4) continue;
      if (dash > 0) {
        const phase = Math.floor(run / dash) % 2;
        run += len;
        if (phase === 1) continue;
      }
      const nA = this.normalAt(pts, n, i);
      const nB = this.normalAt(pts, n, i + 1);
      const a1x = ax + nA[0] * ext;
      const a1y = ay + nA[1] * ext;
      const a2x = ax - nA[0] * ext;
      const a2y = ay - nA[1] * ext;
      const b1x = bx + nB[0] * ext;
      const b1y = by + nB[1] * ext;
      const b2x = bx - nB[0] * ext;
      const b2y = by - nB[1] * ext;
      this.vertex(a1x, a1y, color, ext, half);
      this.vertex(a2x, a2y, color, -ext, half);
      this.vertex(b1x, b1y, color, ext, half);
      this.vertex(b1x, b1y, color, ext, half);
      this.vertex(a2x, a2y, color, -ext, half);
      this.vertex(b2x, b2y, color, -ext, half);
    }
  }

  private normalAt(pts: ArrayLike<number>, n: number, i: number): [number, number] {
    const p = Math.max(0, i - 1);
    const q = Math.min(n - 1, i + 1);
    const dx = pts[q * 2] - pts[p * 2];
    const dy = pts[q * 2 + 1] - pts[p * 2 + 1];
    const len = Math.hypot(dx, dy) || 1;
    return [-dy / len, dx / len];
  }

  segment(x0: number, y0: number, x1: number, y1: number, width: number, color: Rgba, dash = 0): void {
    this.polyline([x0, y0, x1, y1], width, color, dash);
  }

  get size(): number {
    return this.verts;
  }

  flush(gl: WebGL2RenderingContext, vw: number, vh: number): void {
    if (this.verts === 0) return;
    this.ensure(gl);
    gl.useProgram(this.prog);
    gl.uniform2f(this.uRes, vw, vh);
    gl.bindVertexArray(this.vao);
    gl.bindBuffer(gl.ARRAY_BUFFER, this.buf);
    const floats = this.verts * STRIDE;
    if (floats > this.capacity) {
      gl.bufferData(gl.ARRAY_BUFFER, this.data.byteLength, gl.DYNAMIC_DRAW);
      this.capacity = this.data.length;
    }
    gl.bufferSubData(gl.ARRAY_BUFFER, 0, this.data, 0, floats);
    gl.drawArrays(gl.TRIANGLES, 0, this.verts);
    gl.bindVertexArray(null);
    this.verts = 0;
  }

  dispose(): void {
    const gl = this.gl;
    if (gl && !gl.isContextLost()) {
      if (this.prog) gl.deleteProgram(this.prog);
      if (this.vao) gl.deleteVertexArray(this.vao);
      if (this.buf) gl.deleteBuffer(this.buf);
    }
    this.gl = null;
    this.prog = null;
  }
}

import { linkProgram } from '$lib/gfx/gl';
import type { Rgba } from './color';

const VS = `#version 300 es
in vec2 aCorner;
in vec4 aRect;
in vec4 aStyle;
in vec4 aFill;
in vec4 aStroke;
uniform vec2 uRes;
out vec2 vLocal;
out vec2 vHalf;
out vec4 vStyle;
out vec4 vFill;
out vec4 vStroke;
void main() {
  vec2 halfSize = aRect.zw + 1.5;
  vec2 local = aCorner * halfSize;
  float c = cos(aStyle.z);
  float s = sin(aStyle.z);
  vec2 p = aRect.xy + vec2(local.x * c - local.y * s, local.x * s + local.y * c);
  vec2 clip = p / uRes * 2.0 - 1.0;
  gl_Position = vec4(clip.x, -clip.y, 0.0, 1.0);
  vLocal = local;
  vHalf = aRect.zw;
  vStyle = aStyle;
  vFill = aFill;
  vStroke = aStroke;
}`;

const FS = `#version 300 es
precision highp float;
in vec2 vLocal;
in vec2 vHalf;
in vec4 vStyle;
in vec4 vFill;
in vec4 vStroke;
out vec4 outColor;
float sdRoundBox(vec2 p, vec2 b, float r) {
  vec2 q = abs(p) - b + r;
  return length(max(q, 0.0)) + min(max(q.x, q.y), 0.0) - r;
}
void main() {
  float r = min(vStyle.x, min(vHalf.x, vHalf.y));
  float d = sdRoundBox(vLocal, vHalf, r);
  float fillA = clamp(0.5 - d, 0.0, 1.0) * vFill.a;
  vec4 col = vec4(vFill.rgb * fillA, fillA);
  float sw = vStyle.y;
  if (sw > 0.0) {
    float band = abs(d + sw * 0.5) - sw * 0.5;
    float sa = clamp(0.5 - band, 0.0, 1.0);
    // Dashed stroke: vStyle.w = dash length in px (0 = solid), along the perimeter angle.
    if (vStyle.w > 0.0) {
      float along = (abs(vLocal.x) / max(vHalf.x, 1.0) > abs(vLocal.y) / max(vHalf.y, 1.0)) ? vLocal.y : vLocal.x;
      if (mod(along + 10000.0, vStyle.w * 2.0) > vStyle.w) sa = 0.0;
    }
    sa *= vStroke.a;
    col = col * (1.0 - sa) + vec4(vStroke.rgb * sa, sa);
  }
  if (col.a < 0.003) discard;
  outColor = col;
}`;

const STRIDE = 16;

interface SdfShape {
  cx: number;
  cy: number;
  hw: number;
  hh: number;
  radius?: number;
  strokeWidth?: number;
  rotation?: number;
  dash?: number;
  fill?: Rgba;
  stroke?: Rgba;
}

const NONE: Rgba = [0, 0, 0, 0];

export class SdfBatch {
  private gl: WebGL2RenderingContext | null = null;
  private prog: WebGLProgram | null = null;
  private vao: WebGLVertexArrayObject | null = null;
  private quad: WebGLBuffer | null = null;
  private inst: WebGLBuffer | null = null;
  private uRes: WebGLUniformLocation | null = null;
  private data = new Float32Array(STRIDE * 256);
  private count = 0;
  private capacity = 0;

  private ensure(gl: WebGL2RenderingContext): void {
    if (this.gl === gl && this.prog) return;
    this.gl = gl;
    this.prog = linkProgram(gl, VS, FS);
    this.uRes = gl.getUniformLocation(this.prog, 'uRes');
    this.vao = gl.createVertexArray();
    gl.bindVertexArray(this.vao);
    this.quad = gl.createBuffer();
    gl.bindBuffer(gl.ARRAY_BUFFER, this.quad);
    gl.bufferData(gl.ARRAY_BUFFER, new Float32Array([-1, -1, 1, -1, -1, 1, 1, 1]), gl.STATIC_DRAW);
    const aCorner = gl.getAttribLocation(this.prog, 'aCorner');
    gl.enableVertexAttribArray(aCorner);
    gl.vertexAttribPointer(aCorner, 2, gl.FLOAT, false, 0, 0);
    this.inst = gl.createBuffer();
    gl.bindBuffer(gl.ARRAY_BUFFER, this.inst);
    this.capacity = 0;
    const attr = (name: string, offset: number) => {
      const loc = gl.getAttribLocation(this.prog!, name);
      if (loc < 0) return;
      gl.enableVertexAttribArray(loc);
      gl.vertexAttribPointer(loc, 4, gl.FLOAT, false, STRIDE * 4, offset * 4);
      gl.vertexAttribDivisor(loc, 1);
    };
    attr('aRect', 0);
    attr('aStyle', 4);
    attr('aFill', 8);
    attr('aStroke', 12);
    gl.bindVertexArray(null);
  }

  begin(): void {
    this.count = 0;
  }

  push(s: SdfShape): void {
    if ((this.count + 1) * STRIDE > this.data.length) {
      const next = new Float32Array(this.data.length * 2);
      next.set(this.data);
      this.data = next;
    }
    const o = this.count * STRIDE;
    const d = this.data;
    d[o] = s.cx;
    d[o + 1] = s.cy;
    d[o + 2] = s.hw;
    d[o + 3] = s.hh;
    d[o + 4] = s.radius ?? 0;
    d[o + 5] = s.strokeWidth ?? 0;
    d[o + 6] = s.rotation ?? 0;
    d[o + 7] = s.dash ?? 0;
    const f = s.fill ?? NONE;
    const k = s.stroke ?? NONE;
    d[o + 8] = f[0];
    d[o + 9] = f[1];
    d[o + 10] = f[2];
    d[o + 11] = f[3];
    d[o + 12] = k[0];
    d[o + 13] = k[1];
    d[o + 14] = k[2];
    d[o + 15] = k[3];
    this.count++;
  }

  rect(x: number, y: number, w: number, h: number, style: Omit<SdfShape, 'cx' | 'cy' | 'hw' | 'hh'>): void {
    this.push({ cx: x + w / 2, cy: y + h / 2, hw: w / 2, hh: h / 2, ...style });
  }

  circle(cx: number, cy: number, r: number, style: Omit<SdfShape, 'cx' | 'cy' | 'hw' | 'hh' | 'radius'>): void {
    this.push({ cx, cy, hw: r, hh: r, radius: r, ...style });
  }

  get size(): number {
    return this.count;
  }

  flush(gl: WebGL2RenderingContext, vw: number, vh: number): void {
    if (this.count === 0) return;
    this.ensure(gl);
    gl.useProgram(this.prog);
    gl.uniform2f(this.uRes, vw, vh);
    gl.bindVertexArray(this.vao);
    gl.bindBuffer(gl.ARRAY_BUFFER, this.inst);
    const bytes = this.count * STRIDE;
    if (bytes > this.capacity) {
      gl.bufferData(gl.ARRAY_BUFFER, this.data.byteLength, gl.DYNAMIC_DRAW);
      this.capacity = this.data.length;
    }
    gl.bufferSubData(gl.ARRAY_BUFFER, 0, this.data, 0, bytes);
    gl.drawArraysInstanced(gl.TRIANGLE_STRIP, 0, 4, this.count);
    gl.bindVertexArray(null);
    this.count = 0;
  }

  dispose(): void {
    const gl = this.gl;
    if (gl && !gl.isContextLost()) {
      if (this.prog) gl.deleteProgram(this.prog);
      if (this.vao) gl.deleteVertexArray(this.vao);
      if (this.quad) gl.deleteBuffer(this.quad);
      if (this.inst) gl.deleteBuffer(this.inst);
    }
    this.gl = null;
    this.prog = null;
    this.vao = null;
  }
}

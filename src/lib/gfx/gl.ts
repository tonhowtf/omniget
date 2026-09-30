// GL context and program helpers shared by the world, the canvas and the
// calibration. One set of context attributes for the whole app (premultiplied
// alpha end to end), plus the textured-quad program every sprite pass uses.

import { ERR, RenderError } from './types';

export type GL = WebGLRenderingContext | WebGL2RenderingContext;

export const GL_ATTRIBUTES: WebGLContextAttributes = {
  alpha: true,
  antialias: false,
  depth: false,
  stencil: false,
  // Premultiplied end to end (textures, blending, the canvas): the webview hands
  // ImageBitmaps over already premultiplied.
  premultipliedAlpha: true,
  preserveDrawingBuffer: false,
  powerPreference: 'high-performance',
  // Left false on purpose: llvmpipe would be rejected on Linux, where it is the
  // only option. The calibration decides the tier by measuring instead.
  failIfMajorPerformanceCaveat: false,
};

export function getGlContext(canvas: HTMLCanvasElement, version: 1 | 2): GL | null {
  if (version === 2) {
    return canvas.getContext('webgl2', GL_ATTRIBUTES) as WebGL2RenderingContext | null;
  }
  return (canvas.getContext('webgl', GL_ATTRIBUTES) ||
    canvas.getContext('experimental-webgl', GL_ATTRIBUTES)) as WebGLRenderingContext | null;
}

export function compileShader(gl: GL, type: number, src: string): WebGLShader {
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

/** Compiles and links a vertex + fragment pair; throws RenderError on failure. */
export function linkProgram(gl: GL, vsSrc: string, fsSrc: string): WebGLProgram {
  const vs = compileShader(gl, gl.VERTEX_SHADER, vsSrc);
  const fs = compileShader(gl, gl.FRAGMENT_SHADER, fsSrc);
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

export const SPRITE_VS2 = `#version 300 es
in vec2 aPos; in vec2 aUV; in vec4 aColor;
uniform vec2 uRes;
out vec2 vUV; out vec4 vColor;
void main() {
  vec2 p = aPos / uRes * 2.0 - 1.0;
  gl_Position = vec4(p.x, -p.y, 0.0, 1.0);
  vUV = aUV; vColor = aColor;
}`;

export const SPRITE_FS2 = `#version 300 es
precision mediump float;
in vec2 vUV; in vec4 vColor;
uniform sampler2D uTex;
out vec4 outColor;
void main() {
  vec4 t = texture(uTex, vUV);
  vec4 c = vec4(t.rgb * vColor.rgb, t.a) * vColor.a;
  if (c.a < 0.004) discard;
  outColor = c;
}`;

const SPRITE_VS1 = `attribute vec2 aPos; attribute vec2 aUV; attribute vec4 aColor;
uniform vec2 uRes;
varying vec2 vUV; varying vec4 vColor;
void main() {
  vec2 p = aPos / uRes * 2.0 - 1.0;
  gl_Position = vec4(p.x, -p.y, 0.0, 1.0);
  vUV = aUV; vColor = aColor;
}`;

const SPRITE_FS1 = `precision mediump float;
varying vec2 vUV; varying vec4 vColor;
uniform sampler2D uTex;
void main() {
  vec4 t = texture2D(uTex, vUV);
  vec4 c = vec4(t.rgb * vColor.rgb, t.a) * vColor.a;
  if (c.a < 0.004) discard;
  gl_FragColor = c;
}`;

/** The textured-quad program (x, y, u, v, rgba per vertex; uRes; uTex). */
export function buildProgram(gl: GL, version: 1 | 2): WebGLProgram {
  return version === 2
    ? linkProgram(gl, SPRITE_VS2, SPRITE_FS2)
    : linkProgram(gl, SPRITE_VS1, SPRITE_FS1);
}

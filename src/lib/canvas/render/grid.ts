import { linkProgram } from '$lib/gfx/gl';
import { GRID } from '../model/types';
import type { FrameInfo, GlPass } from './gl-layer';
import type { Rgba } from './color';

const VS = `#version 300 es
const vec2 P[3] = vec2[3](vec2(-1.0, -1.0), vec2(3.0, -1.0), vec2(-1.0, 3.0));
void main() { gl_Position = vec4(P[gl_VertexID], 0.0, 1.0); }`;

const FS = `#version 300 es
precision highp float;
uniform vec2 uOffset;   // camera top-left modulo the period, world units
uniform float uZoom;
uniform float uDpr;
uniform float uHeight;  // viewport height, CSS px
uniform float uSpacing; // current level spacing, world units
uniform float uFade;    // 0..1 weight of the minor dots
uniform vec4 uColor;
out vec4 outColor;
float dotAt(vec2 world, float spacing, float radiusPx) {
  vec2 cell = mod(world + spacing * 0.5, spacing) - spacing * 0.5;
  float d = length(cell) * uZoom;
  return clamp(radiusPx + 0.5 - d, 0.0, 1.0);
}
void main() {
  vec2 css = vec2(gl_FragCoord.x, uHeight * uDpr - gl_FragCoord.y) / uDpr;
  vec2 world = uOffset + css / uZoom;
  float minor = dotAt(world, uSpacing, 0.9) * uFade;
  float major = dotAt(world, uSpacing * 5.0, 1.35);
  float a = max(minor, major) * uColor.a;
  if (a < 0.003) discard;
  outColor = vec4(uColor.rgb * a, a);
}`;

export function gridLevel(zoom: number): { spacing: number; fade: number } {
  let spacing = GRID;
  while (spacing * zoom < 8 && spacing < 1e7) spacing *= 5;
  const px = spacing * zoom;
  const fade = Math.max(0, Math.min(1, (px - 8) / 8));
  return { spacing, fade };
}

function mod(a: number, n: number): number {
  return ((a % n) + n) % n;
}

export function createGridPass(color: () => Rgba, enabled: () => boolean = () => true): GlPass {
  let gl: WebGL2RenderingContext | null = null;
  let prog: WebGLProgram | null = null;
  let vao: WebGLVertexArrayObject | null = null;
  const u: Record<string, WebGLUniformLocation | null> = {};
  return {
    id: 'canvas.grid',
    order: 0,
    draw(g: WebGL2RenderingContext, frame: FrameInfo) {
      if (!enabled()) return;
      if (gl !== g || !prog) {
        gl = g;
        prog = linkProgram(g, VS, FS);
        vao = g.createVertexArray();
        for (const name of ['uOffset', 'uZoom', 'uDpr', 'uHeight', 'uSpacing', 'uFade', 'uColor']) {
          u[name] = g.getUniformLocation(prog, name);
        }
      }
      const { spacing, fade } = gridLevel(frame.camera.zoom);
      const period = spacing * 25;
      const c = color();
      g.useProgram(prog);
      g.bindVertexArray(vao);
      g.uniform2f(u.uOffset, mod(frame.camera.x, period), mod(frame.camera.y, period));
      g.uniform1f(u.uZoom, frame.camera.zoom);
      g.uniform1f(u.uDpr, frame.viewport.dpr);
      g.uniform1f(u.uHeight, frame.viewport.h);
      g.uniform1f(u.uSpacing, spacing);
      g.uniform1f(u.uFade, fade);
      g.uniform4f(u.uColor, c[0], c[1], c[2], c[3]);
      g.drawArrays(g.TRIANGLES, 0, 3);
      g.bindVertexArray(null);
    },
    dispose() {
      if (gl && !gl.isContextLost()) {
        if (prog) gl.deleteProgram(prog);
        if (vao) gl.deleteVertexArray(vao);
      }
      gl = null;
      prog = null;
    },
  };
}

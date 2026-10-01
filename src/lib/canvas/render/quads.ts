import { buildProgram } from '$lib/gfx/gl';
import { buildIndices, FLOATS_PER_QUAD, INDICES_PER_QUAD, writeQuad, type Quad } from '$lib/gfx/quad';

type TexSource = ImageBitmap | HTMLCanvasElement | OffscreenCanvas | HTMLImageElement;

export class QuadBatch {
  private gl: WebGL2RenderingContext | null = null;
  private prog: WebGLProgram | null = null;
  private vao: WebGLVertexArrayObject | null = null;
  private vbo: WebGLBuffer | null = null;
  private ibo: WebGLBuffer | null = null;
  private uRes: WebGLUniformLocation | null = null;
  private uTex: WebGLUniformLocation | null = null;
  private data = new Float32Array(FLOATS_PER_QUAD * 256);
  private quads = 0;
  private indexQuads = 0;

  private ensure(gl: WebGL2RenderingContext): void {
    if (this.gl === gl && this.prog) return;
    this.gl = gl;
    this.prog = buildProgram(gl, 2);
    this.uRes = gl.getUniformLocation(this.prog, 'uRes');
    this.uTex = gl.getUniformLocation(this.prog, 'uTex');
    this.vao = gl.createVertexArray();
    gl.bindVertexArray(this.vao);
    this.vbo = gl.createBuffer();
    gl.bindBuffer(gl.ARRAY_BUFFER, this.vbo);
    const stride = 8 * 4;
    const attr = (name: string, size: number, offset: number) => {
      const loc = gl.getAttribLocation(this.prog!, name);
      if (loc < 0) return;
      gl.enableVertexAttribArray(loc);
      gl.vertexAttribPointer(loc, size, gl.FLOAT, false, stride, offset * 4);
    };
    attr('aPos', 2, 0);
    attr('aUV', 2, 2);
    attr('aColor', 4, 4);
    this.ibo = gl.createBuffer();
    gl.bindBuffer(gl.ELEMENT_ARRAY_BUFFER, this.ibo);
    this.indexQuads = 0;
    gl.bindVertexArray(null);
  }

  begin(): void {
    this.quads = 0;
  }

  quad(q: Quad): void {
    if ((this.quads + 1) * FLOATS_PER_QUAD > this.data.length) {
      const next = new Float32Array(this.data.length * 2);
      next.set(this.data);
      this.data = next;
    }
    writeQuad(this.data, this.quads * FLOATS_PER_QUAD, q);
    this.quads++;
  }

  get size(): number {
    return this.quads;
  }

  flush(gl: WebGL2RenderingContext, tex: WebGLTexture, vw: number, vh: number): void {
    if (this.quads === 0) return;
    this.ensure(gl);
    gl.useProgram(this.prog);
    gl.uniform2f(this.uRes, vw, vh);
    gl.activeTexture(gl.TEXTURE0);
    gl.bindTexture(gl.TEXTURE_2D, tex);
    gl.uniform1i(this.uTex, 0);
    gl.bindVertexArray(this.vao);
    gl.bindBuffer(gl.ARRAY_BUFFER, this.vbo);
    gl.bufferData(gl.ARRAY_BUFFER, this.data.subarray(0, this.quads * FLOATS_PER_QUAD), gl.DYNAMIC_DRAW);
    if (this.quads > this.indexQuads) {
      const n = Math.max(this.quads, 256);
      gl.bindBuffer(gl.ELEMENT_ARRAY_BUFFER, this.ibo);
      gl.bufferData(gl.ELEMENT_ARRAY_BUFFER, buildIndices(Math.min(n, 16383)), gl.STATIC_DRAW);
      this.indexQuads = Math.min(n, 16383);
    }
    gl.drawElements(gl.TRIANGLES, Math.min(this.quads, this.indexQuads) * INDICES_PER_QUAD, gl.UNSIGNED_SHORT, 0);
    gl.bindVertexArray(null);
    this.quads = 0;
  }

  dispose(): void {
    const gl = this.gl;
    if (gl && !gl.isContextLost()) {
      if (this.prog) gl.deleteProgram(this.prog);
      if (this.vao) gl.deleteVertexArray(this.vao);
      if (this.vbo) gl.deleteBuffer(this.vbo);
      if (this.ibo) gl.deleteBuffer(this.ibo);
    }
    this.gl = null;
    this.prog = null;
  }
}

export function uploadTexture(
  gl: WebGL2RenderingContext,
  src: TexSource,
  existing?: WebGLTexture | null,
  nearest = false,
): WebGLTexture | null {
  const tex = existing ?? gl.createTexture();
  if (!tex) return null;
  gl.bindTexture(gl.TEXTURE_2D, tex);
  gl.pixelStorei(gl.UNPACK_PREMULTIPLY_ALPHA_WEBGL, true);
  gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA, gl.RGBA, gl.UNSIGNED_BYTE, src as TexImageSource);
  gl.pixelStorei(gl.UNPACK_PREMULTIPLY_ALPHA_WEBGL, false);
  const filter = nearest ? gl.NEAREST : gl.LINEAR;
  gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, filter);
  gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, filter);
  gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_S, gl.CLAMP_TO_EDGE);
  gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_T, gl.CLAMP_TO_EDGE);
  return tex;
}

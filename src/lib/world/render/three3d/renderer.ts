// The `three3d` backend: WebGLRenderer, scene, perspective camera, sun +
// sky light, one shadow, fog, and the context lifecycle. The city content
// (regions, kit, people) hangs off `scene` from `city.ts`; this file only owns
// what a lost context can take away and the policy the tier decides.
//
// Lifecycle (estudo 01 §7, context.ts): sleep() loses the context on purpose
// through WEBGL_lose_context; wake() asks for it back, waits, and if the
// driver refuses (macOS with the window occluded) swaps the <canvas> for a
// fresh one and builds a new WebGLRenderer over the same Scene. three keeps
// every buffer and texture on the CPU, so both paths re-upload on the next
// draw. The restore is checked by reading a pixel of a real frame.

import {
  Color,
  DirectionalLight,
  Fog,
  HemisphereLight,
  PCFShadowMap,
  PerspectiveCamera,
  Scene,
  SRGBColorSpace,
  Vector3,
  WebGLRenderer,
  type BufferGeometry,
  type Object3D,
} from 'three';
import { ContextTracker, replaceCanvas, waitForRestore } from '../context';
import { budget3dForTier, type Tier3DBudget } from '../tier';
import {
  ERR,
  RenderError,
  type Caps3D,
  type FrameStats3D,
  type InitOpts3D,
  type Renderer3D,
  type RenderMemory3D,
  type Tier,
} from '../types';
import { createMaterials, type MaterialSet } from './materials';

export const SKY = 0xbfdcef;
const GROUND_SKY = 0x8f8a6f;

export interface ThreeRendererOpts {
  /** Called when wake() had to put a new <canvas> in the DOM. */
  onCanvasReplaced?: (canvas: HTMLCanvasElement) => void;
  /** A context loss we did not ask for (driver reset, GPU hang). */
  onLost?: () => void;
  onRestored?: () => void;
}

export interface ThreeRenderer extends Renderer3D {
  readonly scene: Scene;
  readonly camera: PerspectiveCamera;
  readonly sun: DirectionalLight;
  readonly materials: MaterialSet;
  readonly gl: WebGLRenderer | null;
  readonly budget: Tier3DBudget;
  readonly caps: Caps3D | null;
  /** Keeps the shadow frustum centred on what the camera looks at. */
  followShadow(target: Vector3): void;
  /** Listeners for tier changes (the city re-applies LOD, outline, people). */
  onTier(cb: (tier: Tier) => void): () => void;
}

function makeGl(canvas: HTMLCanvasElement, budget: Tier3DBudget, preserve: boolean): WebGLRenderer {
  const gl = new WebGLRenderer({
    canvas,
    antialias: budget.antialias,
    powerPreference: 'high-performance',
    preserveDrawingBuffer: preserve,
    alpha: false,
    stencil: false,
  });
  gl.outputColorSpace = SRGBColorSpace;
  gl.setClearColor(new Color(SKY), 1);
  gl.shadowMap.type = PCFShadowMap;
  gl.shadowMap.enabled = budget.shadowMap > 0;
  // Our loop owns the frame; three must not reset stats per render call.
  gl.info.autoReset = false;
  return gl;
}

function geometryBytes(g: BufferGeometry): number {
  let n = 0;
  for (const name of Object.keys(g.attributes)) {
    const a = g.attributes[name] as { array?: ArrayLike<number> & { byteLength?: number } };
    n += a.array?.byteLength ?? 0;
  }
  const idx = g.index as { array?: { byteLength?: number } } | null;
  n += idx?.array?.byteLength ?? 0;
  return n;
}

export function createThreeRenderer(opts: ThreeRendererOpts = {}): ThreeRenderer {
  const scene = new Scene();
  scene.background = new Color(SKY);
  scene.fog = new Fog(SKY, 80, 400);
  const camera = new PerspectiveCamera(42, 16 / 9, 0.5, 600);
  camera.position.set(60, 70, 110);
  camera.lookAt(64, 0, 64);
  const hemi = new HemisphereLight(0xfdf6e3, GROUND_SKY, 1.25);
  const sun = new DirectionalLight(0xfff1d6, 2.2);
  sun.position.set(60, 90, 30);
  sun.target.position.set(0, 0, 0);
  scene.add(hemi, sun, sun.target);
  const materials = createMaterials();

  const ctx = new ContextTracker();
  let canvas: HTMLCanvasElement | null = null;
  let gl: WebGLRenderer | null = null;
  let tier: Tier = 1;
  let budget = budget3dForTier(1);
  let dpr = 1;
  let width = 960;
  let height = 540;
  let preserve = false;
  let dirty = true;
  let sleeping = false;
  let compileMs = 0;
  const tierListeners = new Set<(t: Tier) => void>();
  let caps: Caps3D | null = null;

  function applyBudget(): void {
    budget = budget3dForTier(tier);
    camera.far = budget.drawDistance + 40;
    camera.updateProjectionMatrix();
    const fog = scene.fog as Fog;
    fog.near = budget.drawDistance * 0.45;
    fog.far = budget.drawDistance;
    const shadow = budget.shadowMap > 0;
    sun.castShadow = shadow;
    if (shadow) {
      sun.shadow.mapSize.set(budget.shadowMap, budget.shadowMap);
      // The map follows the camera target; its half-size covers the detail ring.
      const half = Math.min(140, Math.max(48, budget.detailDistance));
      Object.assign(sun.shadow.camera, { left: -half, right: half, top: half, bottom: -half, near: 1, far: 400 });
      sun.shadow.camera.updateProjectionMatrix();
      sun.shadow.bias = -0.0006;
      sun.shadow.normalBias = 0.04;
      // A new size needs a new map.
      sun.shadow.map?.dispose();
      sun.shadow.map = null;
    }
    if (gl) {
      gl.shadowMap.enabled = shadow;
      gl.shadowMap.needsUpdate = true;
      gl.setPixelRatio(Math.min(dpr, budget.maxDpr));
      gl.setSize(width, height, false);
    }
    // The shadow define changes the program: materials must recompile.
    for (const m of materials.all()) m.needsUpdate = true;
    dirty = true;
  }

  function info(): Caps3D {
    const g = gl!.getContext();
    const dbg = g.getExtension('WEBGL_debug_renderer_info');
    return {
      backend: 'three3d',
      tier,
      dpr: Math.min(dpr, budget.maxDpr),
      maxTextureSize: g.getParameter(g.MAX_TEXTURE_SIZE) as number,
      webgl2: typeof WebGL2RenderingContext !== 'undefined' && g instanceof WebGL2RenderingContext,
      gpu: dbg ? String(g.getParameter(dbg.UNMASKED_RENDERER_WEBGL)) : String(g.getParameter(g.RENDERER)),
      compileMs,
    };
  }

  function attachTracker(c: HTMLCanvasElement): void {
    ctx.attach(c, {
      onLost: () => {
        dirty = true;
        opts.onLost?.();
      },
      onRestored: () => {
        dirty = true;
        opts.onRestored?.();
      },
    });
  }

  function readPixel(x: number, y: number): [number, number, number, number] | null {
    if (!gl) return null;
    const g = gl.getContext();
    if (g.isContextLost()) return null;
    const buf = new Uint8Array(4);
    const px = Math.max(0, Math.min(g.drawingBufferWidth - 1, Math.round(x)));
    const py = Math.max(0, Math.min(g.drawingBufferHeight - 1, g.drawingBufferHeight - 1 - Math.round(y)));
    g.readPixels(px, py, 1, 1, g.RGBA, g.UNSIGNED_BYTE, buf);
    return [buf[0], buf[1], buf[2], buf[3]];
  }

  /** Draws a frame and reads it back in the same task: alpha 255 = alive. */
  function frameIsAlive(): boolean {
    if (!gl) return false;
    try {
      gl.render(scene, camera);
      const g = gl.getContext();
      const p = readPixel(g.drawingBufferWidth / 2, g.drawingBufferHeight / 2);
      return !!p && p[3] === 255 && !g.isContextLost();
    } catch {
      return false;
    }
  }

  const renderer: ThreeRenderer = {
    scene,
    camera,
    sun,
    materials,
    get gl() {
      return gl;
    },
    get budget() {
      return budget;
    },
    get caps() {
      return caps;
    },
    get tier() {
      return tier;
    },
    get canvas() {
      return canvas;
    },

    async init(c: HTMLCanvasElement, o: InitOpts3D): Promise<Caps3D> {
      canvas = c;
      tier = o.tier;
      dpr = o.dpr ?? (typeof window !== 'undefined' ? window.devicePixelRatio || 1 : 1);
      preserve = !!o.preserveDrawingBuffer;
      budget = budget3dForTier(tier);
      width = c.clientWidth || c.width || 960;
      height = c.clientHeight || c.height || 540;
      try {
        gl = makeGl(c, budget, preserve);
      } catch (e) {
        throw new RenderError(ERR.NO_WEBGL, e instanceof Error ? e.message : String(e));
      }
      attachTracker(c);
      applyBudget();
      caps = info();
      return caps;
    },

    async compile(): Promise<number> {
      if (!gl) throw new RenderError(ERR.NOT_INITIALISED, 'compile before init');
      const t0 = performance.now();
      // compileAsync uses KHR_parallel_shader_compile when present, so the
      // WebKit first frame (1.5-6.4 s measured with toon + shadow) moves to
      // the loading screen instead of freezing the first draw.
      await gl.compileAsync(scene, camera);
      compileMs = performance.now() - t0;
      if (caps) caps.compileMs = compileMs;
      dirty = true;
      return compileMs;
    },

    invalidate() {
      dirty = true;
    },

    frame(_dtMs: number, force = false): FrameStats3D {
      if (!gl || sleeping || ctx.state === 'lost') {
        return { cpuMs: 0, drawCalls: 0, triangles: 0, instances: 0, rendered: false };
      }
      if (!dirty && !force) return { cpuMs: 0, drawCalls: 0, triangles: 0, instances: 0, rendered: false };
      const t0 = performance.now();
      gl.info.reset();
      gl.render(scene, camera);
      const cpuMs = performance.now() - t0;
      dirty = false;
      let instances = 0;
      scene.traverseVisible((o: Object3D) => {
        const im = o as Object3D & { isInstancedMesh?: boolean; count?: number };
        // Kit instances only: outlines share them and people are counted apart.
        if (im.isInstancedMesh && im.userData.kind === 'instances') instances += im.count ?? 0;
      });
      return {
        cpuMs,
        drawCalls: gl.info.render.calls,
        triangles: gl.info.render.triangles,
        instances,
        rendered: true,
      };
    },

    resize(w: number, h: number, d: number): void {
      width = Math.max(1, Math.round(w));
      height = Math.max(1, Math.round(h));
      dpr = d;
      camera.aspect = width / height;
      camera.updateProjectionMatrix();
      if (gl) {
        gl.setPixelRatio(Math.min(dpr, budget.maxDpr));
        gl.setSize(width, height, false);
      }
      if (caps) caps.dpr = Math.min(dpr, budget.maxDpr);
      dirty = true;
    },

    setTier(t: Tier): void {
      if (t === tier) return;
      tier = t;
      // MSAA is a context attribute: a lower tier keeps the old context
      // (cheaper than a rebuild); the flag only matters on the next init.
      applyBudget();
      if (caps) caps.tier = t;
      for (const cb of tierListeners) cb(t);
    },

    onTier(cb) {
      tierListeners.add(cb);
      return () => tierListeners.delete(cb);
    },

    followShadow(target: Vector3): void {
      if (!sun.castShadow) {
        sun.position.set(target.x + 60, 90, target.z + 30);
        sun.target.position.copy(target);
        sun.target.updateMatrixWorld();
        return;
      }
      // Snap to shadow texels so the edges do not crawl while the camera moves.
      const half = (sun.shadow.camera.right - sun.shadow.camera.left) / 2;
      const texel = (half * 2) / Math.max(1, sun.shadow.mapSize.x);
      const sx = Math.round(target.x / texel) * texel;
      const sz = Math.round(target.z / texel) * texel;
      sun.position.set(sx + 60, 90, sz + 30);
      sun.target.position.set(sx, target.y, sz);
      sun.target.updateMatrixWorld();
    },

    memory(): RenderMemory3D {
      const geos = new Set<BufferGeometry>();
      let bytes = 0;
      scene.traverse((o: Object3D) => {
        const m = o as Object3D & {
          geometry?: BufferGeometry;
          isInstancedMesh?: boolean;
          instanceMatrix?: { array: Float32Array };
          instanceColor?: { array: Float32Array } | null;
        };
        if (m.geometry && !geos.has(m.geometry)) {
          geos.add(m.geometry);
          bytes += geometryBytes(m.geometry);
        }
        if (m.isInstancedMesh) {
          bytes += m.instanceMatrix?.array.byteLength ?? 0;
          bytes += m.instanceColor?.array.byteLength ?? 0;
        }
      });
      if (budget.shadowMap > 0) bytes += budget.shadowMap * budget.shadowMap * 4;
      bytes += 128 * 128 + 3; // brush noise + ramp
      const g = gl?.info.memory;
      return {
        geometries: g?.geometries ?? geos.size,
        textures: g?.textures ?? 0,
        programs: gl?.info.programs?.length ?? 0,
        bytes,
      };
    },

    probe(x: number, y: number) {
      return readPixel(x, y);
    },

    sleep(): void {
      if (sleeping || !gl) return;
      sleeping = true;
      // Through three, so it caches WEBGL_lose_context for the restore.
      ctx.loseDeliberately(null);
      try {
        gl.forceContextLoss();
      } catch {
        // Already lost; the tracker already says so.
      }
    },

    async wake(): Promise<void> {
      if (!canvas || !gl) throw new RenderError(ERR.NOT_INITIALISED, 'wake before init');
      const g = gl.getContext();
      // WEBGL_lose_context has to be the object fetched before the loss: a lost
      // context answers getExtension() with null, so asking again (what
      // restoreDeliberately does) never restores. three cached it in sleep().
      try {
        gl.forceContextRestore();
      } catch {
        // Not lost after all, or no extension: the pixel check decides.
      }
      ctx.restoreDeliberately(null);
      // three re-initialises its GL state on webglcontextrestored and
      // re-uploads everything still on the CPU on the next draw.
      if (g.isContextLost()) await waitForRestore(g, 3000);
      let alive = !g.isContextLost() && frameIsAlive();
      if (!alive) {
        // The driver kept the context (macOS, occluded window): new canvas,
        // new WebGLRenderer, same scene.
        const replacement = replaceCanvas(canvas);
        if (!replacement) throw new RenderError(ERR.CONTEXT_LOST, 'context did not come back');
        ctx.detach();
        try {
          gl.dispose();
        } catch {
          // The old context is gone; its listeners are what dispose() removes.
        }
        canvas = replacement;
        gl = makeGl(canvas, budget, preserve);
        attachTracker(canvas);
        gl.setPixelRatio(Math.min(dpr, budget.maxDpr));
        gl.setSize(width, height, false);
        for (const m of materials.all()) m.needsUpdate = true;
        opts.onCanvasReplaced?.(canvas);
        alive = frameIsAlive();
        if (!alive) throw new RenderError(ERR.CONTEXT_LOST, 'context did not come back');
      }
      sleeping = false;
      dirty = true;
      ctx.apply('restored');
      if (caps) caps = { ...info(), compileMs };
    },

    destroy(): void {
      ctx.apply('destroy');
      ctx.detach();
      scene.traverse((o: Object3D) => {
        const m = o as Object3D & { geometry?: BufferGeometry; dispose?: () => void; isInstancedMesh?: boolean };
        m.geometry?.dispose();
        if (m.isInstancedMesh) m.dispose?.();
      });
      materials.dispose();
      sun.shadow.map?.dispose();
      if (gl) {
        try {
          gl.forceContextLoss();
        } catch {
          // Already lost.
        }
        gl.dispose();
      }
      gl = null;
      canvas = null;
      tierListeners.clear();
    },
  };

  return renderer;
}

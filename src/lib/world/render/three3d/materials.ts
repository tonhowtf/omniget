// Toon materials of the city. One program family for everything built from
// the kit: MeshToonMaterial + a 3-tone ramp + a procedural brush-noise
// texture sampled in world space + a small rim light. Colour never comes from
// a texture: instances carry it (`instanceColor`), terrain carries it per
// vertex. Slots share one program (same defines, same cache key); only
// uniforms differ, so the variant count stays at: instanced, vertex-coloured,
// outline, and the depth variants three makes for shadows.

import {
  BackSide,
  Color,
  DataTexture,
  LinearFilter,
  LinearMipmapLinearFilter,
  MeshBasicMaterial,
  MeshToonMaterial,
  NearestFilter,
  NoColorSpace,
  RedFormat,
  RepeatWrapping,
  type Material,
  type Texture,
  type WebGLProgramParametersWithUniforms,
} from 'three';
import { PALETTE_SLOTS, type PaletteSlot } from '$lib/city3d/types';

/** Per-slot look: how much brush, how much rim. Everything else is colour. */
const SLOT_LOOK: Record<string, { brush: number; rim: number }> = {
  wall: { brush: 0.18, rim: 0.08 },
  roof: { brush: 0.22, rim: 0.06 },
  trim: { brush: 0.1, rim: 0.1 },
  wood: { brush: 0.26, rim: 0.05 },
  stone: { brush: 0.3, rim: 0.05 },
  glass: { brush: 0.04, rim: 0.28 },
  door: { brush: 0.2, rim: 0.06 },
  ground: { brush: 0.28, rim: 0 },
  grass: { brush: 0.3, rim: 0 },
  water: { brush: 0.08, rim: 0.2 },
  street: { brush: 0.24, rim: 0 },
  plaza: { brush: 0.2, rim: 0 },
  leaf: { brush: 0.32, rim: 0.1 },
  trunk: { brush: 0.26, rim: 0.04 },
  metal: { brush: 0.06, rim: 0.24 },
  cloth: { brush: 0.2, rim: 0.08 },
};

const NOISE_SIZE = 128;

/** Deterministic integer hash → [0, 1). */
function hash01(x: number, y: number, seed: number): number {
  let h = (x * 374761393 + y * 668265263 + seed * 2246822519) | 0;
  h = Math.imul(h ^ (h >>> 13), 1274126177);
  h ^= h >>> 16;
  return (h >>> 0) / 4294967296;
}

/**
 * Tileable value noise with a few octaves, stretched along x so it reads as
 * brush strokes rather than grain. Built once in code; no asset.
 */
export function buildBrushNoise(size = NOISE_SIZE): Uint8Array {
  const out = new Uint8Array(size * size);
  const octaves = [
    { cells: 8, amp: 0.5, stretch: 3 },
    { cells: 16, amp: 0.3, stretch: 2 },
    { cells: 32, amp: 0.2, stretch: 1 },
  ];
  const smooth = (t: number) => t * t * (3 - 2 * t);
  for (let y = 0; y < size; y++) {
    for (let x = 0; x < size; x++) {
      let v = 0;
      for (let o = 0; o < octaves.length; o++) {
        const { cells, amp, stretch } = octaves[o];
        const cx = Math.max(1, Math.round(cells / stretch));
        const cy = cells;
        const fx = (x / size) * cx;
        const fy = (y / size) * cy;
        const x0 = Math.floor(fx);
        const y0 = Math.floor(fy);
        const tx = smooth(fx - x0);
        const ty = smooth(fy - y0);
        const g = (i: number, j: number) => hash01(((i % cx) + cx) % cx, ((j % cy) + cy) % cy, o + 7);
        const a = g(x0, y0) + (g(x0 + 1, y0) - g(x0, y0)) * tx;
        const b = g(x0, y0 + 1) + (g(x0 + 1, y0 + 1) - g(x0, y0 + 1)) * tx;
        v += (a + (b - a) * ty) * amp;
      }
      out[y * size + x] = Math.max(0, Math.min(255, Math.round(v * 255)));
    }
  }
  return out;
}

function makeBrushTexture(): DataTexture {
  const tex = new DataTexture(buildBrushNoise(), NOISE_SIZE, NOISE_SIZE, RedFormat);
  tex.wrapS = RepeatWrapping;
  tex.wrapT = RepeatWrapping;
  tex.magFilter = LinearFilter;
  tex.minFilter = LinearMipmapLinearFilter;
  tex.generateMipmaps = true;
  tex.colorSpace = NoColorSpace;
  tex.needsUpdate = true;
  return tex;
}

/** 3 tones: shade, mid, lit. Nearest filtering keeps the bands hard. */
function makeRamp(): DataTexture {
  const tex = new DataTexture(new Uint8Array([90, 170, 255]), 3, 1, RedFormat);
  tex.magFilter = NearestFilter;
  tex.minFilter = NearestFilter;
  tex.generateMipmaps = false;
  tex.colorSpace = NoColorSpace;
  tex.needsUpdate = true;
  return tex;
}

const VERT_DECL = /* glsl */ `
varying vec3 vBrushPos;
varying vec3 vBrushN;
`;

const VERT_BODY = /* glsl */ `
#ifdef USE_INSTANCING
  mat4 brushM = modelMatrix * instanceMatrix;
#else
  mat4 brushM = modelMatrix;
#endif
  vBrushPos = (brushM * vec4(transformed, 1.0)).xyz;
  vBrushN = normalize(mat3(brushM) * objectNormal);
`;

const FRAG_DECL = /* glsl */ `
uniform sampler2D uBrush;
uniform float uBrushAmount;
uniform float uBrushScale;
uniform float uRim;
varying vec3 vBrushPos;
varying vec3 vBrushN;
`;

// Triplanar-ish: the top face reads xz, the sides read (x+z, y).
const FRAG_BRUSH = /* glsl */ `
  {
    vec3 an = abs(normalize(vBrushN));
    float top = texture2D(uBrush, vBrushPos.xz * uBrushScale).r;
    float side = texture2D(uBrush, vec2(vBrushPos.x + vBrushPos.z, vBrushPos.y) * uBrushScale).r;
    float n = mix(side, top, smoothstep(0.55, 0.85, an.y));
    diffuseColor.rgb *= 1.0 + (n - 0.5) * 2.0 * uBrushAmount;
  }
`;

const FRAG_RIM = /* glsl */ `
  {
    float rimF = 1.0 - max(dot(normalize(normal), normalize(vViewPosition)), 0.0);
    gl_FragColor.rgb += uRim * pow(rimF, 3.0) * vec3(1.0, 0.96, 0.88);
  }
`;

export interface MaterialSet {
  /** Toon material of a palette slot, for instanced kit meshes. */
  slot(slot: string): MeshToonMaterial;
  /** Vertex-coloured toon (terrain, fallback meshes with baked colour). */
  readonly terrain: MeshToonMaterial;
  /** Inverted hull, shared by every outlined mesh. */
  readonly outline: MeshBasicMaterial;
  /** Capsules and other plain props. */
  readonly plain: MeshToonMaterial;
  /** Everything, for compile/dispose. */
  all(): Material[];
  setOutlineWidth(metres: number): void;
  dispose(): void;
}

function patchToon(mat: MeshToonMaterial, brush: Texture, look: { brush: number; rim: number }) {
  const uniforms = {
    uBrush: { value: brush },
    uBrushAmount: { value: look.brush },
    uBrushScale: { value: 0.35 },
    uRim: { value: look.rim },
  };
  mat.userData.brushUniforms = uniforms;
  mat.onBeforeCompile = (shader: WebGLProgramParametersWithUniforms) => {
    Object.assign(shader.uniforms, uniforms);
    shader.vertexShader = VERT_DECL + shader.vertexShader.replace('#include <project_vertex>', `#include <project_vertex>\n${VERT_BODY}`);
    shader.fragmentShader =
      FRAG_DECL +
      shader.fragmentShader
        .replace('#include <color_fragment>', `#include <color_fragment>\n${FRAG_BRUSH}`)
        .replace('#include <opaque_fragment>', `#include <opaque_fragment>\n${FRAG_RIM}`);
  };
  // Same source for every slot: one program, many uniform sets.
  mat.customProgramCacheKey = () => 'omniget-toon-1';
}

export function createMaterials(): MaterialSet {
  const brush = makeBrushTexture();
  const ramp = makeRamp();
  const bySlot = new Map<string, MeshToonMaterial>();

  const make = (look: { brush: number; rim: number }, name: string, vertexColors = false) => {
    const m = new MeshToonMaterial({ color: 0xffffff, gradientMap: ramp, vertexColors });
    m.name = name;
    patchToon(m, brush, look);
    return m;
  };

  const terrain = make({ brush: 0.28, rim: 0 }, 'toon:terrain', true);
  const plain = make({ brush: 0.12, rim: 0.12 }, 'toon:plain');
  const outline = new MeshBasicMaterial({ color: new Color(0x1d1a24), side: BackSide });
  outline.name = 'outline';
  const outlineUniforms = { uOutline: { value: 0.045 } };
  outline.onBeforeCompile = (shader: WebGLProgramParametersWithUniforms) => {
    Object.assign(shader.uniforms, outlineUniforms);
    shader.vertexShader =
      'uniform float uOutline;\n' +
      shader.vertexShader.replace('#include <begin_vertex>', '#include <begin_vertex>\n  transformed += normalize(normal) * uOutline;');
  };
  outline.customProgramCacheKey = () => 'omniget-outline-1';

  return {
    slot(slot: string) {
      const key = (PALETTE_SLOTS as readonly string[]).includes(slot) ? slot : 'wall';
      let m = bySlot.get(key);
      if (!m) {
        m = make(SLOT_LOOK[key] ?? SLOT_LOOK.wall, `toon:${key as PaletteSlot}`);
        bySlot.set(key, m);
      }
      return m;
    },
    terrain,
    outline,
    plain,
    all() {
      return [terrain, plain, outline, ...bySlot.values()];
    },
    setOutlineWidth(metres: number) {
      outlineUniforms.uOutline.value = metres;
    },
    dispose() {
      for (const m of bySlot.values()) m.dispose();
      bySlot.clear();
      terrain.dispose();
      plain.dispose();
      outline.dispose();
      brush.dispose();
      ramp.dispose();
    },
  };
}

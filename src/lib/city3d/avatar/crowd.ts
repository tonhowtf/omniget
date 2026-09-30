// Far tier: GPU-skinned instancing. three has no InstancedSkinnedMesh, so this
// is one: per body archetype, an InstancedMesh of the 369-vertex LOD whose
// vertex shader skins with bone matrices baked into a float texture (3 texels
// per bone, one row per clip frame). Per instance: clip rows and frame, and
// the palette colors picked by vertex region.
import * as THREE from 'three';
import { buildBody, type AnnyBase } from './anny';
import { Animator } from './animator';
import type { Clip } from './clips';
import { toonGradient } from './materials';
import { buildBones, measureHead, vertexRegions } from './rig';

export const CROWD_CLIPS = ['idle', 'walk', 'sit', 'talk', 'water', 'plant', 'harvest', 'carry', 'sleep', 'work'];

export interface CrowdInstance {
  archetype: string;
  position: THREE.Vector3;
  yaw: number;
  scale: THREE.Vector3;
  clip: string;
  /** seconds into the clip */
  time: number;
  colors: { skin: number[]; top: number[]; bottom: number[]; shoes: number[]; hair: number[] };
  cover: [number, number, number, number];
}

interface Archetype {
  key: string;
  mesh: THREE.InstancedMesh;
  tex: THREE.DataTexture;
  rows: Map<string, { start: number; count: number; fps: number; loop: boolean }>;
  height: number;
  ids: string[];
  attrs: Record<string, THREE.InstancedBufferAttribute>;
  capacity: number;
}

const ARCHETYPE_PHENO: Record<string, number[]> = {};
for (const [g, gv] of [['m', 0.1], ['f', 0.9]] as const) {
  for (const [a, av] of [['child', 0.42], ['adult', 0.72], ['elder', 0.95]] as const) {
    ARCHETYPE_PHENO[`${g}-${a}`] = [gv, av, 0.5, 0.45, a === 'child' ? 0.5 : 0.4, 0.15];
  }
}

export function archetypeOf(ph: number[]): string {
  const g = ph[0] < 0.5 ? 'm' : 'f';
  const a = ph[1] < 0.56 ? 'child' : ph[1] < 0.88 ? 'adult' : 'elder';
  return `${g}-${a}`;
}

const VERT_HEAD = /* glsl */ `
uniform highp sampler2D boneAnimTex;
attribute vec4 aJoints;
attribute vec4 aWeights;
attribute float aRegion;
attribute vec4 aAnim;
// packed 8-bit linear colors: skin, top, bottom, shoes / hair (-1 = none) and
// the region bits of top, bottom and shoes (16 vertex attributes is the limit)
attribute vec4 aPal0;
attribute vec4 aPal1;
vec3 unpackRGB(float f) {
  float r = floor(f / 65536.0);
  float g = floor((f - r * 65536.0) / 256.0);
  return vec3(r, g, f - r * 65536.0 - g * 256.0) / 255.0;
}
mat4 animBone(float row, float bone) {
  int x = int(bone + 0.5) * 3;
  int y = int(row + 0.5);
  vec4 r0 = texelFetch(boneAnimTex, ivec2(x, y), 0);
  vec4 r1 = texelFetch(boneAnimTex, ivec2(x + 1, y), 0);
  vec4 r2 = texelFetch(boneAnimTex, ivec2(x + 2, y), 0);
  return mat4(vec4(r0.x, r1.x, r2.x, 0.0), vec4(r0.y, r1.y, r2.y, 0.0), vec4(r0.z, r1.z, r2.z, 0.0), vec4(r0.w, r1.w, r2.w, 1.0));
}
`;

const VERT_SKIN_NORMAL = /* glsl */ `
float fa = floor(aAnim.z);
float fb = aAnim.w > 0.5 ? mod(fa + 1.0, aAnim.y) : min(fa + 1.0, aAnim.y - 1.0);
float fr = aAnim.z - fa;
float ra = aAnim.x + fa;
float rb = aAnim.x + fb;
mat4 crowdSkin = mat4(0.0);
crowdSkin += aWeights.x * (animBone(ra, aJoints.x) * (1.0 - fr) + animBone(rb, aJoints.x) * fr);
crowdSkin += aWeights.y * (animBone(ra, aJoints.y) * (1.0 - fr) + animBone(rb, aJoints.y) * fr);
crowdSkin += aWeights.z * (animBone(ra, aJoints.z) * (1.0 - fr) + animBone(rb, aJoints.z) * fr);
crowdSkin += aWeights.w * (animBone(ra, aJoints.w) * (1.0 - fr) + animBone(rb, aJoints.w) * fr);
objectNormal = normalize(mat3(crowdSkin) * objectNormal);
`;

const VERT_SKIN = /* glsl */ `
transformed = (crowdSkin * vec4(transformed, 1.0)).xyz;
`;

const VERT_COLOR = /* glsl */ `
{
  int r = int(aRegion + 0.5);
  int bit = 1 << r;
  vec3 c = unpackRGB(aPal0.x);
  if (r == 1 && aPal1.x >= 0.0) c = unpackRGB(aPal1.x);
  if ((int(aPal1.z) & bit) != 0) c = unpackRGB(aPal0.z);
  if ((int(aPal1.y) & bit) != 0) c = unpackRGB(aPal0.y);
  if ((int(aPal1.w) & bit) != 0) c = unpackRGB(aPal0.w);
  if (r == 2) c = vec3(0.05);
  vColor.rgb = c;
}
`;

export class Crowd {
  group = new THREE.Group();
  private archetypes = new Map<string, Archetype>();
  private where = new Map<string, { a: Archetype; slot: number }>();

  constructor(private base: AnnyBase, private clips: Map<string, Clip>) {
    this.group.name = 'residents-crowd';
  }

  get count() {
    return this.where.size;
  }

  archetypeHeight(key: string) {
    return this.ensure(key).height;
  }

  private ensure(key: string): Archetype {
    let a = this.archetypes.get(key);
    if (a) return a;
    const lod = this.base.lods['369'];
    const ph = ARCHETYPE_PHENO[key] ?? ARCHETYPE_PHENO['m-adult'];
    const body = buildBody(this.base, '369', ph);
    const head = measureHead(lod, body);
    const regions = vertexRegions(this.base, lod, body, head);
    const geo = new THREE.BufferGeometry();
    geo.setAttribute('position', new THREE.BufferAttribute(new Float32Array(body.positions), 3));
    geo.setAttribute('aJoints', new THREE.BufferAttribute(Float32Array.from(lod.joints), 4));
    geo.setAttribute('aWeights', new THREE.BufferAttribute(new Float32Array(lod.weights), 4));
    geo.setAttribute('aRegion', new THREE.BufferAttribute(Float32Array.from(regions), 1));
        geo.setIndex(Array.from(lod.indices));
    geo.computeVertexNormals();
    geo.boundingSphere = new THREE.Sphere(new THREE.Vector3(0, body.height / 2, 0), 1e5);

    // bake the bone matrices of every crowd clip
    const bones = buildBones(this.base, body);
    const root = new THREE.Group();
    root.add(bones[0]);
    root.updateMatrixWorld(true);
    const inverses = bones.map((b) => b.matrixWorld.clone().invert());
    const hipsRest = bones[0].position.clone();
    const rows = new Map<string, { start: number; count: number; fps: number; loop: boolean }>();
    const frames: Float32Array[] = [];
    const anim = new Animator(this.clips);
    const m = new THREE.Matrix4();
    for (const name of CROWD_CLIPS) {
      const clip = this.clips.get(name);
      if (!clip) continue;
      rows.set(name, { start: frames.length, count: clip.frames, fps: clip.fps, loop: clip.loop });
      anim.base = { clip, t: 0, speed: 1 };
      anim.prev = null;
      for (let f = 0; f < clip.frames; f++) {
        anim.base.t = f / clip.fps;
        anim.step(0);
        anim.apply(bones, hipsRest);
        root.updateMatrixWorld(true);
        const row = new Float32Array(bones.length * 12);
        bones.forEach((b, i) => {
          m.multiplyMatrices(b.matrixWorld, inverses[i]);
          const e = m.elements; // column-major
          row.set([e[0], e[4], e[8], e[12], e[1], e[5], e[9], e[13], e[2], e[6], e[10], e[14]], i * 12);
        });
        frames.push(row);
      }
    }
    const width = bones.length * 3;
    const data = new Float32Array(width * 4 * Math.max(1, frames.length));
    frames.forEach((r, i) => data.set(r, i * width * 4));
    const tex = new THREE.DataTexture(data, width, Math.max(1, frames.length), THREE.RGBAFormat, THREE.FloatType);
    tex.minFilter = THREE.NearestFilter;
    tex.magFilter = THREE.NearestFilter;
    tex.generateMipmaps = false;
    tex.needsUpdate = true;

    const mat = new THREE.MeshToonMaterial({ vertexColors: true, gradientMap: toonGradient() });
    mat.name = 'resident-crowd';
    mat.onBeforeCompile = (shader) => {
      shader.uniforms.boneAnimTex = { value: tex };
      shader.vertexShader = shader.vertexShader
        .replace('#include <common>', '#include <common>\n' + VERT_HEAD)
        .replace('#include <skinnormal_vertex>', VERT_SKIN_NORMAL)
        .replace('#include <skinning_vertex>', VERT_SKIN)
        .replace('#include <color_vertex>', VERT_COLOR);
    };
    mat.customProgramCacheKey = () => 'resident-crowd';
    a = { key, mesh: null as unknown as THREE.InstancedMesh, tex, rows, height: body.height, ids: [], attrs: {}, capacity: 0 };
    this.resize(a, 64, geo, mat);
    this.archetypes.set(key, a);
    return a;
  }

  private resize(a: Archetype, capacity: number, geo?: THREE.BufferGeometry, mat?: THREE.Material) {
    const g = geo ?? a.mesh.geometry;
    const m = mat ?? (a.mesh.material as THREE.Material);
    const mesh = new THREE.InstancedMesh(g, m, capacity);
    mesh.name = `crowd-${a.key}`;
    mesh.frustumCulled = false;
    mesh.instanceMatrix.setUsage(THREE.DynamicDrawUsage);
    const sizes: Record<string, number> = { aAnim: 4, aPal0: 4, aPal1: 4 };
    const attrs: Record<string, THREE.InstancedBufferAttribute> = {};
    for (const [name, size] of Object.entries(sizes)) {
      const arr = new Float32Array(capacity * size);
      const old = a.attrs[name];
      if (old) arr.set((old.array as Float32Array).subarray(0, Math.min(old.array.length, arr.length)));
      const attr = new THREE.InstancedBufferAttribute(arr, size);
      attr.setUsage(THREE.DynamicDrawUsage);
      g.setAttribute(name, attr);
      attrs[name] = attr;
    }
    if (a.mesh) {
      mesh.instanceMatrix.array.set((a.mesh.instanceMatrix.array as Float32Array).subarray(0, a.capacity * 16));
      a.mesh.removeFromParent();
      a.mesh.dispose();
    }
    mesh.count = a.ids.length;
    a.mesh = mesh;
    a.attrs = attrs;
    a.capacity = capacity;
    this.group.add(mesh);
  }

  set(id: string, inst: CrowdInstance) {
    const cur = this.where.get(id);
    if (cur && cur.a.key !== inst.archetype) this.remove(id);
    let w = this.where.get(id);
    if (!w) {
      const a = this.ensure(inst.archetype);
      if (a.ids.length >= a.capacity) this.resize(a, a.capacity * 2);
      w = { a, slot: a.ids.length };
      a.ids.push(id);
      a.mesh.count = a.ids.length;
      this.where.set(id, w);
    }
    const { a, slot } = w;
    const r = a.rows.get(inst.clip) ?? a.rows.get('idle')!;
    const f = inst.time * r.fps;
    const frame = r.loop ? ((f % r.count) + r.count) % r.count : Math.min(r.count - 1, Math.max(0, f));
    a.attrs.aAnim.setXYZW(slot, r.start, r.count, frame, r.loop ? 1 : 0);
    const c = inst.colors;
    a.attrs.aPal0.setXYZW(slot, packRGB(c.skin), packRGB(c.top), packRGB(c.bottom), packRGB(c.shoes));
    a.attrs.aPal1.setXYZW(slot, inst.cover[3] ? packRGB(c.hair) : -1, inst.cover[0], inst.cover[1], inst.cover[2]);
    _m.compose(inst.position, _q.setFromAxisAngle(_up, inst.yaw), inst.scale);
    a.mesh.setMatrixAt(slot, _m);
    for (const attr of Object.values(a.attrs)) attr.needsUpdate = true;
    a.mesh.instanceMatrix.needsUpdate = true;
  }

  remove(id: string) {
    const w = this.where.get(id);
    if (!w) return;
    const { a, slot } = w;
    const last = a.ids.length - 1;
    if (slot !== last) {
      const moved = a.ids[last];
      a.ids[slot] = moved;
      a.mesh.getMatrixAt(last, _m);
      a.mesh.setMatrixAt(slot, _m);
      for (const attr of Object.values(a.attrs)) {
        for (let k = 0; k < attr.itemSize; k++) attr.array[slot * attr.itemSize + k] = attr.array[last * attr.itemSize + k];
        attr.needsUpdate = true;
      }
      this.where.set(moved, { a, slot });
    }
    a.ids.pop();
    a.mesh.count = a.ids.length;
    a.mesh.instanceMatrix.needsUpdate = true;
    this.where.delete(id);
  }

  has(id: string) {
    return this.where.has(id);
  }

  dispose() {
    for (const a of this.archetypes.values()) {
      a.mesh.removeFromParent();
      a.mesh.geometry.dispose();
      (a.mesh.material as THREE.Material).dispose();
      a.mesh.dispose();
      a.tex.dispose();
    }
    this.archetypes.clear();
    this.where.clear();
    this.group.removeFromParent();
  }
}

function packRGB(c: number[]): number {
  const q = (v: number) => Math.max(0, Math.min(255, Math.round(v * 255)));
  return q(c[0]) * 65536 + q(c[1]) * 256 + q(c[2]);
}

const _m = new THREE.Matrix4();
const _q = new THREE.Quaternion();
const _up = new THREE.Vector3(0, 1, 0);

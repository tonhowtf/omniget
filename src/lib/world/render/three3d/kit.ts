// The modular kit: one GLB per style (agent E's `city3d_kit {style}`), one
// node per module key `camada/caso/estilo/variante`, each mesh coloured by a
// material named `slot:<slot>`. The kit is indexed into parts (slot +
// geometry, baked into the module's local frame) so the region builder can
// make one InstancedMesh per (key, slot).
//
// Module frame (BRIEFING, "Chave de módulo do kit"): 2×2×3 m cube, x = east,
// y = up, z = south, origin at the centre of the dual cell, floor at y = 0,
// canonical "outside" facing north (−z).
//
// Until a style's kit exists, every key gets generated stand-in geometry
// (boxes, wedges, trees from primitives) so the city never draws holes.
//
// Far variants: agent E's kit also carries `<key>#lod1` nodes (the module
// without mouldings and small parts, simplified). `farParts` hands them to
// the region builder for cells past the tier's `lodDistance`.

import {
  BoxGeometry,
  BufferAttribute,
  BufferGeometry,
  ConeGeometry,
  CylinderGeometry,
  IcosahedronGeometry,
  Matrix4,
  type Mesh,
  type Object3D,
} from 'three';

export interface KitPart {
  slot: string;
  geometry: BufferGeometry;
}

export interface Kit {
  style: string;
  /** True when the parts came from a real GLB, false for stand-ins only. */
  real: boolean;
  /** Parts of a key, resolving variant/style fallbacks; never empty. */
  parts(key: string): KitPart[];
  /**
   * Parts of the far variant (`<key>#lod1`), or null when the key has none
   * (the full parts are drawn at every distance).
   */
  farParts(key: string): KitPart[] | null;
  keys(): string[];
  dispose(): void;
}

// ── key helpers ───────────────────────────────────────────────────────────

export interface ModuleKey {
  layer: string;
  kase: string;
  style: string;
  variant: string;
}

export function parseKey(key: string): ModuleKey {
  const [layer = '', kase = '', style = '', variant = '0'] = key.split('/');
  return { layer, kase, style, variant };
}

/** Node-name suffix of a module's far variant in the kit GLB. */
export const LOD1_SUFFIX = '#lod1';

/** Layers hidden first by distance (LOD), per BRIEFING. */
export function isDetailLayer(layer: string): boolean {
  return layer === 'enfeite' || layer === 'prop';
}

// ── geometry helpers ──────────────────────────────────────────────────────

/** Float32 position + normal only: quantised kits (gltfpack -cc) come as ints. */
function toFloatPN(src: BufferGeometry, m: Matrix4): BufferGeometry {
  const out = new BufferGeometry();
  const pos = src.getAttribute('position');
  const p = new Float32Array(pos.count * 3);
  for (let i = 0; i < pos.count; i++) {
    p[i * 3] = pos.getX(i);
    p[i * 3 + 1] = pos.getY(i);
    p[i * 3 + 2] = pos.getZ(i);
  }
  out.setAttribute('position', new BufferAttribute(p, 3));
  const nor = src.getAttribute('normal');
  if (nor) {
    const n = new Float32Array(nor.count * 3);
    for (let i = 0; i < nor.count; i++) {
      n[i * 3] = nor.getX(i);
      n[i * 3 + 1] = nor.getY(i);
      n[i * 3 + 2] = nor.getZ(i);
    }
    out.setAttribute('normal', new BufferAttribute(n, 3));
  }
  if (src.index) out.setIndex(Array.from(src.index.array as ArrayLike<number>));
  out.applyMatrix4(m);
  if (!nor) out.computeVertexNormals();
  return out;
}

/** Concatenates same-slot geometries (position + normal, indexed). */
function mergeParts(geos: BufferGeometry[]): BufferGeometry {
  if (geos.length === 1) return geos[0];
  let verts = 0;
  let idx = 0;
  for (const g of geos) {
    verts += g.getAttribute('position').count;
    idx += g.index ? g.index.count : g.getAttribute('position').count;
  }
  const p = new Float32Array(verts * 3);
  const n = new Float32Array(verts * 3);
  const index = new Uint32Array(idx);
  let vo = 0;
  let io = 0;
  for (const g of geos) {
    const gp = g.getAttribute('position').array as Float32Array;
    const gn = g.getAttribute('normal').array as Float32Array;
    p.set(gp, vo * 3);
    n.set(gn, vo * 3);
    const count = g.getAttribute('position').count;
    if (g.index) {
      const a = g.index.array as ArrayLike<number>;
      for (let i = 0; i < a.length; i++) index[io++] = a[i] + vo;
    } else {
      for (let i = 0; i < count; i++) index[io++] = i + vo;
    }
    vo += count;
    g.dispose();
  }
  const out = new BufferGeometry();
  out.setAttribute('position', new BufferAttribute(p, 3));
  out.setAttribute('normal', new BufferAttribute(n, 3));
  out.setIndex(new BufferAttribute(index, 1));
  return out;
}

function box(w: number, h: number, d: number, x: number, y: number, z: number): BufferGeometry {
  const g = new BoxGeometry(w, h, d);
  g.translate(x, y + h / 2, z);
  g.deleteAttribute('uv');
  return g;
}

/** Gable: triangular prism along x, `h` tall, spanning [x0,x1] × [z0,z1]. */
function wedge(x0: number, x1: number, z0: number, z1: number, y: number, h: number): BufferGeometry {
  const zm = (z0 + z1) / 2;
  const v = [
    // two slopes
    x0, y, z0, x1, y, z0, x1, y + h, zm, x0, y, z0, x1, y + h, zm, x0, y + h, zm,
    x1, y, z1, x0, y, z1, x0, y + h, zm, x1, y, z1, x0, y + h, zm, x1, y + h, zm,
    // gable ends
    x0, y, z1, x0, y, z0, x0, y + h, zm,
    x1, y, z0, x1, y, z1, x1, y + h, zm,
  ];
  // Listed clockwise above; swap to counter-clockwise (three's front face).
  for (let t = 0; t < v.length; t += 9) {
    for (let k = 0; k < 3; k++) {
      const a = v[t + 3 + k];
      v[t + 3 + k] = v[t + 6 + k];
      v[t + 6 + k] = a;
    }
  }
  const g = new BufferGeometry();
  g.setAttribute('position', new BufferAttribute(new Float32Array(v), 3));
  g.computeVertexNormals();
  return toIndexed(g);
}

function toIndexed(g: BufferGeometry): BufferGeometry {
  if (g.index) return g;
  const n = g.getAttribute('position').count;
  const idx = new Uint32Array(n);
  for (let i = 0; i < n; i++) idx[i] = i;
  g.setIndex(new BufferAttribute(idx, 1));
  return g;
}

function prim(g: BufferGeometry, x = 0, y = 0, z = 0): BufferGeometry {
  g.translate(x, y, z);
  if (g.getAttribute('uv')) g.deleteAttribute('uv');
  return toIndexed(g);
}

/**
 * Occupied quadrants of the canonical case (outside = north, −z), as
 * [x0, x1, z0, z1] boxes inside the 2×2 cell centred on the origin.
 */
function caseQuads(kase: string): Array<[number, number, number, number]> {
  switch (kase) {
    case 'cheio':
      return [[-1, 1, -1, 1]];
    case 'parede':
      return [[-1, 1, 0, 1]];
    case 'convexo':
      return [[0, 1, 0, 1]];
    case 'concavo':
      return [
        [-1, 1, 0, 1],
        [0, 1, -1, 0],
      ];
    case 'diagonal':
      return [
        [0, 1, 0, 1],
        [-1, 0, -1, 0],
      ];
    default:
      return [];
  }
}

/** Stand-in parts for a key; a real kit replaces them key by key. */
export function fallbackParts(key: string): KitPart[] {
  const { layer, kase } = parseKey(key);
  const out: KitPart[] = [];
  const add = (slot: string, g: BufferGeometry) => out.push({ slot, geometry: toIndexed(g) });
  const quads = caseQuads(kase);
  switch (layer) {
    case 'terreo':
    case 'parede': {
      for (const [x0, x1, z0, z1] of quads) {
        add('wall', box(x1 - x0, 3, z1 - z0, (x0 + x1) / 2, 0, (z0 + z1) / 2));
        // A window band on the outer faces reads as a building from afar.
        if (layer === 'parede' && z0 >= 0) add('glass', box((x1 - x0) * 0.5, 1.1, 0.06, (x0 + x1) / 2, 1.1, z0 - 0.02));
        if (layer === 'terreo') add('stone', box(x1 - x0 + 0.04, 0.35, z1 - z0 + 0.04, (x0 + x1) / 2, 0, (z0 + z1) / 2));
      }
      break;
    }
    case 'porta': {
      for (const [x0, x1, z0, z1] of quads.length ? quads : caseQuads('parede')) {
        add('wall', box(x1 - x0, 3, z1 - z0, (x0 + x1) / 2, 0, (z0 + z1) / 2));
      }
      add('door', box(0.9, 2.1, 0.08, 0, 0, -0.03));
      add('trim', box(1.1, 0.12, 0.12, 0, 2.1, -0.06));
      break;
    }
    case 'telhado': {
      for (const [x0, x1, z0, z1] of quads.length ? quads : caseQuads('cheio')) {
        add('roof', wedge(x0 - 0.08, x1 + 0.08, z0 - 0.08, z1 + 0.08, 0, 1.3));
      }
      break;
    }
    case 'junta':
      add('trim', box(0.3, 3, 0.3, 0, 0, 0));
      break;
    case 'torre': {
      add('wall', prim(new CylinderGeometry(0.85, 0.9, 3, 10), 0, 1.5, 0));
      add('roof', prim(new ConeGeometry(1.05, 1.6, 10), 0, 3.8, 0));
      break;
    }
    case 'arvore': {
      add('trunk', prim(new CylinderGeometry(0.16, 0.22, 2.2, 6), 0, 1.1, 0));
      add('leaf', prim(new IcosahedronGeometry(1.25, 1), 0, 2.9, 0));
      break;
    }
    case 'enfeite': {
      if (kase === 'poste' || kase === 'lampiao') {
        add('metal', box(0.12, 3.2, 0.12, 0, 0, 0));
        add('glass', box(0.35, 0.35, 0.35, 0, 3.2, 0));
      } else {
        // Fences, hedges, kerbs: a low run along the cell's north edge.
        add('trim', box(2, 0.8, 0.12, 0, 0, -0.94));
        add('wood', box(0.12, 0.95, 0.12, -0.94, 0, -0.94));
      }
      break;
    }
    case 'prop': {
      if (kase === 'banco') {
        add('wood', box(1.5, 0.08, 0.45, 0, 0.42, 0));
        add('wood', box(1.5, 0.45, 0.08, 0, 0.5, 0.2));
        add('metal', box(0.08, 0.42, 0.4, -0.65, 0, 0));
        add('metal', box(0.08, 0.42, 0.4, 0.65, 0, 0));
      } else if (kase === 'vaso' || kase === 'canteiro') {
        add('stone', box(0.8, 0.5, 0.8, 0, 0, 0));
        add('leaf', prim(new IcosahedronGeometry(0.45, 0), 0, 0.8, 0));
      } else {
        add('wood', box(0.6, 0.6, 0.6, 0, 0, 0));
      }
      break;
    }
    case 'obra': {
      // Scaffold: four poles and two planks.
      for (const [x, z] of [
        [-0.9, -0.9],
        [0.9, -0.9],
        [0.9, 0.9],
        [-0.9, 0.9],
      ])
        add('wood', box(0.1, 3, 0.1, x, 0, z));
      add('wood', box(2, 0.06, 0.5, 0, 1.4, -0.75));
      add('wood', box(2, 0.06, 0.5, 0, 2.9, -0.75));
      break;
    }
    default:
      add('wall', box(1, 1, 1, 0, 0, 0));
  }
  if (out.length === 0 && kase !== 'vazio') add('wall', box(0.4, 0.4, 0.4, 0, 0, 0));
  return out;
}

// ── GLB kit ───────────────────────────────────────────────────────────────

function slotOf(mesh: Mesh): string {
  const mat = Array.isArray(mesh.material) ? mesh.material[0] : mesh.material;
  const name = mat?.name ?? '';
  return name.startsWith('slot:') ? name.slice(5) : 'wall';
}

/** Original glTF node names: GLTFLoader strips `/` from `Object3D.name`. */
function originalName(obj: Object3D, parser: { associations: Map<unknown, { nodes?: number }>; json: { nodes?: Array<{ name?: string }> } }): string {
  const idx = parser.associations.get(obj)?.nodes;
  if (idx !== undefined) {
    const n = parser.json.nodes?.[idx]?.name;
    if (n) return n;
  }
  return obj.name;
}

export async function parseKit(style: string, bytes: ArrayBuffer): Promise<Kit> {
  const [{ GLTFLoader }, { MeshoptDecoder }] = await Promise.all([
    import('three/examples/jsm/loaders/GLTFLoader.js'),
    import('three/examples/jsm/libs/meshopt_decoder.module.js'),
  ]);
  const loader = new GLTFLoader();
  loader.setMeshoptDecoder(MeshoptDecoder);
  const gltf = await loader.parseAsync(bytes, '');
  const parser = gltf.parser as unknown as Parameters<typeof originalName>[1];
  const index = new Map<string, KitPart[]>();
  const far = new Map<string, KitPart[]>();
  gltf.scene.updateMatrixWorld(true);
  const inv = new Matrix4();
  const rel = new Matrix4();

  gltf.scene.traverse((node: Object3D) => {
    const name = originalName(node, parser);
    const lod1 = name.endsWith(LOD1_SUFFIX);
    const key = lod1 ? name.slice(0, -LOD1_SUFFIX.length) : name;
    const into = lod1 ? far : index;
    if (key.split('/').length !== 4 || into.has(key)) return;
    inv.copy(node.matrixWorld).invert();
    const bySlot = new Map<string, BufferGeometry[]>();
    node.traverse((child: Object3D) => {
      const mesh = child as Mesh;
      if (!(mesh as { isMesh?: boolean }).isMesh || !mesh.geometry) return;
      rel.multiplyMatrices(inv, mesh.matrixWorld);
      const slot = slotOf(mesh);
      const list = bySlot.get(slot) ?? [];
      list.push(toFloatPN(mesh.geometry, rel));
      bySlot.set(slot, list);
    });
    const parts: KitPart[] = [];
    for (const [slot, geos] of bySlot) parts.push({ slot, geometry: mergeParts(geos) });
    if (parts.length) into.set(key, parts);
  });
  // Nothing of the parsed scene is drawn; free its GPU-side twins early.
  gltf.scene.traverse((o: Object3D) => (o as Mesh).geometry?.dispose());
  return makeKit(style, index, true, far);
}

export function fallbackKit(style: string): Kit {
  return makeKit(style, new Map(), false);
}

function makeKit(style: string, index: Map<string, KitPart[]>, real: boolean, far = new Map<string, KitPart[]>()): Kit {
  const generated = new Map<string, KitPart[]>();
  const byPrefix = new Map<string, string>();
  for (const key of index.keys()) {
    const { layer, kase, style: s } = parseKey(key);
    const a = `${layer}/${kase}/${s}`;
    if (!byPrefix.has(a)) byPrefix.set(a, key);
    const b = `${layer}/${kase}`;
    if (!byPrefix.has(b)) byPrefix.set(b, key);
  }
  /** The key whose parts `parts(key)` serves (itself, or a fallback). */
  function served(key: string): string | null {
    if (index.has(key)) return key;
    // Missing variant → another variant of the same case/style → any style.
    const { layer, kase, style: s } = parseKey(key);
    return byPrefix.get(`${layer}/${kase}/${s}`) ?? byPrefix.get(`${layer}/${kase}/any`) ?? byPrefix.get(`${layer}/${kase}`) ?? null;
  }
  return {
    style,
    real,
    parts(key: string): KitPart[] {
      const alt = served(key);
      if (alt) return index.get(alt)!;
      let gen = generated.get(key);
      if (!gen) {
        gen = fallbackParts(key);
        generated.set(key, gen);
      }
      return gen;
    },
    farParts(key: string): KitPart[] | null {
      const alt = served(key);
      return alt ? (far.get(alt) ?? null) : null;
    },
    keys() {
      return [...index.keys()];
    },
    dispose() {
      for (const parts of index.values()) for (const p of parts) p.geometry.dispose();
      for (const parts of far.values()) for (const p of parts) p.geometry.dispose();
      far.clear();
      for (const parts of generated.values()) for (const p of parts) p.geometry.dispose();
      index.clear();
      generated.clear();
    },
  };
}

/** Loads a style's kit through Tauri, falling back to stand-ins. */
export async function loadKit(style: string): Promise<Kit> {
  try {
    const { invoke } = await import('@tauri-apps/api/core');
    const res = (await invoke('city3d_kit', { style })) as ArrayBuffer | Uint8Array | number[];
    const buf =
      res instanceof ArrayBuffer
        ? res
        : res instanceof Uint8Array
          ? (res.buffer.slice(res.byteOffset, res.byteOffset + res.byteLength) as ArrayBuffer)
          : new Uint8Array(res).buffer;
    if (buf.byteLength < 20) return fallbackKit(style);
    return await parseKit(style, buf);
  } catch {
    return fallbackKit(style);
  }
}

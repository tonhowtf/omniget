// One region (64×64 tiles, 128×128 m) as three objects: the terrain mesh and
// one InstancedMesh per (module key, palette slot, cell, level of detail).
// Detail layers (`enfeite`, `prop`) are split per 16×16 chunk (32 m) so the
// distance LOD can drop them; everything else per 32×32 quadrant (64 m), so
// a quadrant can swap to the kit's far variants (`<key>#lod1`) as a whole
// (estudo 01 §5: InstancedMesh per piece × cell, culling by cell). Keys with
// no far variant go to a mesh drawn at every distance.

import {
  BufferAttribute,
  BufferGeometry,
  Color,
  Group,
  InstancedMesh,
  Matrix4,
  Mesh,
  Quaternion,
  Sphere,
  Vector3,
} from 'three';
import {
  CHUNK_TILES,
  REGION_TILES,
  TILE_MM,
  type Instance,
  type InstanceList,
  type TerrainMesh,
} from '$lib/city3d/types';
import { isDetailLayer, parseKey, type Kit, type KitPart } from './kit';
import type { MaterialSet } from './materials';
import type { PaletteBook } from './palette';

export const REGION_M = (REGION_TILES * TILE_MM) / 1000; // 128
const CHUNK_M = (CHUNK_TILES * TILE_MM) / 1000; // 32
/** Structure cells: 2×2 per region. */
const QUAD_M = REGION_M / 2; // 64
const OUTLINED = new Set(['terreo', 'parede', 'telhado', 'porta', 'torre', 'junta']);

export interface RegionMeshData {
  instances: InstanceList;
  terrain: TerrainMesh;
}

/** What a click on an instance resolves to. */
export interface InstancePick {
  key: string;
  owner: string | null;
  palette: string;
}

export interface DetailMesh {
  mesh: InstancedMesh;
  /** World-space centre of the chunk it covers. */
  center: Vector3;
}

/**
 * A cell of the distance LOD: `near` meshes (full modules) and `far` meshes
 * (their `#lod1` variants) swap; `both` holds keys without a far variant.
 * Outlines follow the full meshes only.
 */
export interface LodCell {
  /** World-space centre of the cell, on y = 0. */
  center: Vector3;
  /** Detail layers: hidden past the tier's detail distance. */
  detail: boolean;
  near: InstancedMesh[];
  far: InstancedMesh[];
  both: InstancedMesh[];
  outlines: InstancedMesh[];
  /** Current state, for hysteresis. */
  showingNear: boolean;
}

export interface BuiltRegion {
  region: [number, number];
  group: Group;
  terrain: Mesh | null;
  structure: InstancedMesh[];
  detail: DetailMesh[];
  outlines: InstancedMesh[];
  cells: LodCell[];
  instanceCount: number;
  center: Vector3;
  radius: number;
  hash: string;
  dispose(): void;
}

export function regionKey(r: [number, number]): string {
  return `${r[0]},${r[1]}`;
}

export function regionOrigin(r: [number, number]): Vector3 {
  return new Vector3(r[0] * REGION_M, 0, r[1] * REGION_M);
}

const _m = new Matrix4();
const _q = new Quaternion();
const _p = new Vector3();
const _s = new Vector3(1, 1, 1);
const _up = new Vector3(0, 1, 0);

/** Local (region) matrix of an instance: mm → m, quarter turns clockwise from above. */
export function instanceMatrix(inst: Instance, out = _m): Matrix4 {
  _p.set(inst.x_mm / 1000, inst.y_mm / 1000, inst.z_mm / 1000);
  _q.setFromAxisAngle(_up, -((inst.rot & 3) * Math.PI) / 2);
  return out.compose(_p, _q, _s);
}

function buildTerrain(t: TerrainMesh, palettes: PaletteBook, mats: MaterialSet): Mesh | null {
  const n = Math.floor(t.positions.length / 3);
  if (n === 0 || t.indices.length === 0) return null;
  const pos = new Float32Array(n * 3);
  for (let i = 0; i < n * 3; i++) pos[i] = t.positions[i] / 1000;
  const col = new Float32Array(n * 3);
  for (let i = 0; i < n; i++) {
    const c = palettes.slotColor(t.slots[i] ?? 7);
    col[i * 3] = c.r;
    col[i * 3 + 1] = c.g;
    col[i * 3 + 2] = c.b;
  }
  const g = new BufferGeometry();
  g.setAttribute('position', new BufferAttribute(pos, 3));
  g.setAttribute('color', new BufferAttribute(col, 3));
  g.setIndex(new BufferAttribute(n > 65535 ? Uint32Array.from(t.indices) : Uint16Array.from(t.indices), 1));
  g.computeVertexNormals();
  g.computeBoundingSphere();
  const mesh = new Mesh(g, mats.terrain);
  mesh.receiveShadow = true;
  mesh.name = 'terrain';
  mesh.userData.kind = 'terrain';
  return mesh;
}

type Level = 'near' | 'far' | 'both';

interface Bucket {
  key: string;
  slot: string;
  layer: string;
  geometry: BufferGeometry;
  list: Instance[];
  cell: string;
  level: Level;
}

/**
 * Shows the full or the far meshes of each cell for a camera at `cam`, hides
 * detail cells past `detailDistance`. Returns true when something changed.
 * A 10 % band around `lodDistance` keeps cells from flickering.
 */
export function applyCellLod(
  built: BuiltRegion,
  cam: Vector3,
  b: { lodDistance: number; detailDistance: number; outline: boolean },
): boolean {
  let changed = false;
  const set = (m: InstancedMesh, v: boolean) => {
    if (m.visible !== v) {
      m.visible = v;
      changed = true;
    }
  };
  for (const cell of built.cells) {
    const d = cell.center.distanceTo(cam);
    const shown = !cell.detail || d < b.detailDistance;
    const near = cell.showingNear ? d < b.lodDistance * 1.1 : d < b.lodDistance;
    cell.showingNear = near;
    for (const m of cell.both) set(m, shown);
    for (const m of cell.near) set(m, shown && near);
    for (const m of cell.far) set(m, shown && !near);
    for (const o of cell.outlines) set(o, shown && near && b.outline);
  }
  return changed;
}

export function buildRegion(
  data: RegionMeshData,
  kit: Kit,
  palettes: PaletteBook,
  mats: MaterialSet,
  opts: { outline: boolean; shadows: boolean },
): BuiltRegion {
  const region = (data.instances?.region ?? data.terrain?.region ?? [0, 0]) as [number, number];
  const group = new Group();
  group.name = `region:${regionKey(region)}`;
  group.position.copy(regionOrigin(region));
  group.userData.region = region;

  const terrain = data.terrain ? buildTerrain(data.terrain, palettes, mats) : null;
  if (terrain) group.add(terrain);

  // Bucket by (key, slot, cell, level); parts are resolved once per key.
  const buckets = new Map<string, Bucket>();
  const cellInfo = new Map<string, { center: Vector3; detail: boolean }>();
  const instances = data.instances?.instances ?? [];
  const partsOf = new Map<string, { near: KitPart[]; far: KitPart[] | null }>();
  const put = (inst: Instance, layer: string, cell: string, level: Level, parts: KitPart[]) => {
    for (const part of parts) {
      const id = `${inst.key}|${part.slot}|${cell}|${level}|${part.geometry.id}`;
      let b = buckets.get(id);
      if (!b) {
        b = { key: inst.key, slot: part.slot, layer, geometry: part.geometry, list: [], cell, level };
        buckets.set(id, b);
      }
      b.list.push(inst);
    }
  };
  for (const inst of instances) {
    const { layer } = parseKey(inst.key);
    const detail = isDetailLayer(layer);
    const size = detail ? CHUNK_M : QUAD_M;
    const cx = Math.floor(inst.x_mm / (size * 1000));
    const cz = Math.floor(inst.z_mm / (size * 1000));
    const cell = `${detail ? 'd' : 's'}${cx},${cz}`;
    if (!cellInfo.has(cell)) {
      cellInfo.set(cell, {
        center: regionOrigin(region).add(new Vector3((cx + 0.5) * size, 0, (cz + 0.5) * size)),
        detail,
      });
    }
    let p = partsOf.get(inst.key);
    if (!p) {
      p = { near: kit.parts(inst.key), far: kit.farParts(inst.key) };
      partsOf.set(inst.key, p);
    }
    if (p.far) {
      put(inst, layer, cell, 'near', p.near);
      put(inst, layer, cell, 'far', p.far);
    } else {
      put(inst, layer, cell, 'both', p.near);
    }
  }

  const structure: InstancedMesh[] = [];
  const detail: DetailMesh[] = [];
  const outlines: InstancedMesh[] = [];
  const cells = new Map<string, LodCell>();
  for (const [id, info] of cellInfo) {
    cells.set(id, { center: info.center, detail: info.detail, near: [], far: [], both: [], outlines: [], showingNear: true });
  }
  const color = new Color();
  for (const b of buckets.values()) {
    const im = new InstancedMesh(b.geometry, mats.slot(b.slot), b.list.length);
    const picks: InstancePick[] = new Array(b.list.length);
    for (let i = 0; i < b.list.length; i++) {
      const inst = b.list[i];
      im.setMatrixAt(i, instanceMatrix(inst));
      color.copy(palettes.color(inst.palette, b.slot));
      im.setColorAt(i, color);
      picks[i] = { key: inst.key, owner: inst.owner ?? null, palette: inst.palette };
    }
    im.instanceMatrix.needsUpdate = true;
    if (im.instanceColor) im.instanceColor.needsUpdate = true;
    im.computeBoundingSphere();
    im.computeBoundingBox();
    im.castShadow = opts.shadows && b.layer !== 'enfeite';
    im.receiveShadow = true;
    im.name = `${b.key}|${b.slot}${b.level === 'far' ? '|lod1' : ''}`;
    im.userData.kind = 'instances';
    im.userData.layer = b.layer;
    im.userData.picks = picks;
    im.visible = b.level !== 'far';
    group.add(im);
    const cell = cells.get(b.cell)!;
    cell[b.level].push(im);
    if (cell.detail) {
      detail.push({ mesh: im, center: cell.center });
    } else {
      structure.push(im);
      if (b.level !== 'far' && OUTLINED.has(b.layer) && b.slot !== 'glass') {
        const ol = new InstancedMesh(b.geometry, mats.outline, b.list.length);
        ol.instanceMatrix = im.instanceMatrix; // shared buffer, uploaded once
        ol.boundingSphere = im.boundingSphere;
        ol.boundingBox = im.boundingBox;
        ol.castShadow = false;
        ol.receiveShadow = false;
        ol.raycast = () => {};
        ol.visible = opts.outline;
        ol.userData.kind = 'outline';
        group.add(ol);
        outlines.push(ol);
        cell.outlines.push(ol);
      }
    }
  }

  const center = regionOrigin(region).add(new Vector3(REGION_M / 2, 0, REGION_M / 2));
  let top = 0;
  for (const m of structure) {
    const bb = m.boundingBox;
    if (bb) top = Math.max(top, bb.max.y);
  }
  const radius = new Sphere(center, Math.hypot(REGION_M / 2, REGION_M / 2, top)).radius;

  return {
    region,
    group,
    terrain,
    structure,
    detail,
    outlines,
    cells: [...cells.values()],
    instanceCount: instances.length,
    center,
    radius,
    hash: data.instances?.hash ?? '',
    dispose() {
      group.removeFromParent();
      for (const m of [...structure, ...detail.map((d) => d.mesh), ...outlines]) m.dispose();
      terrain?.geometry.dispose();
    },
  };
}

// Geometry by code (agent E): typed wrappers of the `city3d_codegeo_*`,
// `city3d_typolog*` and `city3d_kit*` commands, the preview palettes of the
// kit styles, and a helper that turns a codegeo GLB into a coloured three.js
// object (materials are named `slot:<slot>`; there is no colour texture).

import { invoke } from '@tauri-apps/api/core';
import type { Group, Object3D } from 'three';
import type { Anchor, PaletteSlot } from './types';

export interface ScriptError {
  line: number | null;
  column: number | null;
  message: string;
  /** syntax | runtime | limit | unknown_function | result | geometry | validation | semantic | size | anchor | class | cache */
  kind: string;
}

export interface AnchorPoint {
  action: string;
  /** Metres in the object's frame (x east, y up, z south). */
  pos: [number, number, number];
  /** Clockwise from north. */
  yaw_deg: number;
  capacity: number;
}

export interface Footprint {
  w: number;
  d: number;
  origin_mm: [number, number];
}

export interface CodegeoRun {
  /** Empty when `errors` is not. */
  glb_base64: string;
  hash: string;
  /** `codegeo:<sha256>`, what `BuildingSpec.typology` stores. */
  typology: string;
  anchors: Anchor[];
  anchor_points: AnchorPoint[];
  triangles: number;
  footprint: Footprint | null;
  slots: string[];
  warnings: string[];
  errors: ScriptError[];
  ms: number;
}

export interface Typology {
  name: string;
  class: string;
  script: string;
  hash: string;
  typology: string;
  triangles: number;
  footprint: Footprint;
  anchors: Anchor[];
  created_ms: number;
}

export interface CodegeoExample {
  id: string;
  name: string;
  class: string;
  script: string;
}

export interface ModuleStats {
  key: string;
  triangles: number;
  ms: number;
  budget: [number, number] | null;
  within_budget: boolean;
  /** Triangles of the far variant (`<key>#lod1` in the kit GLB). */
  lod1_triangles: number;
  warnings: string[];
}

export const KIT_STYLES = ['vila-colonial', 'vila-praiana', 'vila-arida'] as const;
export type KitStyle = (typeof KIT_STYLES)[number];

export const CODEGEO_CLASSES = [
  'building',
  'building:house',
  'prop',
  'prop:bench',
  'prop:fountain',
  'prop:kiosk',
  'prop:planter',
  'prop:bed',
  'prop:well',
  'module',
] as const;

/** Preview colours per kit style (the same defaults the kit GLB carries). */
export const STYLE_PALETTES: Record<KitStyle, Record<PaletteSlot, string>> = {
  'vila-colonial': {
    wall: '#F1E7D0', roof: '#B9654C', trim: '#436D59', wood: '#8A5A3B', stone: '#B8AE9C', glass: '#263C37',
    door: '#436D59', ground: '#C9B98F', grass: '#7C9B71', water: '#679FA4', street: '#A89F8E', plaza: '#D8CBB0',
    leaf: '#7C9B71', trunk: '#6B4A34', metal: '#263C37', cloth: '#B9654C',
  },
  'vila-praiana': {
    wall: '#F3D9A4', roof: '#5E7F86', trim: '#F7F3EA', wood: '#CBA568', stone: '#B8AE9C', glass: '#263C37',
    door: '#B9654C', ground: '#D8C79F', grass: '#7C9B71', water: '#679FA4', street: '#A89F8E', plaza: '#E6D8B8',
    leaf: '#6F9A62', trunk: '#6B4A34', metal: '#263C37', cloth: '#D96C5A',
  },
  'vila-arida': {
    wall: '#D9B98C', roof: '#C9A77C', trim: '#8A5A3B', wood: '#8A5A3B', stone: '#B09A7A', glass: '#263C37',
    door: '#6B4A34', ground: '#D4B98E', grass: '#BFB27A', water: '#679FA4', street: '#B8A58A', plaza: '#E0CBA5',
    leaf: '#8FA36B', trunk: '#6B4A34', metal: '#263C37', cloth: '#B9654C',
  },
};

// ── commands ──────────────────────────────────────────────────────────────

export function runCodegeo(script: string, cls: string): Promise<CodegeoRun> {
  return invoke<CodegeoRun>('city3d_codegeo_run', { script, class: cls });
}

export function listTypologies(): Promise<Typology[]> {
  return invoke<Typology[]>('city3d_typologies');
}

export function saveTypology(
  name: string,
  script: string,
  cls: string,
): Promise<{ typology: Typology | null; errors: ScriptError[] }> {
  return invoke('city3d_typology_save', { name, script, class: cls });
}

export function deleteTypology(id: string): Promise<boolean> {
  return invoke<boolean>('city3d_typology_delete', { id });
}

export function codegeoExamples(): Promise<CodegeoExample[]> {
  return invoke<CodegeoExample[]>('city3d_codegeo_examples');
}

/** The kit documentation, the same text an LLM gets in its prompt. */
export function codegeoReference(): Promise<string> {
  return invoke<string>('city3d_codegeo_reference');
}

export function kitStats(style: string): Promise<{ style: string; hash: string; modules: ModuleStats[]; aliases: [string, string][]; missing: string[] }> {
  return invoke('city3d_kit_stats', { style });
}

/** The style's kit GLB bytes (one node per `camada/caso/estilo/variante`). */
export async function kitBytes(style: string): Promise<ArrayBuffer> {
  const res = (await invoke('city3d_kit', { style })) as ArrayBuffer | Uint8Array | number[];
  if (res instanceof ArrayBuffer) return res;
  if (res instanceof Uint8Array) return res.buffer.slice(res.byteOffset, res.byteOffset + res.byteLength) as ArrayBuffer;
  return new Uint8Array(res).buffer;
}

// ── preview ───────────────────────────────────────────────────────────────

export function base64ToArrayBuffer(b64: string): ArrayBuffer {
  const bin = atob(b64);
  const out = new Uint8Array(bin.length);
  for (let i = 0; i < bin.length; i++) out[i] = bin.charCodeAt(i);
  return out.buffer;
}

export function errorLabel(e: ScriptError): string {
  if (e.line != null && e.column != null) return `${e.line}:${e.column}`;
  if (e.line != null) return `${e.line}`;
  return e.kind;
}

/**
 * Parses a codegeo GLB into a group with flat-shaded materials coloured from
 * `palette` by slot. Anchor nodes (`anchor:<action>`) get a small marker.
 */
export async function glbToObject(buf: ArrayBuffer, palette: Record<string, string>, markers = true): Promise<Group> {
  const THREE = await import('three');
  const { GLTFLoader } = await import('three/examples/jsm/loaders/GLTFLoader.js');
  const gltf = await new GLTFLoader().parseAsync(buf, '');
  const cache = new Map<string, InstanceType<typeof THREE.MeshLambertMaterial>>();
  const materialFor = (name: string) => {
    const slot = name.startsWith('slot:') ? name.slice(5) : 'wall';
    let m = cache.get(slot);
    if (!m) {
      m = new THREE.MeshLambertMaterial({ color: new THREE.Color(palette[slot] ?? '#999999'), flatShading: true });
      m.name = `slot:${slot}`;
      cache.set(slot, m);
    }
    return m;
  };
  const anchors: Object3D[] = [];
  gltf.scene.traverse((o: Object3D) => {
    const mesh = o as InstanceType<typeof THREE.Mesh>;
    if (mesh.isMesh) {
      const old = Array.isArray(mesh.material) ? mesh.material[0] : mesh.material;
      const name = old?.name ?? '';
      old?.dispose();
      mesh.material = materialFor(name);
    }
    if (o.name.startsWith('anchor')) anchors.push(o);
  });
  if (markers) {
    const geo = new THREE.ConeGeometry(0.12, 0.35, 8);
    geo.rotateX(-Math.PI / 2);
    geo.translate(0, 0.2, -0.1);
    const mat = new THREE.MeshBasicMaterial({ color: '#E0A33A' });
    for (const a of anchors) {
      const marker = new THREE.Mesh(geo, mat);
      marker.name = 'anchor-marker';
      a.add(marker);
    }
  }
  return gltf.scene;
}

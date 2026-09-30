// Mirror of `omniget-citygen/src/types.rs` and `program.rs`: the contract
// between the Rust generator and every city3d module of the frontend. Owned by
// the orchestrator; change it together with the Rust side.

export const REGION_TILES = 64;
export const CHUNK_TILES = 16;
export const TILE_MM = 2000;
export const LEVEL_MM = 3000;

export type Dir = 'n' | 'e' | 's' | 'w';
export type Axis = 'x' | 'y';
export type Surface =
  | 'grass' | 'forest' | 'sand' | 'rock' | 'water' | 'street'
  | 'sidewalk' | 'plaza' | 'bridge' | 'lot' | 'yard' | 'field';

export const PALETTE_SLOTS = [
  'wall', 'roof', 'trim', 'wood', 'stone', 'glass', 'door', 'ground', 'grass', 'water',
  'street', 'plaza', 'leaf', 'trunk', 'metal', 'cloth',
] as const;
export type PaletteSlot = (typeof PALETTE_SLOTS)[number];

export interface Palette { key: string; slots: Record<string, string> }

export interface Relief {
  sea_level_mm: number; hill_mm: number; mountain_mm: number;
  erosion: number; rivers: number; forest: number; city_flatten_slope: number;
}

export interface Charter {
  schema: string; city_id: string; name: string; seed: string;
  generator_version: string; story: string;
  climate: 'temperate' | 'tropical' | 'arid' | 'cold' | 'coastal';
  culture: string; style: string; palette: string; palettes: Palette[];
  relief: Relief;
  proximity: { kind: string; near: string | null; min_tiles: number | null; max_tiles: number | null }[];
  districts: { id: string; kind: string; regions: [number, number][]; style: string | null }[];
  publish_policy: 'auto' | 'review';
}

export interface Door { cell: [number, number]; outside: [number, number]; face: Dir; level: number }
export interface Roof { kind: string; ridge_axis: Axis }
export interface Bay { level: number; cell: [number, number]; face: Dir; kind: string }
export interface Tower { cells: [number, number][]; from_level: number; levels: number }
export interface Anchor { action: string; cell: [number, number]; level: number; yaw: number; capacity: number }

export interface Building {
  id: string; lot: string; style: string; palette: string; use: string; floors: number;
  levels: [number, number][][]; door: Door; stairs: [number, number][]; roof: Roof;
  bays: Bay[]; tower: Tower | null; typology: string | null; progress: number;
  owner_ref: string | null;
}

export interface Lot {
  id: string; rect: [number, number, number, number]; zone: string; front: Dir;
  building: string | null; owner_ref: string | null;
}

export interface Blueprint {
  schema: string; city_id: string; region: [number, number]; generator_version: string;
  program_hash: string; surface: Surface[]; corner_height_mm: number[];
  streets: { id: string; class: string; width: number; tiles: [number, number][] }[];
  lots: Lot[]; buildings: Building[];
  props: { kind: string; preset: string; cell: [number, number]; seed: number; anchors: Anchor[] }[];
  places: { id: string; kind: string; rect: [number, number, number, number] }[];
  hash: string;
}

export interface Violation {
  code: string; id: string; measure: string; message: string; severity: 'error' | 'warning';
}

export interface Instance {
  key: string; x_mm: number; y_mm: number; z_mm: number; rot: number;
  palette: string; owner: string | null;
}
export interface InstanceList {
  region: [number, number]; blueprint_hash: string; instances: Instance[]; hash: string;
}
export interface TerrainMesh {
  region: [number, number]; positions: number[]; slots: number[]; indices: number[];
}

// program.rs
export interface Zone {
  id: string; kind: string; rect: [number, number, number, number]; style: string;
  floors: [number, number]; density?: number;
  setback?: { front?: number; side?: number; back?: number }; palette?: string | null;
}
export interface Street { id: string; class: string; width: number; path: [number, number][] }
export interface Intent {
  use: string; floors?: number | null; shape?: string | null; roof?: string | null;
  palette?: string | null; style?: string | null; balcony?: boolean; awning?: boolean;
  tower?: boolean; chimney?: boolean; label?: string | null;
}
export interface BuildingSpec {
  id: string; lot: string; owner_ref?: string | null; intent: Intent;
  typology?: string | null; progress?: number;
}
export interface CityProgram {
  schema: string; city_id: string; region: [number, number]; catalog: string;
  zones: Zone[]; streets: Street[];
  water: { id: string; rect: [number, number, number, number]; bridges: string[] }[];
  landmarks: { id: string; kind: string; rect: [number, number, number, number] }[];
  lots: Record<string, unknown>; buildings: BuildingSpec[];
  fill: { zone: string; use: string; shape: string[]; roof: string[] }[];
  props: Record<string, unknown>[];
}
export type PatchOp = { op: string } & Record<string, unknown>;
export interface Proposal { region: [number, number]; base_revision: string; ops: PatchOp[]; admin?: boolean }
export interface ProposalResult { revision: string; violations: Violation[]; summary: Record<string, unknown> }

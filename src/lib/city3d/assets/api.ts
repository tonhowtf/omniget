// Typed wrappers of the import/export commands
// (`src-tauri/src/commands/city3d/asset3d.rs`, agent F). Every import goes
// through the Rust normalizer (nine steps, estudo 06) and is stored under
// `<app_data>/city3d/assets/<sha256>/`.

import { Channel, invoke } from "@tauri-apps/api/core";

export type AssetClass = "prop" | "building" | "character" | "decoration";
export type LicenseKind = "cc0" | "cc-by" | "own" | "vrm";
export type ImageMode = "standee" | "papercraft" | "relief";

export const ASSET_CLASSES: AssetClass[] = ["prop", "building", "character", "decoration"];
export const LICENSE_KINDS: LicenseKind[] = ["own", "cc0", "cc-by", "vrm"];
export const IMAGE_MODES: ImageMode[] = ["papercraft", "standee", "relief"];

/** Extensions the file picker offers. FBX/USDZ are refused by the normalizer. */
export const MODEL_EXTENSIONS = ["glb", "gltf", "vrm", "obj"];
export const IMAGE_EXTENSIONS = ["png", "jpg", "jpeg", "webp"];

export interface License {
  kind: LicenseKind;
  author?: string | null;
  url?: string | null;
}

export interface Footprint {
  w_tiles: number;
  d_tiles: number;
  cells: [number, number][];
  size_mm: [number, number, number];
}

export type Collision =
  | { kind: "box"; min_mm: [number, number, number]; max_mm: [number, number, number] }
  | { kind: "boxes"; boxes: [[number, number, number], [number, number, number]][] }
  | { kind: "capsule"; radius_mm: number; height_mm: number };

export interface AssetAnchor {
  action: string;
  pos_mm: [number, number, number];
  yaw: number;
  capacity: number;
  source: string;
}

export interface Permissions {
  use_as_avatar: boolean;
  redistribute_to_others: boolean;
  modify: boolean;
  export: boolean;
}

export interface VrmInfo {
  version: string;
  meta: Record<string, unknown>;
  permissions: Permissions;
  credit_required: boolean;
}

export interface Stats {
  bytes: number;
  triangles: number;
  vertices: number;
  meshes: number;
  materials: number;
  textures: number;
  texture_px_max: number;
}

export interface Report {
  format: string;
  input: Stats;
  output: Stats;
  steps: { step: string; ms: number }[];
  total_ms: number;
  scale: number;
  warnings: string[];
}

export interface AssetMeta {
  id: string;
  name: string;
  class: AssetClass;
  license: License;
  source: { kind: string; name: string; format: string; source_hash: string; provider?: string | null };
  footprint: Footprint;
  collision: Collision;
  anchors: AssetAnchor[];
  front: "n" | "e" | "s" | "w";
  vrm: VrmInfo | null;
  bytes: number;
  triangles: number;
  fit_style: boolean;
  created_at: number;
  normalizer: string;
  report: Report;
}

export interface ImportOptions {
  class: AssetClass;
  license: License;
  fitStyle: boolean;
  imageMode?: ImageMode;
  /** slot → `#rrggbb` (district palette); the default palette otherwise. */
  palette?: Record<string, string>;
}

export function importFile(path: string, o: ImportOptions): Promise<AssetMeta> {
  return invoke<AssetMeta>("city3d_import", {
    path,
    class: o.class,
    license: o.license,
    fitStyle: o.fitStyle,
    imageMode: o.imageMode ?? null,
    palette: o.palette ?? null,
  });
}

export function importBytes(name: string, bytes: Uint8Array, o: ImportOptions): Promise<AssetMeta> {
  return invoke<AssetMeta>("city3d_import_bytes", {
    name,
    bytesBase64: toBase64(bytes),
    class: o.class,
    license: o.license,
    fitStyle: o.fitStyle,
    imageMode: o.imageMode ?? null,
    palette: o.palette ?? null,
  });
}

export function listAssets(): Promise<AssetMeta[]> {
  return invoke<AssetMeta[]>("city3d_assets_list");
}

export async function assetGlb(id: string): Promise<ArrayBuffer> {
  return invoke<ArrayBuffer>("city3d_asset_glb", { id });
}

export async function assetThumbUrl(id: string): Promise<string> {
  const buf = await invoke<ArrayBuffer>("city3d_asset_thumb", { id });
  return URL.createObjectURL(new Blob([buf], { type: "image/png" }));
}

export function deleteAsset(id: string): Promise<void> {
  return invoke<void>("city3d_asset_delete", { id });
}

export interface VoxExport {
  vox_base64: string | null;
  path: string | null;
  voxel_mm: number;
  voxels: number;
  size: [number, number, number];
}

export function exportVox(id: string, voxelMm: number, path?: string): Promise<VoxExport> {
  return invoke<VoxExport>("city3d_export_vox", { id, voxelMm, path: path ?? null });
}

/** Copies a stored asset's GLB to `path` (refused when a VRM forbids it). */
export function exportAssetGlb(id: string, path: string): Promise<void> {
  return invoke<void>("city3d_asset_export", { id, path });
}

/** Writes bytes built in the front (GLB export) to a path from the save dialog. */
export function writeExport(path: string, bytes: Uint8Array): Promise<void> {
  return invoke<void>("city3d_export_write", { path, bytesBase64: toBase64(bytes) });
}

export function toBase64(bytes: Uint8Array): string {
  let s = "";
  const chunk = 0x8000;
  for (let i = 0; i < bytes.length; i += chunk) {
    s += String.fromCharCode(...bytes.subarray(i, i + chunk));
  }
  return btoa(s);
}

// ── Optional providers: Tripo (text/image to 3D) and skin-tokens (auto-rig) ──

export interface ProvidersStatus {
  tripo_key: boolean;
  rig_cli: boolean;
  rig_weights: boolean;
  rig_cli_path: string;
  rig_model_dir: string;
  /** A prebuilt skintokens-cli package exists for this platform. */
  rig_cli_package: boolean;
}

export function providersStatus(): Promise<ProvidersStatus> {
  return invoke<ProvidersStatus>("city3d_providers_status");
}

/** Saves the user's Tripo key in the system keychain; empty clears it. */
export function setTripoKey(key: string): Promise<void> {
  return invoke<void>("city3d_tripo_key_set", { key });
}

export function tripoBalance(): Promise<number> {
  return invoke<number>("city3d_tripo_balance");
}

export function generate3d(o: {
  prompt?: string;
  imageBase64?: string;
  class: AssetClass;
  fitStyle: boolean;
}): Promise<AssetMeta> {
  return invoke<AssetMeta>("city3d_generate3d", {
    prompt: o.prompt ?? null,
    imageBase64: o.imageBase64 ?? null,
    class: o.class,
    fitStyle: o.fitStyle,
  });
}

export function autorigConfigure(cliPath: string | null, modelDir: string | null): Promise<void> {
  return invoke<void>("city3d_autorig_configure", { cliPath, modelDir });
}

export async function autorigInstallWeights(
  onProgress: (p: { file: string; done: number; total: number | null }) => void,
): Promise<void> {
  const ch = new Channel<{ file: string; done: number; total: number | null }>();
  ch.onmessage = onProgress;
  return invoke<void>("city3d_autorig_install_weights", { onProgress: ch });
}

/** Downloads and extracts the prebuilt skintokens-cli for this platform. */
export async function autorigInstallCli(
  onProgress: (p: { file: string; done: number; total: number | null }) => void,
): Promise<void> {
  const ch = new Channel<{ file: string; done: number; total: number | null }>();
  ch.onmessage = onProgress;
  return invoke<void>("city3d_autorig_install_cli", { onProgress: ch });
}

/** Rigs a stored character without a skeleton; the result is a new asset. */
export function autorig(id: string): Promise<AssetMeta> {
  return invoke<AssetMeta>("city3d_autorig", { id });
}

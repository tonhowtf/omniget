// GLB export of what the city draws (a building, a cut of the city, a
// character): three's GLTFExporter, then glTF-Transform in the webview
// (dedup, weld, prune, meshopt with the npm encoder), estudo 06 §4.

import type { Object3D } from "three";
import { GLTFExporter } from "three/examples/jsm/exporters/GLTFExporter.js";
import { WebIO } from "@gltf-transform/core";
import { ALL_EXTENSIONS } from "@gltf-transform/extensions";
import { dedup, meshopt, prune, weld } from "@gltf-transform/functions";
import { MeshoptDecoder, MeshoptEncoder } from "meshoptimizer";
import { save } from "@tauri-apps/plugin-dialog";
import { writeExport, exportVox, exportAssetGlb } from "./api";

export interface ExportCredit {
  name: string;
  license: string;
  author?: string;
  url?: string;
}

export interface ExportOptions {
  /** Shown as the root node name and in `asset.extras.omniget.name`. */
  name?: string;
  /** Third-party pieces inside the export, written to `asset.extras`. */
  credits?: ExportCredit[];
  /** meshopt compression (default true). Off gives a GLB any tool opens. */
  compress?: boolean;
}

/** Plain GLB of an object tree, straight from three's exporter. */
export async function exportRaw(object: Object3D): Promise<Uint8Array> {
  const exporter = new GLTFExporter();
  const out = await exporter.parseAsync(object, { binary: true, onlyVisible: true, includeCustomExtensions: false });
  if (!(out instanceof ArrayBuffer)) throw new Error("GLTFExporter did not return a GLB");
  return new Uint8Array(out);
}

/** Optimized GLB (dedup, weld, prune, meshopt) of an object tree. */
export async function exportGlb(object: Object3D, opts: ExportOptions = {}): Promise<Uint8Array> {
  const raw = await exportRaw(object);
  await Promise.all([MeshoptEncoder.ready, MeshoptDecoder.ready]);
  const io = new WebIO().registerExtensions(ALL_EXTENSIONS).registerDependencies({
    "meshopt.encoder": MeshoptEncoder,
    "meshopt.decoder": MeshoptDecoder,
  });
  const doc = await io.readBinary(raw);
  const transforms = [dedup(), weld(), prune()];
  if (opts.compress !== false) transforms.push(meshopt({ encoder: MeshoptEncoder, level: "medium" }));
  await doc.transform(...transforms);
  const asset = doc.getRoot().getAsset();
  asset.generator = "OmniGet city3d export";
  const credits = opts.credits ?? [];
  if (credits.length) {
    asset.copyright = credits
      .map((c) => `${c.name}: ${c.license}${c.author ? ` (${c.author})` : ""}`)
      .join("; ");
  }
  asset.extras = { ...(asset.extras ?? {}), omniget: { name: opts.name ?? object.name ?? "export", credits } };
  return io.writeBinary(doc);
}

/** Asks where to save and writes the GLB there. Returns the path or null. */
export async function saveGlb(object: Object3D, defaultName: string, opts: ExportOptions = {}): Promise<string | null> {
  const path = await save({ defaultPath: `${defaultName}.glb`, filters: [{ name: "glTF binary", extensions: ["glb"] }] });
  if (!path) return null;
  const bytes = await exportGlb(object, opts);
  await writeExport(path, bytes);
  return path;
}

/** Save dialog + copy of a stored asset's normalized GLB. */
export async function saveAssetGlb(id: string, defaultName: string): Promise<string | null> {
  const path = await save({ defaultPath: `${defaultName}.glb`, filters: [{ name: "glTF binary", extensions: ["glb"] }] });
  if (!path) return null;
  await exportAssetGlb(id, path);
  return path;
}

/** Save dialog + `.vox` of a stored asset (voxel size in mm). */
export async function saveAssetVox(id: string, defaultName: string, voxelMm = 125): Promise<string | null> {
  const path = await save({ defaultPath: `${defaultName}.vox`, filters: [{ name: "MagicaVoxel", extensions: ["vox"] }] });
  if (!path) return null;
  await exportVox(id, voxelMm, path);
  return path;
}

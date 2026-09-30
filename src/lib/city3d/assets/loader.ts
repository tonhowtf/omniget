// Loads normalized assets (GLB with EXT_meshopt_compression,
// KHR_mesh_quantization and EXT_texture_webp) into three, and wires the
// palette: materials named `slot:<slot>` take the colour of the slot.

import { Color, Mesh, type Material, type Object3D } from "three";
import { GLTFLoader, type GLTF } from "three/examples/jsm/loaders/GLTFLoader.js";
import { MeshoptDecoder } from "three/examples/jsm/libs/meshopt_decoder.module.js";
import { assetGlb } from "./api";

let loader: GLTFLoader | null = null;

function gltfLoader(): GLTFLoader {
  if (!loader) {
    loader = new GLTFLoader();
    loader.setMeshoptDecoder(MeshoptDecoder);
  }
  return loader;
}

export function parseGlb(buf: ArrayBuffer): Promise<GLTF> {
  return gltfLoader().parseAsync(buf, "");
}

export async function loadAsset(id: string): Promise<GLTF> {
  return parseGlb(await assetGlb(id));
}

/** Paints `slot:<slot>` materials with a palette (slot → `#rrggbb`). */
export function applyPalette(root: Object3D, palette: Record<string, string>): void {
  root.traverse((o) => {
    if (!(o as Mesh).isMesh) return;
    const mats = ([] as Material[]).concat((o as Mesh).material);
    for (const m of mats) {
      const slot = m.name.startsWith("slot:") ? m.name.slice(5) : null;
      const hex = slot ? palette[slot] : undefined;
      const mat = m as Material & { color?: Color; vertexColors?: boolean };
      if (hex && mat.color) {
        mat.color.set(hex);
        // The palette replaces the baked slot colour.
        mat.vertexColors = false;
        mat.needsUpdate = true;
      }
    }
  });
}

export function disposeObject(root: Object3D): void {
  root.traverse((o) => {
    const mesh = o as Mesh;
    if (!mesh.isMesh) return;
    mesh.geometry?.dispose();
    for (const m of ([] as Material[]).concat(mesh.material)) {
      for (const v of Object.values(m)) {
        if (v && typeof v === "object" && "isTexture" in v) (v as { dispose(): void }).dispose();
      }
      m.dispose();
    }
  });
}

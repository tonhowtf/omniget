// On-demand MediaPipe files (estudo 13 §2): about 21 MB that never go into the
// bundle. Inside the app the Rust side downloads them once into the app data
// dir (sha256 checked) and serves the bytes; in a plain browser (dev) they
// come straight from the official CDNs.

import { Channel, invoke } from '@tauri-apps/api/core';
import { inTauri } from '$lib/world/session';

export const MEDIAPIPE_VERSION = '1.0.1';

export type LiveAssetName =
  | 'vision_wasm_module_internal.js'
  | 'vision_wasm_module_internal.wasm'
  | 'face_landmarker.task'
  | 'pose_landmarker_lite.task'
  | 'hand_landmarker.task';

/** Same table as `LIVE_ASSETS` in `commands/city3d/live.rs`. */
export const LIVE_ASSETS: Record<LiveAssetName, { url: string; bytes: number }> = {
  'vision_wasm_module_internal.js': {
    url: `https://cdn.jsdelivr.net/npm/@mediapipe/tasks-vision@${MEDIAPIPE_VERSION}/wasm/vision_wasm_module_internal.js`,
    bytes: 323_415,
  },
  'vision_wasm_module_internal.wasm': {
    url: `https://cdn.jsdelivr.net/npm/@mediapipe/tasks-vision@${MEDIAPIPE_VERSION}/wasm/vision_wasm_module_internal.wasm`,
    bytes: 11_756_972,
  },
  'face_landmarker.task': {
    url: 'https://storage.googleapis.com/mediapipe-models/face_landmarker/face_landmarker/float16/1/face_landmarker.task',
    bytes: 3_758_596,
  },
  'pose_landmarker_lite.task': {
    url: 'https://storage.googleapis.com/mediapipe-models/pose_landmarker/pose_landmarker_lite/float16/1/pose_landmarker_lite.task',
    bytes: 5_777_746,
  },
  'hand_landmarker.task': {
    url: 'https://storage.googleapis.com/mediapipe-models/hand_landmarker/hand_landmarker/float16/1/hand_landmarker.task',
    bytes: 7_819_105,
  },
};

export interface AssetProgress {
  name: string;
  received: number;
  total: number;
  /** True when the bytes came from the local copy. */
  cached?: boolean;
}

export interface AssetStatus {
  name: LiveAssetName;
  bytes: number;
  installed: boolean;
}

/** What is already on disk (app only; a browser reports nothing installed). */
export async function assetStatus(): Promise<AssetStatus[]> {
  if (!inTauri()) {
    return (Object.keys(LIVE_ASSETS) as LiveAssetName[]).map((name) => ({
      name,
      bytes: LIVE_ASSETS[name].bytes,
      installed: false,
    }));
  }
  return invoke<AssetStatus[]>('city3d_camera_assets');
}

async function fetchWithProgress(name: string, url: string, onProgress?: (p: AssetProgress) => void) {
  const res = await fetch(url);
  if (!res.ok || !res.body) throw new Error(`${name}: HTTP ${res.status}`);
  const total = Number(res.headers.get('content-length')) || 0;
  const reader = res.body.getReader();
  const chunks: Uint8Array[] = [];
  let received = 0;
  for (;;) {
    const { done, value } = await reader.read();
    if (done) break;
    chunks.push(value);
    received += value.byteLength;
    onProgress?.({ name, received, total });
  }
  const out = new Uint8Array(received);
  let o = 0;
  for (const c of chunks) {
    out.set(c, o);
    o += c.byteLength;
  }
  return out;
}

/** Bytes of one asset, downloading it the first time. */
export async function loadAsset(name: LiveAssetName, onProgress?: (p: AssetProgress) => void): Promise<Uint8Array> {
  if (!inTauri()) return fetchWithProgress(name, LIVE_ASSETS[name].url, onProgress);
  const channel = new Channel<AssetProgress>();
  channel.onmessage = (p) => onProgress?.(p);
  const buf = await invoke<ArrayBuffer>('city3d_camera_asset', { name, onProgress: channel });
  return new Uint8Array(buf);
}

export interface VisionAssets {
  factoryUrl: string;
  wasmUrl: string;
  models: { face?: Uint8Array; pose?: Uint8Array; hand?: Uint8Array };
  /** Revokes the blob URLs. */
  release(): void;
}

let cache: { factory: Uint8Array; wasm: Uint8Array; models: Partial<Record<LiveAssetName, Uint8Array>> } | null = null;

/** Loads the wasm plus the models the session needs; bytes stay cached for the page's lifetime. */
export async function loadVisionAssets(
  need: { face: boolean; body: boolean; hands: boolean },
  onProgress?: (p: AssetProgress) => void,
): Promise<VisionAssets> {
  cache ??= {
    factory: await loadAsset('vision_wasm_module_internal.js', onProgress),
    wasm: await loadAsset('vision_wasm_module_internal.wasm', onProgress),
    models: {},
  };
  const c = cache;
  const model = async (n: LiveAssetName) => (c.models[n] ??= await loadAsset(n, onProgress));
  const models: VisionAssets['models'] = {};
  if (need.face) models.face = await model('face_landmarker.task');
  if (need.body) models.pose = await model('pose_landmarker_lite.task');
  if (need.hands) models.hand = await model('hand_landmarker.task');
  const factoryUrl = URL.createObjectURL(new Blob([c.factory as BlobPart], { type: 'text/javascript' }));
  const wasmUrl = URL.createObjectURL(new Blob([c.wasm as BlobPart], { type: 'application/wasm' }));
  return {
    factoryUrl,
    wasmUrl,
    models,
    release() {
      URL.revokeObjectURL(factoryUrl);
      URL.revokeObjectURL(wasmUrl);
    },
  };
}

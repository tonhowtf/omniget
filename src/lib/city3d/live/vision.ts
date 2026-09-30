// MediaPipe Tasks Vision runner, shared by the worker and the main-thread
// fallback. It owns the three tasks (face, pose lite, hands) and flattens their
// results into the plain arrays the solver reads.
//
// The wasm is never bundled: the caller hands in blob URLs of the module
// flavour of the loader (`vision_wasm_module_internal.js`, which exports the
// factory) and of its `.wasm`. We import the loader ourselves and set
// `self.ModuleFactory` before each task, so the same code works in module
// workers, classic workers and the main thread.

import {
  FaceLandmarker,
  HandLandmarker,
  PoseLandmarker,
  type FaceLandmarkerResult,
  type HandLandmarkerResult,
  type PoseLandmarkerResult,
} from '@mediapipe/tasks-vision';
import { ARKIT_BLENDSHAPES } from './pose';
import type { BodyFrame, FaceFrame, HandsFrame } from './solver';

export type Delegate = 'GPU' | 'CPU';

export interface VisionInit {
  factoryUrl: string;
  wasmUrl: string;
  models: { face?: Uint8Array; pose?: Uint8Array; hand?: Uint8Array };
  delegate: Delegate;
}

export interface VisionWhich {
  face: boolean;
  body: boolean;
  hands: boolean;
}

export interface VisionResult {
  t: number;
  face?: FaceFrame | null;
  body?: BodyFrame | null;
  hands?: HandsFrame | null;
  /** Inference time of this call, ms. */
  ms: number;
}

type WasmFileset = Parameters<typeof FaceLandmarker.createFromOptions>[0];
type Factory = (...args: unknown[]) => unknown;
type FactoryGlobal = { ModuleFactory?: Factory };

const BS_INDEX = new Map<string, number>(ARKIT_BLENDSHAPES.map((n, i) => [n, i]));

async function loadFactory(url: string): Promise<Factory> {
  const mod = (await import(/* @vite-ignore */ url)) as { default?: Factory };
  const f = mod.default ?? (globalThis as FactoryGlobal).ModuleFactory;
  if (typeof f !== 'function') throw new Error('mediapipe loader has no ModuleFactory');
  return f;
}

function makeCanvas(): OffscreenCanvas | HTMLCanvasElement | undefined {
  if (typeof OffscreenCanvas !== 'undefined') return new OffscreenCanvas(1, 1);
  if (typeof document !== 'undefined') return document.createElement('canvas');
  return undefined;
}

export class VisionRunner {
  delegate: Delegate = 'GPU';
  private face: FaceLandmarker | null = null;
  private pose: PoseLandmarker | null = null;
  private hand: HandLandmarker | null = null;
  private lastTs = 0;

  private constructor() {}

  static async create(init: VisionInit): Promise<VisionRunner> {
    const runner = new VisionRunner();
    const factory = await loadFactory(init.factoryUrl);
    const fileset: WasmFileset = { wasmLoaderPath: '', wasmBinaryPath: init.wasmUrl };
    const g = globalThis as FactoryGlobal;

    // Each task instantiates its own wasm module and clears the global
    // factory afterwards, so it is set again every time.
    const build = async <T>(make: (delegate: Delegate) => Promise<T>): Promise<T> => {
      g.ModuleFactory = factory;
      try {
        return await make(init.delegate);
      } catch (e) {
        if (init.delegate === 'CPU') throw e;
        g.ModuleFactory = factory;
        runner.delegate = 'CPU';
        return await make('CPU');
      }
    };

    if (init.models.face) {
      const buf = init.models.face;
      runner.face = await build((delegate) =>
        FaceLandmarker.createFromOptions(fileset, {
          baseOptions: { modelAssetBuffer: buf, delegate },
          canvas: delegate === 'GPU' ? makeCanvas() : undefined,
          runningMode: 'VIDEO',
          numFaces: 1,
          outputFaceBlendshapes: true,
          outputFacialTransformationMatrixes: true,
        }),
      );
    }
    if (init.models.pose) {
      const buf = init.models.pose;
      runner.pose = await build((delegate) =>
        PoseLandmarker.createFromOptions(fileset, {
          baseOptions: { modelAssetBuffer: buf, delegate },
          canvas: delegate === 'GPU' ? makeCanvas() : undefined,
          runningMode: 'VIDEO',
          numPoses: 1,
        }),
      );
    }
    if (init.models.hand) {
      const buf = init.models.hand;
      runner.hand = await build((delegate) =>
        HandLandmarker.createFromOptions(fileset, {
          baseOptions: { modelAssetBuffer: buf, delegate },
          canvas: delegate === 'GPU' ? makeCanvas() : undefined,
          runningMode: 'VIDEO',
          numHands: 2,
        }),
      );
    }
    return runner;
  }

  has(): VisionWhich {
    return { face: !!this.face, body: !!this.pose, hands: !!this.hand };
  }

  /** Runs the requested tasks on one frame. `t` must grow between calls. */
  detect(frame: ImageBitmap | HTMLVideoElement | OffscreenCanvas, t: number, which: VisionWhich): VisionResult {
    const ts = Math.max(t, this.lastTs + 1);
    this.lastTs = ts;
    const start = performance.now();
    const out: VisionResult = { t, ms: 0 };
    if (which.face && this.face) out.face = flattenFace(this.face.detectForVideo(frame, ts));
    if (which.body && this.pose) out.body = flattenPose(this.pose.detectForVideo(frame, ts));
    if (which.hands && this.hand) out.hands = flattenHands(this.hand.detectForVideo(frame, ts));
    out.ms = performance.now() - start;
    return out;
  }

  close(): void {
    this.face?.close();
    this.pose?.close();
    this.hand?.close();
    this.face = this.pose = this.hand = null;
  }
}

export function flattenFace(r: FaceLandmarkerResult): FaceFrame | null {
  const cats = r.faceBlendshapes?.[0]?.categories;
  if (!cats) return null;
  const blendshapes = new Float32Array(ARKIT_BLENDSHAPES.length);
  for (const c of cats) {
    const i = BS_INDEX.get(c.categoryName);
    if (i !== undefined) blendshapes[i] = c.score;
  }
  const m = r.facialTransformationMatrixes?.[0]?.data;
  return { blendshapes, matrix: m ? Array.from(m) : null };
}

export function flattenPose(r: PoseLandmarkerResult): BodyFrame | null {
  const w = r.worldLandmarks?.[0];
  if (!w || w.length < 33) return null;
  const img = r.landmarks?.[0];
  const world = new Float32Array(33 * 3);
  const visibility = new Float32Array(33);
  for (let i = 0; i < 33; i++) {
    world[i * 3] = w[i].x;
    world[i * 3 + 1] = w[i].y;
    world[i * 3 + 2] = w[i].z;
    // A landmark outside the image is a guess, whatever its visibility says.
    const p = img?.[i];
    const inside = !p || (p.x > -0.05 && p.x < 1.05 && p.y > -0.05 && p.y < 1.05);
    visibility[i] = inside ? (w[i].visibility ?? p?.visibility ?? 1) : 0;
  }
  return { world, visibility };
}

export function flattenHands(r: HandLandmarkerResult): HandsFrame | null {
  if (!r.worldLandmarks?.length) return null;
  const out: HandsFrame = {};
  r.worldLandmarks.forEach((lm, i) => {
    const label = r.handedness?.[i]?.[0]?.categoryName;
    const arr = new Float32Array(lm.length * 3);
    lm.forEach((p, j) => {
      arr[j * 3] = p.x;
      arr[j * 3 + 1] = p.y;
      arr[j * 3 + 2] = p.z;
    });
    // MediaPipe labels handedness as if the image were mirrored; our frames
    // are not, so "Left" is the person's right hand.
    if (label === 'Left') out.right = arr;
    else if (label === 'Right') out.left = arr;
  });
  return out;
}

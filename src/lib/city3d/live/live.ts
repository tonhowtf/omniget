// `startLive(opts)`: webcam in, LivePose stream out.
//
// macOS and Windows: getUserMedia + MediaPipe in a Web Worker (GPU delegate on
// an OffscreenCanvas), face at 30 Hz and pose lite at 15 Hz, hands off unless
// asked. If the worker cannot start, the same runner goes to the main thread
// at half the rate. Linux: the native path in `commands/city3d/live.rs`
// (nokhwa + ONNX Runtime) sends landmarks over a Channel, and the same solver
// turns them into bones here.
//
// The video never leaves the machine: only the solved pose is handed out.

import { Channel, invoke } from '@tauri-apps/api/core';
import { inTauri } from '$lib/world/session';
import { loadVisionAssets, type AssetProgress, type VisionAssets } from './assets';
import type { LivePose } from './pose';
import { LiveSolver, type BodyFrame, type SolverOptions } from './solver';
import { VisionRunner, type VisionResult, type VisionWhich } from './vision';
import type { WorkerIn, WorkerOut } from './vision-worker';

export type LiveMode = 'worker' | 'main' | 'native';

export type LiveErrorCode = 'denied' | 'no_camera' | 'unsupported' | 'assets' | 'vision' | 'native';

export class LiveError extends Error {
  constructor(
    public code: LiveErrorCode,
    message: string,
  ) {
    super(message);
  }
}

export interface LiveOptions {
  /** `deviceId` (webview) or native camera index/id (Linux). */
  deviceId?: string | null;
  face?: boolean;
  body?: boolean;
  hands?: boolean;
  mirror?: boolean;
  legs?: boolean;
  faceHz?: number;
  bodyHz?: number;
  handsHz?: number;
  /** Where to show the local preview. Nothing is sent anywhere from it. */
  preview?: HTMLVideoElement | null;
  /** 'auto' picks native on Linux inside the app, the webview elsewhere. */
  path?: 'auto' | 'webview' | 'native';
  onProgress?: (p: AssetProgress) => void;
}

export interface LiveStats {
  mode: LiveMode;
  delegate: string;
  /** Poses emitted per second. */
  fps: number;
  /** Inference time per call, ms (EMA). */
  inferMs: number;
  /** Frame capture to pose, ms (EMA). */
  latencyMs: number;
}

export interface PoseStream {
  subscribe(cb: (pose: LivePose) => void): () => void;
  latest(): LivePose | null;
}

export interface LiveSession extends PoseStream {
  readonly mode: LiveMode;
  /** The camera stream (webview path only), for the local preview. */
  readonly stream: MediaStream | null;
  stats(): LiveStats;
  /** Turns face, body or hands on and off; loads models as needed. */
  setTasks(which: Partial<VisionWhich>): Promise<void>;
  setSolver(opts: Partial<SolverOptions>): void;
  stop(): Promise<void>;
}

export interface CameraInfo {
  id: string;
  label: string;
}

export function isLinux(): boolean {
  return typeof navigator !== 'undefined' && /Linux/i.test(navigator.userAgent) && !/Android/i.test(navigator.userAgent);
}

function useNative(path: LiveOptions['path']): boolean {
  if (path === 'native') return true;
  if (path === 'webview') return false;
  return inTauri() && isLinux();
}

/** Cameras for the picker. Labels may be empty until permission is granted. */
export async function listCameras(path: LiveOptions['path'] = 'auto'): Promise<CameraInfo[]> {
  if (useNative(path)) {
    const list = await invoke<{ id: string; name: string }[]>('city3d_camera_list');
    return list.map((c) => ({ id: c.id, label: c.name }));
  }
  if (!navigator.mediaDevices?.enumerateDevices) return [];
  const devs = await navigator.mediaDevices.enumerateDevices();
  return devs.filter((d) => d.kind === 'videoinput').map((d, i) => ({ id: d.deviceId, label: d.label || `Camera ${i + 1}` }));
}

class Emitter implements PoseStream {
  private subs = new Set<(p: LivePose) => void>();
  private last: LivePose | null = null;
  emit(p: LivePose) {
    this.last = p;
    this.subs.forEach((cb) => cb(p));
  }
  subscribe(cb: (p: LivePose) => void) {
    this.subs.add(cb);
    return () => this.subs.delete(cb);
  }
  latest() {
    return this.last;
  }
  clear() {
    this.subs.clear();
  }
}

const ema = (prev: number, v: number, k = 0.1) => (prev === 0 ? v : prev + (v - prev) * k);

// ------------------------------------------------------------ vision backends

interface Backend {
  mode: 'worker' | 'main';
  delegate: string;
  has: VisionWhich;
  /** Resolves with the result, or null when the backend was busy. */
  run(video: HTMLVideoElement, t: number, which: VisionWhich): Promise<VisionResult | null>;
  close(): void;
}

async function workerBackend(assets: VisionAssets): Promise<Backend> {
  const worker = new Worker(new URL('./vision-worker.ts', import.meta.url), { type: 'module' });
  const post = (m: WorkerIn, transfer: Transferable[] = []) => worker.postMessage(m, transfer);
  const ready = await new Promise<Extract<WorkerOut, { type: 'ready' }>>((resolve, reject) => {
    const timer = setTimeout(() => reject(new Error('vision worker timed out')), 45_000);
    worker.onerror = (e) => {
      clearTimeout(timer);
      reject(new Error(e.message || 'vision worker failed'));
    };
    worker.onmessage = (ev: MessageEvent<WorkerOut>) => {
      clearTimeout(timer);
      if (ev.data.type === 'ready') resolve(ev.data);
      else reject(new Error(ev.data.type === 'error' ? ev.data.message : 'unexpected worker message'));
    };
    post({ type: 'init', init: { factoryUrl: assets.factoryUrl, wasmUrl: assets.wasmUrl, models: assets.models, delegate: 'GPU' } });
  }).catch((e) => {
    worker.terminate();
    throw e;
  });

  let nextId = 1;
  const pending = new Map<number, (r: VisionResult | null) => void>();
  worker.onmessage = (ev: MessageEvent<WorkerOut>) => {
    const m = ev.data;
    if (m.type === 'result') {
      pending.get(m.id)?.(m.result);
      pending.delete(m.id);
    } else if (m.type === 'error' && m.id !== undefined) {
      pending.get(m.id)?.(null);
      pending.delete(m.id);
    }
  };
  return {
    mode: 'worker',
    delegate: ready.delegate,
    has: ready.has,
    async run(video, t, which) {
      if (pending.size > 0) return null;
      const bitmap = await createImageBitmap(video);
      const id = nextId++;
      return new Promise((resolve) => {
        pending.set(id, resolve);
        post({ type: 'frame', id, bitmap, t, which }, [bitmap]);
      });
    },
    close() {
      post({ type: 'close' });
      setTimeout(() => worker.terminate(), 500);
      pending.forEach((r) => r(null));
      pending.clear();
    },
  };
}

async function mainBackend(assets: VisionAssets): Promise<Backend> {
  const runner = await VisionRunner.create({
    factoryUrl: assets.factoryUrl,
    wasmUrl: assets.wasmUrl,
    models: assets.models,
    delegate: 'GPU',
  });
  return {
    mode: 'main',
    delegate: runner.delegate,
    has: runner.has(),
    async run(video, t, which) {
      return runner.detect(video, t, which);
    },
    close() {
      runner.close();
    },
  };
}

// ----------------------------------------------------------------- sessions

/** Starts the camera and the pipeline. Throws `LiveError` (denied, no camera, ...). */
export async function startLive(opts: LiveOptions = {}): Promise<LiveSession> {
  const solver = new LiveSolver({ mirror: opts.mirror ?? false, legs: opts.legs ?? false });
  return useNative(opts.path) ? startNative(opts, solver) : startWebview(opts, solver);
}

async function openCamera(deviceId?: string | null): Promise<MediaStream> {
  if (!navigator.mediaDevices?.getUserMedia) throw new LiveError('unsupported', 'getUserMedia is not available here');
  try {
    return await navigator.mediaDevices.getUserMedia({
      audio: false,
      video: {
        deviceId: deviceId ? { exact: deviceId } : undefined,
        width: { ideal: 640 },
        height: { ideal: 480 },
        frameRate: { ideal: 30 },
      },
    });
  } catch (e) {
    const name = (e as DOMException)?.name;
    if (name === 'NotAllowedError' || name === 'SecurityError') throw new LiveError('denied', String(e));
    if (name === 'NotFoundError' || name === 'OverconstrainedError' || name === 'NotReadableError')
      throw new LiveError('no_camera', String(e));
    throw new LiveError('unsupported', String(e));
  }
}

async function startWebview(opts: LiveOptions, solver: LiveSolver): Promise<LiveSession> {
  const which: VisionWhich = { face: opts.face ?? true, body: opts.body ?? true, hands: opts.hands ?? false };
  const stream = await openCamera(opts.deviceId);
  const video = opts.preview ?? document.createElement('video');
  video.muted = true;
  video.playsInline = true;
  video.srcObject = stream;
  await video.play().catch(() => {});

  const emitter = new Emitter();
  const stats: LiveStats = { mode: 'worker', delegate: 'GPU', fps: 0, inferMs: 0, latencyMs: 0 };
  let backend: Backend | null = null;
  let assets: VisionAssets | null = null;
  let stopped = false;

  const stopTracks = () => stream.getTracks().forEach((t) => t.stop());

  const build = async () => {
    const need = { ...which };
    let a: VisionAssets;
    try {
      a = await loadVisionAssets(need, opts.onProgress);
    } catch (e) {
      throw new LiveError('assets', String((e as Error)?.message ?? e));
    }
    let b: Backend;
    try {
      b = await workerBackend(a);
    } catch (workerErr) {
      console.warn('[city3d/live] worker path failed, using the main thread', workerErr);
      try {
        b = await mainBackend(a);
      } catch (e) {
        a.release();
        throw new LiveError('vision', String((e as Error)?.message ?? e));
      }
    }
    backend?.close();
    assets?.release();
    backend = b;
    assets = a;
    stats.mode = b.mode;
    stats.delegate = b.delegate;
  };

  try {
    await build();
  } catch (e) {
    stopTracks();
    throw e;
  }

  // Main-thread fallback runs at half rate to leave room for the renderer.
  const rate = () => (backend?.mode === 'main' ? 0.5 : 1);
  const faceHz = opts.faceHz ?? 30;
  const bodyHz = opts.bodyHz ?? 15;
  const handsHz = opts.handsHz ?? 15;
  const last = { face: -Infinity, body: -Infinity, hands: -Infinity };
  let busy = false;
  let lastEmit = 0;
  // Watchdog: WebKit can drop the worker's GPU context while the window is
  // hidden, and the landmarkers then keep answering with nothing. After a
  // stretch of empty results that follows real detections, rebuild once.
  let hadHit = false;
  let lastHit = 0;
  let lastRebuild = 0;
  const STALL_MS = 4000;
  const REBUILD_EVERY_MS = 15_000;

  const tick = async () => {
    if (stopped || busy || !backend || video.readyState < 2) return;
    const now = performance.now();
    const k = rate();
    const due: VisionWhich = {
      face: which.face && backend.has.face && now - last.face >= 1000 / (faceHz * k) - 4,
      body: which.body && backend.has.body && now - last.body >= 1000 / (bodyHz * k) - 4,
      hands: which.hands && backend.has.hands && now - last.hands >= 1000 / (handsHz * k) - 4,
    };
    if (!due.face && !due.body && !due.hands) return;
    busy = true;
    try {
      if (hadHit && now - lastHit > STALL_MS && now - lastRebuild > REBUILD_EVERY_MS) {
        lastRebuild = now;
        hadHit = false;
        console.warn('[city3d/live] no detections for a while, restarting the vision backend');
        await build();
        return;
      }
      const r = await backend.run(video, now, due);
      if (!r || stopped) return;
      // Shoulders are MediaPipe pose landmarks 11 and 12.
      const seen = !!r.face || (!!r.body && (r.body.visibility[11] ?? 0) > 0.5 && (r.body.visibility[12] ?? 0) > 0.5);
      if (seen) {
        hadHit = true;
        lastHit = now;
      }
      if (due.face) last.face = now;
      if (due.body) last.body = now;
      if (due.hands) last.hands = now;
      if (r.face) solver.updateFace(r.face, now);
      if (r.body) solver.updateBody(r.body, now);
      if (r.hands) solver.updateHands(r.hands, now);
      const done = performance.now();
      stats.inferMs = ema(stats.inferMs, r.ms);
      stats.latencyMs = ema(stats.latencyMs, done - now);
      if (lastEmit) stats.fps = ema(stats.fps, 1000 / Math.max(1, done - lastEmit));
      lastEmit = done;
      emitter.emit(solver.pose(done));
    } catch (e) {
      console.warn('[city3d/live] frame failed', e);
    } finally {
      busy = false;
    }
  };

  // requestAnimationFrame rather than requestVideoFrameCallback: the latter
  // does not fire for a video that is not on screen (hidden preview).
  let raf = 0;
  const loop = () => {
    if (stopped) return;
    void tick();
    raf = requestAnimationFrame(loop);
  };
  loop();

  return {
    mode: stats.mode,
    stream,
    subscribe: (cb) => emitter.subscribe(cb),
    latest: () => emitter.latest(),
    stats: () => ({ ...stats }),
    async setTasks(next) {
      const before = { ...which };
      Object.assign(which, next);
      const needsModel = (Object.keys(which) as (keyof VisionWhich)[]).some((key) => which[key] && !backend?.has[key]);
      if (needsModel) {
        try {
          await build();
        } catch (e) {
          Object.assign(which, before);
          throw e;
        }
      }
    },
    setSolver(o) {
      Object.assign(solver.opts, o);
      solver.reset();
    },
    async stop() {
      stopped = true;
      cancelAnimationFrame(raf);
      backend?.close();
      assets?.release();
      stopTracks();
      if (video.srcObject === stream) video.srcObject = null;
      emitter.clear();
    },
  };
}

// ------------------------------------------------------------- native (Linux)

/** Messages from `city3d_camera_start`'s channel. Mirrors `NativeMessage` in live.rs. */
export type NativeMessage =
  | { kind: 'status'; state: 'opening' | 'running' | 'stopped' | 'error'; message?: string | null }
  | { kind: 'frame'; t: number; infer_ms: number; body?: BodyFrame | null };

async function startNative(opts: LiveOptions, solver: LiveSolver): Promise<LiveSession> {
  const emitter = new Emitter();
  const stats: LiveStats = { mode: 'native', delegate: 'onnx', fps: 0, inferMs: 0, latencyMs: 0 };
  const channel = new Channel<NativeMessage>();
  let lastEmit = 0;
  let failed: ((e: LiveError) => void) | null = null;
  const started = new Promise<void>((resolve, reject) => {
    failed = reject;
    channel.onmessage = (m) => {
      if (m.kind === 'status') {
        if (m.state === 'running') resolve();
        if (m.state === 'error') {
          const e = new LiveError('native', m.message ?? 'camera error');
          failed?.(e);
          console.warn('[city3d/live] native camera', m.message);
        }
        return;
      }
      const now = performance.now();
      if (m.body && (opts.body ?? true)) solver.updateBody(m.body, now);
      stats.inferMs = ema(stats.inferMs, m.infer_ms);
      if (lastEmit) stats.fps = ema(stats.fps, 1000 / Math.max(1, now - lastEmit));
      lastEmit = now;
      emitter.emit(solver.pose(now));
    };
  });
  try {
    await invoke('city3d_camera_start', { device: opts.deviceId ?? null, channel });
    await started;
  } catch (e) {
    await invoke('city3d_camera_stop').catch(() => {});
    if (e instanceof LiveError) throw e;
    const msg = String(e);
    throw new LiveError(/permission|denied/i.test(msg) ? 'denied' : /no camera|not found/i.test(msg) ? 'no_camera' : 'native', msg);
  }
  failed = null;
  return {
    mode: 'native',
    stream: null,
    subscribe: (cb) => emitter.subscribe(cb),
    latest: () => emitter.latest(),
    stats: () => ({ ...stats }),
    async setTasks() {
      // The native path runs the pose model only (see live.rs).
    },
    setSolver(o) {
      Object.assign(solver.opts, o);
      solver.reset();
    },
    async stop() {
      await invoke('city3d_camera_stop').catch(() => {});
      emitter.clear();
    },
  };
}

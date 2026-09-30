// Web Worker that runs MediaPipe off the render thread. Frames come in as
// ImageBitmaps (transferred), results go back as flat typed arrays.

import { VisionRunner, type VisionInit, type VisionWhich } from './vision';

export type WorkerIn =
  | { type: 'init'; init: VisionInit }
  | { type: 'frame'; id: number; bitmap: ImageBitmap; t: number; which: VisionWhich }
  | { type: 'close' };

export type WorkerOut =
  | { type: 'ready'; delegate: string; has: VisionWhich }
  | { type: 'error'; id?: number; message: string }
  | { type: 'result'; id: number; result: import('./vision').VisionResult };

const ctx = self as unknown as {
  postMessage(msg: WorkerOut, transfer?: Transferable[]): void;
  onmessage: ((ev: MessageEvent<WorkerIn>) => void) | null;
  close(): void;
};

let runner: VisionRunner | null = null;

ctx.onmessage = async (ev) => {
  const msg = ev.data;
  if (msg.type === 'init') {
    try {
      runner = await VisionRunner.create(msg.init);
      ctx.postMessage({ type: 'ready', delegate: runner.delegate, has: runner.has() });
    } catch (e) {
      ctx.postMessage({ type: 'error', message: String((e as Error)?.message ?? e) });
    }
    return;
  }
  if (msg.type === 'frame') {
    const { bitmap } = msg;
    try {
      if (!runner) throw new Error('vision worker not ready');
      const result = runner.detect(bitmap, msg.t, msg.which);
      const transfer: Transferable[] = [];
      const tf = (a: unknown) => {
        if (a instanceof Float32Array) transfer.push(a.buffer);
      };
      tf(result.face?.blendshapes);
      tf(result.body?.world);
      tf(result.body?.visibility);
      tf(result.hands?.left);
      tf(result.hands?.right);
      ctx.postMessage({ type: 'result', id: msg.id, result }, transfer);
    } catch (e) {
      ctx.postMessage({ type: 'error', id: msg.id, message: String((e as Error)?.message ?? e) });
    } finally {
      bitmap.close();
    }
    return;
  }
  if (msg.type === 'close') {
    runner?.close();
    runner = null;
    ctx.close();
  }
};

// What is left of the house's bridge: `toBytes`, which the city uses to read
// its channel, and `createFakeSession`, a session driven by fixture blobs that
// the tests use to exercise the codec and the state machine without an app.

import { decodeBlob, type Blob as WorldBlob } from './codec';

/** The inputs the route can send. Mirrors `omniget_world::Input`. */
export type WorldInput =
  | { type: 'move'; ent: number; to: [number, number] }
  | { type: 'interact'; ent: number; object: number }
  | { type: 'place_object'; object: number; kind: string; tile: [number, number]; dir: number; slot?: string | null }
  | { type: 'remove_object'; object: number }
  | { type: 'say'; ent: number; text: string }
  | { type: 'sleep'; ent: number }
  | { type: 'idle'; ent: number };

type SleepStateName = 'active' | 'dozing' | 'hibernating' | 'never';

/**
 * Bytes out of a channel message. Tauri may hand a channel payload over as a
 * raw ArrayBuffer or as a JSON array of numbers depending on how the Rust side
 * sends it, and the route must not care which.
 */
export function toBytes(message: unknown): Uint8Array {
  if (message instanceof Uint8Array) return message;
  if (message instanceof ArrayBuffer) return new Uint8Array(message);
  if (ArrayBuffer.isView(message)) {
    const v = message as ArrayBufferView;
    return new Uint8Array(v.buffer, v.byteOffset, v.byteLength);
  }
  if (Array.isArray(message)) return Uint8Array.from(message as number[]);
  const data = (message as { data?: unknown } | null)?.data;
  if (data) return toBytes(data);
  throw new TypeError('ERR_WORLD_BAD_CHANNEL_PAYLOAD');
}

export interface FakeSessionControls {
  exists(): Promise<boolean>;
  create(opts?: { seed?: number; house?: string }): Promise<void>;
  /** Starts the diff stream. Resolves with the first snapshot. */
  open(onBlob: (blob: WorldBlob) => void, onError: (code: string) => void): Promise<{ blob: WorldBlob }>;
  close(): Promise<void>;
  send(input: WorldInput): Promise<void>;
  setVisible(visible: boolean): Promise<void>;
  /** Starts the stream over from the first snapshot. */
  resync(): Promise<void>;
  delete(): Promise<void>;
  onSleepState(cb: (state: SleepStateName) => void): () => void;
  dispose(): void;
  /** Hands the next fixture blob to the route, as the channel would. */
  advance(): boolean;
  /** Every input the route sent, in order. */
  readonly sent: WorldInput[];
  readonly opened: boolean;
  readonly visible: boolean;
  /** Feeds arbitrary bytes, to exercise the error path. */
  push(bytes: Uint8Array): void;
}

/**
 * A session driven by fixture blobs instead of by Rust. The route cannot tell
 * the difference, which is the point: the whole state machine is testable
 * without an app, a GPU or a tick thread.
 */
export function createFakeSession(blobs: Uint8Array[]): FakeSessionControls {
  let index = 0;
  let onBlob: ((b: WorldBlob) => void) | null = null;
  let onError: ((code: string) => void) | null = null;
  let sleepCb: ((s: SleepStateName) => void) | null = null;
  const sent: WorldInput[] = [];
  let opened = false;
  let visible = true;
  let exists = blobs.length > 0;

  const session: FakeSessionControls = {
    async exists() {
      return exists;
    },
    async create() {
      exists = true;
    },
    async open(next, err) {
      onBlob = next;
      onError = err;
      opened = true;
      if (blobs.length === 0) throw new Error('ERR_WORLD_NO_FIXTURE');
      index = 1;
      return { blob: decodeBlob(blobs[0]) };
    },
    async close() {
      opened = false;
    },
    async send(input) {
      sent.push(input);
    },
    async setVisible(v) {
      visible = v;
      sleepCb?.(v ? 'active' : 'dozing');
    },
    async resync() {
      index = 0;
      session.advance();
    },
    async delete() {
      exists = false;
      opened = false;
    },
    onSleepState(cb) {
      sleepCb = cb;
      return () => {
        sleepCb = null;
      };
    },
    dispose() {
      onBlob = null;
      onError = null;
      sleepCb = null;
    },
    advance() {
      if (index >= blobs.length) return false;
      session.push(blobs[index++]);
      return true;
    },
    push(bytes: Uint8Array) {
      try {
        onBlob?.(decodeBlob(bytes));
      } catch (e) {
        onError?.(e instanceof Error ? e.message : String(e));
      }
    },
    get sent() {
      return sent;
    },
    get opened() {
      return opened;
    },
    get visible() {
      return visible;
    },
  };
  return session;
}

/** True inside the Tauri webview; outside it the route says so instead of throwing. */
export function inTauri(): boolean {
  return typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window;
}

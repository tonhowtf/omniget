// Pose on the network (estudo 13 §4.2): unreliable, unordered, latest wins.
//
// The app has no WebRTC/LiveKit data channel today (voice left with the
// plugins), so the stream rides a `PoseTransport`. Two implementations here:
// a loopback for local tests and the panel, and a WebSocket one for any relay
// that forwards binary frames. TODO(city socket): when the city server
// (omnidisc-server `/world/v1`) grows a pose relay, add a transport over that
// socket with a size-1 queue per sender on the relay (overwrite, never queue),
// and 5 Hz or zero outside the interest radius. If a WebRTC data channel comes
// back with voice, open it with `ordered: false, maxRetransmits: 0`.

import { PoseBuffer, decodePose, encodePose, seqNewer, type LivePose } from './pose';

export interface PoseTransport {
  /** Fire and forget; a transport may drop frames under pressure. */
  send(frame: Uint8Array): void;
  /** Subscribes to incoming frames. Returns the unsubscribe. */
  onFrame(cb: (frame: Uint8Array) => void): () => void;
  close(): void;
}

/** Loops frames back, optionally with delay, jitter and loss, to exercise the receiver. */
export function createLoopbackTransport(opts: { latencyMs?: number; jitterMs?: number; loss?: number } = {}): PoseTransport {
  const subs = new Set<(f: Uint8Array) => void>();
  const timers = new Set<ReturnType<typeof setTimeout>>();
  return {
    send(frame) {
      if (opts.loss && Math.random() < opts.loss) return;
      const copy = frame.slice();
      const delay = (opts.latencyMs ?? 0) + (opts.jitterMs ? Math.random() * opts.jitterMs : 0);
      if (delay <= 0) {
        subs.forEach((cb) => cb(copy));
        return;
      }
      const id = setTimeout(() => {
        timers.delete(id);
        subs.forEach((cb) => cb(copy));
      }, delay);
      timers.add(id);
    },
    onFrame(cb) {
      subs.add(cb);
      return () => subs.delete(cb);
    },
    close() {
      timers.forEach(clearTimeout);
      timers.clear();
      subs.clear();
    },
  };
}

/** Frame tag on a shared socket, so pose frames can share it with other traffic. */
export const POSE_TAG = 0x50;

/**
 * Binary WebSocket transport. It never queues: when the socket is backed up
 * the new frame is dropped, because an old pose is worth nothing.
 */
export function createWebSocketTransport(url: string, maxBuffered = 4096): PoseTransport {
  const subs = new Set<(f: Uint8Array) => void>();
  const ws = new WebSocket(url);
  ws.binaryType = 'arraybuffer';
  ws.onmessage = (ev) => {
    if (!(ev.data instanceof ArrayBuffer)) return;
    const b = new Uint8Array(ev.data);
    if (b[0] !== POSE_TAG) return;
    const frame = b.subarray(1);
    subs.forEach((cb) => cb(frame));
  };
  return {
    send(frame) {
      if (ws.readyState !== WebSocket.OPEN || ws.bufferedAmount > maxBuffered) return;
      const out = new Uint8Array(frame.byteLength + 1);
      out[0] = POSE_TAG;
      out.set(frame, 1);
      ws.send(out);
    },
    onFrame(cb) {
      subs.add(cb);
      return () => subs.delete(cb);
    },
    close() {
      subs.clear();
      ws.close();
    },
  };
}

export interface SenderStats {
  sent: number;
  bytes: number;
  /** Average frame size, bytes. */
  avgFrame: number;
}

/** Throttles a pose stream to `rateHz` and sends it quantized. */
export class PoseSender {
  private last = -Infinity;
  private seq = 0;
  stats: SenderStats = { sent: 0, bytes: 0, avgFrame: 0 };

  constructor(
    private transport: PoseTransport,
    private actor: number,
    public rateHz = 15,
    public perfectSync = false,
  ) {}

  /** Offers a pose; it goes out only when the rate allows. `now` in ms. */
  push(pose: LivePose, now = performance.now()): boolean {
    if (now - this.last < 1000 / this.rateHz - 1) return false;
    this.last = now;
    this.seq = (this.seq + 1) & 0xffff;
    const frame = encodePose({ ...pose, seq: this.seq }, { actor: this.actor, perfectSync: this.perfectSync });
    this.transport.send(frame);
    this.stats.sent++;
    this.stats.bytes += frame.byteLength;
    this.stats.avgFrame = this.stats.bytes / this.stats.sent;
    return true;
  }
}

/**
 * Receives frames from many actors, drops stale ones by sequence and keeps a
 * jitter buffer per actor. Sample it at render time.
 */
export class PoseReceiver {
  private buffers = new Map<number, PoseBuffer>();
  private newest = new Map<number, number>();
  private unsub: () => void;
  received = 0;
  dropped = 0;
  malformed = 0;

  constructor(
    transport: PoseTransport,
    private delayMs = 120,
    private onPose?: (actor: number, pose: LivePose) => void,
  ) {
    this.unsub = transport.onFrame((frame) => this.accept(frame));
  }

  accept(frame: Uint8Array, now = performance.now()): void {
    let decoded;
    try {
      decoded = decodePose(frame, now);
    } catch {
      this.malformed++;
      return;
    }
    const { actor, pose } = decoded;
    const prev = this.newest.get(actor);
    if (prev !== undefined && !seqNewer(pose.seq, prev)) {
      this.dropped++;
      return;
    }
    this.newest.set(actor, pose.seq);
    this.received++;
    let buf = this.buffers.get(actor);
    if (!buf) {
      buf = new PoseBuffer(this.delayMs);
      this.buffers.set(actor, buf);
    }
    buf.push(pose);
    this.onPose?.(actor, pose);
  }

  /** Interpolated pose for `actor` at render time `now`. */
  sample(actor: number, now = performance.now()): LivePose | null {
    return this.buffers.get(actor)?.sample(now) ?? null;
  }

  actors(): number[] {
    return [...this.buffers.keys()];
  }

  forget(actor: number): void {
    this.buffers.delete(actor);
    this.newest.delete(actor);
  }

  close(): void {
    this.unsub();
    this.buffers.clear();
    this.newest.clear();
  }
}

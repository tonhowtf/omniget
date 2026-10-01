import { pointAtLength, arcLengths } from '../spatial/geometry';

const MAX_ENVELOPES = 16;
const ENVELOPE_SPEED = 230;
const MIN_FLIGHT_S = 0.8;
const MAX_FLIGHT_S = 2.0;
const ARRIVAL_S = 0.34;

const KIND_COLORS: Record<string, number> = {
  ask: 0x6cb8e6,
  request: 0x6cb8e6,
  query: 0xb48ee0,
  check: 0xb48ee0,
  propose: 0xf4d35e,
  message: 0xf3e9d2,
  inform: 0xf3e9d2,
  say: 0xf3e9d2,
  note: 0xf3e9d2,
  reply: 0x5ca97a,
  answer: 0x5ca97a,
  agree: 0x5ca97a,
  done: 0x5ca97a,
  refuse: 0xd96a62,
  failed: 0xd96a62,
  error: 0xd96a62,
  human: 0xd96a62,
  handoff: 0xe8a33c,
  spawn: 0x9b7be0,
  portal: 0x7cc4b8,
};

export function kindColor(kind: string): number {
  return KIND_COLORS[kind] ?? 0xf3e9d2;
}

interface Envelope {
  id: string;
  kind: string;
  color: number;
  pts: number[];
  lens: number[];
  start: number;
  duration: number;
  connectionId?: string;
  toNode: string;
}

interface EnvelopeFrame {
  x: number;
  y: number;
  angle: number;
  color: number;
  t: number;
  ring: number;
  alpha: number;
}

function ease(t: number): number {
  return t < 0.5 ? 2 * t * t : 1 - Math.pow(-2 * t + 2, 2) / 2;
}

const ASK_TTL_MS = 15 * 60 * 1000;

export class Traffic {
  private flying: Envelope[] = [];
  private seen = new Set<string>();
  private asks = new Map<string, { from: string; to: string; at: number }>();

  trackAsk(p: { id: string; from: string; to: string; state: string }, now = Date.now()): boolean {
    if (!p?.id) return false;
    if (p.state === 'queued' || p.state === 'delivered') {
      const had = this.asks.has(p.id);
      this.asks.set(p.id, { from: p.from, to: p.to, at: now });
      return !had;
    }
    return this.asks.delete(p.id);
  }

  pairActive(a: string, b: string, now = Date.now()): boolean {
    for (const [id, x] of this.asks) {
      if (now - x.at > ASK_TTL_MS) {
        this.asks.delete(id);
        continue;
      }
      if ((x.from === a && x.to === b) || (x.from === b && x.to === a)) return true;
    }
    return false;
  }

  launch(e: Omit<Envelope, 'lens' | 'duration' | 'start'> & { lens?: number[] }, now: number, zoom: number): void {
    if (this.seen.has(e.id)) return;
    this.seen.add(e.id);
    if (this.seen.size > 512) this.seen = new Set(Array.from(this.seen).slice(-256));
    const lens = e.lens ?? arcLengths(e.pts);
    const worldLen = lens[lens.length - 1] ?? 0;
    const duration = Math.min(MAX_FLIGHT_S, Math.max(MIN_FLIGHT_S, (worldLen * zoom) / ENVELOPE_SPEED)) * 1000;
    if (this.flying.length >= MAX_ENVELOPES) this.flying.shift();
    this.flying.push({ ...e, lens, start: now, duration });
  }

  get active(): boolean {
    return this.flying.length > 0;
  }

  activeConnections(): Set<string> {
    const out = new Set<string>();
    for (const e of this.flying) if (e.connectionId) out.add(e.connectionId);
    return out;
  }

  frame(now: number): EnvelopeFrame[] {
    const out: EnvelopeFrame[] = [];
    const keep: Envelope[] = [];
    for (const e of this.flying) {
      const t = (now - e.start) / e.duration;
      const ringT = (now - e.start - e.duration) / (ARRIVAL_S * 1000);
      if (ringT >= 1) continue;
      keep.push(e);
      if (t < 1) {
        const p = pointAtLength(e.pts, e.lens, ease(Math.max(0, t)));
        const sway = Math.sin((now - e.start) / 90) * 0.12;
        out.push({ x: p.x, y: p.y, angle: p.angle + sway, color: e.color, t, ring: 0, alpha: 1 });
      } else {
        const p = pointAtLength(e.pts, e.lens, 1);
        out.push({ x: p.x, y: p.y, angle: 0, color: 0xf4d35e, t: 1 + ringT, ring: ringT, alpha: 1 - ringT });
      }
    }
    this.flying = keep;
    return out;
  }

  clear(): void {
    this.flying = [];
    this.asks.clear();
  }
}

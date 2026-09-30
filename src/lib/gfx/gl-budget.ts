// Global WebGL context budget (02-motor-do-canvas §8.2).
// Browsers keep about 16 live WebGL contexts per page and silently kill the
// oldest one past that. Every owner (canvas L0, each xterm WebGL addon, the
// world, the pet) asks for a lease here first; when the budget is full the
// least recently used lease of a lower-or-equal priority is evicted and its
// owner falls back (xterm to the DOM renderer, for instance).

export type GlOwnerKind = 'canvas' | 'xterm' | 'world' | 'pet' | 'other';

export interface GlLeaseRequest {
  kind: GlOwnerKind;
  /** Higher survives longer. Canvas L0 = 100, focused xterm = 50, visible xterm = 20. */
  priority: number;
  /** Called when the lease is taken back; the owner must drop its context. */
  onEvict?: () => void;
  /** Free-form label for debugging. */
  label?: string;
}

export interface GlLease {
  readonly id: number;
  readonly kind: GlOwnerKind;
  priority: number;
  /** Marks the lease as recently used (LRU). */
  touch(): void;
  /** Gives the slot back. Idempotent. */
  release(): void;
  readonly alive: boolean;
}

/** Total contexts we allow ourselves: 16 minus slack for the rest of the app. */
export const GL_TOTAL_LIMIT = 13;
/** xterm WebGL addons at most (the focused one and the most recent visible). */
export const GL_XTERM_LIMIT = 6;

interface Entry {
  id: number;
  kind: GlOwnerKind;
  priority: number;
  lastUsed: number;
  onEvict?: () => void;
  label?: string;
  alive: boolean;
}

export class GlBudget {
  private entries = new Map<number, Entry>();
  private nextId = 1;
  private clock = 0;

  constructor(
    readonly totalLimit = GL_TOTAL_LIMIT,
    readonly perKind: Partial<Record<GlOwnerKind, number>> = { xterm: GL_XTERM_LIMIT },
  ) {}

  /** Asks for a slot. Returns null when nothing of lower or equal priority can be evicted. */
  acquire(req: GlLeaseRequest): GlLease | null {
    const kindLimit = this.perKind[req.kind];
    if (kindLimit !== undefined && this.count(req.kind) >= kindLimit) {
      if (!this.evictOne(req.priority, req.kind)) return null;
    }
    if (this.entries.size >= this.totalLimit) {
      if (!this.evictOne(req.priority)) return null;
    }
    const entry: Entry = {
      id: this.nextId++,
      kind: req.kind,
      priority: req.priority,
      lastUsed: ++this.clock,
      onEvict: req.onEvict,
      label: req.label,
      alive: true,
    };
    this.entries.set(entry.id, entry);
    const budget = this;
    return {
      id: entry.id,
      kind: entry.kind,
      get priority() {
        return entry.priority;
      },
      set priority(p: number) {
        entry.priority = p;
      },
      touch() {
        if (entry.alive) entry.lastUsed = ++budget.clock;
      },
      release() {
        if (!entry.alive) return;
        entry.alive = false;
        budget.entries.delete(entry.id);
      },
      get alive() {
        return entry.alive;
      },
    };
  }

  count(kind?: GlOwnerKind): number {
    if (!kind) return this.entries.size;
    let n = 0;
    for (const e of this.entries.values()) if (e.kind === kind) n++;
    return n;
  }

  /** Evicts the LRU entry with priority <= `maxPriority` (optionally of one kind). */
  private evictOne(maxPriority: number, kind?: GlOwnerKind): boolean {
    let victim: Entry | null = null;
    for (const e of this.entries.values()) {
      if (kind && e.kind !== kind) continue;
      if (e.priority > maxPriority) continue;
      if (
        !victim ||
        e.priority < victim.priority ||
        (e.priority === victim.priority && e.lastUsed < victim.lastUsed)
      ) {
        victim = e;
      }
    }
    if (!victim) return false;
    victim.alive = false;
    this.entries.delete(victim.id);
    try {
      victim.onEvict?.();
    } catch {
      // An owner failing to clean up must not break the budget.
    }
    return true;
  }

  snapshot(): Array<{ id: number; kind: GlOwnerKind; priority: number; label?: string }> {
    return Array.from(this.entries.values()).map((e) => ({
      id: e.id,
      kind: e.kind,
      priority: e.priority,
      label: e.label,
    }));
  }
}

/** The page-wide budget every context owner shares. */
export const glBudget = new GlBudget();

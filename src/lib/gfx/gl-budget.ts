type GlOwnerKind = 'canvas' | 'xterm' | 'world' | 'pet' | 'other';

interface GlLeaseRequest {
  kind: GlOwnerKind;
  priority: number;
  onEvict?: () => void;
  label?: string;
}

export interface GlLease {
  readonly id: number;
  readonly kind: GlOwnerKind;
  priority: number;
  touch(): void;
  release(): void;
  readonly alive: boolean;
}

const GL_TOTAL_LIMIT = 13;
const GL_XTERM_LIMIT = 6;

interface Entry {
  id: number;
  kind: GlOwnerKind;
  priority: number;
  lastUsed: number;
  onEvict?: () => void;
  label?: string;
  alive: boolean;
}

class GlBudget {
  private entries = new Map<number, Entry>();
  private nextId = 1;
  private clock = 0;

  constructor(
    readonly totalLimit = GL_TOTAL_LIMIT,
    readonly perKind: Partial<Record<GlOwnerKind, number>> = { xterm: GL_XTERM_LIMIT },
  ) {}

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

export const glBudget = new GlBudget();

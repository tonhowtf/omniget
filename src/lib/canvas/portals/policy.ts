import { allocate, livePortalsLimit } from '../lod';
import type { CanvasEngine } from '../engine.svelte';
import { closePortal, hidePortal, refreshSnapshot, setPortalBounds, type PortalBounds } from './client';

export const SETTLE_MS = 150;
const LIVE_ZOOM_MIN = 0.5;
const LIVE_ZOOM_MAX = 2;
const UNLOAD_AFTER_MS = 5 * 60_000;
const LIVE_SNAPSHOT_EVERY_MS = 10_000;
const COVER_POLL_MS = 300;

export interface Rect {
  x: number;
  y: number;
  w: number;
  h: number;
}

export interface Candidate {
  id: string;
  rect: Rect;
  stableMs: number;
  covered: boolean;
  priority: number;
}

export interface PolicyInput {
  cameraStill: boolean;
  zoom: number;
  container: Rect;
  budget: number;
  pageVisible: boolean;
  candidates: Candidate[];
}

export function inside(r: Rect, c: Rect, tol = 0.5): boolean {
  return r.x >= c.x - tol && r.y >= c.y - tol && r.x + r.w <= c.x + c.w + tol && r.y + r.h <= c.y + c.h + tol;
}

function decideLive(input: PolicyInput): Set<string> {
  if (!input.pageVisible || !input.cameraStill) return new Set();
  if (input.zoom < LIVE_ZOOM_MIN || input.zoom > LIVE_ZOOM_MAX) return new Set();
  const ok = input.candidates.filter(
    (c) =>
      c.stableMs >= SETTLE_MS &&
      !c.covered &&
      c.rect.w >= 40 &&
      c.rect.h >= 30 &&
      inside(c.rect, input.container),
  );
  return allocate(
    ok.map((c) => ({ id: c.id, score: c.priority })),
    input.budget,
  );
}

interface PortalEntry {
  id: string;
  body: () => HTMLElement | null;
  host: () => HTMLElement | null;
  priority: () => number;
  ensureOpen: () => Promise<boolean>;
  onLive: (live: boolean) => void;
  raised?: () => boolean;
}

interface Track {
  entry: PortalEntry;
  live: boolean;
  rect: Rect | null;
  changedAt: number;
  sent: string;
  lastSnap: number;
  opening: boolean;
}

function toRect(r: DOMRect): Rect {
  return { x: r.left, y: r.top, w: r.width, h: r.height };
}

function sameRect(a: Rect | null, b: Rect): boolean {
  return !!a && Math.abs(a.x - b.x) < 0.5 && Math.abs(a.y - b.y) < 0.5 && Math.abs(a.w - b.w) < 0.5 && Math.abs(a.h - b.h) < 0.5;
}

export function covered(rect: Rect, host: HTMLElement): boolean {
  if (typeof document === 'undefined' || !document.elementFromPoint) return false;
  const fx = [0.15, 0.5, 0.85];
  const fy = [0.15, 0.5, 0.85];
  for (const a of fx) {
    for (const b of fy) {
      const hit = document.elementFromPoint(rect.x + rect.w * a, rect.y + rect.h * b);
      if (!hit) return true;
      if (host.contains(hit)) continue;
      if (hit.closest('[data-cover-ignore]')) continue;
      return true;
    }
  }
  return false;
}

class PortalPolicy {
  private tracks = new Map<string, Track>();
  private unloads = new Map<string, ReturnType<typeof setTimeout>>();
  private offFrame: (() => void) | null = null;
  private settleTimer: ReturnType<typeof setTimeout> | null = null;
  private poll: ReturnType<typeof setInterval> | null = null;
  private onVis = () => this.evaluate();
  private onResize = () => this.kick();

  constructor(private engine: CanvasEngine) {}

  register(entry: PortalEntry): () => void {
    this.cancelUnload(entry.id);
    this.tracks.set(entry.id, {
      entry,
      live: false,
      rect: null,
      changedAt: performance.now(),
      sent: '',
      lastSnap: 0,
      opening: false,
    });
    if (!this.offFrame) {
      this.offFrame = this.engine.onFrame(() => this.onFrame());
      document.addEventListener('visibilitychange', this.onVis);
      window.addEventListener('resize', this.onResize);
    }
    this.kick();
    return () => this.unregister(entry.id);
  }

  private unregister(id: string): void {
    const t = this.tracks.get(id);
    if (!t) return;
    this.tracks.delete(id);
    if (t.live) {
      t.live = false;
      void hidePortal(id).catch(() => {});
    }
    this.scheduleUnload(id);
    if (this.tracks.size === 0) this.stop();
  }

  kick(): void {
    this.onFrame();
  }

  destroy(): void {
    for (const id of Array.from(this.tracks.keys())) this.unregister(id);
    this.stop();
  }

  private stop(): void {
    this.offFrame?.();
    this.offFrame = null;
    if (this.settleTimer) clearTimeout(this.settleTimer);
    this.settleTimer = null;
    if (this.poll) clearInterval(this.poll);
    this.poll = null;
    document.removeEventListener('visibilitychange', this.onVis);
    window.removeEventListener('resize', this.onResize);
  }

  private onFrame(): void {
    if (this.engine.moving || this.engine.pinching) {
      for (const [id, t] of this.tracks) if (t.live && !t.entry.raised?.()) this.goDormant(id, t);
      this.evaluateRaised();
    } else {
      this.evaluate();
    }
    if (this.settleTimer) clearTimeout(this.settleTimer);
    this.settleTimer = setTimeout(() => {
      this.settleTimer = null;
      this.evaluate();
    }, SETTLE_MS + 30);
  }

  private container(): Rect {
    const any = this.tracks.values().next().value as Track | undefined;
    const root = any?.entry.host()?.closest<HTMLElement>('.canvas-root');
    if (root) return toRect(root.getBoundingClientRect());
    return { x: 0, y: 0, w: window.innerWidth, h: window.innerHeight };
  }

  private evaluateRaised(): Set<string> {
    const raised = new Set<string>();
    const visible = typeof document === 'undefined' || document.visibilityState !== 'hidden';
    for (const [id, t] of this.tracks) {
      if (!t.entry.raised?.()) continue;
      raised.add(id);
      const body = t.entry.body();
      if (!body || !body.isConnected || !visible) {
        if (t.live) this.goDormant(id, t);
        continue;
      }
      const r = toRect(body.getBoundingClientRect());
      if (r.w >= 40 && r.h >= 30) this.goLive(id, t, r, 1);
    }
    return raised;
  }

  evaluate(): void {
    const now = performance.now();
    const cands: Candidate[] = [];
    const rects = new Map<string, Rect>();
    const raised = this.evaluateRaised();
    for (const [id, t] of this.tracks) {
      if (raised.has(id)) continue;
      const body = t.entry.body();
      const host = t.entry.host();
      if (!body || !host || !body.isConnected) continue;
      const r = toRect(body.getBoundingClientRect());
      if (!sameRect(t.rect, r)) {
        t.rect = r;
        t.changedAt = now;
      }
      rects.set(id, r);
      cands.push({
        id,
        rect: r,
        stableMs: now - t.changedAt,
        covered: covered(r, host),
        priority: t.entry.priority(),
      });
    }
    const zoom = this.engine.camera.zoom;
    const live = decideLive({
      cameraStill: !this.engine.moving && !this.engine.pinching,
      zoom,
      container: this.container(),
      budget: livePortalsLimit(),
      pageVisible: typeof document === 'undefined' || document.visibilityState !== 'hidden',
      candidates: cands,
    });
    for (const [id, t] of this.tracks) {
      if (raised.has(id)) continue;
      if (live.has(id)) {
        const r = rects.get(id);
        if (r) this.goLive(id, t, r, zoom);
      } else if (t.live) {
        this.goDormant(id, t);
      }
    }
    const anyLive = Array.from(this.tracks.values()).some((t) => t.live);
    if (anyLive && !this.poll) this.poll = setInterval(() => this.evaluate(), COVER_POLL_MS);
    else if (!anyLive && this.poll) {
      clearInterval(this.poll);
      this.poll = null;
    }
    if (cands.some((c) => c.stableMs < SETTLE_MS) && !this.settleTimer) {
      this.settleTimer = setTimeout(() => {
        this.settleTimer = null;
        this.evaluate();
      }, SETTLE_MS + 30);
    }
  }

  private goLive(id: string, t: Track, r: Rect, zoom: number): void {
    this.cancelUnload(id);
    const bounds: PortalBounds = {
      x: r.x,
      y: r.y,
      width: r.w,
      height: r.h,
      scale: window.devicePixelRatio || 1,
      zoom,
    };
    const key = `${Math.round(r.x)},${Math.round(r.y)},${Math.round(r.w)},${Math.round(r.h)},${zoom.toFixed(3)}`;
    const now = performance.now();
    if (t.live && t.sent === key) {
      if (now - t.lastSnap > LIVE_SNAPSHOT_EVERY_MS) {
        t.lastSnap = now;
        void refreshSnapshot(id);
      }
      return;
    }
    if (t.opening) return;
    t.opening = true;
    void t.entry
      .ensureOpen()
      .then(async (ok) => {
        if (!ok || !this.tracks.has(id)) return;
        await setPortalBounds(id, bounds);
        const wasLive = t.live;
        t.live = true;
        t.sent = key;
        if (!wasLive) {
          t.entry.onLive(true);
          t.lastSnap = performance.now();
          setTimeout(() => {
            if (t.live) void refreshSnapshot(id);
          }, 1000);
        }
      })
      .catch(() => {})
      .finally(() => {
        t.opening = false;
      });
  }

  private goDormant(id: string, t: Track): void {
    t.live = false;
    t.sent = '';
    t.entry.onLive(false);
    void hidePortal(id)
      .catch(() => {})
      .finally(() => void refreshSnapshot(id));
    this.scheduleUnload(id);
  }

  private cancelUnload(id: string): void {
    const u = this.unloads.get(id);
    if (u) clearTimeout(u);
    this.unloads.delete(id);
  }

  private scheduleUnload(id: string): void {
    this.cancelUnload(id);
    this.unloads.set(
      id,
      setTimeout(() => {
        this.unloads.delete(id);
        if (this.tracks.get(id)?.live) return;
        void closePortal(id).catch(() => {});
      }, UNLOAD_AFTER_MS),
    );
  }
}

const policies = new WeakMap<CanvasEngine, PortalPolicy>();

export function policyFor(engine: CanvasEngine): PortalPolicy {
  let p = policies.get(engine);
  if (!p) {
    p = new PortalPolicy(engine);
    policies.set(engine, p);
  }
  return p;
}

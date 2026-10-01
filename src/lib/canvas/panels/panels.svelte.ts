import { invoke } from '@tauri-apps/api/core';
import { GRID, isTerminalKind, type Rect } from '../model/types';
import type { CanvasEngine } from '../engine.svelte';

export type DockEdge = 'left' | 'right';

export interface DockedPanel {
  nodeId: string;
  edge: DockEdge;
  width: number;
}

interface PanelsState {
  lifted?: string | null;
  docked?: DockedPanel[];
}

const PANEL_BAR_H = 32;
const DOCK_DEFAULT_W = 520;
const DOCK_MIN_W = 280;
const DOCK_SNAP_BACK = 30;

function same(a: PanelsState, b: PanelsState): boolean {
  return JSON.stringify(norm(a)) === JSON.stringify(norm(b));
}

function norm(p: PanelsState | null | undefined): { lifted: string | null; docked: DockedPanel[] } {
  const docked = Array.isArray(p?.docked)
    ? p!.docked
        .filter((d) => d && typeof d.nodeId === 'string' && (d.edge === 'left' || d.edge === 'right'))
        .map((d) => ({ nodeId: d.nodeId, edge: d.edge, width: Math.round(Number(d.width) || DOCK_DEFAULT_W) }))
    : [];
  return { lifted: typeof p?.lifted === 'string' && p.lifted ? p.lifted : null, docked };
}

export class Panels {
  lifted = $state<string | null>(null);
  docked = $state.raw<DockedPanel[]>([]);
  sidebarW = $state(0);
  rev = $state(0);
  private workspaceId = '';
  private persistTimer: ReturnType<typeof setTimeout> | null = null;

  constructor(private engine: CanvasEngine) {}

  isRaised(id: string): boolean {
    return this.lifted === id || this.docked.some((d) => d.nodeId === id);
  }

  dockOf(id: string): DockedPanel | undefined {
    return this.docked.find((d) => d.nodeId === id);
  }

  raisedIds(): string[] {
    const out = this.docked.map((d) => d.nodeId).filter((id) => !!this.engine.store.node(id));
    if (this.lifted && this.engine.store.node(this.lifted) && !out.includes(this.lifted)) out.push(this.lifted);
    return out;
  }

  dockWidth(edge: DockEdge): number {
    let w = 0;
    for (const d of this.docked) if (d.edge === edge && this.engine.store.node(d.nodeId)) w = Math.max(w, d.width);
    return w;
  }

  rect(id: string): Rect | null {
    void this.rev;
    const vw = this.engine.camera.vw;
    const vh = this.engine.camera.vh;
    const d = this.dockOf(id);
    if (d && this.lifted !== id) {
      const w = Math.min(d.width, Math.max(DOCK_MIN_W, vw - this.sidebarW - 120));
      const x = d.edge === 'left' ? this.sidebarW : vw - w;
      return { x, y: PANEL_BAR_H, w, h: Math.max(80, vh - PANEL_BAR_H) };
    }
    if (this.lifted !== id) return null;
    const left = this.sidebarW + this.dockWidth('left');
    const right = vw - this.dockWidth('right');
    const usable = Math.max(320, right - left);
    const node = this.engine.store.node(id);
    const wide = node?.kind === 'filetree';
    const w = Math.round(Math.max(320, wide ? usable - 80 : Math.min(usable - 120, 1100)));
    const h = Math.round(Math.max(200, Math.min(vh - 140, 780)));
    const x = Math.round(left + (usable - w) / 2);
    const y = Math.round(Math.max(PANEL_BAR_H + 8, (vh - h) / 2 + PANEL_BAR_H / 2));
    return { x, y, w, h };
  }

  barRect(id: string): Rect | null {
    const r = this.rect(id);
    return r ? { x: r.x, y: r.y - PANEL_BAR_H, w: r.w, h: PANEL_BAR_H } : null;
  }

  snapshot(): { lifted: string | null; docked: DockedPanel[] } {
    return { lifted: this.lifted, docked: this.docked.map((d) => ({ ...d })) };
  }

  lift(id: string | null): void {
    if (id && !this.engine.store.node(id)) return;
    if (this.lifted === id) return;
    this.lifted = id;
    if (id) this.engine.selection.set([id]);
    else this.engine.exitFocus();
    this.changed();
  }

  toggleLift(id: string): void {
    this.lift(this.lifted === id ? null : id);
  }

  dock(id: string, edge: DockEdge | 'none', width?: number): void {
    if (!this.engine.store.node(id)) return;
    const rest = this.docked.filter((d) => d.nodeId !== id && (edge === 'none' || d.edge !== edge));
    if (edge === 'none') {
      this.docked = rest;
    } else {
      this.docked = [...rest, { nodeId: id, edge, width: this.snapWidth(width ?? this.dockOf(id)?.width ?? DOCK_DEFAULT_W) }];
      if (this.lifted === id) this.lifted = null;
    }
    this.changed();
  }

  setDockWidth(id: string, width: number, persist = true): void {
    const d = this.dockOf(id);
    if (!d) return;
    const w = this.snapWidth(width);
    if (w === d.width) {
      if (persist) this.changed();
      return;
    }
    this.docked = this.docked.map((x) => (x.nodeId === id ? { ...x, width: w } : x));
    this.changed(persist);
  }

  snapWidth(w: number): number {
    const max = Math.max(DOCK_MIN_W, this.engine.camera.vw - this.sidebarW - 120);
    let v = Math.round(Math.max(DOCK_MIN_W, Math.min(max, w)) / GRID) * GRID;
    if (Math.abs(v - DOCK_DEFAULT_W) <= DOCK_SNAP_BACK) v = DOCK_DEFAULT_W;
    return v;
  }

  clear(): void {
    if (!this.lifted && !this.docked.length) return;
    this.lifted = null;
    this.docked = [];
    this.changed();
  }

  cycle(delta: number): boolean {
    if (!this.lifted) return false;
    const order = this.order();
    if (!order.length) return false;
    const i = order.findIndex((n) => n === this.lifted);
    const next = order[(((i < 0 ? 0 : i + delta) % order.length) + order.length) % order.length];
    this.lift(next);
    return true;
  }

  liftIndex(index: number): boolean {
    if (!this.lifted) return false;
    const n = this.engine.numbered()[index - 1];
    if (!n) return false;
    this.lift(n.id);
    return true;
  }

  order(): string[] {
    return this.engine.store.nodes
      .filter((n) => !n.stackId && (isTerminalKind(n.kind) || ['note', 'portal', 'filetree', 'file', 'text'].includes(n.kind)))
      .sort((a, b) => Math.round(a.y / 200) - Math.round(b.y / 200) || a.x - b.x)
      .map((n) => n.id);
  }

  forget(id: string): void {
    if (!this.isRaised(id)) return;
    if (this.lifted === id) this.lifted = null;
    this.docked = this.docked.filter((d) => d.nodeId !== id);
    this.changed();
  }

  syncFrom(workspaceId: string, p: PanelsState | null | undefined): void {
    if (workspaceId !== this.workspaceId) {
      this.workspaceId = workspaceId;
      const n = norm(p);
      this.lifted = n.lifted;
      this.docked = n.docked;
      this.engine.invalidate();
      return;
    }
    if (this.persistTimer) return;
    if (same(p ?? {}, this.snapshot())) return;
    const n = norm(p);
    this.lifted = n.lifted;
    this.docked = n.docked;
    if (n.lifted && this.engine.store.node(n.lifted)) this.engine.selection.set([n.lifted]);
    this.engine.invalidate();
  }

  private changed(persist = true): void {
    this.engine.invalidate();
    if (!persist || !this.workspaceId) return;
    if (this.persistTimer) clearTimeout(this.persistTimer);
    this.persistTimer = setTimeout(() => {
      this.persistTimer = null;
      const snap = this.snapshot();
      const panels = snap.lifted || snap.docked.length ? snap : null;
      void invoke('canvas_workspace_update', { workspaceId: this.workspaceId, patch: { panels } }).catch(() => {});
    }, 150);
  }
}

import { GridIndex } from '../spatial/grid-index';
import { boundsOf, expand } from '../spatial/geometry';
import { applyOp, docFrom, emptyDoc, type DocState } from './ops';
import type { CameraState, CanvasNode, Connection, Drawing, Group, Op, Origin, Rect, Snapshot } from './types';

type StoreListener = (ops: Op[], origin: Origin) => void;

const LINEAR_DRAWINGS = new Set(['pen', 'brush', 'smartDraw', 'smart', 'highlighter', 'arrow', 'line']);

export function drawingBounds(d: Drawing): Rect {
  const pad = (d.strokeWidth ?? 2) + 2 + (d.kind === 'brush' ? d.strokeWidth ?? 2 : 0);
  let r: Rect =
    d.points && d.points.length >= 2 && LINEAR_DRAWINGS.has(d.kind) ? boundsOf(d.points, d.x, d.y) : { x: d.x, y: d.y, w: d.w, h: d.h };
  if (d.rotation) {
    const a = (d.rotation * Math.PI) / 180;
    const c = Math.abs(Math.cos(a));
    const s = Math.abs(Math.sin(a));
    const cx = d.x + d.w / 2;
    const cy = d.y + d.h / 2;
    const w = r.w * c + r.h * s;
    const h = r.w * s + r.h * c;
    const rcx = r.x + r.w / 2 - cx;
    const rcy = r.y + r.h / 2 - cy;
    const ncx = cx + rcx * Math.cos(a) - rcy * Math.sin(a);
    const ncy = cy + rcx * Math.sin(a) + rcy * Math.cos(a);
    r = { x: ncx - w / 2, y: ncy - h / 2, w, h };
  }
  return expand(r, pad);
}

export class CanvasStore {
  doc: DocState = emptyDoc();
  readonly nodeIndex = new GridIndex();
  readonly drawingIndex = new GridIndex();
  version = $state(0);
  structure = $state(0);
  private nodeVersions = $state.raw<Record<string, number>>({});
  private listeners = new Set<StoreListener>();
  private cache: { v: number; nodes: CanvasNode[]; connections: Connection[]; drawings: Drawing[]; groups: Group[] } | null = null;

  workspaceId = $state('');
  floorId = $state('');
  epoch: string | number = 0;
  rev = 0;

  load(snap: Snapshot): void {
    this.doc = docFrom(snap);
    this.workspaceId = snap.workspaceId;
    this.floorId = snap.floorId;
    this.epoch = snap.epoch;
    this.rev = snap.rev;
    this.nodeIndex.clear();
    this.drawingIndex.clear();
    for (const n of this.doc.nodes.values()) this.nodeIndex.insert(n.id, n);
    for (const d of this.doc.drawings.values()) this.drawingIndex.insert(d.id, drawingBounds(d));
    this.nodeVersions = {};
    this.cache = null;
    this.version++;
    this.structure++;
    for (const l of this.listeners) l([], 'load');
  }

  subscribe(fn: StoreListener): () => void {
    this.listeners.add(fn);
    return () => this.listeners.delete(fn);
  }

  apply(ops: Op[], origin: Origin): Op[] {
    const inverse: Op[] = [];
    let structural = false;
    const touched: string[] = [];
    for (const op of ops) {
      const inv = applyOp(this.doc, op);
      if (!inv) continue;
      inverse.push(inv);
      switch (op.op) {
        case 'add_node': {
          this.nodeIndex.insert(op.node.id, op.node);
          touched.push(op.node.id);
          structural = true;
          break;
        }
        case 'update_node': {
          const n = this.doc.nodes.get(op.id);
          if (n) this.nodeIndex.update(n.id, n);
          touched.push(op.id);
          if ('z' in op.patch || 'groupId' in op.patch) structural = true;
          break;
        }
        case 'remove_node':
          this.nodeIndex.remove(op.id);
          touched.push(op.id);
          structural = true;
          break;
        case 'add_drawing':
          this.drawingIndex.insert(op.drawing.id, drawingBounds(op.drawing));
          structural = true;
          break;
        case 'update_drawing': {
          const d = this.doc.drawings.get(op.id);
          if (d) this.drawingIndex.update(d.id, drawingBounds(d));
          structural = true;
          break;
        }
        case 'remove_drawing':
          this.drawingIndex.remove(op.id);
          structural = true;
          break;
        case 'set_camera':
          break;
        default:
          structural = true;
      }
    }
    inverse.reverse();
    if (inverse.length === 0) return inverse;
    if (touched.length) {
      const next = { ...this.nodeVersions };
      for (const id of touched) next[id] = (next[id] ?? 0) + 1;
      this.nodeVersions = next;
    }
    this.cache = null;
    this.version++;
    if (structural) this.structure++;
    for (const l of this.listeners) l(ops, origin);
    return inverse;
  }

  nodeVersion(id: string): number {
    return this.nodeVersions[id] ?? 0;
  }

  private lists() {
    if (this.cache && this.cache.v === this.version) return this.cache;
    const nodes = Array.from(this.doc.nodes.values()).sort((a, b) => a.z - b.z);
    this.cache = {
      v: this.version,
      nodes,
      connections: Array.from(this.doc.connections.values()),
      drawings: Array.from(this.doc.drawings.values()).sort((a, b) => (a.z ?? 0) - (b.z ?? 0)),
      groups: Array.from(this.doc.groups.values()),
    };
    return this.cache;
  }

  get nodes(): CanvasNode[] {
    return this.lists().nodes;
  }

  get connections(): Connection[] {
    return this.lists().connections;
  }

  get drawings(): Drawing[] {
    return this.lists().drawings;
  }

  get groups(): Group[] {
    return this.lists().groups;
  }

  get camera(): CameraState | null {
    return this.doc.camera;
  }

  node(id: string): CanvasNode | undefined {
    return this.doc.nodes.get(id);
  }

  connection(id: string): Connection | undefined {
    return this.doc.connections.get(id);
  }

  drawing(id: string): Drawing | undefined {
    return this.doc.drawings.get(id);
  }

  group(id: string): Group | undefined {
    return this.doc.groups.get(id);
  }

  groupMembers(groupId: string): CanvasNode[] {
    return this.nodes.filter((n) => n.groupId === groupId);
  }

  drawingGroupMembers(groupId: string): Drawing[] {
    return this.drawings.filter((d) => d.groupId === groupId);
  }

  maxZ(): number {
    let z = 0;
    for (const n of this.doc.nodes.values()) if (n.z > z) z = n.z;
    return z;
  }

  minZ(): number {
    let z = Infinity;
    for (const n of this.doc.nodes.values()) if (n.z < z) z = n.z;
    return Number.isFinite(z) ? z : 0;
  }

  maxDrawingZ(): number {
    let z = 0;
    for (const d of this.doc.drawings.values()) if ((d.z ?? 0) > z) z = d.z ?? 0;
    return z;
  }

  snapshot(): Snapshot {
    return {
      workspaceId: this.workspaceId,
      floorId: this.floorId,
      epoch: this.epoch,
      rev: this.rev,
      camera: this.doc.camera ?? undefined,
      nodes: this.nodes,
      connections: this.connections,
      drawings: this.drawings,
      groups: this.groups,
    };
  }
}

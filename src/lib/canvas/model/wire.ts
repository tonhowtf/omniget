// Translation between the front's document shapes and the Rust wire format
// (scratchpad contract-doc.md, owner doc-rust). Nodes and connections are the
// same shape; drawings differ (tool/origin/size/lineWidth on the wire) and
// patches use `null` to remove a field where the front uses `undefined`.

import type { CanvasStore } from './store.svelte';
import type { CanvasNode, Connection, Drawing, Group, Op } from './types';

export interface WireDrawing {
  id: string;
  tool: string;
  points: number[];
  origin: { x: number; y: number };
  originalSize?: { w: number; h: number };
  size?: { w: number; h: number };
  color?: string;
  lineWidth?: number;
  opacity?: number;
  z: number;
  locked?: boolean;
  rotation?: number;
  textContent?: string;
  textFontSize?: number;
  strokePattern?: string;
  fillStyle?: string;
  groupId?: string;
  [k: string]: unknown;
}

/** Stable sketch seed from the id, so a shape looks the same in every session. */
export function seedFromId(id: string): number {
  let h = 2166136261;
  for (let i = 0; i < id.length; i++) {
    h ^= id.charCodeAt(i);
    h = Math.imul(h, 16777619);
  }
  return (h >>> 0) || 1;
}

const FREEFORM = new Set(['pen', 'brush', 'smartDraw', 'smart', 'highlighter', 'arrow', 'line']);
const SKETCH_PATTERN = 'sketch';

/** Front fill names ↔ wire fill names (the wire is Maestri's). */
function fillToWire(f: Drawing['fillStyle']): string | undefined {
  if (!f || f === 'none') return undefined;
  return f === 'cross' ? 'crossHatch' : f;
}

function fillFromWire(f: unknown): Drawing['fillStyle'] | undefined {
  switch (f) {
    case 'crossHatch':
    case 'cross':
      return 'cross';
    case 'solid':
    case 'hachure':
    case 'tint':
    case 'solidOpaque':
      return f;
    default:
      return undefined;
  }
}

/** Optional wire fields copied verbatim both ways (they have the same name and shape). */
const PASSTHROUGH = ['rotation', 'cornerRadius', 'textFontSize', 'sourceConnection', 'targetConnection', 'controlPointOffset'] as const;

function drawingToWire(d: Drawing): WireDrawing {
  const points = d.points && d.points.length >= 2 ? d.points.slice() : [0, 0, d.w, d.h];
  const w: WireDrawing = {
    id: d.id,
    tool: d.kind,
    points,
    origin: { x: d.x, y: d.y },
    size: { w: d.w, h: d.h },
    originalSize: { w: d.w, h: d.h },
    lineWidth: d.strokeWidth,
    z: d.z ?? 0,
  };
  // An empty colour means "theme ink" (a drawing made without one).
  if (d.color) w.color = d.color;
  if (d.opacity !== undefined) w.opacity = d.opacity;
  if (d.text !== undefined) w.textContent = d.text;
  const fill = fillToWire(d.fillStyle);
  if (fill) w.fillStyle = fill;
  if (d.sketch) w.strokePattern = SKETCH_PATTERN;
  else if (d.strokePattern && d.strokePattern !== 'solid') w.strokePattern = d.strokePattern;
  if (d.groupId) w.groupId = d.groupId;
  if (d.locked) w.locked = true;
  const rec = d as unknown as Record<string, unknown>;
  const out = w as Record<string, unknown>;
  for (const k of PASSTHROUGH) if (rec[k] !== undefined && rec[k] !== null) out[k] = rec[k];
  return w;
}

export function drawingFromWire(w: WireDrawing): Drawing {
  const sx = w.size && w.originalSize && w.originalSize.w > 0 ? w.size.w / w.originalSize.w : 1;
  const sy = w.size && w.originalSize && w.originalSize.h > 0 ? w.size.h / w.originalSize.h : 1;
  const pts = Array.isArray(w.points) ? w.points.map((v, i) => v * (i % 2 === 0 ? sx : sy)) : [];
  let bw = w.size?.w;
  let bh = w.size?.h;
  if (bw === undefined || bh === undefined) {
    let x1 = 0;
    let y1 = 0;
    for (let i = 0; i + 1 < pts.length; i += 2) {
      x1 = Math.max(x1, pts[i]);
      y1 = Math.max(y1, pts[i + 1]);
    }
    bw = bw ?? x1;
    bh = bh ?? y1;
  }
  const d: Drawing = {
    id: w.id,
    kind: w.tool,
    x: w.origin?.x ?? 0,
    y: w.origin?.y ?? 0,
    w: bw,
    h: bh,
    z: w.z ?? 0,
    color: w.color ?? '',
    strokeWidth: w.lineWidth ?? 2,
    seed: seedFromId(w.id),
  };
  if (FREEFORM.has(w.tool)) d.points = pts;
  if (w.tool === 'text' && typeof w.textContent === 'string' && (d.w < 8 || d.h < 8)) {
    // A text drawing sent with only an origin: size it from the text.
    const fs = typeof w.textFontSize === 'number' ? w.textFontSize : 20;
    const lines = w.textContent.split('\n');
    d.w = Math.max(d.w, Math.ceil(Math.max(...lines.map((l) => l.length)) * fs * 0.6));
    d.h = Math.max(d.h, Math.ceil(lines.length * fs * 1.25));
  }
  if (w.opacity !== undefined && w.opacity !== null) d.opacity = w.opacity;
  if (typeof w.textContent === 'string') d.text = w.textContent;
  const fill = fillFromWire(w.fillStyle);
  if (fill) {
    d.fillStyle = fill;
    d.fill = d.color;
  }
  if (w.strokePattern === SKETCH_PATTERN) d.sketch = true;
  else if (w.strokePattern === 'dashed' || w.strokePattern === 'dotted') d.strokePattern = w.strokePattern;
  if (w.groupId) d.groupId = w.groupId;
  if (w.locked) d.locked = true;
  const rec = d as unknown as Record<string, unknown>;
  for (const k of PASSTHROUGH) if (w[k] !== undefined && w[k] !== null) rec[k] = w[k];
  return d;
}

/** `undefined` values become `null` (the wire's "remove this field"). */
function nullify<T extends object>(patch: T): Record<string, unknown> {
  const out: Record<string, unknown> = {};
  for (const [k, v] of Object.entries(patch)) out[k] = v === undefined ? null : v;
  return out;
}

const DRAWING_OPTIONALS = ['color', 'opacity', 'textContent', 'fillStyle', 'strokePattern', 'groupId', 'locked', ...PASSTHROUGH];

/**
 * Local op → wire op. Drawing updates are sent as the full set of wire fields
 * of the drawing as it is now in the store (so origin/size stay coherent),
 * with `null` for optionals it no longer has.
 */
function opToWire(op: Op, store: CanvasStore): Record<string, unknown> | null {
  switch (op.op) {
    case 'update_node': {
      const patch = nullify(op.patch);
      const data = (op.patch as { data?: Record<string, unknown> | null }).data;
      if (data && typeof data === 'object') patch.data = nullify(data);
      return { op: op.op, id: op.id, patch };
    }
    case 'update_connection':
      return { op: op.op, id: op.id, patch: nullify(op.patch) };
    case 'add_drawing':
      return { op: 'add_drawing', drawing: drawingToWire(op.drawing) };
    case 'update_drawing': {
      const d = store.drawing(op.id);
      if (!d) return null;
      const full = drawingToWire(d) as Record<string, unknown>;
      delete full.id;
      for (const k of DRAWING_OPTIONALS) if (!(k in full)) full[k] = null;
      return { op: 'update_drawing', id: op.id, patch: full };
    }
    case 'set_group': {
      if (!op.group) return { op: 'set_group', id: op.id };
      const g: Group = { target: 'nodes', ...op.group };
      return { op: 'set_group', id: op.id, group: g };
    }
    default:
      return op as unknown as Record<string, unknown>;
  }
}

export function opsToWire(ops: Op[], store: CanvasStore): Record<string, unknown>[] {
  const out: Record<string, unknown>[] = [];
  for (const op of ops) {
    const w = opToWire(op, store);
    if (w) out.push(w);
  }
  return out;
}

/** Wire op → local op (drawing updates are merged on the wire shape, then converted back). */
function opFromWire(raw: unknown, store: CanvasStore): Op | null {
  const w = raw as Record<string, unknown> & { op?: string };
  if (!w || typeof w.op !== 'string') return null;
  switch (w.op) {
    case 'add_drawing':
      return { op: 'add_drawing', drawing: drawingFromWire(w.drawing as WireDrawing) };
    case 'update_drawing': {
      const id = String(w.id);
      const cur = store.drawing(id);
      if (!cur) return null;
      const merged = drawingToWire(cur) as Record<string, unknown>;
      for (const [k, v] of Object.entries((w.patch ?? {}) as Record<string, unknown>)) {
        if (v === null) delete merged[k];
        else merged[k] = v;
      }
      return { op: 'add_drawing', drawing: drawingFromWire(merged as WireDrawing) };
    }
    case 'set_group':
      return { op: 'set_group', id: String(w.id), group: (w.group as Group | undefined) ?? null };
    case 'set_cable_tie':
      return null;
    case 'add_node': {
      const n = w.node as CanvasNode;
      return { op: 'add_node', node: { ...n, z: n.z ?? 0, data: n.data ?? {} } };
    }
    case 'add_connection': {
      const c = w.connection as Connection;
      return { op: 'add_connection', connection: { ...c, perms: c.perms ?? [] } };
    }
    default:
      return w as unknown as Op;
  }
}

/**
 * Converts and applies wire ops one by one (a drawing update may follow its
 * own add in the same batch). Returns the local ops that were applied.
 */
export function applyWireOps(list: unknown[] | undefined, store: CanvasStore, origin: string): Op[] {
  const applied: Op[] = [];
  for (const raw of list ?? []) {
    const op = opFromWire(raw, store);
    if (!op) continue;
    store.apply([op], origin);
    applied.push(op);
  }
  return applied;
}

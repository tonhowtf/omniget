// Document ops: apply one op to the in-memory document and return its inverse.
// The same op list is the patch sent to Rust and, inverted, the undo entry
// (the tldraw "diff" design, 02-motor-do-canvas §4.6). Applying is idempotent
// on purpose so an echoed patch never corrupts the document.

import type { CameraState, CanvasNode, Connection, Drawing, Group, Op } from './types';

export interface DocState {
  nodes: Map<string, CanvasNode>;
  connections: Map<string, Connection>;
  drawings: Map<string, Drawing>;
  groups: Map<string, Group>;
  camera: CameraState | null;
}

export function emptyDoc(): DocState {
  return {
    nodes: new Map(),
    connections: new Map(),
    drawings: new Map(),
    groups: new Map(),
    camera: null,
  };
}

/** Stable key of the record an op touches, for undo conflict checks. */
export function recordKey(op: Op): string {
  switch (op.op) {
    case 'add_node':
      return `n:${op.node.id}`;
    case 'update_node':
    case 'remove_node':
      return `n:${op.id}`;
    case 'add_connection':
      return `c:${op.connection.id}`;
    case 'update_connection':
    case 'remove_connection':
      return `c:${op.id}`;
    case 'add_drawing':
      return `d:${op.drawing.id}`;
    case 'update_drawing':
    case 'remove_drawing':
      return `d:${op.id}`;
    case 'set_group':
      return `g:${op.id}`;
    case 'set_camera':
      return 'camera';
  }
}

function pick<T extends object>(src: T, keys: string[]): Partial<T> {
  const out: Record<string, unknown> = {};
  for (const k of keys) out[k] = (src as Record<string, unknown>)[k];
  return out as Partial<T>;
}

function withPatch<T extends object>(src: T, patch: Partial<T>): T {
  const next = { ...src } as Record<string, unknown>;
  for (const [k, v] of Object.entries(patch)) {
    // `undefined` (local) or `null` (wire) in a patch means "remove the field".
    if (v === undefined || v === null) delete next[k];
    else next[k] = v;
  }
  return next as T;
}

/**
 * Applies `op` to `doc` (mutating the maps, replacing records immutably) and
 * returns the inverse op, or null when the op changed nothing.
 */
export function applyOp(doc: DocState, op: Op): Op | null {
  switch (op.op) {
    case 'add_node': {
      const prev = doc.nodes.get(op.node.id);
      doc.nodes.set(op.node.id, op.node);
      return prev ? { op: 'add_node', node: prev } : { op: 'remove_node', id: op.node.id };
    }
    case 'update_node': {
      const prev = doc.nodes.get(op.id);
      if (!prev) return null;
      // `data` merges key by key (null/undefined removes a key), like Rust.
      const { data: dataPatch, ...rest } = op.patch as Partial<CanvasNode> & { data?: Record<string, unknown> | null };
      const next = withPatch(prev, rest as Partial<CanvasNode>);
      const inverse = pick(prev, Object.keys(rest)) as Partial<CanvasNode>;
      if (dataPatch && typeof dataPatch === 'object') {
        const data: Record<string, unknown> = { ...(prev.data ?? {}) };
        const invData: Record<string, unknown> = {};
        for (const [k, v] of Object.entries(dataPatch)) {
          invData[k] = k in (prev.data ?? {}) ? prev.data[k] : null;
          if (v === undefined || v === null) delete data[k];
          else data[k] = v;
        }
        next.data = data;
        inverse.data = invData;
      }
      doc.nodes.set(op.id, next);
      return { op: 'update_node', id: op.id, patch: inverse };
    }
    case 'remove_node': {
      const prev = doc.nodes.get(op.id);
      if (!prev) return null;
      doc.nodes.delete(op.id);
      return { op: 'add_node', node: prev };
    }
    case 'add_connection': {
      const prev = doc.connections.get(op.connection.id);
      doc.connections.set(op.connection.id, op.connection);
      return prev
        ? { op: 'add_connection', connection: prev }
        : { op: 'remove_connection', id: op.connection.id };
    }
    case 'update_connection': {
      const prev = doc.connections.get(op.id);
      if (!prev) return null;
      doc.connections.set(op.id, withPatch(prev, op.patch));
      return { op: 'update_connection', id: op.id, patch: pick(prev, Object.keys(op.patch)) };
    }
    case 'remove_connection': {
      const prev = doc.connections.get(op.id);
      if (!prev) return null;
      doc.connections.delete(op.id);
      return { op: 'add_connection', connection: prev };
    }
    case 'add_drawing': {
      const prev = doc.drawings.get(op.drawing.id);
      doc.drawings.set(op.drawing.id, op.drawing);
      return prev ? { op: 'add_drawing', drawing: prev } : { op: 'remove_drawing', id: op.drawing.id };
    }
    case 'update_drawing': {
      const prev = doc.drawings.get(op.id);
      if (!prev) return null;
      doc.drawings.set(op.id, withPatch(prev, op.patch));
      return { op: 'update_drawing', id: op.id, patch: pick(prev, Object.keys(op.patch)) };
    }
    case 'remove_drawing': {
      const prev = doc.drawings.get(op.id);
      if (!prev) return null;
      doc.drawings.delete(op.id);
      return { op: 'add_drawing', drawing: prev };
    }
    case 'set_group': {
      const prev = doc.groups.get(op.id) ?? null;
      if (op.group) doc.groups.set(op.id, op.group);
      else if (prev) doc.groups.delete(op.id);
      else return null;
      return { op: 'set_group', id: op.id, group: prev };
    }
    case 'set_camera': {
      const prev = doc.camera;
      doc.camera = op.camera;
      return prev ? { op: 'set_camera', camera: prev } : null;
    }
  }
}

/** Applies a list and returns the inverse list (already reversed, ready to apply). */
export function applyOps(doc: DocState, ops: Op[]): Op[] {
  const inverse: Op[] = [];
  for (const op of ops) {
    const inv = applyOp(doc, op);
    if (inv) inverse.push(inv);
  }
  return inverse.reverse();
}

/** Ops that remove nodes together with every cord touching them (cords first). */
export function removeNodesOps(doc: DocState, ids: Iterable<string>): Op[] {
  const set = new Set(ids);
  const ops: Op[] = [];
  for (const c of doc.connections.values()) {
    if (set.has(c.from) || set.has(c.to)) ops.push({ op: 'remove_connection', id: c.id });
  }
  for (const id of set) if (doc.nodes.has(id)) ops.push({ op: 'remove_node', id });
  return ops;
}

/**
 * Collapses a sequence of ops into the minimum that reaches the same state,
 * merging consecutive update_node patches of the same record (used when a
 * drag of many frames becomes one history entry and one patch).
 */
export function squashOps(ops: Op[]): Op[] {
  const out: Op[] = [];
  const lastUpdate = new Map<string, number>();
  for (const op of ops) {
    if (op.op === 'update_node' || op.op === 'update_connection' || op.op === 'update_drawing') {
      const key = recordKey(op);
      const idx = lastUpdate.get(key);
      if (idx !== undefined) {
        const prev = out[idx] as typeof op;
        const merged: Record<string, unknown> = { ...prev.patch, ...op.patch };
        const a = (prev.patch as { data?: unknown }).data;
        const b = (op.patch as { data?: unknown }).data;
        if (a && b && typeof a === 'object' && typeof b === 'object') merged.data = { ...a, ...b };
        out[idx] = { ...prev, patch: merged } as Op;
        continue;
      }
      lastUpdate.set(key, out.length);
      out.push(op);
    } else {
      // Anything else breaks the merge window for its record.
      lastUpdate.delete(recordKey(op));
      out.push(op);
    }
  }
  return out;
}

/** Stamps a snapshot of maps into a DocState. */
export function docFrom(parts: {
  nodes: CanvasNode[];
  connections: Connection[];
  drawings: Drawing[];
  groups: Group[];
  camera?: CameraState | null;
}): DocState {
  return {
    nodes: new Map(parts.nodes.map((n) => [n.id, n])),
    connections: new Map(parts.connections.map((c) => [c.id, c])),
    drawings: new Map(parts.drawings.map((d) => [d.id, d])),
    groups: new Map(parts.groups.map((g) => [g.id, g])),
    camera: parts.camera ?? null,
  };
}

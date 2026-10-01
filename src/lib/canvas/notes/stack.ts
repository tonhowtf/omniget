import type { CanvasEngine } from '../engine.svelte';
import type { CanvasStore } from '../model/store.svelte';
import { newId, type CanvasNode, type Op, type Rect } from '../model/types';
import { stackIdOf } from './api';

interface StackData {
  name?: string;
  hasCustomName?: boolean;
  tabs: string[];
  frontNoteId?: string;
  uniformColor?: string;
}

const STACK_SIZE = { w: 280, h: 240 };

export function stackData(node: CanvasNode): StackData {
  const d = (node.data ?? {}) as Partial<StackData>;
  return { ...d, tabs: Array.isArray(d.tabs) ? d.tabs.filter((t) => typeof t === 'string') : [] } as StackData;
}

export function dataWith(node: CanvasNode, changes: Record<string, unknown>): Record<string, unknown> {
  return { ...(node.data ?? {}), ...changes };
}

function nodePatch(id: string, patch: Record<string, unknown>): Op {
  return { op: 'update_node', id, patch: patch as Partial<CanvasNode> };
}

export function stackPages(store: CanvasStore, stack: CanvasNode): CanvasNode[] {
  const out: CanvasNode[] = [];
  for (const id of stackData(stack).tabs) {
    const n = store.node(id);
    if (n) out.push(n);
  }
  return out;
}

export function frontPage(store: CanvasStore, stack: CanvasNode): CanvasNode | undefined {
  const d = stackData(stack);
  const pages = stackPages(store, stack);
  return pages.find((p) => p.id === d.frontNoteId) ?? pages[0];
}

function addToStackOps(stack: CanvasNode, notes: CanvasNode[], index?: number, store?: CanvasStore): Op[] {
  const d = stackData(stack);
  const ids = notes.map((n) => n.id).filter((id) => id !== stack.id);
  if (!ids.length) return [];
  const tabs = d.tabs.filter((t) => !ids.includes(t));
  const at = index === undefined ? 0 : Math.max(0, Math.min(tabs.length, index));
  tabs.splice(at, 0, ...ids);
  const ops: Op[] = store ? detachOps(store, notes, stack.id) : [];
  ops.push(
    ...notes.filter((n) => n.id !== stack.id).map((n) => nodePatch(n.id, { stackId: stack.id })),
  );
  ops.push(nodePatch(stack.id, { data: dataWith(stack, { tabs, frontNoteId: ids[0] }) }));
  return ops;
}

function detachOps(store: CanvasStore, notes: CanvasNode[], keep: string): Op[] {
  const byStack = new Map<string, string[]>();
  for (const n of notes) {
    const sid = stackIdOf(n);
    if (sid && sid !== keep) byStack.set(sid, [...(byStack.get(sid) ?? []), n.id]);
  }
  const ops: Op[] = [];
  for (const [sid, ids] of byStack) {
    const st = store.node(sid);
    if (!st) continue;
    const d = stackData(st);
    const tabs = d.tabs.filter((t) => !ids.includes(t));
    if (tabs.length === 0 && !d.hasCustomName) ops.push({ op: 'remove_node', id: sid });
    else {
      const front = d.frontNoteId && ids.includes(d.frontNoteId) ? (tabs[0] ?? null) : (d.frontNoteId ?? null);
      ops.push(nodePatch(sid, { data: dataWith(st, { tabs, frontNoteId: front }) }));
    }
  }
  return ops;
}

export function removeFromStackOps(store: CanvasStore, stack: CanvasNode, noteId: string, at?: { x: number; y: number }): Op[] {
  const d = stackData(stack);
  const note = store.node(noteId);
  if (!note) return [];
  const tabs = d.tabs.filter((t) => t !== noteId);
  const pos = at ?? { x: stack.x + stack.w + 40, y: stack.y };
  const ops: Op[] = [
    nodePatch(noteId, { stackId: null, x: Math.round(pos.x / 20) * 20, y: Math.round(pos.y / 20) * 20, z: store.maxZ() + 1 }),
  ];
  if (tabs.length === 0 && !d.hasCustomName) {
    ops.push({ op: 'remove_node', id: stack.id });
  } else {
    const front = d.frontNoteId === noteId ? tabs[Math.max(0, d.tabs.indexOf(noteId) - 1)] ?? tabs[0] ?? null : d.frontNoteId ?? null;
    ops.push(nodePatch(stack.id, { data: dataWith(stack, { tabs, frontNoteId: front }) }));
  }
  return ops;
}

export function reorderOps(stack: CanvasNode, noteId: string, toIndex: number): Op[] {
  const d = stackData(stack);
  const from = d.tabs.indexOf(noteId);
  if (from < 0) return [];
  const tabs = d.tabs.slice();
  tabs.splice(from, 1);
  tabs.splice(Math.max(0, Math.min(tabs.length, toIndex)), 0, noteId);
  if (tabs.every((t, i) => t === d.tabs[i])) return [];
  return [nodePatch(stack.id, { data: dataWith(stack, { tabs }) })];
}

export function bringToFrontOps(stack: CanvasNode, noteId: string): Op[] {
  if (stackData(stack).frontNoteId === noteId) return [];
  return [nodePatch(stack.id, { data: dataWith(stack, { frontNoteId: noteId }) })];
}

export function createStackOps(store: CanvasStore, notes: CanvasNode[], rect: Rect, name?: string): { ops: Op[]; stackId: string } {
  const stack: CanvasNode = {
    id: newId(),
    kind: 'stack',
    x: rect.x,
    y: rect.y,
    w: Math.max(rect.w, STACK_SIZE.w),
    h: Math.max(rect.h, STACK_SIZE.h),
    z: store.maxZ() + 1,
    data: { tabs: [] as string[] },
  };
  stack.title = name ?? notes[0]?.title ?? 'Notes';
  if (name) {
    stack.data = { ...stack.data, name, hasCustomName: true };
  }
  const ops: Op[] = [{ op: 'add_node', node: stack }];
  ops.push(...addToStackOps(stack, notes, 0, store));
  return { ops, stackId: stack.id };
}

export function stackTargets(store: CanvasStore, noteId: string): CanvasNode[] {
  const cur = stackIdOf(store.node(noteId));
  return store.nodes.filter((n) => n.kind === 'stack' && n.id !== cur);
}

export function placeInStack(engine: CanvasEngine, noteId: string, stackId: string | null): string | null {
  const store = engine.store;
  const sel = engine.selection.list();
  const ids = sel.includes(noteId) ? sel : [noteId];
  const notes = ids.map((id) => store.node(id)).filter((n): n is CanvasNode => !!n && n.kind === 'note');
  if (!notes.length) return null;
  if (stackId) {
    const stack = store.node(stackId);
    if (!stack || stack.kind !== 'stack') return null;
    engine.commit(addToStackOps(stack, notes, 0, store), { selectionAfter: [stack.id] });
    return stack.id;
  }
  const first = notes[0];
  const { ops, stackId: id } = createStackOps(store, notes, { x: first.x, y: first.y, w: first.w, h: first.h });
  engine.commit(ops, { selectionAfter: [id] });
  return id;
}

function centerIn(n: Rect, r: Rect): boolean {
  const cx = n.x + n.w / 2;
  const cy = n.y + n.h / 2;
  return cx >= r.x && cx <= r.x + r.w && cy >= r.y && cy <= r.y + r.h;
}

const controllers = new WeakMap<CanvasEngine, () => void>();

export function ensureStackController(engine: CanvasEngine): void {
  if (controllers.has(engine) || typeof window === 'undefined') return;
  let down = false;
  const moved = new Set<string>();
  const onDown = () => {
    down = true;
    moved.clear();
  };
  const unsub = engine.store.subscribe((ops, origin) => {
    if (!down || origin !== 'user') return;
    for (const op of ops) {
      if (op.op !== 'update_node' || !('x' in op.patch || 'y' in op.patch)) continue;
      const n = engine.store.node(op.id);
      if (n && n.kind === 'note' && !stackIdOf(n)) moved.add(n.id);
    }
  });
  const onUp = (ev: PointerEvent) => {
    if (!down) return;
    down = false;
    if (!moved.size || ev.altKey) {
      moved.clear();
      return;
    }
    const ids = Array.from(moved);
    moved.clear();
    setTimeout(() => fileMoved(engine, ids), 0);
  };
  window.addEventListener('pointerdown', onDown, true);
  window.addEventListener('pointerup', onUp, true);
  controllers.set(engine, () => {
    unsub();
    window.removeEventListener('pointerdown', onDown, true);
    window.removeEventListener('pointerup', onUp, true);
  });
}

function fileMoved(engine: CanvasEngine, ids: string[]): void {
  const store = engine.store;
  const notes = ids.map((id) => store.node(id)).filter((n): n is CanvasNode => !!n && !stackIdOf(n));
  if (!notes.length) return;
  const movedSet = new Set(notes.map((n) => n.id));
  const anchor = notes[0];
  const stacks = store.nodes.filter((n) => n.kind === 'stack' && centerIn(anchor, n)).sort((a, b) => b.z - a.z);
  if (stacks[0]) {
    engine.commit(addToStackOps(stacks[0], notes, undefined, store), { selectionAfter: [stacks[0].id] });
    return;
  }
  const target = store.nodes
    .filter((n) => n.kind === 'note' && !movedSet.has(n.id) && !stackIdOf(n) && centerIn(anchor, n))
    .sort((a, b) => b.z - a.z)[0];
  if (!target) return;
  const { ops, stackId } = createStackOps(store, [target, ...notes], { x: target.x, y: target.y, w: target.w, h: target.h + 36 });
  engine.commit(ops, { selectionAfter: [stackId] });
}

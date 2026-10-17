import type { CanvasEngine } from '../engine.svelte';
import { isTerminalKind, type CanvasNode, type Rect } from '../model/types';
import { nearestFreeSpot } from '../interaction/snap';
import { sizeFor } from '../nodes/registry';
import { canvasPrefs, prefSize } from '../prefs.svelte';
import { attachmentCopy } from '../chat/client';
import { noteRead, noteWrite, stackIdOf } from '../notes/api';
import { baseName, IMAGE_RE } from './api';

type NoteInsert = (text: string) => void;
const inserts = new Map<string, NoteInsert>();

export function onNoteInsert(nodeId: string, fn: NoteInsert): () => void {
  inserts.set(nodeId, fn);
  return () => {
    if (inserts.get(nodeId) === fn) inserts.delete(nodeId);
  };
}

export function noteInsert(nodeId: string): NoteInsert | undefined {
  return inserts.get(nodeId);
}

export function fileMarkdown(abs: string, name = baseName(abs)): string {
  const label = (name || abs).replace(/[[\]]/g, '');
  return IMAGE_RE.test(abs) ? `![${label}](<${abs}>)` : `[${label}](<${abs}>)`;
}

export async function filesMarkdown(workspaceId: string, paths: string[], copy = canvasPrefs().always_copy_files): Promise<string[]> {
  const out: string[] = [];
  for (const p of paths) {
    let target = p;
    if (copy) target = await attachmentCopy(workspaceId, p).catch(() => p);
    out.push(fileMarkdown(target, baseName(p)));
  }
  return out;
}

function onFloor(engine: CanvasEngine): CanvasNode[] {
  return engine.store.nodes.filter((n) => !stackIdOf(n));
}

export function noteWithFiles(engine: CanvasEngine, text: string, near: Rect | { x: number; y: number }, connectFrom?: string, title?: string): CanvasNode {
  const size = prefSize('note') ?? sizeFor('note');
  const want = 'w' in near ? { x: near.x + near.w + 40, y: near.y } : near;
  const spot = nearestFreeSpot(want.x, want.y, size.w, size.h, onFloor(engine));
  const body = title ? `# ${title.replace(/\s+/g, ' ').trim()}\n\n${text}` : text;
  const node = engine.spawn('note', null, {
    rect: { x: spot.x, y: spot.y, w: size.w, h: size.h },
    data: { text: body },
    connectFrom,
    title: title ? engine.uniqueTitle(title) : undefined,
  });
  engine.reveal(node.id);
  void persistNoteText(engine, node.id, body);
  return node;
}

async function persistNoteText(engine: CanvasEngine, id: string, text: string): Promise<void> {
  if (!text || engine.syncMode === 'memory') return;
  const ws = engine.store.workspaceId;
  const floor = engine.store.floorId;
  for (let attempt = 0; attempt < 10; attempt++) {
    try {
      const c = await noteRead(ws, floor, id);
      if (c.text) return;
      await noteWrite(ws, floor, id, text, c.revision);
      const n = engine.store.node(id);
      if (n && n.data.text === text) engine.updateNode(id, { data: { ...n.data, text: null } });
      return;
    } catch {
      await new Promise((r) => setTimeout(r, 200));
    }
  }
}

export function cordToFile(engine: CanvasEngine, from: string, target: string): boolean {
  const src = engine.store.node(from);
  const file = engine.store.node(target);
  if (!src || !file || !isTerminalKind(src.kind)) return false;
  if (file.kind !== 'file' && file.kind !== 'image') return false;
  const path = typeof file.data.path === 'string' ? file.data.path : '';
  if (!path) return false;
  noteWithFiles(engine, fileMarkdown(path), file, from, file.title || baseName(path));
  return true;
}

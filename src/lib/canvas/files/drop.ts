import { invoke } from '@tauri-apps/api/core';
import { getCurrentWebview } from '@tauri-apps/api/webview';
import type { CanvasEngine } from '../engine.svelte';
import { GRID } from '../model/types';
import { isTerminalKind } from '../model/types';
import { hasFile, noteData, noteRead, noteWrite, stackIdOf } from '../notes/api';
import { baseName } from './api';
import { nodeForPath } from './open';
import { dropOnTerminal } from '../term/drop';
import { filesMarkdown, noteInsert, noteWithFiles } from './attach';

export function attachFileDrop(engine: CanvasEngine, root: HTMLElement): () => void {
  let unlisten: (() => void) | null = null;
  let disposed = false;
  try {
    getCurrentWebview()
      .onDragDropEvent((event) => {
        const p = event.payload as { type: string; paths?: string[]; position?: { x: number; y: number } };
        if (p.type !== 'drop' || !p.paths?.length || !p.position) return;
        if (!root.isConnected || root.offsetParent === null) return;
        const dpr = window.devicePixelRatio || 1;
        const cx = p.position.x / dpr;
        const cy = p.position.y / dpr;
        const r = root.getBoundingClientRect();
        if (cx < r.left || cx > r.right || cy < r.top || cy > r.bottom) return;
        const w = engine.camera.screenToWorld(cx - r.left, cy - r.top);
        if (dropOnTerminal(engine, p.paths, w)) return;
        if (dropOnNote(engine, p.paths, w)) return;
        if (dropOnCord(engine, p.paths)) return;
        void dropPaths(engine, p.paths, w);
      })
      .then((fn) => {
        if (disposed) fn();
        else unlisten = fn;
      })
      .catch(() => {});
  } catch {
  }
  return () => {
    disposed = true;
    unlisten?.();
  };
}

export async function dropPaths(engine: CanvasEngine, paths: string[], at: { x: number; y: number }): Promise<void> {
  const dirs = await invoke<boolean[]>('canvas_files_kind', { paths }).catch(() => paths.map(() => false));
  const ids: string[] = [];
  paths.forEach((path, i) => {
    const off = i * GRID * 2;
    const node = nodeForPath(engine, path, !!dirs[i], { x: at.x + off, y: at.y + off });
    ids.push(node.id);
  });
  if (ids.length) engine.selection.set(ids);
}

async function appendToNote(engine: CanvasEngine, nodeId: string, add: string): Promise<void> {
  const n = engine.store.node(nodeId);
  if (!n) return;
  const join = (cur: string) => (cur && !cur.endsWith('\n') ? `${cur}\n${add}` : `${cur}${add}`);
  if (hasFile(n) && engine.syncMode !== 'memory') {
    const { workspaceId, floorId } = engine.store;
    const c = await noteRead(workspaceId, floorId, nodeId);
    await noteWrite(workspaceId, floorId, nodeId, join(c.text), c.revision);
    return;
  }
  engine.updateNode(nodeId, { data: { ...n.data, text: join(String(n.data.text ?? '')) } });
}

function dropOnNote(engine: CanvasEngine, paths: string[], at: { x: number; y: number }): boolean {
  const hit = engine.store.nodes
    .filter((n) => n.kind === 'note' && !stackIdOf(n) && at.x >= n.x && at.x <= n.x + n.w && at.y >= n.y && at.y <= n.y + n.h)
    .sort((a, b) => (b.z ?? 0) - (a.z ?? 0))[0];
  if (!hit || !paths.length || noteData(hit).isContentLocked) return false;
  void filesMarkdown(engine.store.workspaceId, paths).then((parts) => {
    const text = parts.join('\n');
    const insert = noteInsert(hit.id);
    if (insert) insert(text);
    else return appendToNote(engine, hit.id, text);
  }).catch(() => {});
  return true;
}

function dropOnCord(engine: CanvasEngine, paths: string[]): boolean {
  const menu = engine.spawnMenu;
  const from = menu?.connectFrom ? engine.store.node(menu.connectFrom) : undefined;
  if (!menu || !from || !paths.length) return false;
  engine.spawnMenu = null;
  const at = { x: menu.wx, y: menu.wy };
  if (isTerminalKind(from.kind)) {
    void filesMarkdown(engine.store.workspaceId, paths).then((parts) => {
      const node = noteWithFiles(engine, parts.join('\n'), at, from.id, paths.length === 1 ? baseName(paths[0]) : `${baseName(paths[0])} +${paths.length - 1}`);
      engine.selection.set([node.id]);
    });
    return true;
  }
  void invoke<boolean[]>('canvas_files_kind', { paths })
    .catch(() => paths.map(() => false))
    .then((dirs) => {
      const ids = paths.map((path, i) => {
        const off = i * GRID * 2;
        const node = nodeForPath(engine, path, !!dirs[i], { x: at.x + off, y: at.y + off });
        engine.connect(from.id, node.id);
        return node.id;
      });
      if (ids.length) engine.selection.set(ids);
    });
  return true;
}

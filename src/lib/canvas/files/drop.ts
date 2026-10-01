import { invoke } from '@tauri-apps/api/core';
import { getCurrentWebview } from '@tauri-apps/api/webview';
import type { CanvasEngine } from '../engine.svelte';
import { GRID } from '../model/types';
import { nodeForPath } from './open';
import { dropOnTerminal } from '../term/drop';

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

async function dropPaths(engine: CanvasEngine, paths: string[], at: { x: number; y: number }): Promise<void> {
  const dirs = await invoke<boolean[]>('canvas_files_kind', { paths }).catch(() => paths.map(() => false));
  const ids: string[] = [];
  paths.forEach((path, i) => {
    const off = i * GRID * 2;
    const node = nodeForPath(engine, path, !!dirs[i], { x: at.x + off, y: at.y + off });
    ids.push(node.id);
  });
  if (ids.length) engine.selection.set(ids);
}

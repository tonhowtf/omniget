import type { CanvasEngine } from '../../engine.svelte';
import { nearestFreeSpot } from '../../interaction/snap';
import type { CanvasNode } from '../../model/types';
import { workspaces } from '../../palette/workspaces.svelte';
import { findEditor, nodeForPath, openEditorBeside } from '../../files/open';
import { baseName, relativeTo } from '../../files/api';
import { isNoteFile } from '../api';

function besideSpot(engine: CanvasEngine, anchor: CanvasNode | null, w: number, h: number) {
  const want = anchor ? { x: anchor.x + anchor.w + 40, y: anchor.y } : engine.camera.center();
  return nearestFreeSpot(want.x, want.y, w, h, engine.store.nodes);
}

export function workspaceDir(engine: CanvasEngine): string | null {
  return (
    workspaces.byId(engine.store.workspaceId)?.workingDirectory ||
    engine.sync.workspaces.find((w) => w.id === engine.store.workspaceId)?.workingDirectory ||
    null
  );
}

export function dirOf(abs: string): string {
  const i = Math.max(abs.lastIndexOf('/'), abs.lastIndexOf('\\'));
  return i <= 0 ? abs.slice(0, i + 1) || '/' : abs.slice(0, i);
}

export function openPathOnCanvas(engine: CanvasEngine, anchor: CanvasNode | null, abs: string, isDir: boolean, line?: number): CanvasNode {
  if (isDir) {
    const spot = besideSpot(engine, anchor, 320, 420);
    const n = engine.spawn('filetree', null, { rect: { x: spot.x, y: spot.y, w: 320, h: 420 }, title: baseName(abs), data: { rootPath: abs } });
    engine.reveal(n.id);
    return n;
  }
  const existing = findEditor(engine, abs);
  if (existing) {
    engine.selection.set([existing.id]);
    engine.reveal(existing.id);
    return existing;
  }
  if (isNoteFile(abs)) {
    const spot = besideSpot(engine, anchor, 360, 300);
    const n = nodeForPath(engine, abs, false, spot);
    engine.reveal(n.id);
    return n;
  }
  const wd = workspaceDir(engine);
  const rel = wd ? relativeTo(wd, abs) : null;
  if (wd && rel) return openEditorBeside(engine, anchor, wd, rel, line);
  const dir = dirOf(abs);
  const tree =
    engine.store.nodes.find((n) => n.kind === 'filetree' && n.data.rootPath === dir) ??
    (() => {
      const spot = besideSpot(engine, anchor, 300, 380);
      return engine.spawn('filetree', null, { rect: { x: spot.x, y: spot.y, w: 300, h: 380 }, title: baseName(dir), data: { rootPath: dir } });
    })();
  return openEditorBeside(engine, tree, dir, baseName(abs), line);
}

import { invoke } from '@tauri-apps/api/core';
import type { CanvasEngine } from '../engine.svelte';
import { isTerminalKind, type CanvasNode } from '../model/types';
import { stackIdOf } from '../notes/api';
import { bringToFrontOps } from '../notes/stack';

const LAST_KEY = 'omniget.canvas.lastWorkspace';

export function lastWorkspaceId(): string | undefined {
  try {
    return localStorage.getItem(LAST_KEY) ?? undefined;
  } catch {
    return undefined;
  }
}

let switching: Promise<void> | null = null;

export async function switchTo(engine: CanvasEngine, workspaceId: string, floorId?: string): Promise<void> {
  if (engine.store.workspaceId === workspaceId && (!floorId || engine.store.floorId === floorId)) return;
  if (switching) await switching;
  const run = async () => {
    engine.sync.flush();
    await invoke('canvas_flush').catch(() => {});
    engine.selection.focus(null);
    engine.selection.clear();
    await engine.sync.open(workspaceId, floorId);
    engine.selection.setScope(engine.store.workspaceId, engine.store.floorId);
    const cam = engine.store.camera;
    if (cam) engine.camera.set(cam);
    else engine.fitAll(false);
    engine.invalidate();
    try {
      localStorage.setItem(LAST_KEY, engine.store.workspaceId);
    } catch {
    }
    if (floorId) {
      void invoke('canvas_workspace_update', {
        workspaceId: engine.store.workspaceId,
        patch: { activeFloorId: engine.store.floorId },
      }).catch(() => {});
    }
  };
  switching = run().finally(() => (switching = null));
  await switching;
}

export function revealNode(engine: CanvasEngine, nodeId: string, opts: { focus?: boolean } = {}): boolean {
  let node: CanvasNode | undefined = engine.store.node(nodeId);
  if (!node) return false;
  const target = node;
  const sid = stackIdOf(node);
  if (sid) {
    const stack = engine.store.node(sid);
    if (stack) {
      const ops = bringToFrontOps(stack, node.id);
      if (ops.length) engine.commit(ops, { history: false });
      node = stack;
    }
  }
  engine.selection.set([node.id]);
  engine.flyToRect(node, Math.max(1, engine.camera.zoom));
  const focus = opts.focus ?? takesKeyboard(target.kind);
  if (focus) engine.selection.focus(node.id);
  engine.selection.editing = target.kind === 'note' && focus ? target.id : null;
  engine.invalidate();
  return true;
}

function takesKeyboard(kind: string): boolean {
  return isTerminalKind(kind) || kind === 'portal' || kind === 'note';
}

export async function goTo(
  engine: CanvasEngine,
  target: { workspaceId: string; floorId: string; nodeId?: string },
  opts: { focus?: boolean } = {},
): Promise<void> {
  if (target.workspaceId !== engine.store.workspaceId || target.floorId !== engine.store.floorId) {
    await switchTo(engine, target.workspaceId, target.floorId);
  }
  if (target.nodeId) {
    const id = target.nodeId;
    await new Promise<void>((resolve) => {
      let tries = 0;
      const step = () => {
        if (revealNode(engine, id, opts) || ++tries > 30) resolve();
        else requestAnimationFrame(step);
      };
      requestAnimationFrame(step);
    });
  }
}

import { invoke } from '@tauri-apps/api/core';
import type { CanvasEngine } from './engine.svelte';
import { showToast } from '$lib/stores/toast-store.svelte';

interface PasteResult {
  source: string;
  nodeIds: string[];
  drawingIds?: string[];
}

function selected(engine: CanvasEngine): { nodeIds: string[]; drawingIds: string[] } {
  const ids = engine.selection.list();
  return {
    nodeIds: ids.filter((id) => !!engine.store.node(id)),
    drawingIds: ids.filter((id) => !!engine.store.drawing(id)),
  };
}

export async function copySelection(engine: CanvasEngine, cut = false): Promise<boolean> {
  const sel = selected(engine);
  if (!sel.nodeIds.length && !sel.drawingIds.length) return false;
  if (engine.syncMode !== 'rust') return false;
  engine.sync.flush();
  try {
    await invoke('canvas_clipboard_copy', {
      workspaceId: engine.store.workspaceId,
      floorId: engine.store.floorId,
      nodeIds: sel.nodeIds,
      drawingIds: sel.drawingIds,
      cut,
    });
    if (cut) engine.selection.clear();
    return true;
  } catch (e) {
    showToast('error', String(e));
    return false;
  }
}

export async function pasteClipboard(engine: CanvasEngine, at?: { x: number; y: number }): Promise<string[]> {
  if (engine.syncMode !== 'rust') return [];
  engine.sync.flush();
  try {
    const r = await invoke<PasteResult>('canvas_clipboard_paste', {
      workspaceId: engine.store.workspaceId,
      floorId: engine.store.floorId,
      at: at ?? null,
      center: at ? null : engine.camera.center(),
    });
    const ids = [...(r.nodeIds ?? []), ...(r.drawingIds ?? [])];
    const t0 = performance.now();
    const pick = () => {
      if (ids.every((id) => engine.exists(id)) || performance.now() - t0 > 1500) {
        const present = ids.filter((id) => engine.exists(id));
        if (present.length) engine.selection.set(present);
        engine.invalidate();
      } else {
        setTimeout(pick, 30);
      }
    };
    pick();
    return ids;
  } catch (e) {
    const msg = String(e);
    if (!msg.includes('CLIPBOARD_EMPTY')) showToast('error', msg);
    return [];
  }
}

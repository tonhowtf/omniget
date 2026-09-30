// Canvas clipboard (Maestri M-B21, M-B30, M-B37): ⌘C / ⌘X / ⌘V and the menu
// entries. The clip is built and stamped in Rust (`canvas_clipboard_*`), so it
// travels across workspaces and floors, survives a relaunch through the OS
// clipboard, and the MCP tool `canvas_clipboard` uses the same path. A paste of
// foreign content (link, SVG, file paths, image, text) becomes nodes.

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

export function canCopy(engine: CanvasEngine): boolean {
  const s = selected(engine);
  return s.nodeIds.length + s.drawingIds.length > 0;
}

/** Copies (or cuts) the selection. Memory mode (no Rust store) does nothing. */
export async function copySelection(engine: CanvasEngine, cut = false): Promise<boolean> {
  const sel = selected(engine);
  if (!sel.nodeIds.length && !sel.drawingIds.length) return false;
  if (engine.syncMode !== 'rust') return false;
  // Pending local edits first, so the clip has the frames the user sees.
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

/**
 * Pastes at `at` (top-left, world; right-click Paste) or at the view centre
 * with the cascade of repeated pastes (⌘V). Selects what arrived.
 */
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
    // The nodes come back through `canvas://patch`; select once they are in.
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

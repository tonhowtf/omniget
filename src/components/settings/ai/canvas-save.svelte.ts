import { canvasPrefs, replaceCanvasPrefs, type CanvasPrefs } from "$lib/canvas/prefs.svelte";
import { showToast } from "$lib/stores/toast-store.svelte";

export const canvasSaving = $state({ busy: false });

export async function saveCanvas(mutate: (c: CanvasPrefs) => void): Promise<boolean> {
  if (canvasSaving.busy) return false;
  canvasSaving.busy = true;
  try {
    const next = structuredClone($state.snapshot(canvasPrefs())) as CanvasPrefs;
    mutate(next);
    await replaceCanvasPrefs(next);
    return true;
  } catch (error) {
    showToast("error", String(error));
    return false;
  } finally {
    canvasSaving.busy = false;
  }
}

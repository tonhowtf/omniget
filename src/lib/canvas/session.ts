// Boot guard of the canvas window (study 100, M-A13 and M-A19): the window
// reopens on the last workspace used (kept in the data folder, so it follows
// the profile), and a workspace that failed to open on two boots in a row is
// skipped once, with a warning.

import { invoke } from '@tauri-apps/api/core';
import { get } from 'svelte/store';
import { t } from '$lib/i18n';
import { showToast } from '$lib/stores/toast-store.svelte';

interface Boot {
  workspaceId?: string | null;
  safeMode?: boolean;
  crashedWorkspaceId?: string;
  crashedName?: string | null;
}

/** Time a workspace must stay open for its boot to count as fine. */
const OK_AFTER_MS = 4000;

/** Workspace the window should open on (`preferred` is the page's guess). */
export async function bootWorkspace(preferred?: string): Promise<string | undefined> {
  let boot: Boot;
  try {
    boot = await invoke<Boot>('canvas_session_boot', { preferred: preferred ?? null });
  } catch {
    return preferred;
  }
  if (boot.safeMode) {
    const name = boot.crashedName ?? boot.crashedWorkspaceId ?? '';
    showToast(
      'info',
      get(t)('canvas.sidebar.safeLaunch', {
        default: '“{{name}}” failed to open twice, so another workspace opened. Open it from the sidebar when ready.',
        name,
      }),
      12000,
    );
  }
  return boot.workspaceId ?? preferred;
}

let okTimer: ReturnType<typeof setTimeout> | null = null;

/** Records the workspace on screen; after a few seconds its boot counts as fine. */
export function markWorkspaceOpen(workspaceId: string): void {
  void invoke('canvas_session_mark', { workspaceId, ok: false }).catch(() => {});
  if (okTimer) clearTimeout(okTimer);
  okTimer = setTimeout(() => {
    okTimer = null;
    void invoke('canvas_session_mark', { workspaceId, ok: true }).catch(() => {});
  }, OK_AFTER_MS);
}

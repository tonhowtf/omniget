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

const OK_AFTER_MS = 4000;

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

export function markWorkspaceOpen(workspaceId: string): void {
  void invoke('canvas_session_mark', { workspaceId, ok: false }).catch(() => {});
  if (okTimer) clearTimeout(okTimer);
  okTimer = setTimeout(() => {
    okTimer = null;
    void invoke('canvas_session_mark', { workspaceId, ok: true }).catch(() => {});
  }, OK_AFTER_MS);
}

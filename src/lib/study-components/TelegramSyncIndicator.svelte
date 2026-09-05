<script lang="ts">
  import { listen, type UnlistenFn } from "@tauri-apps/api/event";
  import { onMount, onDestroy } from "svelte";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import {
    telegramSyncState,
    telegramSyncNow,
    type TelegramSyncState,
  } from "$lib/study-telegram-bridge";

  let syncSnap = $state<TelegramSyncState | null>(null);
  let busy = $state(false);
  let now = $state(Date.now() / 1000);
  let unlisten: UnlistenFn | null = null;
  let tick: ReturnType<typeof setInterval> | null = null;

  onMount(async () => {
    await refresh();
    unlisten = await listen<{ stage: string; state: TelegramSyncState }>(
      "telegram:sync:state",
      (ev) => {
        syncSnap = ev.payload.state;
      },
    );
    tick = setInterval(() => {
      now = Date.now() / 1000;
    }, 30_000);
  });

  onDestroy(() => {
    unlisten?.();
    if (tick) clearInterval(tick);
  });

  async function refresh() {
    try {
      syncSnap = await telegramSyncState();
    } catch {
      syncSnap = null;
    }
  }

  async function syncNow() {
    if (busy) return;
    busy = true;
    try {
      const r = await telegramSyncNow();
      showToast("info", `${r.updated} canais atualizados`);
      await refresh();
    } catch (e: any) {
      showToast("error", typeof e === "string" ? e : (e?.message ?? "Erro ao sincronizar"));
    } finally {
      busy = false;
    }
  }

  function ago(ts: number, current: number): string {
    if (!ts) return "nunca";
    const delta = Math.max(0, current - ts);
    if (delta < 60) return "agora";
    const min = Math.floor(delta / 60);
    if (min < 60) return `há ${min} min`;
    const hr = Math.floor(min / 60);
    if (hr < 24) return `há ${hr}h`;
    return `há ${Math.floor(hr / 24)}d`;
  }

  let label = $derived(
    syncSnap?.is_syncing
      ? "Sincronizando…"
      : syncSnap?.last_success_at
      ? `Sync ${ago(syncSnap.last_success_at, now)}`
      : syncSnap?.enabled
      ? "Sync pendente"
      : "Sync off",
  );

  let dotClass = $derived(
    !syncSnap
      ? "dot-neutral"
      : syncSnap.is_syncing || busy
      ? "dot-active"
      : !syncSnap.enabled
      ? "dot-off"
      : syncSnap.last_success_at && now - syncSnap.last_success_at < syncSnap.interval_min * 60 * 2
      ? "dot-good"
      : "dot-stale",
  );
</script>

<button
  type="button"
  class="sync-pill"
  class:syncing={syncSnap?.is_syncing || busy}
  onclick={syncNow}
  disabled={busy}
  title={syncSnap?.enabled
    ? `Sincronização automática a cada ${syncSnap.interval_min ?? 30} min — clique para forçar agora`
    : "Sincronização desativada — clique para forçar agora"}
  aria-label="Status de sincronização"
>
  <span class="status-dot {dotClass}"></span>
  <span class="sync-label">{label}</span>
</button>

<style>
  .sync-pill {
    display: inline-flex;
    align-items: center;
    gap: 8px;
    padding: 5px 12px;
    background: var(--button-elevated);
    border: 1px solid transparent;
    border-radius: 100px;
    color: var(--gray);
    font-family: inherit;
    font-size: 11.5px;
    font-weight: 500;
    cursor: pointer;
    transition: background 150ms, color 150ms, border-color 150ms;
  }

  .sync-pill:hover:not(:disabled) {
    color: var(--secondary);
    background: var(--button);
    border-color: var(--input-border);
  }

  .sync-pill:disabled {
    opacity: 0.6;
    cursor: not-allowed;
  }

  .sync-pill.syncing {
    border-color: color-mix(in oklab, var(--blue) 40%, transparent);
  }

  .status-dot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    flex-shrink: 0;
  }

  .dot-good {
    background: var(--green);
    box-shadow: 0 0 0 3px color-mix(in oklab, var(--green) 18%, transparent);
  }

  .dot-stale {
    background: var(--warning);
  }

  .dot-off {
    background: var(--gray);
  }

  .dot-neutral {
    background: var(--input-border);
  }

  .dot-active {
    background: var(--blue);
    animation: pulse 1.4s ease-in-out infinite;
  }

  @keyframes pulse {
    0%, 100% { opacity: 1; transform: scale(1); }
    50% { opacity: 0.6; transform: scale(0.85); }
  }

  .sync-label {
    white-space: nowrap;
  }
</style>

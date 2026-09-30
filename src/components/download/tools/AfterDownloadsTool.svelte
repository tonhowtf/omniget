<script lang="ts">
  import { onDestroy, onMount } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";

  // Quit, sleep or shut down once the queue is empty. Idea from Persepolis
  // ("shut down after download") and Varia (with a cancel window).
  type Action = "quit" | "sleep" | "shutdown";
  type State = { action: Action | null; pending: number; saw_work: boolean; countdown: number | null };

  const ACTIONS: ("none" | Action)[] = ["none", "quit", "sleep", "shutdown"];

  let state = $state<State>({ action: null, pending: 0, saw_work: false, countdown: null });
  let timer: ReturnType<typeof setInterval> | undefined;

  async function refresh() {
    try {
      state = await invoke<State>("after_downloads_get");
    } catch {
      // The panel keeps the last state; the next tick tries again.
    }
  }

  async function choose(value: "none" | Action) {
    try {
      state = await invoke<State>("after_downloads_set", { action: value === "none" ? null : value });
      if (value !== "none") showToast("success", $t(`downloads.tools.after_downloads.armed_${value}`));
    } catch {
      showToast("error", $t("common.error"));
    }
  }

  onMount(() => {
    refresh();
    timer = setInterval(refresh, 1000);
  });
  onDestroy(() => clearInterval(timer));

  const status = $derived.by(() => {
    if (!state.action) return $t("downloads.tools.after_downloads.status_off");
    if (state.countdown != null) return $t("downloads.tools.after_downloads.status_countdown", { seconds: state.countdown });
    if (state.pending > 0) return $t("downloads.tools.after_downloads.status_waiting", { count: state.pending });
    return $t("downloads.tools.after_downloads.status_idle");
  });
</script>

<div class="ad" data-agent-readiness="ready" data-armed={state.action ?? "none"}>
  <p class="lead">{$t("downloads.tools.after_downloads.lead")}</p>

  <div class="options" role="radiogroup" aria-label={$t("downloads.tools.after_downloads.title")}>
    {#each ACTIONS as a (a)}
      <button
        class="chip"
        class:on={(state.action ?? "none") === a}
        role="radio"
        aria-checked={(state.action ?? "none") === a}
        onclick={() => choose(a)}
      >
        {$t(`downloads.tools.after_downloads.action_${a}`)}
      </button>
    {/each}
  </div>

  <p class="status" class:alert={state.countdown != null} aria-live="polite">{status}</p>

  {#if state.countdown != null}
    <div class="actions">
      <button class="button" onclick={() => choose("none")}>{$t("downloads.tools.after_downloads.cancel")}</button>
    </div>
  {/if}

  <p class="dim small">{$t("downloads.tools.after_downloads.note")}</p>
</div>

<style>
  .ad {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
  }
  .lead,
  .dim {
    margin: 0;
    color: var(--text-dim);
    font-size: var(--text-sm);
  }
  .small {
    font-size: var(--text-xs, 11px);
  }
  .options {
    display: flex;
    gap: var(--space-1);
    flex-wrap: wrap;
  }
  .chip {
    padding: var(--space-1) var(--space-3);
    border: none;
    border-radius: 999px;
    background: var(--surface);
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
    color: var(--text);
    font-size: var(--text-sm);
    cursor: pointer;
  }
  .chip.on {
    background: var(--accent);
    color: var(--on-accent, #fff);
    box-shadow: none;
  }
  .chip:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: 1px;
  }
  .status {
    margin: 0;
    font-size: var(--text-sm);
    color: var(--text);
  }
  .status.alert {
    font-weight: 600;
  }
  .actions {
    display: flex;
  }
</style>

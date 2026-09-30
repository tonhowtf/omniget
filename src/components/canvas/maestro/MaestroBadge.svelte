<script lang="ts">
  // Maestro Mode on a terminal node: a baton badge in the node header that
  // toggles the flag (`data.isManager`). A Maestro can recruit, connect,
  // swap, dismiss, schedule routines and create workspaces and floors; the
  // server checks the flag on every call.
  import { t } from "$lib/i18n";
  import { errorText, setMaestro } from "$lib/canvas/floors/api";

  interface Props {
    workspaceId: string;
    floorId: string;
    nodeId: string;
    on: boolean;
    /** Only show when on (read-only badge), e.g. on cards at low zoom. */
    compact?: boolean;
  }
  let { workspaceId, floorId, nodeId, on, compact = false }: Props = $props();

  let busy = $state(false);
  let error = $state("");

  async function toggle(e: MouseEvent) {
    e.stopPropagation();
    if (busy) return;
    busy = true;
    error = "";
    try {
      await setMaestro(workspaceId, floorId, nodeId, !on);
    } catch (err) {
      error = errorText(err);
    } finally {
      busy = false;
    }
  }
</script>

{#if !compact || on}
  <button
    type="button"
    class="maestro"
    class:on
    disabled={busy || compact}
    aria-pressed={on}
    title={error ||
      (on
        ? $t("canvas.maestro.on", { default: "Maestro: can recruit, connect and create floors. Click to turn off." })
        : $t("canvas.maestro.off", { default: "Turn on Maestro Mode" }))}
    onpointerdown={(e) => e.stopPropagation()}
    onclick={toggle}
  >
    <svg viewBox="0 0 16 16" width="12" height="12" aria-hidden="true">
      <path d="M2.5 13.5 11 5" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" />
      <circle cx="12.2" cy="3.8" r="1.8" fill="currentColor" />
    </svg>
    {#if on}<span>{$t("canvas.maestro.label", { default: "Maestro" })}</span>{/if}
  </button>
{/if}

<style>
  .maestro {
    all: unset;
    display: inline-flex;
    align-items: center;
    gap: 4px;
    height: 18px;
    padding: 0 6px;
    border-radius: 999px;
    font-size: 11px;
    font-weight: 600;
    color: var(--text-muted);
    border: 1px solid transparent;
    cursor: pointer;
  }
  .maestro:hover:not(:disabled) {
    background: var(--hover);
  }
  .maestro.on {
    color: #b8860b;
    border-color: color-mix(in srgb, #d4a017 60%, transparent);
    background: color-mix(in srgb, #d4a017 14%, transparent);
  }
  .maestro:disabled {
    cursor: default;
  }
  .maestro:focus-visible {
    outline: 2px solid var(--accent);
  }
</style>

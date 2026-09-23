<script lang="ts">
  /** Last requests on the remote listener (API and sockets; the PWA shell is left out). */
  import { invoke } from "@tauri-apps/api/core";
  import { t } from "$lib/i18n";
  import type { AccessEntry } from "./types";

  let { entries, onchange }: { entries: AccessEntry[]; onchange: () => void } = $props();

  async function clear() {
    try {
      await invoke("remote_access_log_clear");
    } finally {
      onchange();
    }
  }
</script>

<div class="group">
  <div class="group-label head">
    <span>{$t("llm.central.remote.log")}</span>
    {#if entries.length}
      <button type="button" class="link" onclick={clear}>{$t("llm.central.remote.log_clear")}</button>
    {/if}
  </div>
  {#if entries.length === 0}
    <div class="group-row">
      <div class="group-row-content"><div class="group-row-sub">{$t("llm.central.remote.log_empty")}</div></div>
    </div>
  {:else}
    <div class="log">
      {#each entries as e, i (i)}
        <div class="line" class:bad={e.status >= 400}>
          <span class="at">{new Date(e.at).toLocaleTimeString()}</span>
          <span class="st">{e.status}</span>
          <span class="m">{e.method}</span>
          <span class="p">{e.path}</span>
          <span class="who">{e.device ?? "—"} · {e.ip}</span>
        </div>
      {/each}
    </div>
  {/if}
</div>

<style>
  .head {
    display: flex;
    justify-content: space-between;
    align-items: center;
  }
  .link {
    border: 0;
    background: none;
    color: var(--accent);
    cursor: pointer;
    font: inherit;
    text-transform: none;
  }
  .log {
    max-height: 320px;
    overflow: auto;
    border: 1px solid var(--separator);
    border-radius: 10px;
    font-family: var(--font-mono, ui-monospace, monospace);
    font-size: 12px;
  }
  .line {
    display: grid;
    grid-template-columns: 80px 40px 48px minmax(0, 1fr) auto;
    gap: 8px;
    padding: 4px 10px;
    border-bottom: 1px solid var(--separator);
  }
  .line:last-child {
    border-bottom: 0;
  }
  .line.bad .st {
    color: var(--danger, #d33b3b);
  }
  .p {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .at,
  .who {
    color: var(--text-muted);
  }
</style>

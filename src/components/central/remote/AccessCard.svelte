<script lang="ts">
  /**
   * The opt-in: turns the second listener (only `/remote/*`, auth per device)
   * on or off on the interface the user picks. The local bridge stays on
   * 127.0.0.1 either way.
   */
  import { invoke } from "@tauri-apps/api/core";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import { errText, type RemoteStatus } from "./types";

  let { status, onchange }: { status: RemoteStatus; onchange: (s: RemoteStatus) => void } = $props();

  let chosen = $state("");
  let port = $state<number | null>(null);
  let busy = $state(false);

  $effect(() => {
    if (!chosen) chosen = status.bindIp || status.interfaces[0]?.ip || "127.0.0.1";
    if (port == null) port = status.port;
  });

  function kindLabel(kind: string): string {
    return $t(`llm.central.remote.iface.${kind}`) as string;
  }

  async function start() {
    busy = true;
    try {
      onchange(await invoke<RemoteStatus>("remote_start", { bindIp: chosen, port: port || null }));
      showToast("success", $t("llm.central.remote.started") as string);
    } catch (e) {
      showToast("error", errText(e));
    } finally {
      busy = false;
    }
  }

  async function stop() {
    busy = true;
    try {
      onchange(await invoke<RemoteStatus>("remote_stop"));
    } catch (e) {
      showToast("error", errText(e));
    } finally {
      busy = false;
    }
  }
</script>

<div class="group">
  <div class="group-label">{$t("llm.central.remote.access")}</div>
  <div class="group-row">
    <div class="group-row-content">
      <div class="group-row-title">
        {#if status.running}
          <span class="live"></span>{$t("llm.central.remote.on_at", { addr: status.bound ?? "" })}
        {:else}
          {$t("llm.central.remote.off")}
        {/if}
      </div>
      <div class="group-row-sub">{$t("llm.central.remote.access_hint")}</div>
      {#if status.lastError}<div class="group-row-sub err">{errText(status.lastError)}</div>{/if}
    </div>
    <div class="group-row-trailing">
      {#if status.running}
        <button type="button" class="button" disabled={busy} onclick={stop}>{$t("llm.central.remote.turn_off")}</button>
      {:else}
        <button type="button" class="button primary" disabled={busy || !chosen} onclick={start}>{$t("llm.central.remote.turn_on")}</button>
      {/if}
    </div>
  </div>
  <div class="group-row ifaces">
    <div class="group-row-content">
      <div class="group-row-title">{$t("llm.central.remote.interface")}</div>
      <div class="choices" role="radiogroup">
        {#each status.interfaces as i (i.ip)}
          <label class="choice" class:on={chosen === i.ip}>
            <input type="radio" name="remote-iface" value={i.ip} bind:group={chosen} disabled={status.running} />
            <span class="ip">{i.ip}</span>
            <span class="tag">{kindLabel(i.kind)}</span>
          </label>
        {/each}
      </div>
      <div class="group-row-sub">{$t("llm.central.remote.interface_hint")}</div>
    </div>
  </div>
  <div class="group-row">
    <div class="group-row-content">
      <div class="group-row-title">{$t("llm.central.remote.port")}</div>
    </div>
    <div class="group-row-trailing">
      <input class="port" type="number" min="1" max="65535" bind:value={port} disabled={status.running} />
    </div>
  </div>
</div>

<style>
  .live {
    display: inline-block;
    width: 8px;
    height: 8px;
    border-radius: 50%;
    background: var(--success, #2f9e5b);
    margin-right: 8px;
    vertical-align: middle;
  }
  .err {
    color: var(--danger, #d33b3b);
  }
  .choices {
    display: flex;
    flex-wrap: wrap;
    gap: 8px;
    margin: 8px 0;
  }
  .choice {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 6px 10px;
    border: 1px solid var(--separator);
    border-radius: 10px;
    cursor: pointer;
  }
  .choice.on {
    border-color: var(--accent);
    background: var(--accent-soft);
  }
  .choice input {
    margin: 0;
  }
  .ip {
    font-family: var(--font-mono, ui-monospace, monospace);
    font-size: 13px;
  }
  .port {
    width: 96px;
  }
</style>

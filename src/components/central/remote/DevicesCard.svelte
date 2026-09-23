<script lang="ts">
  /** Paired devices: scope, last use, open sockets; revoke closes them now. */
  import { invoke } from "@tauri-apps/api/core";
  import { ask } from "@tauri-apps/plugin-dialog";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import { errText, relTime, type DeviceView } from "./types";

  let { devices, onchange }: { devices: DeviceView[]; onchange: () => void } = $props();

  async function revoke(d: DeviceView) {
    const ok = await ask($t("llm.central.remote.revoke_confirm", { name: d.name }) as string, {
      title: $t("llm.central.remote.revoke") as string,
      kind: "warning",
    });
    if (!ok) return;
    try {
      await invoke("remote_device_revoke", { id: d.id });
      onchange();
    } catch (e) {
      showToast("error", errText(e));
    }
  }

  async function rename(d: DeviceView, name: string) {
    if (!name.trim() || name.trim() === d.name) return;
    try {
      await invoke("remote_device_rename", { id: d.id, name: name.trim() });
      onchange();
    } catch (e) {
      showToast("error", errText(e));
    }
  }
</script>

<div class="group">
  <div class="group-label">{$t("llm.central.remote.devices")}</div>
  {#if devices.length === 0}
    <div class="group-row">
      <div class="group-row-content">
        <div class="group-row-sub">{$t("llm.central.remote.no_devices")}</div>
      </div>
    </div>
  {:else}
    {#each devices as d (d.id)}
      <div class="group-row">
        <div class="group-row-content">
          <div class="group-row-title">
            {#if d.connected > 0}<span class="live" title={$t("llm.central.remote.connected")}></span>{/if}
            <input
              class="name"
              value={d.name}
              aria-label={$t("llm.central.remote.device_name")}
              onchange={(e) => rename(d, (e.currentTarget as HTMLInputElement).value)}
            />
          </div>
          <div class="group-row-sub">
            {$t("llm.central.remote.device_meta", {
              created: new Date(d.createdAt).toLocaleDateString(),
              used: relTime(d.lastUsedAt),
              ip: d.lastIp ?? "—",
            })}
          </div>
        </div>
        <div class="group-row-trailing">
          <span class="tag">{d.scope === "drive" ? $t("llm.central.remote.scope_drive") : $t("llm.central.remote.scope_read")}</span>
          <button type="button" class="button" onclick={() => revoke(d)}>{$t("llm.central.remote.revoke")}</button>
        </div>
      </div>
    {/each}
  {/if}
</div>

<style>
  .live {
    display: inline-block;
    width: 8px;
    height: 8px;
    border-radius: 50%;
    background: var(--success, #2f9e5b);
    margin-right: 6px;
    vertical-align: middle;
  }
  .name {
    border: 0;
    background: transparent;
    font: inherit;
    color: inherit;
    padding: 0;
    width: min(260px, 100%);
  }
  .name:focus {
    outline: 1px solid var(--accent);
    border-radius: 4px;
  }
  .group-row-trailing {
    display: flex;
    gap: 8px;
    align-items: center;
  }
</style>

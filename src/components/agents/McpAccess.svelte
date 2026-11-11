<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { onMount } from "svelte";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";

  type Access = { enabled: boolean; open: boolean; askFirst: boolean; disabledScopes: string[]; disabledAreas: string[] };
  type Info = { access: Access; scopes: string[]; areas: { area: string; commands: number }[] };

  let info = $state<Info | null>(null);

  const pretty = (id: string) => id.replaceAll("_", " ").replace(/^./, (c) => c.toUpperCase());

  async function save(access: Access) {
    try {
      info = await invoke<Info>("tool_mcp_access_set", { access });
    } catch (error) {
      showToast("error", String(error));
    }
  }

  function flag(key: "open" | "askFirst", value: boolean) {
    if (info) void save({ ...info.access, [key]: value });
  }

  function toggle(key: "disabledScopes" | "disabledAreas", id: string, on: boolean) {
    if (!info) return;
    const rest = info.access[key].filter((item) => item !== id);
    void save({ ...info.access, [key]: on ? rest : [...rest, id] });
  }

  onMount(async () => {
    try {
      info = await invoke<Info>("tool_mcp_access_get");
    } catch {
      info = null;
    }
  });
</script>

{#if info}
  <div class="access" data-setting-key="connect.access.title">
    <h4>{$t("connect.access.title")}</h4>
    <label class="row">
      <input type="checkbox" checked={info.access.open} onchange={(e) => flag("open", e.currentTarget.checked)} />
      <span><strong>{$t("connect.access.open")}</strong><small>{$t("connect.access.open_hint")}</small></span>
    </label>
    <label class="row">
      <input type="checkbox" checked={info.access.askFirst} onchange={(e) => flag("askFirst", e.currentTarget.checked)} />
      <span><strong>{$t("connect.access.ask")}</strong><small>{$t("connect.access.ask_hint")}</small></span>
    </label>
    <details>
      <summary>{$t("connect.access.tools")}</summary>
      <div class="list">
        {#each info.scopes as scope (scope)}
          <label><input type="checkbox" checked={!info.access.disabledScopes.includes(scope)} onchange={(e) => toggle("disabledScopes", scope, e.currentTarget.checked)} /><bdi>{pretty(scope)}</bdi></label>
        {/each}
      </div>
    </details>
    <details>
      <summary>{$t("connect.access.areas")}</summary>
      <div class="list">
        {#each info.areas as item (item.area)}
          <label><input type="checkbox" checked={!info.access.disabledAreas.includes(item.area)} onchange={(e) => toggle("disabledAreas", item.area, e.currentTarget.checked)} /><bdi>{pretty(item.area)}</bdi> <small>{item.commands}</small></label>
        {/each}
      </div>
    </details>
  </div>
{/if}

<style>
  .access { display: flex; flex-direction: column; gap: var(--space-2); margin-block: var(--space-3); }
  h4 { margin: 0; font-size: 14px; }
  .row { display: flex; align-items: flex-start; gap: var(--space-2); cursor: pointer; }
  .row span { display: flex; flex-direction: column; }
  small { color: var(--text-muted, inherit); opacity: 0.75; }
  summary { cursor: pointer; }
  .list { display: grid; grid-template-columns: repeat(auto-fill, minmax(min(100%, 190px), 1fr)); gap: var(--space-1) var(--space-3); padding-block: var(--space-2); }
  .list label { display: flex; align-items: center; gap: var(--space-1); cursor: pointer; }
</style>

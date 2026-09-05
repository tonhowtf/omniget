<script lang="ts">
  /** Inicialização (estudos 10 e 25): o que abre com o sistema, com liga/desliga. */
  import { invoke } from "@tauri-apps/api/core";
  import { onMount } from "svelte";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import { errText, reveal } from "$lib/tools/rt";

  type Item = { id: string; name: string; command: string; source: string; scope: string; path: string; enabled: boolean; can_toggle: boolean };
  let items = $state<Item[]>([]);
  let busy = $state<string | null>(null);
  let loading = $state(true);
  let query = $state("");

  async function refresh() {
    loading = true;
    try { items = await invoke<Item[]>("tool_startup_list"); } catch (e) { showToast("error", errText(e)); } finally { loading = false; }
  }
  onMount(refresh);

  async function toggle(item: Item) {
    if (busy || !item.can_toggle) return;
    busy = item.id;
    try { items = await invoke<Item[]>("tool_startup_set", { item, enabled: !item.enabled }); }
    catch (e) { showToast("error", errText(e)); } finally { busy = null; }
  }
  let filtered = $derived(items.filter((i) => !query || `${i.name} ${i.command}`.toLowerCase().includes(query.toLowerCase())));
  let enabledCount = $derived(items.filter((i) => i.enabled).length);
</script>

<div class="tool">
  <section>
    <div class="group">
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{enabledCount} {$t("tools.startup.enabled_of")} {items.length}</div><div class="group-row-sub">{$t("tools.startup.hint")}</div></div>
        <div class="group-row-trailing btn-row"><input class="input" type="search" placeholder={$t("tools.hub.search_placeholder")} bind:value={query} style:width="12em" /><button class="btn btn-secondary btn-sm" type="button" disabled={loading} onclick={refresh}>{$t("tools.common.refresh")}</button></div>
      </div>
    </div>
  </section>
  <section>
    <div class="group">
      {#if loading && !items.length}
        <div class="group-row"><div class="group-row-sub">…</div></div>
      {:else if !filtered.length}
        <div class="group-row"><div class="group-row-sub">{$t("tools.startup.none")}</div></div>
      {/if}
      {#each filtered as item (item.id)}
        <div class="group-row" class:off={!item.enabled}>
          <div class="group-row-content">
            <div class="group-row-title">{item.name} <span class="tag">{$t(`tools.startup.source_${item.source.replace(/-/g, "_")}`)}</span>{#if item.scope === "system"}<span class="tag tag-warning">{$t("tools.startup.system")}</span>{/if}</div>
            <div class="group-row-sub mono" title={item.path}>{item.command || item.path}</div>
          </div>
          <div class="group-row-trailing btn-row">
            {#if item.path && item.source !== "systemd-user"}<button class="btn btn-ghost btn-sm" type="button" onclick={() => reveal(item.path)}>{$t("tools.common.reveal")}</button>{/if}
            <button class="toggle" class:on={item.enabled} type="button" role="switch" aria-checked={item.enabled} aria-label={item.name} disabled={!item.can_toggle || busy !== null} title={item.can_toggle ? "" : $t("tools.startup.locked")} onclick={() => toggle(item)}><span class="toggle-knob"></span></button>
          </div>
        </div>
      {/each}
    </div>
  </section>
</div>

<style>
  .tool { display: flex; flex-direction: column; gap: var(--space-5); }
  .mono { font-family: var(--font-mono); font-size: var(--text-xs); overflow-wrap: anywhere; }
  .off .group-row-title { color: var(--text-muted); }
</style>

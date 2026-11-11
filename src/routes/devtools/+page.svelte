<script lang="ts">
  import { page } from "$app/state";
  import { t } from "$lib/i18n";
  import DevShell from "$components/devtools/DevShell.svelte";
  import UtilPanel from "$components/devtools/UtilPanel.svelte";
  import PortsPanel from "$components/devtools/PortsPanel.svelte";

  type Tab = "utilities" | "ports";
  const TABS: Tab[] = ["utilities", "ports"];

  let tab = $state<Tab>(page.url.searchParams.get("tab") === "ports" ? "ports" : "utilities");
</script>

<DevShell icon="devtools" title={$t("devtools.title") as string} subtitle={$t("devtools.subtitle") as string}>
  <div class="segmented dv-tabs" role="tablist" aria-label={$t("devtools.title") as string}>
    {#each TABS as id (id)}
      <button type="button" role="tab" id="dv-tab-{id}" class="segmented-btn" class:active={tab === id} aria-selected={tab === id} aria-controls="dv-panel-{id}" onclick={() => (tab = id)}>
        {$t(`devtools.tab_${id}`)}
      </button>
    {/each}
  </div>

  <div class="dv-stack" id="dv-panel-utilities" role="tabpanel" aria-labelledby="dv-tab-utilities" hidden={tab !== "utilities"}>
    <UtilPanel />
  </div>
  <div class="dv-stack" id="dv-panel-ports" role="tabpanel" aria-labelledby="dv-tab-ports" hidden={tab !== "ports"}>
    {#if tab === "ports"}<PortsPanel />{/if}
  </div>
</DevShell>

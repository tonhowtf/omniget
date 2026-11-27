<script lang="ts">
  import { t } from "$lib/i18n";
  import DevShell from "#components/devtools/DevShell.svelte";
  import KeyPanel from "#components/research/KeyPanel.svelte";
  import Workflows from "#components/research/Workflows.svelte";
  import Explorer from "#components/research/Explorer.svelte";

  type Tab = "workflows" | "explorer";
  const TABS: Tab[] = ["workflows", "explorer"];

  let tab = $state<Tab>("workflows");
  let configured = $state<boolean | null>(null);
  let usageTick = $state(0);
</script>

<DevShell icon="research" title={$t("research.title") as string} subtitle={$t("research.subtitle") as string}>
  <KeyPanel bind:configured {usageTick} />

  <div class="segmented dv-tabs" role="tablist" aria-label={$t("research.title") as string}>
    {#each TABS as id (id)}
      <button type="button" role="tab" id="rs-tab-{id}" class="segmented-btn" class:active={tab === id} aria-selected={tab === id} aria-controls="rs-panel-{id}" onclick={() => (tab = id)}>
        {$t(`research.tab_${id}`)}
      </button>
    {/each}
  </div>

  <div class="dv-stack" id="rs-panel-workflows" role="tabpanel" aria-labelledby="rs-tab-workflows" hidden={tab !== "workflows"}>
    <Workflows ready={configured === true} onran={() => usageTick++} />
  </div>
  <div class="dv-stack" id="rs-panel-explorer" role="tabpanel" aria-labelledby="rs-tab-explorer" hidden={tab !== "explorer"}>
    <Explorer ready={configured === true} onran={() => usageTick++} />
  </div>
</DevShell>

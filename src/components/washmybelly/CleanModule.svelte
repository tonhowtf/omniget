<script lang="ts">
  import { t } from "$lib/i18n";
  import { moduleById, type Caps } from "./registry";
  import ModuleHeader from "./ui/ModuleHeader.svelte";
  import JunkPanel from "./JunkPanel.svelte";
  import ArtifactsPanel from "./ArtifactsPanel.svelte";
  import DuplicatesPanel from "./DuplicatesPanel.svelte";
  import EmptyFoldersPanel from "./EmptyFoldersPanel.svelte";

  /// Clean up: four tabs over one plan-and-receipt flow. Every tab keeps
  /// its scan while the person moves between them; nothing scans by itself.
  let { caps }: { caps: Caps } = $props();

  type Tab = "junk" | "artifacts" | "duplicates" | "empty";
  const TABS: { id: Tab; key: string }[] = [
    { id: "junk", key: "washmybelly.clean.tabs.junk" },
    { id: "artifacts", key: "washmybelly.clean.tabs.artifacts" },
    { id: "duplicates", key: "washmybelly.clean.tabs.duplicates" },
    { id: "empty", key: "washmybelly.clean.tabs.empty" },
  ];
  const mod = moduleById("clean");

  let tab = $state<Tab>("junk");
  // A tab is mounted the first time it is shown, then kept.
  let seen = $state(new Set<Tab>(["junk"]));

  function pick(id: Tab) {
    tab = id;
    if (!seen.has(id)) seen = new Set([...seen, id]);
  }

  function onKey(e: KeyboardEvent) {
    if (e.key !== "ArrowRight" && e.key !== "ArrowLeft") return;
    const i = TABS.findIndex((x) => x.id === tab);
    const next = TABS[(i + (e.key === "ArrowRight" ? 1 : TABS.length - 1)) % TABS.length];
    pick(next.id);
    (document.getElementById(`wmb-clean-tab-${next.id}`) as HTMLElement | null)?.focus();
    e.preventDefault();
  }
</script>

<div class="wmb-clean">
  <ModuleHeader title={$t("washmybelly.modules.clean") as string} subtitle={$t("washmybelly.clean.subtitle") as string} tile={mod} />

  <div class="tabs segmented segmented-lg" role="tablist" aria-label={$t("washmybelly.modules.clean") as string} tabindex="-1" onkeydown={onKey}>
    {#each TABS as x (x.id)}
      <button
        id="wmb-clean-tab-{x.id}"
        type="button"
        class="segmented-btn"
        role="tab"
        aria-selected={tab === x.id}
        aria-controls="wmb-clean-panel-{x.id}"
        tabindex={tab === x.id ? 0 : -1}
        onclick={() => pick(x.id)}>{$t(x.key)}</button
      >
    {/each}
  </div>

  {#each TABS as x (x.id)}
    {#if seen.has(x.id)}
      <div id="wmb-clean-panel-{x.id}" role="tabpanel" aria-labelledby="wmb-clean-tab-{x.id}" hidden={tab !== x.id}>
        {#if x.id === "junk"}
          <JunkPanel {caps} />
        {:else if x.id === "artifacts"}
          <ArtifactsPanel />
        {:else if x.id === "duplicates"}
          <DuplicatesPanel {caps} />
        {:else}
          <EmptyFoldersPanel />
        {/if}
      </div>
    {/if}
  {/each}
</div>

<style>
  .wmb-clean {
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
    min-width: 0;
  }
  .tabs {
    align-self: center;
    max-width: 100%;
    overflow-x: auto;
  }
  [role="tabpanel"][hidden] {
    display: none;
  }
</style>

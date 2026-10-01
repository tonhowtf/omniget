<script lang="ts">
  import { t } from "$lib/i18n";
  import { moduleById, type Caps } from "./registry";
  import ModuleHeader from "./ui/ModuleHeader.svelte";
  import PrivacySettings from "./PrivacySettings.svelte";
  import TccPanel from "./TccPanel.svelte";
  import HostsPanel from "./HostsPanel.svelte";

  let { caps }: { caps: Caps } = $props();

  type Tab = "settings" | "permissions" | "trackers";
  const mod = moduleById("privacy");

  let tabs = $derived<{ id: Tab; key: string }[]>([
    { id: "settings", key: "washmybelly.privacy.tabs.settings" },
    ...(caps.os === "macos" ? [{ id: "permissions" as Tab, key: "washmybelly.privacy.tabs.permissions" }] : []),
    { id: "trackers", key: "washmybelly.privacy.tabs.trackers" },
  ]);

  let tab = $state<Tab>("settings");
  let seen = $state(new Set<Tab>(["settings"]));

  function pick(id: Tab) {
    tab = id;
    if (!seen.has(id)) seen = new Set([...seen, id]);
  }

  function onKey(e: KeyboardEvent) {
    if (e.key !== "ArrowRight" && e.key !== "ArrowLeft") return;
    const i = tabs.findIndex((x) => x.id === tab);
    const next = tabs[(i + (e.key === "ArrowRight" ? 1 : tabs.length - 1)) % tabs.length];
    pick(next.id);
    (document.getElementById(`wmb-privacy-tab-${next.id}`) as HTMLElement | null)?.focus();
    e.preventDefault();
  }
</script>

<div class="wmb-privacy">
  <ModuleHeader title={$t("washmybelly.modules.privacy") as string} subtitle={$t("washmybelly.privacy.subtitle") as string} tile={mod} />

  <div class="tabs segmented segmented-lg" role="tablist" aria-label={$t("washmybelly.modules.privacy") as string} tabindex="-1" onkeydown={onKey}>
    {#each tabs as x (x.id)}
      <button
        id="wmb-privacy-tab-{x.id}"
        type="button"
        class="segmented-btn"
        role="tab"
        aria-selected={tab === x.id}
        aria-controls="wmb-privacy-panel-{x.id}"
        tabindex={tab === x.id ? 0 : -1}
        onclick={() => pick(x.id)}>{$t(x.key)}</button
      >
    {/each}
  </div>

  {#each tabs as x (x.id)}
    {#if seen.has(x.id)}
      <div id="wmb-privacy-panel-{x.id}" role="tabpanel" aria-labelledby="wmb-privacy-tab-{x.id}" hidden={tab !== x.id}>
        {#if x.id === "settings"}
          <PrivacySettings privacySourceUrl={caps.privacy_source_url ?? ""} />
        {:else if x.id === "permissions"}
          <TccPanel />
        {:else}
          <HostsPanel />
        {/if}
      </div>
    {/if}
  {/each}
</div>

<style>
  .wmb-privacy {
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

<script lang="ts">
  import { onDestroy, onMount } from "svelte";
  import { t } from "$lib/i18n";
  import * as api from "$lib/washmybelly/api";
  import type { Progress } from "$lib/washmybelly/api";
  import { errorText } from "$lib/washmybelly/format";
  import { filterApps, removedAppId, sortApps, sourceInfo, sourcesPresent, type App, type AppSort, type AppsList } from "$lib/washmybelly/apps";
  import { moduleById, type Caps } from "./registry";
  import ModuleHeader from "./ui/ModuleHeader.svelte";
  import EmptyState from "./ui/EmptyState.svelte";
  import Icon from "./ui/Icon.svelte";
  import { onProgress } from "./ui/progress";
  import AppRow from "./AppRow.svelte";
  import UninstallFilters from "./UninstallFilters.svelte";
  import UninstallDetail from "./UninstallDetail.svelte";

  /// Uninstall: every app with where it came from, size and when it was last
  /// opened; pick one to see what it left behind and remove it with its
  /// leftovers through a plan. Also finds leftovers of apps already gone.
  let _props: { caps: Caps } = $props();

  const mod = moduleById("uninstall");

  let apps = $state<App[]>([]);
  let loading = $state(true);
  let error = $state<string | null>(null);
  let progress = $state<Progress | null>(null);
  let query = $state("");
  let sources = $state(new Set<string>());
  let sort = $state<AppSort>("name");
  let picked = $state<App | null>(null);
  let removedText = $state("");
  let width = $state(1000);
  let now = $state(Date.now() / 1000);

  let present = $derived(sourcesPresent(apps));
  let visible = $derived(sortApps(filterApps(apps, { query, sources }), sort));
  let narrow = $derived(width < 700);

  let unlisten: (() => void) | null = null;

  async function load() {
    error = null;
    loading = true;
    progress = null;
    try {
      const r = await api.call<AppsList>("wmb_apps_list");
      apps = r.apps ?? [];
      now = Date.now() / 1000;
      // Drop filters for origins that vanished.
      const have = new Set(apps.map((a) => a.source));
      if ([...sources].some((s) => !have.has(s))) sources = new Set([...sources].filter((s) => have.has(s)));
      if (picked && picked.source !== "removed") picked = apps.find((a) => a.id === picked?.id) ?? null;
    } catch (e) {
      error = errorText(e);
    } finally {
      loading = false;
      progress = null;
    }
  }

  onMount(async () => {
    unlisten = await onProgress("apps_list", (p) => (progress = p));
    void load();
  });
  onDestroy(() => unlisten?.());

  function sourceLabel(source: string): string {
    const info = sourceInfo(source);
    const s = $t(info.key) as string;
    return s === info.key ? info.fallback : s;
  }

  function reasonText(reason: string | null | undefined): string {
    if (!reason) return $t("washmybelly.uninstall.locked_default") as string;
    if (!reason.startsWith("washmybelly.")) return reason;
    const s = $t(reason) as string;
    return s === reason ? ($t("washmybelly.uninstall.locked_default") as string) : s;
  }

  function findRemoved(e: SubmitEvent) {
    e.preventDefault();
    const id = removedAppId(removedText);
    if (!id) return;
    const name = removedText.trim();
    picked = { id, name, source: "removed", removable: false, reason: "not installed anymore" };
  }

  function afterDone() {
    picked = null;
    void load();
  }
</script>

<ModuleHeader title={$t("washmybelly.modules.uninstall") as string} subtitle={$t("washmybelly.uninstall.subtitle") as string} tile={mod}>
  {#snippet actions()}
    <button class="btn btn-secondary" type="button" disabled={loading} onclick={() => void load()}>
      <Icon name="refresh" size={13} />{$t("washmybelly.uninstall.refresh")}
    </button>
  {/snippet}
</ModuleHeader>

{#if loading && apps.length === 0}
  <div class="wmb-un-loading" aria-busy="true">
    <p class="dim">{progress?.message || $t("washmybelly.uninstall.loading")}</p>
    {#each [0, 1, 2, 3, 4] as i (i)}<span class="skeleton" style:height="44px"></span>{/each}
  </div>
{:else if error && apps.length === 0}
  <EmptyState art="worried" title={$t("washmybelly.error_title") as string} body={error}>
    {#snippet action()}
      <button class="btn btn-primary" type="button" onclick={() => void load()}>{$t("washmybelly.retry")}</button>
    {/snippet}
  </EmptyState>
{:else}
  <div class="wmb-un" class:narrow bind:clientWidth={width}>
    {#if !(narrow && picked)}
      <section class="list-pane">
        {#if apps.length}
          <UninstallFilters bind:query bind:sources bind:sort {present} {sourceLabel} shown={visible.length} total={apps.length} />
          {#if visible.length}
            <div class="group app-list" role="list">
              {#each visible as a (a.id)}
                <AppRow app={a} selected={picked?.id === a.id} {sourceLabel} {reasonText} {now} onSelect={(x) => (picked = x)} />
              {/each}
            </div>
          {:else}
            <p class="dim none">{$t("washmybelly.uninstall.no_match")}</p>
          {/if}
        {:else}
          <EmptyState art="sleeping" compact title={$t("washmybelly.uninstall.empty_title") as string} body={$t("washmybelly.uninstall.empty_body") as string} />
        {/if}

        <form class="removed" onsubmit={findRemoved}>
          <label class="group-label" for="wmb-removed">{$t("washmybelly.uninstall.removed_title")}</label>
          <div class="removed-row">
            <input
              id="wmb-removed"
              class="input"
              type="text"
              autocomplete="off"
              spellcheck="false"
              placeholder={$t("washmybelly.uninstall.removed_placeholder") as string}
              bind:value={removedText}
            />
            <button class="btn btn-secondary" type="submit" disabled={!removedText.trim()}>{$t("washmybelly.uninstall.removed_button")}</button>
          </div>
          <p class="group-footer">{$t("washmybelly.uninstall.removed_hint")}</p>
        </form>
      </section>
    {/if}

    {#if picked}
      <section class="detail-pane">
        {#key picked.id}
          <UninstallDetail app={picked} {sourceLabel} {reasonText} onDone={afterDone} onBack={narrow ? () => (picked = null) : undefined} />
        {/key}
      </section>
    {:else if !narrow}
      <section class="detail-pane placeholder">
        <EmptyState art="searching" compact title={$t("washmybelly.uninstall.pick_title") as string} body={$t("washmybelly.uninstall.pick_body") as string} />
      </section>
    {/if}
  </div>
{/if}

<style>
  .wmb-un {
    display: grid;
    grid-template-columns: minmax(260px, 340px) minmax(0, 1fr);
    gap: var(--space-5);
    align-items: start;
  }
  .wmb-un.narrow {
    grid-template-columns: minmax(0, 1fr);
  }
  .list-pane {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
    min-width: 0;
  }
  .app-list {
    max-height: calc(100vh - 330px);
    min-height: 200px;
    overflow-y: auto;
    overscroll-behavior: contain;
  }
  .narrow .app-list {
    max-height: none;
  }
  .detail-pane {
    position: sticky;
    top: 0;
    min-width: 0;
  }
  .narrow .detail-pane {
    position: static;
  }
  .detail-pane.placeholder {
    padding-top: var(--space-7);
  }
  .removed {
    display: flex;
    flex-direction: column;
    margin-top: var(--space-2);
  }
  .removed-row {
    display: flex;
    gap: var(--space-2);
  }
  .removed-row .input {
    flex: 1;
    min-width: 0;
  }
  .dim {
    margin: 0;
    font-size: var(--text-sm);
    color: var(--text-dim);
  }
  .none {
    padding: var(--space-4) var(--space-2);
  }
  .wmb-un-loading {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    max-width: 340px;
  }
  .wmb-un-loading .skeleton {
    display: block;
    border-radius: var(--radius-md);
  }
</style>

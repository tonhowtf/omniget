<script lang="ts">
  import { onMount } from "svelte";
  import { goto } from "$app/navigation";
  import { t } from "$lib/i18n";
  import * as api from "$lib/washmybelly/api";
  import { errorText } from "$lib/washmybelly/format";
  import { filterItems, groupItems, kindKey, needsLook, noteCodes, type PersistenceItem, type PersistenceList } from "$lib/washmybelly/security";
  import { moduleById, type Caps } from "./registry";
  import ModuleHeader from "./ui/ModuleHeader.svelte";
  import EmptyState from "./ui/EmptyState.svelte";
  import Icon from "./ui/Icon.svelte";
  import SecurityRow from "./SecurityRow.svelte";

  /// Security (macOS): an honest inventory of everything set up to start by
  /// itself or to live inside the system, with the facts macOS can check
  /// (signature, notarization, quarantine, Team ID). It only reads; switching
  /// items off happens in Startup, linked from each row.
  let { caps }: { caps: Caps } = $props();

  const mod = moduleById("security");

  let list = $state<PersistenceList | null>(null);
  let loading = $state(true);
  let error = $state<string | null>(null);
  let thirdOnly = $state(true);
  let lookOnly = $state(false);
  let query = $state("");

  let items = $derived(list?.items ?? []);
  let visible = $derived(filterItems(items, { thirdPartyOnly: thirdOnly, query, lookOnly }));
  let groups = $derived(groupItems(visible));
  let lookCount = $derived(items.filter((i) => i.third_party && needsLook(i)).length);
  let thirdCount = $derived(items.filter((i) => i.third_party).length);
  let notes = $derived(noteCodes(list?.notes));

  async function load() {
    error = null;
    loading = true;
    try {
      list = await api.call<PersistenceList>("wmb_persistence_list", {});
    } catch (e) {
      error = errorText(e);
    } finally {
      loading = false;
    }
  }

  onMount(() => {
    void load();
  });

  async function reveal(path: string) {
    try {
      await api.ui("disk.reveal", { path });
    } catch (e) {
      error = errorText(e);
    }
  }

  function manage(_it: PersistenceItem) {
    void goto("/washmybelly?m=startup", { noScroll: true, keepFocus: true, replaceState: true });
  }

  async function openLoginItems() {
    try {
      await api.ui("security.open_login_items_settings");
    } catch (e) {
      error = errorText(e);
    }
  }
</script>

<ModuleHeader title={$t("washmybelly.modules.security") as string} subtitle={$t("washmybelly.security.subtitle") as string} tile={mod}>
  {#snippet meta()}
    {#if list}
      <span class="sec-meta">
        {$t("washmybelly.security.count", { count: thirdCount })}
        {#if lookCount}· {$t("washmybelly.security.look_count", { count: lookCount })}{/if}
      </span>
    {/if}
  {/snippet}
  {#snippet actions()}
    <button class="btn btn-secondary" type="button" disabled={loading} onclick={() => void load()}>
      <Icon name="refresh" size={13} />{$t("washmybelly.security.refresh")}
    </button>
  {/snippet}
</ModuleHeader>

<p class="sec-honest"><Icon name="info" size={12} />{$t("washmybelly.security.not_antivirus")}</p>

{#if caps.os !== "macos"}
  <EmptyState art="sleeping" title={$t("washmybelly.security.mac_only") as string} />
{:else if loading && !list}
  <div class="sec-loading" aria-busy="true">
    <p class="dim">{$t("washmybelly.security.loading")}</p>
    {#each [0, 1, 2, 3, 4] as i (i)}<span class="skeleton" style:height="56px"></span>{/each}
  </div>
{:else if error && !list}
  <EmptyState art="worried" title={$t("washmybelly.error_title") as string} body={error}>
    {#snippet action()}
      <button class="btn btn-primary" type="button" onclick={() => void load()}>{$t("washmybelly.retry")}</button>
    {/snippet}
  </EmptyState>
{:else}
  <div class="sec-bar">
    <input
      class="input sec-search"
      type="search"
      autocomplete="off"
      spellcheck="false"
      placeholder={$t("washmybelly.security.search") as string}
      bind:value={query}
    />
    <label class="sec-filter">
      <button type="button" class="toggle" class:on={thirdOnly} role="switch" aria-checked={thirdOnly} aria-label={$t("washmybelly.security.third_only") as string} onclick={() => (thirdOnly = !thirdOnly)}>
        <span class="toggle-knob"></span>
      </button>
      <span>{$t("washmybelly.security.third_only")}</span>
    </label>
    <label class="sec-filter">
      <button type="button" class="toggle" class:on={lookOnly} role="switch" aria-checked={lookOnly} aria-label={$t("washmybelly.security.look_only") as string} onclick={() => (lookOnly = !lookOnly)}>
        <span class="toggle-knob"></span>
      </button>
      <span>{$t("washmybelly.security.look_only")}</span>
    </label>
  </div>

  {#if error}<p class="sec-note warn"><Icon name="warning" size={12} />{error}</p>{/if}
  {#each notes as n (n)}
    <p class="sec-note"><Icon name="info" size={12} />{$t(`washmybelly.security.notes.${n}`)}</p>
  {/each}

  {#if groups.length === 0}
    {#if query.trim()}
      <p class="dim none">{$t("washmybelly.security.no_match")}</p>
    {:else if lookOnly}
      <EmptyState art="happy" title={$t("washmybelly.security.look_empty_title") as string} body={$t("washmybelly.security.look_empty_body") as string} />
    {:else}
      <EmptyState art="sleeping" title={$t("washmybelly.security.empty_title") as string} body={$t("washmybelly.security.empty_body") as string} />
    {/if}
  {:else}
    {#each groups as g (g.kind)}
      <span class="group-label">
        {$t(kindKey(g.kind))}
        <span class="sec-count">{g.items.length}</span>
      </span>
      <div class="group" role="list">
        {#each g.items as it (it.id)}
          <SecurityRow item={it} onReveal={(p) => void reveal(p)} onManage={manage} />
        {/each}
      </div>
    {/each}
    <p class="group-footer">
      {$t("washmybelly.security.footer")}
      <button class="link" type="button" onclick={() => void openLoginItems()}>{$t("washmybelly.security.open_login_items")}</button>
    </p>
  {/if}
{/if}

<style>
  .sec-meta {
    font-size: var(--text-sm);
    color: var(--text-dim);
  }
  .sec-honest {
    display: flex;
    align-items: flex-start;
    gap: 6px;
    margin: 0 0 var(--space-4);
    font-size: var(--text-sm);
    color: var(--text-muted);
    line-height: 1.45;
  }
  .sec-honest :global(.wmb-icon) {
    flex-shrink: 0;
    margin-top: 3px;
  }
  .sec-bar {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-3);
    margin-bottom: var(--space-4);
  }
  .sec-search {
    flex: 1 1 200px;
    min-width: 0;
  }
  .sec-filter {
    display: inline-flex;
    align-items: center;
    gap: var(--space-2);
    font-size: var(--text-sm);
    color: var(--text-muted);
    cursor: pointer;
  }
  .sec-note {
    display: flex;
    align-items: center;
    gap: 6px;
    margin: 0 0 var(--space-2);
    font-size: var(--text-sm);
    color: var(--text-dim);
  }
  .sec-note.warn {
    color: var(--warning, #ff9f0a);
  }
  .sec-count {
    font-weight: 400;
    margin-left: var(--space-1);
  }
  .group {
    margin-bottom: var(--space-5);
  }
  .link {
    padding: 0;
    border: 0;
    background: none;
    color: var(--accent);
    font: inherit;
    cursor: pointer;
  }
  .dim {
    margin: 0;
    font-size: var(--text-sm);
    color: var(--text-dim);
  }
  .none {
    padding: var(--space-4) var(--space-2);
  }
  .sec-loading {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
  }
  .sec-loading .skeleton {
    display: block;
    border-radius: var(--radius-md);
  }
</style>

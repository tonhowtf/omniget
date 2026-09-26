<script lang="ts">
  /**
   * One scope (your profile, a bot's private notes, a room): its memories
   * grouped by kind, and a box to add one yourself. The empty state says
   * what to do next.
   */
  import { t } from "$lib/i18n";
  import MemoryItemRow from "./MemoryItemRow.svelte";
  import {
    createMemory,
    groupByKind,
    type MemoryItem,
    type MemoryKind,
  } from "$lib/stores/assist-memory-store.svelte";

  let {
    scopeKey,
    title,
    lede = "",
    items,
    showInactive = false,
  }: { scopeKey: string; title: string; lede?: string; items: MemoryItem[]; showInactive?: boolean } = $props();

  let groups = $derived(groupByKind(items));
  let inactive = $derived(items.filter((i) => i.status !== "active"));
  let adding = $state(false);
  let draft = $state("");
  let kind = $state<MemoryKind>("declared");
  let busy = $state(false);
  let errorKey = $state<string | null>(null);
  const kinds: MemoryKind[] = ["declared", "temporary", "procedural"];
  const inputId = `memory-add-${Math.random().toString(36).slice(2, 8)}`;

  async function add() {
    const text = draft.trim();
    if (!text || busy) return;
    busy = true;
    const out = await createMemory(scopeKey, kind, text);
    busy = false;
    if (out.ok) {
      draft = "";
      errorKey = null;
      adding = false;
    } else {
      errorKey = out.errorKey; // keep what was typed
    }
  }
</script>

<section class="block memory-scope" aria-label={title}>
  <header class="scope-head">
    <div>
      <h3 class="block-title">{title}</h3>
      {#if lede}<p class="dim">{lede}</p>{/if}
    </div>
    <button type="button" class="button small" aria-expanded={adding} onclick={() => (adding = !adding)}>{$t("assist.memory.add")}</button>
  </header>

  {#if adding}
    <form class="add" onsubmit={(e) => { e.preventDefault(); void add(); }}>
      <label class="field">
        <span class="field-label">{$t("assist.memory.add_label")}</span>
        <textarea id={inputId} class="input" rows="2" bind:value={draft} placeholder={$t("assist.memory.add_placeholder")}></textarea>
      </label>
      <label class="field kind">
        <span class="field-label">{$t("assist.memory.kind_label")}</span>
        <select class="input" bind:value={kind}>
          {#each kinds as k}<option value={k}>{$t(`assist.memory.kind.${k}`)}</option>{/each}
        </select>
      </label>
      <div class="actions">
        <button type="submit" class="button primary" disabled={busy || !draft.trim()}>{$t("assist.memory.save")}</button>
        <button type="button" class="button" onclick={() => { adding = false; errorKey = null; }}>{$t("assist.memory.cancel")}</button>
      </div>
      {#if errorKey}<p class="notice error" role="alert">{$t(errorKey)}</p>{/if}
    </form>
  {/if}

  {#if groups.length === 0}
    <p class="empty">{$t("assist.memory.empty_scope")}</p>
  {:else}
    {#each groups as g (g.kind)}
      <h4 class="kind-title">{$t(`assist.memory.kind.${g.kind}`)} <span class="count">{g.items.length}</span></h4>
      <ul class="items">
        {#each g.items as item (item.id)}<MemoryItemRow {item} />{/each}
      </ul>
    {/each}
  {/if}

  {#if showInactive && inactive.length}
    <details class="inactive">
      <summary>{$t("assist.memory.inactive_title", { count: inactive.length })}</summary>
      <ul class="items">
        {#each inactive as item (item.id)}<MemoryItemRow {item} />{/each}
      </ul>
    </details>
  {/if}
</section>

<style>
  .memory-scope { display: grid; gap: 8px; }
  .scope-head { display: flex; justify-content: space-between; align-items: flex-start; gap: 12px; }
  .dim { margin: 2px 0 0; color: var(--text-muted); font-size: 13px; line-height: 1.5; max-width: 72ch; }
  .kind-title { margin: 12px 0 0; font-size: 13px; font-weight: 600; color: var(--text); }
  .count { color: var(--text-muted); font-weight: 400; }
  .items { margin: 0; padding: 0; }
  .empty { margin: 0; color: var(--text-muted); font-size: 13px; line-height: 1.5; }
  .add { display: grid; gap: 8px; padding: 12px 0; border-bottom: 1px solid var(--separator); }
  .add textarea { width: 100%; resize: vertical; }
  .kind select { max-width: 260px; }
  .actions { display: flex; flex-wrap: wrap; gap: 8px; }
  .small { min-height: 32px; padding: 4px 10px; font-size: 13px; }
  .inactive summary { cursor: pointer; font-size: 13px; color: var(--accent-text); border-radius: var(--radius-sm); }
  .inactive summary:focus-visible, .button:focus-visible, select:focus-visible, textarea:focus-visible { outline: var(--focus-ring); outline-offset: 2px; }
  @media (max-width: 560px) { .scope-head { flex-direction: column; } }
</style>

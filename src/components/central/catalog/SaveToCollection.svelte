<script lang="ts">
  /** "Save to collection" popover: toggle membership in each collection, or create one with this item. */
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import { collections as coll, errText, type Collection } from "$lib/central/catalog";
  import { stack } from "$lib/central/stack-store.svelte";

  let { itemId }: { itemId: string } = $props();

  let open = $state(false);
  let list = $state<Collection[]>([]);
  let name = $state("");
  let busy = $state(false);
  let root = $state<HTMLElement | null>(null);

  let inCount = $derived(list.filter((c) => c.items.some((i) => i.item_id === itemId)).length);

  async function load() {
    try {
      list = await coll.list();
    } catch (e) {
      showToast("error", errText(e));
    }
  }

  $effect(() => {
    void itemId;
    void stack.rev;
    void load();
  });

  async function toggle(c: Collection) {
    busy = true;
    try {
      if (c.items.some((i) => i.item_id === itemId)) await coll.remove(c.id, itemId);
      else await coll.add(c.id, itemId);
      await load();
      stack.rev++;
    } catch (e) {
      showToast("error", errText(e));
    } finally {
      busy = false;
    }
  }

  async function create() {
    const n = name.trim();
    if (!n) return;
    busy = true;
    try {
      const c = await coll.create(n);
      await coll.add(c.id, itemId);
      name = "";
      await load();
      stack.rev++;
    } catch (e) {
      showToast("error", errText(e));
    } finally {
      busy = false;
    }
  }

  function onwindow(e: MouseEvent) {
    if (open && root && !root.contains(e.target as Node)) open = false;
  }
  function onkey(e: KeyboardEvent) {
    if (open && e.key === "Escape") open = false;
  }
</script>

<svelte:window onclick={onwindow} onkeydown={onkey} />

<div class="save" bind:this={root}>
  <button type="button" class="btn" aria-expanded={open} aria-haspopup="true" onclick={() => (open = !open)}>
    {$t("llm.central.catalog.collections.save_to")}{#if inCount} · {inCount}{/if}
  </button>
  {#if open}
    <div class="pop" role="group" aria-label={$t("llm.central.catalog.collections.save_to")}>
      {#each list as c (c.id)}
        <label class="opt">
          <input type="checkbox" checked={c.items.some((i) => i.item_id === itemId)} disabled={busy} onchange={() => toggle(c)} />
          <span>{c.name}</span>
          <span class="n tabular">{c.items.length}</span>
        </label>
      {/each}
      {#if !list.length}<p class="empty">{$t("llm.central.catalog.collections.empty")}</p>{/if}
      <form class="new" onsubmit={(e) => { e.preventDefault(); void create(); }}>
        <input bind:value={name} placeholder={$t("llm.central.catalog.collections.new_with_item")} aria-label={$t("llm.central.catalog.collections.new_with_item")} />
        <button type="submit" disabled={busy || !name.trim()}>+</button>
      </form>
    </div>
  {/if}
</div>

<style>
  .save {
    position: relative;
  }
  .btn {
    height: var(--control-h-lg);
    padding: 0 var(--space-3);
    border: none;
    border-radius: var(--radius-md);
    background: var(--fill-1);
    color: var(--text);
    font-size: var(--text-base);
    cursor: pointer;
  }
  .btn:hover {
    background: var(--fill-2);
  }
  .pop {
    position: absolute;
    top: calc(100% + 6px);
    right: 0;
    z-index: 20;
    width: 260px;
    padding: var(--space-2);
    border-radius: var(--radius-lg);
    background: var(--material-thick);
    backdrop-filter: var(--material-blur);
    box-shadow: var(--elev-2);
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .opt {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    padding: 4px var(--space-1);
    border-radius: var(--radius-sm);
    font-size: var(--text-sm);
    cursor: pointer;
  }
  .opt:hover {
    background: var(--fill-1);
  }
  .opt input {
    accent-color: var(--accent);
  }
  .opt span:first-of-type {
    flex: 1;
  }
  .n {
    color: var(--text-faint);
    font-size: var(--text-xs);
  }
  .empty {
    margin: 0;
    padding: 4px;
    font-size: var(--text-sm);
    color: var(--text-muted);
  }
  .new {
    display: flex;
    gap: 4px;
    margin-top: var(--space-1);
    padding-top: var(--space-2);
    border-top: 1px solid var(--separator);
  }
  .new input {
    flex: 1;
    min-width: 0;
    height: var(--control-h);
    padding: 0 var(--space-2);
    border: none;
    border-radius: var(--radius-sm);
    background: var(--input-bg);
    color: var(--text);
    font-size: var(--text-sm);
  }
  .new button {
    width: var(--control-h);
    border: none;
    border-radius: var(--radius-sm);
    background: var(--cta);
    color: var(--on-cta);
    cursor: pointer;
  }
  .new button:disabled {
    opacity: 0.4;
  }
</style>

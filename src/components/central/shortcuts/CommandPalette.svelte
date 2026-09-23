<script lang="ts">
  /**
   * Paleta de comandos da Central (⌘K): ações (com o atalho ao lado),
   * threads e itens do catálogo (`catalog_search`, só com consulta). `>` no
   * começo filtra só ações. ↑↓ navega, Enter executa, Esc fecha; ao fechar o
   * foco volta para onde estava.
   */
  import { onMount, tick } from "svelte";
  import { t } from "$lib/i18n";
  import { catalogSearch, itemHref, kindStyle } from "$lib/central/catalog";
  import { goto } from "$app/navigation";
  import { filterItems, type PaletteItem } from "./palette";

  let { items, onclose }: { items: PaletteItem[]; onclose: () => void } = $props();

  const GROUPS: PaletteItem["group"][] = ["actions", "threads", "navigation", "catalog"];
  const CATALOG_LIMIT = 8;

  let query = $state("");
  let active = $state(0);
  let inputEl = $state<HTMLInputElement | null>(null);
  let listEl = $state<HTMLDivElement | null>(null);
  let catalog = $state<PaletteItem[]>([]);
  let catalogBusy = $state(false);
  let seq = 0;

  let actionsOnly = $derived(query.trimStart().startsWith(">"));
  let text = $derived(actionsOnly ? query.trimStart().slice(1) : query);
  let results = $derived.by(() => {
    const base = actionsOnly ? items.filter((i) => i.group === "actions") : items;
    let list = filterItems(base, text, GROUPS);
    if (!text.trim() && !actionsOnly) {
      // Sem consulta: ações + 12 threads mais recentes (a ordem já vem do chamador).
      const threads = list.filter((i) => i.group === "threads").slice(0, 12);
      list = [...list.filter((i) => i.group !== "threads"), ...threads];
      list = filterItems(list, "", GROUPS);
    }
    return actionsOnly || !text.trim() ? list : [...list, ...catalog];
  });

  $effect(() => {
    void results.length;
    active = 0;
  });

  // Catálogo: só com consulta de 2+ letras, com espera curta.
  $effect(() => {
    const q = text.trim();
    const mine = ++seq;
    if (actionsOnly || q.length < 2) {
      catalog = [];
      return;
    }
    const timer = setTimeout(async () => {
      catalogBusy = true;
      try {
        const r = await catalogSearch({ query: q, limit: CATALOG_LIMIT });
        if (mine !== seq) return;
        catalog = r.items.map((h) => ({
          id: `catalog:${h.id}`,
          group: "catalog" as const,
          title: h.name,
          subtitle: `${kindStyle(h.kind).glyph} ${h.kind} · ${h.category}${h.description ? ` — ${h.description}` : ""}`,
          run: () => goto(itemHref(h.id)),
        }));
      } catch {
        if (mine === seq) catalog = [];
      } finally {
        if (mine === seq) catalogBusy = false;
      }
    }, 160);
    return () => clearTimeout(timer);
  });

  async function runItem(it: PaletteItem | undefined) {
    if (!it || it.disabled) return;
    onclose();
    await tick();
    await it.run();
  }

  function onkey(e: KeyboardEvent) {
    if (e.key === "ArrowDown") {
      e.preventDefault();
      active = Math.min(results.length - 1, active + 1);
      scrollActive();
    } else if (e.key === "ArrowUp") {
      e.preventDefault();
      active = Math.max(0, active - 1);
      scrollActive();
    } else if (e.key === "Enter") {
      e.preventDefault();
      void runItem(results[active]);
    } else if (e.key === "Escape") {
      e.preventDefault();
      e.stopPropagation();
      onclose();
    }
  }

  function scrollActive() {
    void tick().then(() => listEl?.querySelector(`[data-index="${active}"]`)?.scrollIntoView({ block: "nearest" }));
  }

  onMount(() => {
    inputEl?.focus();
  });

  function groupTitle(g: PaletteItem["group"]): string {
    return $t(`llm.central.shortcuts.palette.group.${g}`) as string;
  }
</script>

<!-- svelte-ignore a11y_click_events_have_key_events -->
<div class="backdrop" role="presentation" data-command-palette onclick={onclose}>
  <div class="palette" role="dialog" aria-modal="true" aria-label={$t("llm.central.shortcuts.palette.title")} tabindex="-1" onclick={(e) => e.stopPropagation()}>
    <input
      bind:this={inputEl}
      bind:value={query}
      class="q"
      type="text"
      role="combobox"
      aria-expanded="true"
      aria-controls="central-palette-list"
      aria-activedescendant={results[active] ? `pal-${active}` : undefined}
      placeholder={$t("llm.central.shortcuts.palette.placeholder") as string}
      spellcheck="false"
      autocomplete="off"
      onkeydown={onkey}
    />
    <div class="list" id="central-palette-list" role="listbox" bind:this={listEl}>
      {#each results as it, i (it.id)}
        {#if i === 0 || results[i - 1].group !== it.group}
          <div class="group">{groupTitle(it.group)}</div>
        {/if}
        <div
          id={`pal-${i}`}
          data-index={i}
          class="row"
          class:active={i === active}
          class:disabled={it.disabled}
          role="option"
          aria-selected={i === active}
          tabindex="-1"
          onmousemove={() => (active = i)}
          onclick={() => runItem(it)}
        >
          <span class="title">{it.title}</span>
          {#if it.subtitle}<span class="sub">{it.subtitle}</span>{/if}
          {#if it.shortcut}<kbd>{it.shortcut}</kbd>{/if}
        </div>
      {:else}
        <div class="empty">
          {catalogBusy ? $t("llm.central.shortcuts.palette.searching") : actionsOnly ? $t("llm.central.shortcuts.palette.no_actions") : $t("llm.central.shortcuts.palette.empty")}
        </div>
      {/each}
      {#if catalogBusy && results.length > 0}<div class="empty">{$t("llm.central.shortcuts.palette.searching")}</div>{/if}
    </div>
    <footer class="legend">
      <span><kbd>↑↓</kbd> {$t("llm.central.shortcuts.palette.navigate")}</span>
      <span><kbd>↩</kbd> {$t("llm.central.shortcuts.palette.select")}</span>
      <span><kbd>esc</kbd> {$t("llm.central.shortcuts.palette.close")}</span>
      <span><kbd>&gt;</kbd> {$t("llm.central.shortcuts.palette.actions_only")}</span>
    </footer>
  </div>
</div>

<style>
  .backdrop {
    position: fixed;
    inset: 0;
    z-index: 1000;
    background: var(--dialog-backdrop, rgba(0, 0, 0, 0.35));
    display: flex;
    justify-content: center;
    align-items: flex-start;
    padding: 12vh 16px 16px;
  }
  .palette {
    width: min(620px, 100%);
    background: var(--popup-bg, var(--surface));
    color: var(--text);
    border-radius: var(--radius-xl, 14px);
    box-shadow: var(--elev-3);
    border: 1px solid var(--separator);
    display: flex;
    flex-direction: column;
    overflow: hidden;
  }
  .q {
    border: none;
    border-bottom: 1px solid var(--separator);
    background: transparent;
    color: var(--text);
    font: inherit;
    font-size: var(--text-md);
    padding: 14px 16px;
    outline: none;
  }
  .list {
    max-height: min(28rem, 60vh);
    overflow-y: auto;
    padding: 6px;
  }
  .group {
    font-size: var(--text-caption);
    color: var(--text-dim);
    padding: 8px 10px 4px;
    text-transform: uppercase;
    letter-spacing: 0.04em;
  }
  .row {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 8px 10px;
    border-radius: 8px;
    cursor: pointer;
    min-width: 0;
  }
  .row.active {
    background: var(--accent-soft);
  }
  .row.disabled {
    opacity: 0.64;
    cursor: default;
  }
  .title {
    font-size: var(--text-sm);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    flex-shrink: 0;
    max-width: 55%;
  }
  .sub {
    font-size: var(--text-caption);
    color: var(--text-dim);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    flex: 1;
    min-width: 0;
  }
  kbd {
    margin-left: auto;
    font-family: var(--font-mono);
    font-size: var(--text-caption);
    color: var(--text-muted);
    background: var(--fill-1);
    border-radius: 5px;
    padding: 1px 6px;
    flex-shrink: 0;
  }
  .empty {
    padding: 16px;
    color: var(--text-dim);
    font-size: var(--text-sm);
    text-align: center;
  }
  .legend {
    display: flex;
    flex-wrap: wrap;
    gap: 14px;
    padding: 8px 14px;
    border-top: 1px solid var(--separator);
    font-size: var(--text-caption);
    color: var(--text-dim);
  }
  .legend kbd {
    margin: 0 2px 0 0;
  }
</style>

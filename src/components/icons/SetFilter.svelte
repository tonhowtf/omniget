<script lang="ts" module>
  export type SetScope = { mode: "recommended" } | { mode: "all" } | { mode: "custom"; prefixes: string[] };
</script>

<script lang="ts">
  import { t } from "$lib/i18n";
  import { groupSets, licenseKind, needsWarning, orderRecommended } from "$lib/icons/sets";
  import type { IconSet } from "$lib/icons/types";
  import LicenseBadge from "./LicenseBadge.svelte";
  import Glyph from "./Glyph.svelte";

  let {
    sets,
    scope,
    loading = false,
    onchange,
  }: { sets: IconSet[]; scope: SetScope; loading?: boolean; onchange: (scope: SetScope) => void } = $props();

  let browsing = $state(false);
  let filter = $state("");
  let recommended = $derived(orderRecommended(sets));
  let recPrefixes = $derived(new Set(recommended.map((s) => s.prefix)));
  let custom = $derived(scope.mode === "custom" ? scope.prefixes : []);
  let extras = $derived(custom.filter((p) => !recPrefixes.has(p)).map((p) => sets.find((s) => s.prefix === p)).filter((s): s is IconSet => Boolean(s)));
  let groups = $derived(groupSets(sets, filter));
  let fmt = $derived(new Intl.NumberFormat());

  function toggle(prefix: string) {
    const next = custom.includes(prefix) ? custom.filter((p) => p !== prefix) : [...custom, prefix];
    onchange(next.length ? { mode: "custom", prefixes: next } : { mode: "recommended" });
  }
</script>

<div class="ic-filter" role="group" aria-label={$t("icons.filter_label") as string}>
  <div class="ic-chips">
    <button type="button" class="ic-chip" class:on={scope.mode === "recommended"} aria-pressed={scope.mode === "recommended"} onclick={() => onchange({ mode: "recommended" })}>
      {$t("icons.scope_recommended")}
    </button>
    {#each recommended as set (set.prefix)}
      <button type="button" class="ic-chip" class:on={custom.includes(set.prefix)} aria-pressed={custom.includes(set.prefix)} onclick={() => toggle(set.prefix)} title={$t("icons.set_title", { name: set.name, total: fmt.format(set.total), license: set.license ?? "?" }) as string}>
        {set.name}
        {#if needsWarning(licenseKind(set.license_spdx))}<LicenseBadge {set} compact />{/if}
      </button>
    {/each}
    {#each extras as set (set.prefix)}
      <button type="button" class="ic-chip on" aria-pressed="true" onclick={() => toggle(set.prefix)}>
        {set.name}
        <LicenseBadge {set} compact />
        <Glyph name="close" size={11} />
      </button>
    {/each}
    <button type="button" class="ic-chip" class:on={scope.mode === "all"} aria-pressed={scope.mode === "all"} onclick={() => onchange({ mode: "all" })}>
      {sets.length ? $t("icons.scope_all_count", { count: sets.length }) : $t("icons.scope_all")}
    </button>
    <button type="button" class="btn btn-ghost btn-sm ic-browse" aria-expanded={browsing} aria-controls="ic-set-browser" disabled={loading || sets.length === 0} title={loading ? ($t("icons.sets_loading") as string) : undefined} onclick={() => (browsing = !browsing)}>
      <Glyph name="grid" size={13} />
      {browsing ? $t("icons.browse_close") : $t("icons.browse_sets")}
    </button>
  </div>

  {#if browsing}
    <div class="ic-browser" id="ic-set-browser">
      <label class="ic-browser-search">
        <Glyph name="search" size={13} />
        <span class="ic-sr">{$t("icons.filter_sets")}</span>
        <input type="search" bind:value={filter} placeholder={$t("icons.filter_sets_placeholder") as string} />
      </label>
      {#if groups.length === 0}
        <p class="ic-browser-empty">{$t("icons.filter_sets_none", { query: filter })}</p>
      {/if}
      {#each groups as group (group.key)}
        <section class="ic-browser-group" aria-labelledby="ic-cat-{group.key}">
          <h3 id="ic-cat-{group.key}">{$t(`icons.category.${group.key}`)} <span>{group.sets.length}</span></h3>
          <ul>
            {#each group.sets as set (set.prefix)}
              {@const on = custom.includes(set.prefix)}
              <li>
                <button type="button" class="ic-set-row" class:on aria-pressed={on} onclick={() => toggle(set.prefix)}>
                  <span class="ic-set-check" aria-hidden="true">{#if on}<Glyph name="check" size={12} />{/if}</span>
                  <span class="ic-set-name">{set.name}</span>
                  <span class="ic-set-total">{fmt.format(set.total)}</span>
                  <LicenseBadge {set} compact />
                </button>
              </li>
            {/each}
          </ul>
        </section>
      {/each}
    </div>
  {/if}
</div>

<style>
  .ic-filter {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
  }

  .ic-chips {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-2);
  }

  .ic-chip {
    display: inline-flex;
    align-items: center;
    gap: var(--space-1);
    min-height: var(--control-h);
    padding: 0 var(--space-3);
    border: none;
    border-radius: var(--radius-full);
    background: var(--fill-1);
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
    color: var(--text);
    font: inherit;
    font-size: var(--text-sm);
    font-weight: 550;
    cursor: pointer;
    transition: background var(--duration-fast) var(--ease-out);
  }

  @media (hover: hover) {
    .ic-chip:hover:not(.on) {
      background: var(--fill-2);
    }
  }

  .ic-chip:focus-visible,
  .ic-set-row:focus-visible {
    outline: var(--focus-ring);
    outline-offset: var(--focus-ring-offset);
  }

  .ic-chip.on {
    background: var(--accent-soft);
    box-shadow: inset 0 0 0 1.5px var(--accent);
  }

  .ic-browse {
    gap: var(--space-1);
  }

  .ic-browser {
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
    max-height: 360px;
    padding: var(--space-3) var(--space-4) var(--space-4);
    overflow-y: auto;
    border-radius: var(--radius-lg);
    background: var(--surface);
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
  }

  .ic-browser-search {
    position: sticky;
    top: calc(-1 * var(--space-3));
    z-index: 1;
    display: flex;
    align-items: center;
    gap: var(--space-2);
    padding: var(--space-2) 0;
    background: var(--surface);
    color: var(--text-dim);
  }

  .ic-browser-search input {
    flex: 1;
    min-width: 0;
    height: var(--control-h);
    padding: 0 var(--space-2);
    border: none;
    border-radius: var(--radius-sm);
    background: var(--input-bg);
    box-shadow: inset 0 0 0 var(--hairline) var(--input-border);
    color: var(--text);
    font: inherit;
    font-size: var(--text-sm);
  }

  .ic-browser-search input:focus-visible {
    outline: var(--focus-ring);
  }

  .ic-browser-empty {
    margin: 0;
    color: var(--text-dim);
    font-size: var(--text-sm);
  }

  .ic-browser-group h3 {
    display: flex;
    gap: var(--space-2);
    margin: 0 0 var(--space-2);
    color: var(--text-dim);
    font-size: var(--text-xs);
    font-weight: 600;
    letter-spacing: var(--track-caps);
    text-transform: uppercase;
  }

  .ic-browser-group h3 span {
    color: var(--text-faint);
  }

  .ic-browser-group ul {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(230px, 1fr));
    gap: 2px var(--space-3);
    margin: 0;
    padding: 0;
    list-style: none;
  }

  .ic-set-row {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    width: 100%;
    min-height: var(--row-compact);
    padding: 0 var(--space-2);
    border: none;
    border-radius: var(--radius-sm);
    background: transparent;
    color: var(--text);
    font: inherit;
    font-size: var(--text-sm);
    text-align: left;
    cursor: pointer;
  }

  @media (hover: hover) {
    .ic-set-row:hover {
      background: var(--fill-1);
    }
  }

  .ic-set-row.on {
    background: var(--accent-soft);
  }

  .ic-set-check {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 14px;
    height: 14px;
    border-radius: var(--radius-xs);
    box-shadow: inset 0 0 0 var(--hairline) var(--input-border);
    color: var(--accent-text);
  }

  .ic-set-name {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .ic-set-total {
    color: var(--text-dim);
    font-size: var(--text-xs);
    font-variant-numeric: tabular-nums;
  }

  .ic-sr {
    position: absolute;
    width: 1px;
    height: 1px;
    overflow: hidden;
    clip: rect(0 0 0 0);
    white-space: nowrap;
  }

  @media (prefers-reduced-motion: reduce) {
    .ic-chip {
      transition: none;
    }
  }
</style>

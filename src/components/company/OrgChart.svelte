<script lang="ts">
  /**
   * Org chart: a tree of staff cards (avatar, name, title, live status).
   * Pass `nodes` from `staff.chart`, or `staff` (a flat list) and the tree
   * is built here. `onselect` makes the cards buttons.
   */
  import type { ChartNode, Staff } from "$lib/company/rpc";
  import { t } from "$lib/i18n";
  import StatusDot from "./StatusDot.svelte";
  import { avatarColor, chartOf, initials } from "./company-ui";

  let {
    nodes = null,
    staff = [],
    selected = null,
    onselect,
    compact = false,
  }: {
    nodes?: ChartNode[] | null;
    staff?: Staff[];
    selected?: string | null;
    onselect?: (s: Staff) => void;
    compact?: boolean;
  } = $props();

  let tree = $derived(nodes ?? chartOf(staff));
</script>

{#snippet card(s: Staff)}
  {#if onselect}
    <button type="button" class="org-card" class:selected={selected === s.id} class:dim={s.status === "terminated"} onclick={() => onselect?.(s)}>
      {@render cardInner(s)}
    </button>
  {:else}
    <div class="org-card" class:dim={s.status === "terminated"}>{@render cardInner(s)}</div>
  {/if}
{/snippet}

{#snippet cardInner(s: Staff)}
  <span class="org-avatar" style:background={avatarColor(s.name)} aria-hidden="true">{initials(s.name)}</span>
  <span class="org-text">
    <span class="org-name">{s.name}</span>
    <span class="org-title">{s.title || s.role}</span>
  </span>
  <StatusDot status={s.status} />
{/snippet}

{#snippet branch(list: ChartNode[])}
  <ul>
    {#each list as n (n.staff.id)}
      <li>
        {@render card(n.staff)}
        {#if n.reports.length}{@render branch(n.reports)}{/if}
      </li>
    {/each}
  </ul>
{/snippet}

<div class="org-chart" class:compact role="group" aria-label={$t("company.team.chart")}>
  {#if tree.length}
    {@render branch(tree)}
  {/if}
</div>

<style>
  .org-chart { overflow-x: auto; padding: var(--space-2) 0 var(--space-3); }
  .org-chart ul { display: flex; justify-content: center; gap: 0; margin: 0; padding: 18px 0 0; list-style: none; position: relative; }
  .org-chart > ul { padding-top: 0; }
  .org-chart li { display: flex; flex-direction: column; align-items: center; position: relative; padding: 18px 8px 0; }
  .org-chart > ul > li { padding-top: 0; }
  /* Connectors: a horizontal bar over siblings, a vertical tick into each card. */
  .org-chart li::before,
  .org-chart li::after { content: ""; position: absolute; top: 0; width: 50%; height: 18px; border-top: var(--hairline) solid var(--border-hi, var(--separator)); }
  .org-chart li::before { right: 50%; }
  .org-chart li::after { left: 50%; border-left: var(--hairline) solid var(--border-hi, var(--separator)); }
  .org-chart li:only-child::before { display: none; }
  .org-chart li:only-child::after { border-top: 0; }
  .org-chart li:first-child::before,
  .org-chart li:last-child::after { border-top: 0; }
  .org-chart li:last-child::before { border-right: var(--hairline) solid var(--border-hi, var(--separator)); }
  .org-chart li:first-child:not(:only-child)::after { border-radius: 6px 0 0 0; }
  .org-chart li:last-child:not(:only-child)::before { border-radius: 0 6px 0 0; }
  .org-chart > ul > li::before,
  .org-chart > ul > li::after { display: none; }
  .org-chart ul ul::before { content: ""; position: absolute; top: 0; left: 50%; height: 18px; border-left: var(--hairline) solid var(--border-hi, var(--separator)); }

  .org-card {
    display: flex; align-items: center; gap: var(--space-2);
    min-width: 168px; max-width: 220px; padding: var(--space-2) var(--space-3);
    background: var(--surface); border: 0; border-radius: var(--radius-md);
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border, var(--separator));
    color: inherit; text-align: left; font: inherit; position: relative; z-index: 1;
  }
  button.org-card { cursor: pointer; }
  @media (hover: hover) { button.org-card:hover { background: var(--surface-hi); } }
  button.org-card:focus-visible { outline: var(--focus-ring); outline-offset: var(--focus-ring-offset); }
  .org-card.selected { box-shadow: inset 0 0 0 1.5px var(--accent); }
  .org-card.dim { opacity: 0.55; }
  .org-avatar {
    display: inline-flex; align-items: center; justify-content: center; flex-shrink: 0;
    width: 30px; height: 30px; border-radius: 50%; color: #fff; font-size: var(--text-xs); font-weight: 700;
  }
  .org-text { display: flex; flex-direction: column; min-width: 0; flex: 1; }
  .org-name { font-size: var(--text-sm); font-weight: 600; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .org-title { font-size: var(--text-xs); color: var(--text-dim); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .compact .org-card { min-width: 140px; padding: 6px var(--space-2); }
  .compact .org-avatar { width: 24px; height: 24px; font-size: 10px; }
</style>

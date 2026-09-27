<script lang="ts">
  /**
   * One catalog result (grid tile or list row): kind tile, name, category and
   * source, description, tools coloured by compatibility, stars, licence,
   * guard seal and the stack toggle. The whole card links to the detail.
   */
  import { t } from "$lib/i18n";
  import GuardBadge from "$components/central/guard/GuardBadge.svelte";
  import { compactNumber, itemHref, type Compat, type SearchHit } from "$lib/central/catalog";
  import { guardSeals } from "$lib/central/stack-store.svelte";
  import KindIcon from "./KindIcon.svelte";
  import ToolDots from "./ToolDots.svelte";

  let {
    hit,
    compat = {},
    tools = [],
    detected = [],
    view = "grid",
    inStack = false,
    href = null,
    ontoggle,
    onopen,
  }: {
    hit: SearchHit;
    compat?: Record<string, Compat>;
    tools?: { id: string; name: string }[];
    detected?: string[];
    view?: "grid" | "list";
    inStack?: boolean;
    href?: string | null;
    ontoggle?: (hit: SearchHit) => void;
    onopen?: (hit: SearchHit) => void;
  } = $props();

  let source = $derived(hit.marketplace ?? (hit.source_id === "cct" ? "claude-code-templates" : hit.source_id));
  let seal = $derived(guardSeals[hit.id] ?? null);
  let link = $derived(href ?? itemHref(hit.id));

  function open(e: MouseEvent) {
    if (onopen) {
      e.preventDefault();
      onopen(hit);
    }
  }
</script>

<article class="card {view}" class:in-stack={inStack}>
  <a class="hit" href={link} onclick={open} aria-label={`${hit.name} — ${$t(`llm.central.catalog.kind.${hit.kind}`)}`}></a>
  <div class="top">
    <KindIcon kind={hit.kind} size={view === "grid" ? 36 : 28} />
    <div class="titles">
      <h3 class="name" title={hit.name}>{hit.name}</h3>
      <p class="sub" title={`${hit.category} · ${source}`}>
        <span>{$t(`llm.central.catalog.kind.${hit.kind}`)}</span>
        <span class="sep">·</span>
        <span>{hit.category}</span>
        {#if view === "list"}<span class="sep">·</span><span class="src">{source}</span>{/if}
      </p>
    </div>
    <button
      type="button"
      class="stack-btn"
      class:on={inStack}
      aria-pressed={inStack}
      title={inStack ? $t("llm.central.catalog.stack.remove") : $t("llm.central.catalog.stack.add")}
      aria-label={inStack ? $t("llm.central.catalog.stack.remove") : $t("llm.central.catalog.stack.add")}
      onclick={() => ontoggle?.(hit)}>{inStack ? "✓" : "+"}</button
    >
  </div>
  {#if hit.description}
    <p class="desc">{hit.description}</p>
  {/if}
  <footer class="foot">
    <ToolDots {tools} {compat} {detected} max={view === "grid" ? 7 : 12} />
    <span class="meta">
      {#if hit.stars}<span class="stars tabular" title={$t("llm.central.catalog.meta.stars")}>★ {compactNumber(hit.stars)}</span>{/if}
      {#if hit.license}<span class="lic" title={$t("llm.central.catalog.meta.license")}>{hit.license}</span>{:else}<span class="lic warn">{$t("llm.central.catalog.meta.no_license")}</span>{/if}
      {#if hit.collides}<span class="lic" title={$t("llm.central.catalog.meta.collides_hint")}>{$t("llm.central.catalog.meta.collides")}</span>{/if}
      <GuardBadge report={seal} compact />
    </span>
  </footer>
</article>

<style>
  .card {
    position: relative;
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    padding: var(--space-3);
    border-radius: var(--radius-lg);
    background: var(--surface);
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
    transition:
      box-shadow var(--duration-fast) var(--ease-out),
      transform var(--duration-fast) var(--ease-out);
    min-width: 0;
  }
  .card:hover {
    box-shadow:
      inset 0 0 0 var(--hairline) var(--border-hi),
      var(--elev-1);
  }
  .card.in-stack {
    box-shadow: inset 0 0 0 1.5px color-mix(in srgb, var(--accent) 70%, transparent);
  }
  .card.list {
    display: grid;
    grid-template-columns: minmax(220px, 1.2fr) minmax(0, 2fr) auto;
    align-items: center;
    padding: var(--space-2) var(--space-3);
    border-radius: var(--radius-md);
  }
  .hit {
    position: absolute;
    inset: 0;
    border-radius: inherit;
    z-index: 0;
  }
  .hit:focus-visible {
    outline: var(--focus-ring);
    outline-offset: var(--focus-ring-offset);
  }
  .top {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    min-width: 0;
  }
  .titles {
    flex: 1;
    min-width: 0;
  }
  .name {
    margin: 0;
    font-size: var(--text-md);
    font-weight: 600;
    letter-spacing: var(--track-snug);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .sub {
    margin: 0;
    display: flex;
    gap: 4px;
    font-size: var(--text-xs);
    color: var(--text-muted);
    overflow: hidden;
    white-space: nowrap;
  }
  .sub span {
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .sep {
    color: var(--text-faint);
    flex-shrink: 0;
  }
  .stack-btn {
    position: relative;
    z-index: 1;
    flex-shrink: 0;
    width: 28px;
    height: 28px;
    border: none;
    border-radius: var(--radius-full);
    background: var(--fill-1);
    color: var(--text);
    font-size: 16px;
    font-weight: 600;
    cursor: pointer;
  }
  .stack-btn:hover {
    background: var(--fill-2);
  }
  .stack-btn.on {
    background: var(--accent);
    color: var(--on-accent);
  }
  .stack-btn:focus-visible {
    outline: var(--focus-ring);
  }
  .desc {
    margin: 0;
    font-size: var(--text-sm);
    line-height: var(--leading-sm);
    color: var(--text-muted);
    display: -webkit-box;
    -webkit-line-clamp: 3;
    line-clamp: 3;
    -webkit-box-orient: vertical;
    overflow: hidden;
    flex: 1;
  }
  .list .desc {
    -webkit-line-clamp: 2;
    line-clamp: 2;
  }
  .foot {
    position: relative;
    z-index: 1;
    pointer-events: none;
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-2);
    flex-wrap: wrap;
  }
  .foot > :global(*) {
    pointer-events: auto;
  }
  .list .foot {
    justify-content: flex-end;
    flex-wrap: nowrap;
  }
  .meta {
    position: relative;
    z-index: 1;
    display: inline-flex;
    align-items: center;
    gap: var(--space-1);
    font-size: var(--text-xs);
    color: var(--text-muted);
  }
  .lic {
    padding: 1px 6px;
    border-radius: var(--radius-full);
    background: var(--fill-1);
    white-space: nowrap;
  }
  .lic.warn {
    color: var(--warning);
  }
  .stars {
    white-space: nowrap;
  }
  @media (max-width: 900px) {
    .card.list {
      grid-template-columns: 1fr;
    }
  }
</style>

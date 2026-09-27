<script lang="ts">
  /** Row of tool tiles for a component: detected tools first, then the rest up to `max`. */
  import { t } from "$lib/i18n";
  import type { Compat } from "$lib/central/catalog";
  import ToolDot from "./ToolDot.svelte";

  let {
    tools,
    compat = {},
    detected = [],
    max = 8,
    size = 18,
  }: {
    tools: { id: string; name: string }[];
    compat?: Record<string, Compat>;
    detected?: string[];
    max?: number;
    size?: number;
  } = $props();

  let ordered = $derived(
    [...tools].sort((a, b) => {
      const da = detected.includes(a.id) ? 0 : 1;
      const db = detected.includes(b.id) ? 0 : 1;
      if (da !== db) return da - db;
      const rank = (id: string) => ["native", "converted", "degraded", "unsupported"].indexOf(compat[id]?.status ?? "unsupported");
      return rank(a.id) - rank(b.id);
    }),
  );
  let shown = $derived(ordered.slice(0, max));
  let rest = $derived(ordered.length - shown.length);
  let works = $derived(tools.filter((x) => compat[x.id] && compat[x.id].status !== "unsupported").length);
</script>

<span class="dots" aria-label={$t("llm.central.catalog.compat.works_in", { count: String(works), total: String(tools.length) })}>
  {#each shown as tool (tool.id)}
    <ToolDot id={tool.id} name={tool.name} compat={compat[tool.id] ?? null} {size} detected={detected.includes(tool.id)} />
  {/each}
  {#if rest > 0}<span class="rest tabular" title={$t("llm.central.catalog.compat.works_in", { count: String(works), total: String(tools.length) })}>+{rest}</span>{/if}
</span>

<style>
  .dots {
    display: inline-flex;
    align-items: center;
    gap: 3px;
    flex-wrap: wrap;
  }
  .rest {
    font-size: var(--text-xs);
    color: var(--text-muted);
    margin-left: 2px;
  }
</style>

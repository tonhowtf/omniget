<script lang="ts">
  import { t } from "$lib/i18n";
  import { formatBytes } from "$lib/washmybelly/api";
  import type { Gain } from "$lib/washmybelly/monitor";

  // "What you can win now" cards. Takes every gain the core reports: free
  // space and battery today; junk and login items once the clean and
  // startup modules fill `score_inputs`.
  let { gains, onOpen }: { gains: Gain[]; onOpen: (module: string) => void } = $props();

  const GLYPH: Record<Gain["id"], string> = {
    junk: "M4 7h16M9 7V4h6v3m-8 0 1 13h8l1-13",
    free_space: "M3 6h18v12H3zM7 14h.01M11 14h6",
    login_items: "M12 3v10m0 0-4-4m4 4 4-4M4 17h16v4H4z",
    battery: "M3 8h15v8H3zM20 11v2M6 11h6",
  };

  function big(g: Gain): string {
    if (g.id === "junk" || (g.id === "free_space" && g.bytes !== undefined)) return formatBytes(g.bytes);
    if (g.id === "login_items") return String(g.count ?? 0);
    return g.percent !== undefined ? `${Math.round(g.percent)}%` : "—";
  }
</script>

{#if gains.length}
  <div class="gains">
    {#each gains as g (g.id)}
      <button class="gain {g.id}" onclick={() => onOpen(g.module)}>
        <svg width="22" height="22" viewBox="0 0 24 24" aria-hidden="true"><path d={GLYPH[g.id] ?? GLYPH.free_space} fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" /></svg>
        <span class="v">{big(g)}</span>
        <span class="l">{$t(`washmybelly.monitor.gains.${g.id}`)}</span>
        <span class="go">{$t(`washmybelly.monitor.gains.${g.id}_action`)} ›</span>
      </button>
    {/each}
  </div>
{/if}

<style>
  .gains { display: grid; grid-template-columns: repeat(auto-fill, minmax(180px, 1fr)); gap: 12px; }
  .gain { all: unset; cursor: pointer; display: flex; flex-direction: column; gap: 4px; padding: 12px 14px; border-radius: var(--radius-lg, 12px); background: var(--surface); box-shadow: inset 0 0 0 var(--hairline, 1px) var(--content-border, var(--separator)); }
  .gain:hover { background: var(--surface-hi, var(--surface)); }
  .gain:focus-visible { box-shadow: 0 0 0 2px var(--accent); }
  svg { color: var(--accent); }
  .junk svg { color: #e0527f; }
  .battery svg { color: #30d158; }
  .login_items svg { color: #f28500; }
  .v { font-size: 20px; font-weight: 600; font-variant-numeric: tabular-nums; }
  .l { font-size: 12px; color: var(--text-dim); }
  .go { font-size: 12px; color: var(--accent-text, var(--accent)); margin-top: 2px; }
</style>

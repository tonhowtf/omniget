<script lang="ts">
  // Context ring (20 px): tokens in the window after the last request / the
  // window size, red above 90 %. Hover or focus opens the details with the
  // driver's rate-limit windows and a Compact action when the driver has it.
  // A placeholder keeps the slot while data loads, so nothing shifts.
  import { t } from "$lib/i18n";
  import { formatTokens } from "$lib/central/threads-ui/format";

  let {
    used = null,
    max = null,
    estimated = false,
    limits = [],
    cancompact = false,
    oncompact,
  }: {
    used?: number | null;
    max?: number | null;
    estimated?: boolean;
    limits?: { id: string; label: string; usedPercent?: number; resetsAt?: string }[];
    cancompact?: boolean;
    oncompact?: () => void;
  } = $props();

  let open = $state(false);
  let timer: ReturnType<typeof setTimeout> | null = null;
  const R = 8;
  const C = 2 * Math.PI * R;
  let pct = $derived(used != null && max ? Math.min(1, used / max) : 0);
  let known = $derived(used != null);

  function show() {
    if (timer) clearTimeout(timer);
    timer = setTimeout(() => (open = true), 150);
  }

  function hide() {
    if (timer) clearTimeout(timer);
    timer = setTimeout(() => (open = false), 120);
  }
</script>

<!-- svelte-ignore a11y_no_static_element_interactions -->
<span class="meter-wrap" onmouseenter={show} onmouseleave={hide} onfocusin={show} onfocusout={hide}>
  <button
    type="button"
    class="meter"
    class:hot={pct > 0.9}
    aria-label={known ? ($t("llm.central.threads.context.aria", { pct: Math.round(pct * 100), used: formatTokens(used), max: max ? formatTokens(max) : "?" }) as string) : ($t("llm.central.threads.context.unknown") as string)}
    aria-expanded={open}
    onclick={() => (open = !open)}
  >
    <svg viewBox="0 0 20 20" width="20" height="20" aria-hidden="true">
      <circle cx="10" cy="10" r={R} class="track" />
      {#if known}
        <circle cx="10" cy="10" r={R} class="fill" stroke-dasharray={C} stroke-dashoffset={C * (1 - pct)} transform="rotate(-90 10 10)" />
      {/if}
    </svg>
  </button>
  {#if open}
    <div class="pop" role="dialog" aria-label={$t("llm.central.threads.context.title")}>
      <p class="title">
        {$t("llm.central.threads.context.title")}
        {#if known && max}<strong>{Math.round(pct * 100)}%</strong> · {formatTokens(used)}/{formatTokens(max)}{:else if known}<strong>{formatTokens(used)}</strong>{/if}
      </p>
      {#if known && max}<div class="bar"><span style:width="{pct * 100}%"></span></div>{/if}
      {#if estimated}<p class="muted">{$t("llm.central.threads.context.estimated")}</p>{/if}
      {#if !known}<p class="muted">{$t("llm.central.threads.context.unknown")}</p>{/if}
      {#if limits.length}
        <p class="sub">{$t("llm.central.threads.context.limits")}</p>
        {#each limits as l (l.id)}
          <div class="limit">
            <span>{l.label}</span>
            <span class="mono">{l.usedPercent != null ? `${Math.round(l.usedPercent)}%` : "—"}</span>
            {#if l.usedPercent != null}<div class="bar small"><span style:width="{Math.min(100, l.usedPercent)}%"></span></div>{/if}
            {#if l.resetsAt}<span class="muted reset">{$t("llm.central.threads.context.resets", { when: new Date(l.resetsAt).toLocaleString() })}</span>{/if}
          </div>
        {/each}
      {/if}
      <button type="button" class="button compact" disabled={!cancompact} title={cancompact ? "" : ($t("llm.central.threads.context.compact_unavailable") as string)} onclick={() => oncompact?.()}>
        {$t("llm.central.threads.context.compact")}
      </button>
    </div>
  {/if}
</span>

<style>
  .meter-wrap { position: relative; display: inline-flex; width: 28px; height: 28px; align-items: center; justify-content: center; }
  .meter { width: 28px; height: 28px; border: 0; background: transparent; border-radius: 50%; display: inline-flex; align-items: center; justify-content: center; cursor: pointer; padding: 0; }
  .meter:hover { background: var(--fill-1); }
  .track { fill: none; stroke: var(--fill-3, var(--separator)); stroke-width: 2.5; }
  .fill { fill: none; stroke: var(--text-muted); stroke-width: 2.5; stroke-linecap: round; transition: stroke-dashoffset 500ms var(--ease-out, ease); }
  .hot .fill { stroke: var(--red, #ef4444); }
  .pop { position: absolute; bottom: calc(100% + 8px); right: -8px; width: 260px; z-index: 30; padding: 12px; border-radius: 12px; background: var(--popup-bg, var(--surface)); border: 1px solid var(--separator); box-shadow: 0 12px 32px color-mix(in srgb, #000 22%, transparent); display: grid; gap: 8px; font-size: 12.5px; }
  .title { margin: 0; display: flex; gap: 6px; flex-wrap: wrap; font-weight: 600; }
  .title strong { font-variant-numeric: tabular-nums; }
  .muted { margin: 0; color: var(--text-muted); font-size: 11.5px; }
  .sub { margin: 4px 0 0; font-weight: 600; font-size: 11.5px; color: var(--text-muted); text-transform: uppercase; letter-spacing: 0.03em; }
  .bar { height: 5px; border-radius: 3px; background: var(--fill-2); overflow: hidden; }
  .bar span { display: block; height: 100%; background: var(--accent); border-radius: 3px; }
  .bar.small { grid-column: 1 / -1; height: 3px; }
  .limit { display: grid; grid-template-columns: 1fr auto; gap: 3px 8px; }
  .mono { font-family: var(--font-mono); font-size: 11.5px; }
  .reset { grid-column: 1 / -1; }
  .compact { justify-self: start; }
</style>

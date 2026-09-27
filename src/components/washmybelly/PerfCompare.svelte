<script lang="ts">
  import { t } from "$lib/i18n";
  import * as api from "$lib/washmybelly/api";
  import { errorText, formatTime } from "$lib/washmybelly/format";
  import { comparable, formatDelta, formatMetric, METRIC_KEYS, type Compare, type SessionSummary } from "$lib/washmybelly/perf";

  /// Before and after, metric by metric. A change smaller than how much the
  /// metric moves inside one capture is shown as "within noise". Delta chip
  /// colouring ported from the "Stats Card" of ravikatiyar162 (21st.dev).
  let { sessions }: { sessions: SessionSummary[] } = $props();

  let list = $derived(comparable(sessions));
  let before = $state("");
  let after = $state("");
  let result = $state<Compare | null>(null);
  let busy = $state(false);
  let error = $state<string | null>(null);

  $effect(() => {
    if (!after && list[0]) after = list[0].session_id;
    if (!before && list[1]) before = list[1].session_id;
  });

  async function run() {
    if (!before || !after) return;
    busy = true;
    error = null;
    try {
      result = await api.call<Compare>("wmb_perf_compare", { before, after });
    } catch (e) {
      error = errorText(e);
    } finally {
      busy = false;
    }
  }

  function name(s: SessionSummary): string {
    return `${s.label} · ${formatTime(Math.floor(s.started_ms / 1000), undefined, true)}`;
  }
</script>

{#if list.length < 2}
  <p class="dim">{$t("washmybelly.perf.compare_need_two")}</p>
{:else}
  <div class="pick">
    <label>
      <span>{$t("washmybelly.perf.before")}</span>
      <select bind:value={before}>
        {#each list as s (s.session_id)}<option value={s.session_id}>{name(s)}</option>{/each}
      </select>
    </label>
    <label>
      <span>{$t("washmybelly.perf.after")}</span>
      <select bind:value={after}>
        {#each list as s (s.session_id)}<option value={s.session_id}>{name(s)}</option>{/each}
      </select>
    </label>
    <button class="btn btn-secondary btn-sm" type="button" disabled={busy || !before || !after || before === after} onclick={run}>{$t("washmybelly.perf.compare")}</button>
  </div>
  {#if error}<p class="error" role="alert">{error}</p>{/if}
  {#if result}
    <p class="verdict v-{result.delta.verdict}">{$t(`washmybelly.perf.verdict.${result.delta.verdict}`)}</p>
    <ul class="rows">
      {#each result.delta.metrics as d (d.metric)}
        <li>
          <span class="m">{$t(`washmybelly.perf.${METRIC_KEYS[d.metric] ?? d.metric}`)}</span>
          <span class="v">{formatMetric(d.metric, d.before)}</span>
          <span class="arrow" aria-hidden="true">→</span>
          <span class="v">{formatMetric(d.metric, d.after)}</span>
          <span class="chip c-{d.verdict}" title={d.noise_pct == null ? "" : ($t("washmybelly.perf.noise_hint", { pct: d.noise_pct.toFixed(1) }) as string)}>
            {d.verdict === "same" ? $t("washmybelly.perf.within_noise") : formatDelta(d)}
          </span>
        </li>
      {/each}
    </ul>
    {#if result.warnings.length}
      <ul class="warnings">
        {#each result.warnings as w (w)}<li>{$t(`washmybelly.perf.compare_warning.${w}`)}</li>{/each}
      </ul>
    {/if}
  {/if}
{/if}

<style>
  .pick { display: flex; flex-wrap: wrap; align-items: flex-end; gap: 10px; }
  label { display: flex; flex-direction: column; gap: 3px; font-size: 11px; color: var(--text-dim); flex: 1; min-width: 180px; }
  select { font: inherit; font-size: 12.5px; color: var(--text); padding: 4px 6px; border-radius: 7px; border: var(--hairline, 1px) solid var(--content-border, var(--separator)); background: var(--surface); }
  .dim { margin: 0; font-size: 12px; color: var(--text-dim); }
  .error { margin: 0; font-size: 12px; color: var(--danger); }
  .verdict { margin: 0; font-size: 13px; font-weight: 600; }
  .v-better { color: var(--success); }
  .v-worse { color: var(--danger); }
  .v-mixed { color: var(--warning); }
  .v-same { color: var(--text-dim); }
  .rows { list-style: none; margin: 0; padding: 0; }
  .rows li { display: grid; grid-template-columns: 1fr auto auto auto 110px; align-items: center; gap: 10px; padding: 7px 0; border-bottom: var(--hairline, 1px) solid var(--content-border, var(--separator)); font-size: 12.5px; }
  .rows li:last-child { border-bottom: none; }
  .m { color: var(--text); }
  .v { font-variant-numeric: tabular-nums; text-align: right; min-width: 56px; }
  .arrow { color: var(--text-dim); }
  .chip { justify-self: end; padding: 2px 8px; border-radius: 999px; font-size: 11.5px; font-weight: 600; font-variant-numeric: tabular-nums; background: var(--surface-mut, var(--separator)); color: var(--text-dim); }
  .c-better { background: color-mix(in srgb, var(--success) 16%, transparent); color: var(--success); }
  .c-worse { background: color-mix(in srgb, var(--danger) 16%, transparent); color: var(--danger); }
  .warnings { margin: 0; padding-left: 18px; font-size: 11.5px; color: var(--text-dim); }
  @media (max-width: 560px) { .rows li { grid-template-columns: 1fr auto auto; } .arrow, .rows li .v:first-of-type { display: none; } }
</style>

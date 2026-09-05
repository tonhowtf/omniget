<script lang="ts">
  /** Quanto gastei com IA (estudo 17): ledger local por dia, modelo e tarefa. */
  import { invoke } from "@tauri-apps/api/core";
  import { onMount } from "svelte";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import { errText, fmtUsd, reveal } from "$lib/tools/rt";

  type Bucket = { key: string; calls: number; input_tokens: number; output_tokens: number; cost_usd: number; unknown_price: number };
  type Report = { since: string; calls: number; input_tokens: number; output_tokens: number; cost_usd: number; unknown_price: number; by_day: Bucket[]; by_model: Bucket[]; by_task: Bucket[]; entries_path: string | null };

  let days = $state(30);
  let report = $state<Report | null>(null);
  let busy = $state(false);

  async function load() {
    busy = true;
    try {
      report = await invoke<Report>("tool_usage_report", { days });
    } catch (e) {
      showToast("error", errText(e));
    } finally {
      busy = false;
    }
  }

  async function clear() {
    if (!confirm($t("tools.usage.clear_confirm") as string)) return;
    try {
      await invoke("tool_usage_clear");
      await load();
    } catch (e) {
      showToast("error", errText(e));
    }
  }

  function k(n: number): string {
    return n >= 1e6 ? `${(n / 1e6).toFixed(1)}M` : n >= 1e3 ? `${(n / 1e3).toFixed(1)}k` : String(n);
  }

  let maxDay = $derived(Math.max(1, ...(report?.by_day.map((b) => b.cost_usd) ?? [1])));

  onMount(load);
</script>

<div class="tool">
  <section>
    <div class="group">
      <div class="group-row">
        <div class="group-row-content">
          <div class="group-row-title">{report ? fmtUsd(report.cost_usd) : "…"} <span class="dim">· {report?.calls ?? 0} {$t("tools.usage.calls")} · {k(report?.input_tokens ?? 0)} in / {k(report?.output_tokens ?? 0)} out</span></div>
          <div class="group-row-sub">{$t("tools.usage.intro")}{#if report?.unknown_price} · {report.unknown_price} {$t("tools.usage.unknown_price")}{/if}</div>
        </div>
        <div class="group-row-trailing btn-row">
          <div class="segmented">
            {#each [7, 30, 90, 365] as d (d)}<button class="segmented-btn" class:active={days === d} type="button" onclick={() => { days = d; load(); }}>{d}d</button>{/each}
          </div>
          <button class="btn btn-secondary btn-sm" type="button" disabled={busy} onclick={load}>{$t("tools.common.refresh")}</button>
        </div>
      </div>
    </div>
  </section>

  {#if report && report.by_day.length}
    <section>
      <span class="group-label">{$t("tools.usage.by_day")}</span>
      <div class="group"><div class="group-row"><div class="bars">
        {#each report.by_day as b (b.key)}
          <div class="bar" title="{b.key}: {fmtUsd(b.cost_usd)} · {b.calls}"><div class="bar-fill" style:height="{Math.max(2, (b.cost_usd / maxDay) * 100)}%"></div><span class="bar-label">{b.key.slice(5)}</span></div>
        {/each}
      </div></div></div>
    </section>
  {/if}

  {#if report}
    <section>
      <span class="group-label">{$t("tools.usage.by_model")}</span>
      <div class="group">
        {#each report.by_model as b (b.key)}
          <div class="group-row">
            <div class="group-row-content"><div class="group-row-title mono">{b.key}</div><div class="group-row-sub">{b.calls} {$t("tools.usage.calls")} · {k(b.input_tokens)} in / {k(b.output_tokens)} out</div></div>
            <div class="group-row-trailing"><strong>{fmtUsd(b.cost_usd)}</strong></div>
          </div>
        {/each}
        {#if report.by_model.length === 0}<div class="group-row"><div class="group-row-sub">{$t("tools.usage.empty")}</div></div>{/if}
      </div>
    </section>
    <section>
      <span class="group-label">{$t("tools.usage.by_task")}</span>
      <div class="group">
        {#each report.by_task as b (b.key)}
          <div class="group-row">
            <div class="group-row-content"><div class="group-row-title">{b.key}</div><div class="group-row-sub">{b.calls} {$t("tools.usage.calls")}</div></div>
            <div class="group-row-trailing"><strong>{fmtUsd(b.cost_usd)}</strong></div>
          </div>
        {/each}
        <div class="group-row">
          <div class="group-row-content"><div class="group-row-sub mono">{report.entries_path}</div></div>
          <div class="group-row-trailing btn-row">
            {#if report.entries_path}<button class="btn btn-ghost btn-sm" type="button" onclick={() => reveal(report!.entries_path!)}>{$t("tools.common.reveal")}</button>{/if}
            <button class="btn btn-destructive btn-sm" type="button" onclick={clear}>{$t("tools.usage.clear")}</button>
          </div>
        </div>
      </div>
    </section>
  {/if}
</div>

<style>
  .tool { display: flex; flex-direction: column; gap: var(--space-5); }
  .dim { color: var(--text-dim); font-weight: 400; }
  .mono { font-family: var(--font-mono); font-size: var(--text-xs); overflow-wrap: anywhere; }
  .segmented-btn.active { background: var(--surface-hi); color: var(--text); }
  .bars { display: flex; align-items: flex-end; gap: 4px; height: 96px; width: 100%; overflow-x: auto; }
  .bar { display: flex; flex-direction: column; align-items: center; justify-content: flex-end; min-width: 22px; height: 100%; }
  .bar-fill { width: 100%; background: var(--accent); border-radius: 3px 3px 0 0; }
  .bar-label { font-size: 9px; color: var(--text-dim); margin-top: 2px; }
</style>

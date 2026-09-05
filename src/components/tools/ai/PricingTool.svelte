<script lang="ts">
  /** Comparar preços de modelos (estudos 14, 15): tabela do LiteLLM, por milhão de tokens. */
  import { invoke } from "@tauri-apps/api/core";
  import { onMount } from "svelte";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import { errText } from "$lib/tools/rt";

  type Price = {
    key: string; provider: string; mode: string; input_per_m: number | null; output_per_m: number | null; cache_read_per_m: number | null;
    max_input_tokens: number | null; max_output_tokens: number | null; input_per_second: number | null; input_per_character: number | null;
    supports_vision: boolean; supports_tools: boolean; supports_reasoning: boolean; supports_caching: boolean; deprecation_date: string | null;
  };
  type Info = { models: number; updated_at: string | null; path: string | null };

  let info = $state<Info | null>(null);
  let query = $state("");
  let mode = $state("chat");
  let rows = $state<Price[]>([]);
  let busy = $state(false);
  let timer: ReturnType<typeof setTimeout> | null = null;
  let inTok = $state(100000);
  let outTok = $state(20000);

  async function load(force = false) {
    busy = true;
    try {
      info = await invoke<Info>("tool_pricing_info", { force });
      await search();
    } catch (e) {
      showToast("error", errText(e));
    } finally {
      busy = false;
    }
  }

  async function search() {
    try {
      rows = await invoke<Price[]>("tool_pricing_search", { query, mode: mode === "all" ? "" : mode, limit: 80 });
    } catch (e) {
      showToast("error", errText(e));
    }
  }

  function onInput() {
    if (timer) clearTimeout(timer);
    timer = setTimeout(search, 200);
  }

  function money(v: number | null): string {
    if (v === null) return "—";
    if (v === 0) return "$0";
    return `$${v < 1 ? v.toFixed(3) : v.toFixed(2)}`;
  }

  function estimate(p: Price): string {
    if (p.input_per_m === null) return "—";
    const c = (p.input_per_m / 1e6) * inTok + ((p.output_per_m ?? 0) / 1e6) * outTok;
    return c === 0 ? "$0" : `$${c < 0.01 ? c.toFixed(4) : c.toFixed(2)}`;
  }

  function ctx(n: number | null): string {
    if (!n) return "—";
    return n >= 1000 ? `${Math.round(n / 1000)}k` : String(n);
  }

  onMount(() => load(false));
</script>

<div class="tool">
  <section>
    <div class="group">
      <div class="group-row">
        <div class="group-row-content">
          <input class="input input-search" type="search" bind:value={query} oninput={onInput} placeholder={$t("tools.pricing.search")} />
        </div>
        <div class="group-row-trailing btn-row">
          <select class="input" bind:value={mode} onchange={search}>
            <option value="chat">chat</option><option value="embedding">embedding</option><option value="audio_transcription">transcription</option><option value="audio_speech">tts</option><option value="image_generation">image</option><option value="all">{$t("tools.pricing.all")}</option>
          </select>
          <button class="btn btn-secondary btn-sm" type="button" disabled={busy} onclick={() => load(true)}>{$t("tools.common.refresh")}</button>
        </div>
      </div>
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-sub">{$t("tools.pricing.estimate_for")}</div></div>
        <div class="group-row-trailing btn-row">
          <input class="input" type="number" min="0" step="1000" bind:value={inTok} style:width="8em" /> <span class="dim">in</span>
          <input class="input" type="number" min="0" step="1000" bind:value={outTok} style:width="8em" /> <span class="dim">out</span>
        </div>
      </div>
      <div class="group-row"><div class="group-row-sub">{info ? `${info.models} ${$t("tools.pricing.models")} · ${info.updated_at ? new Date(info.updated_at).toLocaleString() : ""}` : "…"} · {$t("tools.pricing.source")}</div></div>
    </div>
  </section>
  <section>
    <div class="table-wrap">
      <table class="table">
        <thead><tr><th>{$t("tools.pricing.model")}</th><th>{$t("tools.pricing.provider")}</th><th class="num">in /1M</th><th class="num">out /1M</th><th class="num">cache /1M</th><th class="num">ctx</th><th>{$t("tools.pricing.caps")}</th><th class="num">{$t("tools.pricing.estimate")}</th></tr></thead>
        <tbody>
          {#each rows as r (r.key)}
            <tr>
              <td class="mono">{r.key}{#if r.deprecation_date} <span class="tag tag-warning" title={r.deprecation_date}>dep</span>{/if}</td>
              <td>{r.provider}</td>
              <td class="num">{r.input_per_second !== null ? `${money(r.input_per_second * 60)}/min` : r.input_per_character !== null ? `${money(r.input_per_character * 1e6)}/1M chars` : money(r.input_per_m)}</td>
              <td class="num">{money(r.output_per_m)}</td>
              <td class="num">{money(r.cache_read_per_m)}</td>
              <td class="num">{ctx(r.max_input_tokens)}</td>
              <td class="caps">{r.supports_tools ? "🛠" : ""}{r.supports_vision ? "👁" : ""}{r.supports_reasoning ? "🧠" : ""}{r.supports_caching ? "💾" : ""}</td>
              <td class="num">{estimate(r)}</td>
            </tr>
          {/each}
          {#if rows.length === 0}<tr><td colspan="8" class="dim">{$t("tools.hub.empty_title")}</td></tr>{/if}
        </tbody>
      </table>
    </div>
  </section>
</div>

<style>
  .tool { display: flex; flex-direction: column; gap: var(--space-4); }
  .dim { color: var(--text-dim); }
  .mono { font-family: var(--font-mono); font-size: var(--text-xs); }
  .table-wrap { overflow-x: auto; border-radius: var(--radius-lg); background: var(--surface); box-shadow: inset 0 0 0 var(--hairline) var(--content-border); }
  .table { width: 100%; border-collapse: collapse; font-size: var(--text-sm); }
  .table th, .table td { padding: 6px 10px; text-align: left; border-bottom: var(--hairline) solid var(--content-border); white-space: nowrap; }
  .table th { color: var(--text-dim); font-weight: 500; font-size: var(--text-xs); }
  .num { text-align: right !important; font-variant-numeric: tabular-nums; }
  .caps { letter-spacing: 2px; }
</style>

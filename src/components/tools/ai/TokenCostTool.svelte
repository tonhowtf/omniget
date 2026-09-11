<script lang="ts">
  /** Contar tokens e calcular custo por modelo. O numero e sempre estimativa:
   *  contagem exata exigiria o tokenizador de cada familia. A UI diz isso. */
  import { invoke } from "@tauri-apps/api/core";
  import { onMount } from "svelte";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import { baseName, errText, fmtUsd, pickDir, pickFiles } from "$lib/tools/rt";

  type FileCount = { path: string; bytes: number; chars: number; tokens: number };
  type ModelCost = {
    model: string; matched: string; provider: string; family: string; family_label: string;
    input_tokens: number; input_low: number; input_high: number; output_tokens: number;
    input_usd: number | null; output_usd: number | null; total_usd: number | null;
    total_low_usd: number | null; total_high_usd: number | null;
    cached_input_usd: number | null; cache_write_usd: number | null;
    max_input_tokens: number | null; fits_context: boolean | null;
  };
  type Report = {
    chars: number; bytes: number; words: number; lines: number;
    files: FileCount[]; files_read: number; files_skipped: number;
    tokens: number; tokens_low: number; tokens_high: number;
    margin_pct: number; estimated: boolean; calls: number;
    models: ModelCost[]; missing: string[];
  };

  let text = $state("");
  let paths = $state<string[]>([]);
  let extensions = $state("");
  let outputTokens = $state(1000);
  let calls = $state(1);
  let models = $state<string[]>([]);
  let modelQuery = $state("");
  let report = $state<Report | null>(null);
  let busy = $state(false);

  const has = $derived(text.trim().length > 0 || paths.length > 0);

  async function addFiles() {
    const picked = await pickFiles();
    if (picked.length) paths = [...paths, ...picked];
  }

  async function addDir() {
    const dir = await pickDir();
    if (dir) paths = [...paths, dir];
  }

  function removePath(p: string) {
    paths = paths.filter((x) => x !== p);
  }

  function addModel() {
    const m = modelQuery.trim();
    if (!m || models.includes(m)) return;
    models = [...models, m];
    modelQuery = "";
  }

  function removeModel(m: string) {
    models = models.filter((x) => x !== m);
  }

  async function run() {
    if (!has) return;
    busy = true;
    try {
      report = await invoke<Report>("tool_tokens_cost", {
        opts: {
          text,
          paths,
          extensions,
          models,
          output_tokens: Math.max(0, Math.round(outputTokens)),
          calls: Math.max(1, Math.round(calls)),
        },
      });
      if (report.missing.length) showToast("info", `${$t("tools.tokencost.missing")}: ${report.missing.join(", ")}`);
    } catch (e) {
      showToast("error", errText(e));
    } finally {
      busy = false;
    }
  }

  function k(n: number): string {
    return n >= 1e6 ? `${(n / 1e6).toFixed(2)}M` : n >= 1e3 ? `${(n / 1e3).toFixed(1)}k` : String(n);
  }

  function usd(v: number | null): string {
    return v === null ? "—" : fmtUsd(v);
  }

  onMount(() => {
    invoke<string[]>("tool_tokens_default_models")
      .then((m) => { if (!models.length) models = m; })
      .catch(() => {});
  });
</script>

<div class="tool">
  <section>
    <div class="group">
      <div class="group-row">
        <div class="group-row-content">
          <div class="group-row-title">{$t("tools.tokencost.estimate_badge")}</div>
          <div class="group-row-sub">{$t("tools.tokencost.estimate_note")}</div>
        </div>
      </div>
    </div>
  </section>

  <section>
    <textarea class="input area" bind:value={text} placeholder={$t("tools.tokencost.paste")}></textarea>
    <div class="group">
      <div class="group-row">
        <div class="group-row-content">
          <div class="group-row-title">{$t("tools.tokencost.files")}</div>
          <div class="group-row-sub">{paths.length ? `${paths.length} ${$t("tools.tokencost.selected")}` : $t("tools.tokencost.files_sub")}</div>
        </div>
        <div class="group-row-trailing btn-row">
          <button class="btn btn-secondary btn-sm" type="button" onclick={addFiles}>{$t("tools.tokencost.add_files")}</button>
          <button class="btn btn-secondary btn-sm" type="button" onclick={addDir}>{$t("tools.tokencost.add_folder")}</button>
        </div>
      </div>
      {#each paths as p (p)}
        <div class="group-row">
          <div class="group-row-content"><div class="group-row-sub mono">{p}</div></div>
          <div class="group-row-trailing"><button class="btn btn-ghost btn-sm" type="button" onclick={() => removePath(p)}>{$t("tools.common.remove")}</button></div>
        </div>
      {/each}
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-sub">{$t("tools.tokencost.extensions")}</div></div>
        <div class="group-row-trailing"><input class="input" bind:value={extensions} placeholder="md, rs, ts" /></div>
      </div>
    </div>
  </section>

  <section>
    <div class="group">
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-sub">{$t("tools.tokencost.output_tokens")}</div></div>
        <div class="group-row-trailing btn-row">
          <input class="input" type="number" min="0" step="500" bind:value={outputTokens} style:width="8em" />
          <span class="dim">×</span>
          <input class="input" type="number" min="1" step="1" bind:value={calls} style:width="6em" />
          <span class="dim">{$t("tools.tokencost.calls")}</span>
        </div>
      </div>
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-sub">{$t("tools.tokencost.models")}</div></div>
        <div class="group-row-trailing btn-row">
          <input class="input" bind:value={modelQuery} placeholder="gpt-4o" onkeydown={(e) => { if (e.key === "Enter") addModel(); }} />
          <button class="btn btn-secondary btn-sm" type="button" onclick={addModel}>{$t("tools.common.add")}</button>
        </div>
      </div>
      <div class="group-row">
        <div class="group-row-content chips">
          {#each models as m (m)}
            <button class="tag" type="button" onclick={() => removeModel(m)}>{m} ×</button>
          {/each}
        </div>
      </div>
      <div class="group-row">
        <div class="group-row-content"></div>
        <div class="group-row-trailing"><button class="btn btn-primary" type="button" disabled={busy || !has} onclick={run}>{$t("tools.tokencost.run")}</button></div>
      </div>
    </div>
  </section>

  {#if report}
    <section>
      <div class="group">
        <div class="group-row">
          <div class="group-row-content">
            <div class="group-row-title">~{k(report.tokens)} {$t("tools.tokencost.tokens")} <span class="dim">({k(report.tokens_low)} – {k(report.tokens_high)}, ±{report.margin_pct}%)</span></div>
            <div class="group-row-sub">{k(report.chars)} {$t("tools.tokencost.chars")} · {k(report.words)} {$t("tools.tokencost.words")}{#if report.files_read} · {report.files_read} {$t("tools.tokencost.files_read")}{/if}{#if report.files_skipped} · {report.files_skipped} {$t("tools.tokencost.files_skipped")}{/if}</div>
          </div>
        </div>
      </div>
    </section>

    <section>
      <div class="table-wrap">
        <table class="table">
          <thead>
            <tr>
              <th>{$t("tools.tokencost.model")}</th>
              <th>{$t("tools.tokencost.family")}</th>
              <th class="num">{$t("tools.tokencost.in_tokens")}</th>
              <th class="num">{$t("tools.tokencost.in_usd")}</th>
              <th class="num">{$t("tools.tokencost.out_usd")}</th>
              <th class="num">{$t("tools.tokencost.cached")}</th>
              <th class="num">{$t("tools.tokencost.total")}</th>
              <th class="num">{$t("tools.tokencost.range")}</th>
            </tr>
          </thead>
          <tbody>
            {#each report.models as m (m.model)}
              <tr>
                <td class="mono">{m.matched}{#if m.fits_context === false} <span class="tag tag-warning">{$t("tools.tokencost.over_context")}</span>{/if}</td>
                <td class="dim">{m.family}</td>
                <td class="num">~{k(m.input_tokens)}</td>
                <td class="num">{usd(m.input_usd)}</td>
                <td class="num">{usd(m.output_usd)}</td>
                <td class="num">{usd(m.cached_input_usd)}</td>
                <td class="num">{usd(m.total_usd)}</td>
                <td class="num dim">{usd(m.total_low_usd)} – {usd(m.total_high_usd)}</td>
              </tr>
            {/each}
            {#if report.models.length === 0}
              <tr><td colspan="8" class="dim">{$t("tools.tokencost.no_models")}</td></tr>
            {/if}
          </tbody>
        </table>
      </div>
    </section>

    {#if report.files.length}
      <section>
        <div class="group">
          {#each report.files.slice(0, 50) as f (f.path)}
            <div class="group-row">
              <div class="group-row-content"><div class="group-row-sub mono">{baseName(f.path)}</div></div>
              <div class="group-row-trailing dim">~{k(f.tokens)}</div>
            </div>
          {/each}
        </div>
      </section>
    {/if}
  {/if}
</div>

<style>
  .tool { display: flex; flex-direction: column; gap: var(--space-4); }
  .dim { color: var(--text-dim); }
  .mono { font-family: var(--font-mono); font-size: var(--text-xs); overflow-wrap: anywhere; }
  .area { min-height: 160px; resize: vertical; width: 100%; font-family: var(--font-mono); font-size: var(--text-xs); }
  .chips { display: flex; flex-wrap: wrap; gap: 6px; }
  .table-wrap { overflow-x: auto; border-radius: var(--radius-lg); background: var(--surface); box-shadow: inset 0 0 0 var(--hairline) var(--content-border); }
  .table { width: 100%; border-collapse: collapse; font-size: var(--text-sm); }
  .table th, .table td { padding: 6px 10px; text-align: left; border-bottom: var(--hairline) solid var(--content-border); white-space: nowrap; }
  .table th { color: var(--text-dim); font-weight: 500; font-size: var(--text-xs); }
  .num { text-align: right !important; font-variant-numeric: tabular-nums; }
</style>

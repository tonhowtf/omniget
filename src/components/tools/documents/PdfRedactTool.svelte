<script lang="ts">
  /** Tarja de verdade: apaga o texto e só depois pinta a barra preta. */
  import { invoke } from "@tauri-apps/api/core";
  import { onDestroy, onMount } from "svelte";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import { baseName, errText, fmtBytes, onToolProgress, openPath, pct, pickFile, reveal, type ToolProgress } from "$lib/tools/rt";

  type Area = { page: number; x: number; y: number; width: number; height: number };
  type PageReport = { page: number; areas: number; chars_removed: number; mode: string; note: string };
  type Result = {
    input: string;
    output: string;
    pages: number;
    areas: number;
    chars_removed: number;
    terms_found: number;
    leftovers: number;
    verified: boolean;
    bytes: number;
    by_page: PageReport[];
  };

  const PDF = [{ name: "PDF", extensions: ["pdf"] }];

  let input = $state("");
  let termsText = $state("");
  let pages = $state("");
  let areas = $state<Area[]>([]);
  let padding = $state(1);
  let bar = $state(true);
  let mode = $state("text");
  let busy = $state(false);
  let progress = $state<ToolProgress | null>(null);
  let result = $state<Result | null>(null);
  let unlisten: (() => void) | undefined;

  const terms = $derived(termsText.split("\n").map((s) => s.trim()).filter(Boolean));
  const ready = $derived(!!input && (terms.length > 0 || areas.length > 0));

  onMount(async () => {
    unlisten = await onToolProgress((p) => {
      if (p.id === "pdf-redact") progress = p;
    });
  });
  onDestroy(() => unlisten?.());

  function addArea() {
    areas = [...areas, { page: 1, x: 50, y: 500, width: 200, height: 20 }];
  }

  async function run() {
    if (!ready || busy) return;
    busy = true;
    result = null;
    progress = null;
    try {
      result = await invoke<Result>("tool_pdf_redact", {
        opts: { input, terms, areas, pages, padding, bar, mode, dpi: 150, quality: 88 },
      });
      showToast(result.verified ? "success" : "error", (result.verified ? $t("tools.redact.verified") : $t("tools.redact.not_verified")) as string);
    } catch (e) {
      showToast("error", errText(e));
    } finally {
      busy = false;
    }
  }
</script>

<div class="tool">
  <section>
    <div class="group">
      <div class="group-row">
        <div class="group-row-content">
          <div class="group-row-title">{input ? baseName(input) : $t("tools.redact.file")}</div>
          <div class="group-row-sub">{$t("tools.redact.file_sub")}</div>
        </div>
        <div class="group-row-trailing">
          <button class="btn btn-secondary btn-sm" type="button" onclick={async () => { const f = await pickFile(PDF); if (f) { input = f; result = null; } }}>{$t("tools.common.choose")}</button>
        </div>
      </div>
      <div class="group-row">
        <div class="group-row-content">
          <div class="group-row-title">{$t("tools.redact.terms")}</div>
          <div class="group-row-sub">{$t("tools.redact.terms_sub")}</div>
          <textarea class="input area" rows="4" bind:value={termsText} placeholder={$t("tools.redact.terms_ph") as string}></textarea>
        </div>
      </div>
      <div class="group-row">
        <div class="group-row-content">
          <div class="group-row-title">{$t("tools.redact.pages")}</div>
          <div class="group-row-sub">{$t("tools.redact.pages_sub")}</div>
        </div>
        <div class="group-row-trailing"><input class="input mono" type="text" placeholder="1-5, 8" bind:value={pages} style:width="9em" /></div>
      </div>
    </div>
  </section>

  <section>
    <div class="group">
      <div class="group-row">
        <div class="group-row-content">
          <div class="group-row-title">{$t("tools.redact.areas")}</div>
          <div class="group-row-sub">{$t("tools.redact.areas_sub")}</div>
        </div>
        <div class="group-row-trailing"><button class="btn btn-secondary btn-sm" type="button" onclick={addArea}>{$t("tools.redact.add_area")}</button></div>
      </div>
      {#each areas as a, i (i)}
        <div class="group-row">
          <div class="group-row-content">
            <div class="group-row-sub grid">
              <label>{$t("tools.redact.page")}<input class="input mono" type="number" min="1" bind:value={a.page} /></label>
              <label>x<input class="input mono" type="number" bind:value={a.x} /></label>
              <label>y<input class="input mono" type="number" bind:value={a.y} /></label>
              <label>{$t("tools.redact.w")}<input class="input mono" type="number" bind:value={a.width} /></label>
              <label>{$t("tools.redact.h")}<input class="input mono" type="number" bind:value={a.height} /></label>
            </div>
          </div>
          <div class="group-row-trailing"><button class="btn btn-ghost btn-sm" type="button" onclick={() => (areas = areas.filter((_, j) => j !== i))}>×</button></div>
        </div>
      {/each}
    </div>
  </section>

  <section>
    <div class="group">
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{$t("tools.redact.bar")}</div><div class="group-row-sub">{$t("tools.redact.bar_sub")}</div></div>
        <div class="group-row-trailing"><button class="toggle" class:on={bar} type="button" role="switch" aria-checked={bar} aria-label={$t("tools.redact.bar") as string} onclick={() => (bar = !bar)}><span class="toggle-knob"></span></button></div>
      </div>
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{$t("tools.redact.mode")}</div><div class="group-row-sub">{$t("tools.redact.mode_sub")}</div></div>
        <div class="group-row-trailing">
          <select class="input" bind:value={mode}>
            <option value="text">{$t("tools.redact.mode_text")}</option>
            <option value="raster">{$t("tools.redact.mode_raster")}</option>
          </select>
        </div>
      </div>
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{$t("tools.redact.padding")} <span class="dim">· {padding} pt</span></div></div>
        <div class="group-row-trailing"><input type="range" min="0" max="8" step="1" bind:value={padding} /></div>
      </div>
      <div class="group-row">
        <div class="group-row-content">
          {#if busy}
            <div class="group-row-sub mono">{progress?.message ?? "…"}</div>
            <div class="progress"><div class="progress-fill" style:width="{pct(progress) ?? 0}%"></div></div>
          {/if}
        </div>
        <div class="group-row-trailing"><button class="btn btn-primary" type="button" disabled={busy || !ready} onclick={run}>{busy ? $t("tools.common.working") : $t("tools.redact.run")}</button></div>
      </div>
    </div>
  </section>

  {#if result}
    <section>
      <div class="group">
        <div class="group-row">
          <div class="group-row-content">
            <div class="group-row-title">
              {#if result.verified}<span class="tag tag-success">{$t("tools.redact.verified")}</span>{:else}<span class="tag tag-danger">{$t("tools.redact.not_verified")}</span>{/if}
            </div>
            <div class="group-row-sub">
              {result.chars_removed} {$t("tools.redact.chars")} · {result.areas} {$t("tools.redact.areas_done")}
              {#if result.terms_found}· {result.terms_found} {$t("tools.redact.hits")}{/if}
              <span class="dim"> · {fmtBytes(result.bytes)}</span>
            </div>
          </div>
          <div class="group-row-trailing btn-row">
            <button class="btn btn-ghost btn-sm" type="button" onclick={() => openPath(result!.output)}>{$t("tools.common.open")}</button>
            <button class="btn btn-secondary btn-sm" type="button" onclick={() => reveal(result!.output)}>{$t("tools.common.reveal")}</button>
          </div>
        </div>
        {#each result.by_page as p (p.page)}
          <div class="group-row">
            <div class="group-row-content">
              <div class="group-row-title">{$t("tools.redact.page")} {p.page} <span class="tag">{p.mode === "text" ? $t("tools.redact.mode_text") : $t("tools.redact.mode_raster")}</span></div>
              <div class="group-row-sub">{p.note}</div>
            </div>
          </div>
        {/each}
      </div>
    </section>
  {/if}
</div>

<style>
  .tool { display: flex; flex-direction: column; gap: var(--space-5); }
  .dim { color: var(--text-dim); font-weight: 400; }
  .mono { font-family: var(--font-mono); font-size: var(--text-xs); overflow-wrap: anywhere; }
  .area { width: 100%; margin-top: var(--space-2); resize: vertical; font-family: var(--font-mono); font-size: var(--text-xs); }
  .grid { display: flex; flex-wrap: wrap; gap: var(--space-2); align-items: center; }
  .grid label { display: flex; align-items: center; gap: var(--space-1); }
  .grid input { width: 5.5em; }
</style>

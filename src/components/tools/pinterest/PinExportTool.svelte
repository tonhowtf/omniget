<script lang="ts">
  /**
   * Exportar (estudo 67): galeria HTML offline com busca e seções (o board
   * sem "More ideas"), PDF um pin por página, CSV/JSON para planilha ou
   * migração (Eagle, Notion, Are.na).
   */
  import { invoke } from "@tauri-apps/api/core";
  import { onDestroy, onMount } from "svelte";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import { errText, onToolProgress, openPath, pct, pickDir, reveal, type ToolProgress } from "$lib/tools/rt";
  import { defaultFilters, loadCookies, type ExportOut } from "$lib/tools/pinterest";
  import PinCookies from "./PinCookies.svelte";
  import PinFilters from "./PinFilters.svelte";

  let url = $state("");
  let cookies = $state(loadCookies());
  let format = $state<"html" | "pdf" | "csv" | "json">("html");
  let offline = $state(true);
  let dest = $state("");
  let limit = $state(0);
  let filters = $state({ ...defaultFilters(), ai_level: 0 });
  let busy = $state(false);
  let out = $state<ExportOut | null>(null);
  let progress = $state<ToolProgress | null>(null);
  let unlisten: (() => void) | null = null;

  onMount(async () => { unlisten = await onToolProgress((p) => { if (p.id === "pinterest:export") progress = p; }); });
  onDestroy(() => unlisten?.());

  async function run() {
    if (!url.trim() || busy) return;
    if (!dest) { const d = await pickDir(); if (!d) return; dest = d; }
    busy = true; out = null; progress = null;
    try {
      out = await invoke<ExportOut>("tool_pin_export", { opts: { url, dest, format, cookies: cookies || null, limit, filters, offline } });
      showToast("success", `${out.pins} ${$t("tools.pinterest.pins")}`);
    } catch (e) { showToast("error", errText(e)); } finally { busy = false; }
  }
</script>

<div class="tool">
  <section>
    <div class="group">
      <div class="group-row">
        <div class="group-row-content"><input class="input" type="text" bind:value={url} placeholder={$t("tools.pinterest.board_placeholder")} onkeydown={(e) => e.key === "Enter" && run()} /></div>
      </div>
      <PinCookies bind:value={cookies} />
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{$t("tools.pinterest.format")}</div></div>
        <div class="group-row-trailing">
          <div class="segmented">
            <button class="segmented-btn" class:active={format === "html"} type="button" onclick={() => (format = "html")}>{$t("tools.pinterest.fmt_html")}</button>
            <button class="segmented-btn" class:active={format === "pdf"} type="button" onclick={() => (format = "pdf")}>{$t("tools.pinterest.fmt_pdf")}</button>
            <button class="segmented-btn" class:active={format === "csv"} type="button" onclick={() => (format = "csv")}>{$t("tools.pinterest.fmt_csv")}</button>
            <button class="segmented-btn" class:active={format === "json"} type="button" onclick={() => (format = "json")}>{$t("tools.pinterest.fmt_json")}</button>
          </div>
        </div>
      </div>
      {#if format === "html"}
        <div class="group-row">
          <div class="group-row-content"><div class="group-row-title">{$t("tools.pinterest.offline")}</div></div>
          <div class="group-row-trailing"><input class="checkbox" type="checkbox" bind:checked={offline} /></div>
        </div>
      {/if}
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{$t("tools.common.output_folder")}</div><div class="group-row-sub mono">{dest || $t("tools.common.ask_on_run")}</div></div>
        <div class="group-row-trailing"><button class="btn btn-secondary btn-sm" type="button" onclick={async () => { const d = await pickDir(); if (d) dest = d; }}>{$t("tools.common.choose")}</button></div>
      </div>
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{$t("tools.pinterest.limit")}</div><div class="group-row-sub">{$t("tools.pinterest.limit_hint")}</div></div>
        <div class="group-row-trailing"><input class="input" type="number" min="0" step="50" bind:value={limit} style:width="7em" /></div>
      </div>
    </div>
  </section>

  <section>
    <span class="group-label">{$t("tools.pinterest.filters")}</span>
    <div class="group">
      <PinFilters bind:filters />
      <div class="group-row">
        <div class="group-row-content">{#if busy && progress}<div class="group-row-sub">{$t(progress.stage === "download" ? "tools.pinterest.stage_download" : "tools.pinterest.stage_list")} {progress.done}{#if progress.total}/{progress.total}{/if}{#if progress.message} · {progress.message}{/if}</div><div class="progress"><div class="progress-fill" style:width="{pct(progress) ?? 0}%"></div></div>{/if}</div>
        <div class="group-row-trailing"><button class="btn btn-primary" type="button" disabled={busy || !url.trim()} onclick={run}>{busy ? $t("tools.common.working") : $t("tools.pinterest.export")}</button></div>
      </div>
    </div>
  </section>

  {#if out}
    <section>
      <div class="group">
        <div class="group-row">
          <div class="group-row-content"><div class="group-row-title">{out.title} · {out.pins} {$t("tools.pinterest.pins")}</div><div class="group-row-sub mono">{out.path}</div></div>
          <div class="group-row-trailing btn-row">
            <button class="btn btn-secondary btn-sm" type="button" onclick={() => openPath(out!.path)}>{$t("tools.common.open")}</button>
            <button class="btn btn-ghost btn-sm" type="button" onclick={() => reveal(out!.path)}>{$t("tools.common.reveal")}</button>
          </div>
        </div>
      </div>
    </section>
  {/if}
</div>

<style>
  .tool { display: flex; flex-direction: column; gap: var(--space-5); }
  .mono { font-family: var(--font-mono); font-size: var(--text-xs); overflow-wrap: anywhere; }
  .segmented-btn.active { background: var(--surface-hi); color: var(--text); }
</style>

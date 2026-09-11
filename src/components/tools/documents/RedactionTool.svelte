<script lang="ts">
  /** Confere se a tarja do PDF escondeu texto ou só pintou por cima. */
  import { invoke } from "@tauri-apps/api/core";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import { baseName, errText, pickFile } from "$lib/tools/rt";

  type Run = { page: number; text: string; cover: string; rect: [number, number, number, number] };
  type Report = { path: string; pages: number; pages_checked: number; chars_total: number; chars_hidden: number; runs: Run[] };

  const PDF = [{ name: "PDF", extensions: ["pdf"] }];

  let file = $state("");
  let dpi = $state(110);
  let pages = $state("");
  let busy = $state(false);
  let report = $state<Report | null>(null);

  async function run() {
    if (!file || busy) return;
    busy = true;
    report = null;
    try {
      report = await invoke<Report>("tool_pdf_redaction_check", { input: file, dpi, pages });
      showToast(report.chars_hidden ? "info" : "success", report.chars_hidden ? `${report.chars_hidden} ${$t("tools.redaction.chars")}` : ($t("tools.redaction.clean") as string));
    } catch (e) {
      showToast("error", errText(e));
    } finally {
      busy = false;
    }
  }

  async function copyAll() {
    if (!report) return;
    await navigator.clipboard.writeText(report.runs.map((r) => `p.${r.page}: ${r.text}`).join("\n"));
    showToast("success", $t("tools.common.copied") as string);
  }
</script>

<div class="tool">
  <section>
    <div class="group">
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{$t("tools.redaction.file")}</div><div class="group-row-sub">{file ? baseName(file) : $t("tools.redaction.note")}</div></div>
        <div class="group-row-trailing"><button class="btn btn-secondary btn-sm" type="button" onclick={async () => { const f = await pickFile(PDF); if (f) { file = f; report = null; } }}>{$t("tools.common.choose")}</button></div>
      </div>
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{$t("tools.redaction.pages")} <span class="dim">· {$t("tools.redaction.dpi")} {dpi}</span></div></div>
        <div class="group-row-trailing btn-row">
          <input class="input mono" type="text" placeholder="1-5" bind:value={pages} style:width="7em" />
          <input type="range" min="72" max="200" step="10" bind:value={dpi} />
        </div>
      </div>
      <div class="group-row">
        <div class="group-row-content"></div>
        <div class="group-row-trailing"><button class="btn btn-primary" type="button" disabled={busy || !file} onclick={run}>{busy ? $t("tools.common.working") : $t("tools.redaction.run")}</button></div>
      </div>
    </div>
  </section>

  {#if report}
    <section>
      <div class="group">
        <div class="group-row">
          <div class="group-row-content">
            {#if report.chars_hidden}
              <div class="group-row-title"><span class="tag tag-danger">{$t("tools.redaction.found", { count: report.chars_hidden })}</span></div>
            {:else}
              <div class="group-row-title"><span class="tag tag-success">{$t("tools.redaction.clean")}</span></div>
            {/if}
            <div class="group-row-sub">{report.pages_checked}/{report.pages} {$t("tools.redaction.page")} · {report.chars_total} {$t("tools.redaction.chars")}</div>
          </div>
          {#if report.runs.length}<div class="group-row-trailing"><button class="btn btn-secondary btn-sm" type="button" onclick={copyAll}>{$t("tools.redaction.copy")}</button></div>{/if}
        </div>
        {#each report.runs as r, i (i)}
          <div class="group-row">
            <div class="group-row-content">
              <div class="group-row-title">{r.text}</div>
              <div class="group-row-sub">{$t("tools.redaction.page")} {r.page} · {$t("tools.redaction.cover")} <span class="swatch" style:background={r.cover}></span> <span class="mono">{r.cover}</span></div>
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
  .swatch { display: inline-block; width: 0.8em; height: 0.8em; border-radius: 3px; border: 1px solid var(--border); vertical-align: -1px; }
</style>

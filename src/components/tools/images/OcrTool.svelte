<script lang="ts">
  /** OCR com o Tesseract do sistema (estudo 29). */
  import { invoke } from "@tauri-apps/api/core";
  import { onMount } from "svelte";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import { baseName, errText, pickFiles, FILTERS } from "$lib/tools/rt";

  type Status = { installed: boolean; path: string | null; version: string | null; languages: string[]; install_hint: string };
  type Result = { path: string; text: string };
  let status = $state<Status | null>(null);
  let files = $state<string[]>([]);
  let langs = $state("por+eng");
  let busy = $state(false);
  let results = $state<Result[]>([]);

  onMount(async () => {
    status = await invoke<Status>("tool_ocr_status");
    if (status.languages.length && !status.languages.includes("por")) langs = status.languages.includes("eng") ? "eng" : status.languages[0];
  });

  async function run() {
    if (!files.length || busy) return;
    busy = true; results = [];
    try { results = await invoke<Result[]>("tool_ocr_run", { inputs: files, langs }); }
    catch (e) { showToast("error", errText(e)); } finally { busy = false; }
  }
  async function copy(text: string) { await navigator.clipboard.writeText(text); showToast("success", $t("tools.common.copied") as string); }
</script>

<div class="tool">
  <section>
    <div class="group">
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">Tesseract</div><div class="group-row-sub">{#if !status}…{:else if status.installed}{status.version} · {status.languages.join(", ")}{:else}{$t("tools.common.not_installed")} · <span class="mono">{status.install_hint}</span>{/if}</div></div>
      </div>
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{files.length} {$t("tools.common.files")}</div><div class="group-row-sub mono">{files.map(baseName).slice(0, 5).join(", ")}{files.length > 5 ? "…" : ""}</div></div>
        <div class="group-row-trailing btn-row">{#if files.length}<button class="btn btn-ghost btn-sm" type="button" onclick={() => (files = [])}>×</button>{/if}<button class="btn btn-secondary btn-sm" type="button" onclick={async () => { files = [...files, ...(await pickFiles(FILTERS.images))]; }}>{$t("tools.common.add")}</button></div>
      </div>
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{$t("tools.ocr.languages")}</div><div class="group-row-sub">{$t("tools.ocr.languages_hint")}</div></div>
        <div class="group-row-trailing"><input class="input mono" type="text" bind:value={langs} style:width="10em" /></div>
      </div>
      <div class="group-row">
        <div class="group-row-content"></div>
        <div class="group-row-trailing"><button class="btn btn-primary" type="button" disabled={busy || !files.length || !status?.installed} onclick={run}>{busy ? $t("tools.common.working") : $t("tools.ocr.run")}</button></div>
      </div>
    </div>
  </section>
  {#if results.length}
    <section>
      <div class="group">
        <div class="group-row"><div class="group-row-content"></div><div class="group-row-trailing"><button class="btn btn-secondary btn-sm" type="button" onclick={() => copy(results.map((r) => r.text).join("\n\n"))}>{$t("tools.ocr.copy_all")}</button></div></div>
        {#each results as r (r.path)}
          <div class="group-row">
            <div class="group-row-content"><div class="group-row-title">{baseName(r.path)}</div><pre class="text">{r.text}</pre></div>
            <div class="group-row-trailing"><button class="btn btn-ghost btn-sm" type="button" onclick={() => copy(r.text)}>{$t("tools.common.copy")}</button></div>
          </div>
        {/each}
      </div>
    </section>
  {/if}
</div>

<style>
  .tool { display: flex; flex-direction: column; gap: var(--space-5); }
  .mono { font-family: var(--font-mono); font-size: var(--text-xs); overflow-wrap: anywhere; }
  .text { margin: var(--space-1) 0 0; white-space: pre-wrap; max-height: 260px; overflow: auto; font-size: var(--text-sm); color: var(--text-muted); }
</style>

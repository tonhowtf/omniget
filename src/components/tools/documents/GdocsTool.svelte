<script lang="ts">
  /** Google Docs/Slides/Sheets públicos via export oficial (estudo 51). */
  import { invoke } from "@tauri-apps/api/core";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import { errText, pickDir, reveal } from "$lib/tools/rt";

  type Info = { kind: string; id: string; formats: string[] };
  let url = $state("");
  let info = $state<Info | null>(null);
  let format = $state("pdf");
  let dest = $state("");
  let busy = $state(false);
  let out = $state<string | null>(null);

  async function parse() {
    info = await invoke<Info | null>("tool_gdocs_parse", { url });
    if (!info) showToast("error", $t("tools.gdocs.bad_url") as string);
    else if (!info.formats.includes(format)) format = info.formats[0];
  }

  async function run() {
    if (!info || busy) return;
    if (!dest) { const d = await pickDir(); if (!d) return; dest = d; }
    busy = true; out = null;
    try {
      out = await invoke<string>("tool_gdocs_download", { url, format, dest });
      showToast("success", $t("tools.common.done") as string);
    } catch (e) { showToast("error", errText(e)); } finally { busy = false; }
  }
</script>

<div class="tool">
  <section>
    <div class="group">
      <div class="group-row">
        <div class="group-row-content"><input class="input" type="url" bind:value={url} oninput={() => (info = null)} placeholder="https://docs.google.com/presentation/d/…" onkeydown={(e) => e.key === "Enter" && parse()} /></div>
        <div class="group-row-trailing"><button class="btn btn-secondary btn-sm" type="button" disabled={!url.trim()} onclick={parse}>{$t("tools.gdocs.check")}</button></div>
      </div>
      {#if info}
        <div class="group-row">
          <div class="group-row-content"><div class="group-row-title">{info.kind} <span class="mono">{info.id}</span></div></div>
          <div class="group-row-trailing"><select class="input" bind:value={format}>{#each info.formats as f (f)}<option value={f}>{f}</option>{/each}</select></div>
        </div>
      {/if}
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{$t("tools.common.output_folder")}</div><div class="group-row-sub mono">{dest || $t("tools.common.ask_on_run")}</div></div>
        <div class="group-row-trailing"><button class="btn btn-secondary btn-sm" type="button" onclick={async () => { const d = await pickDir(); if (d) dest = d; }}>{$t("tools.common.choose")}</button></div>
      </div>
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-sub">{$t("tools.gdocs.note")}</div></div>
        <div class="group-row-trailing"><button class="btn btn-primary" type="button" disabled={busy || !info} onclick={run}>{busy ? $t("tools.common.working") : $t("tools.common.download")}</button></div>
      </div>
    </div>
  </section>
  {#if out}
    <section><div class="group"><div class="group-row">
      <div class="group-row-content"><div class="group-row-sub mono">{out}</div></div>
      <div class="group-row-trailing"><button class="btn btn-secondary btn-sm" type="button" onclick={() => reveal(out!)}>{$t("tools.common.reveal")}</button></div>
    </div></div></section>
  {/if}
</div>

<style>
  .tool { display: flex; flex-direction: column; gap: var(--space-5); }
  .mono { font-family: var(--font-mono); font-size: var(--text-xs); overflow-wrap: anywhere; }
</style>

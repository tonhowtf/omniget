<script lang="ts">
  /** Calameo (estudos 48/49): páginas SVG/JPG numa pasta; PDF quando as páginas são JPEG. */
  import { invoke } from "@tauri-apps/api/core";
  import { onDestroy, onMount } from "svelte";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import { errText, onToolProgress, pickDir, reveal, type ToolProgress } from "$lib/tools/rt";

  type Result = { title: string; pages: number; folder: string; format: string };
  let url = $state("");
  let dest = $state("");
  let busy = $state(false);
  let progress = $state<ToolProgress | null>(null);
  let result = $state<Result | null>(null);
  let unlisten: (() => void) | null = null;

  onMount(async () => { unlisten = await onToolProgress((p) => { if (p.id.startsWith("calameo:")) progress = p; }); });
  onDestroy(() => unlisten?.());

  async function run() {
    if (!url.trim() || busy) return;
    if (!dest) { const d = await pickDir(); if (!d) return; dest = d; }
    busy = true; result = null; progress = null;
    try {
      result = await invoke<Result>("tool_calameo", { url, dest });
      showToast("success", $t("tools.common.done") as string);
    } catch (e) { showToast("error", errText(e)); } finally { busy = false; }
  }
</script>

<div class="tool">
  <div class="notice notice-info"><div class="notice-text">{$t("tools.calameo.intro")}</div></div>
  <section>
    <div class="group">
      <div class="group-row"><div class="group-row-content"><input class="input" type="url" bind:value={url} placeholder="https://www.calameo.com/books/…" onkeydown={(e) => e.key === "Enter" && run()} /></div></div>
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{$t("tools.common.output_folder")}</div><div class="group-row-sub mono">{dest || $t("tools.common.ask_on_run")}</div></div>
        <div class="group-row-trailing"><button class="btn btn-secondary btn-sm" type="button" onclick={async () => { const d = await pickDir(); if (d) dest = d; }}>{$t("tools.common.choose")}</button></div>
      </div>
      <div class="group-row">
        <div class="group-row-content">{#if busy}<div class="group-row-sub">{$t("tools.calameo.page")} {progress?.done ?? 0}…</div>{/if}</div>
        <div class="group-row-trailing"><button class="btn btn-primary" type="button" disabled={busy || !url.trim()} onclick={run}>{busy ? $t("tools.common.working") : $t("tools.common.download")}</button></div>
      </div>
    </div>
  </section>
  {#if result}
    <section><div class="group"><div class="group-row">
      <div class="group-row-content"><div class="group-row-title">{result.title} · {result.pages} {$t("tools.slideshare.pages")} · {result.format}</div><div class="group-row-sub mono">{result.folder}</div></div>
      <div class="group-row-trailing"><button class="btn btn-secondary btn-sm" type="button" onclick={() => reveal(result!.folder)}>{$t("tools.common.reveal")}</button></div>
    </div></div></section>
  {/if}
</div>

<style>
  .tool { display: flex; flex-direction: column; gap: var(--space-5); }
  .mono { font-family: var(--font-mono); font-size: var(--text-xs); overflow-wrap: anywhere; }
</style>

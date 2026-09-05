<script lang="ts">
  /** gallery-dl (estudo 52): galerias e perfis inteiros de 250+ sites. */
  import { invoke } from "@tauri-apps/api/core";
  import { onDestroy, onMount } from "svelte";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import { errText, onToolProgress, pickDir, pickFile, reveal, type ToolProgress } from "$lib/tools/rt";

  type Status = { installed: boolean; path: string | null; version: string | null };
  type Result = { files: string[]; dest: string; log_tail: string };
  let status = $state<Status | null>(null);
  let url = $state("");
  let dest = $state("");
  let cookies = $state("");
  let busy = $state<string | null>(null);
  let progress = $state<ToolProgress | null>(null);
  let result = $state<Result | null>(null);
  let unlisten: (() => void) | null = null;

  async function refresh() { status = await invoke<Status>("tool_gallery_status"); }
  onMount(async () => { await refresh(); unlisten = await onToolProgress((p) => { if (p.id.startsWith("gallery:")) progress = p; }); });
  onDestroy(() => unlisten?.());

  async function install() {
    busy = "install";
    try { await invoke("tool_gallery_install"); showToast("success", $t("tools.common.installed") as string); }
    catch (e) { showToast("error", errText(e)); } finally { busy = null; await refresh(); }
  }

  async function run() {
    if (!url.trim() || busy) return;
    if (!dest) { const d = await pickDir(); if (!d) return; dest = d; }
    busy = "run"; result = null; progress = null;
    try {
      result = await invoke<Result>("tool_gallery_download", { url, dest, cookiesFile: cookies || null });
      showToast("success", `${result.files.length} ${$t("tools.gallery.files")}`);
    } catch (e) { showToast("error", errText(e)); } finally { busy = null; }
  }
</script>

<div class="tool">
  <section>
    <span class="group-label">gallery-dl</span>
    <div class="group">
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{$t("tools.gallery.engine")}</div><div class="group-row-sub">{#if !status}…{:else if status.installed}{status.version ?? ""} · <span class="mono">{status.path}</span>{:else}{$t("tools.common.not_installed")}{/if}</div></div>
        <div class="group-row-trailing">{#if status && !status.installed}<button class="btn btn-primary btn-sm" type="button" disabled={busy !== null} onclick={install}>{busy === "install" ? $t("tools.common.installing") : $t("tools.common.install")}</button>{/if}</div>
      </div>
    </div>
  </section>
  <section>
    <div class="group">
      <div class="group-row"><div class="group-row-content"><input class="input" type="url" bind:value={url} placeholder={$t("tools.gallery.placeholder")} onkeydown={(e) => e.key === "Enter" && run()} /></div></div>
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{$t("tools.common.output_folder")}</div><div class="group-row-sub mono">{dest || $t("tools.common.ask_on_run")}</div></div>
        <div class="group-row-trailing"><button class="btn btn-secondary btn-sm" type="button" onclick={async () => { const d = await pickDir(); if (d) dest = d; }}>{$t("tools.common.choose")}</button></div>
      </div>
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{$t("tools.gallery.cookies")}</div><div class="group-row-sub mono">{cookies || $t("tools.common.optional")}</div></div>
        <div class="group-row-trailing btn-row">{#if cookies}<button class="btn btn-ghost btn-sm" type="button" onclick={() => (cookies = "")}>×</button>{/if}<button class="btn btn-secondary btn-sm" type="button" onclick={async () => { const f = await pickFile([{ name: "cookies.txt", extensions: ["txt"] }]); if (f) cookies = f; }}>{$t("tools.common.choose")}</button></div>
      </div>
      <div class="group-row">
        <div class="group-row-content">{#if busy === "run"}<div class="group-row-sub">{progress?.done ?? 0} {$t("tools.gallery.files")} · <span class="mono">{progress?.message ?? ""}</span></div>{/if}</div>
        <div class="group-row-trailing"><button class="btn btn-primary" type="button" disabled={busy !== null || !url.trim() || !status?.installed} onclick={run}>{busy === "run" ? $t("tools.common.working") : $t("tools.common.download")}</button></div>
      </div>
    </div>
  </section>
  {#if result}
    <section><div class="group"><div class="group-row">
      <div class="group-row-content"><div class="group-row-title">{result.files.length} {$t("tools.gallery.files")}</div><div class="group-row-sub mono">{result.dest}</div></div>
      <div class="group-row-trailing"><button class="btn btn-secondary btn-sm" type="button" onclick={() => reveal(result!.dest)}>{$t("tools.common.reveal")}</button></div>
    </div></div></section>
  {/if}
</div>

<style>
  .tool { display: flex; flex-direction: column; gap: var(--space-5); }
  .mono { font-family: var(--font-mono); font-size: var(--text-xs); overflow-wrap: anywhere; }
</style>

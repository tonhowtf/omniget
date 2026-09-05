<script lang="ts">
  /** Download acelerado com aria2c (estudo 53). */
  import { invoke } from "@tauri-apps/api/core";
  import { onDestroy, onMount } from "svelte";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import { errText, fmtBytes, onToolProgress, pickDir, reveal, type ToolProgress } from "$lib/tools/rt";

  type Status = { installed: boolean; path: string | null; version: string | null };
  type Result = { path: string; bytes: number };
  let status = $state<Status | null>(null);
  let url = $state("");
  let dest = $state("");
  let name = $state("");
  let conns = $state(16);
  let sha = $state("");
  let busy = $state(false);
  let progress = $state<ToolProgress | null>(null);
  let result = $state<Result | null>(null);
  let unlisten: (() => void) | null = null;

  onMount(async () => { status = await invoke<Status>("tool_aria2_status"); unlisten = await onToolProgress((p) => { if (p.id.startsWith("aria2:")) progress = p; }); });
  onDestroy(() => unlisten?.());

  async function run() {
    if (!url.trim() || busy) return;
    if (!dest) { const d = await pickDir(); if (!d) return; dest = d; }
    busy = true; result = null; progress = null;
    try {
      result = await invoke<Result>("tool_aria2_download", { opts: { url, dest_dir: dest, file_name: name, connections: conns, sha256: sha, headers: [] } });
      showToast("success", $t("tools.common.done") as string);
    } catch (e) { showToast("error", errText(e)); } finally { busy = false; }
  }
</script>

<div class="tool">
  <section>
    <div class="group">
      <div class="group-row"><div class="group-row-content"><div class="group-row-title">aria2c</div><div class="group-row-sub">{#if !status}…{:else if status.installed}{status.version ?? ""} · <span class="mono">{status.path}</span>{:else}{$t("tools.common.not_installed")} · {$t("tools.aria2.install_hint")}{/if}</div></div></div>
      <div class="group-row"><div class="group-row-content"><input class="input" type="url" bind:value={url} placeholder="https://…/arquivo.zip" onkeydown={(e) => e.key === "Enter" && run()} /></div></div>
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{$t("tools.common.output_folder")}</div><div class="group-row-sub mono">{dest || $t("tools.common.ask_on_run")}</div></div>
        <div class="group-row-trailing"><button class="btn btn-secondary btn-sm" type="button" onclick={async () => { const d = await pickDir(); if (d) dest = d; }}>{$t("tools.common.choose")}</button></div>
      </div>
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{$t("tools.aria2.options")}</div></div>
        <div class="group-row-trailing btn-row">
          <input class="input" type="text" bind:value={name} placeholder={$t("tools.aria2.file_name")} style:width="12em" />
          <input class="input" type="number" min="1" max="16" bind:value={conns} style:width="4em" title={$t("tools.aria2.connections")} />
          <input class="input mono" type="text" bind:value={sha} placeholder="sha256 (opcional)" style:width="14em" />
        </div>
      </div>
      <div class="group-row">
        <div class="group-row-content">{#if busy}<div class="group-row-sub">{progress?.done ?? 0}% · {progress?.message ?? ""}/s</div><div class="progress"><div class="progress-fill" style:width="{progress?.done ?? 0}%"></div></div>{/if}</div>
        <div class="group-row-trailing"><button class="btn btn-primary" type="button" disabled={busy || !url.trim() || !status?.installed} onclick={run}>{busy ? $t("tools.common.working") : $t("tools.common.download")}</button></div>
      </div>
    </div>
  </section>
  {#if result}
    <section><div class="group"><div class="group-row">
      <div class="group-row-content"><div class="group-row-title">{fmtBytes(result.bytes)}</div><div class="group-row-sub mono">{result.path}</div></div>
      <div class="group-row-trailing"><button class="btn btn-secondary btn-sm" type="button" onclick={() => reveal(result!.path)}>{$t("tools.common.reveal")}</button></div>
    </div></div></section>
  {/if}
</div>

<style>
  .tool { display: flex; flex-direction: column; gap: var(--space-5); }
  .mono { font-family: var(--font-mono); font-size: var(--text-xs); overflow-wrap: anywhere; }
</style>

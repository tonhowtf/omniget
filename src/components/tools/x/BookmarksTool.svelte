<script lang="ts">
  /** Exportar favoritos (estudo 67): pastas + todos os bookmarks, sem o limite de 800. */
  import { invoke } from "@tauri-apps/api/core";
  import { onDestroy, onMount } from "svelte";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import { onToolProgress, pickDir, reveal, type ToolProgress } from "$lib/tools/rt";
  import { xErr, type XPost, type XSession } from "$lib/tools/x";
  import PostCard from "./PostCard.svelte";
  import XSessionRow from "./XSession.svelte";

  type Folder = { id: string; name: string; count: number };
  type Item = { post: XPost; folders: string[] };
  type Result = { count: number; folders: Folder[]; files: string[]; media_files: number; cancelled: boolean; preview: Item[] };

  let sess = $state<XSession | null>(null);
  let dest = $state("");
  let formats = $state<Record<string, boolean>>({ json: true, csv: true, md: false, html: true });
  let withMedia = $state(false);
  let max = $state(0);
  let busy = $state(false);
  let progress = $state<ToolProgress | null>(null);
  let result = $state<Result | null>(null);
  let unlisten: (() => void) | null = null;

  onMount(async () => {
    unlisten = await onToolProgress((p) => {
      if (p.id === "x-bookmarks") progress = p;
    });
  });
  onDestroy(() => unlisten?.());

  async function run() {
    if (busy) return;
    const chosen = Object.entries(formats).filter(([, v]) => v).map(([k]) => k);
    if (!chosen.length) return;
    if (!dest) {
      const d = await pickDir();
      if (!d) return;
      dest = d;
    }
    busy = true;
    result = null;
    progress = null;
    try {
      result = await invoke<Result>("tool_x_bookmarks_export", { dest, formats: chosen, withMedia, max });
      showToast(result.cancelled ? "info" : "success", `${result.count} ${$t("tools.x.bookmarks")}`);
    } catch (e) {
      showToast("error", xErr(e));
    } finally {
      busy = false;
    }
  }

  let stageText = $derived.by(() => {
    if (!progress) return "";
    switch (progress.stage) {
      case "folders": return $t("tools.x.bm_stage_folders");
      case "folder": return `${$t("tools.x.bm_stage_folder")} ${progress.message ?? ""}`;
      case "bookmarks": return `${progress.done} ${$t("tools.x.bookmarks")}`;
      case "download": return `${progress.done}/${progress.total ?? "?"} · ${progress.message ?? ""}`;
      default: return progress.stage;
    }
  });
</script>

<div class="tool">
  <section>
    <div class="group"><XSessionRow required onchange={(s) => (sess = s)} /></div>
  </section>
  <section>
    <div class="group">
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{$t("tools.common.output_folder")}</div><div class="group-row-sub mono">{dest || $t("tools.common.ask_on_run")}</div></div>
        <div class="group-row-trailing"><button class="btn btn-secondary btn-sm" type="button" onclick={async () => { const d = await pickDir(); if (d) dest = d; }}>{$t("tools.common.choose")}</button></div>
      </div>
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{$t("tools.x.formats")}</div></div>
        <div class="group-row-trailing btn-row">
          {#each ["json", "csv", "md", "html"] as f (f)}<label class="opt"><input class="checkbox" type="checkbox" bind:checked={formats[f]} /> {f.toUpperCase()}</label>{/each}
        </div>
      </div>
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{$t("tools.x.bm_media")}</div><div class="group-row-sub">{$t("tools.x.bm_media_hint")}</div></div>
        <div class="group-row-trailing"><label class="opt"><input class="checkbox" type="checkbox" bind:checked={withMedia} /></label></div>
      </div>
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{$t("tools.x.bm_max")}</div></div>
        <div class="group-row-trailing"><div class="segmented">{#each [200, 1000, 0] as n (n)}<button class="segmented-btn" class:active={max === n} type="button" onclick={() => (max = n)}>{n === 0 ? $t("tools.x.all") : n}</button>{/each}</div></div>
      </div>
      <div class="group-row">
        <div class="group-row-content">{#if busy}<div class="group-row-sub">{stageText}</div>{:else}<div class="group-row-sub">{$t("tools.x.bm_intro")}</div>{/if}</div>
        <div class="group-row-trailing btn-row">
          {#if busy}<button class="btn btn-secondary" type="button" onclick={() => invoke("tool_x_cancel", { job: "x-bookmarks" })}>{$t("tools.x.stop")}</button>{/if}
          <button class="btn btn-primary" type="button" disabled={busy || !sess?.logged_in} onclick={run}>{busy ? $t("tools.common.working") : $t("tools.x.export")}</button>
        </div>
      </div>
    </div>
  </section>
  {#if result}
    <section>
      <div class="group">
        <div class="group-row">
          <div class="group-row-content">
            <div class="group-row-title">{result.count} {$t("tools.x.bookmarks")} · {result.folders.length} {$t("tools.x.folders")}{#if result.media_files} · {result.media_files} {$t("tools.common.files")}{/if}</div>
            <div class="group-row-sub">{#each result.files as f (f)}<span class="mono">{f}</span><br />{/each}</div>
          </div>
          <div class="group-row-trailing">{#if result.files[0]}<button class="btn btn-secondary btn-sm" type="button" onclick={() => reveal(result!.files[0])}>{$t("tools.common.reveal")}</button>{/if}</div>
        </div>
        {#if result.folders.length}
          <div class="group-row"><div class="group-row-content chips">{#each result.folders as f (f.id)}<span class="tag">{f.name} · {f.count}</span>{/each}</div></div>
        {/if}
      </div>
    </section>
    <section>
      <span class="group-label">{$t("tools.x.preview")}</span>
      <div class="group">{#each result.preview as it (it.post.id)}<PostCard post={it.post} compact />{/each}</div>
    </section>
  {/if}
</div>

<style>
  .tool { display: flex; flex-direction: column; gap: var(--space-5); }
  .mono { font-family: var(--font-mono); font-size: var(--text-xs); overflow-wrap: anywhere; }
  .opt { display: inline-flex; align-items: center; gap: var(--space-1); font-size: var(--text-sm); }
  .chips { display: flex; flex-wrap: wrap; gap: var(--space-1); }
</style>

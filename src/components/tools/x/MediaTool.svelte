<script lang="ts">
  /** Mídia em lote de um perfil (estudo 67): fotos em `name=orig` e o melhor mp4. */
  import { invoke } from "@tauri-apps/api/core";
  import { onDestroy, onMount } from "svelte";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import { onToolProgress, pickDir, reveal, type ToolProgress } from "$lib/tools/rt";
  import { handleFrom, xErr } from "$lib/tools/x";

  type Result = { files: string[]; skipped: number; failed: number; posts: number; dest: string; cancelled: boolean };

  let input = $state("");
  let dest = $state("");
  let limit = $state(500);
  let photos = $state(true);
  let videos = $state(true);
  let busy = $state(false);
  let progress = $state<ToolProgress | null>(null);
  let result = $state<Result | null>(null);
  let unlisten: (() => void) | null = null;

  onMount(async () => {
    unlisten = await onToolProgress((p) => {
      if (p.id.startsWith("x-media:")) progress = p;
    });
  });
  onDestroy(() => unlisten?.());

  async function run() {
    if (!input.trim() || busy) return;
    if (!dest) {
      const d = await pickDir();
      if (!d) return;
      dest = d;
    }
    busy = true;
    result = null;
    progress = null;
    try {
      result = await invoke<Result>("tool_x_media", { input, dest, limit, photos, videos });
      showToast(result.cancelled ? "info" : "success", `${result.files.length} ${$t("tools.common.files")}`);
    } catch (e) {
      showToast("error", xErr(e));
    } finally {
      busy = false;
    }
  }

  function cancel() {
    const h = handleFrom(input);
    if (h) invoke("tool_x_cancel", { job: `x-media:${h.toLowerCase()}` });
  }

  let pct = $derived(progress?.total ? Math.round((progress.done / progress.total) * 100) : null);
</script>

<div class="tool">
  <section>
    <div class="group">
      <div class="group-row"><div class="group-row-content"><input class="input" type="text" bind:value={input} placeholder={$t("tools.x.handle_placeholder")} onkeydown={(e) => e.key === "Enter" && run()} /></div></div>
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{$t("tools.common.output_folder")}</div><div class="group-row-sub mono">{dest || $t("tools.common.ask_on_run")}</div></div>
        <div class="group-row-trailing"><button class="btn btn-secondary btn-sm" type="button" onclick={async () => { const d = await pickDir(); if (d) dest = d; }}>{$t("tools.common.choose")}</button></div>
      </div>
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{$t("tools.x.media_limit")}</div><div class="group-row-sub">{$t("tools.x.media_limit_hint")}</div></div>
        <div class="group-row-trailing">
          <div class="segmented">{#each [100, 500, 2000, 0] as n (n)}<button class="segmented-btn" class:active={limit === n} type="button" onclick={() => (limit = n)}>{n === 0 ? $t("tools.x.all") : n}</button>{/each}</div>
        </div>
      </div>
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{$t("tools.x.media_kinds")}</div></div>
        <div class="group-row-trailing btn-row">
          <label class="opt"><input class="checkbox" type="checkbox" bind:checked={photos} /> {$t("tools.x.photos")}</label>
          <label class="opt"><input class="checkbox" type="checkbox" bind:checked={videos} /> {$t("tools.x.videos")}</label>
        </div>
      </div>
      <div class="group-row">
        <div class="group-row-content">
          {#if busy}
            <div class="group-row-sub">{progress?.stage === "listing" ? `${$t("tools.x.listing")} ${progress.done} ${$t("tools.x.posts")}` : `${progress?.done ?? 0}${progress?.total ? ` / ${progress.total}` : ""} · ${progress?.message ?? ""}`}</div>
            {#if pct !== null}<div class="bar"><div class="bar-fill" style:width="{pct}%"></div></div>{/if}
          {/if}
        </div>
        <div class="group-row-trailing btn-row">
          {#if busy}<button class="btn btn-secondary" type="button" onclick={cancel}>{$t("tools.x.stop")}</button>{/if}
          <button class="btn btn-primary" type="button" disabled={busy || !input.trim() || (!photos && !videos)} onclick={run}>{busy ? $t("tools.common.working") : $t("tools.common.download")}</button>
        </div>
      </div>
    </div>
  </section>
  {#if result}
    <section>
      <div class="group"><div class="group-row">
        <div class="group-row-content">
          <div class="group-row-title">{result.files.length} {$t("tools.common.files")} · {result.posts} {$t("tools.x.posts")}</div>
          <div class="group-row-sub">{result.skipped} {$t("tools.x.skipped")} · {result.failed} {$t("tools.common.failed")}{#if result.cancelled} · {$t("tools.x.cancelled")}{/if} · <span class="mono">{result.dest}</span></div>
        </div>
        <div class="group-row-trailing"><button class="btn btn-secondary btn-sm" type="button" onclick={() => reveal(result!.dest)}>{$t("tools.common.reveal")}</button></div>
      </div></div>
    </section>
  {/if}
</div>

<style>
  .tool { display: flex; flex-direction: column; gap: var(--space-5); }
  .mono { font-family: var(--font-mono); font-size: var(--text-xs); overflow-wrap: anywhere; }
  .opt { display: inline-flex; align-items: center; gap: var(--space-1); font-size: var(--text-sm); }
  .bar { height: 4px; border-radius: 2px; background: var(--content-border); overflow: hidden; margin-top: var(--space-1); }
  .bar-fill { height: 100%; background: var(--accent); transition: width 0.2s; }
</style>

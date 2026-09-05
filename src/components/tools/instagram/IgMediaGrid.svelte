<script lang="ts">
  /**
   * Grade de itens (post, reel, story, highlight) com seleção, opções de
   * download e barra de progresso. Reutilizada por todas as tools que
   * terminam em "baixar".
   */
  import { invoke } from "@tauri-apps/api/core";
  import { onDestroy, onMount, untrack } from "svelte";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import { errText, onToolProgress, openUrl, pickDir, reveal, type ToolProgress } from "$lib/tools/rt";
  import { cancelJob, defaultDownloadOptions, fmtDay, jobId, kindLabel, n, recall, remember, slugArg, type DownloadOptions, type DownloadResult, type MediaItem } from "$lib/tools/ig.svelte";

  let { items, jobPrefix = "dl", audioDefault = "" as "" | "m4a" | "mp3" }: { items: MediaItem[]; jobPrefix?: string; audioDefault?: "" | "m4a" | "mp3" } = $props();

  let selected = $state<Set<string>>(new Set());
  let dest = $state(recall("dest"));
  let opts = $state<DownloadOptions>({ ...defaultDownloadOptions(), audio_only: untrack(() => audioDefault) || defaultDownloadOptions().audio_only });
  let busy = $state(false);
  let job = $state("");
  let progress = $state<ToolProgress | null>(null);
  let result = $state<DownloadResult | null>(null);
  let unlisten: (() => void) | null = null;

  $effect(() => {
    selected = new Set(items.map((i) => i.pk));
    result = null;
  });

  onMount(async () => {
    unlisten = await onToolProgress((p) => {
      if (job && p.id === `ig:${job}`) progress = p;
    });
  });
  onDestroy(() => unlisten?.());

  function toggle(pk: string) {
    const s = new Set(selected);
    if (s.has(pk)) s.delete(pk);
    else s.add(pk);
    selected = s;
  }

  let files = $derived(items.filter((i) => selected.has(i.pk)).reduce((a, i) => a + i.files.length, 0));

  async function download() {
    if (busy) return;
    if (!dest) {
      const d = await pickDir();
      if (!d) return;
      dest = d;
    }
    remember("dest", dest);
    remember("dlopts", JSON.stringify(opts));
    busy = true;
    result = null;
    progress = null;
    job = jobId(jobPrefix);
    try {
      result = await invoke<DownloadResult>("tool_ig_download", { slug: slugArg(), items: items.filter((i) => selected.has(i.pk)), dest, opts: $state.snapshot(opts), job });
      if (result.failed.length) showToast("error", `${result.failed.length} ${$t("tools.common.failed")}`);
      else showToast("success", `${result.files.length} ${$t("tools.common.files")}`);
    } catch (e) {
      showToast("error", errText(e));
    } finally {
      busy = false;
    }
  }

  async function copyCaption(text: string) {
    try {
      await navigator.clipboard.writeText(text);
      showToast("success", $t("tools.common.copied") as string);
    } catch {
      /* ignore */
    }
  }

  let pct = $derived(progress?.total ? Math.round((progress.done / progress.total) * 100) : 0);
</script>

{#if items.length}
  <section>
    <div class="head">
      <span class="group-label">{items.length} {$t("tools.ig.grid.items")} · {files} {$t("tools.common.files")}</span>
      <div class="btn-row">
        <button class="btn btn-ghost btn-sm" type="button" onclick={() => (selected = new Set(items.map((i) => i.pk)))}>{$t("tools.ig.grid.select_all")}</button>
        <button class="btn btn-ghost btn-sm" type="button" onclick={() => (selected = new Set())}>{$t("tools.ig.grid.select_none")}</button>
      </div>
    </div>
    <div class="grid">
      {#each items as item (item.pk)}
        <div class="card" class:off={!selected.has(item.pk)}>
          <button class="thumb" type="button" onclick={() => toggle(item.pk)} aria-pressed={selected.has(item.pk)}>
            <img src={item.thumbnail} alt="" loading="lazy" onerror={(e) => ((e.currentTarget as HTMLImageElement).style.opacity = "0.2")} />
            <span class="badge">{kindLabel(item)}{#if item.files.length > 1} · {item.files.length}{/if}</span>
            <span class="check">{selected.has(item.pk) ? "✓" : ""}</span>
          </button>
          <div class="meta">
            <div class="line"><span>@{item.username}</span><span class="muted">{fmtDay(item.taken_at)}</span></div>
            {#if item.product_type !== "story"}
              <div class="line muted">♥ {n(item.like_count)} · 💬 {n(item.comment_count)}{#if item.play_count} · ▶ {n(item.play_count)}{/if}</div>
            {:else if item.title}
              <div class="line muted">{item.title}</div>
            {/if}
            {#if item.caption}<div class="cap">{item.caption}</div>{/if}
            <div class="btn-row">
              <button class="btn btn-ghost btn-sm" type="button" onclick={() => openUrl(item.url)}>{$t("tools.common.open")}</button>
              {#if item.caption}<button class="btn btn-ghost btn-sm" type="button" onclick={() => copyCaption(item.caption)}>{$t("tools.ig.grid.copy_caption")}</button>{/if}
            </div>
          </div>
        </div>
      {/each}
    </div>
  </section>
  <section>
    <span class="group-label">{$t("tools.common.download")}</span>
    <div class="group">
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{$t("tools.common.output_folder")}</div><div class="group-row-sub mono">{dest || $t("tools.common.ask_on_run")}</div></div>
        <div class="group-row-trailing"><button class="btn btn-secondary btn-sm" type="button" onclick={async () => { const d = await pickDir(); if (d) dest = d; }}>{$t("tools.common.choose")}</button></div>
      </div>
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{$t("tools.ig.grid.audio")}</div><div class="group-row-sub">{$t("tools.ig.grid.audio_hint")}</div></div>
        <div class="group-row-trailing">
          <select class="select" bind:value={opts.audio_only}>
            <option value="">{$t("tools.ig.grid.audio_no")}</option>
            <option value="m4a">M4A</option>
            <option value="mp3">MP3</option>
          </select>
        </div>
      </div>
      <div class="group-row">
        <div class="group-row-content">
          <div class="opts">
            <label class="chk"><input type="checkbox" bind:checked={opts.caption_txt} /> {$t("tools.ig.grid.caption_txt")}</label>
            <label class="chk"><input type="checkbox" bind:checked={opts.metadata_json} /> {$t("tools.ig.grid.metadata_json")}</label>
            <label class="chk"><input type="checkbox" bind:checked={opts.per_user_folder} /> {$t("tools.ig.grid.per_user")}</label>
            <label class="chk"><input type="checkbox" bind:checked={opts.skip_existing} /> {$t("tools.ig.grid.skip_existing")}</label>
          </div>
        </div>
      </div>
      <div class="group-row">
        <div class="group-row-content">
          {#if busy}
            <div class="bar"><div class="bar-fill" style:width="{pct}%"></div></div>
            <div class="group-row-sub mono">{progress?.done ?? 0}/{progress?.total ?? files} · {progress?.message ?? ""}</div>
          {:else if result}
            <div class="group-row-title">{result.files.length} {$t("tools.common.files")}{#if result.skipped} · {result.skipped} {$t("tools.ig.grid.skipped")}{/if}{#if result.failed.length} · {result.failed.length} {$t("tools.common.failed")}{/if}</div>
            <div class="group-row-sub mono">{result.dest}</div>
          {/if}
        </div>
        <div class="group-row-trailing btn-row">
          {#if busy}
            <button class="btn btn-secondary btn-sm" type="button" onclick={() => cancelJob(job)}>{$t("tools.ig.common.cancel")}</button>
          {:else}
            {#if result}<button class="btn btn-secondary btn-sm" type="button" onclick={() => reveal(result!.dest)}>{$t("tools.common.reveal")}</button>{/if}
            <button class="btn btn-primary" type="button" disabled={!files} onclick={download}>{$t("tools.common.download")} ({files})</button>
          {/if}
        </div>
      </div>
    </div>
  </section>
{/if}

<style>
  .head { display: flex; justify-content: space-between; align-items: center; gap: var(--space-2); }
  .grid { display: grid; grid-template-columns: repeat(auto-fill, minmax(180px, 1fr)); gap: var(--space-3); }
  .card { display: flex; flex-direction: column; border-radius: var(--radius-lg); overflow: hidden; background: var(--surface); box-shadow: inset 0 0 0 var(--hairline) var(--content-border); transition: opacity 0.15s; }
  .card.off { opacity: 0.45; }
  .thumb { position: relative; display: block; width: 100%; aspect-ratio: 1; padding: 0; border: 0; background: var(--surface-2, var(--surface)); cursor: pointer; }
  .thumb img { width: 100%; height: 100%; object-fit: cover; display: block; }
  .badge { position: absolute; left: 6px; bottom: 6px; padding: 2px 6px; border-radius: 6px; font-size: 11px; font-weight: 600; color: #fff; background: rgba(0, 0, 0, 0.6); text-transform: uppercase; }
  .check { position: absolute; right: 6px; top: 6px; width: 22px; height: 22px; border-radius: 50%; display: grid; place-items: center; font-size: 13px; font-weight: 700; color: #fff; background: var(--accent); box-shadow: 0 0 0 2px #fff; }
  .off .check { background: rgba(0, 0, 0, 0.35); }
  .meta { display: flex; flex-direction: column; gap: 4px; padding: var(--space-2) var(--space-3) var(--space-3); font-size: var(--text-xs); }
  .line { display: flex; justify-content: space-between; gap: var(--space-2); }
  .muted { color: var(--text-muted); }
  .cap { color: var(--text-muted); display: -webkit-box; -webkit-line-clamp: 2; line-clamp: 2; -webkit-box-orient: vertical; overflow: hidden; }
  .opts { display: flex; flex-wrap: wrap; gap: var(--space-2) var(--space-4); }
  .chk { display: inline-flex; align-items: center; gap: var(--space-1); font-size: var(--text-sm); }
  .bar { width: 100%; height: 6px; border-radius: 3px; background: var(--content-border); overflow: hidden; margin-bottom: var(--space-1); }
  .bar-fill { height: 100%; background: var(--accent); transition: width 0.2s; }
  .mono { font-family: var(--font-mono); font-size: var(--text-xs); overflow-wrap: anywhere; }
</style>

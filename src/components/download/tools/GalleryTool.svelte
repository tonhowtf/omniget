<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { listen } from "@tauri-apps/api/event";
  import { onDestroy } from "svelte";
  import { t } from "$lib/i18n";
  import { translateBackendError } from "$lib/error-translate";
  import { formatBytes } from "$lib/stores/download-store.svelte";
  import { getSettings } from "$lib/stores/settings-store.svelte";

  type Result = { folder: string; files: number; bytes: number; skipped: number };
  type Progress = { files: number; skipped: number; last: string };

  let url = $state("");
  let images = $state(true);
  let videos = $state(true);
  let other = $state(false);
  let minKb = $state(0);
  let running = $state(false);
  let progress = $state<Progress | null>(null);
  let result = $state<Result | null>(null);
  let error = $state<string | null>(null);

  const SIZES = [0, 20, 100, 500];

  let unlisten: (() => void) | null = null;
  listen<Progress>("gallery-grab-progress", (e) => {
    if (running) progress = e.payload;
  }).then((fn) => (unlisten = fn));
  onDestroy(() => unlisten?.());

  async function start() {
    const target = url.trim();
    if (!target || running) return;
    const outputDir = getSettings()?.download.default_output_dir ?? "";
    if (!outputDir) {
      error = $t("downloads.tools.gallery.no_folder");
      return;
    }
    const kinds = [images && "images", videos && "videos", other && "other"].filter(Boolean) as string[];
    if (!kinds.length) {
      error = $t("downloads.tools.gallery.no_kind");
      return;
    }
    running = true;
    error = null;
    result = null;
    progress = null;
    try {
      result = await invoke<Result>("gallery_grab", { url: target, outputDir, kinds, minKb: minKb || null });
    } catch (e: any) {
      const msg = typeof e === "string" ? e : (e?.message ?? "");
      error = msg === "Cancelled" ? $t("downloads.tools.gallery.cancelled") : msg ? translateBackendError(msg, $t) : $t("common.error");
    } finally {
      running = false;
    }
  }

  function cancel() {
    void invoke("gallery_grab_cancel");
  }
</script>

<div class="gal" data-agent-readiness={running ? "loading" : "ready"}>
  <p class="lead">{$t("downloads.tools.gallery.lead")}</p>
  <form
    class="row"
    onsubmit={(e) => {
      e.preventDefault();
      start();
    }}
  >
    <input
      type="url"
      bind:value={url}
      placeholder={$t("downloads.tools.gallery.placeholder")}
      aria-label={$t("downloads.tools.gallery.placeholder")}
      spellcheck="false"
      autocomplete="off"
      disabled={running}
    />
    {#if running}
      <button class="button" type="button" onclick={cancel}>{$t("downloads.tools.gallery.cancel")}</button>
    {:else}
      <button class="button" type="submit" disabled={!url.trim()}>{$t("downloads.tools.gallery.start")}</button>
    {/if}
  </form>

  <div class="opts">
    <span class="dim">{$t("downloads.tools.gallery.keep")}</span>
    <label><input type="checkbox" bind:checked={images} disabled={running} /> {$t("downloads.tools.gallery.images")}</label>
    <label><input type="checkbox" bind:checked={videos} disabled={running} /> {$t("downloads.tools.gallery.videos")}</label>
    <label><input type="checkbox" bind:checked={other} disabled={running} /> {$t("downloads.tools.gallery.other")}</label>
    <label class="size">
      {$t("downloads.tools.gallery.min_size")}
      <select bind:value={minKb} disabled={running}>
        {#each SIZES as s (s)}
          <option value={s}>{s ? `${s} KB` : $t("downloads.tools.gallery.any_size")}</option>
        {/each}
      </select>
    </label>
  </div>

  {#if running}
    <p class="status" role="status">
      {$t("downloads.tools.gallery.progress", { count: progress?.files ?? 0 })}{#if progress?.last} · <span class="dim">{progress.last}</span>{/if}
    </p>
  {/if}

  {#if error}
    <p class="error" role="alert">{error}</p>
  {/if}

  {#if result}
    <div class="done">
      <p>
        {$t("downloads.tools.gallery.done", { count: result.files, size: formatBytes(result.bytes) })}
        {#if result.skipped}<span class="dim"> · {$t("downloads.tools.gallery.skipped", { count: result.skipped })}</span>{/if}
      </p>
      {#if result.files}
        <button class="link" onclick={() => void invoke("reveal_file", { path: result!.folder })}>{$t("downloads.tools.gallery.show")}</button>
      {/if}
    </div>
  {/if}
  <p class="dim small">{$t("downloads.tools.gallery.note")}</p>
</div>

<style>
  .gal {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
  }
  .lead,
  .dim {
    margin: 0;
    color: var(--text-dim);
    font-size: var(--text-sm);
  }
  .small {
    font-size: var(--text-xs, 11px);
  }
  .row {
    display: flex;
    gap: var(--space-2);
  }
  .row input {
    flex: 1;
    min-width: 0;
    padding: var(--space-2) var(--space-3);
    border: none;
    border-radius: var(--radius-md, 8px);
    background: var(--surface);
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
    color: var(--text);
    font-size: var(--text-sm);
  }
  .row input:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: 1px;
  }
  .opts {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-3);
    font-size: var(--text-sm);
  }
  .opts label {
    display: inline-flex;
    align-items: center;
    gap: 4px;
  }
  .size select {
    background: var(--surface);
    color: var(--text);
    border: none;
    border-radius: var(--radius-md, 8px);
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
    padding: 2px var(--space-2);
  }
  .status,
  .done p {
    margin: 0;
    font-size: var(--text-sm);
  }
  .done {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    padding: var(--space-2) var(--space-3);
    border-radius: var(--radius-lg);
    background: var(--surface);
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
  }
  .error {
    margin: 0;
    color: var(--danger, #d33);
    font-size: var(--text-sm);
  }
  .link {
    border: none;
    background: none;
    color: var(--accent);
    font-size: var(--text-sm);
    cursor: pointer;
    padding: 2px var(--space-1);
    border-radius: var(--radius-full);
  }
  .link:hover {
    background: var(--accent-soft);
  }
</style>

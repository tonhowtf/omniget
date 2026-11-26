<script lang="ts">
  import { onDestroy, onMount } from "svelte";
  import { open as openDialog } from "@tauri-apps/plugin-dialog";
  import { t } from "$lib/i18n";
  import * as api from "$lib/pdf/api";
  import { formatBytes } from "$lib/download-format";
  import { baseName, joinPath, parentDir, pdfName, plural, samePath } from "$lib/pdf/pages";
  import type { PageSize, SavedFile } from "$lib/pdf/types";
  import StatusIcon from "#components/converter/StatusIcon.svelte";

  let { onbusy }: { onbusy?: (busy: boolean) => void } = $props();

  type Pic = { path: string; url: string | null; failed: boolean };

  const SIZES: PageSize[] = ["fit", "a4", "letter"];
  const THUMB_PARALLEL = 3;

  let pics = $state<Pic[]>([]);
  let size = $state<PageSize>("a4");
  let margin = $state(true);
  let adding = $state(false);
  let running = $state(false);
  let stopping = $state(false);
  let progress = $state({ done: 0, total: 0 });
  let notice = $state<string | null>(null);
  let result = $state<SavedFile | null>(null);
  let jobId: string | null = null;
  let thumbQueue: string[] = [];
  let thumbActive = 0;
  let destroyed = false;

  let busy = $derived(adding || running);

  $effect(() => onbusy?.(busy));

  function errorText(code: string | null): string {
    return $t(`pdf.error_${code ?? "io"}`) as string;
  }

  function pumpThumbs() {
    while (thumbActive < THUMB_PARALLEL && thumbQueue.length > 0) {
      const path = thumbQueue.shift() as string;
      if (!pics.some((p) => p.path === path)) continue;
      thumbActive += 1;
      void api
        .imageThumb(path)
        .then((blob) => {
          const live = destroyed ? undefined : pics.find((p) => p.path === path);
          if (live) live.url = URL.createObjectURL(blob);
        })
        .catch(() => {
          const live = pics.find((p) => p.path === path);
          if (live) live.failed = true;
        })
        .finally(() => {
          thumbActive -= 1;
          if (!destroyed) pumpThumbs();
        });
    }
  }

  function imageError(error: unknown): string {
    const message = error && typeof error === "object" && "message" in error ? String((error as { message: unknown }).message) : "";
    const bad = pics.find((p) => message.endsWith(p.path));
    if (!bad) return errorText("image");
    bad.failed = true;
    return $t("pdf.error_image_named", { name: baseName(bad.path) }) as string;
  }

  export async function add(paths: string[]) {
    if (paths.length === 0 || busy) return;
    adding = true;
    notice = null;
    result = null;
    try {
      const found = await api.expandImages(paths);
      const fresh = found.filter((p) => !pics.some((x) => samePath(x.path, p)));
      if (found.length === 0) notice = $t("pdf.images_none") as string;
      const added = fresh.map((path) => ({ path, url: null, failed: false }));
      pics = [...pics, ...added];
      thumbQueue.push(...added.map((p) => p.path));
      pumpThumbs();
    } finally {
      adding = false;
    }
  }

  async function chooseFiles() {
    const picked = await openDialog({ multiple: true, directory: false, filters: [{ name: "Images", extensions: ["jpg", "jpeg", "png", "webp", "gif", "bmp", "tif", "tiff"] }] }).catch(() => null);
    const paths = Array.isArray(picked) ? picked : typeof picked === "string" ? [picked] : [];
    await add(paths);
  }

  async function chooseFolder() {
    const picked = await openDialog({ directory: true, multiple: false }).catch(() => null);
    if (typeof picked === "string") await add([picked]);
  }

  function move(index: number, delta: number) {
    const to = index + delta;
    if (to < 0 || to >= pics.length || busy) return;
    const next = [...pics];
    const [pic] = next.splice(index, 1);
    next.splice(to, 0, pic);
    pics = next;
    result = null;
  }

  function remove(index: number) {
    if (busy) return;
    const [pic] = pics.splice(index, 1);
    if (pic?.url) URL.revokeObjectURL(pic.url);
    pics = [...pics];
    result = null;
  }

  function clearAll() {
    if (busy) return;
    for (const pic of pics) if (pic.url) URL.revokeObjectURL(pic.url);
    pics = [];
    result = null;
    notice = null;
  }

  async function build() {
    if (busy || pics.length === 0) return;
    running = true;
    stopping = false;
    notice = null;
    result = null;
    progress = { done: 0, total: pics.length };
    jobId = api.newJobId();
    const first = pics[0].path;
    const name = baseName(parentDir(first)) || "images";
    const out = joinPath(parentDir(first), pdfName($t("pdf.name_images", { name }) as string));
    try {
      result = await api.fromImages(
        jobId,
        pics.map((p) => p.path),
        { size, margin },
        out,
      );
    } catch (e) {
      const code = api.failureCode(e);
      if (code === "image") notice = imageError(e);
      else if (code !== "cancelled") notice = errorText(code);
    } finally {
      running = false;
      stopping = false;
      jobId = null;
    }
  }

  async function stop() {
    if (!jobId || stopping) return;
    stopping = true;
    await api.cancel(jobId).catch(() => {});
  }

  onMount(() => {
    let dispose: (() => void) | null = null;
    let gone = false;
    void api
      .onProgress((event) => {
        if (event.job_id === jobId) progress = { done: event.done, total: event.total };
      })
      .then((fn) => (gone ? fn() : (dispose = fn)))
      .catch(() => {});
    return () => {
      gone = true;
      dispose?.();
    };
  });

  onDestroy(() => {
    destroyed = true;
    thumbQueue = [];
    if (jobId) void api.cancel(jobId).catch(() => {});
    for (const pic of pics) if (pic.url) URL.revokeObjectURL(pic.url);
  });
</script>

{#if pics.length === 0}
  <div class="pv-drop">
    <svg class="pv-drop-art" viewBox="0 0 48 48" width="48" height="48" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
      <rect x="8" y="10" width="24" height="20" rx="2" />
      <path d="m8 26 7-7 6 6 4-4 7 7" />
      <path d="M30 22h10M36 18l4 4-4 4" />
    </svg>
    <h2 class="pv-drop-title">{$t("pdf.images_empty_title")}</h2>
    <p class="pv-drop-body">{$t("pdf.images_empty_body")}</p>
    <div class="pv-drop-actions">
      <button type="button" class="btn btn-primary" disabled={adding} onclick={chooseFiles}>{$t("pdf.choose_images")}</button>
      <button type="button" class="btn btn-secondary" disabled={adding} onclick={chooseFolder}>{$t("pdf.choose_folder")}</button>
    </div>
    {#if notice}<p class="pv-drop-body" role="alert">{notice}</p>{/if}
  </div>
{:else}
  <div class="pv-block">
    <h2 class="pv-block-title">{$t("pdf.page_size")}</h2>
    <div class="segmented" role="radiogroup" aria-label={$t("pdf.page_size") as string}>
      {#each SIZES as s (s)}
        <button type="button" class="segmented-btn" class:active={size === s} role="radio" aria-checked={size === s} disabled={running} onclick={() => (size = s)}>
          {$t(`pdf.size_${s}`)}
        </button>
      {/each}
    </div>
    <label class="pv-check">
      <input type="checkbox" bind:checked={margin} disabled={running} />
      <span>{$t("pdf.margin")}</span>
    </label>
  </div>

  {#if notice}
    <div class="pv-notice" role="alert">
      <StatusIcon name="warning" />
      <p>{notice}</p>
    </div>
  {/if}

  {#if result}
    <div class="pv-result" role="status">
      <StatusIcon name="done" size={20} />
      <div class="pv-result-copy">
        <h2>{$t("pdf.saved_one")}</h2>
        <p title={result.path}>{baseName(result.path)} · {formatBytes(result.size_bytes)}</p>
      </div>
      <div class="pv-result-actions">
        <button type="button" class="btn btn-secondary btn-sm" onclick={() => result && void api.openFile(result.path).catch(() => {})}>{$t("pdf.open")}</button>
        <button type="button" class="btn btn-secondary btn-sm" onclick={() => result && void api.reveal(result.path).catch(() => {})}>{$t("pdf.show_in_folder")}</button>
      </div>
    </div>
  {/if}

  <ol class="pv-pics" aria-label={$t("pdf.images_label") as string}>
    {#each pics as pic, i (pic.path)}
      <li class="pv-pic">
        <div class="pv-pic-frame">
          {#if pic.url}
            <img src={pic.url} alt="" draggable="false" />
          {:else if pic.failed}
            <StatusIcon name="warning" />
          {:else}
            <span class="spinner pv-spin" aria-hidden="true"></span>
          {/if}
        </div>
        <span class="pv-pic-name" title={pic.path}><b>{i + 1}</b> {baseName(pic.path)}</span>
        <div class="pv-pic-actions">
          <button type="button" class="pv-file-x" disabled={busy || i === 0} aria-label={$t("pdf.move_earlier") as string} onclick={() => move(i, -1)}>
            <svg viewBox="0 0 16 16" width="12" height="12" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M10 3 5 8l5 5" /></svg>
          </button>
          <button type="button" class="pv-file-x" disabled={busy || i === pics.length - 1} aria-label={$t("pdf.move_later") as string} onclick={() => move(i, 1)}>
            <svg viewBox="0 0 16 16" width="12" height="12" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="m6 3 5 5-5 5" /></svg>
          </button>
          <button type="button" class="pv-file-x" disabled={busy} aria-label={$t("pdf.remove_file", { name: baseName(pic.path) }) as string} onclick={() => remove(i)}>
            <svg viewBox="0 0 16 16" width="12" height="12" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" aria-hidden="true"><path d="m4 4 8 8M12 4l-8 8" /></svg>
          </button>
        </div>
      </li>
    {/each}
  </ol>

  <footer class="pv-bar">
    <div class="pv-bar-side">
      <button type="button" class="btn btn-secondary" disabled={busy} onclick={chooseFiles}>{$t("pdf.add_images")}</button>
      <button type="button" class="btn btn-ghost" disabled={busy} onclick={clearAll}>{$t("pdf.clear")}</button>
    </div>
    <div class="pv-bar-main">
      {#if running}
        <span class="pv-bar-note">{$t("pdf.images_progress", { done: progress.done, total: progress.total })}</span>
        <button type="button" class="btn btn-secondary" disabled={stopping} onclick={stop}>{stopping ? $t("pdf.stopping") : $t("pdf.stop")}</button>
      {:else}
        <button type="button" class="btn btn-primary" disabled={busy} onclick={build}>{plural($t, "pdf.images_build", pics.length)}</button>
      {/if}
    </div>
  </footer>
{/if}

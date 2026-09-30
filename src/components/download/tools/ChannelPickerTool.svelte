<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { t } from "$lib/i18n";
  import { pluralKey } from "$lib/home/plural";
  import { translateBackendError } from "$lib/error-translate";
  import { getSettings } from "$lib/stores/settings-store.svelte";
  import { showToast } from "$lib/stores/toast-store.svelte";

  type Video = {
    id: string;
    title: string;
    url: string;
    duration: number | null;
    view_count: number | null;
    is_short: boolean;
    live_status: string | null;
    downloaded: boolean;
  };
  type Page = {
    title: string;
    uploader: string | null;
    start: number;
    entries: Video[];
    has_more: boolean;
  };
  type Length = "any" | "short" | "medium" | "long";

  const PAGE = 100;
  const QUALITIES = ["best", "1080p", "720p", "480p"];

  let url = $state("");
  let listedUrl = $state("");
  let loading = $state(false);
  let error = $state<string | null>(null);
  let title = $state("");
  let videos = $state<Video[]>([]);
  let hasMore = $state(false);
  let picked = $state<Set<string>>(new Set());

  let search = $state("");
  let length = $state<Length>("any");
  let hideShorts = $state(false);
  let hideDone = $state(true);

  let mode = $state<"video" | "audio">("video");
  let quality = $state("1080p");
  let queueing = $state(false);

  function fitsLength(v: Video): boolean {
    if (length === "any") return true;
    if (v.duration == null) return false;
    if (length === "short") return v.duration < 240;
    if (length === "medium") return v.duration >= 240 && v.duration <= 1200;
    return v.duration > 1200;
  }

  const visible = $derived.by(() => {
    const q = search.trim().toLowerCase();
    return videos.filter(
      (v) =>
        (!q || v.title.toLowerCase().includes(q)) &&
        fitsLength(v) &&
        !(hideShorts && v.is_short) &&
        !(hideDone && v.downloaded),
    );
  });
  const pickedCount = $derived(videos.filter((v) => picked.has(v.id)).length);
  const doneCount = $derived(videos.filter((v) => v.downloaded).length);

  function selectable(v: Video): boolean {
    return v.live_status !== "is_upcoming" && v.live_status !== "is_live";
  }

  function clock(seconds: number | null): string {
    if (seconds == null) return "";
    const s = Math.round(seconds);
    const h = Math.floor(s / 3600);
    const m = Math.floor((s % 3600) / 60);
    const ss = String(s % 60).padStart(2, "0");
    return h > 0 ? `${h}:${String(m).padStart(2, "0")}:${ss}` : `${m}:${ss}`;
  }

  function views(n: number | null): string {
    if (n == null) return "";
    return new Intl.NumberFormat(undefined, { notation: "compact" }).format(n);
  }

  async function load(more = false) {
    const target = more ? listedUrl : url.trim();
    if (!target || loading) return;
    loading = true;
    error = null;
    try {
      const page = await invoke<Page>("list_channel_videos", {
        url: target,
        start: more ? videos.length + 1 : 1,
        count: PAGE,
      });
      if (more) {
        const seen = new Set(videos.map((v) => v.id));
        videos = [...videos, ...page.entries.filter((v) => !seen.has(v.id))];
      } else {
        videos = page.entries;
        picked = new Set();
        listedUrl = target;
        title = page.uploader && !page.title.includes(page.uploader) ? `${page.uploader} · ${page.title}` : page.title;
      }
      hasMore = page.has_more;
    } catch (e: any) {
      const msg = typeof e === "string" ? e : (e?.message ?? "");
      error = msg ? translateBackendError(msg, $t) : $t("downloads.tools.channel_picker.error");
    } finally {
      loading = false;
    }
  }

  function toggle(id: string) {
    const next = new Set(picked);
    if (next.has(id)) next.delete(id);
    else next.add(id);
    picked = next;
  }

  function selectVisible() {
    const next = new Set(picked);
    for (const v of visible) if (selectable(v)) next.add(v.id);
    picked = next;
  }

  async function downloadPicked() {
    const outputDir = getSettings()?.download.default_output_dir ?? "";
    if (!outputDir) {
      showToast("error", $t("downloads.tools.channel_picker.no_folder"));
      return;
    }
    const chosen = videos.filter((v) => picked.has(v.id));
    if (!chosen.length || queueing) return;
    queueing = true;
    let added = 0;
    let lastError = "";
    for (const v of chosen) {
      try {
        await invoke("download_from_url", {
          url: v.url,
          outputDir,
          downloadMode: mode === "audio" ? "audio" : null,
          quality: mode === "audio" ? "best" : quality,
          formatId: null,
          referer: null,
          cookieSlug: null,
        });
        added++;
      } catch (e: any) {
        lastError = typeof e === "string" ? e : (e?.message ?? "");
      }
    }
    queueing = false;
    if (added) {
      showToast("success", $t(pluralKey("downloads.tools.channel_picker.queued", added), { count: added }));
      picked = new Set();
    }
    if (lastError) showToast("error", translateBackendError(lastError, $t));
  }
</script>

<div class="cp" data-agent-readiness={loading || queueing ? "loading" : "ready"}>
  <p class="lead">{$t("downloads.tools.channel_picker.lead")}</p>
  <form
    class="row"
    onsubmit={(e) => {
      e.preventDefault();
      load(false);
    }}
  >
    <input
      type="url"
      bind:value={url}
      placeholder={$t("downloads.tools.channel_picker.placeholder")}
      aria-label={$t("downloads.tools.channel_picker.placeholder")}
      spellcheck="false"
      autocomplete="off"
    />
    <button class="button" type="submit" disabled={loading || !url.trim()}>
      {loading && !videos.length ? $t("downloads.tools.channel_picker.loading") : $t("downloads.tools.channel_picker.list")}
    </button>
  </form>

  {#if error}
    <p class="error" role="alert">{error}</p>
  {/if}

  {#if videos.length}
    <div class="head">
      <strong title={title}>{title}</strong>
      <span class="dim">
        {$t(pluralKey("downloads.tools.channel_picker.loaded", videos.length), { count: videos.length })}{#if doneCount}
          · {$t("downloads.tools.channel_picker.done_count", { count: doneCount })}{/if}
      </span>
    </div>

    <div class="filters">
      <input
        class="search"
        type="search"
        bind:value={search}
        placeholder={$t("downloads.tools.channel_picker.search")}
        aria-label={$t("downloads.tools.channel_picker.search")}
      />
      <select bind:value={length} aria-label={$t("downloads.tools.channel_picker.length")}>
        <option value="any">{$t("downloads.tools.channel_picker.length_any")}</option>
        <option value="short">{$t("downloads.tools.channel_picker.length_short")}</option>
        <option value="medium">{$t("downloads.tools.channel_picker.length_medium")}</option>
        <option value="long">{$t("downloads.tools.channel_picker.length_long")}</option>
      </select>
      <label class="check"><input type="checkbox" bind:checked={hideShorts} /> {$t("downloads.tools.channel_picker.hide_shorts")}</label>
      <label class="check"><input type="checkbox" bind:checked={hideDone} /> {$t("downloads.tools.channel_picker.hide_done")}</label>
    </div>

    <div class="bulk">
      <button class="link" onclick={selectVisible}>{$t("downloads.tools.channel_picker.select_visible", { count: visible.length })}</button>
      <button class="link" onclick={() => (picked = new Set())} disabled={!pickedCount}>{$t("downloads.tools.channel_picker.clear")}</button>
    </div>

    {#if visible.length}
      <ul class="list">
        {#each visible as v (v.id)}
          <li class="video" class:done={v.downloaded}>
            <label>
              <input type="checkbox" checked={picked.has(v.id)} disabled={!selectable(v)} onchange={() => toggle(v.id)} />
              <span class="name" title={v.title}>{v.title}</span>
            </label>
            {#if v.downloaded}<span class="badge">{$t("downloads.tools.channel_picker.downloaded")}</span>{/if}
            {#if v.is_short}<span class="badge">Shorts</span>{/if}
            {#if v.view_count != null}<span class="dim small">{$t("downloads.tools.channel_picker.views", { count: views(v.view_count) })}</span>{/if}
            <span class="time">{clock(v.duration)}</span>
          </li>
        {/each}
      </ul>
    {:else}
      <p class="dim">{$t("downloads.tools.channel_picker.none_visible")}</p>
    {/if}

    {#if hasMore}
      <button class="button more" onclick={() => load(true)} disabled={loading}>
        {loading ? $t("downloads.tools.channel_picker.loading") : $t("downloads.tools.channel_picker.load_more", { count: PAGE })}
      </button>
    {/if}

    <div class="actions">
      <select bind:value={mode} aria-label={$t("downloads.tools.channel_picker.mode")}>
        <option value="video">{$t("downloads.tools.channel_picker.mode_video")}</option>
        <option value="audio">{$t("downloads.tools.channel_picker.mode_audio")}</option>
      </select>
      {#if mode === "video"}
        <select bind:value={quality} aria-label={$t("downloads.tools.channel_picker.quality")}>
          {#each QUALITIES as q (q)}
            <option value={q}>{q === "best" ? $t("downloads.tools.channel_picker.best") : q}</option>
          {/each}
        </select>
      {/if}
      <button class="button primary" onclick={downloadPicked} disabled={!pickedCount || queueing}>
        {$t(pluralKey("downloads.tools.channel_picker.download", pickedCount), { count: pickedCount })}
      </button>
    </div>
  {/if}
</div>

<style>
  .cp {
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
    white-space: nowrap;
  }
  .row,
  .filters,
  .bulk,
  .actions {
    display: flex;
    gap: var(--space-2);
    align-items: center;
    flex-wrap: wrap;
  }
  .row input,
  .search,
  select {
    padding: var(--space-2) var(--space-3);
    border: none;
    border-radius: var(--radius-md, 8px);
    background: var(--surface);
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
    color: var(--text);
    font-size: var(--text-sm);
  }
  .row input {
    flex: 1;
    min-width: 0;
  }
  .search {
    flex: 1;
    min-width: 140px;
  }
  .row input:focus-visible,
  .search:focus-visible,
  select:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: 1px;
  }
  .check {
    display: flex;
    align-items: center;
    gap: 4px;
    font-size: var(--text-sm);
    color: var(--text-dim);
  }
  .error {
    margin: 0;
    color: var(--danger, #d33);
    font-size: var(--text-sm);
  }
  .head {
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-width: 0;
  }
  .head strong {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .list {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 2px;
    max-height: 420px;
    overflow-y: auto;
  }
  .video {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    padding: var(--space-1) var(--space-3);
    border-radius: var(--radius-md, 8px);
    background: var(--surface);
  }
  .video.done {
    opacity: 0.6;
  }
  .video label {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    flex: 1;
    min-width: 0;
    cursor: pointer;
  }
  .name {
    font-size: var(--text-sm);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .time {
    font-size: var(--text-sm);
    font-variant-numeric: tabular-nums;
    color: var(--text-dim);
    min-width: 44px;
    text-align: right;
  }
  .badge {
    font-size: 11px;
    padding: 1px 6px;
    border-radius: var(--radius-full);
    background: var(--fill-1);
    color: var(--text-dim);
    white-space: nowrap;
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
  .link:hover:not(:disabled) {
    background: var(--accent-soft);
  }
  .link:disabled {
    color: var(--text-dim);
    cursor: default;
  }
  .more {
    align-self: flex-start;
  }
  .actions {
    position: sticky;
    bottom: 0;
    padding-top: var(--space-2);
    background: var(--bg, transparent);
  }
</style>

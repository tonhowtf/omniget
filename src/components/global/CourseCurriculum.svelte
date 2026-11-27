<script lang="ts">
  import { t } from "$lib/i18n";
  import {
    collectDownloadable,
    countDrm,
    findThumbnail,
    platformVisual,
    type GdTreeNode,
  } from "$lib/global/platform-visuals";

  let {
    course,
    connectorId = null,
    connectorLabel = "",
    selected,
    collapsed,
    outDir,
    maxHeight = $bindable<number | null>(null),
    busy = false,
    showBack = false,
    onBack,
    onToggleNode,
    onToggleCollapse,
    onSelectAll,
    onSelectNone,
    onPickOutDir,
    onDownload,
  }: {
    course: GdTreeNode;
    connectorId?: string | null;
    connectorLabel?: string;
    selected: Set<string>;
    collapsed: Set<string>;
    outDir: string;
    maxHeight: number | null;
    busy?: boolean;
    showBack?: boolean;
    onBack?: () => void;
    onToggleNode: (node: GdTreeNode) => void;
    onToggleCollapse: (node: GdTreeNode) => void;
    onSelectAll: () => void;
    onSelectNone: () => void;
    onPickOutDir: () => void;
    onDownload: () => void;
  } = $props();

  const KINDS = new Set([
    "folder",
    "video",
    "audio",
    "file",
    "subtitle",
    "text",
    "link",
    "hls",
    "dash",
    "document",
    "image",
  ]);

  let visual = $derived(platformVisual(connectorId || course.title));
  let cover = $derived(findThumbnail(course));
  let imgFailed = $state(false);
  let downloadable = $derived(collectDownloadable(course));
  let drmTotal = $derived(countDrm(course));
  let selectedCount = $derived(downloadable.filter((n) => selected.has(n.id)).length);

  function leafIds(n: GdTreeNode): string[] {
    return collectDownloadable(n).map((x) => x.id);
  }

  function checkState(n: GdTreeNode): "all" | "some" | "none" {
    const ids = leafIds(n);
    if (!ids.length) return "none";
    const c = ids.filter((i) => selected.has(i)).length;
    return c === 0 ? "none" : c === ids.length ? "all" : "some";
  }

  function selectedIn(n: GdTreeNode): number {
    return leafIds(n).filter((i) => selected.has(i)).length;
  }

  function kindLabel(k: string) {
    return $t(`global_dl.kind.${KINDS.has(k) ? k : "file"}`);
  }

  function nodeTitle(n: GdTreeNode) {
    return n.source?.format === "ytdlp" ? n.title.replace(/\s*\(yt-dlp\)$/, "") : n.title;
  }

  function shortPath(p: string) {
    const parts = p.split(/[\\/]/).filter(Boolean);
    return parts.length > 2 ? `…/${parts.slice(-2).join("/")}` : p;
  }
</script>

{#snippet kindIcon(kind: string)}
  {#if kind === "video" || kind === "hls" || kind === "dash"}
    <svg class="k-ic" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
      <polygon points="5.5 3.5 13 8 5.5 12.5 5.5 3.5" />
    </svg>
  {:else if kind === "audio"}
    <svg class="k-ic" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
      <path d="M6 12V4l7-1.5V10.5" />
      <circle cx="4.5" cy="12" r="1.5" />
      <circle cx="11.5" cy="10.5" r="1.5" />
    </svg>
  {:else if kind === "subtitle"}
    <svg class="k-ic" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
      <rect x="2" y="3.5" width="12" height="9" rx="1.5" />
      <path d="M4.5 9.5h3M9.5 9.5h2M4.5 7h5" />
    </svg>
  {:else if kind === "document" || kind === "text"}
    <svg class="k-ic" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
      <path d="M4 2.5h5.5L13 6v7.5a1 1 0 0 1-1 1H4a1 1 0 0 1-1-1v-10a1 1 0 0 1 1-1z" />
      <path d="M5.5 9h5M5.5 11.5h3.5" />
    </svg>
  {:else}
    <svg class="k-ic" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
      <path d="M8 2.5v7.5M5 7l3 3 3-3M3 13h10" />
    </svg>
  {/if}
{/snippet}

{#snippet lessonItem(n: GdTreeNode, depth: number)}
  {@const isFolder = !!n.children?.length}
  {@const st = checkState(n)}
  {#if isFolder}
    {@const total = leafIds(n).length}
    {@const picked = selectedIn(n)}
    <li class="module" class:module-nested={depth > 0}>
      <div class="module-head">
        <button
          type="button"
          class="twisty"
          onclick={() => onToggleCollapse(n)}
          aria-label={$t("global_dl.toggle")}
          aria-expanded={!collapsed.has(n.id)}
        >
          <svg class="chev" class:chev-open={!collapsed.has(n.id)} viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
            <polyline points="6 4 10 8 6 12" />
          </svg>
        </button>
        <label class="module-check">
          <input
            type="checkbox"
            checked={st === "all"}
            indeterminate={st === "some"}
            disabled={total === 0}
            onchange={() => onToggleNode(n)}
          />
          <span class="module-title" title={nodeTitle(n)}>{nodeTitle(n)}</span>
        </label>
        <span class="module-count">{picked}/{total}</span>
      </div>
      {#if !collapsed.has(n.id)}
        <ul class="lesson-list">
          {#each n.children ?? [] as c (c.id)}
            {@render lessonItem(c, depth + 1)}
          {/each}
        </ul>
      {/if}
    </li>
  {:else}
    <li class="lesson" class:lesson-drm={n.drm}>
      <label class="lesson-row">
        <input
          type="checkbox"
          checked={selected.has(n.id)}
          disabled={n.drm || !n.source}
          onchange={() => onToggleNode(n)}
        />
        <span class="kind-pill kind-{n.kind}">
          {@render kindIcon(n.kind)}
          <span>{kindLabel(n.kind)}</span>
        </span>
        <span class="lesson-title" title={nodeTitle(n)}>{nodeTitle(n)}</span>
      </label>
      {#if n.drm}
        <span class="badge-drm" title={$t("global_dl.drm_tooltip")}>{$t("global_dl.drm")}</span>
      {:else if n.source?.format === "ytdlp"}
        <span class="note">{$t("global_dl.note_handoff")}</span>
      {:else if n.note}
        <span class="note">{n.note}</span>
      {/if}
    </li>
  {/if}
{/snippet}

<section class="curriculum">
  <div class="banner" style:background={visual.gradient}>
    {#if showBack && onBack}
      <button type="button" class="back-btn" onclick={onBack}>
        ← {$t("global_dl.back_to_courses")}
      </button>
    {/if}

    <div class="banner-main">
      <div class="thumb">
        {#if cover && !imgFailed}
          <img src={cover} alt="" decoding="async" onerror={() => (imgFailed = true)} />
        {:else}
          <div class="thumb-fallback" style:--emblem-color={visual.color} aria-hidden="true">
            <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round">
              {@html visual.iconPaths}
            </svg>
          </div>
        {/if}
      </div>

      <div class="banner-copy">
        <div class="banner-pills">
          {#if connectorLabel}
            <span class="pill-brand" style:--brand={visual.color}>{connectorLabel}</span>
          {/if}
          <span class="pill-meta">{$t("global_dl.items_count", { count: downloadable.length })}</span>
          {#if drmTotal > 0}
            <span class="pill-drm" title={$t("global_dl.drm_tooltip")}>
              {$t("global_dl.drm_count", { count: drmTotal })}
            </span>
          {/if}
        </div>
        <h2 title={course.title}>{course.title}</h2>
      </div>

      {#if downloadable.length > 1}
        <div class="banner-select">
          <button type="button" class="btn btn-ghost btn-sm" onclick={onSelectAll}>{$t("global_dl.select_all")}</button>
          <button type="button" class="btn btn-ghost btn-sm" onclick={onSelectNone}>{$t("global_dl.select_none")}</button>
        </div>
      {/if}
    </div>
  </div>

  <ul class="modules">
    {#each course.children ?? [] as child (child.id)}
      {@render lessonItem(child, 0)}
    {/each}
  </ul>

  <footer class="bar">
    <div class="bar-field">
      <span class="note">{$t("global_dl.folder")}</span>
      <button
        type="button"
        class="btn btn-ghost btn-sm path"
        onclick={onPickOutDir}
        title={outDir}
        aria-label={`${$t("global_dl.folder_change")}: ${outDir}`}
      >
        {shortPath(outDir)}
      </button>
    </div>

    <label class="bar-field">
      <span class="note">{$t("global_dl.quality")}</span>
      <select bind:value={maxHeight}>
        <option value={null}>{$t("global_dl.best_quality")}</option>
        <option value={1080}>1080p</option>
        <option value={720}>720p</option>
        <option value={480}>480p</option>
      </select>
    </label>

    <button
      type="button"
      class="btn btn-primary bar-cta"
      onclick={onDownload}
      disabled={selectedCount === 0 || busy}
    >
      {$t("global_dl.download_selected", { count: selectedCount })}
    </button>
  </footer>
</section>

<style>
  .curriculum {
    display: flex;
    flex-direction: column;
    background: var(--surface);
    border: 1px solid var(--hairline);
    border-radius: var(--radius-lg, 14px);
    overflow: hidden;
    max-height: 74vh;
  }

  .banner {
    padding: var(--space-4);
    border-bottom: 1px solid var(--hairline);
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
  }

  .back-btn {
    align-self: flex-start;
    background: color-mix(in srgb, var(--surface) 75%, transparent);
    color: var(--text);
    border: 1px solid var(--hairline);
    border-radius: var(--radius-full, 9999px);
    padding: 4px 12px;
    font-size: var(--text-xs);
    font-weight: 500;
    cursor: pointer;
  }

  .back-btn:hover {
    background: var(--surface-hi, var(--surface));
  }

  .banner-main {
    display: flex;
    align-items: center;
    gap: var(--space-4);
    flex-wrap: wrap;
  }

  .thumb {
    width: 112px;
    aspect-ratio: 16 / 9;
    border-radius: var(--radius-md, 8px);
    overflow: hidden;
    border: 1px solid var(--hairline);
    background: var(--surface);
    flex-shrink: 0;
  }

  .thumb img {
    width: 100%;
    height: 100%;
    object-fit: cover;
    display: block;
  }

  .thumb-fallback {
    width: 100%;
    height: 100%;
    display: grid;
    place-items: center;
    color: var(--emblem-color);
    background: color-mix(in srgb, var(--emblem-color) 16%, var(--surface));
  }

  .thumb-fallback svg {
    width: 26px;
    height: 26px;
  }

  .banner-copy {
    flex: 1;
    min-width: 200px;
    display: flex;
    flex-direction: column;
    gap: 6px;
  }

  .banner-copy h2 {
    margin: 0;
    font-size: var(--text-lg, 17px);
    font-weight: 600;
    line-height: 1.3;
  }

  .banner-pills {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
    align-items: center;
  }

  .pill-brand,
  .pill-meta,
  .pill-drm {
    font-size: 11px;
    font-weight: 600;
    padding: 2px 8px;
    border-radius: var(--radius-full, 9999px);
  }

  .pill-brand {
    background: color-mix(in srgb, var(--brand) 18%, var(--surface));
    color: var(--text);
    border: 1px solid color-mix(in srgb, var(--brand) 35%, transparent);
  }

  .pill-meta {
    background: var(--fill-2, var(--surface-hi));
    color: var(--text-dim);
  }

  .pill-drm {
    background: color-mix(in srgb, #ff453a 18%, transparent);
    color: var(--text);
  }

  .banner-select {
    display: flex;
    gap: 6px;
    flex-shrink: 0;
  }

  .modules {
    list-style: none;
    margin: 0;
    padding: var(--space-3) var(--space-4);
    overflow-y: auto;
    flex: 1;
    min-height: 0;
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
  }

  .module {
    border: 1px solid var(--hairline);
    border-radius: var(--radius-md, 10px);
    background: color-mix(in srgb, var(--bg) 45%, var(--surface));
    overflow: hidden;
  }

  .module-nested {
    margin: 4px 0 4px 18px;
  }

  .module-head {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 8px 12px;
    background: color-mix(in srgb, var(--surface-hi, var(--surface)) 55%, transparent);
  }

  .twisty {
    width: 22px;
    height: 22px;
    display: grid;
    place-items: center;
    background: none;
    border: none;
    color: var(--text-dim);
    cursor: pointer;
    padding: 0;
    border-radius: 6px;
  }

  .twisty:hover {
    background: var(--fill-2);
    color: var(--text);
  }

  .chev {
    width: 14px;
    height: 14px;
    transition: transform 140ms ease;
  }

  .chev-open {
    transform: rotate(90deg);
  }

  .module-check {
    display: flex;
    align-items: center;
    gap: 8px;
    flex: 1;
    min-width: 0;
    cursor: pointer;
  }

  .module-title {
    font-size: var(--text-sm);
    font-weight: 600;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .module-count {
    font-size: 11px;
    font-weight: 600;
    color: var(--text-dim);
    background: var(--fill-2);
    padding: 2px 8px;
    border-radius: var(--radius-full, 9999px);
    flex-shrink: 0;
  }

  .lesson-list {
    list-style: none;
    margin: 0;
    padding: 4px 8px 8px 28px;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }

  .lesson {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 5px 8px;
    border-radius: 7px;
  }

  .lesson:hover {
    background: color-mix(in srgb, var(--fill-2) 60%, transparent);
  }

  .lesson-drm {
    opacity: 0.55;
  }

  .lesson-row {
    display: flex;
    align-items: center;
    gap: 8px;
    flex: 1;
    min-width: 0;
    cursor: pointer;
  }

  .lesson-title {
    font-size: var(--text-sm);
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .kind-pill {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    font-size: 10px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.03em;
    padding: 2px 7px;
    border-radius: 6px;
    background: var(--fill-2);
    color: var(--text-dim);
    flex-shrink: 0;
  }

  .k-ic {
    width: 11px;
    height: 11px;
  }

  .kind-hls,
  .kind-dash,
  .kind-video {
    background: color-mix(in srgb, #0a84ff 16%, transparent);
    color: var(--text);
  }

  .kind-subtitle {
    background: color-mix(in srgb, #bf5af2 16%, transparent);
    color: var(--text);
  }

  .kind-document,
  .kind-file {
    background: color-mix(in srgb, #ff9f0a 16%, transparent);
    color: var(--text);
  }

  .badge-drm {
    font-size: 10px;
    font-weight: 600;
    padding: 2px 7px;
    border-radius: 6px;
    background: color-mix(in srgb, #ff453a 22%, transparent);
    cursor: help;
  }

  .note {
    color: var(--text-dim);
    font-size: var(--text-xs);
  }

  .bar {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-3);
    padding: var(--space-3) var(--space-4);
    border-top: 1px solid var(--hairline);
    background: color-mix(in srgb, var(--surface-hi, var(--surface)) 50%, var(--surface));
  }

  .bar-field {
    display: flex;
    align-items: center;
    gap: 6px;
  }

  .bar-field select {
    background: var(--surface);
    color: var(--text);
    border: 1px solid var(--hairline);
    border-radius: 8px;
    padding: 4px 8px;
    font-size: var(--text-sm);
  }

  .path {
    max-width: 220px;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .bar-cta {
    margin-left: auto;
  }
</style>

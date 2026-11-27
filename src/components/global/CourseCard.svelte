<script lang="ts">
  import { t } from "$lib/i18n";
  import {
    collectDownloadable,
    countDrm,
    countModules,
    findThumbnail,
    platformVisual,
    type GdTreeNode,
  } from "$lib/global/platform-visuals";
  import SpotlightCard from "./SpotlightCard.svelte";

  let {
    course,
    connectorId = null,
    busy = false,
    onInspect,
    onDownload,
  }: {
    course: GdTreeNode;
    connectorId?: string | null;
    busy?: boolean;
    onInspect: (course: GdTreeNode) => void;
    onDownload: (course: GdTreeNode) => void;
  } = $props();

  let visual = $derived(platformVisual(connectorId || course.title));
  let cover = $derived(findThumbnail(course));
  let imgFailed = $state(false);
  let lessonCount = $derived(collectDownloadable(course).length);
  let moduleCount = $derived(countModules(course));
  let drmCount = $derived(countDrm(course));
</script>

<SpotlightCard accent={visual.color} class="course-card-wrap">
  <article class="course-card">
    <button
      type="button"
      class="cover-btn"
      onclick={() => onInspect(course)}
      aria-label={course.title}
    >
      <div class="cover" style:background={visual.gradient}>
        {#if cover && !imgFailed}
          <img
            src={cover}
            alt=""
            loading="lazy"
            decoding="async"
            onerror={() => (imgFailed = true)}
          />
        {:else}
          <div class="cover-art" aria-hidden="true">
            <div class="cover-grid"></div>
            <div class="cover-emblem" style:--emblem-color={visual.color}>
              <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round">
                {@html visual.iconPaths}
              </svg>
            </div>
          </div>
        {/if}

        <div class="cover-scrim" aria-hidden="true"></div>

        <div class="cover-badges">
          <span class="badge-glass">
            {$t("global_dl.items_count", { count: lessonCount })}
          </span>
          {#if drmCount > 0}
            <span class="badge-drm" title={$t("global_dl.drm_tooltip")}>
              {$t("global_dl.drm")} · {drmCount}
            </span>
          {/if}
        </div>
      </div>
    </button>

    <div class="body">
      <button type="button" class="title-btn" onclick={() => onInspect(course)}>
        <h3 title={course.title}>{course.title}</h3>
      </button>

      {#if moduleCount > 0 || course.note}
        <p class="meta">
          {#if moduleCount > 0}
            <span>{$t("global_dl.modules_count", { count: moduleCount })}</span>
          {/if}
          {#if course.note}
            <span>{course.note}</span>
          {/if}
        </p>
      {/if}

      <div class="actions">
        <button
          type="button"
          class="btn btn-primary btn-sm action-main"
          onclick={() => onDownload(course)}
          disabled={busy || lessonCount === 0}
        >
          <svg class="btn-ic" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
            <path d="M8 2.5v7.5" />
            <path d="M4.75 7.25L8 10.5l3.25-3.25" />
            <path d="M3 13h10" />
          </svg>
          {$t("global_dl.download")}
        </button>
        <button
          type="button"
          class="btn btn-secondary btn-sm"
          onclick={() => onInspect(course)}
        >
          {$t("global_dl.lessons_button")}
        </button>
      </div>
    </div>
  </article>
</SpotlightCard>

<style>
  .course-card {
    display: flex;
    flex-direction: column;
    height: 100%;
  }

  .cover-btn {
    all: unset;
    cursor: pointer;
    display: block;
    width: 100%;
  }

  .cover-btn:focus-visible {
    outline: var(--focus-ring);
    outline-offset: -2px;
  }

  .cover {
    position: relative;
    aspect-ratio: 16 / 9;
    width: 100%;
    overflow: hidden;
    border-bottom: 1px solid var(--hairline);
  }

  .cover img {
    width: 100%;
    height: 100%;
    object-fit: cover;
    display: block;
    transition: transform 240ms var(--ease-out, ease-out);
  }

  .course-card:hover .cover img {
    transform: scale(1.03);
  }

  .cover-art {
    position: absolute;
    inset: 0;
    display: grid;
    place-items: center;
  }

  .cover-grid {
    position: absolute;
    inset: 0;
    background-image:
      linear-gradient(to right, color-mix(in srgb, var(--text) 5%, transparent) 1px, transparent 1px),
      linear-gradient(to bottom, color-mix(in srgb, var(--text) 5%, transparent) 1px, transparent 1px);
    background-size: 22px 22px;
    mask-image: radial-gradient(circle at 50% 45%, black 30%, transparent 85%);
  }

  .cover-emblem {
    position: relative;
    width: 48px;
    height: 48px;
    border-radius: 14px;
    display: grid;
    place-items: center;
    color: var(--emblem-color);
    background: color-mix(in srgb, var(--emblem-color) 16%, var(--surface));
    border: 1px solid color-mix(in srgb, var(--emblem-color) 35%, transparent);
    box-shadow: 0 8px 24px color-mix(in srgb, var(--emblem-color) 20%, transparent);
    transition: transform 220ms var(--ease-out, ease-out);
  }

  .course-card:hover .cover-emblem {
    transform: scale(1.06);
  }

  .cover-emblem svg {
    width: 24px;
    height: 24px;
  }

  .cover-scrim {
    position: absolute;
    inset: 0;
    background: linear-gradient(to top, rgba(0, 0, 0, 0.45) 0%, transparent 55%);
    pointer-events: none;
  }

  .cover-badges {
    position: absolute;
    left: 10px;
    right: 10px;
    bottom: 8px;
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 6px;
    pointer-events: none;
  }

  .badge-glass,
  .badge-drm {
    font-size: 11px;
    font-weight: 600;
    padding: 2px 8px;
    border-radius: var(--radius-full, 9999px);
    backdrop-filter: blur(8px);
    -webkit-backdrop-filter: blur(8px);
  }

  .badge-glass {
    background: rgba(16, 18, 23, 0.72);
    color: #f3f4f6;
    border: 1px solid rgba(255, 255, 255, 0.14);
  }

  .badge-drm {
    background: rgba(217, 70, 62, 0.82);
    color: #fff;
  }

  .body {
    padding: var(--space-3);
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    flex: 1;
  }

  .title-btn {
    all: unset;
    cursor: pointer;
    display: block;
  }

  .title-btn:focus-visible {
    outline: var(--focus-ring);
    outline-offset: 2px;
    border-radius: 4px;
  }

  h3 {
    margin: 0;
    font-size: var(--text-base);
    font-weight: 600;
    line-height: 1.35;
    color: var(--text);
    display: -webkit-box;
    line-clamp: 2;
    -webkit-line-clamp: 2;
    -webkit-box-orient: vertical;
    overflow: hidden;
  }

  .meta {
    margin: 0;
    font-size: var(--text-xs);
    color: var(--text-dim);
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
  }

  .actions {
    display: flex;
    gap: var(--space-2);
    margin-top: auto;
    padding-top: var(--space-1);
  }

  .action-main {
    flex: 1;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    gap: 6px;
  }

  .btn-ic {
    width: 14px;
    height: 14px;
    flex-shrink: 0;
  }

  @media (prefers-reduced-motion: reduce) {
    .cover img,
    .cover-emblem {
      transition: none;
    }
  }
</style>

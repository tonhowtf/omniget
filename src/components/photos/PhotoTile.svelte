<script lang="ts">
  import { t } from "$lib/i18n";
  import { thumbSrc } from "$lib/photos/api";
  import type { Photo } from "$lib/photos/types";

  let {
    photo,
    selected,
    badge = null,
    checkLabel,
    onopen,
    ontoggle,
  }: {
    photo: Photo;
    selected: boolean;
    badge?: string | null;
    checkLabel: string;
    onopen: (e: MouseEvent) => void;
    ontoggle: (e: MouseEvent) => void;
  } = $props();
</script>

<div class="pt" class:selected class:failed={!!photo.error}>
  <button type="button" class="pt-open" title={photo.error ?? photo.name} aria-label={photo.name} onclick={onopen}>
    {#if photo.thumb}
      <img src={thumbSrc(photo.thumb)} alt="" loading="lazy" decoding="async" draggable="false" />
    {:else}
      <span class="pt-fail">
        <svg viewBox="0 0 24 24" width="22" height="22" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
          <rect x="3" y="4" width="18" height="16" rx="2" />
          <path d="m3 16 5-5 4 4 3-3 6 6M4 4l16 16" />
        </svg>
        <span class="pt-fail-name">{photo.name}</span>
      </span>
    {/if}
  </button>
  <label class="pt-check" title={checkLabel}>
    <input
      type="checkbox"
      class="checkbox"
      checked={selected}
      aria-label={`${checkLabel}: ${photo.name}`}
      onclick={(e) => {
        e.preventDefault();
        ontoggle(e);
      }}
    />
  </label>
  {#if badge}
    <span class="pt-badge">{badge}</span>
  {:else if photo.edited}
    <span class="pt-badge pt-edited">{$t("photos.edited")}</span>
  {/if}
</div>

<style>
  .pt {
    position: relative;
    aspect-ratio: 1;
    overflow: hidden;
    border-radius: var(--radius-md);
    background: var(--fill-1);
  }

  .pt-open {
    display: block;
    width: 100%;
    height: 100%;
    padding: 0;
    border: none;
    background: none;
    cursor: pointer;
  }

  .pt-open:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: -2px;
  }

  .pt img {
    display: block;
    width: 100%;
    height: 100%;
    object-fit: cover;
    transition: transform var(--duration-fast) var(--ease-out);
  }

  .pt.selected {
    box-shadow: 0 0 0 3px var(--accent);
  }

  .pt.selected img {
    transform: scale(0.92);
    border-radius: var(--radius-sm);
  }

  .pt-fail {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: var(--space-1);
    height: 100%;
    padding: var(--space-2);
    color: var(--text-dim);
  }

  .pt-fail-name {
    max-width: 100%;
    overflow: hidden;
    font-size: var(--text-xs);
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .pt-check {
    position: absolute;
    top: 6px;
    left: 6px;
    display: flex;
    padding: 3px;
    border-radius: 6px;
    background: color-mix(in srgb, var(--pane-bg) 70%, transparent);
    opacity: 0;
    transition: opacity var(--duration-fast) var(--ease-out);
  }

  .pt:hover .pt-check,
  .pt:focus-within .pt-check,
  .pt.selected .pt-check {
    opacity: 1;
  }

  .pt-badge {
    position: absolute;
    right: 6px;
    bottom: 6px;
    padding: 1px 6px;
    border-radius: 999px;
    background: color-mix(in srgb, var(--pane-bg) 80%, transparent);
    color: var(--text);
    font-size: 10px;
    font-weight: 600;
    pointer-events: none;
  }

  .pt-edited {
    color: var(--accent);
  }

  @media (prefers-reduced-motion: reduce) {
    .pt img,
    .pt-check {
      transition: none;
    }
  }
</style>

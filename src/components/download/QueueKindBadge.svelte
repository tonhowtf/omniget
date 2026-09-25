<script lang="ts">
  import type { QueueKind } from "$lib/stores/download-store.svelte";

  let { kind = "generic", size = 14, showLabel = false }: {
    kind?: QueueKind | undefined;
    size?: number;
    showLabel?: boolean;
  } = $props();

  const safeKind = $derived<QueueKind>(kind ?? "generic");

  const labelMap: Record<QueueKind, string> = {
    video: "Video",
    audio: "Audio",
    image: "Image",
    pdf: "PDF",
    webpage: "Web",
    generic: "File",
  };

  const label = $derived(labelMap[safeKind]);
</script>

<span
  class="queue-kind-badge"
  class:show-label={showLabel}
  data-kind={safeKind}
  style:--size="{size}px"
  title={label}
  aria-label={label}
>
  {#if safeKind === "video"}
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
      <polygon points="23 7 16 12 23 17 23 7" fill="currentColor" stroke="none" />
      <rect x="1" y="5" width="15" height="14" rx="2" ry="2" />
    </svg>
  {:else if safeKind === "audio"}
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
      <path d="M9 18V5l12-2v13" />
      <circle cx="6" cy="18" r="3" fill="currentColor" stroke="none" />
      <circle cx="18" cy="16" r="3" fill="currentColor" stroke="none" />
    </svg>
  {:else if safeKind === "image"}
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
      <rect x="3" y="3" width="18" height="18" rx="2" ry="2" />
      <circle cx="8.5" cy="8.5" r="1.5" fill="currentColor" stroke="none" />
      <polyline points="21 15 16 10 5 21" />
    </svg>
  {:else if safeKind === "pdf"}
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
      <path d="M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8z" />
      <polyline points="14 2 14 8 20 8" />
      <text x="12" y="17" text-anchor="middle" font-size="6" font-weight="700" font-family="sans-serif" fill="currentColor" stroke="none">PDF</text>
    </svg>
  {:else if safeKind === "webpage"}
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
      <circle cx="12" cy="12" r="10" />
      <line x1="2" y1="12" x2="22" y2="12" />
      <path d="M12 2a15.3 15.3 0 0 1 4 10 15.3 15.3 0 0 1-4 10 15.3 15.3 0 0 1-4-10 15.3 15.3 0 0 1 4-10z" />
    </svg>
  {:else}
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
      <path d="M21 15v4a2 2 0 01-2 2H5a2 2 0 01-2-2v-4" />
      <polyline points="7 10 12 15 17 10" />
      <line x1="12" y1="15" x2="12" y2="3" />
    </svg>
  {/if}
  {#if showLabel}
    <span class="label">{label}</span>
  {/if}
</span>

<style>
  .queue-kind-badge {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    gap: 4px;
    width: var(--size, 14px);
    height: var(--size, 14px);
    border-radius: 4px;
    flex-shrink: 0;
    background: var(--badge-bg);
    color: var(--badge-fg);
    padding: 2px;
  }
  .queue-kind-badge.show-label {
    width: auto;
    padding: 2px 6px;
    font-size: 11px;
    font-weight: 600;
    letter-spacing: 0.02em;
  }
  .queue-kind-badge svg {
    width: 100%;
    height: 100%;
    display: block;
  }
  .queue-kind-badge.show-label svg {
    width: var(--size, 14px);
    height: var(--size, 14px);
  }
  .queue-kind-badge[data-kind="video"] {
    --badge-bg: var(--queue-kind-video-bg);
    --badge-fg: var(--queue-kind-video-fg);
  }
  .queue-kind-badge[data-kind="audio"] {
    --badge-bg: var(--queue-kind-audio-bg);
    --badge-fg: var(--queue-kind-audio-fg);
  }
  .queue-kind-badge[data-kind="image"] {
    --badge-bg: var(--queue-kind-image-bg);
    --badge-fg: var(--queue-kind-image-fg);
  }
  .queue-kind-badge[data-kind="pdf"] {
    --badge-bg: var(--queue-kind-pdf-bg);
    --badge-fg: var(--queue-kind-pdf-fg);
  }
  .queue-kind-badge[data-kind="webpage"] {
    --badge-bg: var(--queue-kind-webpage-bg);
    --badge-fg: var(--queue-kind-webpage-fg);
  }
  .queue-kind-badge[data-kind="generic"] {
    --badge-bg: var(--queue-kind-generic-bg);
    --badge-fg: var(--queue-kind-generic-fg);
  }
</style>

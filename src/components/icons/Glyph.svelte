<script lang="ts" module>
  export type GlyphName =
    | "search"
    | "close"
    | "star"
    | "star-fill"
    | "copy"
    | "check"
    | "download"
    | "warning"
    | "offline"
    | "folder"
    | "plus"
    | "trash"
    | "pencil"
    | "rotate"
    | "flip-h"
    | "flip-v"
    | "grid"
    | "info"
    | "refresh";
</script>

<script lang="ts">
  let { name, size = 16 }: { name: GlyphName; size?: number } = $props();
</script>

<svg class="ic-glyph" viewBox="0 0 16 16" width={size} height={size} fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
  {#if name === "search"}
    <circle cx="7" cy="7" r="4.5" /><path d="m10.5 10.5 3 3" />
  {:else if name === "close"}
    <path d="m4 4 8 8M12 4l-8 8" />
  {:else if name === "star"}
    <path d="m8 2 1.8 3.8 4.2.5-3.1 2.9.8 4.1L8 11.3l-3.7 2 .8-4.1L2 6.3l4.2-.5z" />
  {:else if name === "star-fill"}
    <path d="m8 2 1.8 3.8 4.2.5-3.1 2.9.8 4.1L8 11.3l-3.7 2 .8-4.1L2 6.3l4.2-.5z" fill="currentColor" />
  {:else if name === "copy"}
    <rect x="5.5" y="5.5" width="8" height="8" rx="1.5" /><path d="M10.5 3.5v-.5a1 1 0 0 0-1-1h-6a1 1 0 0 0-1 1v6a1 1 0 0 0 1 1h.5" />
  {:else if name === "check"}
    <path d="m3.5 8.3 2.8 2.7 6.2-6.5" />
  {:else if name === "download"}
    <path d="M8 2.5v8M4.8 7.5 8 10.7l3.2-3.2M3 13.5h10" />
  {:else if name === "warning"}
    <path d="M8 2.2 14.3 13H1.7z" /><path d="M8 6.4v3M8 11.2v.1" />
  {:else if name === "offline"}
    <path d="M2 6.2a9 9 0 0 1 12 0M4.3 8.7a5.5 5.5 0 0 1 7.4 0M6.5 11.2a2.2 2.2 0 0 1 3 0M2.5 2.5l11 11" />
  {:else if name === "folder"}
    <path d="M2 4.5a1 1 0 0 1 1-1h3.2l1.5 1.6H13a1 1 0 0 1 1 1V12a1 1 0 0 1-1 1H3a1 1 0 0 1-1-1z" />
  {:else if name === "plus"}
    <path d="M8 3v10M3 8h10" />
  {:else if name === "trash"}
    <path d="M3 4.5h10M6.5 4.5V3h3v1.5M4.5 4.5l.6 8.5h5.8l.6-8.5" />
  {:else if name === "pencil"}
    <path d="m10.5 2.8 2.7 2.7L6 12.7l-3.3.6.6-3.3z" />
  {:else if name === "rotate"}
    <path d="M13 8a5 5 0 1 1-1.5-3.6M13 2.5v2.8h-2.8" />
  {:else if name === "flip-h"}
    <path d="M8 2v12M6 4 2.5 12H6zM10 4l3.5 8H10z" />
  {:else if name === "flip-v"}
    <path d="M2 8h12M4 6l8-3.5V6zM4 10l8 3.5V10z" />
  {:else if name === "grid"}
    <rect x="2.5" y="2.5" width="4.5" height="4.5" rx="1" /><rect x="9" y="2.5" width="4.5" height="4.5" rx="1" /><rect x="2.5" y="9" width="4.5" height="4.5" rx="1" /><rect x="9" y="9" width="4.5" height="4.5" rx="1" />
  {:else if name === "info"}
    <circle cx="8" cy="8" r="6" /><path d="M8 7.2v3.6M8 5v.1" />
  {:else}
    <path d="M13 8a5 5 0 1 1-1.5-3.6M13 2.5v2.8h-2.8" />
  {/if}
</svg>

<style>
  .ic-glyph {
    display: block;
    flex-shrink: 0;
  }
</style>

<script lang="ts" module>
  export type DictationIcon = "mic" | "done" | "warning" | "error" | "copy" | "trash" | "stop" | "refresh" | "plus" | "close" | "clipboard" | "keyboard" | "download";
</script>

<script lang="ts">
  let { name, size = 16 }: { name: DictationIcon; size?: number } = $props();
</script>

<svg class="dc-icon" viewBox="0 0 16 16" width={size} height={size} fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
  {#if name === "mic"}
    <rect x="5.5" y="1.75" width="5" height="8.5" rx="2.5" />
    <path d="M3.25 7.75a4.75 4.75 0 0 0 9.5 0M8 12.5v2" />
  {:else if name === "done"}
    <circle cx="8" cy="8" r="6.5" />
    <path d="m5.2 8.2 1.9 1.9 3.7-4" />
  {:else if name === "warning"}
    <path d="M8 2.2 14.3 13H1.7z" />
    <path d="M8 6.4v3M8 11.2v.1" />
  {:else if name === "error"}
    <circle cx="8" cy="8" r="6.5" />
    <path d="M8 4.8v3.8M8 11.1v.1" />
  {:else if name === "copy"}
    <rect x="5.25" y="5.25" width="8.5" height="8.5" rx="1.5" />
    <path d="M10.75 5.25V3.5a1.25 1.25 0 0 0-1.25-1.25h-6A1.25 1.25 0 0 0 2.25 3.5v6a1.25 1.25 0 0 0 1.25 1.25h1.75" />
  {:else if name === "trash"}
    <path d="M2.75 4.25h10.5M6.25 4.25V2.75h3.5v1.5M4 4.25l.6 9h6.8l.6-9M6.75 6.75v4M9.25 6.75v4" />
  {:else if name === "stop"}
    <rect x="4" y="4" width="8" height="8" rx="1.5" />
  {:else if name === "refresh"}
    <path d="M13.25 8a5.25 5.25 0 1 1-1.54-3.71M13.25 2.5v3h-3" />
  {:else if name === "plus"}
    <path d="M8 3v10M3 8h10" />
  {:else if name === "close"}
    <path d="m4 4 8 8M12 4l-8 8" />
  {:else if name === "clipboard"}
    <rect x="3.25" y="2.75" width="9.5" height="11" rx="1.5" />
    <path d="M6 2.75h4v2H6zM5.75 8h4.5M5.75 10.5h3" />
  {:else if name === "keyboard"}
    <rect x="1.75" y="4" width="12.5" height="8" rx="1.5" />
    <path d="M4.5 6.75h.01M7 6.75h.01M9.5 6.75h.01M12 6.75h.01M5 9.5h6" />
  {:else if name === "download"}
    <path d="M8 2.5v8M4.75 7.5 8 10.75l3.25-3.25M3 13.25h10" />
  {/if}
</svg>

<style>
  .dc-icon {
    flex-shrink: 0;
  }
</style>

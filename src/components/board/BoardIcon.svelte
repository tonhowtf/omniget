<script lang="ts" module>
  export type BoardIconName =
    | "plus"
    | "folder"
    | "file"
    | "import"
    | "search"
    | "more"
    | "trash"
    | "copy"
    | "edit"
    | "export"
    | "check"
    | "warning"
    | "dot"
    | "reveal"
    | "close"
    | "board";
</script>

<script lang="ts">
  let { name, size = 16 }: { name: BoardIconName; size?: number } = $props();
</script>

<svg class="bi" viewBox="0 0 16 16" width={size} height={size} fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
  {#if name === "plus"}
    <path d="M8 3.2v9.6M3.2 8h9.6" />
  {:else if name === "folder"}
    <path d="M2 4.5A1.5 1.5 0 0 1 3.5 3h2.6l1.5 1.6h4.9A1.5 1.5 0 0 1 14 6.1v5.4a1.5 1.5 0 0 1-1.5 1.5h-9A1.5 1.5 0 0 1 2 11.5z" />
  {:else if name === "file"}
    <path d="M4.5 2h4.8L12 4.7v8.1a1.2 1.2 0 0 1-1.2 1.2H4.5a1.2 1.2 0 0 1-1.2-1.2V3.2A1.2 1.2 0 0 1 4.5 2z" />
    <path d="M9.2 2v2.8H12" />
  {:else if name === "import"}
    <path d="M8 2.5v7M5.2 6.8 8 9.6l2.8-2.8" />
    <path d="M3 10.5v1.8A1.2 1.2 0 0 0 4.2 13.5h7.6a1.2 1.2 0 0 0 1.2-1.2v-1.8" />
  {:else if name === "search"}
    <circle cx="7" cy="7" r="4.3" />
    <path d="m10.3 10.3 3.2 3.2" />
  {:else if name === "more"}
    <circle cx="3.6" cy="8" r=".9" fill="currentColor" stroke="none" />
    <circle cx="8" cy="8" r=".9" fill="currentColor" stroke="none" />
    <circle cx="12.4" cy="8" r=".9" fill="currentColor" stroke="none" />
  {:else if name === "trash"}
    <path d="M2.8 4.3h10.4M6.3 4.3V2.8h3.4v1.5M4.2 4.3l.6 8.5a1.1 1.1 0 0 0 1.1 1h4.2a1.1 1.1 0 0 0 1.1-1l.6-8.5" />
  {:else if name === "copy"}
    <rect x="5.3" y="5.3" width="8" height="8" rx="1.4" />
    <path d="M10.7 5.3V3.9a1.2 1.2 0 0 0-1.2-1.2H3.9a1.2 1.2 0 0 0-1.2 1.2v5.6a1.2 1.2 0 0 0 1.2 1.2h1.4" />
  {:else if name === "edit"}
    <path d="M10.4 2.9 13.1 5.6 5.8 12.9 2.6 13.4l.5-3.2z" />
  {:else if name === "export"}
    <path d="M8 10V2.6M5.2 5.2 8 2.4l2.8 2.8" />
    <path d="M3 9.5v2.8a1.2 1.2 0 0 0 1.2 1.2h7.6a1.2 1.2 0 0 0 1.2-1.2V9.5" />
  {:else if name === "check"}
    <circle cx="8" cy="8" r="6.3" />
    <path d="m5.3 8.2 1.8 1.8 3.6-3.9" />
  {:else if name === "warning"}
    <path d="M8 2.2 14.3 13H1.7z" />
    <path d="M8 6.4v3M8 11.2v.1" />
  {:else if name === "dot"}
    <circle cx="8" cy="8" r="3" fill="currentColor" stroke="none" />
  {:else if name === "reveal"}
    <path d="M9.5 2.5h4v4M13.5 2.5 7.5 8.5" />
    <path d="M12 9.5v2.8a1.2 1.2 0 0 1-1.2 1.2H3.7a1.2 1.2 0 0 1-1.2-1.2V5.2A1.2 1.2 0 0 1 3.7 4h2.8" />
  {:else if name === "close"}
    <path d="m4 4 8 8M12 4l-8 8" />
  {:else}
    <rect x="2" y="2.5" width="12" height="11" rx="1.8" />
    <path d="M4.6 10.4c1.2-2.6 2.4-3.6 3.4-1.6s2.2 1 3.4-1.8" />
  {/if}
</svg>

<style>
  .bi {
    flex-shrink: 0;
  }
</style>

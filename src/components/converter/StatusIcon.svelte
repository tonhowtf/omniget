<script lang="ts" module>
  export type StatusIconName = "ready" | "queued" | "running" | "done" | "failed" | "skipped" | "warning" | "stopped" | "idle";
</script>

<script lang="ts">
  let { name, size = 16 }: { name: StatusIconName; size?: number } = $props();
</script>

{#if name === "running"}
  <span class="spinner cv-spin" style:width="{size}px" style:height="{size}px" aria-hidden="true"></span>
{:else}
  <svg class="cv-icon cv-icon-{name}" viewBox="0 0 16 16" width={size} height={size} fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
    {#if name === "done"}
      <circle cx="8" cy="8" r="6.5" />
      <path d="m5.2 8.2 1.9 1.9 3.7-4" />
    {:else if name === "failed"}
      <circle cx="8" cy="8" r="6.5" />
      <path d="M8 4.8v3.8M8 11.1v.1" />
    {:else if name === "warning"}
      <path d="M8 2.2 14.3 13H1.7z" />
      <path d="M8 6.4v3M8 11.2v.1" />
    {:else if name === "skipped"}
      <circle cx="8" cy="8" r="6.5" />
      <path d="M5.5 8h5M8.8 6.2 10.6 8l-1.8 1.8" />
    {:else if name === "queued"}
      <circle cx="8" cy="8" r="6.5" />
      <path d="M8 4.8V8l2 1.4" />
    {:else if name === "stopped"}
      <circle cx="8" cy="8" r="6.5" />
      <path d="M6 6h4v4H6z" />
    {:else if name === "ready"}
      <circle cx="8" cy="8" r="6.5" />
      <path d="M5 8h6M8.6 5.6 11 8l-2.4 2.4" />
    {:else}
      <circle cx="8" cy="8" r="6.5" stroke-dasharray="2.4 2.2" />
    {/if}
  </svg>
{/if}

<style>
  .cv-icon {
    display: block;
    flex-shrink: 0;
  }

  .cv-spin {
    flex-shrink: 0;
  }

  .cv-icon-done {
    color: var(--success);
  }

  .cv-icon-failed {
    color: var(--error);
  }

  .cv-icon-warning {
    color: var(--warning);
  }

  .cv-icon-skipped,
  .cv-icon-stopped,
  .cv-icon-queued,
  .cv-icon-idle {
    color: var(--text-dim);
  }

  .cv-icon-ready {
    color: var(--accent-text);
  }
</style>

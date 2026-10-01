<script lang="ts">
  import { t } from "$lib/i18n";
  import { issueStatusColor, issueStatusKey } from "./company-format";

  let { status, label = true }: { status: string; label?: boolean } = $props();
</script>

<span class="st" style:color={issueStatusColor(status)} title={$t(issueStatusKey(status))}>
  <svg width="12" height="12" viewBox="0 0 12 12" aria-hidden="true">
    {#if status === "done"}
      <circle cx="6" cy="6" r="5.5" fill="currentColor" />
      <path d="M3.4 6.2 5.2 8l3.4-3.8" stroke="var(--surface)" stroke-width="1.4" fill="none" stroke-linecap="round" />
    {:else if status === "cancelled"}
      <circle cx="6" cy="6" r="5.5" fill="currentColor" />
      <path d="M4 4l4 4M8 4l-4 4" stroke="var(--surface)" stroke-width="1.3" stroke-linecap="round" />
    {:else if status === "in_progress"}
      <circle cx="6" cy="6" r="4.8" fill="none" stroke="currentColor" stroke-width="1.4" />
      <path d="M6 2.6A3.4 3.4 0 0 1 6 9.4Z" fill="currentColor" />
    {:else if status === "in_review"}
      <circle cx="6" cy="6" r="4.8" fill="none" stroke="currentColor" stroke-width="1.4" />
      <path d="M6 2.6A3.4 3.4 0 1 1 2.6 6H6Z" fill="currentColor" />
    {:else if status === "blocked"}
      <circle cx="6" cy="6" r="4.8" fill="none" stroke="currentColor" stroke-width="1.4" />
      <path d="M3.6 6h4.8" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" />
    {:else}
      <circle
        cx="6"
        cy="6"
        r="4.8"
        fill="none"
        stroke="currentColor"
        stroke-width="1.4"
        stroke-dasharray={status === "backlog" ? "2 1.8" : undefined}
      />
    {/if}
  </svg>
  {#if label}<span class="text">{$t(issueStatusKey(status))}</span>{/if}
</span>

<style>
  .st {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    flex-shrink: 0;
  }
  .text {
    font-size: var(--text-sm);
    color: var(--text);
    white-space: nowrap;
  }
</style>

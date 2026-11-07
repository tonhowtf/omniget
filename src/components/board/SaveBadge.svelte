<script lang="ts">
  import { t } from "$lib/i18n";
  import type { SaveStatus } from "$lib/board/autosave";
  import BoardIcon from "./BoardIcon.svelte";

  let { status, onretry }: { status: SaveStatus; onretry: () => void } = $props();

  let label = $derived.by(() => {
    switch (status.kind) {
      case "clean":
        return $t("board.status_clean") as string;
      case "dirty":
        return $t("board.status_dirty") as string;
      case "saving":
        return $t("board.status_saving") as string;
      case "saved":
        return $t("board.status_saved") as string;
      case "error":
        return $t("board.status_error", { reason: status.reason }) as string;
    }
  });
</script>

<div class="sb sb-{status.kind}" role="status" aria-live="polite">
  {#if status.kind === "saving"}
    <span class="spinner sb-spin" aria-hidden="true"></span>
  {:else if status.kind === "error"}
    <BoardIcon name="warning" size={14} />
  {:else if status.kind === "dirty"}
    <BoardIcon name="dot" size={14} />
  {:else}
    <BoardIcon name="check" size={14} />
  {/if}
  <span class="sb-text" title={label}>{label}</span>
  {#if status.kind === "error"}
    <button type="button" class="btn btn-ghost btn-sm sb-retry" onclick={onretry}>{$t("board.retry_save")}</button>
  {/if}
</div>

<style>
  .sb {
    display: inline-flex;
    align-items: center;
    gap: var(--space-1);
    min-width: 0;
    font-size: var(--text-sm);
    color: var(--text-dim);
  }

  .sb-text {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .sb-saved,
  .sb-clean {
    color: var(--text-dim);
  }

  .sb-saved :global(.bi) {
    color: var(--success);
  }

  .sb-error {
    color: var(--error);
    font-weight: 500;
  }

  .sb-dirty :global(.bi) {
    color: var(--accent);
  }

  .sb-spin {
    width: 12px;
    height: 12px;
    border-width: 1.5px;
  }

  .sb-retry {
    flex-shrink: 0;
  }
</style>

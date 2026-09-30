<script lang="ts">
  // Running history scans (whole-chat preview, auto-save backfill) with
  // their live numbers and a cancel button. `only` limits it to some ids.
  import { t } from "$lib/i18n";
  import TgIcon from "./TgIcon.svelte";
  import { scans, cancelScan } from "./session.svelte";

  let { only = null, compact = false }: { only?: string[] | null; compact?: boolean } = $props();

  let list = $derived(Object.values(scans).filter((s) => (only ? only.includes(s.id) : true)));
</script>

{#if list.length}
  <ul class="scans" class:compact>
    {#each list as s (s.id)}
      <li class="scan" class:done={s.done}>
        {#if s.done}<TgIcon name="check" size={14} />{:else}<span class="spinner"></span>{/if}
        <span class="s-body">
          {#if !compact}<span class="s-title">{$t(`telegram.scan.kind.${s.kind}`, { chat: s.chatTitle })}</span>{/if}
          <span class="s-count">
            {s.cancelled ? $t("telegram.scan.cancelled") : $t("telegram.scan.progress", { scanned: s.scanned.toLocaleString(), matched: s.matched.toLocaleString() })}
          </span>
        </span>
        {#if !s.done}
          <button class="btn btn-ghost btn-sm" onclick={() => void cancelScan(s.id)}>{$t("telegram.scan.stop")}</button>
        {/if}
      </li>
    {/each}
  </ul>
{/if}

<style>
  .scans {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
  }

  .scan {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    padding: 6px var(--space-3);
    border-radius: var(--radius-md);
    background: var(--fill-1);
    font-size: var(--text-sm);
    color: var(--text);
  }

  .scan.done {
    color: var(--text-dim);
  }

  .s-body {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
  }

  .compact .s-body {
    flex-direction: row;
  }

  .s-title {
    overflow: hidden;
    white-space: nowrap;
    text-overflow: ellipsis;
    font-weight: 550;
  }

  .s-count {
    color: var(--text-dim);
    font-variant-numeric: tabular-nums;
  }
</style>

<script lang="ts">
  /** Modal shell shared by the run dialogs (same look as the catalog's PlanDialog). */
  import type { Snippet } from "svelte";
  import { t } from "$lib/i18n";

  let {
    title,
    subtitle = "",
    busy = false,
    wide = false,
    onclose,
    children,
    footer,
  }: {
    title: string;
    subtitle?: string;
    busy?: boolean;
    wide?: boolean;
    onclose?: () => void;
    children: Snippet;
    footer?: Snippet;
  } = $props();

  const titleId = `run-dlg-${Math.random().toString(36).slice(2, 8)}`;

  function key(e: KeyboardEvent) {
    if (e.key === "Escape" && !busy) onclose?.();
  }
</script>

<svelte:window onkeydown={key} />

<div class="overlay" role="presentation" onclick={(e) => e.target === e.currentTarget && !busy && onclose?.()}>
  <div class="dialog" class:wide role="dialog" aria-modal="true" aria-labelledby={titleId} tabindex="-1">
    <header class="head">
      <div>
        <h2 id={titleId}>{title}</h2>
        {#if subtitle}<p class="sub">{subtitle}</p>{/if}
      </div>
      <button type="button" class="x" onclick={() => onclose?.()} aria-label={$t("llm.central.run.close")} disabled={busy}>✕</button>
    </header>
    <div class="body">{@render children()}</div>
    {#if footer}<footer class="foot">{@render footer()}</footer>{/if}
  </div>
</div>

<style>
  .overlay {
    position: fixed;
    inset: 0;
    z-index: 60;
    display: flex;
    align-items: center;
    justify-content: center;
    background: var(--dialog-backdrop);
  }
  .dialog {
    width: min(720px, 95vw);
    max-height: 90vh;
    display: flex;
    flex-direction: column;
    background: var(--popup-bg);
    border-radius: var(--radius-xl);
    box-shadow: var(--elev-3);
    outline: none;
  }
  .dialog.wide {
    width: min(980px, 95vw);
  }
  .head {
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
    gap: var(--space-3);
    padding: var(--space-4) var(--space-5) var(--space-2);
  }
  h2 {
    margin: 0;
    font-size: var(--text-lg);
  }
  .sub {
    margin: 2px 0 0;
    font-size: var(--text-sm);
    color: var(--text-muted);
  }
  .x {
    border: none;
    background: var(--fill-1);
    width: 28px;
    height: 28px;
    border-radius: var(--radius-full);
    color: var(--text);
    cursor: pointer;
  }
  .body {
    flex: 1;
    min-height: 0;
    overflow: auto;
    padding: var(--space-2) var(--space-5) var(--space-4);
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
  }
  .foot {
    display: flex;
    justify-content: flex-end;
    gap: var(--space-2);
    padding: var(--space-3) var(--space-5) var(--space-4);
    border-top: var(--hairline) solid var(--separator);
  }
</style>

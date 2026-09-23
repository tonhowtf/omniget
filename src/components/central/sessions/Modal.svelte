<script lang="ts">
  /**
   * Janela modal sobre `<dialog>` nativo: foco preso, Esc fecha, fundo inerte.
   */
  import type { Snippet } from "svelte";
  import { t } from "$lib/i18n";

  interface Props {
    title: string;
    width?: number;
    onclose: () => void;
    children: Snippet;
    actions?: Snippet;
  }

  let { title, width = 560, onclose, children, actions }: Props = $props();
  let el: HTMLDialogElement | undefined = $state();

  $effect(() => {
    if (el && !el.open) el.showModal();
  });
</script>

<dialog
  bind:this={el}
  class="modal"
  style:--w="{width}px"
  aria-label={title}
  onclose={onclose}
  onclick={(e) => {
    if (e.target === el) el?.close();
  }}
>
  <div class="card">
    <header>
      <h2 class="dialog-title">{title}</h2>
      <button type="button" class="btn btn-icon btn-ghost btn-sm" aria-label={$t("llm.central.sessions.close")} onclick={() => el?.close()}>✕</button>
    </header>
    <div class="body">{@render children()}</div>
    {#if actions}<footer class="dialog-actions">{@render actions()}</footer>{/if}
  </div>
</dialog>

<style>
  .modal {
    padding: 0;
    border: none;
    background: transparent;
    max-width: calc(100vw - 24px);
    max-height: calc(100vh - 24px);
    width: min(var(--w), calc(100vw - 24px));
    color: inherit;
  }
  .modal::backdrop {
    background: var(--dialog-backdrop);
  }
  .card {
    background: var(--popup-bg);
    border-radius: var(--radius-xl);
    box-shadow: 0 12px 40px rgba(var(--shadow-ink), var(--elev-alpha-3)), inset 0 0 0 1px var(--content-border);
    padding: var(--space-4) var(--space-5) var(--space-5);
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
    max-height: calc(100vh - 24px);
    box-sizing: border-box;
  }
  header {
    display: flex;
    justify-content: space-between;
    align-items: center;
    gap: 12px;
  }
  .body {
    overflow: auto;
    min-height: 0;
    font-size: var(--text-base);
    color: var(--text-muted);
  }
  @media (max-width: 640px) {
    .card {
      padding: var(--space-3);
    }
  }
</style>

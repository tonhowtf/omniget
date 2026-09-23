<script lang="ts">
  // Native <dialog> (focus trap and Esc for free) styled for the threads UI.
  import type { Snippet } from "svelte";

  let {
    open,
    title,
    onclose,
    width = 520,
    children,
    footer,
  }: { open: boolean; title: string; onclose: () => void; width?: number; children: Snippet; footer?: Snippet } = $props();

  let el = $state<HTMLDialogElement | null>(null);
  let returnTo: HTMLElement | null = null;

  $effect(() => {
    if (!el) return;
    if (open && !el.open) {
      returnTo = document.activeElement as HTMLElement | null;
      el.showModal();
    } else if (!open && el.open) {
      el.close();
      returnTo?.focus?.();
    }
  });
</script>

<dialog
  bind:this={el}
  class="modal"
  style:--w="{width}px"
  aria-label={title}
  oncancel={(e) => {
    e.preventDefault();
    onclose();
  }}
  onclick={(e) => {
    if (e.target === el) onclose();
  }}
>
  {#if open}
    <div class="inner">
      <h2>{title}</h2>
      <div class="body">{@render children()}</div>
      {#if footer}<div class="foot">{@render footer()}</div>{/if}
    </div>
  {/if}
</dialog>

<style>
  .modal { border: 0; padding: 0; background: transparent; max-width: min(var(--w), calc(100vw - 32px)); width: 100%; color: var(--text); }
  .modal::backdrop { background: var(--dialog-backdrop, rgba(0, 0, 0, 0.35)); backdrop-filter: blur(2px); }
  .inner { background: var(--popup-bg, var(--surface)); border: 1px solid var(--separator); border-radius: 16px; box-shadow: 0 24px 60px color-mix(in srgb, #000 30%, transparent); padding: 18px 20px 16px; display: grid; gap: 14px; max-height: calc(100vh - 64px); overflow: auto; }
  h2 { margin: 0; font-size: 16px; font-weight: 650; }
  .body { display: grid; gap: 12px; }
  .foot { display: flex; justify-content: flex-end; gap: 8px; flex-wrap: wrap; }
</style>

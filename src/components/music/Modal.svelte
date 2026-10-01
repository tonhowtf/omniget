<script lang="ts">
  import { onMount, tick, type Snippet } from "svelte";

  let {
    title,
    width = 440,
    onclose,
    children,
  }: { title: string; width?: number; onclose: () => void; children: Snippet } = $props();

  let card: HTMLDivElement | undefined = $state();
  const id = `mm-${Math.random().toString(36).slice(2, 8)}`;

  onMount(() => {
    const prev = document.activeElement as HTMLElement | null;
    void tick().then(() => card?.querySelector<HTMLElement>("input, select, textarea, button")?.focus());
    const key = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.stopPropagation();
        onclose();
      }
      if (e.key === "Tab" && card) {
        const f = [...card.querySelectorAll<HTMLElement>("input, select, textarea, button, [tabindex='0']")].filter((x) => !x.hasAttribute("disabled"));
        if (!f.length) return;
        const first = f[0];
        const last = f[f.length - 1];
        if (e.shiftKey && document.activeElement === first) {
          last.focus();
          e.preventDefault();
        } else if (!e.shiftKey && document.activeElement === last) {
          first.focus();
          e.preventDefault();
        }
      }
    };
    window.addEventListener("keydown", key, true);
    return () => {
      window.removeEventListener("keydown", key, true);
      prev?.focus?.();
    };
  });
</script>

<div
  class="dialog-backdrop"
  role="presentation"
  onpointerdown={(e) => {
    if (e.target === e.currentTarget) onclose();
  }}
>
  <div class="dialog-card mm" bind:this={card} role="dialog" aria-modal="true" aria-labelledby={id} style:width="min({width}px, 100%)">
    <h2 class="dialog-title" {id}>{title}</h2>
    {@render children()}
  </div>
</div>

<style>
  .mm {
    max-height: 86vh;
    overflow-y: auto;
  }
</style>

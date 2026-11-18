<script lang="ts">
  import { onMount, tick, type Snippet } from "svelte";
  import { t } from "$lib/i18n";
  import WaIcon from "./WaIcon.svelte";

  let {
    title,
    subtitle = "",
    width = 560,
    onclose,
    children,
    footer,
  }: {
    title: string;
    subtitle?: string;
    width?: number;
    onclose: () => void;
    children: Snippet;
    footer?: Snippet;
  } = $props();

  let card: HTMLDivElement | undefined = $state();
  const id = `wam-${Math.random().toString(36).slice(2, 8)}`;

  onMount(() => {
    const prev = document.activeElement as HTMLElement | null;
    void tick().then(() => card?.querySelector<HTMLElement>(".wam-body input, .wam-body select, .wam-body textarea, .wam-body button")?.focus());
    const key = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.stopPropagation();
        onclose();
      }
      if (e.key === "Tab" && card) {
        const f = [...card.querySelectorAll<HTMLElement>("input, select, textarea, button, [tabindex='0']")].filter(
          (x) => !x.hasAttribute("disabled"),
        );
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
  <div class="dialog-card wam" bind:this={card} role="dialog" aria-modal="true" aria-labelledby={id} style:width="min({width}px, 100%)">
    <header class="wam-head">
      <div class="wam-titles">
        <h2 class="dialog-title" {id}>{title}</h2>
        {#if subtitle}<p class="wam-sub">{subtitle}</p>{/if}
      </div>
      <button class="btn btn-ghost btn-sm wam-x" onclick={onclose} aria-label={$t("phonechat.close")}><WaIcon name="x" size={16} /></button>
    </header>
    <div class="wam-body">
      {@render children()}
    </div>
    {#if footer}
      <footer class="wam-foot">{@render footer()}</footer>
    {/if}
  </div>
</div>

<style>
  .wam {
    max-height: 88vh;
    padding: 0;
    gap: 0;
    overflow: hidden;
  }

  .wam-head {
    display: flex;
    align-items: flex-start;
    gap: var(--space-3);
    padding: var(--space-4) var(--space-5) var(--space-3);
  }

  .wam-titles {
    flex: 1;
    min-width: 0;
  }

  .wam-sub {
    margin: 2px 0 0;
    font-size: var(--text-sm);
    color: var(--text-dim);
    line-height: 1.45;
  }

  .wam-x {
    margin: -4px -8px 0 0;
  }

  .wam-body {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    padding: 0 var(--space-5) var(--space-4);
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
  }

  .wam-foot {
    display: flex;
    align-items: center;
    justify-content: flex-end;
    gap: var(--space-2);
    padding: var(--space-3) var(--space-5);
    border-top: var(--hairline) solid var(--separator);
    background: color-mix(in srgb, var(--popup-bg) 92%, var(--fill-1));
  }
</style>

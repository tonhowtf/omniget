<script lang="ts">
  /**
   * Catalog shell: the list and the detail share the stack drawer and its
   * floating button (the stack persists across both and across restarts).
   */
  import { onMount, type Snippet } from "svelte";
  import { t } from "$lib/i18n";
  import StackDrawer from "$components/central/catalog/StackDrawer.svelte";
  import { agentkitTargets, type TargetAdapter } from "$lib/central/catalog";
  import { stack } from "$lib/central/stack-store.svelte";

  let { children }: { children: Snippet } = $props();
  let targets = $state<TargetAdapter[]>([]);

  onMount(async () => {
    try {
      targets = await agentkitTargets();
    } catch {
      targets = [];
    }
  });
</script>

<div class="catalog-root">{@render children()}</div>

{#if !stack.open}
  <button
    type="button"
    class="stack-fab"
    class:empty={stack.count === 0}
    onclick={() => (stack.open = true)}
    aria-label={$t("llm.central.catalog.stack.open", { count: String(stack.count) })}
  >
    <span class="glyph" aria-hidden="true"></span>
    <span>{$t("llm.central.catalog.stack.title")}</span>
    <span class="n tabular">{stack.count}</span>
  </button>
{/if}

<StackDrawer {targets} oncollections={() => stack.rev++} />

<style>
  .catalog-root {
    flex: 1;
    min-height: 0;
    display: flex;
    flex-direction: column;
    overflow: hidden;
  }
  .stack-fab {
    position: fixed;
    right: var(--space-5);
    bottom: calc(var(--space-5) + var(--safe-area-bottom));
    z-index: 30;
    display: inline-flex;
    align-items: center;
    gap: var(--space-2);
    height: 40px;
    padding: 0 var(--space-4) 0 var(--space-3);
    border: none;
    border-radius: var(--radius-full);
    background: var(--cta);
    color: var(--on-cta);
    font-size: var(--text-base);
    font-weight: 600;
    box-shadow: var(--elev-2);
    cursor: pointer;
    transition: transform var(--duration-fast) var(--ease-spring);
  }
  .stack-fab:hover {
    transform: translateY(-1px);
    background: var(--cta-hover);
  }
  .stack-fab:focus-visible {
    outline: var(--focus-ring);
    outline-offset: var(--focus-ring-offset);
  }
  .stack-fab.empty {
    background: var(--surface-hi);
    color: var(--text);
  }
  .glyph {
    width: 18px;
    height: 18px;
    background: currentColor;
    -webkit-mask: url(/icons/stack.svg) center / contain no-repeat;
    mask: url(/icons/stack.svg) center / contain no-repeat;
  }
  .n {
    min-width: 20px;
    padding: 0 6px;
    border-radius: var(--radius-full);
    background: color-mix(in srgb, currentColor 18%, transparent);
    font-size: var(--text-sm);
    text-align: center;
  }
</style>

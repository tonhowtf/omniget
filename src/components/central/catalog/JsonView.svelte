<script lang="ts">
  /** JSON viewer: collapsible tree (first two levels open) or raw code, copy whole or section. */
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import CodeView from "./CodeView.svelte";
  import FindBar from "./FindBar.svelte";
  import JsonNode from "./JsonNode.svelte";

  let { text = "", maxHeight = "70vh" }: { text?: string; maxHeight?: string } = $props();

  let parsed = $derived.by(() => {
    try {
      return { ok: true as const, value: JSON.parse(text) as unknown };
    } catch {
      return { ok: false as const, value: null };
    }
  });
  let mode = $state<"tree" | "code">("tree");
  let findOpen = $state(false);
  let root = $state<HTMLElement | null>(null);
  let codeEl = $state<HTMLElement | null>(null);

  async function copy(v: unknown) {
    try {
      await navigator.clipboard.writeText(typeof v === "string" ? v : JSON.stringify(v, null, 2));
      showToast("success", $t("llm.central.catalog.copied"));
    } catch {
      showToast("error", $t("llm.central.catalog.copy_failed"));
    }
  }

  function onkey(e: KeyboardEvent) {
    if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === "f") {
      e.preventDefault();
      findOpen = true;
    }
  }
</script>

<svelte:window onkeydown={onkey} />

<div class="json">
  <div class="bar">
    <div class="seg" role="tablist" aria-label={$t("llm.central.catalog.view.mode")}>
      <button type="button" role="tab" aria-selected={mode === "tree"} onclick={() => (mode = "tree")} disabled={!parsed.ok}>{$t("llm.central.catalog.view.tree")}</button>
      <button type="button" role="tab" aria-selected={mode === "code" || !parsed.ok} onclick={() => (mode = "code")}>{$t("llm.central.catalog.view.code")}</button>
    </div>
    <span class="spacer"></span>
    <button type="button" class="ghost" onclick={() => (findOpen = true)}>{$t("llm.central.catalog.find.open")}</button>
    <button type="button" class="ghost" onclick={() => copy(text)}>{$t("llm.central.catalog.copy")}</button>
  </div>
  <div class="scroller" style:max-height={maxHeight}>
    <FindBar root={mode === "tree" && parsed.ok ? root : codeEl} bind:open={findOpen} version={mode === "tree" ? 1 : 2} />
    {#if mode === "tree" && parsed.ok}
      <div class="tree" bind:this={root}>
        <JsonNode value={parsed.value} oncopy={copy} />
      </div>
    {:else}
      <CodeView {text} bind:el={codeEl} />
    {/if}
  </div>
</div>

<style>
  .json {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    min-width: 0;
  }
  .bar {
    display: flex;
    align-items: center;
    gap: var(--space-1);
  }
  .spacer {
    flex: 1;
  }
  .seg {
    display: inline-flex;
    padding: 2px;
    border-radius: var(--radius-md);
    background: var(--fill-1);
  }
  .seg button {
    height: 24px;
    padding: 0 var(--space-3);
    border: none;
    border-radius: var(--radius-sm);
    background: transparent;
    color: var(--text-muted);
    font-size: var(--text-sm);
    cursor: pointer;
  }
  .seg button[aria-selected="true"] {
    background: var(--surface-hi);
    color: var(--text);
    box-shadow: var(--elev-1);
  }
  .ghost {
    height: 24px;
    padding: 0 var(--space-2);
    border: none;
    border-radius: var(--radius-sm);
    background: transparent;
    color: var(--text-muted);
    font-size: var(--text-sm);
    cursor: pointer;
  }
  .ghost:hover {
    background: var(--fill-1);
    color: var(--text);
  }
  .scroller {
    overflow: auto;
  }
  .tree {
    padding: var(--space-2);
    border-radius: var(--radius-md);
    background: var(--surface-mut);
  }
</style>

<script lang="ts">
  // A web link: title and host; opens in the system browser.
  import { open as openUrl } from "@tauri-apps/plugin-shell";
  import { t } from "$lib/i18n";
  import type { CanvasNode } from "$lib/canvas/model/types";
  import { useEngine } from "$lib/canvas/engine.svelte";
  import { openLinkInPortal } from "$lib/canvas/menu/model";

  let { node }: { node: CanvasNode } = $props();
  const engine = useEngine();

  const url = $derived(String(node.data.url ?? ""));
  const host = $derived.by(() => {
    try {
      return new URL(url).host;
    } catch {
      return url;
    }
  });

  function open(e: MouseEvent) {
    e.preventDefault();
    e.stopPropagation();
    if (/^https?:\/\//i.test(url)) void openUrl(url).catch(() => {});
  }

  // ⇧ + double click opens the link in a portal next to the node (Maestri).
  function dbl(e: MouseEvent) {
    if (!e.shiftKey || !url) return;
    e.preventDefault();
    e.stopPropagation();
    openLinkInPortal(engine, node.id);
  }
</script>

<!-- svelte-ignore a11y_no_static_element_interactions -->
<div class="link" ondblclick={dbl}>
  <div class="title">{node.title || host || $t("canvas.engine.linkEmpty", { default: "Link" })}</div>
  {#if url}
    <button type="button" class="url" onclick={open} title={url}>{host}</button>
  {/if}
</div>

<style>
  .link {
    height: 100%;
    padding: 8px 12px;
    display: flex;
    flex-direction: column;
    justify-content: center;
    gap: 2px;
    min-width: 0;
    overflow: hidden;
    color: var(--text);
  }
  .title {
    font-weight: 600;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .url {
    all: unset;
    display: block;
    max-width: 100%;
    cursor: pointer;
    color: var(--accent-text, var(--accent));
    font-size: 12px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .url:hover {
    text-decoration: underline;
  }
</style>

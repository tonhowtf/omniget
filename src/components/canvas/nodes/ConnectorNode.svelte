<script lang="ts">
  import { emit } from "@tauri-apps/api/event";
  import { t } from "$lib/i18n";
  import type { CanvasNode } from "$lib/canvas/model/types";

  let { node }: { node: CanvasNode } = $props();

  const route = $derived(String(node.data.route ?? ""));
  const remoteWs = $derived(typeof node.data.remoteWorkspaceId === "string" ? node.data.remoteWorkspaceId : "");
  const remoteFloor = $derived(typeof node.data.remoteFloorId === "string" ? node.data.remoteFloorId : "");
  const remoteNode = $derived(typeof node.data.remoteNodeId === "string" ? node.data.remoteNodeId : "");

  function go(e: MouseEvent) {
    e.preventDefault();
    e.stopPropagation();
    if (!remoteWs) return;
    void emit("canvas://navigate", { workspaceId: remoteWs, floorId: remoteFloor || null, nodeId: remoteNode || null });
  }
</script>

<div class="chip" data-route={route}>
  <span class="glyph" aria-hidden="true">{route === "crossWorkspace" ? "⇄" : "⇅"}</span>
  <span class="title">{node.title || $t("canvas.link.connector", { default: "Connector" })}</span>
  <button
    type="button"
    class="go"
    onclick={go}
    title={$t("canvas.link.goToOtherEnd", { default: "Go to the other end" })}
    aria-label={$t("canvas.link.goToOtherEnd", { default: "Go to the other end" })}>→</button
  >
</div>

<style>
  .chip {
    height: 100%;
    box-sizing: border-box;
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 0 8px 0 12px;
    border-radius: 999px;
    background: var(--popup-bg, var(--surface));
    border: 1px dashed var(--border);
    color: var(--text);
    font-size: 13px;
  }
  .glyph {
    color: var(--accent-text, var(--accent));
    font-size: 15px;
  }
  .title {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-weight: 600;
  }
  .go {
    all: unset;
    cursor: pointer;
    width: 24px;
    height: 24px;
    border-radius: 12px;
    text-align: center;
    line-height: 24px;
    color: var(--accent-text, var(--accent));
  }
  .go:hover {
    background: var(--border);
  }
</style>

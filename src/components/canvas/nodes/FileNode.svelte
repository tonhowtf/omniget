<script lang="ts">
  import { convertFileSrc, invoke } from "@tauri-apps/api/core";
  import { t } from "$lib/i18n";
  import type { CanvasNode } from "$lib/canvas/model/types";
  import { useEngine } from "$lib/canvas/engine.svelte";

  let { node }: { node: CanvasNode } = $props();
  const engine = useEngine();

  const path = $derived(String(node.data.path ?? ""));
  const name = $derived(path.split(/[\\/]/).filter(Boolean).pop() ?? node.title ?? "");
  const ext = $derived(name.includes(".") ? name.split(".").pop()?.toUpperCase() : "");
  const IMAGE = /\.(png|jpe?g|gif|webp|svg|bmp|ico|avif)$/i;
  const isImage = $derived(node.data.isImage === true || IMAGE.test(path));

  let preview = $state<string | null>(null);

  $effect(() => {
    const p = path;
    const img = isImage;
    const ws = engine.store.workspaceId;
    const floor = engine.store.floorId;
    const id = node.id;
    preview = null;
    if (!img || !p || engine.syncMode !== "rust") return;
    let alive = true;
    invoke<string | null>("canvas_file_node_preview", { workspaceId: ws, floorId: floor, nodeId: id })
      .then((thumb) => {
        if (alive) preview = !thumb ? null : thumb.startsWith("data:") ? thumb : convertFileSrc(thumb);
      })
      .catch(() => {});
    return () => {
      alive = false;
    };
  });
</script>

<div class="file" class:has-preview={!!preview}>
  {#if preview}
    <img class="preview" src={preview} alt={name} draggable="false" />
    <div class="caption"><bdi>{name}</bdi></div>
  {:else}
    <div class="badge">{ext || "FILE"}</div>
    <div class="meta">
      <div class="name">{name || $t("canvas.engine.fileMissing", { default: "No file" })}</div>
      {#if path}<div class="path" title={path}><bdi dir="ltr">{path}</bdi></div>{/if}
    </div>
  {/if}
</div>

<style>
  .file {
    display: flex;
    align-items: center;
    gap: 10px;
    height: 100%;
    padding: 8px 12px;
    color: var(--text);
    min-width: 0;
    overflow: hidden;
  }
  .file.has-preview {
    flex-direction: column;
    align-items: stretch;
    gap: 4px;
    padding: 6px;
  }
  .preview {
    flex: 1;
    min-height: 0;
    width: 100%;
    object-fit: contain;
    user-select: none;
  }
  .caption {
    flex: none;
    font-size: 12px;
    color: var(--text-muted);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    text-align: center;
  }
  .badge {
    flex: none;
    min-width: 40px;
    height: 48px;
    border-radius: var(--radius-sm);
    background: var(--fill-2, rgba(127, 127, 127, 0.18));
    display: grid;
    place-items: center;
    font: 600 10px/1 var(--font-mono, monospace);
    padding: 0 4px;
  }
  .meta {
    min-width: 0;
  }
  .name {
    font-weight: 600;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .path {
    color: var(--text-muted);
    font-size: 12px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    direction: rtl;
    text-align: left;
  }
</style>

<script lang="ts">
  import { convertFileSrc } from "@tauri-apps/api/core";
  import { t } from "$lib/i18n";
  import type { CanvasNode } from "$lib/canvas/model/types";

  let { node }: { node: CanvasNode } = $props();

  const src = $derived.by(() => {
    const s = node.data.src;
    if (typeof s === "string" && s) return s;
    const p = node.data.path;
    if (typeof p === "string" && p) {
      try {
        return convertFileSrc(p);
      } catch {
        return "";
      }
    }
    return "";
  });
  let failed = $state(false);
</script>

<div class="image">
  {#if src && !failed}
    <img {src} alt={node.title ?? ""} draggable="false" onerror={() => (failed = true)} />
  {:else}
    <div class="missing">{$t("canvas.engine.imageMissing", { default: "Image not found" })}</div>
  {/if}
</div>

<style>
  .image {
    height: 100%;
    display: grid;
    place-items: center;
    overflow: hidden;
  }
  img {
    width: 100%;
    height: 100%;
    object-fit: contain;
    user-select: none;
  }
  .missing {
    color: var(--text-muted);
    font-size: 13px;
  }
</style>

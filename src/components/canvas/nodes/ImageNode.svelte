<script lang="ts">
  import { convertFileSrc } from "@tauri-apps/api/core";
  import { t } from "$lib/i18n";
  import type { CanvasNode } from "$lib/canvas/model/types";
  import { useEngine } from "$lib/canvas/engine.svelte";
  import { imageView, requestImage, retryImage } from "$lib/canvas/images.svelte";

  let { node }: { node: CanvasNode } = $props();
  const engine = useEngine();

  $effect(() => {
    void node.data.asset;
    void node.data.path;
    void node.data.url;
    void node.data.src;
    void engine.syncMode;
    requestImage(engine, node);
  });

  const view = $derived(imageView(engine, node));
  const src = $derived.by(() => {
    if (!view.original) return "";
    try {
      return convertFileSrc(view.original);
    } catch {
      return "";
    }
  });
  let failed = $state(false);
  $effect(() => {
    void src;
    failed = false;
  });

  const error = $derived(failed ? "ASSET_DECODE" : view.error);
  const message = $derived.by(() => {
    if (error.includes("ASSET_URL_OFF")) return $t("canvas.image.webOff", { default: "Downloading images from the web is off in Settings" });
    if (error.includes("ASSET_URL")) return $t("canvas.image.downloadFailed", { default: "Couldn't download this image" });
    if (error.includes("ASSET_TOO_LARGE")) return $t("canvas.image.tooLarge", { default: "This image is too large" });
    if (error.includes("ASSET_UNSUPPORTED")) return $t("canvas.image.unsupported", { default: "This image format isn't supported" });
    if (error.includes("ASSET_NOT_FOUND")) return $t("canvas.image.notFound", { default: "The image file is gone" });
    return $t("canvas.image.loadFailed", { default: "Couldn't load this image" });
  });
</script>

<div class="image">
  {#if src && !failed}
    <img {src} alt={node.title ?? ""} draggable="false" onerror={() => (failed = true)} />
  {:else if error}
    <div class="missing" title={error}>
      <span>{message}</span>
      <button
        type="button"
        class="retry"
        onpointerdown={(e) => e.stopPropagation()}
        onclick={(e) => {
          e.stopPropagation();
          failed = false;
          retryImage(engine, node);
        }}>{$t("canvas.image.retry", { default: "Try again" })}</button
      >
    </div>
  {:else}
    <div class="loading" aria-label={$t("canvas.image.loading", { default: "Loading image" })}></div>
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
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 8px;
    padding: 12px;
    text-align: center;
    color: var(--text-muted);
    font-size: 13px;
  }
  .retry {
    font: inherit;
    font-size: 12px;
    padding: 4px 10px;
    border-radius: var(--radius-sm, 6px);
    border: 1px solid var(--border, rgba(127, 127, 127, 0.3));
    background: var(--fill-2, rgba(127, 127, 127, 0.12));
    color: var(--text);
    cursor: pointer;
  }
  .loading {
    width: 100%;
    height: 100%;
    background: color-mix(in srgb, #c58af9 18%, transparent);
  }
</style>

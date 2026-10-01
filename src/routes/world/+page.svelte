<script lang="ts">
  /** `/world` — the agent canvas. */
  import { t } from "$lib/i18n";
  import { getSettings } from "$lib/stores/settings-store.svelte";
  import Canvas from "$components/canvas/Canvas.svelte";
  import { lastWorkspaceId } from "$lib/canvas/palette/navigate";

  let enabled = $derived(getSettings()?.world?.enabled ?? true);
</script>

<div class="world-page" class:fill={enabled}>
  <header class="world-head">
    <h1>{$t("world.title")}</h1>
    <p class="world-sub">{$t("world.subtitle")}</p>
  </header>

  {#if enabled}
    <div class="place-canvas">
      <Canvas workspaceId={lastWorkspaceId()} />
    </div>
  {:else}
    <p class="world-off">{$t("world.disabled")}</p>
  {/if}
</div>

<style>
  .world-page {
    padding: 1.5rem;
    display: flex;
    flex-direction: column;
    gap: 1rem;
  }
  /* The canvas takes every pixel the shell gives the route. */
  .world-page.fill {
    flex: 1;
    min-height: 0;
    box-sizing: border-box;
  }
  .place-canvas {
    position: relative;
    flex: 1;
    min-height: 24rem;
    border-radius: 12px;
    overflow: hidden;
  }
  .world-head h1 {
    margin: 0;
    font-size: 1.4rem;
  }
  .world-sub {
    margin: 0.25rem 0 0;
    opacity: 0.7;
    font-size: 0.85rem;
  }
  .world-off {
    margin: 0;
    opacity: 0.7;
    font-size: 0.9rem;
  }
</style>

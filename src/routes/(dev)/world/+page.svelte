<script lang="ts">
  import { t } from "$lib/i18n";
  import { getSettings } from "$lib/stores/settings-store.svelte";
  import Canvas from "#components/canvas/Canvas.svelte";
  import { lastWorkspaceId } from "$lib/canvas/palette/navigate";

  let enabled = $derived(getSettings()?.world?.enabled ?? true);
</script>

<svelte:head>
  <title>{$t("world.title")}</title>
</svelte:head>

<div class="world-page" class:fill={enabled}>
  <h1 class="sr-only">{$t("world.title")}</h1>
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
    display: flex;
    flex-direction: column;
    padding: 1.5rem;
  }
  .world-page.fill {
    flex: 1;
    min-height: 0;
    padding: 0;
    box-sizing: border-box;
  }
  .place-canvas {
    position: relative;
    flex: 1;
    min-height: 0;
    overflow: hidden;
  }
  .sr-only {
    position: absolute;
    width: 1px;
    height: 1px;
    overflow: hidden;
    clip: rect(0 0 0 0);
    white-space: nowrap;
  }
  .world-off {
    margin: 0;
    opacity: 0.7;
    font-size: 0.9rem;
  }
</style>

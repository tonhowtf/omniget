<script lang="ts">
  import OmniboxInput from "$components/omnibox/OmniboxInput.svelte";
  import type { HomeInputMode } from "$lib/home/omnibox-controller";
  import { t } from "$lib/i18n";

  let {
    url = $bindable(""),
    mode = $bindable<HomeInputMode>("url"),
    onInput,
    onModeChange,
  }: {
    url?: string;
    mode?: HomeInputMode;
    onInput: () => void;
    onModeChange?: (mode: HomeInputMode) => void;
  } = $props();

  const modes: HomeInputMode[] = ["url", "batch", "torrent", "p2p"];

  function setMode(next: HomeInputMode) {
    mode = next;
    onModeChange?.(next);
  }
</script>

<div class="home-url-bar">
  <div class="mac-segmented" role="tablist">
    {#each modes as m}
      <button
        type="button"
        class="mac-segmented-btn"
        class:active={mode === m}
        role="tab"
        aria-selected={mode === m}
        onclick={() => setMode(m)}
      >
        {$t(`home.mode_${m}`)}
      </button>
    {/each}
  </div>
  {#if mode === "url"}
    <OmniboxInput bind:url onInput={onInput} />
  {/if}
</div>

<style>
  .home-url-bar {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
    width: 100%;
  }
</style>

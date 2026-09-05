<script lang="ts">
  import OmniboxInput from "$components/omnibox/OmniboxInput.svelte";
  import type { HomeInputMode } from "$lib/home/omnibox-controller";
  import { t } from "$lib/i18n";

  let {
    url = $bindable(""),
    mode = $bindable<HomeInputMode>("url"),
    variant = "bar",
    advanced = false,
    onInput,
    onModeChange,
    onAdvanced,
  }: {
    url?: string;
    mode?: HomeInputMode;
    variant?: "bar" | "stage";
    advanced?: boolean;
    onInput: () => void;
    onModeChange?: (mode: HomeInputMode) => void;
    onAdvanced?: () => void;
  } = $props();

  // Secondary entry points, each with an SF-style symbol so the row reads as
  // three quiet tools rather than a sentence of links.
  const secondary: { mode: HomeInputMode; key: string; icon: string }[] = [
    { mode: "batch", key: "home.action_batch", icon: "M4 6h16M4 12h16M4 18h10" },
    { mode: "torrent", key: "home.action_torrent", icon: "M14 3H7a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h10a2 2 0 0 0 2-2V8zM14 3v5h5" },
    { mode: "p2p", key: "home.action_p2p", icon: "M22 2 11 13M22 2l-7 20-4-9-9-4z" },
  ];

  function pick(next: HomeInputMode) {
    mode = next;
    onModeChange?.(next);
  }
</script>

<div class="home-url-bar" class:stage={variant === "stage"}>
  {#if mode === "url"}
    <OmniboxInput bind:url onInput={onInput} prominent={variant === "stage"} />
  {/if}
  <div class="home-secondary">
    {#each secondary as item (item.mode)}
      <button type="button" class="home-secondary-link" onclick={() => pick(item.mode)}>
        <svg viewBox="0 0 24 24" width="13" height="13" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d={item.icon} /></svg>
        {$t(item.key)}
      </button>
    {/each}
    {#if onAdvanced}
      <button type="button" class="home-secondary-link" onclick={onAdvanced}>
        <svg viewBox="0 0 24 24" width="13" height="13" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M4 21v-7M4 10V3M12 21v-9M12 8V3M20 21v-5M20 12V3M1 14h6M9 8h6M17 16h6" /></svg>
        {$t(advanced ? "home.action_simple" : "home.action_advanced")}
      </button>
    {/if}
  </div>
</div>

<style>
  .home-url-bar {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    width: 100%;
  }

  .home-url-bar.stage {
    align-items: center;
    gap: var(--space-3);
    max-width: 680px;
  }

  .home-url-bar.stage :global(.home-secondary) {
    gap: 2px;
  }
</style>

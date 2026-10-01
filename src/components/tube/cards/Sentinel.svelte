<script lang="ts">
  import { t } from "$lib/i18n";
  import { tubePrefs } from "$lib/tube/prefs.svelte";
  import TIcon from "../TIcon.svelte";

  let {
    onmore,
    loading = false,
    error = null,
    auto = true,
  }: { onmore: () => void; loading?: boolean; error?: string | null; auto?: boolean } = $props();

  let el = $state<HTMLDivElement>();
  let infinite = $derived(auto && tubePrefs().infinite_scroll);

  $effect(() => {
    if (!el || !infinite || error) return;
    const io = new IntersectionObserver((entries) => {
      if (entries.some((e) => e.isIntersecting)) onmore();
    }, { rootMargin: "900px 0px" });
    io.observe(el);
    return () => io.disconnect();
  });
</script>

<div class="sentinel" bind:this={el}>
  {#if loading}
    <span class="spin" aria-label={$t("tube.common.loading") as string}><TIcon name="loader" size={28} /></span>
  {:else if error}
    <p class="err">{$t("tube.common.load_more_failed")}</p>
    <button class="yt-btn yt-btn--s" onclick={onmore}>{$t("tube.common.retry")}</button>
  {:else if !infinite}
    <button class="yt-btn" onclick={onmore}>{$t("tube.common.load_more")}</button>
  {/if}
</div>

<style>
  .sentinel {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 8px;
    min-height: 48px;
    padding: 16px 0 32px;
  }
  .spin {
    display: inline-flex;
    color: var(--yt-text-2);
    animation: spin 1s linear infinite;
  }
  .err {
    margin: 0;
    color: var(--yt-text-2);
  }
  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .spin {
      animation-duration: 3s;
    }
  }
</style>

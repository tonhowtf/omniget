<script lang="ts">
  // "↑ 3 new videos" pill floating over the top of a feed (21st 23546).
  import { t } from "$lib/i18n";
  import TIcon from "../TIcon.svelte";

  let { count, onjump }: { count: number; onjump: () => void } = $props();
</script>

<div class="wrap">
  {#if count > 0}
    <button class="pill" onclick={onjump}>
      <TIcon name="chevron-up" size={16} stroke={2.5} />
      <span>{$t("tube.subs.new_videos", { count })}</span>
    </button>
  {/if}
  <span class="sr" role="status" aria-live="polite">{count > 0 ? $t("tube.subs.new_videos", { count }) : ""}</span>
</div>

<style>
  .wrap {
    position: sticky;
    top: 64px;
    z-index: 6;
    display: flex;
    justify-content: center;
    height: 0;
    pointer-events: none;
  }
  .pill {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    height: 36px;
    padding: 0 16px 0 12px;
    border: 0;
    border-radius: 999px;
    font: var(--yt-type-body-strong);
    color: #fff;
    background: var(--yt-link);
    box-shadow: 0 6px 18px var(--yt-shadow);
    cursor: pointer;
    pointer-events: auto;
    animation: drop 220ms var(--yt-ease-standard);
  }
  .pill:hover {
    filter: brightness(1.1);
  }
  .pill:focus-visible {
    outline: 2px solid var(--yt-text);
    outline-offset: 2px;
  }
  .sr {
    position: absolute;
    width: 1px;
    height: 1px;
    overflow: hidden;
    clip: rect(0 0 0 0);
  }
  @keyframes drop {
    from {
      opacity: 0;
      transform: translateY(-8px) scale(0.95);
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .pill {
      animation: none;
    }
  }
</style>

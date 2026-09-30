<script lang="ts">
  // Empty / error / sign-in states with the OmniTube illustration.
  import type { Snippet } from "svelte";
  import { t } from "$lib/i18n";

  let {
    icon = "tube-music",
    title,
    text = "",
    error = false,
    onretry,
    children,
  }: { icon?: string; title: string; text?: string; error?: boolean; onretry?: () => void; children?: Snippet } = $props();
</script>

<div class="mst" class:error role={error ? "alert" : "status"}>
  <img src="/icons/tube/{icon}-128.webp" alt="" width="128" height="128" decoding="async" />
  <h2>{title}</h2>
  {#if text}<p>{text}</p>{/if}
  <div class="mst-actions">
    {#if onretry}
      <button class="yt-btn mst-retry" onclick={onretry}>{$t("tube.common.retry")}</button>
    {/if}
    {@render children?.()}
  </div>
</div>

<style>
  .mst {
    display: flex; flex-direction: column; align-items: center; text-align: center;
    gap: 8px; padding: 64px 24px; max-width: 520px; margin: 0 auto;
    color: var(--ytm-text, var(--yt-text));
  }
  .mst img { width: 128px; height: 128px; margin-bottom: 8px; filter: drop-shadow(0 8px 24px rgba(0, 0, 0, .35)); }
  .mst h2 { margin: 0; font: var(--yt-type-headline-s); }
  .mst p { margin: 0; font: var(--yt-type-meta); color: var(--ytm-text-2, var(--yt-text-2)); }
  .mst.error p { font-family: var(--font-mono, monospace); font-size: 12px; word-break: break-word; }
  .mst-actions { display: flex; gap: 8px; margin-top: 16px; flex-wrap: wrap; justify-content: center; }
  .mst-actions:empty { display: none; }
  .mst-retry { background: var(--ytm-chip, var(--yt-tonal)); color: var(--ytm-text, var(--yt-text)); }
</style>

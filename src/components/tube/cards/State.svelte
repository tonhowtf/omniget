<script lang="ts">
  import type { Snippet } from "svelte";
  import { t } from "$lib/i18n";

  let {
    icon = "tube",
    title,
    text = "",
    error = null,
    onretry,
    children,
    compact = false,
  }: {
    icon?: string;
    title?: string;
    text?: string;
    error?: string | null;
    onretry?: () => void;
    children?: Snippet;
    compact?: boolean;
  } = $props();
</script>

<div class="state" class:compact role={error ? "alert" : undefined}>
  <img
    src={`/icons/tube/${icon}-128.webp`}
    srcset={`/icons/tube/${icon}-64.webp 1x, /icons/tube/${icon}-128.webp 2x`}
    width={compact ? 64 : 96}
    height={compact ? 64 : 96}
    alt=""
    decoding="async"
  />
  <h2>{title ?? (error ? $t("tube.common.error_title") : "")}</h2>
  {#if error}
    <p class="detail">{error}</p>
  {:else if text}
    <p>{text}</p>
  {/if}
  <div class="actions">
    {#if error && onretry}
      <button class="yt-btn yt-btn--outline" onclick={onretry}>{$t("tube.common.retry")}</button>
    {/if}
    {@render children?.()}
  </div>
</div>

<style>
  .state {
    display: flex;
    flex-direction: column;
    align-items: center;
    text-align: center;
    gap: 8px;
    max-width: 440px;
    margin: 0 auto;
    padding: 72px 24px;
  }
  .state.compact {
    padding: 32px 16px;
  }
  img {
    margin-bottom: 8px;
    filter: drop-shadow(0 8px 24px var(--yt-shadow));
  }
  h2 {
    margin: 0;
    font: var(--yt-type-headline-s);
    color: var(--yt-text);
  }
  .compact h2 {
    font: var(--yt-type-card-title);
  }
  p {
    margin: 0;
    color: var(--yt-text-2);
    font: var(--yt-type-meta);
  }
  .detail {
    font: var(--yt-type-caption);
    word-break: break-word;
  }
  .actions {
    display: flex;
    flex-wrap: wrap;
    justify-content: center;
    gap: 8px;
    margin-top: 12px;
  }
  .actions:empty {
    display: none;
  }
</style>

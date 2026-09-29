<script lang="ts">
  // Page header: optional back arrow, title, subtitle and trailing actions.
  import type { Snippet } from "svelte";
  import { t } from "$lib/i18n";
  import { back, ui } from "$lib/video/ui.svelte";
  import VIcon from "./VIcon.svelte";

  let {
    title,
    subtitle = "",
    showBack = false,
    actions,
  }: { title: string; subtitle?: string; showBack?: boolean; actions?: Snippet } = $props();
</script>

<header class="vh">
  {#if showBack && ui.stack.length}
    <button class="bk" onclick={back} aria-label={$t("video.common.back") as string}><VIcon name="back" size={16} /></button>
  {/if}
  <div class="tt">
    <h1>{title}</h1>
    {#if subtitle}<p>{subtitle}</p>{/if}
  </div>
  {#if actions}<div class="acts">{@render actions()}</div>{/if}
</header>

<style>
  .vh {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    flex-wrap: wrap;
  }
  .bk {
    all: unset;
    display: grid;
    place-items: center;
    width: 30px;
    height: 30px;
    border-radius: 50%;
    background: var(--fill-1);
    cursor: pointer;
  }
  .bk:hover {
    background: var(--fill-2);
  }
  .bk:focus-visible {
    outline: var(--focus-ring);
  }
  .tt {
    flex: 1;
    min-width: 0;
  }
  h1 {
    margin: 0;
    font-family: var(--font-display);
    font-size: var(--text-3xl);
    font-weight: 750;
    letter-spacing: var(--track-tight);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  p {
    margin: 2px 0 0;
    color: var(--text-dim);
  }
  .acts {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    flex-wrap: wrap;
  }
</style>

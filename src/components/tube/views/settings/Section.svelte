<script lang="ts">
  import type { Snippet } from "svelte";

  let {
    id,
    icon,
    title,
    desc = "",
    extra,
    children,
  }: { id: string; icon: string; title: string; desc?: string; extra?: Snippet; children: Snippet } = $props();
</script>

<section class="sc" id="tube-set-{id}" data-section={id} aria-labelledby="tube-set-{id}-h">
  <header class="sc-head">
    <img src="/icons/tube/{icon}-128.webp" alt="" width="64" height="64" loading="lazy" decoding="async" />
    <div class="sc-titles">
      <h2 id="tube-set-{id}-h">{title}</h2>
      {#if desc}<p>{desc}</p>{/if}
    </div>
    {#if extra}<div class="sc-extra">{@render extra()}</div>{/if}
  </header>
  <div class="sc-card">
    {@render children()}
  </div>
</section>

<style>
  .sc { scroll-margin-top: 16px; margin-bottom: 40px; }
  .sc-head { display: flex; align-items: center; gap: 16px; margin-bottom: 12px; }
  .sc-head img { width: 64px; height: 64px; flex: none; filter: drop-shadow(0 6px 16px rgba(0, 0, 0, .25)); }
  .sc-titles { flex: 1; min-width: 0; }
  .sc-titles h2 { margin: 0; font: var(--yt-type-headline-s); color: var(--yt-text); }
  .sc-titles p { margin: 2px 0 0; font: var(--yt-type-meta); color: var(--yt-text-2); }
  .sc-extra { flex: none; }
  .sc-card {
    padding: 4px 20px;
    border-radius: 12px;
    background: var(--yt-bg-b);
    border: 1px solid var(--yt-divider);
  }
</style>

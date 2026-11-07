<script lang="ts">
  import type { Snippet } from "svelte";

  let { id, title, intro, settingKey, children }: { id: string; title: string; intro?: string; settingKey?: string; children: Snippet } = $props();
</script>

<section class="subgroup" id={`settings-sub-${id}`} aria-labelledby={`settings-sub-${id}-title`} data-setting-key={settingKey}>
  <h3 id={`settings-sub-${id}-title`}>{title}</h3>
  {#if intro}<p class="intro"><bdi>{intro}</bdi></p>{/if}
  {@render children()}
</section>

<style>
  .subgroup { display: flex; flex-direction: column; min-inline-size: 0; padding-block: var(--space-3) var(--space-1); border-block-start: 1px solid var(--separator); }
  .subgroup:first-child { border-block-start: 0; padding-block-start: var(--space-1); }
  h3 { margin: 0; font-size: var(--text-base); line-height: 20px; font-weight: 600; color: var(--text); }
  .intro { margin: 2px 0 var(--space-1); font-size: var(--text-sm); line-height: 18px; color: var(--text-muted); max-inline-size: 76ch; }
</style>

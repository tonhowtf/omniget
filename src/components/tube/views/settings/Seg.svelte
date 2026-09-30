<script lang="ts" generics="V extends string">
  // Segmented choice for short option lists (look, codec, SponsorBlock mode...).
  let {
    value,
    options,
    label,
    disabled = false,
    onchange,
  }: { value: V; options: { value: V; label: string; dot?: string }[]; label: string; disabled?: boolean; onchange: (v: V) => void } = $props();
</script>

<div class="sg" role="radiogroup" aria-label={label} class:disabled>
  {#each options as o (o.value)}
    <button class="sg-opt" class:on={o.value === value} role="radio" aria-checked={o.value === value} {disabled} onclick={() => o.value !== value && onchange(o.value)}>
      {#if o.dot}<span class="sg-dot" style:background={o.dot}></span>{/if}
      {o.label}
    </button>
  {/each}
</div>

<style>
  .sg { display: inline-flex; flex-wrap: wrap; gap: 2px; padding: 2px; border-radius: 10px; background: var(--yt-tonal); }
  .sg.disabled { opacity: .45; }
  .sg-opt {
    display: inline-flex; align-items: center; gap: 6px;
    height: 28px; padding: 0 12px; border: 0; border-radius: 8px;
    font: 500 13px/18px var(--yt-font); color: var(--yt-text-2); background: transparent; cursor: pointer; white-space: nowrap;
    transition: background var(--yt-dur-fast), color var(--yt-dur-fast);
  }
  .sg-opt:hover:not(:disabled) { color: var(--yt-text); }
  .sg-opt.on { background: var(--yt-bg-inverted); color: var(--yt-on-inverted); }
  .sg-opt:disabled { cursor: default; }
  .sg-opt:focus-visible { outline: 2px solid var(--yt-link); outline-offset: 1px; }
  .sg-dot { width: 8px; height: 8px; border-radius: 50%; box-shadow: 0 0 0 1px rgba(0, 0, 0, .25); }
</style>

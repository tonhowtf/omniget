<script lang="ts">
  import { areaPath, linePath } from "$lib/washmybelly/ui";

  let {
    values,
    width = 120,
    height = 32,
  }: { values: number[]; width?: number; height?: number } = $props();

  const uid = `wmb-spark-${Math.random().toString(36).slice(2, 9)}`;
  let cap = $derived(Math.max(values.length, 2));
  let line = $derived(linePath(values, width, height - 1, 100, cap));
  let fill = $derived(areaPath(values, width, height - 1, 100, cap));
</script>

<svg class="wmb-spark" {width} {height} viewBox="0 0 {width} {height}" aria-hidden="true">
  {#if fill}
    <defs>
      <linearGradient id={uid} x1="0" y1="0" x2="0" y2="1">
        <stop offset="0%" style:stop-color="var(--accent)" stop-opacity="0.28" />
        <stop offset="100%" style:stop-color="var(--accent)" stop-opacity="0" />
      </linearGradient>
    </defs>
    <path d={fill} fill="url(#{uid})" />
  {/if}
  {#if line}
    <path d={line} fill="none" style:stroke="var(--accent)" stroke-width="1.6" stroke-linejoin="round" stroke-linecap="round" transform="translate(0 0.5)" />
  {/if}
</svg>

<style>
  .wmb-spark {
    display: block;
    overflow: visible;
  }
</style>

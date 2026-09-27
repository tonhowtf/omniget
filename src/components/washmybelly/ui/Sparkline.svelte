<script lang="ts">
  import { areaPath, linePath } from "$lib/washmybelly/ui";

  /// Tiny trend line with a soft area. `capacity` fixes how many points fill
  /// the width, so a series that is still filling grows from the right.
  let {
    values,
    max = 100,
    color = "var(--accent)",
    width = 120,
    height = 32,
    capacity,
    area = true,
  }: { values: number[]; max?: number; color?: string; width?: number; height?: number; capacity?: number; area?: boolean } = $props();

  const uid = `wmb-spark-${Math.random().toString(36).slice(2, 9)}`;
  let cap = $derived(capacity ?? Math.max(values.length, 2));
  let line = $derived(linePath(values, width, height - 1, max, cap));
  let fill = $derived(area ? areaPath(values, width, height - 1, max, cap) : "");
</script>

<svg class="wmb-spark" {width} {height} viewBox="0 0 {width} {height}" aria-hidden="true">
  {#if fill}
    <defs>
      <linearGradient id={uid} x1="0" y1="0" x2="0" y2="1">
        <stop offset="0%" style:stop-color={color} stop-opacity="0.28" />
        <stop offset="100%" style:stop-color={color} stop-opacity="0" />
      </linearGradient>
    </defs>
    <path d={fill} fill="url(#{uid})" />
  {/if}
  {#if line}
    <path d={line} fill="none" style:stroke={color} stroke-width="1.6" stroke-linejoin="round" stroke-linecap="round" transform="translate(0 0.5)" />
  {/if}
</svg>

<style>
  .wmb-spark {
    display: block;
    overflow: visible;
  }
</style>

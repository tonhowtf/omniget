<script lang="ts">
  // Stroke icon on a 24×24 viewBox; `d` may hold several subpaths.
  let { d, size = 16 }: { d: string; size?: number } = $props();
</script>

<svg
  width={size}
  height={size}
  viewBox="0 0 24 24"
  fill="none"
  stroke="currentColor"
  stroke-width="1.8"
  stroke-linecap="round"
  stroke-linejoin="round"
  aria-hidden="true"
>
  <path {d} />
</svg>

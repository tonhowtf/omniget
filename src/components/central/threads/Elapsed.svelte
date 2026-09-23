<script lang="ts">
  // Self-ticking elapsed time: writes textContent once a second, so nothing
  // else re-renders (T3 `WorkingTimer`).
  import { formatDuration } from "$lib/central/threads-ui/format";

  let { since, prefix = "" }: { since: string; prefix?: string } = $props();
  let el = $state<HTMLSpanElement | null>(null);

  $effect(() => {
    const start = Date.parse(since);
    const node = el;
    if (!node) return;
    const paint = () => {
      node.textContent = prefix + formatDuration(Number.isNaN(start) ? 0 : Date.now() - start);
    };
    paint();
    const timer = setInterval(paint, 1000);
    return () => clearInterval(timer);
  });
</script>

<span class="elapsed" bind:this={el}></span>

<style>
  .elapsed { font-variant-numeric: tabular-nums; }
</style>

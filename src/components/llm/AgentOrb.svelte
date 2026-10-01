<script lang="ts">
  import { onMount } from "svelte";
  import type { OrbState } from "thinking-orbs/engine";
  import { orb } from "$components/limits-strip/orb";

  let { activity, after = 2000 }: { activity: OrbState; after?: number } = $props();

  let shown = $state(false);
  let theme = $state<"dark" | "light">("dark");

  onMount(() => {
    const timer = setTimeout(() => (shown = true), after);
    const root = document.documentElement;
    const dark = matchMedia("(prefers-color-scheme: dark)");
    const readTheme = () => {
      const set = root.getAttribute("data-theme");
      theme = set === "light" || set === "dark" ? set : dark.matches ? "dark" : "light";
    };
    readTheme();
    dark.addEventListener("change", readTheme);
    const themeWatch = new MutationObserver(readTheme);
    themeWatch.observe(root, { attributes: true, attributeFilter: ["data-theme"] });
    return () => {
      clearTimeout(timer);
      dark.removeEventListener("change", readTheme);
      themeWatch.disconnect();
    };
  });
</script>

{#if shown}
  <canvas class="agent-orb" aria-hidden="true" use:orb={{ state: activity, size: 20, theme }}></canvas>
{/if}

<style>
  .agent-orb {
    display: inline-block;
    flex-shrink: 0;
    vertical-align: middle;
  }
</style>

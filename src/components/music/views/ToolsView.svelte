<script lang="ts">
  import { t } from "$lib/i18n";
  import { library } from "$lib/music/library-store.svelte";
  import MusicIcon from "../MusicIcon.svelte";
  import AnalyzeTool from "../tools/AnalyzeTool.svelte";
  import ConvertTool from "../tools/ConvertTool.svelte";
  import DuplicatesTool from "../tools/DuplicatesTool.svelte";
  import LyricsTool from "../tools/LyricsTool.svelte";
  import QualityTool from "../tools/QualityTool.svelte";
  import RenameTool from "../tools/RenameTool.svelte";

  let { tab = "analyze", ids = [] }: { tab?: string; ids?: number[] } = $props();

  const TABS = [
    { id: "analyze", icon: "activity" },
    { id: "convert", icon: "convert" },
    { id: "rename", icon: "edit" },
    { id: "lyrics", icon: "lyrics" },
    { id: "duplicates", icon: "copy" },
    { id: "quality", icon: "wave" },
  ];

  let current = $state("analyze");
  $effect(() => {
    current = tab || "analyze";
  });
  let lib = $derived(library());
</script>

<div class="tv">
  <header>
    <img src="/music/tools.webp" alt="" width="96" height="78" />
    <div>
      <h1>{$t("music.nav.tools")}</h1>
      <p class="sub">{$t("music.tools.subtitle")}</p>
    </div>
  </header>
  <nav class="tabs" aria-label={$t("music.nav.tools") as string}>
    {#each TABS as tb (tb.id)}
      <button class="tab" class:on={current === tb.id} onclick={() => (current = tb.id)} aria-current={current === tb.id}>
        <MusicIcon name={tb.icon} size={15} /> {$t(`music.tools.${tb.id}`)}
      </button>
    {/each}
  </nav>
  {#if !lib.status?.ffmpeg && ["analyze", "convert", "quality"].includes(current)}
    <p class="warn"><MusicIcon name="activity" size={14} /> {$t("music.tools.no_ffmpeg")}</p>
  {/if}
  <div class="body">
    {#if current === "analyze"}<AnalyzeTool {ids} />
    {:else if current === "convert"}<ConvertTool {ids} />
    {:else if current === "rename"}<RenameTool {ids} />
    {:else if current === "lyrics"}<LyricsTool {ids} />
    {:else if current === "duplicates"}<DuplicatesTool />
    {:else if current === "quality"}<QualityTool id={ids[0] ?? null} />
    {/if}
  </div>
</div>

<style>
  .tv {
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
    padding-bottom: var(--space-7);
  }
  header {
    display: flex;
    align-items: center;
    gap: var(--space-3);
  }
  header img {
    width: 96px;
    height: auto;
  }
  h1 {
    margin: 0;
    font-family: var(--font-display);
    font-size: var(--text-2xl);
    font-weight: 750;
  }
  .sub {
    margin: 2px 0 0;
    color: var(--text-dim);
    font-size: var(--text-sm);
  }
  .tabs {
    display: flex;
    gap: 4px;
    flex-wrap: wrap;
    border-bottom: var(--hairline) solid var(--content-border);
    padding-bottom: var(--space-2);
  }
  .tab {
    all: unset;
    display: inline-flex;
    align-items: center;
    gap: 6px;
    padding: 6px 12px;
    border-radius: var(--radius-md);
    font-size: var(--text-sm);
    font-weight: 550;
    color: var(--text-dim);
    cursor: pointer;
  }
  .tab:hover {
    background: var(--fill-1);
    color: var(--text);
  }
  .tab.on {
    background: var(--accent-soft);
    color: var(--accent-hi);
  }
  .tab:focus-visible {
    outline: var(--focus-ring);
  }
  .warn {
    display: flex;
    align-items: center;
    gap: 6px;
    margin: 0;
    padding: 8px 12px;
    border-radius: var(--radius-md);
    background: color-mix(in srgb, var(--warning) 14%, transparent);
    color: var(--warning);
    font-size: var(--text-sm);
  }
</style>

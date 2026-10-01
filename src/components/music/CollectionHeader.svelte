<script lang="ts">
  import type { Snippet } from "svelte";
  import { t } from "$lib/i18n";
  import Cover from "./Cover.svelte";
  import MusicIcon from "./MusicIcon.svelte";

  let {
    kicker,
    title,
    subtitle = "",
    cover = null,
    covers = [],
    round = false,
    icon = "note",
    onplay,
    onshuffle,
    actions,
  }: {
    kicker: string;
    title: string;
    subtitle?: string;
    cover?: string | null;
    covers?: string[];
    round?: boolean;
    icon?: string;
    onplay?: () => void;
    onshuffle?: () => void;
    actions?: Snippet;
  } = $props();
</script>

<header class="ch">
  <div class="art">
    {#if covers.length >= 4}
      <div class="collage">
        {#each covers.slice(0, 4) as c (c)}<Cover hash={c} seed={c} size="100%" radius={0} />{/each}
      </div>
    {:else}
      <Cover hash={cover ?? covers[0] ?? null} seed={title} size="100%" radius={round ? 0 : 10} {round} {icon} />
    {/if}
  </div>
  <div class="info">
    <span class="kicker">{kicker}</span>
    <h1 title={title}>{title}</h1>
    {#if subtitle}<p class="sub">{subtitle}</p>{/if}
    <div class="acts">
      {#if onplay}
        <button class="btn btn-primary btn-lg" onclick={onplay}><MusicIcon name="play" size={15} /> {$t("music.play")}</button>
      {/if}
      {#if onshuffle}
        <button class="btn btn-lg" onclick={onshuffle}><MusicIcon name="shuffle" size={15} /> {$t("music.shuffle")}</button>
      {/if}
      {@render actions?.()}
    </div>
  </div>
</header>

<style>
  .ch {
    display: flex;
    align-items: flex-end;
    gap: var(--space-5);
    padding: var(--space-2) 0 var(--space-5);
  }
  .art {
    width: 200px;
    height: 200px;
    flex-shrink: 0;
    border-radius: 10px;
    overflow: hidden;
    box-shadow: 0 16px 40px rgba(0, 0, 0, 0.35);
  }
  .art :global(.cover) {
    width: 100% !important;
    height: 100% !important;
  }
  .collage {
    display: grid;
    grid-template-columns: 1fr 1fr;
    width: 100%;
    height: 100%;
  }
  .info {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
    min-width: 0;
  }
  .kicker {
    font-size: var(--text-xs);
    font-weight: 650;
    text-transform: uppercase;
    letter-spacing: 0.08em;
    color: var(--text-muted);
  }
  h1 {
    margin: 0;
    font-family: var(--font-display);
    font-size: var(--text-3xl);
    line-height: var(--leading-3xl);
    font-weight: 750;
    letter-spacing: var(--track-tight);
    color: var(--text);
    display: -webkit-box;
    -webkit-line-clamp: 2;
    line-clamp: 2;
    -webkit-box-orient: vertical;
    overflow: hidden;
  }
  .sub {
    margin: 0;
    color: var(--text-dim);
    font-size: var(--text-sm);
  }
  .acts {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-2);
    margin-top: var(--space-3);
  }
  @media (max-width: 760px) {
    .art {
      width: 132px;
      height: 132px;
    }
    h1 {
      font-size: var(--text-2xl);
    }
  }
</style>

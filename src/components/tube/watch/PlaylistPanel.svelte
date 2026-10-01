<script lang="ts">
  import { t } from "$lib/i18n";
  import type { WatchPlaylist } from "$lib/tube/types";
  import TIcon from "../TIcon.svelte";
  import VideoRow from "../cards/VideoRow.svelte";
  import { openPlaylist } from "../cards/util";

  let { playlist, currentId }: { playlist: WatchPlaylist; currentId: string } = $props();

  let collapsed = $state(false);
  let list = $state<HTMLDivElement>();
  let pos = $derived(Math.max(0, playlist.items.findIndex((v) => v.id === currentId)));

  $effect(() => {
    void currentId;
    if (!list || collapsed) return;
    const el = list.querySelector<HTMLElement>(".active");
    if (el) list.scrollTop = el.offsetTop - list.offsetTop - 8;
  });
</script>

<section class="pp" aria-label={playlist.title}>
  <header>
    <div class="ht">
      <button class="title yt-clamp-1" onclick={() => openPlaylist(playlist.id)}>{playlist.title}</button>
      <div class="sub">
        {#if playlist.is_mix}{$t("tube.card.mix")}{:else if playlist.owner}{playlist.owner}{/if}
        <span>– {pos + 1} / {playlist.total ?? playlist.items.length}</span>
      </div>
    </div>
    <button class="yt-btn yt-btn--icon" onclick={() => (collapsed = !collapsed)} aria-expanded={!collapsed} aria-label={$t(collapsed ? "tube.watch.expand" : "tube.watch.collapse") as string}>
      <TIcon name={collapsed ? "chevron-down" : "chevron-up"} />
    </button>
  </header>
  {#if !collapsed}
    <div class="list yt-scroll" bind:this={list}>
      {#each playlist.items as v, i (v.id + ":" + i)}
        <div class:active={v.id === currentId}>
          <VideoRow item={v} size="compact" number={i + 1} active={v.id === currentId} list={playlist.id} index={i} feedbackActions={false} />
        </div>
      {/each}
    </div>
  {/if}
</section>

<style>
  .pp {
    margin-bottom: 16px;
    border: 1px solid var(--yt-divider);
    border-radius: 12px;
    overflow: hidden;
  }
  header {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 12px 8px 12px 16px;
    background: var(--yt-tonal);
  }
  .ht {
    flex: 1;
    min-width: 0;
  }
  .title {
    display: block;
    max-width: 100%;
    padding: 0;
    border: 0;
    font: var(--yt-type-headline-s);
    color: var(--yt-text);
    background: none;
    text-align: left;
    cursor: pointer;
  }
  .title:hover {
    text-decoration: underline;
  }
  .sub {
    display: flex;
    gap: 6px;
    font: var(--yt-type-caption);
    color: var(--yt-text-2);
  }
  .list {
    display: flex;
    flex-direction: column;
    gap: 4px;
    max-height: 400px;
    padding: 8px 8px 8px 4px;
  }
</style>

<script lang="ts">
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import * as api from "$lib/video/api";
  import { openPlayer } from "$lib/video/player.svelte";
  import { ui } from "$lib/video/ui.svelte";
  import type { MediaItem, ShowDetail } from "$lib/video/types";
  import ContextMenu, { type MenuItem } from "$components/music/ContextMenu.svelte";
  import { itemMenu, markWatched } from "./parts/actions";
  import Empty from "./parts/Empty.svelte";
  import Head from "./parts/Head.svelte";
  import Skel from "./parts/Skel.svelte";
  import Thumb from "./parts/Thumb.svelte";
  import VIcon from "./parts/VIcon.svelte";
  import { errText, fmtDur, onLibraryChanged, ratio, ready } from "./parts/util";

  let { show = undefined }: { show?: string } = $props();

  let name = $derived(show ?? (ui.view.name === "show" ? ui.view.show : ""));
  let detail = $state<ShowDetail | null>(null);
  let error = $state<string | null>(null);
  let season = $state<number | null>(null);
  let cm = $state<{ x: number; y: number; items: MenuItem[] } | null>(null);

  async function load(n: string) {
    if (!n) return;
    try {
      await ready();
      const d = await api.show(n);
      if (n !== name) return;
      detail = d;
      error = null;
      if (season == null || !d.seasons.some((s) => s.season === season)) {
        const withNext = d.next != null ? d.seasons.find((s) => s.episodes.some((e) => e.id === d.next)) : null;
        season = (withNext ?? d.seasons[0])?.season ?? null;
      }
    } catch (e) {
      error = errText(e);
    }
  }

  $effect(() => {
    const n = name;
    detail = null;
    season = null;
    void load(n);
    return onLibraryChanged(() => void load(n));
  });

  let current = $derived(detail?.seasons.find((s) => s.season === season) ?? null);
  let all = $derived(detail?.seasons.flatMap((s) => s.episodes) ?? []);
  let watchedCount = $derived(all.filter((e) => e.progress?.watched).length);
  let nextEp = $derived(detail?.next != null ? (all.find((e) => e.id === detail!.next) ?? null) : null);
  let seasonAllWatched = $derived(!!current && current.episodes.every((e) => e.progress?.watched));

  function epTitle(e: MediaItem): string {
    const bare = !e.title || /\bS\d{1,2}\s*E\d{1,3}\b|\b\d{1,2}x\d{2,3}\b/i.test(e.title);
    return bare && e.episode != null ? ($t("video.show.episode_n", { number: e.episode }) as string) : e.title;
  }

  function epLabel(e: MediaItem): string {
    if (e.episode == null) return "";
    const end = e.episode_end && e.episode_end !== e.episode ? `–${e.episode_end}` : "";
    return `${e.episode}${end}`;
  }

  function seasonLabel(n: number): string {
    return n === 0 ? ($t("video.show.specials") as string) : ($t("video.show.season", { number: n }) as string);
  }

  function playNext() {
    const id = detail?.next ?? all[0]?.id;
    if (id != null) void openPlayer({ kind: "local", id });
  }

  async function toggle(e: MediaItem) {
    await markWatched([e.key], !e.progress?.watched, () => void load(name));
  }

  async function markSeason(watched: boolean) {
    if (!current) return;
    await markWatched(
      current.episodes.map((e) => e.key),
      watched,
      () => void load(name),
    );
    showToast("success", (watched ? $t("video.show.season_marked_watched") : $t("video.show.season_marked_unwatched")) as string);
  }

  function menu(ev: MouseEvent, e: MediaItem) {
    ev.preventDefault();
    cm = { x: ev.clientX, y: ev.clientY, items: itemMenu(e, () => void load(name)).filter((m) => !("label" in m) || m.label !== $t("video.menu.go_to_show")) };
  }
</script>

<div class="page">
  <Head title={name} showBack subtitle={detail ? ($t("video.show.progress", { watched: watchedCount, total: all.length }) as string) : ""}>
    {#snippet actions()}
      {#if detail && all.length}
        <button class="btn btn-primary" onclick={playNext}>
          <VIcon name="play" size={13} />
          {#if nextEp && nextEp.season != null && nextEp.episode != null}
            {$t("video.show.play_episode", { code: `S${nextEp.season}E${nextEp.episode}` })}
          {:else}
            {$t("video.show.play_next")}
          {/if}
        </button>
      {/if}
    {/snippet}
  </Head>

  {#if error}
    <div class="notice notice-danger"><VIcon name="alert" size={16} /><div class="notice-body">{error}</div></div>
  {/if}

  {#if !detail}
    {#if !error}<Skel kind="list" count={8} />{/if}
  {:else if !detail.seasons.length}
    <Empty icon="series" title={$t("video.show.empty") as string} />
  {:else}
    <div class="bar">
      {#if detail.seasons.length > 1}
        <div class="segmented segmented-lg seasons" role="tablist" aria-label={$t("video.show.seasons") as string}>
          {#each detail.seasons as s (s.season)}
            <button class="segmented-btn" role="tab" aria-selected={season === s.season} onclick={() => (season = s.season)}>{seasonLabel(s.season)}</button>
          {/each}
        </div>
      {:else}
        <h2>{seasonLabel(detail.seasons[0].season)}</h2>
      {/if}
      <span class="sp"></span>
      {#if current}
        <button class="btn btn-sm" onclick={() => void markSeason(!seasonAllWatched)}>
          <VIcon name={seasonAllWatched ? "eye-off" : "check"} size={13} />
          {seasonAllWatched ? $t("video.show.mark_season_unwatched") : $t("video.show.mark_season_watched")}
        </button>
      {/if}
    </div>

    {#if current}
      <ol class="eps" role="list">
        {#each current.episodes as e (e.id)}
          {@const r = ratio(e.progress)}
          {@const seen = !!e.progress?.watched}
          <li class="ep" class:seen class:next={e.id === detail.next} class:offline={e.offline}>
            <button class="ep-open" onclick={() => void openPlayer({ kind: "local", id: e.id })} oncontextmenu={(ev) => menu(ev, e)}>
              <span class="ep-th">
                <Thumb src={e.thumb} seed={name} radius="var(--radius-sm)" />
                {#if r != null}<span class="ep-bar"><span style:width="{r * 100}%"></span></span>{/if}
                <span class="ep-pl" aria-hidden="true"><VIcon name="play" size={14} /></span>
              </span>
              <span class="ep-no">{epLabel(e)}</span>
              <span class="ep-tx">
                <b>{epTitle(e)}</b>
                <small>
                  {fmtDur(e.duration_ms)}
                  {#if e.id === detail.next}· <em>{$t("video.show.up_next")}</em>{/if}
                  {#if e.offline}· {$t("video.common.offline")}{/if}
                </small>
              </span>
            </button>
            <button
              class="ep-check"
              class:on={seen}
              role="checkbox"
              aria-checked={seen}
              aria-label={(seen ? $t("video.menu.mark_unwatched") : $t("video.menu.mark_watched")) as string}
              title={(seen ? $t("video.menu.mark_unwatched") : $t("video.menu.mark_watched")) as string}
              onclick={() => void toggle(e)}
            >
              <VIcon name="check" size={13} />
            </button>
          </li>
        {/each}
      </ol>
    {/if}
  {/if}
</div>

{#if cm}<ContextMenu x={cm.x} y={cm.y} items={cm.items} onclose={() => (cm = null)} />{/if}

<style>
  .page {
    display: flex;
    flex-direction: column;
    gap: var(--space-5);
    padding-bottom: var(--space-7);
  }
  .bar {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    flex-wrap: wrap;
  }
  .seasons {
    max-width: 100%;
    overflow-x: auto;
  }
  .sp {
    flex: 1;
  }
  h2 {
    margin: 0;
    font-size: var(--text-lg);
    font-weight: 700;
  }
  .eps {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .ep {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    padding: 6px;
    border-radius: var(--radius-md);
    transition: background var(--duration-fast);
  }
  .ep:hover {
    background: var(--fill-1);
  }
  .ep.next {
    background: var(--accent-soft);
  }
  .ep.offline {
    opacity: 0.55;
  }
  .ep-open {
    all: unset;
    flex: 1;
    min-width: 0;
    display: flex;
    align-items: center;
    gap: var(--space-3);
    cursor: pointer;
  }
  .ep-open:focus-visible {
    outline: var(--focus-ring);
    border-radius: var(--radius-sm);
  }
  .ep-th {
    position: relative;
    width: 128px;
    flex-shrink: 0;
  }
  .ep-bar {
    position: absolute;
    left: 4px;
    right: 4px;
    bottom: 4px;
    height: 3px;
    border-radius: var(--radius-full);
    background: rgba(0, 0, 0, 0.45);
    overflow: hidden;
  }
  .ep-bar span {
    display: block;
    height: 100%;
    background: var(--accent);
  }
  .ep-pl {
    position: absolute;
    inset: 0;
    display: grid;
    place-items: center;
    color: white;
    background: rgba(0, 0, 0, 0.35);
    border-radius: var(--radius-sm);
    opacity: 0;
    transition: opacity var(--duration-fast);
  }
  .ep:hover .ep-pl {
    opacity: 1;
  }
  .ep-no {
    width: 36px;
    text-align: right;
    font-variant-numeric: tabular-nums;
    font-weight: 600;
    color: var(--text-dim);
  }
  .ep-tx {
    display: flex;
    flex-direction: column;
    min-width: 0;
  }
  .ep-tx b {
    font-weight: 600;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .seen .ep-tx b {
    color: var(--text-dim);
  }
  .ep-tx small {
    color: var(--text-dim);
    font-size: var(--text-sm);
  }
  .ep-tx em {
    font-style: normal;
    color: var(--accent-text);
    font-weight: 600;
  }
  .ep-check {
    all: unset;
    display: grid;
    place-items: center;
    width: 24px;
    height: 24px;
    border-radius: 50%;
    color: transparent;
    box-shadow: inset 0 0 0 1.5px var(--fill-4);
    cursor: pointer;
    transition:
      background var(--duration-fast),
      color var(--duration-fast);
  }
  .ep-check:hover {
    color: var(--text-dim);
  }
  .ep-check.on {
    background: var(--success);
    color: var(--on-success);
    box-shadow: none;
  }
  .ep-check:focus-visible {
    outline: var(--focus-ring);
  }
</style>

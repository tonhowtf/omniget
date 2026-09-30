<script lang="ts">
  // Shorts: a vertical 9:16 feed, one per screen with scroll snap. Only the
  // slide on screen mounts the player (W5's VideoPlayer in `short` mode);
  // the others are thumbnails. ↑/↓ and the round buttons move between them.
  import { t } from "$lib/i18n";
  import * as api from "$lib/tube/api";
  import { bestAvatar, thumbUrl, videoThumb } from "$lib/tube/format";
  import { patchTubePrefs, tubePrefs } from "$lib/tube/prefs.svelte";
  import type { Item, ShortItem } from "$lib/tube/types";
  import type { Route } from "$lib/tube/ui.svelte";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import TIcon from "../../TIcon.svelte";
  import ItemMenu from "../../cards/ItemMenu.svelte";
  import State from "../../cards/State.svelte";
  import UndoPill from "../../cards/UndoPill.svelte";
  import { isVisible } from "../../cards/feedback.svelte";
  import { Pager } from "../../cards/pager.svelte";
  import { copyText, errText, openChannel } from "../../cards/util";
  import VideoPlayer from "../../player/VideoPlayer.svelte";
  import DownloadMenu from "../../watch/DownloadMenu.svelte";

  let { route: _route }: { route: Extract<Route, { name: "shorts" }> } = $props();

  type Slide = { id: string; title: string; thumb: string; channel: ShortItem["channel"]; views: string | null; item: Item };

  const pager = new Pager("feed", () => api.shorts());
  let active = $state(0);
  let feed = $state<HTMLDivElement>();
  let liked = $state<Record<string, "like" | "dislike" | undefined>>({});
  let menuFor = $state<{ slide: Slide; anchor: HTMLElement } | null>(null);
  let dlFor = $state<{ slide: Slide; anchor: HTMLElement } | null>(null);

  let off = $derived(tubePrefs().hide_shorts);

  $effect(() => {
    if (!off) void pager.load();
  });

  let slides = $derived.by((): Slide[] => {
    const out: Slide[] = [];
    const seen = new Set<string>();
    for (const it of pager.shelves.flatMap((s) => s.items)) {
      if ((it.kind !== "short" && it.kind !== "video") || seen.has(it.id) || !isVisible(it)) continue;
      seen.add(it.id);
      out.push({
        id: it.id,
        title: it.title,
        thumb: thumbUrl(it.thumbs, 400) || videoThumb(it.id, "hqdefault"),
        channel: it.channel ?? null,
        views: it.views_text ?? null,
        item: it,
      });
    }
    return out;
  });

  // Which slide is on screen.
  $effect(() => {
    if (!feed) return;
    void slides.length;
    const io = new IntersectionObserver(
      (entries) => {
        for (const e of entries) {
          if (e.isIntersecting) active = Number((e.target as HTMLElement).dataset.i);
        }
      },
      { root: feed, threshold: 0.6 },
    );
    for (const el of feed.querySelectorAll<HTMLElement>("[data-i]")) io.observe(el);
    return () => io.disconnect();
  });

  // Keep a few slides ahead loaded.
  $effect(() => {
    if (slides.length && active >= slides.length - 3 && pager.hasMore) void pager.more();
  });

  function goTo(i: number) {
    const el = feed?.querySelector<HTMLElement>(`[data-i="${i}"]`);
    el?.scrollIntoView({ behavior: "smooth", block: "start" });
  }

  function onKey(e: KeyboardEvent) {
    const tag = (e.target as HTMLElement | null)?.tagName;
    if (tag === "INPUT" || tag === "TEXTAREA" || e.altKey || e.metaKey || e.ctrlKey) return;
    if (e.key !== "ArrowDown" && e.key !== "ArrowUp") return;
    e.preventDefault();
    e.stopImmediatePropagation();
    goTo(Math.max(0, Math.min(slides.length - 1, active + (e.key === "ArrowDown" ? 1 : -1))));
  }

  async function rate(s: Slide, r: "like" | "dislike") {
    const prev = liked[s.id];
    const next = prev === r ? undefined : r;
    liked[s.id] = next;
    try {
      await api.action(next === "like" ? "like" : next === "dislike" ? "dislike" : "remove_like", { video_id: s.id });
    } catch (e) {
      liked[s.id] = prev;
      showToast("error", errText(e));
    }
  }
</script>

<svelte:window onkeydowncapture={onKey} />

<div class="shorts" data-agent-readiness={pager.loading ? "loading" : pager.error ? "error" : "ready"} data-agent-surface="omnitube-shorts">
  {#if off}
    <State icon="shorts-off" title={$t("tube.shorts.off_title") as string} text={$t("tube.shorts.off_text") as string}>
      <button class="yt-btn yt-btn--outline" onclick={() => patchTubePrefs({ hide_shorts: false })}>{$t("tube.shorts.turn_on")}</button>
    </State>
  {:else if pager.loading && !pager.page}
    <div class="feed">
      <div class="slide"><div class="yt-skel frame-sk"></div></div>
    </div>
  {:else if pager.error && !pager.page}
    <State icon="shorts-off" error={pager.error} onretry={() => pager.load()} />
  {:else if !slides.length}
    <State icon="shorts-off" title={$t("tube.shorts.empty") as string} />
  {:else}
    <div class="feed" bind:this={feed}>
      {#each slides as s, i (s.id)}
        <section class="slide" data-i={i} aria-label={s.title}>
          <div class="frame">
            {#if i === active}
              <VideoPlayer videoId={s.id} short />
            {:else}
              <img class="poster" src={s.thumb} alt="" loading="lazy" decoding="async" referrerpolicy="no-referrer" />
            {/if}
            <div class="meta">
              {#if s.channel}
                <button class="chan" onclick={() => openChannel(s.channel!.id)}>
                  {#if s.channel.avatar}<img class="yt-avatar" src={bestAvatar(s.channel.avatar, 32)} width="32" height="32" alt="" referrerpolicy="no-referrer" />{/if}
                  <span>{s.channel.name}</span>
                </button>
              {/if}
              <p class="yt-clamp-2">{s.title}</p>
            </div>
          </div>
          <div class="rail">
            <button class="rb" aria-pressed={liked[s.id] === "like"} onclick={() => rate(s, "like")} aria-label={$t("tube.watch.like") as string}>
              <span class="c"><TIcon name={liked[s.id] === "like" ? "thumbs-up-fill" : "thumbs-up"} /></span>
              <span class="l">{$t("tube.watch.like")}</span>
            </button>
            <button class="rb" aria-pressed={liked[s.id] === "dislike"} onclick={() => rate(s, "dislike")} aria-label={$t("tube.watch.dislike") as string}>
              <span class="c"><TIcon name={liked[s.id] === "dislike" ? "thumbs-down-fill" : "thumbs-down"} /></span>
              <span class="l">{$t("tube.watch.dislike")}</span>
            </button>
            <button class="rb" onclick={() => copyText(`https://www.youtube.com/shorts/${s.id}`, $t("tube.menu.link_copied") as string)} aria-label={$t("tube.watch.share") as string}>
              <span class="c"><TIcon name="share-arrow" /></span>
              <span class="l">{$t("tube.watch.share")}</span>
            </button>
            <button class="rb" onclick={(e) => (dlFor = { slide: s, anchor: e.currentTarget })} aria-label={$t("tube.watch.download") as string}>
              <span class="c"><TIcon name="download" /></span>
              <span class="l">{$t("tube.watch.download")}</span>
            </button>
            <button class="rb" onclick={(e) => (menuFor = { slide: s, anchor: e.currentTarget })} aria-label={$t("tube.card.actions") as string}>
              <span class="c"><TIcon name="more-horiz" /></span>
            </button>
          </div>
        </section>
      {/each}
      {#if pager.loadingMore}
        <section class="slide"><div class="yt-skel frame-sk"></div></section>
      {/if}
    </div>
    <div class="nav">
      <button class="yt-btn yt-btn--icon navb" disabled={active === 0} onclick={() => goTo(active - 1)} aria-label={$t("tube.shorts.prev") as string}>
        <TIcon name="chevron-up" />
      </button>
      <button class="yt-btn yt-btn--icon navb" disabled={active >= slides.length - 1} onclick={() => goTo(active + 1)} aria-label={$t("tube.shorts.next") as string}>
        <TIcon name="chevron-down" />
      </button>
    </div>
  {/if}
</div>

{#if menuFor}
  <ItemMenu item={menuFor.slide.item} anchor={menuFor.anchor} onclose={() => (menuFor = null)} />
{/if}
{#if dlFor}
  <DownloadMenu videoId={dlFor.slide.id} title={dlFor.slide.title} anchor={dlFor.anchor} onclose={() => (dlFor = null)} />
{/if}
<UndoPill />

<style>
  .shorts {
    position: relative;
    height: calc(100vh - var(--yt-masthead-h));
    min-height: 480px;
  }
  .feed {
    height: 100%;
    overflow-y: auto;
    scroll-snap-type: y mandatory;
    scrollbar-width: none;
    overscroll-behavior: contain;
  }
  .feed::-webkit-scrollbar {
    display: none;
  }
  .slide {
    display: flex;
    align-items: flex-end;
    justify-content: center;
    gap: 16px;
    height: 100%;
    padding: 16px 0;
    scroll-snap-align: start;
    scroll-snap-stop: always;
  }
  .frame,
  .frame-sk {
    position: relative;
    height: 100%;
    aspect-ratio: 9 / 16;
    max-width: calc(100% - 96px);
    border-radius: 12px;
    overflow: hidden;
    background: #000;
  }
  .frame-sk {
    background: var(--yt-skeleton);
  }
  .frame :global(.vp) {
    height: 100%;
  }
  .poster {
    width: 100%;
    height: 100%;
    object-fit: cover;
    display: block;
  }
  .meta {
    position: absolute;
    left: 0;
    right: 0;
    top: 0;
    padding: 16px 16px 40px;
    color: #fff;
    background: linear-gradient(to bottom, rgba(0, 0, 0, 0.6), transparent);
    pointer-events: none;
  }
  .chan {
    display: inline-flex;
    align-items: center;
    gap: 8px;
    padding: 0;
    border: 0;
    font: var(--yt-type-body-strong);
    color: #fff;
    background: none;
    cursor: pointer;
    pointer-events: auto;
  }
  .meta p {
    margin: 8px 0 0;
    font: var(--yt-type-meta);
    text-shadow: 0 1px 2px rgba(0, 0, 0, 0.6);
  }
  .rail {
    display: flex;
    flex-direction: column;
    gap: 16px;
    padding-bottom: 8px;
  }
  .rb {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 4px;
    padding: 0;
    border: 0;
    font: var(--yt-type-caption);
    color: var(--yt-text);
    background: none;
    cursor: pointer;
  }
  .c {
    display: grid;
    place-items: center;
    width: 48px;
    height: 48px;
    border-radius: 50%;
    background: var(--yt-tonal);
    transition: background var(--yt-dur-fast);
  }
  .rb:hover .c {
    background: var(--yt-tonal-hover);
  }
  .rb[aria-pressed="true"] .c {
    color: var(--yt-on-inverted);
    background: var(--yt-bg-inverted);
  }
  .rb:focus-visible .c {
    outline: 2px solid var(--yt-link);
  }
  .nav {
    position: absolute;
    right: 24px;
    top: 50%;
    display: flex;
    flex-direction: column;
    gap: 16px;
    transform: translateY(-50%);
  }
  .navb {
    width: 56px;
    height: 56px;
    background: var(--yt-tonal);
  }
  .navb:hover {
    background: var(--yt-tonal-hover);
  }
  @container omnitube (max-width: 760px) {
    .nav {
      display: none;
    }
    .l {
      display: none;
    }
  }
</style>

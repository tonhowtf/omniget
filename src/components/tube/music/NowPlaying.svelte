<script lang="ts">
  // Now playing (spec §3.5): slides up over the content. Big cover (or the
  // song's video, muted and following the audio) on the left, tabs Up next /
  // Lyrics / Related on the right, background tinted with the cover's
  // dominant colour. Covers crossfade when the song changes.
  import { onDestroy } from "svelte";
  import { fade } from "svelte/transition";
  import { t } from "$lib/i18n";
  import * as api from "$lib/tube/api";
  import { isWebKit, mediaUrl, mimeCodecs, pickDirectVideo } from "$lib/tube/dash";
  import { thumbUrl } from "$lib/tube/format";
  import { currentSong, mplayer, next, prev, toggle } from "$lib/tube/music.svelte";
  import { tubePrefs } from "$lib/tube/prefs.svelte";
  import { ui } from "$lib/tube/ui.svelte";
  import type { Page } from "$lib/tube/types";
  import TIcon from "../TIcon.svelte";
  import MusicShelf from "./MusicShelf.svelte";
  import LyricsPanel from "./LyricsPanel.svelte";
  import QueuePanel from "./QueuePanel.svelte";

  type Tab = "next" | "lyrics" | "related";
  let tab = $state<Tab>("next");
  let media = $state<"song" | "video">("song");
  let tint = $state("rgb(40, 40, 40)");
  let tabEls: Record<Tab, HTMLButtonElement | undefined> = $state({ next: undefined, lyrics: undefined, related: undefined });
  let related = $state<Page | null>(null);
  let relatedFor = "";
  let relatedErr = $state(false);

  let song = $derived(currentSong());
  let art = $derived(song ? thumbUrl(song.thumbs, 544) : "");
  let artKey = $derived(song?.id ?? "");

  // underline follows the active tab
  let ind = $derived.by(() => {
    const el = tabEls[tab];
    return el ? { left: el.offsetLeft, width: el.offsetWidth } : { left: 0, width: 0 };
  });

  // dominant colour of the cover, darkened (spec: luminance ≤ 20 %)
  $effect(() => {
    const src = art;
    if (!src) return;
    const img = new Image();
    img.crossOrigin = "anonymous";
    img.referrerPolicy = "no-referrer";
    img.onload = () => {
      try {
        const c = document.createElement("canvas");
        c.width = 8;
        c.height = 8;
        const ctx = c.getContext("2d");
        if (!ctx) return;
        ctx.drawImage(img, 0, 0, 8, 8);
        const d = ctx.getImageData(0, 0, 8, 8).data;
        let r = 0;
        let g = 0;
        let b = 0;
        for (let i = 0; i < d.length; i += 4) {
          r += d[i];
          g += d[i + 1];
          b += d[i + 2];
        }
        const n = d.length / 4;
        r /= n;
        g /= n;
        b /= n;
        const lum = (0.2126 * r + 0.7152 * g + 0.0722 * b) / 255;
        const k = lum > 0.2 ? 0.2 / lum : 1;
        tint = `rgb(${Math.round(r * k)}, ${Math.round(g * k)}, ${Math.round(b * k)})`;
      } catch {
        tint = "rgb(40, 40, 40)";
      }
    };
    img.src = src;
  });

  // related tab loads on demand
  $effect(() => {
    const id = mplayer.relatedId;
    if (tab !== "related" || !id || id === relatedFor) return;
    relatedFor = id;
    related = null;
    relatedErr = false;
    api
      .musicRelated(id)
      .then((p) => {
        if (mplayer.relatedId === id) related = p;
      })
      .catch(() => (relatedErr = true));
  });

  // video mode: a muted <video> of the song that follows the audio clock
  let vid = $state<HTMLVideoElement>();
  let vidSrc = $state("");
  let vidFor = "";
  let vidErr = $state(false);

  $effect(() => {
    if (media !== "video" || !song || song.id === vidFor) return;
    const id = song.id;
    vidFor = id;
    vidSrc = "";
    vidErr = false;
    api
      .streams(id, false)
      .then((set) => {
        if (currentSong()?.id !== id) return;
        const probe = document.createElement("video");
        const f = pickDirectVideo(set.formats, { codec: tubePrefs().prefer_codec, webkit: isWebKit(), supports: (x) => probe.canPlayType(mimeCodecs(x)) !== "" }, 720);
        if (!f) throw new Error("no video");
        vidSrc = mediaUrl(set, f.itag);
      })
      .catch(() => (vidErr = true));
  });

  $effect(() => {
    const v = vid;
    if (!v || !vidSrc) return;
    const pos = mplayer.position;
    if (Math.abs(v.currentTime - pos) > 0.35 && v.readyState >= 1) v.currentTime = pos;
    if (mplayer.playing && v.paused) void v.play().catch(() => {});
    if (!mplayer.playing && !v.paused) v.pause();
  });

  onDestroy(() => vid?.pause());

  function close() {
    ui.nowPlaying = false;
  }
</script>

<section
  class="np"
  style:--tint={tint}
  aria-label={$t("tube.mplayer.now_playing") as string}
  data-agent-readiness={song ? "ready" : "empty"}
>
  <div class="bg" aria-hidden="true">
    {#key artKey}
      {#if art}<img src={art} alt="" referrerpolicy="no-referrer" transition:fade={{ duration: 500 }} />{/if}
    {/key}
  </div>

  <div class="left">
    <div class="seg" role="tablist" aria-label={$t("tube.mplayer.media") as string}>
      <button role="tab" aria-selected={media === "song"} class:on={media === "song"} onclick={() => (media = "song")}>{$t("tube.mplayer.song")}</button>
      <button role="tab" aria-selected={media === "video"} class:on={media === "video"} onclick={() => (media = "video")}>{$t("tube.mplayer.video")}</button>
    </div>
    <div class="art" class:wide={media === "video"}>
      {#if media === "video"}
        {#if vidSrc}
          <!-- svelte-ignore a11y_media_has_caption -->
          <video bind:this={vid} src={vidSrc} muted playsinline preload="auto"></video>
        {:else if vidErr}
          <div class="novid">{$t("tube.mplayer.no_video")}</div>
        {:else}
          <div class="yt-skel fill"></div>
        {/if}
      {:else}
        {#key artKey}
          {#if art}
            <img src={art} alt="" referrerpolicy="no-referrer" decoding="async" transition:fade={{ duration: 400 }} />
          {:else}
            <div class="ph"><TIcon name="music-note" size={72} /></div>
          {/if}
        {/key}
      {/if}
      <div class="hov">
        <button class="c" onclick={prev} aria-label={$t("tube.player.prev") as string}><TIcon name="prev" size={28} /></button>
        <button class="c big" onclick={toggle} aria-label={$t(mplayer.playing ? "tube.player.pause" : "tube.player.play") as string}>
          <TIcon name={mplayer.playing ? "pause" : "play"} size={36} />
        </button>
        <button class="c" onclick={next} aria-label={$t("tube.player.next") as string}><TIcon name="next" size={28} /></button>
      </div>
    </div>
  </div>

  <div class="right">
    <div class="tabs" role="tablist">
      <button bind:this={tabEls.next} role="tab" aria-selected={tab === "next"} class:on={tab === "next"} onclick={() => (tab = "next")}>{$t("tube.mplayer.up_next")}</button>
      <button bind:this={tabEls.lyrics} role="tab" aria-selected={tab === "lyrics"} class:on={tab === "lyrics"} onclick={() => (tab = "lyrics")}>{$t("tube.mplayer.lyrics")}</button>
      <button
        bind:this={tabEls.related}
        role="tab"
        aria-selected={tab === "related"}
        class:on={tab === "related"}
        disabled={!mplayer.relatedId}
        onclick={() => (tab = "related")}>{$t("tube.mplayer.related")}</button
      >
      <span class="ind" style:transform="translateX({ind.left}px)" style:width="{ind.width}px"></span>
    </div>
    <div class="panel" role="tabpanel">
      {#if tab === "next"}
        <QueuePanel />
      {:else if tab === "lyrics"}
        <LyricsPanel />
      {:else if related}
        <div class="rel yt-scroll">
          {#each related.shelves as sh (sh.id)}<MusicShelf shelf={sh} />{/each}
        </div>
      {:else if relatedErr}
        <div class="novid">{$t("tube.mplayer.related_error")}</div>
      {:else}
        <div class="rel-skel">
          {#each [0, 1, 2, 3, 4] as i (i)}<span class="yt-skel"></span>{/each}
        </div>
      {/if}
    </div>
  </div>

  <button class="close" onclick={close} aria-label={$t("tube.player.close") as string}><TIcon name="chevron-down" size={28} /></button>
</section>

<style>
  .np {
    position: absolute;
    inset: 0;
    z-index: 15;
    display: grid;
    grid-template-columns: minmax(0, 1fr) minmax(320px, 40%);
    gap: clamp(32px, 5cqw, 96px);
    padding: 24px clamp(24px, 5cqw, 96px) 24px;
    overflow: hidden;
    color: var(--ytm-text);
    background: linear-gradient(to bottom, var(--tint), var(--ytm-bg) 70%);
    transition: background 600ms var(--ytm-ease);
    animation: up var(--ytm-dur-page) var(--ytm-ease);
  }
  @keyframes up {
    from {
      transform: translateY(100%);
    }
  }
  .bg {
    position: absolute;
    inset: -80px;
    z-index: -1;
    pointer-events: none;
    opacity: 0.35;
  }
  .bg img {
    position: absolute;
    inset: 0;
    width: 100%;
    height: 100%;
    object-fit: cover;
    filter: blur(80px) saturate(1.4);
  }
  .bg::after {
    content: "";
    position: absolute;
    inset: 0;
    background: linear-gradient(to bottom, transparent 20%, var(--ytm-bg) 80%);
  }
  .left {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 20px;
    min-width: 0;
    min-height: 0;
  }
  .seg {
    display: inline-flex;
    padding: 3px;
    border-radius: 999px;
    background: rgba(255, 255, 255, 0.1);
  }
  .seg button {
    height: 32px;
    padding: 0 18px;
    border: 0;
    border-radius: 999px;
    background: transparent;
    color: var(--ytm-text-2);
    font: 500 14px/1 var(--yt-font);
    cursor: pointer;
    transition:
      background var(--yt-dur-base) var(--yt-ease-standard),
      color var(--yt-dur-base);
  }
  .seg button.on {
    background: rgba(255, 255, 255, 0.2);
    color: #fff;
  }
  .art {
    position: relative;
    width: min(100%, clamp(200px, calc(100vh - 330px), 640px));
    aspect-ratio: 1;
    border-radius: 8px;
    overflow: hidden;
    background: rgba(255, 255, 255, 0.06);
    box-shadow: 0 16px 48px rgba(0, 0, 0, 0.5);
    display: grid;
  }
  .art.wide {
    aspect-ratio: 16 / 9;
    width: min(100%, 960px);
  }
  .art > img,
  .art > video,
  .art > .ph,
  .art > .fill,
  .art > .novid {
    grid-area: 1 / 1;
    width: 100%;
    height: 100%;
    object-fit: cover;
    display: block;
  }
  .art > video {
    object-fit: contain;
    background: #000;
  }
  .ph,
  .novid {
    display: grid;
    place-items: center;
    color: var(--ytm-text-2);
    text-align: center;
    padding: 16px;
  }
  .hov {
    grid-area: 1 / 1;
    z-index: 1;
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 28px;
    background: linear-gradient(transparent 40%, rgba(0, 0, 0, 0.6));
    opacity: 0;
    transition: opacity var(--yt-dur-base) var(--yt-ease-standard);
  }
  .art:hover .hov,
  .art:focus-within .hov {
    opacity: 1;
  }
  .c {
    display: grid;
    place-items: center;
    width: 52px;
    height: 52px;
    border: 0;
    border-radius: 50%;
    color: #fff;
    background: transparent;
    cursor: pointer;
    filter: drop-shadow(0 1px 3px rgba(0, 0, 0, 0.6));
  }
  .c.big {
    width: 72px;
    height: 72px;
    background: rgba(255, 255, 255, 0.15);
    backdrop-filter: blur(12px);
  }
  .c:hover {
    background: rgba(255, 255, 255, 0.2);
  }
  .right {
    display: flex;
    flex-direction: column;
    min-width: 0;
    min-height: 0;
  }
  .tabs {
    position: relative;
    display: flex;
    height: 68px;
    flex: none;
    border-bottom: 1px solid var(--ytm-divider);
  }
  .tabs button {
    flex: 1;
    border: 0;
    background: none;
    color: rgba(255, 255, 255, 0.5);
    font: 500 14px/1 var(--yt-font);
    text-transform: uppercase;
    letter-spacing: 0.05em;
    cursor: pointer;
    transition: color var(--yt-dur-base);
  }
  .tabs button:hover:not(:disabled),
  .tabs button.on {
    color: #fff;
  }
  .tabs button:disabled {
    opacity: 0.4;
    cursor: default;
  }
  .tabs button:focus-visible,
  .seg button:focus-visible,
  .c:focus-visible,
  .close:focus-visible {
    outline: 2px solid var(--ytm-link);
    outline-offset: -2px;
  }
  .ind {
    position: absolute;
    left: 0;
    bottom: -1px;
    height: 2px;
    background: #fff;
    transition:
      transform 250ms var(--yt-ease-standard),
      width 250ms var(--yt-ease-standard);
  }
  .panel {
    flex: 1;
    min-height: 0;
  }
  .rel {
    height: 100%;
    overflow-y: auto;
    padding: 8px 0 24px;
  }
  .rel-skel {
    display: flex;
    flex-direction: column;
    gap: 12px;
    padding: 16px 8px;
  }
  .rel-skel .yt-skel {
    height: 48px;
  }
  .close {
    position: absolute;
    top: 12px;
    left: 12px;
    display: grid;
    place-items: center;
    width: 40px;
    height: 40px;
    border: 0;
    border-radius: 50%;
    color: #fff;
    background: rgba(0, 0, 0, 0.25);
    cursor: pointer;
  }
  .close:hover {
    background: rgba(255, 255, 255, 0.15);
  }

  @container omnitube (max-width: 900px) {
    .np {
      grid-template-columns: 1fr;
      grid-template-rows: auto minmax(0, 1fr);
      gap: 16px;
      padding: 56px 16px 16px;
    }
    .art {
      width: min(100%, 280px);
    }
    .art.wide {
      width: min(100%, 520px);
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .np {
      animation: none;
    }
    .ind,
    .hov {
      transition: none;
    }
  }
</style>

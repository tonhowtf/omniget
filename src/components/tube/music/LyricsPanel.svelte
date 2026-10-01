<script lang="ts">
  import { t } from "$lib/i18n";
  import { currentSong, lyricsFor, mplayer, seek } from "$lib/tube/music.svelte";
  import type { Lyrics } from "$lib/tube/types";

  let lyrics = $state<Lyrics | null>(null);
  let loading = $state(false);
  let failed = $state(false);
  let box = $state<HTMLDivElement>();
  let userScrollAt = 0;
  let loadedFor = "";

  let song = $derived(currentSong());

  $effect(() => {
    const s = song;
    if (!s || s.id === loadedFor) return;
    loadedFor = s.id;
    lyrics = null;
    failed = false;
    loading = true;
    lyricsFor(s)
      .then((l) => {
        if (currentSong()?.id === s.id) lyrics = l;
      })
      .catch(() => {
        if (currentSong()?.id === s.id) failed = true;
      })
      .finally(() => {
        if (currentSong()?.id === s.id) loading = false;
      });
  });

  let active = $derived.by(() => {
    if (!lyrics?.synced) return -1;
    const ms = mplayer.position * 1000 + 150;
    let a = -1;
    for (let i = 0; i < lyrics.lines.length; i++) {
      const st = lyrics.lines[i].start_ms;
      if (st != null && st <= ms) a = i;
      else if (st != null && st > ms) break;
    }
    return a;
  });

  $effect(() => {
    const i = active;
    if (i < 0 || !box || Date.now() - userScrollAt < 4000) return;
    const el = box.querySelector<HTMLElement>(`[data-l="${i}"]`);
    if (!el) return;
    const top = el.offsetTop - box.clientHeight / 2 + el.clientHeight / 2;
    box.scrollTo({ top, behavior: matchMedia("(prefers-reduced-motion: reduce)").matches ? "auto" : "smooth" });
  });

  let sourceName = $derived(lyrics?.source === "lrclib" ? "LRCLIB" : lyrics?.source === "youtube" ? "YouTube Music" : lyrics?.source ?? "");
</script>

<div class="lp">
  {#if loading}
    <div class="skel">
      {#each [80, 65, 90, 50, 75, 60] as w, i (i)}<span class="yt-skel" style:width="{w}%"></span>{/each}
    </div>
  {:else if !lyrics || failed || !lyrics.lines.length}
    <div class="empty">
      <img src="/icons/tube/lyrics-128.webp" alt="" width="96" height="96" />
      <p>{$t(failed ? "tube.mplayer.lyrics_error" : "tube.mplayer.lyrics_none")}</p>
    </div>
  {:else}
    <div
      class="lines yt-scroll"
      class:synced={lyrics.synced}
      bind:this={box}
      role="region"
      aria-label={$t("tube.mplayer.lyrics") as string}
      onwheel={() => (userScrollAt = Date.now())}
      ontouchmove={() => (userScrollAt = Date.now())}
    >
      {#each lyrics.lines as l, i (i)}
        {#if lyrics.synced && l.start_ms != null}
          <button class="ln" class:on={i === active} class:past={i < active} data-l={i} onclick={() => (seek((l.start_ms ?? 0) / 1000), (userScrollAt = 0))}>
            {l.text || "♪"}
          </button>
        {:else}
          <p class="ln plain" data-l={i}>{l.text || " "}</p>
        {/if}
      {/each}
      <p class="foot">
        {#if lyrics.footer}{lyrics.footer}{:else if sourceName}{$t("tube.mplayer.lyrics_source", { name: sourceName })}{/if}
      </p>
    </div>
  {/if}
</div>

<style>
  .lp {
    height: 100%;
    min-height: 0;
    display: flex;
    flex-direction: column;
  }
  .lines {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    padding: 24px 8px 40%;
    mask-image: linear-gradient(transparent, #000 8%, #000 88%, transparent);
    -webkit-mask-image: linear-gradient(transparent, #000 8%, #000 88%, transparent);
  }
  .ln {
    display: block;
    width: 100%;
    margin: 0;
    padding: 6px 4px;
    border: 0;
    border-radius: 6px;
    background: none;
    text-align: left;
    color: rgba(255, 255, 255, 0.5);
    font: 700 24px/32px var(--yt-font-display, var(--yt-font));
    cursor: pointer;
    transition:
      color 250ms var(--yt-ease-standard),
      transform 250ms var(--yt-ease-standard);
    transform-origin: left center;
  }
  .ln:hover {
    color: rgba(255, 255, 255, 0.8);
  }
  .ln:focus-visible {
    outline: 2px solid var(--ytm-link);
  }
  .ln.past {
    color: rgba(255, 255, 255, 0.35);
  }
  .ln.on {
    color: #fff;
    transform: scale(1.02);
  }
  .plain {
    font: 500 18px/28px var(--yt-font);
    color: var(--ytm-text);
    cursor: default;
    padding: 0 4px;
    white-space: pre-wrap;
  }
  .lines:not(.synced) {
    padding-bottom: 24px;
  }
  .foot {
    margin: 24px 4px 0;
    font: 400 12px/16px var(--yt-font);
    color: var(--ytm-text-3);
  }
  .skel {
    display: flex;
    flex-direction: column;
    gap: 16px;
    padding: 24px 8px;
  }
  .skel .yt-skel {
    height: 24px;
  }
  .empty {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 12px;
    margin: auto;
    padding: 32px;
    color: var(--ytm-text-2);
    text-align: center;
  }
  @media (prefers-reduced-motion: reduce) {
    .ln {
      transition: none;
    }
  }
</style>

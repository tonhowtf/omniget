<script lang="ts">
  import { goto } from "$app/navigation";
  import { t } from "$lib/i18n";
  import { agentActions } from "$lib/native-agent/actions";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import { fmtDuration, thumbUrl } from "$lib/tube/format";
  import {
    currentSong,
    cycleRepeat,
    mplayer,
    next,
    prev,
    rateSong,
    seek,
    setVolume,
    stopMusic,
    toggle,
    toggleMute,
    toggleShuffle,
  } from "$lib/tube/music.svelte";
  import { go, setFace, ui } from "$lib/tube/ui.svelte";
  import TIcon from "../TIcon.svelte";
  import ItemMenu from "../views/music/ItemMenu.svelte";

  let { floating = false }: { floating?: boolean } = $props();

  let song = $derived(currentSong());
  let cover = $derived(song ? thumbUrl(song.thumbs, 60) : "");
  let bar = $state<HTMLDivElement>();
  let hover = $state<number | null>(null);
  let drag = $state<number | null>(null);
  let barW = $state(0);
  let volOpen = $state(false);
  let volEl = $state<HTMLDivElement>();
  let volDrag = $state(false);
  let menu = $state<{ x: number; y: number } | null>(null);

  let dur = $derived(mplayer.duration || song?.duration_s || 0);
  let pos = $derived(drag != null ? drag * dur : mplayer.position);
  let pct = $derived(dur > 0 ? Math.min(100, (pos / dur) * 100) : 0);
  let volIcon = $derived(mplayer.muted || mplayer.volume === 0 ? "mute" : mplayer.volume < 0.5 ? "volume-low" : "volume");
  let subtitle = $derived.by(() => {
    if (!song) return [] as { text: string; artist?: string; album?: string }[];
    const parts: { text: string; artist?: string; album?: string }[] = song.artists.map((a) => ({ text: a.name, artist: a.id }));
    if (song.album) parts.push({ text: song.album.name, album: song.album.id });
    return parts;
  });

  function fracAt(x: number): number {
    if (!bar) return 0;
    const r = bar.getBoundingClientRect();
    return Math.min(1, Math.max(0, (x - r.left) / r.width));
  }
  function pdown(e: PointerEvent) {
    if (e.button !== 0 || dur <= 0) return;
    (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
    drag = fracAt(e.clientX);
  }
  function pmove(e: PointerEvent) {
    const f = fracAt(e.clientX);
    hover = f;
    if (drag != null) drag = f;
  }
  function pup(e: PointerEvent) {
    if (drag == null) return;
    const f = fracAt(e.clientX);
    drag = null;
    seek(f * dur);
  }
  function pkey(e: KeyboardEvent) {
    if (e.key === "ArrowLeft" || e.key === "ArrowRight") {
      e.preventDefault();
      e.stopPropagation();
      seek(mplayer.position + (e.key === "ArrowLeft" ? -5 : 5));
    }
  }

  function volAt(x: number) {
    if (!volEl) return;
    const r = volEl.getBoundingClientRect();
    setVolume(Math.min(1, Math.max(0, (x - r.left) / r.width)));
  }

  function openNowPlaying() {
    if (floating) {
      setFace("music");
      ui.nowPlaying = true;
      void goto("/youtube");
      return;
    }
    ui.nowPlaying = !ui.nowPlaying;
  }

  function clickBar(e: MouseEvent) {
    if ((e.target as HTMLElement).closest("button, a, [role=slider]")) return;
    openNowPlaying();
  }

  async function like(v: boolean) {
    if (!song) return;
    try {
      await rateSong(song, song.liked === v ? null : v);
    } catch (e) {
      showToast("error", String(e));
    }
  }

  function openArtist(id: string) {
    if (floating) void goto("/youtube");
    go({ name: "m_artist", id });
  }
  function openAlbum(id: string) {
    if (floating) void goto("/youtube");
    go({ name: "m_album", id });
  }
</script>

{#if song}
  <div class="pbar" class:floating class:open={ui.nowPlaying && !floating} role="region" aria-label={$t("tube.mplayer.player") as string}>
    <div
      class="prog"
      bind:this={bar}
      bind:clientWidth={barW}
      class:dragging={drag != null}
      role="slider"
      tabindex="0"
      aria-label={$t("tube.player.seek") as string}
      aria-valuemin={0}
      aria-valuemax={Math.round(dur)}
      aria-valuenow={Math.round(pos)}
      aria-valuetext={fmtDuration(pos)}
      onpointerdown={pdown}
      onpointermove={pmove}
      onpointerup={pup}
      onpointerleave={() => drag == null && (hover = null)}
      onkeydown={pkey}
    >
      <span class="track"><i class="fill" style:width="{pct}%"></i></span>
      <span class="knob" style:left="{pct}%"></span>
      {#if hover != null && dur > 0}
        <span class="tip" style:left="{Math.min(Math.max(hover * barW, 24), barW - 24)}px">{fmtDuration(hover * dur)}</span>
      {/if}
    </div>

    <!-- svelte-ignore a11y_click_events_have_key_events -->
    <div class="grid" role="presentation" onclick={clickBar}>
      <div class="start">
        <button class="ib" onclick={prev} aria-label={$t("tube.player.prev") as string}><TIcon name="prev" size={22} /></button>
        <button class="ib play" onclick={toggle} aria-label={$t(mplayer.playing ? "tube.player.pause" : "tube.player.play") as string}>
          {#if mplayer.buffering && !mplayer.playing}
            <span class="spin" aria-hidden="true"></span>
          {:else}
            <TIcon name={mplayer.playing ? "pause" : "play"} size={30} />
          {/if}
        </button>
        <button class="ib" onclick={next} aria-label={$t("tube.player.next") as string}><TIcon name="next" size={22} /></button>
        <span class="time">{fmtDuration(pos)} / {fmtDuration(dur)}</span>
      </div>

      <div class="middle">
        <button class="art" onclick={openNowPlaying} aria-label={$t("tube.mplayer.now_playing") as string}>
          {#if cover}<img src={cover} alt="" width="40" height="40" decoding="async" referrerpolicy="no-referrer" />{/if}
        </button>
        <div class="meta">
          <span class="title yt-clamp-1">{song.title}</span>
          <span class="sub yt-clamp-1">
            {#each subtitle as p, i (i)}
              {#if i > 0}<span class="sep">{i === subtitle.length - 1 && p.album ? " • " : ", "}</span>{/if}
              {#if p.artist}<button class="lnk" onclick={() => openArtist(p.artist!)}>{p.text}</button>
              {:else if p.album}<button class="lnk" onclick={() => openAlbum(p.album!)}>{p.text}</button>
              {:else}{p.text}{/if}
            {/each}
          </span>
        </div>
        {#if !floating}
          <button class="ib sm" class:on={song.liked === false} onclick={() => like(false)} aria-pressed={song.liked === false} aria-label={$t("tube.mplayer.dislike") as string}>
            <TIcon name={song.liked === false ? "thumbs-down-fill" : "thumbs-down"} size={20} />
          </button>
          <button class="ib sm" class:on={song.liked === true} onclick={() => like(true)} aria-pressed={song.liked === true} aria-label={$t("tube.mplayer.like") as string}>
            <span class="pop" class:liked={song.liked === true}><TIcon name={song.liked === true ? "thumbs-up-fill" : "thumbs-up"} size={20} /></span>
          </button>
          <button class="ib sm" onclick={(e) => (menu = { x: e.clientX, y: e.clientY })} aria-label={$t("tube.mplayer.more") as string}>
            <TIcon name="more-vert" size={20} />
          </button>
        {/if}
      </div>

      <div class="end">
        <div class="vol" class:open={volOpen || volDrag} role="group" onpointerenter={() => (volOpen = true)} onpointerleave={() => (volOpen = false)}>
          <div
            class="vtrack"
            bind:this={volEl}
            role="slider"
            tabindex="0"
            aria-label={$t("tube.player.volume") as string}
            aria-valuemin={0}
            aria-valuemax={100}
            aria-valuenow={Math.round((mplayer.muted ? 0 : mplayer.volume) * 100)}
            onpointerdown={(e) => {
              (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
              volDrag = true;
              volAt(e.clientX);
            }}
            onpointermove={(e) => volDrag && volAt(e.clientX)}
            onpointerup={() => (volDrag = false)}
            onfocus={() => (volOpen = true)}
            onblur={() => (volOpen = false)}
            onkeydown={(e) => {
              if (e.key === "ArrowLeft" || e.key === "ArrowRight") {
                e.preventDefault();
                e.stopPropagation();
                setVolume(mplayer.volume + (e.key === "ArrowLeft" ? -0.05 : 0.05));
              }
            }}
          >
            <span class="vfill" style:width="{(mplayer.muted ? 0 : mplayer.volume) * 100}%"></span>
            <span class="vknob" style:left="{(mplayer.muted ? 0 : mplayer.volume) * 100}%"></span>
          </div>
          <button class="ib sm" onclick={toggleMute} aria-label={$t(mplayer.muted ? "tube.player.unmute" : "tube.player.mute") as string}>
            <TIcon name={volIcon} size={22} />
          </button>
        </div>
        {#if !floating}
          <button class="ib sm" class:on={mplayer.repeat !== "off"} onclick={cycleRepeat} aria-label={$t(`tube.mplayer.repeat_${mplayer.repeat}`) as string} title={$t(`tube.mplayer.repeat_${mplayer.repeat}`) as string}>
            <TIcon name={mplayer.repeat === "one" ? "repeat-one" : "repeat"} size={22} />
          </button>
          <button class="ib sm" class:on={mplayer.shuffle} onclick={toggleShuffle} aria-pressed={mplayer.shuffle} aria-label={$t("tube.mplayer.shuffle") as string} title={$t("tube.mplayer.shuffle") as string}>
            <TIcon name="shuffle" size={22} />
          </button>
        {/if}
        <button use:agentActions={["click"]} class="ib sm expand" class:flip={ui.nowPlaying && !floating} onclick={openNowPlaying} aria-expanded={ui.nowPlaying && !floating} aria-label={$t("tube.mplayer.now_playing") as string}>
          <TIcon name="chevron-up" size={24} />
        </button>
        {#if floating}
          <button class="ib sm" onclick={stopMusic} aria-label={$t("tube.player.close") as string}><TIcon name="close" size={20} /></button>
        {/if}
      </div>
    </div>
  </div>
  {#if menu && song}
    <ItemMenu item={song} x={menu.x} y={menu.y} onclose={() => (menu = null)} />
  {/if}
{/if}

<style>
  .pbar {
    position: relative;
    height: var(--ytm-player-h, 72px);
    color: var(--ytm-text, #fff);
    background: var(--ytm-bg-raised, #212121);
    font-family: var(--yt-font);
    animation: up 300ms var(--ytm-ease, ease-out);
    z-index: 20;
  }
  @keyframes up {
    from {
      transform: translateY(100%);
    }
  }
  .floating {
    position: fixed;
    right: 16px;
    bottom: calc(16px + var(--overlay-inset, 0px));
    width: min(720px, calc(100vw - 32px));
    height: 64px;
    border-radius: 14px;
    overflow: visible;
    background: color-mix(in srgb, var(--ytm-bg-raised, #212121) 90%, transparent);
    backdrop-filter: blur(20px) saturate(1.5);
    -webkit-backdrop-filter: blur(20px) saturate(1.5);
    box-shadow:
      0 0 0 1px rgba(255, 255, 255, 0.08),
      0 14px 36px rgba(0, 0, 0, 0.4);
    pointer-events: auto;
    z-index: 305;
  }
  .prog {
    position: absolute;
    left: 0;
    right: 0;
    top: -7px;
    height: 16px;
    display: flex;
    align-items: center;
    cursor: pointer;
    touch-action: none;
    outline: none;
    z-index: 2;
  }
  .floating .prog {
    left: 12px;
    right: 12px;
  }
  .track {
    position: relative;
    width: 100%;
    height: 4px;
    background: var(--ytm-progress-track, rgba(255, 255, 255, 0.3));
    transform: scaleY(0.5);
    transition: transform var(--yt-dur-fast) var(--yt-ease-exit);
  }
  .prog:hover .track,
  .prog.dragging .track,
  .prog:focus-visible .track {
    transform: scaleY(1);
  }
  .fill {
    position: absolute;
    left: 0;
    top: 0;
    bottom: 0;
    background: var(--ytm-progress, #f03);
  }
  .knob {
    position: absolute;
    top: 50%;
    width: 12px;
    height: 12px;
    margin: -6px 0 0 -6px;
    border-radius: 50%;
    background: var(--ytm-progress, #f03);
    transform: scale(0);
    transition: transform var(--yt-dur-fast) var(--yt-ease-exit);
  }
  .prog:hover .knob,
  .prog.dragging .knob,
  .prog:focus-visible .knob {
    transform: scale(1);
  }
  .tip {
    position: absolute;
    bottom: 16px;
    transform: translateX(-50%);
    padding: 2px 6px;
    border-radius: 4px;
    background: var(--ytm-bg-raised, #212121);
    box-shadow: 0 2px 8px rgba(0, 0, 0, 0.4);
    font: 500 12px/16px var(--yt-font);
    pointer-events: none;
  }
  .grid {
    display: grid;
    grid-template-columns: minmax(0, 1fr) auto minmax(0, 1fr);
    align-items: center;
    height: 100%;
    padding: 0 16px 0 8px;
    gap: 16px;
    cursor: pointer;
  }
  .start,
  .end,
  .middle {
    display: flex;
    align-items: center;
    min-width: 0;
  }
  .start {
    gap: 4px;
  }
  .end {
    justify-content: flex-end;
    gap: 4px;
  }
  .middle {
    gap: 8px;
    max-width: min(620px, 46cqw);
  }
  .ib {
    display: grid;
    place-items: center;
    width: 40px;
    height: 40px;
    border: 0;
    border-radius: 50%;
    padding: 0;
    color: var(--ytm-text, #fff);
    background: transparent;
    cursor: pointer;
    flex: none;
    transition:
      background var(--yt-dur-base) var(--yt-ease-standard),
      color var(--yt-dur-base);
  }
  .ib:hover {
    background: rgba(255, 255, 255, 0.1);
  }
  .ib:focus-visible,
  .art:focus-visible,
  .lnk:focus-visible,
  .prog:focus-visible,
  .vtrack:focus-visible {
    outline: 2px solid var(--ytm-link, #3ea6ff);
    outline-offset: 1px;
  }
  .ib.sm {
    color: var(--ytm-text-2, #aaa);
  }
  .ib.sm:hover,
  .ib.sm.on {
    color: var(--ytm-text, #fff);
  }
  .play {
    width: 44px;
    height: 44px;
  }
  .time {
    margin-left: 8px;
    font: 400 12px/16px var(--yt-font);
    color: var(--ytm-text-2, #aaa);
    font-variant-numeric: tabular-nums;
    white-space: nowrap;
  }
  .art {
    width: 40px;
    height: 40px;
    padding: 0;
    border: 0;
    border-radius: 4px;
    overflow: hidden;
    background: rgba(255, 255, 255, 0.1);
    cursor: pointer;
    flex: none;
  }
  .art img {
    width: 100%;
    height: 100%;
    object-fit: cover;
    display: block;
  }
  .meta {
    display: flex;
    flex-direction: column;
    min-width: 0;
    max-width: 360px;
    margin-right: 8px;
  }
  .title {
    font: 500 16px/22px var(--yt-font);
    color: var(--ytm-text, #fff);
  }
  .sub {
    font: 400 14px/20px var(--yt-font);
    color: var(--ytm-text-2, #aaa);
  }
  .lnk {
    padding: 0;
    border: 0;
    background: none;
    color: inherit;
    font: inherit;
    cursor: pointer;
  }
  .lnk:hover {
    text-decoration: underline;
    color: var(--ytm-text, #fff);
  }
  .pop.liked {
    display: grid;
    animation: pop 350ms var(--yt-ease-standard);
  }
  @keyframes pop {
    50% {
      transform: scale(1.25);
    }
  }
  .vol {
    display: flex;
    align-items: center;
  }
  .vtrack {
    position: relative;
    width: 0;
    height: 24px;
    overflow: hidden;
    cursor: pointer;
    outline: none;
    touch-action: none;
    transition:
      width var(--yt-dur-base) var(--yt-ease-standard),
      margin var(--yt-dur-base) var(--yt-ease-standard);
  }
  .vtrack::before {
    content: "";
    position: absolute;
    left: 0;
    right: 0;
    top: 50%;
    height: 2px;
    margin-top: -1px;
    background: rgba(255, 255, 255, 0.3);
  }
  .vol.open .vtrack {
    width: 100px;
    margin-right: 6px;
    overflow: visible;
  }
  .vfill {
    position: absolute;
    left: 0;
    top: 50%;
    height: 2px;
    margin-top: -1px;
    background: #fff;
  }
  .vknob {
    position: absolute;
    top: 50%;
    width: 12px;
    height: 12px;
    margin: -6px 0 0 -6px;
    border-radius: 50%;
    background: #fff;
  }
  .expand :global(svg) {
    transition: transform var(--ytm-dur-page, 300ms) var(--ytm-ease, ease);
  }
  .expand.flip :global(svg) {
    transform: rotate(180deg);
  }
  .spin {
    width: 24px;
    height: 24px;
    border-radius: 50%;
    border: 3px solid rgba(255, 255, 255, 0.25);
    border-top-color: #fff;
    animation: spin 0.9s linear infinite;
  }
  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }
  .floating .grid {
    grid-template-columns: auto minmax(0, 1fr) auto;
    padding: 0 8px 0 4px;
    gap: 8px;
  }
  .floating .time {
    display: none;
  }
  .floating .middle {
    order: -1;
    max-width: none;
    padding-left: 8px;
  }
  .floating .start {
    order: 0;
    justify-content: center;
  }

  @container omnitube (max-width: 820px) {
    .time,
    .vol {
      display: none;
    }
  }
  @container omnitube (max-width: 620px) {
    .middle .sm {
      display: none;
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .pbar {
      animation: none;
    }
    .track,
    .knob,
    .vtrack,
    .expand :global(svg) {
      transition: none;
    }
  }
</style>

<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { t } from "$lib/i18n";
  import { fmtTime, qualityLabel } from "$lib/music/library";
  import { patchTracksLocal } from "$lib/music/library-store.svelte";
  import {
    cycleRepeat,
    next,
    player,
    prefs,
    prev,
    seek,
    setVolume,
    toggle,
    toggleMute,
    toggleShuffle,
  } from "$lib/music/player-store.svelte";
  import { go, ui } from "$lib/music/ui-store.svelte";
  import Cover from "./Cover.svelte";
  import StationIcon from "./StationIcon.svelte";
  import MusicIcon from "./MusicIcon.svelte";
  import Waveform from "./Waveform.svelte";
  import Visualizer from "./Visualizer.svelte";

  let { peaks = [] }: { peaks?: ArrayLike<number> } = $props();

  let cur = $derived(player.current);
  let volIcon = $derived(prefs.muted || prefs.volume === 0 ? "mute" : prefs.volume < 0.5 ? "volume-low" : "volume");
  let markers = $derived(
    cur && (prefs.automix || prefs.trimSilence)
      ? [
          ...(cur.intro_s && cur.intro_s > 0.5 ? [{ at: cur.intro_s, kind: "intro" }] : []),
          ...(cur.outro_s ? [{ at: cur.outro_s, kind: "outro" }] : []),
        ]
      : [],
  );

  async function like() {
    if (!cur) return;
    const liked = !cur.liked;
    patchTracksLocal([cur.id], { liked });
    await invoke("music_set_liked", { ids: [cur.id], liked }).catch(() => {});
  }

  function wheelVolume(e: WheelEvent) {
    e.preventDefault();
    setVolume(prefs.volume + (e.deltaY < 0 ? 0.05 : -0.05));
  }

  function side(s: "queue" | "lyrics" | "eq") {
    ui.side = ui.side === s ? null : s;
  }
</script>

<footer class="pb" aria-label={$t("music.player") as string}>
  <div class="pb-now">
    {#if player.radio}
      <button class="pb-cover" onclick={() => go({ kind: "radio" })} aria-label={$t("music.nav.radio") as string}>
        <StationIcon src={player.radio.favicon} name={player.radio.name} size={52} radius={6} />
      </button>
      <div class="pb-meta">
        <span class="pb-title">{player.radioMeta.title || player.radio.name}</span>
        <span class="pb-sub"><span class="live">{$t("music.radio.live")}</span> {player.error.startsWith("radio_failed") ? $t("music.radio.failed") : player.radioMeta.artist || player.radio.name}</span>
      </div>
    {:else if cur}
      <button class="pb-cover" onclick={() => go({ kind: "nowplaying" })} aria-label={$t("music.now_playing") as string}>
        <Cover hash={cur.cover} seed={cur.album || cur.title} size={52} />
      </button>
      <div class="pb-meta">
        <button class="pb-title link" onclick={() => go({ kind: "nowplaying" })}>{cur.title}</button>
        <button class="pb-sub link" onclick={() => cur && cur.artist && go({ kind: "artist", name: cur.artist })}>{cur.artist || $t("music.unknown_artist")}</button>
      </div>
      <button class="ib like" class:on={cur.liked} onclick={like} aria-label={$t(cur.liked ? "music.unlike" : "music.like") as string}>
        <MusicIcon name={cur.liked ? "heart-fill" : "heart"} size={17} />
      </button>
    {:else}
      <div class="pb-meta idle">{$t("music.nothing_playing")}</div>
    {/if}
  </div>

  <div class="pb-center">
    <div class="pb-transport">
      <button class="ib" class:on={prefs.shuffle} onclick={toggleShuffle} aria-pressed={prefs.shuffle} aria-label={$t("music.shuffle") as string} title={$t("music.shuffle") as string} disabled={!!player.radio}>
        <MusicIcon name="shuffle" size={17} />
      </button>
      <button class="ib" onclick={() => void prev()} aria-label={$t("music.previous") as string} disabled={!!player.radio}><MusicIcon name="prev" size={18} /></button>
      <button class="play" onclick={() => void toggle()} aria-label={$t(player.playing ? "music.pause" : "music.play") as string} disabled={!cur && !player.radio && !player.queue.length}>
        {#if player.buffering && player.playing}
          <MusicIcon name="loader" size={20} spin />
        {:else}
          <MusicIcon name={player.playing ? "pause" : "play"} size={20} />
        {/if}
      </button>
      <button class="ib" onclick={() => void next()} aria-label={$t("music.next") as string} disabled={!!player.radio}><MusicIcon name="next" size={18} /></button>
      <button class="ib" class:on={prefs.repeat !== "off"} onclick={cycleRepeat} aria-label={$t(`music.repeat_${prefs.repeat}`) as string} title={$t(`music.repeat_${prefs.repeat}`) as string} disabled={!!player.radio}>
        <MusicIcon name={prefs.repeat === "one" ? "repeat-one" : "repeat"} size={17} />
      </button>
    </div>
    <div class="pb-seek">
      {#if player.radio}
        <div class="pb-radioviz"><Visualizer mode="bars" bars={48} /></div>
      {:else}
        <span class="tm">{fmtTime(player.position * 1000)}</span>
        <div class="wfwrap">
          <Waveform {peaks} position={player.position} duration={player.duration} onseek={(s) => void seek(s)} height={32} label={$t("music.seek") as string} {markers} />
        </div>
        <span class="tm">{fmtTime(player.duration * 1000)}</span>
      {/if}
    </div>
  </div>

  <div class="pb-right">
    {#if cur && !player.radio}
      <span class="q" title={qualityLabel(cur)}>{qualityLabel(cur)}</span>
    {/if}
    {#if player.sleepAt || player.sleepEndOfTrack}
      <span class="sleep" title={$t("music.sleep.active") as string}><MusicIcon name="moon" size={14} /></span>
    {/if}
    <button class="ib" class:on={ui.side === "lyrics"} onclick={() => side("lyrics")} aria-label={$t("music.lyrics.title") as string} title={$t("music.lyrics.title") as string}><MusicIcon name="lyrics" size={17} /></button>
    <button class="ib" class:on={ui.side === "queue"} onclick={() => side("queue")} aria-label={$t("music.queue.title") as string} title={$t("music.queue.title") as string}><MusicIcon name="queue" size={17} /></button>
    <button class="ib" class:on={ui.side === "eq"} onclick={() => side("eq")} aria-label={$t("music.eq.title") as string} title={$t("music.eq.title") as string}><MusicIcon name="eq" size={17} /></button>
    <div class="vol" onwheel={wheelVolume}>
      <button class="ib" onclick={toggleMute} aria-label={$t(prefs.muted ? "music.unmute" : "music.mute") as string}><MusicIcon name={volIcon} size={17} /></button>
      <input
        type="range"
        min="0"
        max="1"
        step="0.01"
        value={prefs.muted ? 0 : prefs.volume}
        oninput={(e) => setVolume(Number((e.currentTarget as HTMLInputElement).value))}
        aria-label={$t("music.volume") as string}
        style:--fill="{(prefs.muted ? 0 : prefs.volume) * 100}%"
      />
    </div>
  </div>
</footer>

<style>
  .pb {
    display: grid;
    grid-template-columns: minmax(200px, 1fr) minmax(320px, 1.6fr) minmax(200px, 1fr);
    align-items: center;
    gap: var(--space-4);
    height: 84px;
    padding: 0 var(--space-4);
    background: color-mix(in srgb, var(--surface) 88%, transparent);
    backdrop-filter: blur(18px) saturate(1.4);
    border-top: var(--hairline) solid var(--content-border);
  }
  .pb-now {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    min-width: 0;
  }
  .pb-cover {
    all: unset;
    cursor: pointer;
    border-radius: 6px;
  }
  .pb-cover:focus-visible {
    outline: var(--focus-ring);
  }
  .pb-meta {
    display: flex;
    flex-direction: column;
    min-width: 0;
    gap: 2px;
  }
  .pb-meta.idle {
    color: var(--text-muted);
    font-size: var(--text-sm);
  }
  .pb-title,
  .pb-sub {
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    text-align: left;
  }
  .pb-title {
    font-weight: 600;
    font-size: var(--text-md);
    color: var(--text);
  }
  .pb-sub {
    font-size: var(--text-sm);
    color: var(--text-dim);
  }
  .link {
    all: unset;
    cursor: pointer;
    max-width: 100%;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .link.pb-title {
    font-weight: 600;
    font-size: var(--text-md);
    color: var(--text);
  }
  .link.pb-sub {
    font-size: var(--text-sm);
    color: var(--text-dim);
  }
  .link:hover {
    text-decoration: underline;
  }
  .link:focus-visible {
    outline: var(--focus-ring);
  }
  .live {
    display: inline-block;
    padding: 0 5px;
    margin-right: 4px;
    border-radius: 3px;
    background: #e5484d;
    color: white;
    font-size: var(--text-caption);
    font-weight: 700;
    letter-spacing: 0.05em;
    vertical-align: 1px;
  }
  .pb-center {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 4px;
    min-width: 0;
  }
  .pb-transport {
    display: flex;
    align-items: center;
    gap: var(--space-2);
  }
  .ib {
    all: unset;
    display: grid;
    place-items: center;
    width: 32px;
    height: 32px;
    border-radius: 50%;
    color: var(--text-dim);
    cursor: pointer;
    transition: color var(--duration-fast), background var(--duration-fast);
  }
  .ib:hover {
    color: var(--text);
    background: var(--fill-1);
  }
  .ib.on {
    color: var(--accent-hi);
  }
  .ib:disabled {
    opacity: 0.35;
    cursor: default;
  }
  .ib:focus-visible,
  .play:focus-visible {
    outline: var(--focus-ring);
  }
  .like.on {
    color: #ff4d6d;
  }
  .play {
    all: unset;
    display: grid;
    place-items: center;
    width: 38px;
    height: 38px;
    border-radius: 50%;
    background: var(--text);
    color: var(--bg);
    cursor: pointer;
    transition: transform var(--duration-fast);
  }
  .play:hover {
    transform: scale(1.06);
  }
  .play:disabled {
    opacity: 0.4;
    cursor: default;
    transform: none;
  }
  .pb-seek {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    width: 100%;
  }
  .wfwrap {
    flex: 1;
    min-width: 0;
  }
  .pb-radioviz {
    width: 100%;
    height: 28px;
  }
  .tm {
    font-size: var(--text-xs);
    color: var(--text-muted);
    font-variant-numeric: tabular-nums;
    min-width: 36px;
    text-align: center;
  }
  .pb-right {
    display: flex;
    align-items: center;
    justify-content: flex-end;
    gap: 2px;
    min-width: 0;
  }
  .q {
    margin-right: var(--space-2);
    font-size: var(--text-xs);
    color: var(--text-muted);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .sleep {
    color: var(--accent-hi);
    display: grid;
    place-items: center;
    margin-right: 4px;
  }
  .vol {
    display: flex;
    align-items: center;
    gap: 2px;
  }
  .vol input {
    width: 96px;
    appearance: none;
    height: 4px;
    border-radius: 2px;
    background: linear-gradient(to right, var(--text) var(--fill), var(--fill-3) var(--fill));
    cursor: pointer;
  }
  .vol input::-webkit-slider-thumb {
    appearance: none;
    width: 12px;
    height: 12px;
    border-radius: 50%;
    background: var(--text);
    opacity: 0;
    transition: opacity var(--duration-fast);
  }
  .vol:hover input::-webkit-slider-thumb,
  .vol input:focus-visible::-webkit-slider-thumb {
    opacity: 1;
  }
  @media (max-width: 900px) {
    .pb {
      grid-template-columns: minmax(140px, 1fr) minmax(260px, 2fr) auto;
    }
    .q,
    .vol input {
      display: none;
    }
  }
</style>

<script lang="ts">
  import { t } from "$lib/i18n";
  import { albumKey, camelotCompatible, qualityLabel } from "$lib/music/library";
  import { library } from "$lib/music/library-store.svelte";
  import { enqueue, player, prefs } from "$lib/music/player-store.svelte";
  import { analyze, go } from "$lib/music/ui-store.svelte";
  import Cover from "../Cover.svelte";
  import EmptyState from "../EmptyState.svelte";
  import KaraokeBar from "../KaraokeBar.svelte";
  import LyricsPanel from "../LyricsPanel.svelte";
  import MusicIcon from "../MusicIcon.svelte";
  import Visualizer from "../Visualizer.svelte";

  let cur = $derived(player.current);
  let lib = $derived(library());

  let harmonic = $derived.by(() => {
    void lib.version;
    const c = cur;
    if (!c || !c.camelot || !c.bpm) return [];
    return lib.tracks
      .filter((x) => x.id !== c.id && x.camelot && x.bpm && camelotCompatible(c.camelot, x.camelot))
      .map((x) => {
        const ratio = [x.bpm! / c.bpm!, (x.bpm! * 2) / c.bpm!, x.bpm! / (c.bpm! * 2)].map((r) => Math.abs(1 - r));
        return { x, d: Math.min(...ratio) };
      })
      .filter((r) => r.d <= 0.06)
      .sort((a, b) => a.d - b.d)
      .slice(0, 8)
      .map((r) => r.x);
  });

  let facts = $derived(
    cur
      ? [
          cur.camelot ? `${cur.camelot} · ${cur.musical_key}` : cur.musical_key,
          cur.bpm ? `${Math.round(cur.bpm)} BPM` : "",
          cur.loudness_lufs != null ? `${cur.loudness_lufs.toFixed(1)} LUFS` : "",
          cur.rg_track_gain != null && prefs.rgMode !== "off" ? `${cur.rg_track_gain > 0 ? "+" : ""}${cur.rg_track_gain.toFixed(1)} dB` : "",
          qualityLabel(cur),
          cur.play_count ? $t("music.plays", { count: cur.play_count }) : "",
        ].filter(Boolean)
      : [],
  );
</script>

{#if !cur && !player.radio}
  <EmptyState art="now-playing" title={$t("music.np.idle_title") as string} body={$t("music.np.idle_body") as string}>
    <button class="btn btn-primary" onclick={() => go({ kind: "songs" })}>{$t("music.nav.songs")}</button>
    <button class="btn" onclick={() => go({ kind: "radio" })}>{$t("music.nav.radio")}</button>
  </EmptyState>
{:else}
  <div class="np">
    <div class="stage">
      <div class="ring">
        <Visualizer mode={prefs.visualizer === "off" ? "off" : "circle"} bars={96} />
      </div>
      <div class="disc" class:spin={player.playing}>
        {#if player.radio}
          <Cover hash={null} seed={player.radio.name} size="100%" round icon="radio" />
        {:else if cur}
          <Cover hash={cur.cover} seed={cur.album || cur.title} size="100%" round />
        {/if}
      </div>
    </div>

    <div class="meta">
      {#if player.radio}
        <span class="kick">{$t("music.radio.live")} · {player.radio.name}</span>
        <h1>{player.radioMeta.title || player.radio.name}</h1>
        {#if player.radioMeta.artist}<p class="by">{player.radioMeta.artist}</p>{/if}
      {:else if cur}
        {#if player.context}<span class="kick">{player.context}</span>{/if}
        <h1>{cur.title}</h1>
        <p class="by">
          <button class="lnk" onclick={() => cur && go({ kind: "artist", name: cur.artist })}>{cur.artist || $t("music.unknown_artist")}</button>
          {#if cur.album}· <button class="lnk" onclick={() => cur && go({ kind: "album", key: albumKey(cur) })}>{cur.album}</button>{/if}
          {#if cur.year}· {cur.year}{/if}
        </p>
        <div class="facts">
          {#each facts as f (f)}<span class="tag">{f}</span>{/each}
          {#if !cur.analyzed}
            <button class="btn btn-sm btn-ghost" onclick={() => cur && analyze([cur.id])}><MusicIcon name="activity" size={12} /> {$t("music.menu.analyze")}</button>
          {/if}
        </div>
        <KaraokeBar />
      {/if}
    </div>

    <div class="lyrics"><LyricsPanel big /></div>

    {#if harmonic.length}
      <section class="mix">
        <h2><MusicIcon name="mix" size={15} /> {$t("music.np.harmonic")}</h2>
        <p class="hint">{$t("music.np.harmonic_hint")}</p>
        <div class="hl">
          {#each harmonic as x (x.id)}
            <button class="hx" onclick={() => enqueue([x.id], true)} title={$t("music.menu.play_next") as string}>
              <Cover hash={x.cover} seed={x.album || x.title} size={36} radius={4} />
              <span class="ht"><b>{x.title}</b><small>{x.artist} · {x.camelot} · {Math.round(x.bpm ?? 0)}</small></span>
              <MusicIcon name="plus" size={14} />
            </button>
          {/each}
        </div>
      </section>
    {/if}
  </div>
{/if}

<style>
  .np {
    display: grid;
    grid-template-columns: minmax(260px, 420px) minmax(280px, 1fr);
    grid-template-areas:
      "stage lyrics"
      "meta lyrics"
      "mix lyrics";
    grid-template-rows: auto auto 1fr;
    gap: var(--space-4) var(--space-6);
    height: 100%;
    min-height: 0;
    padding-bottom: var(--space-4);
  }
  .stage {
    grid-area: stage;
    position: relative;
    aspect-ratio: 1;
    width: 100%;
    max-width: 420px;
  }
  .ring {
    position: absolute;
    inset: 0;
  }
  .disc {
    position: absolute;
    inset: 21%;
    border-radius: 50%;
    overflow: hidden;
    box-shadow:
      0 0 0 6px #111,
      0 20px 50px rgba(0, 0, 0, 0.45);
  }
  .disc :global(.cover) {
    width: 100% !important;
    height: 100% !important;
    object-fit: cover;
  }
  .disc::after {
    content: "";
    position: absolute;
    inset: 44%;
    border-radius: 50%;
    background: var(--bg);
    box-shadow: inset 0 0 0 3px #111;
  }
  .disc.spin {
    animation: rot 12s linear infinite;
  }
  @keyframes rot {
    to {
      transform: rotate(360deg);
    }
  }
  .meta {
    grid-area: meta;
    display: flex;
    flex-direction: column;
    gap: 4px;
    min-width: 0;
  }
  .kick {
    font-size: var(--text-xs);
    text-transform: uppercase;
    letter-spacing: 0.06em;
    color: var(--text-muted);
  }
  h1 {
    margin: 0;
    font-family: var(--font-display);
    font-size: var(--text-3xl);
    line-height: 1.1;
    font-weight: 750;
  }
  .by {
    margin: 0;
    color: var(--text-dim);
  }
  .lnk {
    all: unset;
    cursor: pointer;
  }
  .lnk:hover {
    text-decoration: underline;
    color: var(--text);
  }
  .lnk:focus-visible {
    outline: var(--focus-ring);
  }
  .facts {
    display: flex;
    flex-wrap: wrap;
    gap: 4px;
    margin-top: var(--space-2);
  }
  .lyrics {
    grid-area: lyrics;
    min-height: 0;
    border-radius: var(--radius-xl);
    background: var(--fill-1);
    overflow: hidden;
  }
  .mix {
    grid-area: mix;
    min-height: 0;
  }
  h2 {
    display: flex;
    align-items: center;
    gap: 6px;
    margin: 0;
    font-size: var(--text-md);
  }
  .hint {
    margin: 2px 0 var(--space-2);
    font-size: var(--text-xs);
    color: var(--text-dim);
  }
  .hl {
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .hx {
    all: unset;
    display: flex;
    align-items: center;
    gap: var(--space-2);
    padding: 4px 6px;
    border-radius: var(--radius-sm);
    cursor: pointer;
    color: var(--text-muted);
  }
  .hx:hover {
    background: var(--fill-1);
    color: var(--text);
  }
  .hx:focus-visible {
    outline: var(--focus-ring);
  }
  .ht {
    display: flex;
    flex-direction: column;
    flex: 1;
    min-width: 0;
  }
  .ht b,
  .ht small {
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .ht b {
    font-size: var(--text-sm);
    color: var(--text);
  }
  .ht small {
    font-size: var(--text-xs);
  }
  @media (max-width: 900px) {
    .np {
      grid-template-columns: 1fr;
      grid-template-areas: "stage" "meta" "lyrics" "mix";
      grid-template-rows: auto auto 420px auto;
    }
    .stage {
      max-width: 320px;
      margin: 0 auto;
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .disc.spin {
      animation: none;
    }
  }
</style>

<script lang="ts">
  // Album tile. On hover the record slides out of its sleeve (idea from the
  // "Vinyl Album Card" on 21st.dev, redone in CSS so it costs nothing at rest).
  import { t } from "$lib/i18n";
  import { coverUrl } from "$lib/music/library-store.svelte";
  import type { Album } from "$lib/music/types";
  import Cover from "./Cover.svelte";
  import MusicIcon from "./MusicIcon.svelte";

  let {
    album,
    onopen,
    onplay,
    onmenu,
    playing = false,
  }: {
    album: Album;
    onopen: () => void;
    onplay: () => void;
    onmenu?: (e: MouseEvent) => void;
    playing?: boolean;
  } = $props();

  let label = $derived(coverUrl(album.cover, true));
</script>

<div class="ac" class:playing>
  <div class="sleeve">
    <div class="vinyl" aria-hidden="true">
      <span class="grooves"></span>
      <span class="label" style:background-image={label ? `url("${label}")` : "none"}></span>
    </div>
    <button class="cover-btn" onclick={onopen} oncontextmenu={onmenu} aria-label={album.title}>
      <Cover hash={album.cover} seed={album.title} size="100%" radius={8} icon="disc" />
    </button>
    <button class="play" onclick={onplay} aria-label={$t("music.play_album", { name: album.title }) as string}>
      <MusicIcon name="play" size={18} />
    </button>
  </div>
  <button class="meta" onclick={onopen} oncontextmenu={onmenu}>
    <span class="t1">{album.title}</span>
    <span class="t2">{album.artist || $t("music.unknown_artist")}{album.year ? ` · ${album.year}` : ""}</span>
  </button>
</div>

<style>
  .ac {
    display: flex;
    flex-direction: column;
    gap: 8px;
    min-width: 0;
  }
  .sleeve {
    position: relative;
    aspect-ratio: 1;
    width: 100%;
  }
  .cover-btn {
    all: unset;
    position: absolute;
    inset: 0;
    z-index: 2;
    border-radius: 8px;
    cursor: pointer;
    box-shadow: 0 6px 18px rgba(0, 0, 0, 0.28);
    transition: transform 0.35s var(--ease-out);
  }
  .cover-btn :global(.cover) {
    width: 100% !important;
    height: 100% !important;
  }
  .cover-btn:focus-visible {
    outline: var(--focus-ring);
    outline-offset: 2px;
  }
  .vinyl {
    position: absolute;
    z-index: 1;
    top: 4%;
    left: 4%;
    width: 92%;
    height: 92%;
    border-radius: 50%;
    background: radial-gradient(circle, #1b1b1d 0 30%, #0c0c0d 31% 100%);
    box-shadow: 0 4px 14px rgba(0, 0, 0, 0.35);
    transition: transform 0.5s var(--ease-out);
  }
  .grooves {
    position: absolute;
    inset: 6%;
    border-radius: 50%;
    background: repeating-radial-gradient(circle, rgba(255, 255, 255, 0.05) 0 1px, transparent 1px 4px);
  }
  .label {
    position: absolute;
    inset: 34%;
    border-radius: 50%;
    background: var(--accent) center / cover;
    box-shadow: 0 0 0 2px rgba(0, 0, 0, 0.4);
  }
  .label::after {
    content: "";
    position: absolute;
    inset: 44%;
    border-radius: 50%;
    background: #0c0c0d;
  }
  .ac:hover .vinyl,
  .ac:focus-within .vinyl,
  .ac.playing .vinyl {
    transform: translateX(22%) rotate(120deg);
  }
  .ac:hover .cover-btn,
  .ac:focus-within .cover-btn,
  .ac.playing .cover-btn {
    transform: translateX(-6%);
  }
  .ac.playing .vinyl {
    animation: spin 3.6s linear infinite;
  }
  @keyframes spin {
    from {
      transform: translateX(22%) rotate(0deg);
    }
    to {
      transform: translateX(22%) rotate(360deg);
    }
  }
  .play {
    all: unset;
    position: absolute;
    z-index: 3;
    right: 14%;
    bottom: 8%;
    display: grid;
    place-items: center;
    width: 40px;
    height: 40px;
    border-radius: 50%;
    background: var(--accent);
    color: var(--on-accent);
    box-shadow: 0 6px 16px rgba(0, 0, 0, 0.35);
    cursor: pointer;
    opacity: 0;
    transform: translateY(6px);
    transition:
      opacity 0.2s,
      transform 0.2s;
  }
  .ac:hover .play,
  .play:focus-visible {
    opacity: 1;
    transform: none;
  }
  .play:focus-visible {
    outline: var(--focus-ring);
  }
  .meta {
    all: unset;
    display: flex;
    flex-direction: column;
    min-width: 0;
    cursor: pointer;
  }
  .t1,
  .t2 {
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .t1 {
    font-weight: 600;
    font-size: var(--text-base);
  }
  .t2 {
    font-size: var(--text-sm);
    color: var(--text-dim);
  }
  @media (prefers-reduced-motion: reduce) {
    .vinyl,
    .cover-btn {
      transition: none;
    }
    .ac.playing .vinyl {
      animation: none;
    }
  }
</style>

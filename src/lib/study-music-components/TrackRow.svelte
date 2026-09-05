<script lang="ts">
  import { fmtDuration, trackDisplayTitle } from "$lib/study-music/format";
  import { musicPlayer, type MusicTrack } from "$lib/study-music/player-store.svelte";
  import { musicUI } from "$lib/study-music/ui-store.svelte";
  import { t } from "$lib/i18n";
  import CoverImage from "./CoverImage.svelte";

  type Props = {
    track: MusicTrack;
    queue?: MusicTrack[];
    index?: number;
    showCover?: boolean;
    showAlbum?: boolean;
    showNumber?: boolean;
    showRemove?: boolean;
    onRemove?: (track: MusicTrack) => void;
    onContextMenu?: (e: MouseEvent, track: MusicTrack) => void;
  };

  let {
    track,
    queue,
    index = 0,
    showCover = true,
    showAlbum = false,
    showNumber = false,
    showRemove = false,
    onRemove,
    onContextMenu,
  }: Props = $props();

  const isCurrent = $derived(musicPlayer.currentTrack?.id === track.id);
  const isPlaying = $derived(isCurrent && !musicPlayer.paused);
  const isSelected = $derived(musicUI.isSelected(track.id));
  const hasSelection = $derived(musicUI.selectedCount() > 0);

  function handleClick(e: MouseEvent) {
    if (e.shiftKey && queue) {
      e.preventDefault();
      musicUI.selectRange(track.id, queue);
      return;
    }
    if (e.ctrlKey || e.metaKey) {
      e.preventDefault();
      musicUI.toggleSelected(track.id, queue);
      return;
    }
    if (hasSelection) {
      musicUI.clearSelection();
    }
    void musicPlayer.play(track, queue);
  }

  function handleContextMenu(e: MouseEvent) {
    e.preventDefault();
    if (onContextMenu) {
      onContextMenu(e, track);
      return;
    }
    if (!isSelected && hasSelection) {
      musicUI.selectOnly(track.id, queue);
    } else if (!hasSelection && queue) {
      musicUI.selectionAnchor = { trackId: track.id, queue };
    }
    musicUI.openContextMenu(track, e.clientX, e.clientY);
  }

  async function favoriteToggle(e: MouseEvent) {
    e.stopPropagation();
    await musicPlayer.toggleFavorite(track.id);
    track.favorite = !track.favorite;
  }
</script>

<div
  class="track-row"
  class:current={isCurrent}
  class:selected={isSelected}
  role="button"
  tabindex="0"
  onclick={handleClick}
  onkeydown={(e) => {
    if (e.key === "Enter" || e.key === " ") {
      e.preventDefault();
      void musicPlayer.play(track, queue);
    }
  }}
  oncontextmenu={handleContextMenu}
>
  {#if showNumber}
    <span class="track-num">
      {#if isPlaying}
        <span class="playing-dots" aria-hidden="true">
          <i></i><i></i><i></i>
        </span>
      {:else}
        <span class="num">{index + 1}</span>
        <span class="play-icon" aria-hidden="true">
          <svg viewBox="0 0 24 24" width="12" height="12" fill="currentColor"><path d="M8 5v14l11-7z"/></svg>
        </span>
      {/if}
    </span>
  {/if}
  {#if showCover}
    <CoverImage
      src={track.cover_path}
      alt={track.title ?? track.path}
      size={36}
      fallbackSeed={track.album ?? track.title}
      rounded="sm"
      trackId={track.id}
    />
  {/if}
  <span class="track-info">
    <span class="track-title">{trackDisplayTitle(track)}</span>
    {#if track.artist}
      <span class="track-artist">{track.artist}</span>
    {/if}
  </span>
  {#if showAlbum && track.album}
    <span class="track-album">{track.album}</span>
  {/if}
  <button
    type="button"
    class="fav-btn"
    class:on={track.favorite}
    onclick={favoriteToggle}
    aria-label={track.favorite ? ($t("study.music.unfavorite") as string) : ($t("study.music.favorite") as string)}
  >
    <svg viewBox="0 0 24 24" width="14" height="14" fill={track.favorite ? "currentColor" : "none"} stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
      <path d="M20.84 4.61a5.5 5.5 0 0 0-7.78 0L12 5.67l-1.06-1.06a5.5 5.5 0 0 0-7.78 7.78l1.06 1.06L12 21.23l7.78-7.78 1.06-1.06a5.5 5.5 0 0 0 0-7.78z"/>
    </svg>
  </button>
  <button
    type="button"
    class="more-btn"
    onclick={(e) => {
      e.stopPropagation();
      musicUI.openAddToPlaylist(track);
    }}
    aria-label={$t("study.music.add_to_playlist") as string}
    title={$t("study.music.add_to_playlist") as string}
  >
    <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
      <line x1="12" y1="5" x2="12" y2="19"/>
      <line x1="5" y1="12" x2="19" y2="12"/>
    </svg>
  </button>
  {#if showRemove}
    <button
      type="button"
      class="remove-btn"
      onclick={(e) => {
        e.stopPropagation();
        onRemove?.(track);
      }}
      aria-label={$t("study.music.remove_from_playlist") as string}
      title={$t("study.music.remove_from_playlist") as string}
    >
      <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
        <line x1="18" y1="6" x2="6" y2="18"/>
        <line x1="6" y1="6" x2="18" y2="18"/>
      </svg>
    </button>
  {/if}
  <span class="track-dur">{fmtDuration(track.duration_ms)}</span>
</div>

<style>
  .track-row {
    display: flex;
    align-items: center;
    gap: 12px;
    width: 100%;
    padding: 6px 12px;
    background: transparent;
    border-radius: 6px;
    color: rgba(255, 255, 255, 0.95);
    font-family: inherit;
    text-align: left;
    cursor: pointer;
    transition: background 120ms ease;
    user-select: none;
    min-height: 48px;
  }
  .track-row:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: 1px;
  }
  .track-row:hover {
    background: rgba(255, 255, 255, 0.05);
  }
  .track-row.current {
    background: color-mix(in oklab, var(--accent) 14%, transparent);
  }
  .track-row.current .track-title {
    color: var(--accent);
  }
  .track-row.selected {
    background: color-mix(in oklab, var(--accent) 22%, transparent);
    outline: 1px solid color-mix(in oklab, var(--accent) 50%, transparent);
  }
  .track-num {
    position: relative;
    flex-shrink: 0;
    width: 28px;
    text-align: center;
    color: rgba(255, 255, 255, 0.5);
    font-size: 12px;
    font-variant-numeric: tabular-nums;
  }
  .track-num .play-icon {
    position: absolute;
    inset: 0;
    display: grid;
    place-items: center;
    color: rgba(255, 255, 255, 0.95);
    opacity: 0;
    transition: opacity 120ms ease;
  }
  .track-row:hover .track-num .num {
    opacity: 0;
  }
  .track-row:hover .track-num .play-icon {
    opacity: 1;
  }
  .track-row :global(.cover) {
    flex-shrink: 0;
  }
  .track-info {
    flex: 1 1 auto;
    display: flex;
    flex-direction: column;
    min-width: 0;
    overflow: hidden;
    gap: 2px;
  }
  .track-title {
    font-size: 13px;
    font-weight: 600;
    color: rgba(255, 255, 255, 0.95);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    line-height: 1.3;
  }
  .track-artist {
    font-size: 11px;
    color: rgba(255, 255, 255, 0.5);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    line-height: 1.3;
  }
  .track-album {
    flex: 0 1 auto;
    width: clamp(120px, 24vw, 240px);
    font-size: 12px;
    color: rgba(255, 255, 255, 0.5);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .fav-btn {
    flex-shrink: 0;
    width: 28px;
    height: 28px;
    padding: 0;
    background: transparent;
    border: 0;
    border-radius: 50%;
    color: rgba(255, 255, 255, 0.5);
    cursor: pointer;
    display: grid;
    place-items: center;
    opacity: 0;
    transition: color 120ms ease, background 120ms ease, opacity 120ms ease;
  }
  .track-row:hover .fav-btn {
    opacity: 1;
  }
  .fav-btn.on {
    color: var(--accent);
    opacity: 1;
  }
  .fav-btn:hover {
    background: rgba(255, 255, 255, 0.1);
    color: var(--accent);
  }
  .more-btn,
  .remove-btn {
    flex-shrink: 0;
    width: 28px;
    height: 28px;
    padding: 0;
    background: transparent;
    border: 0;
    border-radius: 50%;
    color: rgba(255, 255, 255, 0.5);
    cursor: pointer;
    display: grid;
    place-items: center;
    opacity: 0;
    transition: color 120ms ease, background 120ms ease, opacity 120ms ease;
  }
  .track-row:hover .more-btn,
  .track-row:hover .remove-btn {
    opacity: 1;
  }
  .more-btn:hover {
    color: var(--accent);
    background: rgba(255, 255, 255, 0.1);
  }
  .remove-btn:hover {
    color: var(--error);
    background: color-mix(in oklab, var(--error) 12%, transparent);
  }
  .track-dur {
    flex-shrink: 0;
    font-size: 12px;
    color: rgba(255, 255, 255, 0.5);
    font-variant-numeric: tabular-nums;
    min-width: 48px;
    text-align: right;
  }
  .playing-dots {
    display: inline-flex;
    align-items: flex-end;
    gap: 2px;
    height: 14px;
  }
  .playing-dots i {
    display: block;
    width: 3px;
    height: 100%;
    background: var(--accent);
    border-radius: 1px;
    animation: bars 0.9s ease-in-out infinite;
  }
  .playing-dots i:nth-child(2) { animation-delay: 0.15s; }
  .playing-dots i:nth-child(3) { animation-delay: 0.3s; }
  @keyframes bars {
    0%, 100% { transform: scaleY(0.35); }
    50% { transform: scaleY(1); }
  }
  @media (prefers-reduced-motion: reduce) {
    .playing-dots i { animation: none; transform: scaleY(0.7); }
  }
</style>

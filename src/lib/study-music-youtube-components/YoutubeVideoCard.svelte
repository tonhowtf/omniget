<script lang="ts">
  import { goto } from "$app/navigation";
  import type { YoutubeSearchVideoItem } from "$lib/study-bridge";

  type Props = {
    item: YoutubeSearchVideoItem;
    onPlay?: (videoId: string) => void;
  };
  let { item, onPlay }: Props = $props();

  function play() {
    if (onPlay) onPlay(item.video_id);
  }

  function openChannel(e: MouseEvent) {
    e.stopPropagation();
    if (item.channel_id) goto(`/study/music/youtube/channel/${item.channel_id}`);
  }
</script>

<div class="card">
  <button type="button" class="thumb-wrap" onclick={play} aria-label={item.title}>
    {#if item.thumbnail_url}
      <img src={item.thumbnail_url} alt="" loading="lazy" />
    {:else}
      <div class="thumb-placeholder"></div>
    {/if}
    {#if item.duration_text}
      <span class="duration tabular-nums">{item.duration_text}</span>
    {/if}
    {#if item.is_live}
      <span class="live">AO VIVO</span>
    {/if}
  </button>
  <div class="meta">
    <button type="button" class="title-btn" onclick={play} title={item.title}>
      <span class="title">{item.title}</span>
    </button>
    {#if item.channel_title}
      <button class="channel" onclick={openChannel} type="button" title={item.channel_title}>
        {item.channel_title}
      </button>
    {/if}
    <p class="stats">
      {#if item.view_count_text}<span>{item.view_count_text}</span>{/if}
      {#if item.view_count_text && item.published_time_text}<span> · </span>{/if}
      {#if item.published_time_text}<span>{item.published_time_text}</span>{/if}
    </p>
  </div>
</div>

<style>
  .card {
    display: flex;
    flex-direction: column;
    gap: 8px;
    padding: 0;
    color: var(--secondary);
  }
  .thumb-wrap {
    position: relative;
    aspect-ratio: 16 / 9;
    border-radius: 8px;
    overflow: hidden;
    background: color-mix(in oklab, var(--button) 60%, transparent);
    padding: 0;
    border: 0;
    cursor: pointer;
    display: block;
    width: 100%;
  }
  .thumb-wrap:hover { transform: translateY(-2px); transition: transform 120ms; }
  .title-btn {
    padding: 0;
    background: transparent;
    border: 0;
    text-align: left;
    cursor: pointer;
    color: inherit;
    width: 100%;
  }
  .title-btn:hover .title { color: var(--accent); }
  .thumb-wrap img {
    width: 100%;
    height: 100%;
    object-fit: cover;
    display: block;
  }
  .thumb-placeholder {
    width: 100%;
    height: 100%;
    background: color-mix(in oklab, var(--button) 50%, transparent);
  }
  .duration {
    position: absolute;
    right: 6px;
    bottom: 6px;
    padding: 2px 6px;
    border-radius: 4px;
    background: rgba(0, 0, 0, 0.75);
    color: #fff;
    font-size: 11px;
    font-weight: 500;
  }
  .live {
    position: absolute;
    left: 6px;
    top: 6px;
    padding: 2px 6px;
    border-radius: 4px;
    background: #d33;
    color: #fff;
    font-size: 10px;
    font-weight: 700;
    letter-spacing: 0.04em;
  }
  .meta { display: flex; flex-direction: column; gap: 2px; min-width: 0; }
  .title {
    margin: 0;
    font-size: 13px;
    font-weight: 600;
    color: var(--secondary);
    line-height: 1.3;
    display: -webkit-box;
    -webkit-line-clamp: 2;
    -webkit-box-orient: vertical;
    overflow: hidden;
  }
  .channel {
    margin: 0;
    padding: 0;
    background: transparent;
    border: 0;
    color: var(--tertiary);
    font-size: 12px;
    cursor: pointer;
    text-align: left;
    align-self: flex-start;
    max-width: 100%;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .channel:hover { color: var(--accent); }
  .stats {
    margin: 0;
    font-size: 11px;
    color: var(--tertiary);
  }
</style>

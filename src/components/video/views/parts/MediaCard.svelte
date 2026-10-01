<script lang="ts">
  import { t } from "$lib/i18n";
  import Thumb from "./Thumb.svelte";
  import VIcon from "./VIcon.svelte";

  let {
    title,
    subtitle = "",
    src,
    seed = "",
    ratio = "16 / 9",
    icon = "film",
    progress = null,
    watched = false,
    badge = "",
    offline = false,
    onopen,
    onplay,
    onmenu,
    onremove,
    removeLabel = "",
  }: {
    title: string;
    subtitle?: string;
    src: string | null | undefined;
    seed?: string;
    ratio?: string;
    icon?: string;
    progress?: number | null;
    watched?: boolean;
    badge?: string;
    offline?: boolean;
    onopen: () => void;
    onplay?: () => void;
    onmenu?: (e: MouseEvent) => void;
    onremove?: () => void;
    removeLabel?: string;
  } = $props();
</script>

<div class="mc" class:offline>
  <div class="art">
    <button class="open" onclick={onopen} oncontextmenu={onmenu} aria-label={title}>
      <Thumb {src} seed={seed || title} {ratio} {icon} />
    </button>
    {#if progress != null && progress > 0}
      <span class="bar" aria-hidden="true"><span style:width="{Math.max(3, progress * 100)}%"></span></span>
    {/if}
    {#if watched}
      <span class="seen" title={$t("video.common.watched") as string}><VIcon name="check" size={12} /></span>
    {/if}
    {#if badge}<span class="badge-c">{badge}</span>{/if}
    {#if offline}<span class="badge-c off">{$t("video.common.offline")}</span>{/if}
    {#if onplay}
      <button class="play" onclick={onplay} aria-label={$t("video.common.play_title", { title }) as string}><VIcon name="play" size={16} /></button>
    {/if}
    {#if onremove}
      <button class="rm" onclick={onremove} aria-label={removeLabel || ($t("video.common.remove") as string)} title={removeLabel || ($t("video.common.remove") as string)}>
        <VIcon name="close" size={12} />
      </button>
    {/if}
  </div>
  <button class="meta" onclick={onopen} oncontextmenu={onmenu}>
    <span class="t1">{title}</span>
    {#if subtitle}<span class="t2">{subtitle}</span>{/if}
  </button>
</div>

<style>
  .mc {
    display: flex;
    flex-direction: column;
    gap: 6px;
    min-width: 0;
  }
  .mc.offline {
    opacity: 0.55;
  }
  .art {
    position: relative;
    border-radius: var(--radius-md);
    transition: transform var(--duration-base) var(--ease-out);
  }
  .mc:hover .art {
    transform: translateY(-2px);
  }
  .open {
    all: unset;
    display: block;
    width: 100%;
    border-radius: var(--radius-md);
    cursor: pointer;
    box-shadow: var(--elev-1);
    transition: box-shadow var(--duration-base) var(--ease-out);
  }
  .mc:hover .open {
    box-shadow: var(--elev-2);
  }
  .open:focus-visible {
    outline: var(--focus-ring);
    outline-offset: 2px;
  }
  .bar {
    position: absolute;
    left: 8px;
    right: 8px;
    bottom: 8px;
    height: 4px;
    border-radius: var(--radius-full);
    background: rgba(0, 0, 0, 0.45);
    overflow: hidden;
    pointer-events: none;
  }
  .bar span {
    display: block;
    height: 100%;
    background: var(--accent);
    border-radius: inherit;
  }
  .seen {
    position: absolute;
    top: 6px;
    right: 6px;
    display: grid;
    place-items: center;
    width: 20px;
    height: 20px;
    border-radius: 50%;
    background: var(--success);
    color: var(--on-success);
    pointer-events: none;
  }
  .badge-c {
    position: absolute;
    top: 6px;
    left: 6px;
    padding: 1px 6px;
    border-radius: var(--radius-xs);
    background: rgba(0, 0, 0, 0.6);
    color: white;
    font-size: var(--text-xs);
    font-weight: 600;
    pointer-events: none;
  }
  .badge-c.off {
    top: auto;
    bottom: 6px;
  }
  .play,
  .rm {
    all: unset;
    position: absolute;
    display: grid;
    place-items: center;
    border-radius: 50%;
    cursor: pointer;
    opacity: 0;
    transition:
      opacity var(--duration-fast),
      transform var(--duration-fast);
  }
  .play {
    right: 10px;
    bottom: 14px;
    width: 36px;
    height: 36px;
    background: var(--accent);
    color: var(--on-accent);
    box-shadow: var(--elev-2);
    transform: translateY(4px);
  }
  .rm {
    top: 6px;
    right: 6px;
    width: 22px;
    height: 22px;
    background: rgba(0, 0, 0, 0.6);
    color: white;
  }
  .mc:hover .play,
  .mc:hover .rm,
  .mc:focus-within .play,
  .mc:focus-within .rm {
    opacity: 1;
    transform: none;
  }
  .play:focus-visible,
  .rm:focus-visible {
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
  @media (hover: none) {
    .play,
    .rm {
      opacity: 1;
      transform: none;
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .art,
    .play {
      transition: none;
      transform: none;
    }
  }
</style>

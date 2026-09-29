<script lang="ts">
  import { goto } from "$app/navigation";
  import { t } from "$lib/i18n";
  import { next, player, prev, stop, toggle } from "$lib/music/player-store.svelte";
  import { go } from "$lib/music/ui-store.svelte";
  import { shellLayout } from "$lib/stores/shell-layout.svelte";
  import Cover from "./Cover.svelte";
  import StationIcon from "./StationIcon.svelte";
  import MusicIcon from "./MusicIcon.svelte";

  let pct = $derived(player.duration > 0 ? Math.min(100, (player.position / player.duration) * 100) : 0);

  function open() {
    go({ kind: "nowplaying" });
    void goto("/music");
  }
</script>

<div
  class="mini"
  role="region"
  aria-label={$t("music.player") as string}
  style:bottom={`calc(16px + ${shellLayout.overlayBottomInset}px)`}
>
  <button class="art" onclick={open} aria-label={$t("music.now_playing") as string}>
    {#if player.radio}
      <StationIcon src={player.radio.favicon} name={player.radio.name} size={40} radius={6} />
    {:else}
      <Cover hash={player.current?.cover} seed={player.current?.album || player.current?.title} size={40} radius={6} />
    {/if}
  </button>
  <button class="meta" onclick={open}>
    <span class="t1">{player.radio ? player.radioMeta.title || player.radio.name : player.current?.title}</span>
    <span class="t2">{player.radio ? player.radio.name : player.current?.artist}</span>
  </button>
  {#if !player.radio}
    <button class="ib" onclick={() => void prev()} aria-label={$t("music.previous") as string}><MusicIcon name="prev" size={15} /></button>
  {/if}
  <button class="ib main" onclick={() => void toggle()} aria-label={$t(player.playing ? "music.pause" : "music.play") as string}>
    <MusicIcon name={player.playing ? "pause" : "play"} size={16} />
  </button>
  {#if !player.radio}
    <button class="ib" onclick={() => void next()} aria-label={$t("music.next") as string}><MusicIcon name="next" size={15} /></button>
  {/if}
  <button class="ib x" onclick={stop} aria-label={$t("music.stop") as string}><MusicIcon name="close" size={13} /></button>
  {#if !player.radio}<span class="prog" style:width="{pct}%"></span>{/if}
</div>

<style>
  .mini {
    position: fixed;
    right: 16px;
    z-index: 300;
    display: flex;
    align-items: center;
    gap: 6px;
    width: 340px;
    max-width: calc(100vw - 32px);
    padding: 6px 8px 8px 6px;
    border-radius: 14px;
    overflow: hidden;
    background: color-mix(in srgb, var(--surface-hi) 92%, transparent);
    backdrop-filter: blur(20px) saturate(1.5);
    box-shadow:
      0 0 0 var(--hairline) var(--content-border),
      0 14px 36px rgba(0, 0, 0, 0.35);
    animation: pop 0.25s var(--ease-out);
  }
  /* movement only: a frozen animation (hidden window) must not leave it invisible */
  @keyframes pop {
    from {
      transform: translateY(10px);
    }
  }
  .art {
    all: unset;
    cursor: pointer;
    border-radius: 6px;
    flex-shrink: 0;
  }
  .meta {
    all: unset;
    display: flex;
    flex-direction: column;
    flex: 1;
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
    font-size: var(--text-sm);
    font-weight: 600;
  }
  .t2 {
    font-size: var(--text-xs);
    color: var(--text-dim);
  }
  .ib {
    all: unset;
    display: grid;
    place-items: center;
    width: 28px;
    height: 28px;
    border-radius: 50%;
    color: var(--text-dim);
    cursor: pointer;
    flex-shrink: 0;
  }
  .ib:hover {
    background: var(--fill-2);
    color: var(--text);
  }
  .ib.main {
    width: 32px;
    height: 32px;
    background: var(--text);
    color: var(--bg);
  }
  .ib.x {
    width: 22px;
    height: 22px;
  }
  .art:focus-visible,
  .meta:focus-visible,
  .ib:focus-visible {
    outline: var(--focus-ring);
  }
  .prog {
    position: absolute;
    left: 0;
    bottom: 0;
    height: 2px;
    background: var(--accent);
    transition: width 0.25s linear;
  }
  @media (prefers-reduced-motion: reduce) {
    .mini {
      animation: none;
    }
  }
</style>

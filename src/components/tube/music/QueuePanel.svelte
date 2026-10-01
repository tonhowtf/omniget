<script lang="ts">
  import { t } from "$lib/i18n";
  import { fmtDuration, thumbUrl } from "$lib/tube/format";
  import { mplayer, moveInQueue, playAt, removeAt, toggle } from "$lib/tube/music.svelte";
  import TIcon from "../TIcon.svelte";

  const ROW = 56;

  let list = $state<HTMLDivElement>();
  let drag = $state<{ from: number; start: number; dy: number } | null>(null);

  let over = $derived(drag ? Math.min(mplayer.queue.length - 1, Math.max(0, drag.from + Math.round(drag.dy / ROW))) : -1);

  let source = $derived.by(() => {
    const c = mplayer.context;
    if (c.startsWith("radio:")) return $t("tube.mplayer.from_radio") as string;
    if (c.startsWith("album:")) return $t("tube.mplayer.from_album") as string;
    if (c.startsWith("playlist:")) return $t("tube.mplayer.from_playlist") as string;
    return "";
  });

  function shift(i: number): number {
    if (!drag || i === drag.from) return 0;
    if (drag.from < over && i > drag.from && i <= over) return -ROW;
    if (drag.from > over && i < drag.from && i >= over) return ROW;
    return 0;
  }

  function down(e: PointerEvent, i: number) {
    if (e.button !== 0) return;
    e.stopPropagation();
    (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
    drag = { from: i, start: e.clientY, dy: 0 };
  }
  function move(e: PointerEvent) {
    if (!drag) return;
    drag.dy = e.clientY - drag.start;
    if (list) {
      const r = list.getBoundingClientRect();
      if (e.clientY < r.top + 40) list.scrollTop -= 8;
      else if (e.clientY > r.bottom - 40) list.scrollTop += 8;
    }
  }
  function up() {
    if (!drag) return;
    const from = drag.from;
    const to = over;
    drag = null;
    if (to >= 0 && to !== from) moveInQueue(from, to);
  }
  function keyMove(e: KeyboardEvent, i: number) {
    if (!e.altKey) return;
    if (e.key === "ArrowUp" && i > 0) moveInQueue(i, i - 1);
    else if (e.key === "ArrowDown" && i < mplayer.queue.length - 1) moveInQueue(i, i + 1);
    else return;
    e.preventDefault();
  }

  $effect(() => {
    const i = mplayer.index;
    if (!list || i < 0 || drag) return;
    const row = list.querySelector<HTMLElement>(`[data-i="${i}"]`);
    row?.scrollIntoView({ block: "nearest", behavior: "smooth" });
  });
</script>

<div class="qp">
  {#if source}<div class="src">{source}</div>{/if}
  {#if !mplayer.queue.length}
    <div class="empty">
      <img src="/icons/tube/queue-128.webp" alt="" width="96" height="96" />
      <p>{$t("tube.mplayer.queue_empty")}</p>
    </div>
  {:else}
    <div class="list yt-scroll" bind:this={list} role="list" onpointermove={move} onpointerup={up} onpointercancel={up}>
      {#each mplayer.queue as s, i (s.id + ":" + i)}
        {@const cur = i === mplayer.index}
        <div
          class="row"
          class:cur
          class:dragging={drag?.from === i}
          data-i={i}
          role="listitem"
          style:transform={drag?.from === i ? `translateY(${drag.dy}px)` : shift(i) ? `translateY(${shift(i)}px)` : undefined}
        >
          <button class="main" onclick={() => (cur ? toggle() : playAt(i))} onkeydown={(e) => keyMove(e, i)} aria-current={cur ? "true" : undefined}>
            <span class="art">
              <img src={thumbUrl(s.thumbs, 40)} alt="" width="40" height="40" loading="lazy" decoding="async" referrerpolicy="no-referrer" />
              {#if cur}
                <span class="eq" class:paused={!mplayer.playing} aria-hidden="true"><i></i><i></i><i></i></span>
              {:else}
                <span class="hov" aria-hidden="true"><TIcon name="play" size={18} /></span>
              {/if}
            </span>
            <span class="meta">
              <span class="ti yt-clamp-1">{s.title}</span>
              <span class="ar yt-clamp-1">{s.artists.map((a) => a.name).join(", ")}</span>
            </span>
            <span class="dur">{s.duration_s ? fmtDuration(s.duration_s) : ""}</span>
          </button>
          {#if !cur}
            <button class="x" onclick={() => removeAt(i)} aria-label={$t("tube.mplayer.remove") as string} title={$t("tube.mplayer.remove") as string}>
              <TIcon name="close" size={16} />
            </button>
          {/if}
          <span class="handle" role="button" tabindex="-1" aria-label={$t("tube.mplayer.reorder") as string} onpointerdown={(e) => down(e, i)}>
            <TIcon name="menu" size={18} />
          </span>
        </div>
      {/each}
      {#if mplayer.extending}
        <div class="more"><span class="yt-skel"></span><span class="yt-skel"></span></div>
      {/if}
    </div>
  {/if}
</div>

<style>
  .qp {
    display: flex;
    flex-direction: column;
    min-height: 0;
    height: 100%;
  }
  .src {
    padding: 12px 8px 8px;
    font: 400 12px/16px var(--yt-font);
    color: var(--ytm-text-2);
    text-transform: uppercase;
    letter-spacing: 0.04em;
  }
  .list {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    overscroll-behavior: contain;
    padding-bottom: 16px;
  }
  .row {
    position: relative;
    display: flex;
    align-items: center;
    height: 56px;
    border-radius: 6px;
    transition: transform 180ms var(--yt-ease-standard);
    background: transparent;
  }
  .row:hover {
    background: var(--ytm-row-hover);
  }
  .row.cur {
    background: var(--ytm-row-playing);
  }
  .row.dragging {
    z-index: 2;
    transition: none;
    background: var(--ytm-bg-raised);
    box-shadow: 0 8px 24px rgba(0, 0, 0, 0.45);
  }
  .main {
    display: flex;
    align-items: center;
    gap: 14px;
    flex: 1;
    min-width: 0;
    height: 100%;
    padding: 0 8px;
    border: 0;
    background: none;
    color: inherit;
    text-align: left;
    cursor: pointer;
  }
  .main:focus-visible,
  .x:focus-visible {
    outline: 2px solid var(--ytm-link);
    outline-offset: -2px;
  }
  .art {
    position: relative;
    width: 40px;
    height: 40px;
    border-radius: 4px;
    overflow: hidden;
    background: var(--ytm-chip);
    flex: none;
  }
  .art img {
    width: 100%;
    height: 100%;
    object-fit: cover;
    display: block;
  }
  .hov,
  .eq {
    position: absolute;
    inset: 0;
    display: grid;
    place-items: center;
    background: rgba(0, 0, 0, 0.5);
    color: #fff;
  }
  .hov {
    opacity: 0;
    transition: opacity var(--yt-dur-base);
  }
  .row:hover .hov {
    opacity: 1;
  }
  .eq {
    display: flex;
    align-items: flex-end;
    justify-content: center;
    gap: 2px;
    padding-bottom: 12px;
  }
  .eq i {
    width: 3px;
    height: 14px;
    border-radius: 1px;
    background: #fff;
    transform-origin: bottom;
    animation: eq 0.9s ease-in-out infinite;
  }
  .eq i:nth-child(2) {
    animation-delay: -0.3s;
  }
  .eq i:nth-child(3) {
    animation-delay: -0.6s;
  }
  .eq.paused i {
    animation-play-state: paused;
  }
  @keyframes eq {
    0%,
    100% {
      transform: scaleY(0.3);
    }
    50% {
      transform: scaleY(1);
    }
  }
  .meta {
    display: flex;
    flex-direction: column;
    min-width: 0;
    flex: 1;
  }
  .ti {
    font: 500 14px/20px var(--yt-font);
    color: var(--ytm-text);
  }
  .ar {
    font: 400 14px/20px var(--yt-font);
    color: var(--ytm-text-2);
  }
  .dur {
    font: 400 14px/20px var(--yt-font);
    color: var(--ytm-text-2);
    font-variant-numeric: tabular-nums;
  }
  .x,
  .handle {
    display: grid;
    place-items: center;
    width: 32px;
    height: 32px;
    border: 0;
    border-radius: 50%;
    color: var(--ytm-text-2);
    background: transparent;
    opacity: 0;
    flex: none;
    cursor: pointer;
    transition: opacity var(--yt-dur-base);
  }
  .handle {
    cursor: grab;
    margin-right: 4px;
    touch-action: none;
  }
  .row:hover .x,
  .row:hover .handle,
  .row:focus-within .x,
  .row.dragging .handle {
    opacity: 1;
  }
  .x:hover {
    background: var(--ytm-chip);
    color: var(--ytm-text);
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
  .more {
    display: flex;
    flex-direction: column;
    gap: 8px;
    padding: 8px;
  }
  .more .yt-skel {
    height: 40px;
  }
  @media (prefers-reduced-motion: reduce) {
    .row {
      transition: none;
    }
    .eq i {
      animation: none;
    }
  }
</style>

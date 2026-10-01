<script lang="ts">
  import { t } from "$lib/i18n";
  import { cancelNext, player, playNext } from "$lib/video/player.svelte";
  import VIcon from "./VIcon.svelte";

  const TOTAL = 8;
  let left = $derived(player.nextIn ?? 0);
  let nextTitle = $derived.by(() => {
    const next = player.info?.next;
    if (!next || next.kind !== "local") return "";
    for (const m of player.course?.modules ?? []) {
      const l = m.lessons.find((x) => x.id === next.id);
      if (l) return `${l.number} · ${l.title}`;
    }
    return player.folder.find((x) => x.id === next.id)?.title ?? "";
  });
</script>

{#if player.nextIn != null}
  <div class="next" role="status">
    <svg class="ring" viewBox="0 0 36 36" aria-hidden="true">
      <circle cx="18" cy="18" r="15" class="bgc" />
      <circle cx="18" cy="18" r="15" class="fg" style:stroke-dashoffset={94.25 * (1 - left / TOTAL)} />
      <text x="18" y="22" text-anchor="middle">{left}</text>
    </svg>
    <div class="txt">
      <span class="k">{$t(player.info?.course_id != null ? "video.player.next_lesson" : "video.player.next_up")}</span>
      {#if nextTitle}<span class="n">{nextTitle}</span>{/if}
    </div>
    <button class="btn btn-sm btn-primary" onclick={playNext}><VIcon name="play" size={12} /> {$t("video.player.play_now")}</button>
    <button class="btn btn-sm" onclick={cancelNext}>{$t("video.cancel")}</button>
  </div>
{/if}

<style>
  .next {
    position: absolute;
    right: 24px;
    bottom: 120px;
    display: flex;
    align-items: center;
    gap: 12px;
    max-width: min(520px, 90%);
    padding: 12px 14px;
    border-radius: var(--radius-xl);
    background: var(--material-thick);
    backdrop-filter: var(--material-blur);
    -webkit-backdrop-filter: var(--material-blur);
    box-shadow: var(--elev-3);
    color: var(--text);
    z-index: 12;
  }
  .ring {
    width: 42px;
    height: 42px;
    flex-shrink: 0;
    transform: rotate(-90deg);
  }
  .ring text {
    transform: rotate(90deg);
    transform-origin: 18px 18px;
    font-size: 12px;
    font-weight: 700;
    fill: currentColor;
  }
  .bgc,
  .fg {
    fill: none;
    stroke-width: 3;
  }
  .bgc {
    stroke: var(--fill-3);
  }
  .fg {
    stroke: var(--accent);
    stroke-linecap: round;
    stroke-dasharray: 94.25;
    transition: stroke-dashoffset 1s linear;
  }
  .txt {
    display: flex;
    flex-direction: column;
    min-width: 0;
    flex: 1;
  }
  .k {
    font-size: var(--text-xs);
    color: var(--text-dim);
    text-transform: uppercase;
    letter-spacing: var(--track-caps);
  }
  .n {
    font-weight: 600;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
</style>

<script lang="ts">
  import { t } from "$lib/i18n";
  import { fmtDuration } from "$lib/tube/format";
  import { absUrl, storyboardFrame } from "$lib/tube/dash";
  import { tubePrefs } from "$lib/tube/prefs.svelte";
  import { seek, vplayer } from "$lib/tube/video.svelte";

  let { compact = false }: { compact?: boolean } = $props();

  let bar = $state<HTMLDivElement>();
  let hover = $state<number | null>(null);
  let drag = $state<number | null>(null);
  let barW = $state(0);

  let dur = $derived(vplayer.duration || vplayer.streams?.duration_s || 0);
  let pos = $derived(drag != null ? drag * dur : vplayer.position);
  let chapters = $derived(vplayer.info?.chapters ?? []);

  let ranges = $derived.by(() => {
    if (dur <= 0 || chapters.length < 2) return [{ start: 0, end: Math.max(dur, 1), title: "" }];
    const cs = [...chapters].sort((a, b) => a.start_s - b.start_s);
    return cs.map((c, i) => ({ start: c.start_s, end: i + 1 < cs.length ? cs[i + 1].start_s : dur, title: c.title }));
  });

  let sbSegs = $derived.by(() => {
    const p = tubePrefs();
    if (!p.sponsorblock) return [];
    return vplayer.segments.filter((s) => (s.action === "skip" || s.action === "mute") && (p.sb_categories[s.category] ?? "off") !== "off" && s.end_s > s.start_s);
  });
  let highlight = $derived(vplayer.sbHighlight);

  let hoverT = $derived(hover != null ? hover * dur : null);
  let hoverRange = $derived(hoverT != null ? ranges.findIndex((r) => hoverT! >= r.start && hoverT! < r.end) : -1);
  let frame = $derived.by(() => {
    const set = vplayer.streams;
    const sb = set?.storyboard;
    if (!set || !sb || hoverT == null) return null;
    const f = storyboardFrame(sb, hoverT, dur, 160);
    return f ? { ...f, url: absUrl(set, f.url) } : null;
  });

  function fracAt(x: number): number {
    if (!bar) return 0;
    const r = bar.getBoundingClientRect();
    return Math.min(1, Math.max(0, (x - r.left) / r.width));
  }

  function down(e: PointerEvent) {
    if (e.button !== 0 || dur <= 0 || vplayer.streams?.is_live) return;
    (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
    drag = fracAt(e.clientX);
    hover = drag;
  }
  function move(e: PointerEvent) {
    const f = fracAt(e.clientX);
    hover = f;
    if (drag != null) drag = f;
  }
  function up(e: PointerEvent) {
    if (drag == null) return;
    const f = fracAt(e.clientX);
    drag = null;
    seek(f * dur);
  }
  function key(e: KeyboardEvent) {
    if (e.key === "ArrowLeft" || e.key === "ArrowRight") {
      e.preventDefault();
      e.stopPropagation();
      seek(vplayer.position + (e.key === "ArrowLeft" ? -5 : 5));
    }
  }

  function pct(s: number): number {
    return dur > 0 ? Math.min(100, Math.max(0, (s / dur) * 100)) : 0;
  }
  function within(r: { start: number; end: number }, s: number): number {
    const w = r.end - r.start;
    return w > 0 ? Math.min(100, Math.max(0, ((s - r.start) / w) * 100)) : 0;
  }

  let tipX = $derived.by(() => {
    if (hover == null) return 0;
    const half = frame ? 84 : 40;
    return Math.min(Math.max(hover * barW, half), Math.max(half, barW - half));
  });
</script>

<div
  class="scrub"
  class:compact
  class:dragging={drag != null}
  bind:this={bar}
  bind:clientWidth={barW}
  role="slider"
  tabindex="0"
  aria-label={$t("tube.player.seek") as string}
  aria-valuemin={0}
  aria-valuemax={Math.round(dur)}
  aria-valuenow={Math.round(pos)}
  aria-valuetext={fmtDuration(pos)}
  onpointerdown={down}
  onpointermove={move}
  onpointerup={up}
  onpointerleave={() => drag == null && (hover = null)}
  onkeydown={key}
>
  <div class="segs">
    {#each ranges as r, i (r.start)}
      <div class="seg" class:hot={i === hoverRange && ranges.length > 1} style:flex-grow={r.end - r.start}>
        <span class="lay buf" style:width="{within(r, vplayer.buffered)}%"></span>
        {#if hoverT != null && drag == null}<span class="lay hov" style:width="{within(r, hoverT)}%"></span>{/if}
        <span class="lay play" style:width="{within(r, pos)}%"></span>
      </div>
    {/each}
    {#each sbSegs as s (s.uuid)}
      <span class="sb" style:left="{pct(s.start_s)}%" style:width="{Math.max(0.3, pct(s.end_s) - pct(s.start_s))}%" style:background="var(--sb-{s.category}, #fff)"></span>
    {/each}
    {#if highlight}
      <span class="poi" style:left="{pct(highlight.start_s)}%" style:background="var(--sb-poi_highlight)"></span>
    {/if}
  </div>
  <span class="knob" style:left="{pct(pos)}%"></span>

  {#if hover != null && dur > 0}
    <div class="tip" style:left="{tipX}px">
      {#if frame}
        <span
          class="frame"
          style:width="{frame.w}px"
          style:height="{frame.h}px"
          style:background-image="url('{frame.url}')"
          style:background-size="{frame.sheetW}px {frame.sheetH}px"
          style:background-position="-{frame.x}px -{frame.y}px"
        ></span>
      {/if}
      {#if hoverRange >= 0 && ranges[hoverRange].title}<span class="ch">{ranges[hoverRange].title}</span>{/if}
      <span class="tm">{fmtDuration(hoverT)}</span>
    </div>
  {/if}
</div>

<style>
  .scrub {
    position: relative;
    height: 20px;
    margin: 0 12px;
    display: flex;
    align-items: center;
    cursor: pointer;
    touch-action: none;
    outline: none;
  }
  .scrub:focus-visible {
    outline: 2px solid var(--yt-link);
    outline-offset: 2px;
    border-radius: 4px;
  }
  .segs {
    position: relative;
    display: flex;
    gap: 2px;
    width: 100%;
    height: 5px;
    transform: scaleY(0.6);
    transition: transform var(--yt-dur-fast) var(--yt-ease-exit);
  }
  .scrub:hover .segs,
  .scrub.dragging .segs,
  .scrub:focus-visible .segs {
    transform: scaleY(1);
  }
  .seg {
    position: relative;
    flex-basis: 0;
    min-width: 2px;
    height: 100%;
    overflow: hidden;
    background: var(--yt-ov-track);
    transition: transform var(--yt-dur-fast) var(--yt-ease-exit);
  }
  .seg:first-child {
    border-radius: 3px 0 0 3px;
  }
  .seg:last-child {
    border-radius: 0 3px 3px 0;
  }
  .seg:only-child {
    border-radius: 3px;
  }
  .seg.hot {
    transform: scaleY(1.6);
  }
  .lay {
    position: absolute;
    left: 0;
    top: 0;
    bottom: 0;
  }
  .buf {
    background: var(--yt-ov-buffer);
  }
  .hov {
    background: rgba(255, 255, 255, 0.5);
  }
  .play {
    background: var(--yt-accent);
  }
  .sb {
    position: absolute;
    top: 0;
    bottom: 0;
    opacity: 0.85;
    pointer-events: none;
  }
  .poi {
    position: absolute;
    top: -1px;
    bottom: -1px;
    width: 4px;
    margin-left: -2px;
    border-radius: 2px;
    pointer-events: none;
  }
  .knob {
    position: absolute;
    top: 50%;
    width: 13px;
    height: 13px;
    margin: -6.5px 0 0 -6.5px;
    border-radius: 50%;
    background: var(--yt-accent);
    transform: scale(0);
    transition: transform var(--yt-dur-fast) var(--yt-ease-exit);
    pointer-events: none;
  }
  .scrub:hover .knob,
  .scrub.dragging .knob,
  .scrub:focus-visible .knob {
    transform: scale(1);
  }
  .tip {
    position: absolute;
    bottom: 22px;
    transform: translateX(-50%);
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 4px;
    pointer-events: none;
    color: var(--yt-ov-text);
    text-shadow: 0 0 2px rgba(0, 0, 0, 0.8);
  }
  .frame {
    display: block;
    border-radius: 8px;
    background-repeat: no-repeat;
    background-color: #000;
    box-shadow:
      0 0 0 2px rgba(255, 255, 255, 0.9),
      0 8px 24px rgba(0, 0, 0, 0.5);
  }
  .ch {
    max-width: 200px;
    font: 500 13px/18px var(--yt-font);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .tm {
    font: 500 13px/18px var(--yt-font);
    padding: 1px 6px;
    border-radius: 4px;
    background: var(--yt-ov-pill);
    backdrop-filter: blur(16px);
    -webkit-backdrop-filter: blur(16px);
  }
  .compact {
    margin: 0 8px;
  }
  @media (prefers-reduced-motion: reduce) {
    .segs,
    .seg,
    .knob {
      transition: none;
    }
  }
</style>

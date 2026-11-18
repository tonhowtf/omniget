<script lang="ts">
  import { t } from "$lib/i18n";
  import { formatTimecode } from "$lib/capture/timecode";
  import type { Segment } from "$lib/capture/segments";

  let {
    duration,
    current,
    keyframes,
    segments,
    pendingIn,
    selectedId,
    onseek,
    onselect,
  }: {
    duration: number;
    current: number;
    keyframes: number[];
    segments: Segment[];
    pendingIn: number | null;
    selectedId: string | null;
    onseek: (t: number) => void;
    onselect: (id: string) => void;
  } = $props();

  let track: HTMLDivElement | null = $state(null);
  let scrubbing = false;

  const pct = (v: number) => (duration > 0 ? Math.min(100, Math.max(0, (v / duration) * 100)) : 0);

  let ticksPath = $derived(
    duration > 0
      ? keyframes
          .filter((k) => k >= 0 && k <= duration)
          .map((k) => `M${((k / duration) * 1000).toFixed(2)} 0V10`)
          .join("")
      : "",
  );

  let rulers = $derived(duration > 0 ? [0, 0.25, 0.5, 0.75, 1].map((f) => f * duration) : []);

  function timeAt(clientX: number): number {
    if (!track || duration <= 0) return 0;
    const rect = track.getBoundingClientRect();
    const f = Math.min(1, Math.max(0, (clientX - rect.left) / rect.width));
    return f * duration;
  }

  function down(event: PointerEvent) {
    if (event.button !== 0) return;
    scrubbing = true;
    track?.setPointerCapture(event.pointerId);
    onseek(timeAt(event.clientX));
  }

  function move(event: PointerEvent) {
    if (scrubbing) onseek(timeAt(event.clientX));
  }

  function up(event: PointerEvent) {
    scrubbing = false;
    track?.releasePointerCapture(event.pointerId);
  }
</script>

<div class="tl">
  <div
    class="tl-track"
    bind:this={track}
    role="slider"
    tabindex="0"
    aria-label={$t("capture.timeline")}
    aria-valuemin={0}
    aria-valuemax={Math.round(duration * 1000) / 1000}
    aria-valuenow={Math.round(current * 1000) / 1000}
    aria-valuetext={formatTimecode(current)}
    onpointerdown={down}
    onpointermove={move}
    onpointerup={up}
    onpointercancel={up}
  >
    {#each segments as seg, i (seg.id)}
      <button
        type="button"
        class="tl-seg"
        class:selected={seg.id === selectedId}
        style:left="{pct(seg.start)}%"
        style:width="{Math.max(0.3, pct(seg.end) - pct(seg.start))}%"
        aria-label={$t("capture.segment_label", { num: i + 1, start: formatTimecode(seg.start), end: formatTimecode(seg.end) })}
        onpointerdown={(e) => e.stopPropagation()}
        onclick={() => onselect(seg.id)}
      >
        <span class="tl-seg-n">{i + 1}</span>
      </button>
    {/each}
    {#if pendingIn !== null}
      <span class="tl-pending" style:left="{pct(pendingIn)}%" style:width="{Math.max(0, pct(current) - pct(pendingIn))}%" aria-hidden="true"></span>
      <span class="tl-in" style:left="{pct(pendingIn)}%" aria-hidden="true"></span>
    {/if}
    <svg class="tl-keys" viewBox="0 0 1000 10" preserveAspectRatio="none" aria-hidden="true">
      <path d={ticksPath} />
    </svg>
    <span class="tl-head" style:left="{pct(current)}%" aria-hidden="true"></span>
  </div>
  <div class="tl-ruler" aria-hidden="true">
    {#each rulers as r, i (i)}
      <span style:left="{pct(r)}%" class:end={i === rulers.length - 1} class:start={i === 0}>{formatTimecode(r, false)}</span>
    {/each}
  </div>
</div>

<style>
  .tl {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
    user-select: none;
  }

  .tl-track {
    position: relative;
    height: 52px;
    border-radius: var(--radius-md);
    background: var(--surface-mut);
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
    cursor: ew-resize;
    touch-action: none;
    overflow: hidden;
  }

  .tl-track:focus-visible {
    outline: var(--focus-ring);
    outline-offset: 2px;
  }

  .tl-seg {
    position: absolute;
    top: 6px;
    bottom: 14px;
    display: flex;
    align-items: flex-start;
    padding: 2px 4px;
    border: none;
    border-radius: var(--radius-xs);
    background: color-mix(in srgb, var(--accent) 32%, transparent);
    box-shadow: inset 0 0 0 1px color-mix(in srgb, var(--accent) 70%, transparent);
    color: var(--text);
    cursor: pointer;
  }

  .tl-seg.selected {
    background: color-mix(in srgb, var(--accent) 55%, transparent);
    box-shadow: inset 0 0 0 2px var(--accent);
  }

  .tl-seg:focus-visible {
    outline: var(--focus-ring);
    outline-offset: 1px;
  }

  .tl-seg-n {
    font-size: var(--text-caption);
    font-weight: 700;
    font-variant-numeric: tabular-nums;
  }

  .tl-pending {
    position: absolute;
    top: 6px;
    bottom: 14px;
    background: repeating-linear-gradient(135deg, color-mix(in srgb, var(--accent) 22%, transparent) 0 6px, transparent 6px 12px);
    pointer-events: none;
  }

  .tl-in {
    position: absolute;
    top: 2px;
    bottom: 10px;
    width: 2px;
    margin-left: -1px;
    background: var(--accent);
    pointer-events: none;
  }

  .tl-keys {
    position: absolute;
    left: 0;
    right: 0;
    bottom: 0;
    width: 100%;
    height: 10px;
    pointer-events: none;
  }

  .tl-keys path {
    stroke: var(--text-dim);
    stroke-width: 1;
    vector-effect: non-scaling-stroke;
  }

  .tl-head {
    position: absolute;
    top: 0;
    bottom: 0;
    width: 2px;
    margin-left: -1px;
    background: var(--text);
    pointer-events: none;
  }

  .tl-ruler {
    position: relative;
    height: 14px;
    font-size: var(--text-caption);
    color: var(--text-dim);
    font-variant-numeric: tabular-nums;
  }

  .tl-ruler span {
    position: absolute;
    transform: translateX(-50%);
  }

  .tl-ruler span.start {
    transform: none;
  }

  .tl-ruler span.end {
    transform: translateX(-100%);
  }
</style>

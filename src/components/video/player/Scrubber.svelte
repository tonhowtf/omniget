<script lang="ts">
  import { t } from "$lib/i18n";
  import { fmtTime } from "$lib/video/format";
  import { player, seek } from "$lib/video/player.svelte";

  let bar: HTMLDivElement | undefined = $state();
  let hoverX = $state<number | null>(null);
  let dragFrac = $state<number | null>(null);

  let dur = $derived(player.duration);
  let chapters = $derived(player.info?.chapters ?? []);
  let frac = $derived(dragFrac ?? (dur > 0 ? Math.min(1, player.position / dur) : 0));

  function fracAt(clientX: number): number {
    if (!bar) return 0;
    const r = bar.getBoundingClientRect();
    return Math.min(1, Math.max(0, (clientX - r.left) / r.width));
  }

  let hoverFrac = $derived(dragFrac ?? (hoverX != null ? hoverX : null));
  let hoverMs = $derived(hoverFrac != null ? hoverFrac * dur * 1000 : 0);
  let hoverChapter = $derived.by(() => {
    if (hoverFrac == null) return "";
    const ms = hoverMs;
    return chapters.find((c) => ms >= c.start_ms && ms < c.end_ms)?.title ?? "";
  });

  function down(e: PointerEvent) {
    if (dur <= 0 || e.button !== 0) return;
    (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
    dragFrac = fracAt(e.clientX);
  }
  function move(e: PointerEvent) {
    const f = fracAt(e.clientX);
    hoverX = f;
    if (dragFrac != null) dragFrac = f;
  }
  function up(e: PointerEvent) {
    if (dragFrac == null) return;
    const f = fracAt(e.clientX);
    dragFrac = null;
    seek(f * dur);
  }

  function pct(ms: number): string {
    return `${dur > 0 ? Math.min(100, (ms / 1000 / dur) * 100) : 0}%`;
  }
</script>

<div
  class="scrub"
  bind:this={bar}
  role="slider"
  tabindex="-1"
  aria-label={$t("video.player.seek") as string}
  aria-valuemin={0}
  aria-valuemax={Math.round(dur)}
  aria-valuenow={Math.round(player.position)}
  aria-valuetext={fmtTime(player.position * 1000)}
  onpointerdown={down}
  onpointermove={move}
  onpointerup={up}
  onpointerleave={() => (hoverX = null)}
>
  <div class="track">
    {#if chapters.length > 1}
      {#each chapters.slice(1) as c (c.start_ms)}
        <span class="gap" style:left={pct(c.start_ms)}></span>
      {/each}
    {/if}
    {#each player.silences as s (s[0])}
      <span class="silence" style:left={pct(s[0] * 1000)} style:width={pct((s[1] - s[0]) * 1000)}></span>
    {/each}
    {#each chapters.filter((c) => c.kind) as c (c.start_ms)}
      <span class="skipseg" style:left={pct(c.start_ms)} style:width={pct(c.end_ms - c.start_ms)}></span>
    {/each}
    <span class="fill" style:width="{frac * 100}%"></span>
    {#if player.ab.a != null}
      <span
        class="ab"
        style:left={pct(player.ab.a * 1000)}
        style:width={player.ab.b != null ? pct((player.ab.b - player.ab.a) * 1000) : "2px"}
      ></span>
    {/if}
  </div>
  {#each player.notes as n (n.id)}
    <button
      class="mark"
      style:left={pct(n.t_ms)}
      title="{fmtTime(n.t_ms)} · {n.text}"
      aria-label="{$t('video.notes.jump')} {fmtTime(n.t_ms)}"
      onpointerdown={(e) => e.stopPropagation()}
      onclick={() => seek(n.t_ms / 1000)}
    ></button>
  {/each}
  <span class="thumb" style:left="{frac * 100}%"></span>
  {#if hoverFrac != null && dur > 0}
    <div class="tip" style:left="{hoverFrac * 100}%">
      {#if hoverChapter}<span class="ch">{hoverChapter}</span>{/if}
      <span class="tm">{fmtTime(hoverMs)}</span>
    </div>
  {/if}
</div>

<style>
  .scrub {
    position: relative;
    height: 18px;
    display: flex;
    align-items: center;
    cursor: pointer;
    touch-action: none;
    outline: none;
  }
  .track {
    position: relative;
    width: 100%;
    height: 4px;
    border-radius: var(--radius-full);
    background: color-mix(in srgb, var(--pl-ink) 22%, transparent);
    overflow: hidden;
    transition: height var(--duration-fast) var(--ease-out);
  }
  .scrub:hover .track {
    height: 7px;
  }
  .fill {
    position: absolute;
    inset: 0 auto 0 0;
    background: var(--accent);
    pointer-events: none;
  }
  .gap {
    position: absolute;
    top: 0;
    bottom: 0;
    width: 2px;
    margin-left: -1px;
    background: color-mix(in srgb, var(--pl-shade) 55%, transparent);
    z-index: 2;
  }
  .silence {
    position: absolute;
    top: 0;
    bottom: 0;
    background: color-mix(in srgb, var(--pl-ink) 12%, transparent);
  }
  .skipseg {
    position: absolute;
    top: 0;
    bottom: 0;
    background: color-mix(in srgb, var(--teal) 55%, transparent);
  }
  .ab {
    position: absolute;
    top: 0;
    bottom: 0;
    background: color-mix(in srgb, var(--purple) 60%, transparent);
    z-index: 1;
  }
  .mark {
    all: unset;
    position: absolute;
    top: 50%;
    width: 8px;
    height: 8px;
    margin: -4px 0 0 -4px;
    border-radius: 50%;
    background: var(--warning);
    box-shadow: 0 0 0 2px color-mix(in srgb, var(--pl-shade) 50%, transparent);
    cursor: pointer;
    z-index: 3;
  }
  .mark:hover,
  .mark:focus-visible {
    transform: scale(1.4);
  }
  .thumb {
    position: absolute;
    top: 50%;
    width: 13px;
    height: 13px;
    margin: -6.5px 0 0 -6.5px;
    border-radius: 50%;
    background: var(--pl-ink);
    box-shadow: 0 1px 4px color-mix(in srgb, var(--pl-shade) 50%, transparent);
    opacity: 0;
    transition: opacity var(--duration-fast);
    pointer-events: none;
    z-index: 4;
  }
  .scrub:hover .thumb {
    opacity: 1;
  }
  .tip {
    position: absolute;
    bottom: 22px;
    transform: translateX(-50%);
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 1px;
    padding: 4px 8px;
    border-radius: var(--radius-sm);
    background: color-mix(in srgb, var(--pl-shade) 78%, transparent);
    color: var(--pl-ink);
    font-size: var(--text-xs);
    white-space: nowrap;
    pointer-events: none;
    z-index: 5;
  }
  .ch {
    max-width: 220px;
    overflow: hidden;
    text-overflow: ellipsis;
    font-weight: 600;
  }
  .tm {
    font-variant-numeric: tabular-nums;
  }
</style>

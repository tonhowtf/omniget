<script lang="ts">
  import { t } from "$lib/i18n";
  import { Y_MAX, Y_MIN, bezierEase, moveHandle, type Bezier } from "$lib/motion/bezier";
  import { num } from "$lib/motion/format";

  let {
    value = $bindable(),
    ghost = null,
    label,
    compact = false,
    disabled = false,
  }: { value: Bezier; ghost?: Bezier | null; label: string; compact?: boolean; disabled?: boolean } = $props();

  const L = 50;
  const R = 250;
  const TOP = 50;
  const BOTTOM = 250;
  const SIZE = R - L;

  let svg = $state<SVGSVGElement | null>(null);
  let dragging = $state<0 | 1 | null>(null);

  const px = (x: number) => L + x * SIZE;
  const py = (y: number) => BOTTOM - y * SIZE;

  function path(b: Bezier): string {
    return `M${px(0)},${py(0)} C${px(b[0])},${py(b[1])} ${px(b[2])},${py(b[3])} ${px(1)},${py(1)}`;
  }

  let curve = $derived(path(value));
  let ghostCurve = $derived(ghost ? path(ghost) : "");
  let samples = $derived.by(() => {
    const f = bezierEase(value);
    return Array.from({ length: 11 }, (_, i) => ({ x: i / 10, y: f(i / 10) }));
  });

  function toValue(event: PointerEvent): [number, number] | null {
    if (!svg) return null;
    const ctm = svg.getScreenCTM();
    if (!ctm) return null;
    const pt = new DOMPoint(event.clientX, event.clientY).matrixTransform(ctm.inverse());
    return [(pt.x - L) / SIZE, (BOTTOM - pt.y) / SIZE];
  }

  function setHandle(handle: 0 | 1, x: number, y: number) {
    value = moveHandle(value, handle, x, y);
  }

  function down(handle: 0 | 1, event: PointerEvent) {
    if (disabled) return;
    event.preventDefault();
    dragging = handle;
    (event.currentTarget as Element).setPointerCapture?.(event.pointerId);
    (event.currentTarget as SVGElement).focus();
  }

  function move(event: PointerEvent) {
    if (dragging === null) return;
    const v = toValue(event);
    if (v) setHandle(dragging, v[0], v[1]);
  }

  function up() {
    dragging = null;
  }

  function key(handle: 0 | 1, event: KeyboardEvent) {
    if (disabled) return;
    const step = event.shiftKey ? 0.1 : event.altKey ? 0.001 : 0.01;
    let dx = 0;
    let dy = 0;
    if (event.key === "ArrowLeft") dx = -step;
    else if (event.key === "ArrowRight") dx = step;
    else if (event.key === "ArrowUp") dy = step;
    else if (event.key === "ArrowDown") dy = -step;
    else return;
    event.preventDefault();
    setHandle(handle, value[handle * 2] + dx, value[handle * 2 + 1] + dy);
  }

  function handleLabel(handle: 0 | 1): string {
    return $t("motion.curve.handle", {
      n: handle + 1,
      x: num(value[handle * 2], 2),
      y: num(value[handle * 2 + 1], 2),
    }) as string;
  }
</script>

<figure class="ce" class:compact class:disabled>
  <svg
    bind:this={svg}
    viewBox="0 {py(Y_MAX) - 10} 300 {(Y_MAX - Y_MIN) * SIZE + 20}"
    role="group"
    aria-label={label}
    onpointermove={move}
    onpointerup={up}
    onpointercancel={up}
  >
    <rect class="ce-over" x={L} y={py(Y_MAX)} width={SIZE} height={(Y_MAX - Y_MIN) * SIZE} />
    <rect class="ce-unit" x={L} y={TOP} width={SIZE} height={SIZE} />
    {#each [0.25, 0.5, 0.75] as g (g)}
      <line class="ce-grid" x1={px(g)} x2={px(g)} y1={TOP} y2={BOTTOM} />
      <line class="ce-grid" x1={L} x2={R} y1={py(g)} y2={py(g)} />
    {/each}
    <line class="ce-diag" x1={px(0)} y1={py(0)} x2={px(1)} y2={py(1)} />
    {#if ghostCurve}
      <path class="ce-ghost" d={ghostCurve} />
    {/if}
    <line class="ce-arm" x1={px(0)} y1={py(0)} x2={px(value[0])} y2={py(value[1])} />
    <line class="ce-arm" x1={px(1)} y1={py(1)} x2={px(value[2])} y2={py(value[3])} />
    <path class="ce-curve" d={curve} />
    {#each samples as s (s.x)}
      <circle class="ce-tick" cx={px(s.x)} cy={py(s.y)} r="1.6" />
    {/each}
    <circle class="ce-end" cx={px(0)} cy={py(0)} r="4" />
    <circle class="ce-end" cx={px(1)} cy={py(1)} r="4" />
    {#each [0, 1] as const as h (h)}
      <circle
        class="ce-handle"
        class:active={dragging === h}
        cx={px(value[h * 2])}
        cy={py(value[h * 2 + 1])}
        r="9"
        tabindex={disabled ? -1 : 0}
        role="slider"
        aria-label={handleLabel(h)}
        aria-valuetext={handleLabel(h)}
        aria-valuemin={Y_MIN}
        aria-valuemax={Y_MAX}
        aria-valuenow={value[h * 2 + 1]}
        aria-disabled={disabled}
        onpointerdown={(e) => down(h, e)}
        onkeydown={(e) => key(h, e)}
      />
    {/each}
  </svg>
  {#if !compact}
    <figcaption class="ce-cap">{$t("motion.curve.keys_hint")}</figcaption>
  {/if}
</figure>

<style>
  .ce {
    margin: 0;
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    align-items: center;
  }

  svg {
    width: 100%;
    max-width: 300px;
    touch-action: none;
    user-select: none;
    overflow: visible;
  }

  .compact svg {
    max-width: 200px;
  }

  .ce-over {
    fill: var(--fill-1);
  }

  .ce-unit {
    fill: var(--surface-hi);
    stroke: var(--content-border);
    stroke-width: 1;
  }

  .ce-grid {
    stroke: var(--separator);
    stroke-width: 1;
  }

  .ce-diag {
    stroke: var(--text-faint);
    stroke-width: 1;
    stroke-dasharray: 4 4;
  }

  .ce-ghost {
    fill: none;
    stroke: var(--teal);
    stroke-width: 2;
    stroke-dasharray: 6 4;
    opacity: 0.85;
  }

  .ce-arm {
    stroke: var(--text-dim);
    stroke-width: 1.5;
  }

  .ce-curve {
    fill: none;
    stroke: var(--accent);
    stroke-width: 3;
    stroke-linecap: round;
  }

  .ce-tick {
    fill: var(--accent);
  }

  .ce-end {
    fill: var(--text-muted);
  }

  .ce-handle {
    fill: var(--surface-hi);
    stroke: var(--accent);
    stroke-width: 3;
    cursor: grab;
    outline: none;
  }

  .ce-handle.active {
    cursor: grabbing;
    fill: var(--accent);
  }

  .ce-handle:focus-visible {
    stroke: var(--secondary);
    stroke-width: 4;
  }

  .disabled .ce-handle {
    cursor: default;
    opacity: 0.5;
  }

  .ce-cap {
    font-size: var(--text-xs);
    color: var(--text-dim);
    text-align: center;
  }
</style>

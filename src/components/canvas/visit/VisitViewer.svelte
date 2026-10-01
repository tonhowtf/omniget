<script lang="ts">
  import { t } from "$lib/i18n";
  import type { Scene } from "$lib/canvas/visit/api";
  import { cordPaths, sceneBounds, screenTail } from "$lib/canvas/visit/scene";
  import { normaliseState, stateCss } from "$lib/canvas/state/agents.svelte";

  interface Props {
    scene: Scene;
    onPoint?: (x: number, y: number) => void;
  }
  let { scene, onPoint }: Props = $props();

  let svg: SVGSVGElement | undefined = $state();
  let zoom = $state(1);
  let pan = $state({ x: 0, y: 0 });
  let drag: { x: number; y: number; px: number; py: number; moved: boolean } | null = null;

  const base = $derived(sceneBounds(scene));
  const view = $derived({
    w: base.w / zoom,
    h: base.h / zoom,
    x: base.x + (base.w - base.w / zoom) / 2 + pan.x,
    y: base.y + (base.h - base.h / zoom) / 2 + pan.y,
  });
  const cords = $derived(cordPaths(scene));
  const nodes = $derived([...scene.nodes].sort((a, b) => (a.z ?? 0) - (b.z ?? 0)));

  function toWorld(ev: PointerEvent | WheelEvent): [number, number] {
    const r = svg!.getBoundingClientRect();
    const s = Math.max(view.w / r.width, view.h / r.height);
    const ox = view.x + (view.w - r.width * s) / 2;
    const oy = view.y + (view.h - r.height * s) / 2;
    return [ox + (ev.clientX - r.left) * s, oy + (ev.clientY - r.top) * s];
  }

  function wheel(ev: WheelEvent) {
    ev.preventDefault();
    zoom = Math.min(6, Math.max(0.3, zoom * (ev.deltaY < 0 ? 1.12 : 1 / 1.12)));
  }

  function down(ev: PointerEvent) {
    drag = { x: ev.clientX, y: ev.clientY, px: pan.x, py: pan.y, moved: false };
    svg?.setPointerCapture(ev.pointerId);
  }

  function move(ev: PointerEvent) {
    if (!drag || !svg) return;
    const r = svg.getBoundingClientRect();
    const s = Math.max(view.w / r.width, view.h / r.height);
    const dx = ev.clientX - drag.x;
    const dy = ev.clientY - drag.y;
    if (Math.abs(dx) + Math.abs(dy) > 4) drag.moved = true;
    pan = { x: drag.px - dx * s, y: drag.py - dy * s };
  }

  function up(ev: PointerEvent) {
    const d = drag;
    drag = null;
    if (d && !d.moved && onPoint) {
      const [x, y] = toWorld(ev);
      onPoint(x, y);
    }
  }

  function stateLabel(s: string | null | undefined): string {
    const st = normaliseState(s);
    return $t(`canvas.visit.state.${st}`, { default: st }) as string;
  }
</script>

<svg
  bind:this={svg}
  class="viewer"
  viewBox="{view.x} {view.y} {view.w} {view.h}"
  preserveAspectRatio="xMidYMid meet"
  role="img"
  aria-label={$t("canvas.visit.viewer", { default: "Shared canvas" }) as string}
  onwheel={wheel}
  onpointerdown={down}
  onpointermove={move}
  onpointerup={up}
>
  {#each cords as c (c.id)}
    <path d={c.d} class="cord" class:note={c.kind === "note"} />
    {#if c.label}<text x={c.mid[0]} y={c.mid[1]} class="cord-label">{c.label}</text>{/if}
  {/each}
  {#each nodes as n (n.id)}
    {@const color = n.state ? stateCss(normaliseState(n.state)) : "var(--border)"}
    <g>
      <rect x={n.x} y={n.y} width={n.w} height={n.h} rx="12" class="card" />
      {#if n.state}<rect x={n.x} y={n.y} width="6" height={n.h} rx="3" fill={color} />{/if}
      <foreignObject x={n.x + 12} y={n.y + 8} width={Math.max(0, n.w - 20)} height={Math.max(0, n.h - 14)}>
        <div class="body" xmlns="http://www.w3.org/1999/xhtml">
          <div class="title">
            <span class="name">{n.name}</span>
            {#if n.agent}<span class="agent">{n.agent}</span>{/if}
          </div>
          {#if n.state}<div class="state" style="color: {color}">{stateLabel(n.state)}</div>{/if}
          {#if n.screen}
            <pre class="screen">{screenTail(n.screen, Math.max(3, Math.floor((n.h - 60) / 14)))}</pre>
          {:else if n.kind === "terminal" || n.kind === "agent"}
            <div class="private">{$t("canvas.visit.private", { default: "Screen not shared" })}</div>
          {/if}
        </div>
      </foreignObject>
    </g>
  {/each}
  {#each scene.reactions ?? [] as r (r.at + ":" + r.visitorId)}
    {#if r.x != null && r.y != null}
      <text x={r.x} y={r.y} class="reaction">{r.emoji ?? (r.type === "wave" ? "👋" : "👉")}</text>
      {#if r.name}<text x={r.x} y={r.y + 26} class="reaction-name">{r.name}</text>{/if}
    {/if}
  {/each}
</svg>

<style>
  .viewer {
    width: 100%;
    height: 100%;
    display: block;
    background: var(--canvas-bg, var(--bg, #111));
    touch-action: none;
    cursor: grab;
  }
  .cord {
    fill: none;
    stroke: var(--accent, #0a84ff);
    stroke-width: 3;
    opacity: 0.75;
  }
  .cord.note {
    stroke-dasharray: 8 6;
  }
  .cord-label {
    font-size: 13px;
    fill: var(--text-muted, #999);
    text-anchor: middle;
  }
  .card {
    fill: var(--surface, #1c1c1e);
    stroke: var(--border, #38383a);
    stroke-width: 1.5;
  }
  .body {
    color: var(--text, #f5f5f7);
    font: 13px/1.3 -apple-system, system-ui, sans-serif;
    height: 100%;
    overflow: hidden;
  }
  .title {
    display: flex;
    gap: 6px;
    align-items: baseline;
  }
  .name {
    font-weight: 600;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .agent,
  .private {
    color: var(--text-muted, #999);
    font-size: 11px;
  }
  .state {
    font-size: 11px;
    margin: 2px 0 4px;
  }
  .screen {
    margin: 0;
    font: 11px/14px ui-monospace, Menlo, monospace;
    white-space: pre-wrap;
    word-break: break-word;
    color: var(--text, #eee);
    opacity: 0.9;
  }
  .reaction {
    font-size: 28px;
    text-anchor: middle;
  }
  .reaction-name {
    font-size: 12px;
    text-anchor: middle;
    fill: var(--text, #fff);
  }
</style>

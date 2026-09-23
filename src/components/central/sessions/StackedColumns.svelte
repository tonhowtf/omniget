<script lang="ts">
  /**
   * Colunas empilhadas (ou uma série só) em SVG próprio.
   *
   * Uma coluna por categoria ordenada (dia, hora), segmentos na ordem fixa das
   * séries, 2px de superfície entre segmentos, topo arredondado de 4px, grade
   * recessiva, pico rotulado direto. Hover e teclado (setas) mostram um
   * tooltip com todas as séries daquela coluna; a tabela de valores fica em
   * "Valores" para quem não usa ponteiro e para o contraste baixo no tema
   * claro. Com uma série só não há legenda: o título do cartão nomeia.
   */
  import { t } from "$lib/i18n";
  import "./viz.css";

  type Series = { id: string; label: string; color: string };
  type Row = { key: string; label: string; values: Record<string, number> };

  interface Props {
    rows: Row[];
    series: Series[];
    format?: (v: number) => string;
    /** Rótulo do eixo x a cada N colunas (0 = automático). */
    tickEvery?: number;
    height?: number;
    label: string;
    /** Clique numa coluna (ex.: abrir o dia). */
    onpick?: (key: string) => void;
  }

  let { rows, series, format = (v) => String(Math.round(v)), tickEvery = 0, height = 180, label, onpick }: Props = $props();

  const W = 720;
  const PAD_TOP = 18;

  let totals = $derived(rows.map((r) => series.reduce((s, x) => s + Math.max(0, r.values[x.id] ?? 0), 0)));
  let max = $derived(Math.max(0, ...totals));
  let axisTop = $derived.by(() => {
    if (max <= 0) return 1;
    const exp = Math.floor(Math.log10(max));
    const base = Math.pow(10, exp);
    for (const m of [1, 2, 2.5, 5, 10]) if (max <= m * base) return m * base;
    return 10 * base;
  });
  let slot = $derived(rows.length ? W / rows.length : W);
  let gap = $derived(slot > 8 ? 2 : slot > 3 ? 1 : 0);
  let barW = $derived(Math.max(1, Math.min(28, slot - gap)));
  let plotH = $derived(height - PAD_TOP);
  let peak = $derived(totals.reduce((b, v, i) => (v > (totals[b] ?? -1) ? i : b), 0));
  let every = $derived(tickEvery > 0 ? tickEvery : Math.max(1, Math.ceil(rows.length / 8)));

  let hover = $state(-1);
  let wrap: HTMLDivElement | undefined = $state();
  let tipX = $state(0);

  function y(v: number): number {
    return PAD_TOP + plotH - (v / axisTop) * plotH;
  }

  /** Segmentos da coluna i: [serie, y0, y1]. */
  function segs(i: number): { s: Series; top: number; h: number; last: boolean }[] {
    const out: { s: Series; top: number; h: number; last: boolean }[] = [];
    let acc = 0;
    const r = rows[i];
    const visible = series.filter((s) => (r.values[s.id] ?? 0) > 0);
    visible.forEach((s, k) => {
      const v = Math.max(0, r.values[s.id] ?? 0);
      const y1 = y(acc);
      acc += v;
      const y0 = y(acc);
      const g = k < visible.length - 1 && series.length > 1 ? 1 : 0;
      out.push({ s, top: y0 + g, h: Math.max(0, y1 - y0 - g), last: k === visible.length - 1 });
    });
    return out;
  }

  function path(x: number, top: number, w: number, h: number, round: boolean): string {
    if (h <= 0) return "";
    const r = round ? Math.min(4, w / 2, h) : 0;
    const b = top + h;
    return `M${x} ${b}V${top + r}${r ? `Q${x} ${top} ${x + r} ${top}` : ""}H${x + w - r}${r ? `Q${x + w} ${top} ${x + w} ${top + r}` : ""}V${b}Z`;
  }

  function onMove(e: PointerEvent) {
    const svg = e.currentTarget as SVGSVGElement;
    const rect = svg.getBoundingClientRect();
    if (!rect.width || !rows.length) return;
    const x = ((e.clientX - rect.left) / rect.width) * W;
    hover = Math.max(0, Math.min(rows.length - 1, Math.floor(x / slot)));
    placeTip();
  }

  function placeTip() {
    if (!wrap || hover < 0) return;
    const w = wrap.clientWidth;
    const px = ((hover + 0.5) * slot * w) / W;
    tipX = Math.max(0, Math.min(w - 180, px + 12 > w - 180 ? px - 192 : px + 12));
  }

  function onKey(e: KeyboardEvent) {
    if (!rows.length) return;
    if (e.key === "ArrowRight") hover = Math.min(rows.length - 1, (hover < 0 ? -1 : hover) + 1);
    else if (e.key === "ArrowLeft") hover = Math.max(0, (hover < 0 ? rows.length : hover) - 1);
    else if (e.key === "Home") hover = 0;
    else if (e.key === "End") hover = rows.length - 1;
    else if ((e.key === "Enter" || e.key === " ") && hover >= 0 && onpick) {
      e.preventDefault();
      onpick(rows[hover].key);
      return;
    } else if (e.key === "Escape") hover = -1;
    else return;
    e.preventDefault();
    placeTip();
  }
</script>

{#if series.length > 1}
  <ul class="viz-legend" aria-hidden="true">
    {#each series as s (s.id)}
      <li><span class="sw" style:background={s.color}></span>{s.label}</li>
    {/each}
  </ul>
{/if}
<div class="chart" bind:this={wrap} style:--h="{height}px">
  <div class="axis" aria-hidden="true">
    <span>{format(axisTop)}</span>
    <span>{format(axisTop / 2)}</span>
    <span>0</span>
  </div>
  <!-- svelte-ignore a11y_no_noninteractive_tabindex, a11y_no_noninteractive_element_interactions -->
  <svg
    viewBox="0 0 {W} {height}"
    preserveAspectRatio="none"
    role="img"
    aria-label={label}
    tabindex="0"
    onpointermove={onMove}
    onpointerleave={() => (hover = -1)}
    onkeydown={onKey}
    onblur={() => (hover = -1)}
    onclick={() => hover >= 0 && onpick?.(rows[hover].key)}
    class:pickable={!!onpick}
  >
    {#each [0, 0.5, 1] as f (f)}
      <line x1="0" x2={W} y1={y(axisTop * f)} y2={y(axisTop * f)} class={f === 0 ? "base" : "grid"} vector-effect="non-scaling-stroke" />
    {/each}
    {#each rows as r, i (r.key)}
      {@const x = i * slot + (slot - barW) / 2}
      <g class:dim={hover >= 0 && hover !== i}>
        {#each segs(i) as sg (sg.s.id)}
          <path d={path(x, sg.top, barW, sg.h, sg.last)} fill={sg.s.color} />
        {/each}
      </g>
    {/each}
    {#if hover >= 0}
      <rect x={hover * slot} y={PAD_TOP} width={slot} height={plotH} class="hl" />
    {/if}
  </svg>
  {#if rows.length && totals[peak] > 0}
    <span class="peak" style:left="{((peak + 0.5) / rows.length) * 100}%" style:top="{Math.max(0, y(totals[peak]) - 16)}px">
      {format(totals[peak])}
    </span>
  {/if}
  {#if hover >= 0 && rows[hover]}
    <div class="viz-tip" style:left="{tipX}px" style:top="4px" role="status">
      <div class="tip-head">{rows[hover].label}</div>
      {#each series as s (s.id)}
        <div class="tip-row"><i class="key" style:background={s.color}></i><span>{s.label}</span><strong>{format(rows[hover].values[s.id] ?? 0)}</strong></div>
      {/each}
      {#if series.length > 1}
        <div class="tip-row"><i></i><span>{$t("llm.central.sessions.total")}</span><strong>{format(totals[hover])}</strong></div>
      {/if}
    </div>
  {/if}
</div>
<div class="ticks" aria-hidden="true">
  {#each rows as r, i (r.key)}
    <span style:width="{100 / rows.length}%">{i % every === 0 ? r.label : ""}</span>
  {/each}
</div>
<details class="viz-values">
  <summary>{$t("llm.central.sessions.values")}</summary>
  <div class="table-wrap">
    <table>
      <thead>
        <tr><th scope="col"></th>{#each series as s (s.id)}<th scope="col">{s.label}</th>{/each}{#if series.length > 1}<th scope="col">{$t("llm.central.sessions.total")}</th>{/if}</tr>
      </thead>
      <tbody>
        {#each rows as r, i (r.key)}
          <tr><td>{r.label}</td>{#each series as s (s.id)}<td>{format(r.values[s.id] ?? 0)}</td>{/each}{#if series.length > 1}<td>{format(totals[i])}</td>{/if}</tr>
        {/each}
      </tbody>
    </table>
  </div>
</details>

<style>
  .chart {
    position: relative;
    display: grid;
    grid-template-columns: auto 1fr;
    gap: 6px;
    height: var(--h);
  }
  .axis {
    display: flex;
    flex-direction: column;
    justify-content: space-between;
    padding-top: 12px;
    font-size: var(--text-caption);
    color: var(--text-dim);
    text-align: right;
    font-variant-numeric: tabular-nums;
    min-width: 34px;
  }
  svg {
    width: 100%;
    height: 100%;
    display: block;
    pointer-events: all;
    outline: none;
    border-radius: 4px;
  }
  svg:focus-visible {
    outline: var(--focus-ring);
    outline-offset: 2px;
  }
  svg.pickable {
    cursor: pointer;
  }
  .grid {
    stroke: var(--viz-grid);
    stroke-width: 1;
  }
  .base {
    stroke: var(--viz-base);
    stroke-width: 1;
  }
  g {
    transition: opacity var(--duration-fast) var(--ease-out);
  }
  g.dim {
    opacity: 0.55;
  }
  .hl {
    fill: var(--fill-1);
  }
  .peak {
    position: absolute;
    transform: translateX(calc(-50% + 20px));
    font-size: var(--text-caption);
    color: var(--text-muted);
    pointer-events: none;
    white-space: nowrap;
    font-variant-numeric: tabular-nums;
  }
  .ticks {
    display: flex;
    margin-left: 40px;
    font-size: var(--text-caption);
    color: var(--text-dim);
    overflow: hidden;
    white-space: nowrap;
  }
  .ticks span {
    overflow: visible;
    flex-shrink: 0;
  }
</style>

<script lang="ts">
  /**
   * Heatmap de atividade estilo calendário: semanas em colunas, segunda a
   * domingo em linhas, uma rampa de um só tom (o accent) de claro para forte
   * pelo `level` 0–4 que o núcleo calcula por quartis. Hover ou setas mostram
   * o dia; Enter chama `onpick`.
   */
  import { t } from "$lib/i18n";
  import { compact, isoDay, type HeatDay } from "$lib/central/sessions";
  import "./viz.css";

  interface Props {
    days: HeatDay[];
    from: string;
    to: string;
    onpick?: (date: string) => void;
  }

  let { days, from, to, onpick }: Props = $props();

  type Cell = { date: string; d: HeatDay | null; col: number; row: number };

  let byDate = $derived(new Map(days.map((d) => [d.date, d])));

  let cells = $derived.by<Cell[]>(() => {
    if (!from || !to) return [];
    const [fy, fm, fd] = from.split("-").map(Number);
    const [ty, tm, td] = to.split("-").map(Number);
    const start = new Date(fy, fm - 1, fd);
    const end = new Date(ty, tm - 1, td);
    const lead = (start.getDay() + 6) % 7; // 0 = segunda
    const out: Cell[] = [];
    const cur = new Date(start);
    let i = lead;
    let guard = 0;
    while (cur <= end && guard++ < 2000) {
      const date = isoDay(cur);
      out.push({ date, d: byDate.get(date) ?? null, col: Math.floor(i / 7), row: i % 7 });
      cur.setDate(cur.getDate() + 1);
      i++;
    }
    return out;
  });

  let cols = $derived(cells.length ? cells[cells.length - 1].col + 1 : 0);

  /** Rótulo no primeiro "segunda-feira" de cada mês, sem encostar no anterior. */
  let months = $derived.by(() => {
    const out: { col: number; label: string }[] = [];
    const done = new Set<string>();
    for (const c of cells) {
      if (c.row !== 0) continue;
      const m = c.date.slice(0, 7);
      if (done.has(m)) continue;
      done.add(m);
      if (out.length && c.col - out[out.length - 1].col < 3) continue;
      const [y, mo] = m.split("-").map(Number);
      out.push({ col: c.col, label: new Date(y, mo - 1, 1).toLocaleDateString(undefined, { month: "short" }) });
    }
    return out;
  });

  const CELL = 11;
  const GAP = 3;
  let focus = $state(-1);

  let current = $derived(focus >= 0 ? cells[focus] : null);

  function fill(level: number): string {
    if (level <= 0) return "var(--viz-empty)";
    const pct = [0, 28, 50, 74, 100][Math.min(4, level)];
    return `color-mix(in srgb, var(--viz-seq) ${pct}%, var(--viz-empty))`;
  }

  function onKey(e: KeyboardEvent) {
    if (!cells.length) return;
    let f = focus < 0 ? cells.length - 1 : focus;
    if (e.key === "ArrowRight") f += 7;
    else if (e.key === "ArrowLeft") f -= 7;
    else if (e.key === "ArrowDown") f += 1;
    else if (e.key === "ArrowUp") f -= 1;
    else if (e.key === "Home") f = 0;
    else if (e.key === "End") f = cells.length - 1;
    else if ((e.key === "Enter" || e.key === " ") && focus >= 0) {
      e.preventDefault();
      onpick?.(cells[focus].date);
      return;
    } else if (e.key === "Escape") {
      focus = -1;
      return;
    } else return;
    e.preventDefault();
    focus = Math.max(0, Math.min(cells.length - 1, f));
  }

  function longDate(date: string): string {
    const [y, m, d] = date.split("-").map(Number);
    return new Date(y, m - 1, d).toLocaleDateString(undefined, { weekday: "short", day: "numeric", month: "short", year: "numeric" });
  }
</script>

<div class="heat">
  <div class="scroller">
    <!-- svelte-ignore a11y_no_noninteractive_tabindex, a11y_no_noninteractive_element_interactions -->
    <svg
      width={cols * (CELL + GAP) + 24}
      height={7 * (CELL + GAP) + 16}
      role="img"
      aria-label={$t("llm.central.sessions.usage.heatmap")}
      tabindex="0"
      onkeydown={onKey}
      onblur={() => (focus = -1)}
      onpointerleave={() => (focus = -1)}
    >
      {#each months as m (m.col + m.label)}
        <text x={24 + m.col * (CELL + GAP)} y="9" class="lbl">{m.label}</text>
      {/each}
      {#each [0, 2, 4] as r (r)}
        <text x="0" y={16 + r * (CELL + GAP) + 9} class="lbl">{new Date(2024, 0, 1 + r).toLocaleDateString(undefined, { weekday: "narrow" })}</text>
      {/each}
      {#each cells as c, i (c.date)}
        <rect
          x={24 + c.col * (CELL + GAP)}
          y={16 + c.row * (CELL + GAP)}
          width={CELL}
          height={CELL}
          rx="2.5"
          fill={fill(c.d?.level ?? 0)}
          class:on={focus === i}
          class:pick={!!onpick}
          onpointerenter={() => (focus = i)}
          onclick={() => onpick?.(c.date)}
          role="presentation"
        />
      {/each}
    </svg>
  </div>
  <div class="foot">
    <span class="readout" role="status">
      {#if current}
        <strong>{longDate(current.date)}</strong> ·
        {current.d?.sessions ?? 0} {$t("llm.central.sessions.sessions_unit")} ·
        {current.d?.messages ?? 0} {$t("llm.central.sessions.messages_unit")} ·
        {compact(current.d?.tokens ?? 0)} tokens ·
        {current.d?.tool_calls ?? 0} {$t("llm.central.sessions.tool_calls_unit")}
      {:else}
        {$t("llm.central.sessions.usage.heatmap_hint")}
      {/if}
    </span>
    <span class="scale" aria-hidden="true">
      {$t("llm.central.sessions.less")}
      {#each [0, 1, 2, 3, 4] as l (l)}<i style:background={fill(l)}></i>{/each}
      {$t("llm.central.sessions.more")}
    </span>
  </div>
</div>

<style>
  .heat {
    display: flex;
    flex-direction: column;
    gap: 8px;
    min-width: 0;
  }
  .scroller {
    overflow-x: auto;
    padding-bottom: 2px;
    direction: rtl;
  }
  svg {
    direction: ltr;
    display: block;
    pointer-events: all;
    outline: none;
    border-radius: 4px;
  }
  svg:focus-visible {
    outline: var(--focus-ring);
    outline-offset: 2px;
  }
  rect {
    pointer-events: all;
  }
  rect.pick {
    cursor: pointer;
  }
  rect.on {
    stroke: var(--text);
    stroke-width: 1.5;
  }
  .lbl {
    font-size: 9px;
    fill: var(--text-dim);
  }
  .foot {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    justify-content: space-between;
    gap: 6px 12px;
    font-size: var(--text-xs);
    color: var(--text-dim);
    font-variant-numeric: tabular-nums;
  }
  .readout strong {
    color: var(--text);
    font-weight: 600;
  }
  .scale {
    display: inline-flex;
    align-items: center;
    gap: 3px;
  }
  .scale i {
    width: 10px;
    height: 10px;
    border-radius: 2px;
  }
</style>

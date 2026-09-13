<script lang="ts">
  import { t } from "$lib/i18n";
  import type { JournalSummary } from "$lib/notes-bridge";

  type Props = {
    journals: JournalSummary[];
    onPick?: (day: number) => void;
  };

  let { journals, onPick }: Props = $props();

  const WEEKS = 13;
  const DAYS_PER_WEEK = 7;

  function encodeDayDate(date: Date): number {
    return (
      date.getFullYear() * 10000 +
      (date.getMonth() + 1) * 100 +
      date.getDate()
    );
  }

  function decodeDay(d: number): Date {
    const y = Math.floor(d / 10000);
    const m = Math.floor((d % 10000) / 100);
    const day = d % 100;
    return new Date(y, m - 1, day);
  }

  function fmtDayBR(d: number): string {
    return decodeDay(d).toLocaleDateString("pt-BR", {
      weekday: "long",
      day: "2-digit",
      month: "long",
    });
  }

  type Cell = { day: number; count: number; level: 0 | 1 | 2 | 3 | 4; isFuture: boolean };

  const cells = $derived.by<Cell[]>(() => {
    const today = new Date();
    today.setHours(0, 0, 0, 0);
    const todayCode = encodeDayDate(today);
    const journalMap = new Map<number, number>();
    for (const j of journals) journalMap.set(j.journal_day, j.block_count);

    const max = Math.max(1, ...journals.map((j) => j.block_count));
    const levelFor = (count: number): 0 | 1 | 2 | 3 | 4 => {
      if (count <= 0) return 0;
      const pct = count / max;
      if (pct < 0.25) return 1;
      if (pct < 0.5) return 2;
      if (pct < 0.8) return 3;
      return 4;
    };

    const totalDays = WEEKS * DAYS_PER_WEEK;
    const start = new Date(today);
    start.setDate(today.getDate() - (totalDays - 1));
    const weekday = start.getDay();
    start.setDate(start.getDate() - weekday);

    const out: Cell[] = [];
    for (let i = 0; i < totalDays; i++) {
      const d = new Date(start);
      d.setDate(start.getDate() + i);
      const code = encodeDayDate(d);
      const count = journalMap.get(code) ?? 0;
      out.push({
        day: code,
        count,
        level: levelFor(count),
        isFuture: code > todayCode,
      });
    }
    return out;
  });

  const grid = $derived.by(() => {
    const cols: Cell[][] = [];
    for (let w = 0; w < WEEKS; w++) {
      const col: Cell[] = [];
      for (let d = 0; d < DAYS_PER_WEEK; d++) {
        const idx = w * DAYS_PER_WEEK + d;
        if (idx < cells.length) col.push(cells[idx]);
      }
      cols.push(col);
    }
    return cols;
  });

  let hoverCell = $state<Cell | null>(null);
  let hoverPos = $state({ x: 0, y: 0 });
</script>

<div class="heatmap-host">
  <header class="heatmap-head">
    <h3>Atividade — 90 dias</h3>
    <span class="legend">
      <span class="legend-label">menos</span>
      <span class="lvl lvl-0"></span>
      <span class="lvl lvl-1"></span>
      <span class="lvl lvl-2"></span>
      <span class="lvl lvl-3"></span>
      <span class="lvl lvl-4"></span>
      <span class="legend-label">mais</span>
    </span>
  </header>
  <div class="heatmap" role="img" aria-label={$t("study.notes.nb.activity_90")}>
    {#each grid as col, ci (ci)}
      <div class="col">
        {#each col as cell (cell.day)}
          <button
            type="button"
            class="cell lvl-{cell.level}"
            class:future={cell.isFuture}
            disabled={cell.isFuture}
            aria-label="{cell.count} blocos em {fmtDayBR(cell.day)}"
            onmouseenter={(e) => {
              hoverCell = cell;
              const r = (e.target as HTMLElement).getBoundingClientRect();
              hoverPos = { x: r.left + r.width / 2, y: r.top - 8 };
            }}
            onmouseleave={() => (hoverCell = null)}
            onclick={() => onPick?.(cell.day)}
          ></button>
        {/each}
      </div>
    {/each}
  </div>
  {#if hoverCell}
    <div
      class="tooltip"
      style:left={`${hoverPos.x}px`}
      style:top={`${hoverPos.y}px`}
      role="status"
    >
      {hoverCell.count} bloco{hoverCell.count === 1 ? "" : "s"} · {fmtDayBR(hoverCell.day)}
    </div>
  {/if}
</div>

<style>
  .heatmap-host {
    background: var(--surface);
    border: none;
    border-radius: var(--border-radius);
    padding: 12px;
  }
  .heatmap-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
    margin-bottom: 8px;
  }
  .heatmap-head h3 {
    margin: 0;
    font-size: 12px;
    font-weight: 600;
    color: var(--tertiary);
    text-transform: uppercase;
    letter-spacing: 0.05em;
  }
  .legend {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    font-size: 10px;
    color: var(--tertiary);
  }
  .legend-label {
    text-transform: lowercase;
  }
  .legend .lvl {
    width: 10px;
    height: 10px;
    border-radius: 2px;
  }
  .heatmap {
    display: flex;
    gap: 3px;
    overflow-x: auto;
  }
  .col {
    display: flex;
    flex-direction: column;
    gap: 3px;
  }
  .cell {
    width: 12px;
    height: 12px;
    border: 0;
    border-radius: 2px;
    cursor: pointer;
    background: var(--cell-bg);
  }
  .cell:disabled {
    cursor: not-allowed;
    opacity: 0.3;
  }
  .lvl-0 {
    --cell-bg: color-mix(in oklab, var(--input-border) 40%, transparent);
  }
  .lvl-1 {
    --cell-bg: color-mix(in oklab, var(--accent) 18%, transparent);
  }
  .lvl-2 {
    --cell-bg: color-mix(in oklab, var(--accent) 38%, transparent);
  }
  .lvl-3 {
    --cell-bg: color-mix(in oklab, var(--accent) 60%, transparent);
  }
  .lvl-4 {
    --cell-bg: var(--accent);
  }
  .tooltip {
    position: fixed;
    transform: translate(-50%, -100%);
    background: var(--surface);
    border: none;
    border-radius: var(--border-radius);
    padding: 4px 8px;
    font-size: 11px;
    color: var(--text);
    box-shadow: 0 4px 12px color-mix(in oklab, black 24%, transparent);
    pointer-events: none;
    z-index: 100;
    white-space: nowrap;
  }
</style>

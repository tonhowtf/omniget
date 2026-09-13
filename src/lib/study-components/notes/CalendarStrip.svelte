<script lang="ts">
  import { t } from "$lib/i18n";
  import type { JournalSummary } from "$lib/notes-bridge";

  type Props = {
    journals: JournalSummary[];
    currentDay: number | null;
    onPick: (day: number) => void;
    onShowMonth?: () => void;
  };

  let { journals, currentDay, onPick, onShowMonth }: Props = $props();

  let weekOffset = $state(0);

  function encodeDayDate(date: Date): number {
    return (
      date.getFullYear() * 10000 +
      (date.getMonth() + 1) * 100 +
      date.getDate()
    );
  }

  function decodeDay(d: number): { y: number; m: number; d: number } {
    return {
      y: Math.floor(d / 10000),
      m: Math.floor((d % 10000) / 100),
      d: d % 100,
    };
  }

  function todayCode(): number {
    return encodeDayDate(new Date());
  }

  const days = $derived.by(() => {
    const today = new Date();
    const start = new Date(today);
    start.setDate(today.getDate() - 6 + weekOffset * 14);
    const out: { day: number; date: Date; weekday: string; isToday: boolean; has: boolean; blockCount: number }[] = [];
    const today0 = todayCode();
    for (let i = 0; i < 14; i++) {
      const d = new Date(start);
      d.setDate(start.getDate() + i);
      const code = encodeDayDate(d);
      const j = journals.find((row) => row.journal_day === code);
      out.push({
        day: code,
        date: d,
        weekday: d.toLocaleDateString("pt-BR", { weekday: "narrow" }).toUpperCase(),
        isToday: code === today0,
        has: !!j,
        blockCount: j?.block_count ?? 0,
      });
    }
    return out;
  });

  function fmtDayShort(code: number): string {
    const { d } = decodeDay(code);
    return String(d).padStart(2, "0");
  }
</script>

<div class="calendar-strip" role="group" aria-label={$t("study.notes.nb.calendar_aria")}>
  <button
    type="button"
    class="nav-btn"
    aria-label="Semanas anteriores"
    onclick={() => (weekOffset -= 1)}
  >‹</button>

  <div class="days">
    {#each days as cell (cell.day)}
      <button
        type="button"
        class="day-cell"
        class:today={cell.isToday}
        class:has={cell.has}
        class:current={currentDay === cell.day}
        onclick={() => onPick(cell.day)}
        title={cell.date.toLocaleDateString("pt-BR")}
      >
        <span class="weekday">{cell.weekday}</span>
        <span class="day-num">{fmtDayShort(cell.day)}</span>
        {#if cell.has}
          <span class="dot" aria-hidden="true"></span>
        {/if}
      </button>
    {/each}
  </div>

  <button
    type="button"
    class="nav-btn"
    aria-label="Semanas seguintes"
    onclick={() => (weekOffset += 1)}
  >›</button>

  {#if onShowMonth}
    <button
      type="button"
      class="month-btn"
      onclick={onShowMonth}
      title={$t("study.notes.nb.view_full_month")}
    >Mês</button>
  {/if}
</div>

<style>
  .calendar-strip {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 8px;
    background: var(--surface);
    border: none;
    border-radius: var(--border-radius);
  }
  .nav-btn {
    flex: 0 0 auto;
    width: 28px;
    height: 28px;
    border: 0;
    border-radius: var(--border-radius);
    background: transparent;
    color: var(--secondary);
    font: inherit;
    font-size: 16px;
    cursor: pointer;
  }
  .nav-btn:hover {
    background: color-mix(in oklab, var(--accent) 8%, transparent);
    color: var(--text);
  }
  .days {
    display: grid;
    grid-template-columns: repeat(14, 1fr);
    gap: 2px;
    flex: 1 1 auto;
    overflow-x: auto;
  }
  .day-cell {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 2px;
    padding: 6px 2px;
    border-radius: var(--border-radius);
    border: 1px solid transparent;
    background: transparent;
    color: var(--secondary);
    font: inherit;
    cursor: pointer;
    position: relative;
    transition: background 120ms ease, border-color 120ms ease;
    min-width: 32px;
  }
  .day-cell:hover {
    background: color-mix(in oklab, var(--accent) 8%, transparent);
  }
  .day-cell.has {
    color: var(--text);
  }
  .day-cell.today {
    border-color: var(--accent);
    color: var(--accent);
  }
  .day-cell.current {
    background: color-mix(in oklab, var(--accent) 18%, transparent);
    color: var(--text);
  }
  .weekday {
    font-size: 9px;
    font-weight: 600;
    color: var(--tertiary);
    text-transform: uppercase;
    letter-spacing: 0.06em;
  }
  .day-cell.today .weekday {
    color: var(--accent);
  }
  .day-num {
    font-size: 13px;
    font-family: var(--font-mono, ui-monospace, monospace);
    font-weight: 600;
  }
  .dot {
    position: absolute;
    bottom: 4px;
    width: 4px;
    height: 4px;
    border-radius: 50%;
    background: var(--accent);
  }
  .month-btn {
    flex: 0 0 auto;
    padding: 4px 10px;
    border: none;
    border-radius: var(--border-radius);
    background: transparent;
    color: var(--secondary);
    font: inherit;
    font-size: 11px;
    cursor: pointer;
  }
  .month-btn:hover {
    border-color: var(--accent);
    color: var(--accent);
  }
</style>

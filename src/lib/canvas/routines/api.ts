// Canvas routines: typed wrappers over the `canvas_routine_*` commands and
// a plain-language description of a schedule.

import { invoke } from '@tauri-apps/api/core';

export const EVENT_ROUTINE = 'canvas://routine';
export const EVENT_NOTIFY = 'canvas://notify';

export type Schedule =
  | { kind: 'once'; at: string }
  | { kind: 'interval'; minutes: number; seconds?: number; startAt?: string }
  | { kind: 'daily'; time: string }
  | { kind: 'weekly'; days: number[]; time: string };

export type End = { kind: 'never' } | { kind: 'afterCount'; count: number } | { kind: 'onDate'; date: string };

export type RunCode = 'sentToTerminal' | 'terminalNotRunning' | 'reminder' | 'skippedByScript' | 'skippedWhileBusy' | 'failed';

export interface RunRecord {
  at: string;
  status: 'fired' | 'skipped' | 'failed' | 'reminded';
  /** Maestri's result code. */
  code?: RunCode;
  /** Seconds after its time it ran (one catch-up only). */
  lateSecs?: number;
  message?: string;
  steps: number;
  manual: boolean;
}

export interface Routine {
  id: string;
  name: string;
  workspaceId: string;
  floorId: string;
  nodeId?: string;
  prompt: string;
  schedule: Schedule;
  end: End;
  preRun?: string;
  skipIfBusy: boolean;
  notify: boolean;
  enabled: boolean;
  createdAt: string;
  createdBy?: string;
  lastRunAt?: string;
  nextRunAt?: string;
  runCount: number;
  history: RunRecord[];
}

export interface RoutineInput {
  id?: string;
  name: string;
  workspaceId: string;
  floorId?: string;
  nodeId?: string | null;
  prompt: string;
  schedule: Schedule;
  end?: End;
  preRun?: string | null;
  skipIfBusy?: boolean;
  notify?: boolean;
  enabled?: boolean;
}

export interface NotifyEvent {
  workspaceId: string;
  floorId?: string;
  nodeId?: string;
  name?: string;
  message: string;
  source: 'maestro' | 'routine';
}

export const listRoutines = (workspaceId?: string) =>
  invoke<Routine[]>('canvas_routine_list', { workspaceId: workspaceId ?? null });

export const saveRoutine = (input: RoutineInput) => invoke<Routine>('canvas_routine_save', { input });

export const setRoutineEnabled = (id: string, enabled: boolean) =>
  invoke<Routine>('canvas_routine_set_enabled', { id, enabled });

export const deleteRoutine = (id: string) => invoke<void>('canvas_routine_delete', { id });

export const runRoutineNow = (id: string) => invoke<Routine>('canvas_routine_run_now', { id });

export interface HistoryItem extends RunRecord {
  routineId: string;
  routineName: string;
  /** The routine left the list (a finished once). */
  removed: boolean;
}

export const routineHistory = (workspaceId: string, id?: string) =>
  invoke<HistoryItem[]>('canvas_routine_history', { workspaceId, id: id ?? null });

export const clearRoutineHistory = (workspaceId: string, id?: string) =>
  invoke<number>('canvas_routine_history_clear', { workspaceId, id: id ?? null });

/** Seconds between runs of an interval schedule. */
export function intervalSecs(s: Schedule): number {
  return s.kind === 'interval' ? (s.seconds ?? s.minutes * 60) : 0;
}

/**
 * The next `n` run times after `from` (the calendar the list shows). Same
 * rules as the Rust side: interval from `from`, daily/weekly at local time.
 */
export function projectRuns(s: Schedule, from: Date, n = 3): Date[] {
  const out: Date[] = [];
  if (s.kind === 'once') {
    const d = new Date(s.at);
    return d > from ? [d] : [];
  }
  if (s.kind === 'interval') {
    const step = intervalSecs(s) * 1000;
    if (step <= 0) return out;
    for (let i = 0; i < n; i++) out.push(new Date(from.getTime() + step * i));
    return out;
  }
  const [h, m] = s.time.split(':').map((x) => parseInt(x, 10));
  const day = new Date(from);
  for (let i = 0; i < 15 && out.length < n; i++) {
    const c = new Date(day.getFullYear(), day.getMonth(), day.getDate() + i, h || 0, m || 0);
    if (c < from) continue;
    if (s.kind === 'weekly' && !s.days.includes(c.getDay())) continue;
    out.push(c);
  }
  return out;
}

/** Same rule as Rust: `&&` alone on a line separates steps. */
export function splitSteps(prompt: string): string[] {
  const out: string[] = [];
  let cur: string[] = [];
  for (const line of prompt.split('\n')) {
    if (line.trim() === '&&') {
      out.push(cur.join('\n'));
      cur = [];
    } else cur.push(line);
  }
  out.push(cur.join('\n'));
  return out.map((s) => s.trim()).filter(Boolean);
}

type Tr = (key: string, vars?: Record<string, unknown>) => string;

const DAY_KEYS = ['sun', 'mon', 'tue', 'wed', 'thu', 'fri', 'sat'];
const DAY_NAMES = ['Sun', 'Mon', 'Tue', 'Wed', 'Thu', 'Fri', 'Sat'];

export function dayLabel(tr: Tr, d: number): string {
  return tr(`canvas.routines.day.${DAY_KEYS[d]}`, { default: DAY_NAMES[d] });
}

export function describeSchedule(tr: Tr, s: Schedule): string {
  switch (s.kind) {
    case 'once':
      return tr('canvas.routines.describe.once', { default: 'Once, {{when}}', when: new Date(s.at).toLocaleString() });
    case 'interval': {
      const secs = intervalSecs(s);
      const m = secs / 60;
      const every =
        secs % 60 !== 0 ? `${secs} s` : m % 1440 === 0 ? `${m / 1440} d` : m % 60 === 0 ? `${m / 60} h` : `${m} min`;
      return tr('canvas.routines.describe.interval', { default: 'Every {{every}}', every });
    }
    case 'daily':
      return tr('canvas.routines.describe.daily', { default: 'Every day at {{time}}', time: s.time });
    case 'weekly':
      return tr('canvas.routines.describe.weekly', {
        default: '{{days}} at {{time}}',
        days: s.days.map((d) => dayLabel(tr, d)).join(', '),
        time: s.time,
      });
  }
}

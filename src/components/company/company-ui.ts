import type { ChartNode, Staff } from '$lib/company/rpc';

export type Tone = 'green' | 'blue' | 'orange' | 'red' | 'yellow' | 'gray' | 'faint';

export function statusTone(status: string | null | undefined): Tone {
  switch (status) {
    case 'running':
    case 'in_progress':
    case 'active':
      return 'green';
    case 'succeeded':
    case 'done':
    case 'achieved':
    case 'completed':
      return 'blue';
    case 'queued':
    case 'todo':
    case 'in_review':
    case 'planned':
      return 'blue';
    case 'paused':
    case 'blocked':
      return 'orange';
    case 'pending_approval':
      return 'yellow';
    case 'error':
    case 'failed':
    case 'timed_out':
      return 'red';
    case 'terminated':
    case 'archived':
    case 'cancelled':
      return 'faint';
    default:
      return 'gray';
  }
}

export function initials(name: string): string {
  const parts = name.trim().split(/\s+/).filter(Boolean);
  if (parts.length === 0) return '?';
  const first = parts[0][0] ?? '';
  const last = parts.length > 1 ? parts[parts.length - 1][0] ?? '' : '';
  return (first + last).toUpperCase();
}

const HUES = ['--blue', '--green', '--orange', '--purple', '--teal', '--red'];

export function avatarColor(seed: string): string {
  let h = 0;
  for (let i = 0; i < seed.length; i++) h = (h * 31 + seed.charCodeAt(i)) | 0;
  return `var(${HUES[Math.abs(h) % HUES.length]})`;
}

type Translate = (key: string, vars?: Record<string, unknown>) => string;

export function relTime(ms: number | null | undefined, t: Translate, now = Date.now()): string {
  if (!ms) return t('company.common.never');
  const diff = ms - now;
  const abs = Math.abs(diff);
  const min = Math.round(abs / 60000);
  if (min < 1) return t('company.common.now');
  const [unit, count] = min < 60 ? ['minutes', min] : min < 60 * 48 ? ['hours', Math.round(min / 60)] : ['days', Math.round(min / 1440)];
  return t(diff >= 0 ? `company.common.in_${unit}` : `company.common.${unit}_ago`, { count });
}

export function duration(startMs: number | null | undefined, endMs: number | null | undefined): string {
  if (!startMs) return '—';
  const s = Math.max(0, Math.round(((endMs ?? Date.now()) - startMs) / 1000));
  if (s < 60) return `${s}s`;
  const m = Math.floor(s / 60);
  if (m < 60) return `${m}m ${s % 60}s`;
  return `${Math.floor(m / 60)}h ${m % 60}m`;
}

export function usd(value: number): string {
  if (!value) return '0.00';
  return value < 0.01 ? value.toFixed(4) : value.toFixed(2);
}

export function microsToUsd(micros: number): string {
  return usd(micros / 1_000_000);
}

export const CRON_PRESETS: { id: string; cron: string }[] = [
  { id: 'every_15', cron: '*/15 * * * *' },
  { id: 'hourly', cron: '0 * * * *' },
  { id: 'daily', cron: '0 9 * * *' },
  { id: 'weekdays', cron: '0 9 * * 1-5' },
  { id: 'weekly', cron: '0 9 * * 1' },
  { id: 'monthly', cron: '0 9 1 * *' },
];

export function presetOf(cron: string | null | undefined): string | null {
  if (!cron) return null;
  return CRON_PRESETS.find((p) => p.cron === cron.trim())?.id ?? null;
}

export function flattenChart(nodes: ChartNode[]): Staff[] {
  const out: Staff[] = [];
  const walk = (n: ChartNode) => {
    out.push(n.staff);
    n.reports.forEach(walk);
  };
  nodes.forEach(walk);
  return out;
}

export function chartOf(staff: Staff[], includeTerminated = false): ChartNode[] {
  const list = includeTerminated ? staff : staff.filter((s) => s.status !== 'terminated');
  const ids = new Set(list.map((s) => s.id));
  const byParent = new Map<string | null, Staff[]>();
  for (const s of list) {
    const parent = s.reports_to && ids.has(s.reports_to) ? s.reports_to : null;
    byParent.set(parent, [...(byParent.get(parent) ?? []), s]);
  }
  const build = (s: Staff, seen: Set<string>): ChartNode => {
    seen.add(s.id);
    return { staff: s, reports: (byParent.get(s.id) ?? []).filter((c) => !seen.has(c.id)).map((c) => build(c, seen)) };
  };
  const seen = new Set<string>();
  return (byParent.get(null) ?? []).map((s) => build(s, seen));
}

export function downloadText(name: string, text: string, type = 'application/json'): void {
  const url = URL.createObjectURL(new Blob([text], { type }));
  const a = document.createElement('a');
  a.href = url;
  a.download = name;
  a.click();
  setTimeout(() => URL.revokeObjectURL(url), 1000);
}

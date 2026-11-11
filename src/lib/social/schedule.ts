export type ScheduleStatus = "draft" | "scheduled" | "sending" | "sent" | "failed" | "missed" | "cancelled";
export type ScheduleKind = "post" | "thread" | "repost" | "reply" | "quote";

export type ScheduleItem = {
  id: number;
  platform: string;
  kind: ScheduleKind;
  status: ScheduleStatus;
  text: string | null;
  thread: string[];
  reply_to: string | null;
  quote: string | null;
  repost: string | null;
  media: string[];
  at: string | null;
  sent_at: string | null;
  post_id: string | null;
  post_url: string | null;
  parts_total: number;
  parts_sent: number;
  error: string | null;
  created_by: string;
  updated_at: string | null;
};

export type Slot = { weekday: string; time: string };

export type Feed = {
  id: number;
  url: string;
  template: string;
  interval_minutes: number;
  monthly_cap: number;
  posted_this_month: number;
  skipped_over_cap: number;
  primed: boolean;
  last_checked_at: string | null;
  last_error: string | null;
};

export type ScheduleList = {
  items: ScheduleItem[];
  total: number;
  returned: number;
  truncated: boolean;
  counts: Partial<Record<ScheduleStatus, number>>;
  slots: Slot[];
  next_slot: string | null;
  feeds: Feed[];
  settings: { grace_minutes: number; long_posts: "auto" | "on" | "off" };
  limits: { text: number; thread_parts: number; images: number };
  supported: boolean;
};

export type ImportReport = {
  dry_run: boolean;
  rows: number;
  valid: number;
  scheduled: number;
  rejected_count: number;
  rejected: { row: number; code: string; reason: string }[];
  preview: { row: number; at: string | null; kind: ScheduleKind; text: string | null; repost: string | null }[];
  truncated: boolean;
};

export const WEEKDAYS = ["mon", "tue", "wed", "thu", "fri", "sat", "sun"] as const;
export const SHORT_LIMIT = 280;
export const LONG_LIMIT = 4000;

const URL_RE =
  /https?:\/\/[^\s<>"']+|\b(?:[a-z0-9-]+\.)+(?:com|org|net|io|co|dev|app|me|tv|ai|br|uk|de|fr|es|it|jp|ru|in|info|xyz|gg|ly|to|edu|gov|us|ca|au|nl|ch|se|pt|mx|ar)\b(?:\/[^\s<>"']*)?/gi;

const light = (c: number) => c <= 4351 || (c >= 8192 && c <= 8205) || (c >= 8208 && c <= 8223) || (c >= 8242 && c <= 8247);
const emojiBase = (c: number) => (c >= 0x2190 && c <= 0x2bff) || (c >= 0x1f000 && c <= 0x1faff);
const regional = (c: number) => c >= 0x1f1e6 && c <= 0x1f1ff;
const modifier = (c: number) => c === 0xfe0f || c === 0x20e3 || (c >= 0x1f3fb && c <= 0x1f3ff) || (c >= 0xe0020 && c <= 0xe007f);

function plainWeight(s: string): number {
  const cs = Array.from(s, (ch) => ch.codePointAt(0) ?? 0);
  let i = 0;
  let w = 0;
  while (i < cs.length) {
    const c = cs[i];
    const keyChar = (c >= 48 && c <= 57) || c === 35 || c === 42;
    if (keyChar && (cs[i + 1] === 0x20e3 || (cs[i + 1] === 0xfe0f && cs[i + 2] === 0x20e3))) {
      w += 2;
      i += 1;
      while (i < cs.length && modifier(cs[i])) i += 1;
      continue;
    }
    if (!emojiBase(c)) {
      w += light(c) ? 1 : 2;
      i += 1;
      continue;
    }
    w += 2;
    i += 1;
    if (regional(c) && i < cs.length && regional(cs[i])) {
      i += 1;
      continue;
    }
    for (;;) {
      if (i < cs.length && modifier(cs[i])) i += 1;
      else if (cs[i] === 0x200d && i + 1 < cs.length && emojiBase(cs[i + 1])) i += 2;
      else break;
    }
  }
  return w;
}

export function weightedLength(text: string): number {
  const t = text.normalize("NFC");
  let total = 0;
  let last = 0;
  for (const m of t.matchAll(URL_RE)) {
    const start = m.index ?? 0;
    const url = m[0].replace(/[.,;:!?)\]']+$/, "");
    total += plainWeight(t.slice(last, start)) + 23;
    last = start + url.length;
  }
  return total + plainWeight(t.slice(last));
}

export function textLimit(mode: "auto" | "on" | "off", verified: boolean): number {
  return mode === "on" || (mode === "auto" && verified) ? LONG_LIMIT : SHORT_LIMIT;
}

const pad = (n: number) => String(Math.abs(n)).padStart(2, "0");

export function localInputToIso(value: string): string | null {
  const d = new Date(value);
  if (!value || Number.isNaN(d.getTime())) return null;
  const off = -d.getTimezoneOffset();
  const sign = off >= 0 ? "+" : "-";
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}T${pad(d.getHours())}:${pad(d.getMinutes())}:00${sign}${pad(Math.trunc(off / 60))}:${pad(off % 60)}`;
}

export function isoToLocalInput(iso: string | null): string {
  if (!iso) return "";
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return "";
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}T${pad(d.getHours())}:${pad(d.getMinutes())}`;
}

export function dayKey(iso: string): string {
  const d = new Date(iso);
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}`;
}

export function groupByDay<T extends { at: string | null }>(items: T[]): { day: string; items: T[] }[] {
  const out: { day: string; items: T[] }[] = [];
  for (const it of items) {
    if (!it.at) continue;
    const day = dayKey(it.at);
    const last = out[out.length - 1];
    if (last && last.day === day) last.items.push(it);
    else out.push({ day, items: [it] });
  }
  return out;
}

export type Rejection = { code: string; message: string; next_step: string };

export function rejection(e: unknown): Rejection {
  const raw = typeof e === "string" ? e : e && typeof e === "object" && "message" in e ? String((e as { message: unknown }).message) : String(e);
  const i = raw.indexOf(":");
  const code = i > 0 && /^[A-Z_0-9]+$/.test(raw.slice(0, i)) ? raw.slice(0, i) : "UNKNOWN";
  const rest = code === "UNKNOWN" ? raw : raw.slice(i + 1).trim();
  try {
    const v = JSON.parse(rest) as { message?: unknown; next_step?: unknown };
    if (v && typeof v === "object" && typeof v.message === "string") return { code, message: v.message, next_step: typeof v.next_step === "string" ? v.next_step : "" };
  } catch {}
  return { code, message: rest, next_step: "" };
}

export function sortSlots(slots: Slot[]): Slot[] {
  const order = (s: Slot) => WEEKDAYS.indexOf(s.weekday as (typeof WEEKDAYS)[number]) * 1440 + Number(s.time.slice(0, 2)) * 60 + Number(s.time.slice(3, 5));
  return [...slots].sort((a, b) => order(a) - order(b));
}

export function fileName(path: string): string {
  return path.split(/[\\/]/).pop() ?? path;
}

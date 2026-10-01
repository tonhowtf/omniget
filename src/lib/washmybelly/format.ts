const UNITS = ["B", "KB", "MB", "GB", "TB"];

export function formatBytes(n: number | null | undefined): string {
  if (!n || n < 0 || !Number.isFinite(n)) return "0 B";
  let v = n;
  let i = 0;
  while (v >= 1000 && i < UNITS.length - 1) {
    v /= 1000;
    i++;
  }
  return `${v >= 100 || i === 0 ? Math.round(v) : v.toFixed(1)} ${UNITS[i]}`;
}

export function splitBytes(n: number | null | undefined): { value: string; unit: string } {
  const [value, unit] = formatBytes(n).split(" ");
  return { value, unit };
}

export function tildify(path: string): string {
  const m = path.match(/^(\/Users\/[^/]+|\/home\/[^/]+|[A-Za-z]:\\Users\\[^\\]+)(?=$|[\\/])/);
  return m ? `~${path.slice(m[1].length)}` : path;
}

export function shortenPath(path: string, max = 56): string {
  const p = tildify(path);
  if (p.length <= max) return p;
  const sep = p.includes("\\") && !p.includes("/") ? "\\" : "/";
  const parts = p.split(sep).filter((s, i) => s !== "" || i === 0);
  const headN = parts[0] === "" || parts[0] === "~" || /^[A-Za-z]:$/.test(parts[0]) ? 2 : 1;
  if (parts.length <= headN + 1) return middle(p, max);
  const head = parts.slice(0, headN);
  const tail = [parts[parts.length - 1]];
  for (let i = parts.length - 2; i >= headN; i--) {
    if ([...head, "…", parts[i], ...tail].join(sep).length > max) break;
    tail.unshift(parts[i]);
  }
  const out = [...head, "…", ...tail].join(sep);
  return out.length <= max ? out : middle(out, max);
}

export function middle(s: string, max: number): string {
  if (s.length <= max) return s;
  if (max <= 1) return "…";
  const keep = max - 1;
  const left = Math.ceil(keep / 2);
  const right = Math.floor(keep / 2);
  return `${s.slice(0, left)}…${s.slice(s.length - right)}`;
}

function fromSecs(ts: number): Date {
  return new Date(ts * 1000);
}

export type DayBucket = "today" | "yesterday" | "earlier";

function startOfDay(d: Date): number {
  return new Date(d.getFullYear(), d.getMonth(), d.getDate()).getTime();
}

export function dayBucket(ts: number, now: Date = new Date()): { bucket: DayBucket; key: string } {
  const d = fromSecs(ts);
  const diff = Math.round((startOfDay(now) - startOfDay(d)) / 86_400_000);
  const key = `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, "0")}-${String(d.getDate()).padStart(2, "0")}`;
  if (diff <= 0) return { bucket: "today", key };
  if (diff === 1) return { bucket: "yesterday", key };
  return { bucket: "earlier", key };
}

export function formatTime(ts: number, locale?: string, withDate = false): string {
  const d = fromSecs(ts);
  const opts: Intl.DateTimeFormatOptions = withDate
    ? { month: "short", day: "numeric", hour: "2-digit", minute: "2-digit" }
    : { hour: "2-digit", minute: "2-digit" };
  try {
    return new Intl.DateTimeFormat(locale, opts).format(d);
  } catch {
    return new Intl.DateTimeFormat(undefined, opts).format(d);
  }
}

export function formatDay(ts: number, locale?: string, now: Date = new Date()): string {
  const d = fromSecs(ts);
  const opts: Intl.DateTimeFormatOptions = { month: "long", day: "numeric" };
  if (d.getFullYear() !== now.getFullYear()) opts.year = "numeric";
  try {
    return new Intl.DateTimeFormat(locale, opts).format(d);
  } catch {
    return new Intl.DateTimeFormat(undefined, opts).format(d);
  }
}

export function formatCount(n: number, locale?: string): string {
  try {
    return new Intl.NumberFormat(locale).format(n);
  } catch {
    return String(n);
  }
}

export function formatPercent(v: number | null | undefined, digits = 0): string {
  if (v == null || !Number.isFinite(v)) return "–";
  const pct = v <= 1 ? v * 100 : v;
  return `${pct.toFixed(digits)}%`;
}

const KNOWN_CODES = [
  "CHANGED",
  "PROTECTED",
  "EXPIRED",
  "PLAN_EXPIRED",
  "PLAN_ALREADY_EXECUTED",
  "PLAN_DENIED",
  "PLAN_RUNNING",
  "NO_ELEVATION",
  "NO_HELPER",
  "CANCELLED",
  "TRASH_MANUAL",
  "ALREADY_RESTORED",
  "UNKNOWN_ITEMS",
  "RECEIPT_NOT_FOUND",
  "NOT_READY",
] as const;

type ExplainedDetail = { code: string | null; key: string | null; text: string };

export function explainDetail(detail: string | null | undefined): ExplainedDetail {
  const s = String(detail ?? "").trim();
  const m = s.match(/^([A-Z][A-Z_]{3,}):\s*(.*)$/s);
  if (!m) return { code: null, key: null, text: s };
  const code = m[1];
  const known = (KNOWN_CODES as readonly string[]).includes(code);
  return { code, key: known ? `washmybelly.ui.reason.${code.toLowerCase()}` : null, text: m[2] };
}

export function errorText(e: unknown): string {
  if (typeof e === "string") return e;
  if (e && typeof e === "object" && "message" in e) return String((e as { message: unknown }).message);
  return String(e);
}

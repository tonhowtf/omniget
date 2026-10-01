/// Types and pure helpers of the Energy module (`wmb_power_status`,
/// `wmb_power_hold`, `wmb_battery_health`).

export type SleepBlocker = { process: string; pid?: number; kind: string; reason: string; display: boolean };

export type PowerAction = {
  id: string;
  label: string;
  detail?: string | null;
  admin: boolean;
  risk: string;
  action: "set_scheme" | "ultimate" | "lowpowermode" | "cpu_epp" | "cpu_governor" | "charge_limit" | "resolve_conflict" | string;
  args: Record<string, unknown>;
};

export type HoldState = {
  active: boolean;
  until?: number | null;
  started?: number | null;
  reason?: string | null;
  auto_holders: string[];
  auto_enabled: boolean;
  backend: string;
  error?: string | null;
};

export type Conflict = { a: string; b: string; keep: string; disable: string[]; action: string };
export type Scheme = { guid: string; name: string; active: boolean };
type ChargeLimit = { battery: string; end: number; start?: number };
type CpuKnobs = {
  driver?: string | null;
  governor?: string | null;
  governors: string[];
  epp?: string | null;
  epps: string[];
  turbo?: boolean | null;
  platform_profile?: string | null;
  platform_profiles: string[];
  cpus: number;
};

export type PowerStatus = {
  os: "macos" | "windows" | "linux" | string;
  profile?: {
    current?: string | null;
    available?: string[];
    holds?: { profile: string; reason: string; app: string }[];
    can_switch?: boolean;
    overlay?: { ac?: string | null; dc?: string | null };
  } | null;
  manager?: string | null;
  managers_running?: string[];
  conflicts?: Conflict[];
  suggestion?: string | null;
  cpu?: CpuKnobs;
  charge_limits?: ChargeLimit[];
  low_power_mode?: boolean | null;
  power_mode?: number | null;
  plans?: Scheme[];
  active_plan?: Scheme | null;
  ultimate?: { present: boolean };
  sleep_blockers?: SleepBlocker[];
  sleep_blockers_need_admin?: boolean;
  sleep_prevented_by?: string[];
  reasons?: Record<string, string>;
  actions?: PowerAction[];
  plan_kind?: string;
  hold?: HoldState;
};

export type Battery = {
  present: boolean;
  cycles?: number;
  health_percent?: number;
  design_capacity?: number;
  full_charge_capacity?: number;
  capacity_unit?: "mAh" | "mWh" | string;
  charge_percent?: number;
  temperature_c?: number;
  state: "charging" | "discharging" | "full" | "not_charging" | "unknown" | string;
  on_ac?: boolean;
  voltage_v?: number;
  power_w?: number;
  time_remaining_min?: number;
  name?: string;
  source: string;
};

/// The hold lengths the screen offers, in minutes.
export const HOLD_CHOICES = [30, 60, 120] as const;

/// Profiles of the power-profiles API, in the order the desktop shows them.
const PROFILE_ORDER = ["power-saver", "balanced", "performance"] as const;

export function remainingSecs(until: number | null | undefined, nowMs: number): number {
  if (!until) return 0;
  return Math.max(0, Math.round(until - nowMs / 1000));
}

/// `4:05`, `1:02:09`.
export function formatCountdown(secs: number): string {
  const s = Math.max(0, Math.floor(secs));
  const h = Math.floor(s / 3600);
  const m = Math.floor((s % 3600) / 60);
  const r = s % 60;
  const pad = (n: number) => String(n).padStart(2, "0");
  return h > 0 ? `${h}:${pad(m)}:${pad(r)}` : `${m}:${pad(r)}`;
}

/// Share of the manual hold already gone, 0..1 (for the pill's ring).
export function holdProgress(h: HoldState | null | undefined, nowMs: number): number {
  if (!h?.until || h.started == null || h.until <= h.started) return 0;
  const span = h.until - h.started;
  return Math.min(1, Math.max(0, (nowMs / 1000 - h.started) / span));
}

export function healthTone(pct: number | null | undefined): "good" | "warn" | "bad" | "default" {
  if (pct == null) return "default";
  if (pct >= 80) return "good";
  if (pct >= 60) return "warn";
  return "bad";
}

/// Full charge against design, as widths for the two bars of the card.
export function capacityBars(b: Battery | null | undefined): { design: number; full: number; unit: string; lost: number } | null {
  if (!b?.design_capacity || !b.full_charge_capacity) return null;
  const design = b.design_capacity;
  const full = Math.min(b.full_charge_capacity, design);
  return { design, full, unit: b.capacity_unit ?? "", lost: Math.max(0, Math.round((1 - full / design) * 100)) };
}

/// The segmented options of the Linux profile switch.
export function profileOptions(s: PowerStatus | null | undefined): string[] {
  const avail = s?.profile?.available ?? [];
  if (!s?.profile?.can_switch) return [];
  return PROFILE_ORDER.filter((p) => avail.includes(p));
}

export function actionsOf(s: PowerStatus | null | undefined, action: string): PowerAction[] {
  return (s?.actions ?? []).filter((a) => a.action === action);
}

/// Charge limit choices for one battery, as `{percent, id}` sorted.
export function chargeChoices(s: PowerStatus | null | undefined, battery: string): { percent: number; id: string }[] {
  return actionsOf(s, "charge_limit")
    .filter((a) => a.args.battery === battery)
    .map((a) => ({ percent: Number(a.args.percent), id: a.id }))
    .sort((x, y) => x.percent - y.percent);
}

/// One line per process: several assertions of one app collapse, the
/// reasons kept in order.
export function groupBlockers(list: SleepBlocker[] | null | undefined): { process: string; count: number; display: boolean; reasons: string[] }[] {
  const out = new Map<string, { process: string; count: number; display: boolean; reasons: string[] }>();
  for (const b of list ?? []) {
    const key = b.process;
    const cur = out.get(key) ?? { process: b.process, count: 0, display: false, reasons: [] };
    cur.count++;
    cur.display ||= b.display;
    if (b.reason && !cur.reasons.includes(b.reason)) cur.reasons.push(b.reason);
    out.set(key, cur);
  }
  return [...out.values()];
}

/// The i18n key suffix of a manager id (`power-profiles-daemon` → `ppd`).
export function managerKey(id: string | null | undefined): string {
  switch (id) {
    case "power-profiles-daemon":
      return "ppd";
    case "tlp-pd":
    case "tlp":
      return "tlp";
    case "tuned-ppd":
    case "tuned":
      return "tuned";
    case "auto-cpufreq":
      return "auto_cpufreq";
    case "pmset":
      return "pmset";
    case "windows":
      return "windows";
    default:
      return "none";
  }
}

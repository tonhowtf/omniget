export type SecurityClass =
  | "apple"
  | "app_store"
  | "notarized"
  | "signed"
  | "adhoc"
  | "unsigned"
  | "invalid"
  | "missing"
  | "unknown";

type PersistenceKind =
  | "launch_agent"
  | "launch_daemon"
  | "background_task"
  | "privileged_helper"
  | "profile"
  | "system_extension"
  | "network_extension"
  | "kernel_extension"
  | "cron";

export type PersistenceItem = {
  id: string;
  kind: PersistenceKind | string;
  scope: "user" | "system" | string;
  name: string;
  path?: string | null;
  program?: string | null;
  script?: string | null;
  command?: string | null;
  checked?: string | null;
  exists: boolean;
  class?: SecurityClass | null;
  summary: string;
  signer?: string | null;
  team_id?: string | null;
  notarized?: boolean | null;
  hardened_runtime?: boolean;
  signed_at?: string | null;
  invalid_reason?: string | null;
  quarantine?: { agent?: string | null; at?: number | null; approved: boolean } | null;
  installed?: number | null;
  state?: string | null;
  third_party: boolean;
  notes: string[];
  xprotect_matches: string[];
  startup_id?: string | null;
  detail?: string | null;
};

export type PersistenceList = {
  items: PersistenceItem[];
  count: number;
  third_party: number;
  by_kind: Record<string, number>;
  by_class: Record<string, number>;
  notes: string[];
  xprotect_yara: { ran: boolean; reason?: string; rules?: string };
  plan_id: null;
};

export const KIND_ORDER: string[] = [
  "launch_agent",
  "launch_daemon",
  "background_task",
  "privileged_helper",
  "system_extension",
  "network_extension",
  "kernel_extension",
  "profile",
  "cron",
];

export type Tone = "good" | "neutral" | "warn" | "muted";

export function toneOf(c: SecurityClass | null | undefined): Tone {
  switch (c) {
    case "apple":
    case "app_store":
    case "notarized":
      return "good";
    case "signed":
    case "adhoc":
      return "neutral";
    case "unsigned":
    case "invalid":
    case "missing":
      return "warn";
    default:
      return "muted";
  }
}

export function classKey(c: SecurityClass | null | undefined): string {
  return `washmybelly.security.class.${c ?? "unknown"}`;
}

export function kindKey(kind: string): string {
  return `washmybelly.security.kind.${KIND_ORDER.includes(kind) ? kind : "other"}`;
}

export function needsLook(it: PersistenceItem): boolean {
  return toneOf(it.class) === "warn" || it.xprotect_matches.length > 0;
}

export function filterItems(items: PersistenceItem[], opts: { thirdPartyOnly: boolean; query: string; lookOnly?: boolean }): PersistenceItem[] {
  const q = opts.query.trim().toLowerCase();
  return items.filter((it) => {
    if (opts.thirdPartyOnly && !it.third_party) return false;
    if (opts.lookOnly && !needsLook(it)) return false;
    if (!q) return true;
    return [it.name, it.path, it.program, it.script, it.signer, it.team_id, it.detail]
      .filter(Boolean)
      .some((s) => (s as string).toLowerCase().includes(q));
  });
}

export type Group = { kind: string; items: PersistenceItem[]; look: number };

export function groupItems(items: PersistenceItem[]): Group[] {
  const map = new Map<string, PersistenceItem[]>();
  for (const it of items) {
    const k = KIND_ORDER.includes(it.kind) ? it.kind : "other";
    const list = map.get(k) ?? [];
    list.push(it);
    map.set(k, list);
  }
  const order = [...KIND_ORDER, "other"];
  return order
    .filter((k) => map.has(k))
    .map((k) => {
      const list = map.get(k)!;
      list.sort((a, b) => Number(needsLook(b)) - Number(needsLook(a)) || a.name.localeCompare(b.name));
      return { kind: k, items: list, look: list.filter(needsLook).length };
    });
}

export function noteCodes(notes: string[] | null | undefined): string[] {
  const known = new Set([
    "btm_unavailable",
    "profiles_device_need_admin",
    "profiles_unavailable",
    "yara_unavailable",
    "system_extensions_unavailable",
    "kexts_unavailable",
  ]);
  const out: string[] = [];
  for (const n of notes ?? []) {
    const code = n.split(":")[0].trim();
    if (known.has(code) && !out.includes(code)) out.push(code);
  }
  return out;
}

export function revealPath(it: PersistenceItem): string | null {
  for (const p of [it.checked, it.path, it.program]) {
    if (p && p.startsWith("/") && (it.exists || p !== it.checked)) return p;
  }
  return null;
}

export function factTags(it: PersistenceItem): string[] {
  const order = ["quarantined", "runs_script", "inline_command", "in_temp_folder", "hidden_folder", "removal_disallowed"];
  return order.filter((n) => it.notes.includes(n));
}

export function stateKey(state: string | null | undefined): string | null {
  if (!state) return null;
  if (state === "enabled" || state === "disabled" || state === "loaded" || state === "not_loaded") {
    return `washmybelly.security.state.${state}`;
  }
  return null;
}

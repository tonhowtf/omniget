/// Trackers tab: the hosts file block (`hosts.status`, `hosts.preview`,
/// `hosts.plan` screen actions).

type HostsList = { id: string; name: string; license: string; source_url: string; domains: number };

export type HostsPrefs = { lists: string[]; whitelist: string[]; extra: string[] };

export type HostsStatus = {
  path: string;
  exists: boolean;
  writable: boolean;
  bytes: number;
  block_present: boolean;
  blocked: number;
  updated_at?: string | null;
  user_lines: number;
  user_preview: string[];
  foreign_lines: number;
  lists: HostsList[];
  prefs: HostsPrefs;
  flush_commands: string[];
};

export type HostsPreview = {
  unchanged: boolean;
  domains: number;
  blocked_before: number;
  added: number;
  removed: number;
  skipped_whitelist: number;
  bytes: number;
  preview: string[];
  preview_hidden: number;
};

export type HostsPlan = HostsPreview & {
  plan_id: string | null;
  item_ids: string[];
  admin?: boolean;
  staged?: string;
  commands?: string[];
};

/// Same rules as the core's `normalize_domain`, so the screen can refuse a
/// typo before the backend drops it silently.
function normalizeDomain(raw: string): string | null {
  let d = raw.trim().toLowerCase().replace(/^https?:\/\//, "").replace(/\/.*$/, "").replace(/\.$/, "");
  if (d.startsWith("*.")) d = d.slice(2);
  if (d.length < 4 || d.length > 253 || !d.includes(".")) return null;
  if (!/^[a-z0-9._-]+$/.test(d)) return null;
  if (d.startsWith(".") || d.startsWith("-") || d.endsWith("-")) return null;
  if (d.split(".").some((l) => l.length === 0 || l.length > 63)) return null;
  if (/^\d+\.\d+\.\d+\.\d+$/.test(d)) return null;
  return d;
}

/// Split free text (spaces, commas, new lines) into valid domains; returns
/// what was accepted and what was not.
function parseDomains(text: string): { ok: string[]; bad: string[] } {
  const ok: string[] = [];
  const bad: string[] = [];
  for (const tok of text.split(/[\s,;]+/)) {
    if (!tok) continue;
    const d = normalizeDomain(tok);
    if (d) {
      if (!ok.includes(d)) ok.push(d);
    } else bad.push(tok);
  }
  return { ok, bad };
}

export function addDomains(list: readonly string[], text: string): { list: string[]; bad: string[] } {
  const { ok, bad } = parseDomains(text);
  const next = [...list];
  for (const d of ok) if (!next.includes(d)) next.push(d);
  return { list: next.sort(), bad };
}

export function removeDomain(list: readonly string[], d: string): string[] {
  return list.filter((x) => x !== d);
}

export function toggleList(lists: readonly string[], id: string, on: boolean): string[] {
  const set = new Set(lists);
  if (on) set.add(id);
  else set.delete(id);
  return [...set];
}

/// Lines of the preview that belong to the OmniGet block (to tint them).
export function blockLineFlags(lines: readonly string[]): boolean[] {
  let inside = false;
  return lines.map((l) => {
    const t = l.trim();
    if (t === "# >>> OmniGet block list >>>") {
      inside = true;
      return true;
    }
    if (t === "# <<< OmniGet block list <<<") {
      inside = false;
      return true;
    }
    return inside;
  });
}

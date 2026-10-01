/// Pure logic behind the Uninstall screen: sources, filters, sorting, "last
/// opened", and how the leftovers of one app are split, grouped and ticked.
/// No Svelte and no Tauri here, so all of it runs under vitest.

import type { Risk } from "./api";

// ── Contract (`wmb_apps_list`, `wmb_app_leftovers`) ─────────────────

/// Mirrors `core/washmybelly/apps/mod.rs::App`.
export type App = {
  id: string;
  name: string;
  version?: string | null;
  publisher?: string | null;
  source: string;
  path?: string | null;
  bytes?: number | null;
  native_id?: string | null;
  /// Unix seconds.
  installed?: number | null;
  last_used?: number | null;
  removable: boolean;
  reason?: string | null;
  needs_admin?: boolean;
  extra?: Record<string, unknown> | null;
};

export type AppsList = { apps: App[]; total: number };

type Confidence = "high" | "medium" | "low";

/// Mirrors `apps/mod.rs::Leftover`.
type Leftover = {
  path: string;
  bytes: number;
  confidence: Confidence | string;
  evidence: string;
  kind: string;
};

type LeftoverItemPreview = {
  id: string;
  label: string;
  detail?: string | null;
  group: string;
  bytes: number;
  risk: Risk;
  admin: boolean;
  /// Left out of the default plan (app open, needs a password, weak match).
  unticked?: boolean;
  /// Why it is unticked (`open`, `admin`, `medium`, `low`, `alternative`).
  why?: string;
};

export type LeftoversResponse = {
  app: App;
  items: LeftoverItemPreview[];
  total_bytes: number;
  plan_id: string | null;
  extra?: {
    leftovers?: Leftover[];
    notes?: string[];
    cask?: string | null;
    running?: boolean | null;
    [k: string]: unknown;
  } | null;
};

// ── Sources ──────────────────────────────────────────────────────────

export type SourceInfo = {
  /// i18n key of the chip / tag label.
  key: string;
  /// English label used when the key is missing (brand names never change).
  fallback: string;
  /// Tile gradient (Apple system palette).
  from: string;
  to: string;
};

const SOURCES: Record<string, Omit<SourceInfo, "key">> = {
  app_bundle: { fallback: "App", from: "#5AA9FF", to: "#1E6FE8" },
  homebrew_cask: { fallback: "Homebrew", from: "#FFB340", to: "#E08600" },
  homebrew_formula: { fallback: "Homebrew formula", from: "#FFC766", to: "#C98A1C" },
  pkg: { fallback: ".pkg", from: "#A3A3A8", to: "#6F6F75" },
  msi: { fallback: "MSI", from: "#5AC8FA", to: "#0A84FF" },
  registry: { fallback: "Installer", from: "#8E8E93", to: "#5C5C60" },
  winget: { fallback: "winget", from: "#64D2FF", to: "#0071C5" },
  appx: { fallback: "Store", from: "#7D7AFF", to: "#4B45D6" },
  steam: { fallback: "Steam", from: "#4C6B8A", to: "#1B2838" },
  scoop: { fallback: "Scoop", from: "#67D27E", to: "#2F9E52" },
  apt: { fallback: "apt", from: "#FF8A65", to: "#D84315" },
  dnf: { fallback: "dnf", from: "#6FA8DC", to: "#294172" },
  pacman: { fallback: "pacman", from: "#5AC8FA", to: "#1793D1" },
  zypper: { fallback: "zypper", from: "#8BD66B", to: "#4F9A2E" },
  snap: { fallback: "Snap", from: "#FF9E80", to: "#E95420" },
  flatpak: { fallback: "Flatpak", from: "#72A7FF", to: "#4A86CF" },
  appimage: { fallback: "AppImage", from: "#B39DDB", to: "#6A4FB3" },
  desktop_file: { fallback: "Desktop entry", from: "#A3A3A8", to: "#6F6F75" },
  removed: { fallback: "Removed", from: "#C7C7CC", to: "#8E8E93" },
};

const UNKNOWN_SOURCE = { from: "#A3A3A8", to: "#6F6F75" };

export function sourceInfo(source: string): SourceInfo {
  const s = SOURCES[source];
  return {
    key: `washmybelly.uninstall.source.${source}`,
    fallback: s?.fallback ?? source.replace(/_/g, " "),
    from: s?.from ?? UNKNOWN_SOURCE.from,
    to: s?.to ?? UNKNOWN_SOURCE.to,
  };
}

/// The sources that appear in the list, most apps first (then by name), so
/// the chips only offer what exists on this machine.
export function sourcesPresent(apps: readonly App[]): { source: string; count: number }[] {
  const counts = new Map<string, number>();
  for (const a of apps) counts.set(a.source, (counts.get(a.source) ?? 0) + 1);
  return [...counts.entries()]
    .map(([source, count]) => ({ source, count }))
    .sort((a, b) => b.count - a.count || a.source.localeCompare(b.source));
}

/// One or two letters for the generic tile: "Visual Studio Code" → "VS".
export function monogram(name: string): string {
  const words = name
    .replace(/[^\p{L}\p{N}\s]/gu, " ")
    .split(/\s+/)
    .filter(Boolean);
  if (words.length === 0) return "?";
  if (words.length === 1) return words[0].slice(0, 1).toUpperCase();
  return (words[0][0] + words[1][0]).toUpperCase();
}

// ── Filter and sort ──────────────────────────────────────────────────

export type AppSort = "name" | "size" | "last_used";

type AppFilter = { query?: string; sources?: ReadonlySet<string> };

function norm(s: string): string {
  return s
    .normalize("NFD")
    .replace(/\p{M}/gu, "")
    .toLowerCase();
}

/// Search matches name, publisher, native id and app id, ignoring case and
/// accents. An empty source set means every source.
export function filterApps(apps: readonly App[], f: AppFilter): App[] {
  const q = norm((f.query ?? "").trim());
  const sources = f.sources;
  return apps.filter((a) => {
    if (sources && sources.size > 0 && !sources.has(a.source)) return false;
    if (!q) return true;
    const hay = norm([a.name, a.publisher ?? "", a.native_id ?? "", a.id].join(" "));
    return q.split(/\s+/).every((w) => hay.includes(w));
  });
}

/// Name: A→Z. Size: biggest first, unknown last. Last used: longest unused
/// first (the ones worth removing), never-seen last. Ties go by name.
export function sortApps(apps: readonly App[], sort: AppSort): App[] {
  const byName = (a: App, b: App) => a.name.localeCompare(b.name, undefined, { sensitivity: "base" });
  const list = [...apps];
  if (sort === "name") return list.sort(byName);
  if (sort === "size") {
    return list.sort((a, b) => (b.bytes ?? -1) - (a.bytes ?? -1) || byName(a, b));
  }
  return list.sort((a, b) => {
    const x = a.last_used ?? null;
    const y = b.last_used ?? null;
    if (x === null && y === null) return byName(a, b);
    if (x === null) return 1;
    if (y === null) return -1;
    return x - y || byName(a, b);
  });
}

// ── "Not opened in 8 months" ─────────────────────────────────────────

type LastUsed = {
  /// i18n key under `washmybelly.uninstall.used.*`.
  key: string;
  count: number;
  /// Unused for 90 days or more: drawn in the warning tone.
  stale: boolean;
};

const DAY = 86_400;

/// How long ago an app was opened, in the unit a person would say.
export function lastUsed(ts: number | null | undefined, now: number = Date.now() / 1000): LastUsed | null {
  if (ts == null || !Number.isFinite(ts) || ts <= 0) return null;
  const days = Math.max(0, Math.floor((now - ts) / DAY));
  const stale = days >= 90;
  if (days < 1) return { key: "washmybelly.uninstall.used.today", count: 0, stale };
  const [unit, count] = days < 30 ? ["days", days] : days < 365 ? ["months", Math.floor(days / 30)] : ["years", Math.floor(days / 365)];
  // The i18n parser has no plural rules: one gets its own key.
  return { key: `washmybelly.uninstall.used.${unit}${count === 1 ? "_one" : ""}`, count, stale };
}

// ── Leftovers of one app ─────────────────────────────────────────────

const CONFIDENCE_ORDER: readonly Confidence[] = ["high", "medium", "low"];

export type LeftoverItem = LeftoverItemPreview & {
  confidence: Confidence;
  evidence: string;
  kind: string;
  path: string | null;
};

export type Classified = {
  /// Removes the app itself (bundle, uninstaller, package manager).
  app: LeftoverItemPreview[];
  /// `brew uninstall --zap` (or another manager's "remove with data").
  zap: LeftoverItemPreview[];
  /// Everything the app left behind, with its confidence.
  leftovers: LeftoverItem[];
};

const APP_GROUPS = new Set(["app", "application", "bundle", "package", "uninstaller", "uninstall"]);

function isZap(it: LeftoverItemPreview): boolean {
  return it.group === "zap" || /(^|:)zap(:|$)/.test(it.id);
}

function asConfidence(c: string | null | undefined): Confidence {
  return c === "high" || c === "medium" || c === "low" ? c : "low";
}

/// Splits the plan items of `wmb_app_leftovers` into the app, the zap
/// alternative and the leftovers. A leftover is matched to its evidence by
/// path (the item's detail, or the tail of its id); an item with no evidence
/// and an app-like group is the app itself; an item with no evidence and any
/// other group is a leftover of low confidence (never ticked blindly).
export function classify(resp: Pick<LeftoversResponse, "items" | "extra">): Classified {
  const evidence = new Map<string, Leftover>();
  for (const l of resp.extra?.leftovers ?? []) evidence.set(l.path, l);
  const out: Classified = { app: [], zap: [], leftovers: [] };
  for (const it of resp.items ?? []) {
    if (isZap(it)) {
      out.zap.push(it);
      continue;
    }
    const byDetail = it.detail ? evidence.get(it.detail) : undefined;
    let hit = byDetail;
    if (!hit) {
      for (const [path, l] of evidence) {
        if (it.id.endsWith(`:${path}`)) {
          hit = l;
          break;
        }
      }
    }
    if (hit) {
      out.leftovers.push({ ...it, confidence: asConfidence(hit.confidence), evidence: hit.evidence, kind: hit.kind, path: hit.path });
    } else if (APP_GROUPS.has(it.group)) {
      out.app.push(it);
    } else {
      out.leftovers.push({ ...it, confidence: "low", evidence: "", kind: "", path: it.detail ?? null });
    }
  }
  return out;
}

/// Leftovers under confidence headings (high → low), empty ones dropped.
export function byConfidence(items: readonly LeftoverItem[]): { confidence: Confidence; items: LeftoverItem[]; bytes: number }[] {
  return CONFIDENCE_ORDER.map((confidence) => {
    const list = items.filter((i) => i.confidence === confidence);
    return { confidence, items: list, bytes: list.reduce((n, i) => n + (i.bytes || 0), 0) };
  }).filter((g) => g.items.length > 0);
}

/// Leftovers ticked when the app opens: high and medium; low never.
export function defaultLeftovers(items: readonly LeftoverItem[]): Set<string> {
  return new Set(items.filter((i) => i.confidence !== "low" && !i.unticked).map((i) => i.id));
}

/// What the one button does: remove the app normally, remove it through the
/// package manager with its data (`--zap`), or keep the app and only wipe
/// its data and settings.
export type UninstallMode = "uninstall" | "zap" | "reset";

/// Modes this app offers, in display order.
export function modesFor(c: Classified, installed: boolean): UninstallMode[] {
  const modes: UninstallMode[] = [];
  if (c.app.length > 0 || (c.zap.length === 0 && installed)) modes.push("uninstall");
  if (c.zap.length > 0) modes.push("zap");
  if (installed && c.leftovers.length > 0) modes.push("reset");
  return modes;
}

/// Item ids that go into the plan for a mode and the ticked leftovers.
export function planIds(c: Classified, mode: UninstallMode, chosen: ReadonlySet<string>): string[] {
  const left = c.leftovers.filter((l) => chosen.has(l.id)).map((l) => l.id);
  if (mode === "reset") return left;
  if (mode === "zap") return [...c.zap.filter((i) => !i.unticked).map((i) => i.id), ...left];
  // An app item left out by the backend (the app is open) never joins.
  return [...c.app.filter((i) => !i.unticked).map((i) => i.id), ...left];
}

/// Bytes the plan for a mode frees (the zap item's own size may be 0: the
/// manager measures it).
export function planBytes(c: Classified, mode: UninstallMode, chosen: ReadonlySet<string>): number {
  const ids = new Set(planIds(c, mode, chosen));
  let n = 0;
  for (const it of [...c.app, ...c.zap, ...c.leftovers]) if (ids.has(it.id)) n += it.bytes || 0;
  return n;
}

/// Id sent to `wmb_app_leftovers` for an app that is not installed anymore.
/// The core builds a bare App from it (`source` = "removed", `name` = the
/// text) and searches by name and bundle id. A full id (`app:com.x.y`) goes
/// through untouched.
export function removedAppId(text: string): string | null {
  const s = text.trim();
  if (!s) return null;
  if (/^[a-z_]+:.+/.test(s)) return s;
  return `removed:${s}`;
}

/// The app is still installed (the core marks a bare, removed one with
/// `removable: false` and no path/version).
export function isInstalled(app: App): boolean {
  return !(app.source === "removed" || (!app.removable && !app.path && !app.version && app.reason === "not installed anymore"));
}

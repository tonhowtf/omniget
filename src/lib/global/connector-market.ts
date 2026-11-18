export type MarketEntry = {
  id: string;
  name: string;
  description: string;
  version: string;
  author: string;
  match: string[];
  domains: string[];
  login?: { url: string };
  min_omniget: string;
  sha256: string;
  package: string;
  category: string;
  permissions: string[];
  state: "not_installed" | "installed" | "update_available";
  installed_version: string | null;
  dev: boolean;
  in_catalog: boolean;
  compatible: boolean;
  has_options: boolean;
};

export type InvalidConnector = { id: string; dir: string; dev: boolean; error: string };

export type MarketView = {
  entries: MarketEntry[];
  invalid: InvalidConnector[];
  source: "fresh" | "network" | "cache" | "none";
  fetched_at: number | null;
  error: string | null;
  rejected: string[];
};

export type UrlLookup = {
  opted_out: boolean;
  installed: { id: string; name: string; login_url: string | null; dev: boolean } | null;
  catalog: MarketEntry | null;
};

export type PackageSummary = {
  id: string;
  name: string;
  version: string;
  author: string;
  description: string;
  domains: string[];
  permissions: string[];
  replaces_version: string | null;
};

export type InstallProgress = { stage: string; received: number; total: number | null };

const CODES = new Set(["signature", "hash", "mismatch", "package", "network", "stale", "not_found"]);
const OPTED_OUT = "The owner of this site asked OmniGet not to support it.";

export function errorKey(raw: string): string {
  if (raw.includes(OPTED_OUT)) return "errors.platform_opted_out";
  const code = raw.split(":", 1)[0];
  return CODES.has(code) ? `global_dl.market.err.${code}` : "global_dl.market.err.generic";
}

export function authProblem(raw: string): boolean {
  return /sign in|log ?in|login required|\bHTTP 40[13]\b/i.test(raw);
}

export function looksLikeUrl(text: string): boolean {
  const v = text.trim();
  if (!v || /\s/.test(v)) return false;
  if (/^https?:\/\//i.test(v)) return true;
  return /^[a-z0-9-]+(\.[a-z0-9-]+)+(\/\S*)?$/i.test(v);
}

const STATE_ORDER = { update_available: 0, installed: 1, not_installed: 2 };

export function filterEntries(entries: MarketEntry[], query: string, focusId: string): MarketEntry[] {
  const q = query.trim().toLowerCase();
  const hit = (e: MarketEntry) => [e.name, e.id, e.description, ...e.domains].some((v) => v.toLowerCase().includes(q));
  const rank = (e: MarketEntry) => (e.id === focusId ? -1 : STATE_ORDER[e.state]);
  return entries.filter((e) => !q || hit(e)).sort((a, b) => rank(a) - rank(b) || a.name.localeCompare(b.name));
}

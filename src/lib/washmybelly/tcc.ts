type TccAuth = "allowed" | "denied" | "limited" | "unknown";

export type TccApp = {
  client: string;
  client_type: number;
  auth: TccAuth;
  last_modified?: number | null;
  target?: string | null;
  db: "user" | "system";
  can_reset: boolean;
  item_id?: string | null;
};

export type TccService = {
  service: string;
  key: string;
  allowed: number;
  apps: TccApp[];
};

export type TccList =
  | { available: true; system_db: boolean; count: number; services: TccService[] }
  | { available: false; reason: "full_disk_access" | "not_found" | "unreadable" | "not_macos"; services: TccService[] };

export function appName(client: string): string {
  if (client.includes("/")) {
    const last = client.split("/").filter(Boolean).pop() ?? client;
    return last.replace(/\.app$/i, "");
  }
  const parts = client.split(".").filter(Boolean);
  const skip = new Set(["xos", "mac", "macos", "osx", "app", "desktop", "helper"]);
  let pick = parts[parts.length - 1] ?? client;
  for (let i = parts.length - 1; i >= 0; i--) {
    if (!skip.has(parts[i].toLowerCase())) {
      pick = parts[i];
      break;
    }
  }
  if (pick.toLowerCase() === "iterm2") return "iTerm2";
  return pick.charAt(0).toUpperCase() + pick.slice(1);
}

export function serviceKey(s: Pick<TccService, "key">): string {
  return `washmybelly.privacy.tcc.service.${s.key}`;
}

export function orderServices(list: readonly TccService[], allowedOnly = false): TccService[] {
  return list
    .map((s) => (allowedOnly ? { ...s, apps: s.apps.filter((a) => a.auth === "allowed" || a.auth === "limited") } : s))
    .filter((s) => s.apps.length > 0)
    .sort((a, b) => Number(b.allowed > 0) - Number(a.allowed > 0));
}

export function mergeTargets(apps: readonly TccApp[]): (TccApp & { targets: string[] })[] {
  const out = new Map<string, TccApp & { targets: string[] }>();
  for (const a of apps) {
    const k = `${a.db}:${a.client}`;
    const cur = out.get(k);
    if (cur) {
      if (a.target) cur.targets.push(a.target);
      if (a.auth === "allowed") cur.auth = "allowed";
    } else {
      out.set(k, { ...a, targets: a.target ? [a.target] : [] });
    }
  }
  return [...out.values()].sort((a, b) => appName(a.client).localeCompare(appName(b.client)));
}


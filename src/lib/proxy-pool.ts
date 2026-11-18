export type ProxyChecked = {
  ok: boolean;
  ms: number | null;
  ip: string | null;
  country: string | null;
  status: number | null;
  error: string | null;
};

export type PoolProxy = {
  id: string;
  proxy: string;
  current: boolean;
  fails: number;
  restingSecs: number;
  checked?: ProxyChecked;
};

export type PoolRotation = "sticky" | "round_robin";

export type PoolView = {
  enabled: boolean;
  rotation: PoolRotation;
  proxies: PoolProxy[];
  total: number;
  truncated: boolean;
  available: number;
  reason?: string;
};

export type PoolChange = PoolView & {
  added?: number;
  duplicates?: number;
  rejected?: { input: string; reason: string }[];
  removed?: number;
  checked?: number;
  unchecked?: number;
};

export type PoolDetected = { found: { proxy: string; app: string; in_pool: boolean }[]; total: number };

export type ProxyState = "failed" | "resting" | "ok" | "untested";

export function proxyState(proxy: PoolProxy): ProxyState {
  if (proxy.checked && !proxy.checked.ok) return "failed";
  if (proxy.restingSecs > 0) return "resting";
  return proxy.checked ? "ok" : "untested";
}

export function emptyPool(): PoolView {
  return { enabled: false, rotation: "sticky", proxies: [], total: 0, truncated: false, available: 0 };
}

export const POOL_PAGE = 50;
export const MAX_CHECK_ROUNDS = 25;

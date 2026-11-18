import type { PoolChange, PoolDetected, PoolProxy, PoolRotation, PoolView } from "$lib/proxy-pool";

type Args = Record<string, unknown>;

let enabled = true;
let rotation: PoolRotation = "sticky";
let proxies: PoolProxy[] = [
  { id: "px_1a2b3c4d", proxy: "socks5h://127.0.0.1:7890", current: true, fails: 0, restingSecs: 0, checked: { ok: true, ms: 182, ip: "203.0.113.9", country: "JP", status: 200, error: null } },
  { id: "px_5e6f7a8b", proxy: "http://reader:***@proxy.example:3128", current: false, fails: 2, restingSecs: 60, checked: { ok: false, ms: null, ip: null, country: null, status: null, error: "could not connect through the proxy" } },
  { id: "px_9c0d1e2f", proxy: "http://198.51.100.7:8080", current: false, fails: 0, restingSecs: 0 },
];

function view(): PoolView {
  const available = proxies.filter((p) => p.restingSecs === 0).length;
  return { enabled, rotation, proxies, total: proxies.length, truncated: false, available };
}

function idOf(index: number): string {
  return `px_preview${String(index).padStart(2, "0")}`;
}

export const proxyPoolFixtures: Record<string, (args: Args) => unknown> = {
  proxy_pool_status: (): PoolView => view(),
  proxy_pool_add: ({ text }): PoolChange => {
    const lines = String(text ?? "").split("\n").map((l) => l.trim()).filter((l) => l && !l.startsWith("#"));
    const rejected = lines.filter((l) => !/:\d+/.test(l)).map((input) => ({ input, reason: "a proxy needs a host and a port, like socks5h://127.0.0.1:7890" }));
    const fresh = lines.filter((l) => /:\d+/.test(l)).map((l) => (l.includes("://") ? l : `http://${l}`)).filter((l) => !proxies.some((p) => p.proxy === l));
    proxies = [...proxies, ...fresh.map((proxy, i) => ({ id: idOf(proxies.length + i), proxy, current: false, fails: 0, restingSecs: 0 }))];
    return { ...view(), added: fresh.length, duplicates: lines.length - fresh.length - rejected.length, rejected };
  },
  proxy_pool_remove: ({ ids, failing }): PoolChange => {
    const before = proxies.length;
    const gone = new Set((ids as string[] | undefined) ?? []);
    proxies = proxies.filter((p) => !gone.has(p.id) && !(failing === true && p.checked && !p.checked.ok));
    return { ...view(), removed: before - proxies.length };
  },
  proxy_pool_set: (args): PoolView => {
    if (typeof args.enabled === "boolean") enabled = args.enabled;
    if (args.rotation === "sticky" || args.rotation === "round_robin") rotation = args.rotation;
    return view();
  },
  proxy_pool_check: (): PoolChange => {
    proxies = proxies.map((p) => (p.checked ? p : { ...p, checked: { ok: true, ms: 240, ip: "198.51.100.7", country: "DE", status: 200, error: null } }));
    return { ...view(), checked: proxies.length, unchecked: 0 };
  },
  proxy_pool_detect: (): PoolDetected => ({ found: [{ proxy: "socks5h://127.0.0.1:7890", app: "Clash / mihomo / ClashX / FlClash / Stash", in_pool: proxies.some((p) => p.proxy === "socks5h://127.0.0.1:7890") }], total: 1 }),
};

/** Wire types of the `remote_*` commands (src-tauri/src/local_bridge_remote.rs). */

export type RemoteScope = "read" | "drive";

export type InterfaceOption = { ip: string; kind: "loopback" | "lan" | "tailscale" | "other" | string };

export type Endpoint = { kind: "direct" | "tailscale-serve" | "ssh" | string; url: string };

export type RemoteStatus = {
  enabled: boolean;
  running: boolean;
  bindIp: string;
  port: number;
  bound: string | null;
  interfaces: InterfaceOption[];
  endpoints: Endpoint[];
  devices: number;
  pendingPairs: number;
  tailscaleHttpsPort: number;
  tailscaleServeUrl: string | null;
  lastError: string | null;
};

export type PairLink = {
  url: string;
  base: string;
  scope: RemoteScope;
  label: string | null;
  expiresAt: string;
  qrSvg: string;
};

export type DeviceView = {
  id: string;
  name: string;
  scope: RemoteScope;
  createdAt: string;
  lastUsedAt: string | null;
  lastIp: string | null;
  userAgent: string | null;
  connected: number;
};

export type AccessEntry = {
  at: string;
  ip: string;
  deviceId: string | null;
  device: string | null;
  method: string;
  path: string;
  status: number;
};

export type TailscaleInfo = {
  installed: boolean;
  binary: string | null;
  running: boolean;
  backendState: string | null;
  dnsName: string | null;
  ips: string[];
  httpsPort: number;
  target: string | null;
  serveArgs: string[];
  serveCommand: string | null;
  serveOffArgs: string[];
  serveOffCommand: string | null;
  serveUrl: string | null;
  serveEnabled: boolean;
  error: string | null;
};

export type CommandRun = { ok: boolean; command: string; stdout: string; stderr: string; serveUrl: string | null };

export type SshInfo = {
  command: string;
  user: string;
  host: string;
  localPort: number;
  remoteTarget: string;
  base: string;
  running: boolean;
};

export function errText(e: unknown): string {
  const s = typeof e === "string" ? e : e instanceof Error ? e.message : JSON.stringify(e);
  return s.replace(/^[A-Z_]+:\s*/, "");
}

export function relTime(iso: string | null | undefined): string {
  if (!iso) return "—";
  const t = new Date(iso).getTime();
  if (!Number.isFinite(t)) return "—";
  const s = Math.max(0, (Date.now() - t) / 1000);
  if (s < 60) return `${Math.round(s)}s`;
  if (s < 3600) return `${Math.round(s / 60)}m`;
  if (s < 86400) return `${Math.round(s / 3600)}h`;
  return `${Math.round(s / 86400)}d`;
}

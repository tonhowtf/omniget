import type { RemoteEntry, ServerDraft, TransferProtocol, TransferServer, TransferSnapshot } from "./types";

export const DEFAULT_PORTS: Record<TransferProtocol, number> = { ftp: 21, ftps: 21, sftp: 22 };

export interface Crumb {
  label: string;
  path: string;
}

export function crumbs(path: string): Crumb[] {
  const out: Crumb[] = [{ label: "/", path: "/" }];
  let walked = "";
  for (const part of path.split("/").filter(Boolean)) {
    walked += `/${part}`;
    out.push({ label: part, path: walked });
  }
  return out;
}

export function childPath(folder: string, name: string): string {
  return folder === "/" ? `/${name}` : `${folder}/${name}`;
}

export function parentPath(path: string): string {
  const at = path.lastIndexOf("/");
  return at <= 0 ? "/" : path.slice(0, at);
}

const UNITS = ["B", "KB", "MB", "GB", "TB"];

export function formatBytes(bytes: number | null): string {
  if (bytes === null || !Number.isFinite(bytes) || bytes < 0) return "";
  let value = bytes;
  let unit = 0;
  while (value >= 1024 && unit < UNITS.length - 1) {
    value /= 1024;
    unit += 1;
  }
  const digits = unit === 0 || value >= 100 ? 0 : 1;
  return `${value.toFixed(digits)} ${UNITS[unit]}`;
}

export function progressPercent(snapshot: Pick<TransferSnapshot, "bytes" | "total" | "status">): number | null {
  if (snapshot.status === "done") return 100;
  if (snapshot.total === null || snapshot.total <= 0) return null;
  return Math.min(100, Math.floor((snapshot.bytes / snapshot.total) * 100));
}

export function validName(name: string): boolean {
  const trimmed = name.trim();
  if (!trimmed || trimmed === "." || trimmed === "..") return false;
  return !/[/\\]/.test(trimmed) && ![...trimmed].some((c) => c.charCodeAt(0) < 32 || c.charCodeAt(0) === 127);
}

export interface ServerForm {
  name: string;
  protocol: TransferProtocol;
  host: string;
  port: string;
  user: string;
  startPath: string;
  keyPath: string;
  secret: string;
}

export function emptyForm(): ServerForm {
  return { name: "", protocol: "ftp", host: "", port: "", user: "", startPath: "", keyPath: "", secret: "" };
}

export function formOf(server: TransferServer): ServerForm {
  return {
    name: server.name,
    protocol: server.protocol,
    host: server.host,
    port: server.port === DEFAULT_PORTS[server.protocol] ? "" : String(server.port),
    user: server.user,
    startPath: server.startPath,
    keyPath: server.keyPath ?? "",
    secret: "",
  };
}

export function draftOf(form: ServerForm): ServerDraft {
  const port = Number.parseInt(form.port.trim(), 10);
  const draft: ServerDraft = {
    name: form.name.trim(),
    protocol: form.protocol,
    host: form.host.trim(),
    user: form.user.trim(),
    startPath: form.startPath.trim(),
  };
  if (Number.isFinite(port)) draft.port = port;
  if (form.protocol === "sftp" && form.keyPath.trim()) draft.keyPath = form.keyPath.trim();
  return draft;
}

export function secretOf(form: ServerForm, editing: boolean): string | null {
  if (form.secret) return form.secret;
  return editing ? null : "";
}

export function formReady(form: ServerForm): boolean {
  if (!form.name.trim() || !form.host.trim()) return false;
  if (form.protocol === "sftp" && !form.user.trim()) return false;
  const port = form.port.trim();
  return port === "" || (/^\d+$/.test(port) && Number(port) >= 1 && Number(port) <= 65535);
}

export function serverAddress(server: Pick<TransferServer, "protocol" | "host" | "port" | "user">): string {
  const user = server.user ? `${server.user}@` : "";
  const port = server.port === DEFAULT_PORTS[server.protocol] ? "" : `:${server.port}`;
  return `${server.protocol}://${user}${server.host}${port}`;
}

export function filterEntries(entries: RemoteEntry[], query: string): RemoteEntry[] {
  const needle = query.trim().toLowerCase();
  if (!needle) return entries;
  return entries.filter((entry) => entry.name.toLowerCase().includes(needle));
}

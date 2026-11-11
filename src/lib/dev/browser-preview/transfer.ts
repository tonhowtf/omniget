import type { ManageResult, RemoteEntry, RemoteListing, ServerDraft, TransferServer, TransferSnapshot } from "$lib/transfer/types";

type Args = Record<string, unknown>;

function fail(code: string, message: string, nextStep: string, retryable = false): never {
  throw `${code}: ${JSON.stringify({ message, retryable, next_step: nextStep })}`;
}

let servers: TransferServer[] = [
  { name: "Switch", protocol: "ftp", host: "192.168.0.42", port: 5000, user: "", startPath: "", keyPath: null, hasSecret: false },
  { name: "NAS", protocol: "sftp", host: "nas.local", port: 22, user: "me", startPath: "/music", keyPath: null, hasSecret: true },
  { name: "Old host", protocol: "ftps", host: "ftp.example.com", port: 21, user: "site", startPath: "", keyPath: null, hasSecret: true },
];

const tree: Record<string, RemoteEntry[]> = {
  "/": [
    { name: "atmosphere", kind: "dir", size: null, modified: "2026-10-02T18:20:00Z" },
    { name: "switch", kind: "dir", size: null, modified: "2026-11-08T09:12:00Z" },
    { name: "music", kind: "dir", size: null, modified: "2026-09-14T21:00:00Z" },
    { name: "hbmenu.nro", kind: "file", size: 6_291_456, modified: "2026-08-30T12:00:00Z" },
    { name: "boot.dat", kind: "file", size: 262_144, modified: "2026-08-30T12:00:00Z" },
  ],
  "/atmosphere": [],
  "/switch": [
    { name: "ftpd.nro", kind: "file", size: 1_153_434, modified: "2026-11-08T09:12:00Z" },
    { name: "checkpoint", kind: "dir", size: null, modified: "2026-11-01T10:00:00Z" },
  ],
  "/switch/checkpoint": [{ name: "saves.zip", kind: "file", size: 48_234_496, modified: "2026-11-01T10:05:00Z" }],
  "/music": [
    { name: "album", kind: "dir", size: null, modified: "2026-09-14T21:00:00Z" },
    { name: "current", kind: "link", size: null, modified: null },
    { name: "mix.flac", kind: "file", size: 31_457_280, modified: "2026-09-14T21:00:00Z" },
  ],
  "/music/album": [],
};

const jobs = new Map<string, { snapshot: TransferSnapshot; polls: number }>();
let serial = 0;

function serverOf(target: unknown): TransferServer {
  const name = String((target as { server?: string } | undefined)?.server ?? "");
  const found = servers.find((server) => server.name.toLowerCase() === name.toLowerCase());
  if (!found) fail("UNKNOWN_SERVER", `no saved server is named "${name}"`, "call transfer_servers with op=list to see the saved names, or pass protocol and host instead of server");
  return found;
}

function folder(path: string): RemoteEntry[] {
  const entries = tree[path];
  if (!entries) fail("NOT_FOUND", `${path} does not exist on the server`, "call transfer_list on the parent folder to see the real names");
  return entries;
}

function split(path: string): [string, string] {
  const at = path.lastIndexOf("/");
  return [at <= 0 ? "/" : path.slice(0, at), path.slice(at + 1)];
}

function start(direction: TransferSnapshot["direction"], remote: string, local: string, total: number): TransferSnapshot {
  const snapshot: TransferSnapshot = { transferId: `preview-${++serial}`, direction, status: "running", bytes: 0, total, remote, local };
  jobs.set(snapshot.transferId, { snapshot, polls: 0 });
  return snapshot;
}

export const transferFixtures: Record<string, (args: Args) => unknown> = {
  transfer_servers_list: (): TransferServer[] => servers,
  transfer_server_save: ({ draft, secret }): TransferServer => {
    const d = draft as ServerDraft;
    if (!d.name.trim() || !d.host.trim()) fail("INVALID_INPUT", "name and host are required", "fix the argument named in the message and call again");
    const before = servers.find((server) => server.name.toLowerCase() === d.name.toLowerCase());
    const saved: TransferServer = {
      name: d.name,
      protocol: d.protocol,
      host: d.host,
      port: d.port ?? (d.protocol === "sftp" ? 22 : 21),
      user: d.user,
      startPath: d.startPath,
      keyPath: d.keyPath ?? null,
      hasSecret: secret === null || secret === undefined ? (before?.hasSecret ?? false) : secret !== "",
    };
    servers = [...servers.filter((server) => server !== before), saved].sort((a, b) => a.name.localeCompare(b.name));
    return saved;
  },
  transfer_server_remove: ({ name }): string => {
    servers = servers.filter((server) => server.name !== name);
    return String(name);
  },
  transfer_host_forget: (): number => 1,
  transfer_list: ({ target, path }): RemoteListing => {
    const server = serverOf(target);
    if (server.name === "Old host") {
      fail("HOST_UNREACHABLE", `cannot reach ${server.host}:${server.port}: connection refused`, "check that the server is on, on the same network, and that host and port are right (FTP 21, SFTP 22, homebrew console servers often 5000), then retry", true);
    }
    const at = typeof path === "string" && path ? path : server.startPath || "/";
    const entries = folder(at);
    return { path: at, entries, total: entries.length, truncated: false, skipped: 0, untrusted: true, ...(entries.length === 0 ? { reason: "empty_directory" } : {}) };
  },
  transfer_get: ({ target, path, saveTo, overwrite }): TransferSnapshot => {
    serverOf(target);
    const [parent, name] = split(String(path));
    const entry = folder(parent).find((found) => found.name === name);
    if (!entry) fail("NOT_FOUND", `${String(path)} does not exist on the server`, "call transfer_list on the parent folder to see the real names");
    const local = `${String(saveTo)}/${name}`;
    if (name === "boot.dat" && overwrite !== true) {
      fail("ALREADY_EXISTS", `${local} already exists and overwrite is off`, "pass overwrite=true to replace it, or choose another name; nothing was written");
    }
    return start("download", String(path), local, entry.size ?? 0);
  },
  transfer_put: ({ target, localPath, path }): TransferSnapshot => {
    serverOf(target);
    const name = String(localPath).split(/[\\/]/).pop() ?? "file";
    const parent = String(path).replace(/\/$/, "") || "/";
    const entries = folder(parent);
    if (!entries.some((entry) => entry.name === name)) entries.push({ name, kind: "file", size: 2_097_152, modified: new Date().toISOString() });
    return start("upload", `${parent === "/" ? "" : parent}/${name}`, String(localPath), 2_097_152);
  },
  transfer_status: ({ transferId }): TransferSnapshot => {
    const job = jobs.get(String(transferId));
    if (!job) fail("UNKNOWN_TRANSFER", `no transfer has the id ${String(transferId)}`, "transfer ids are forgotten when the app restarts; start the transfer again");
    job.polls += 1;
    const total = job.snapshot.total ?? 0;
    const done = job.polls >= 4;
    job.snapshot = { ...job.snapshot, status: done ? "done" : "running", bytes: done ? total : Math.floor((total * job.polls) / 4) };
    return job.snapshot;
  },
  transfer_manage: ({ target, op, path, to, confirm }): ManageResult => {
    serverOf(target);
    const [parent, name] = split(String(path));
    const entries = folder(parent);
    const entry = entries.find((found) => found.name === name);
    if (op === "mkdir") {
      if (entry) fail("ALREADY_EXISTS", `${String(path)} already exists and overwrite is off`, "pass overwrite=true to replace it, or choose another name; nothing was written");
      entries.push({ name, kind: "dir", size: null, modified: new Date().toISOString() });
      tree[String(path)] = [];
      return { op: "mkdir", path: String(path), done: true, dryRun: false };
    }
    if (!entry) fail("NOT_FOUND", `${String(path)} does not exist on the server`, "call transfer_list on the parent folder to see the real names");
    if (op === "rename") {
      entry.name = String(to);
      return { op: "rename", path: String(path), to: `${parent === "/" ? "" : parent}/${String(to)}`, done: true, dryRun: false };
    }
    if (entry.kind === "dir" && (tree[String(path)] ?? []).length > 0) {
      fail("NOT_EMPTY", `${String(path)} is a directory that still has entries`, "delete only removes files and empty folders; list the folder and delete its entries first");
    }
    const result: ManageResult = { op: "delete", path: String(path), done: confirm === true, dryRun: confirm !== true, target: { path: String(path), kind: entry.kind, size: entry.size } };
    if (confirm === true) entries.splice(entries.indexOf(entry), 1);
    else result.would = `delete the ${entry.kind === "dir" ? "empty folder" : "file"} ${String(path)}`;
    return result;
  },
};

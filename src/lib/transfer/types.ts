export type TransferProtocol = "ftp" | "ftps" | "sftp";
export type EntryKind = "file" | "dir" | "link" | "other";

export interface TransferServer {
  name: string;
  protocol: TransferProtocol;
  host: string;
  port: number;
  user: string;
  startPath: string;
  keyPath: string | null;
  hasSecret: boolean;
}

export interface ServerDraft {
  name: string;
  protocol: TransferProtocol;
  host: string;
  port?: number;
  user: string;
  startPath: string;
  keyPath?: string;
}

export interface TransferTarget {
  server: string;
}

export interface RemoteEntry {
  name: string;
  kind: EntryKind;
  size: number | null;
  modified: string | null;
}

export interface RemoteListing {
  path: string;
  entries: RemoteEntry[];
  total: number;
  truncated: boolean;
  skipped: number;
  untrusted: boolean;
  reason?: string;
}

export interface TransferSnapshot {
  transferId: string;
  direction: "download" | "upload";
  status: "running" | "done" | "failed";
  bytes: number;
  total: number | null;
  remote: string;
  local: string;
}

export interface ManageResult {
  op: "mkdir" | "rename" | "delete";
  path: string;
  to?: string;
  done: boolean;
  dryRun: boolean;
  target?: { path: string; kind: EntryKind; size: number | null };
  would?: string;
}

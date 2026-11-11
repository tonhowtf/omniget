import { invoke } from "@tauri-apps/api/core";
import type { ManageResult, RemoteListing, ServerDraft, TransferServer, TransferSnapshot, TransferTarget } from "./types";

export { parseError, reveal } from "$lib/devtools/api";

export function serversList(): Promise<TransferServer[]> {
  return invoke<TransferServer[]>("transfer_servers_list");
}

export function serverSave(draft: ServerDraft, secret: string | null): Promise<TransferServer> {
  return invoke<TransferServer>("transfer_server_save", { draft, secret });
}

export function serverRemove(name: string): Promise<string> {
  return invoke<string>("transfer_server_remove", { name });
}

export function hostForget(host: string, port: number): Promise<number> {
  return invoke<number>("transfer_host_forget", { host, port });
}

export function list(target: TransferTarget, path: string | null, limit?: number): Promise<RemoteListing> {
  return invoke<RemoteListing>("transfer_list", { target, path, limit });
}

export function get(target: TransferTarget, path: string, saveTo: string, overwrite = false): Promise<TransferSnapshot> {
  return invoke<TransferSnapshot>("transfer_get", { target, path, saveTo, overwrite });
}

export function put(target: TransferTarget, localPath: string, path: string, overwrite = false): Promise<TransferSnapshot> {
  return invoke<TransferSnapshot>("transfer_put", { target, localPath, path, overwrite });
}

export function status(transferId: string, waitSeconds = 0): Promise<TransferSnapshot> {
  return invoke<TransferSnapshot>("transfer_status", { transferId, waitSeconds });
}

export function mkdir(target: TransferTarget, path: string): Promise<ManageResult> {
  return invoke<ManageResult>("transfer_manage", { target, op: "mkdir", path, to: null, confirm: false });
}

export function rename(target: TransferTarget, path: string, to: string): Promise<ManageResult> {
  return invoke<ManageResult>("transfer_manage", { target, op: "rename", path, to, confirm: false });
}

export function remove(target: TransferTarget, path: string, confirm: boolean): Promise<ManageResult> {
  return invoke<ManageResult>("transfer_manage", { target, op: "delete", path, to: null, confirm });
}

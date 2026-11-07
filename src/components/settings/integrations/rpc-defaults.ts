export const RPC_DEFAULTS = {
  app_id: "1502867748353478656",
  large_image_key: "omniget_music",
} as const;

type RpcIdentity = { app_id?: string | null; large_image_key?: string | null };

export function rpcIdentityChanged(rpc: RpcIdentity | null | undefined): boolean {
  if (!rpc) return false;
  return (rpc.app_id ?? "") !== RPC_DEFAULTS.app_id || (rpc.large_image_key ?? "") !== RPC_DEFAULTS.large_image_key;
}

export type RpcTestOutcome = "ok" | "fail" | "missing_app_id";

export function rpcTestOutcome(res: { ok: boolean; reason?: string } | null): RpcTestOutcome {
  if (res?.ok) return "ok";
  if (res?.reason === "missing_app_id") return "missing_app_id";
  return "fail";
}

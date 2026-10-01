import { spawnSync } from "node:child_process";
import { homedir } from "node:os";
import { join } from "node:path";

export const sharedTargetDir = join(homedir(), ".cache", "omniget", "target");

export function has(cmd, args = ["--version"]) {
  return spawnSync(cmd, args, { stdio: "ignore", shell: process.platform === "win32" }).status === 0;
}

export function cargoEnv(base = process.env) {
  const env = { ...base };
  if (!env.RUSTC_WRAPPER && has("sccache")) env.RUSTC_WRAPPER = "sccache";
  if (!env.CARGO_TARGET_DIR && env.OMNIGET_SHARED_TARGET !== "0") env.CARGO_TARGET_DIR = sharedTargetDir;
  return env;
}

export function cargoEnvSummary(env) {
  return `${env.RUSTC_WRAPPER ? " (sccache)" : ""}${env.CARGO_TARGET_DIR ? ` target=${env.CARGO_TARGET_DIR}` : ""}`;
}

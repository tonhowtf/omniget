// Environment for cargo runs started by the dev scripts (test-fast, verify).
//
// - Shared build directory by default: ~/.cache/omniget/target. Every checkout
//   and worktree that builds there pays a cold build (~20 min) once instead of
//   once per directory; cargo's lock serializes concurrent builds. An explicit
//   CARGO_TARGET_DIR wins, and OMNIGET_SHARED_TARGET=0 goes back to the
//   checkout's own src-tauri/target.
// - sccache as RUSTC_WRAPPER when it is installed and no wrapper is set.
import { spawnSync } from "node:child_process";
import { homedir } from "node:os";
import { join } from "node:path";

export const sharedTargetDir = join(homedir(), ".cache", "omniget", "target");

/** True when `cmd args` runs and exits 0 (a quick "is it installed" probe). */
export function has(cmd, args = ["--version"]) {
  return spawnSync(cmd, args, { stdio: "ignore", shell: process.platform === "win32" }).status === 0;
}

/** A copy of `base` with CARGO_TARGET_DIR and RUSTC_WRAPPER filled in. */
export function cargoEnv(base = process.env) {
  const env = { ...base };
  if (!env.RUSTC_WRAPPER && has("sccache")) env.RUSTC_WRAPPER = "sccache";
  if (!env.CARGO_TARGET_DIR && env.OMNIGET_SHARED_TARGET !== "0") env.CARGO_TARGET_DIR = sharedTargetDir;
  return env;
}

/** One line for the end of a run: where cargo built and whether sccache was on. */
export function cargoEnvSummary(env) {
  return `${env.RUSTC_WRAPPER ? " (sccache)" : ""}${env.CARGO_TARGET_DIR ? ` target=${env.CARGO_TARGET_DIR}` : ""}`;
}

#!/usr/bin/env node
// `pnpm hooks:install` / `pnpm hooks:uninstall`: points git at the hooks in
// scripts/git-hooks (core.hooksPath), or back at the default .git/hooks.
//
// The setting lives in the repository's config, so it covers every worktree of
// this clone; the path is relative and resolves inside each one.
import { spawnSync } from "node:child_process";
import { chmodSync, readdirSync } from "node:fs";
import { join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const here = fileURLToPath(new URL(".", import.meta.url));
const root = resolve(here, "..", "..");
const HOOKS_PATH = "scripts/git-hooks";
const git = (...args) => spawnSync("git", args, { cwd: root, encoding: "utf8" });

if (process.argv.includes("--uninstall")) {
  const current = git("config", "--get", "core.hooksPath").stdout.trim();
  if (current !== HOOKS_PATH) {
    console.log(current ? `core.hooksPath is ${current}, not ours; left alone.` : "No hooks installed; nothing to do.");
    process.exit(0);
  }
  const r = git("config", "--unset", "core.hooksPath");
  if (r.status !== 0) {
    console.error(r.stderr);
    process.exit(1);
  }
  console.log("Hooks uninstalled: git uses .git/hooks again.");
  process.exit(0);
}

// Git runs a hook only when it is executable; a checkout on a filesystem that
// drops the bit (or a Windows clone) may not have it.
if (process.platform !== "win32") {
  for (const name of readdirSync(here)) {
    if (!name.includes(".")) chmodSync(join(here, name), 0o755);
  }
}
const r = git("config", "core.hooksPath", HOOKS_PATH);
if (r.status !== 0) {
  console.error(r.stderr);
  process.exit(1);
}
console.log(`Hooks installed (core.hooksPath=${HOOKS_PATH}).`);
console.log("pre-push runs `node scripts/verify.mjs --changed`; skip it once with `git push --no-verify`.");
console.log("Undo with `pnpm hooks:uninstall`.");

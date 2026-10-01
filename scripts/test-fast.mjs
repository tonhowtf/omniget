#!/usr/bin/env node
// Fast local test run: Rust and web checks at the same time.
//
//   node scripts/test-fast.mjs              rust + web
//   node scripts/test-fast.mjs rust         only Rust (core, app, cli)
//   node scripts/test-fast.mjs web          vitest + svelte-check + i18n
//   node scripts/test-fast.mjs rust --nextest   one process per test (isolation, slow-test report)
//   node scripts/test-fast.mjs rust -- missions::   extra args go to the test runner
//
// What makes it fast (measured 25/09/2026 on an M5, 10 cores):
// - One build directory for everyone. A cold build of core+app tests takes
//   ~20 min; a small change on top of an existing build ~25 s. Separate
//   CARGO_TARGET_DIRs per agent/worktree each pay the 20 min, and sccache does
//   not save them: crates with build scripts embed the target path, so their
//   dependents miss the cache. So every checkout builds in
//   ~/.cache/omniget/target by default (CARGO_TARGET_DIR, when set, still wins;
//   OMNIGET_SHARED_TARGET=0 uses the checkout's own src-tauri/target); cargo's
//   lock serializes concurrent builds. See scripts/cargo-env.mjs.
// - Always the same package set in one cargo invocation: testing omniget-core
//   alone re-resolves features and recompiles it (~3.5 min).
// - cargo test is faster than nextest here (thousands of millisecond tests;
//   process-per-test overhead): core 14 s vs 19 s, app 5 s vs 10 s.
// - sccache (used when installed) helps rebuilds in the same directory, e.g.
//   after a clean or a branch switch.
import { spawn } from "node:child_process";
import { join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { cargoEnv, cargoEnvSummary, has } from "./cargo-env.mjs";

const root = resolve(fileURLToPath(new URL(".", import.meta.url)), "..");
const argv = process.argv.slice(2);
const split = argv.indexOf("--");
const extra = split >= 0 ? argv.slice(split + 1) : [];
const own = split >= 0 ? argv.slice(0, split) : argv;
const useNextest = own.includes("--nextest");
const what = own.find((a) => !a.startsWith("--")) ?? "all";

const env = cargoEnv();

function run(label, cmd, args, cwd = root) {
  const started = Date.now();
  return new Promise((resolve) => {
    // pnpm is a .cmd shim on Windows, which spawn only finds through a shell.
    const child = spawn(cmd, args, { cwd, env, stdio: ["ignore", "pipe", "pipe"], shell: process.platform === "win32" });
    let out = "";
    child.stdout.on("data", (d) => (out += d));
    child.stderr.on("data", (d) => (out += d));
    child.on("close", (code) => {
      const secs = ((Date.now() - started) / 1000).toFixed(1);
      resolve({ label, ok: code === 0, secs, out });
    });
  });
}

async function rust() {
  const tauri = join(root, "src-tauri");
  const nextest = useNextest && has("cargo", ["nextest", "--version"]);
  // Libraries of core and app in one run (one link step each), then the CLI
  // crate, which has integration tests under tests/.
  const libs = nextest
    ? ["nextest", "run", "--lib", "-p", "omniget-core", "-p", "omniget", ...extra]
    : ["test", "--lib", "-p", "omniget-core", "-p", "omniget", ...extra];
  const cli = nextest ? ["nextest", "run", "-p", "omniget-cli", ...extra] : ["test", "-p", "omniget-cli", ...extra];
  const a = await run(`rust libs (${nextest ? "nextest" : "cargo test"})`, "cargo", libs, tauri);
  const b = await run("rust cli", "cargo", cli, tauri);
  return [a, b];
}

function web() {
  return Promise.all([
    run("vitest", "pnpm", ["-s", "vitest", "run"]),
    run("svelte-check", "pnpm", ["-s", "check"]),
    run("i18n strict", "node", ["scripts/check-i18n.js", "--strict"]),
  ]);
}

const started = Date.now();
// Web checks are mostly single-threaded; they run while cargo compiles.
const jobs = [];
if (what === "all" || what === "rust") jobs.push(rust());
if (what === "all" || what === "web") jobs.push(web());
const results = (await Promise.all(jobs)).flat();

for (const r of results.filter((r) => !r.ok)) {
  console.log(`\n── ${r.label} failed ──\n${r.out.split("\n").slice(-60).join("\n")}`);
}
console.log("");
for (const r of results) console.log(`${r.ok ? "ok  " : "FAIL"}  ${r.label.padEnd(28)} ${r.secs}s`);
console.log(`total ${((Date.now() - started) / 1000).toFixed(1)}s${cargoEnvSummary(env)}`);
process.exit(results.every((r) => r.ok) ? 0 : 1);

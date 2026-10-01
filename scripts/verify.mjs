#!/usr/bin/env node
// The verification gate (there is no CI): runs every check and prints one table at the end.
//
//   pnpm verify              every check (Rust + web + optional suites)
//   pnpm verify --changed    only the areas touched since the merge-base with
//                            main (commits, working tree and untracked files)
//   pnpm verify --fix        run `cargo fmt --all` first
//
// Rust checks run one after another (they share cargo's target lock); the web
// checks run in parallel with them. Exits non-zero when any check fails and
// prints the tail of each failing check's output.
//
// cargo builds in the shared target (~/.cache/omniget/target) unless
// CARGO_TARGET_DIR is set or OMNIGET_SHARED_TARGET=0; see scripts/cargo-env.mjs.
// The opt-in pre-push hook (`pnpm hooks:install`) runs `--changed`.
import { spawn, spawnSync } from "node:child_process";
import { readFileSync } from "node:fs";
import { join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { cargoEnv, cargoEnvSummary } from "./cargo-env.mjs";

const root = resolve(fileURLToPath(new URL(".", import.meta.url)), "..");
const tauriDir = join(root, "src-tauri");
const isWindows = process.platform === "win32";

const argv = process.argv.slice(2);
if (argv.includes("--help") || argv.includes("-h")) {
  const lines = readFileSync(fileURLToPath(import.meta.url), "utf8").split("\n").slice(1);
  const header = lines.slice(0, lines.findIndex((l) => !l.startsWith("//")));
  console.log(header.map((l) => l.replace(/^\/\/ ?/, "")).join("\n"));
  process.exit(0);
}
const onlyChanged = argv.includes("--changed");
const fix = argv.includes("--fix");

const env = cargoEnv();
const scripts = JSON.parse(readFileSync(join(root, "package.json"), "utf8")).scripts ?? {};

// ---------------------------------------------------------------------------
// What changed (--changed)
// ---------------------------------------------------------------------------

function git(args) {
  const r = spawnSync("git", args, { cwd: root, encoding: "utf8" });
  return r.status === 0 ? r.stdout : null;
}

/** Files changed since the merge-base with main, or null when git cannot tell. */
function changedFiles() {
  let base = null;
  for (const ref of ["main", "origin/main"]) {
    base = git(["merge-base", "HEAD", ref])?.trim();
    if (base) break;
  }
  if (!base) return null;
  const tracked = git(["diff", "--name-only", "--no-renames", base]);
  const untracked = git(["ls-files", "--others", "--exclude-standard"]);
  if (tracked === null || untracked === null) return null;
  return [...new Set(`${tracked}\n${untracked}`.split("\n").map((f) => f.trim()).filter(Boolean))];
}

const WEB_FILES = [
  /^src\//,
  /^static\//,
  /^package\.json$/,
  /^pnpm-lock\.yaml$/,
  /^pnpm-workspace\.yaml$/,
  /^(svelte|vite|vitest)\.config\.[cm]?[jt]s$/,
  /^tsconfig[^/]*\.json$/,
  /^scripts\/check-i18n\.js$/,
];
const areas = {
  rust: (f) =>
    f.startsWith("src-tauri/") ||
    f === "rust-toolchain.toml" ||
    f === "rustfmt.toml" ||
    f === "scripts/clippy-baseline.mjs",
  web: (f) => WEB_FILES.some((re) => re.test(f)),
  ipc: (f) =>
    f.startsWith("src/") ||
    f.startsWith("src-tauri/") ||
    f === "scripts/check-ipc.mjs" ||
    f === "scripts/ipc-allowlist.json",
  extension: (f) => f.startsWith("browser-extension/") || f === "scripts/test-extension.mjs",
  plugin: (f) => f.startsWith("claude-plugin/"),
};

let touched = null; // null = run everything
if (onlyChanged) {
  const files = changedFiles();
  if (files === null) {
    console.log("[verify] could not find the merge-base with main; running everything");
  } else {
    touched = Object.fromEntries(Object.entries(areas).map(([k, test]) => [k, files.some(test)]));
    const hit = Object.keys(touched).filter((k) => touched[k]);
    console.log(`[verify] ${files.length} changed file(s); areas: ${hit.length ? hit.join(", ") : "none"}`);
  }
}
const wants = (area) => touched === null || touched[area];

// ---------------------------------------------------------------------------
// Running checks
// ---------------------------------------------------------------------------

const results = [];

function record(label, ok, secs, out = "", note = "") {
  const r = { label, status: ok ? "ok" : "FAIL", secs, out, note };
  results.push(r);
  console.log(`[verify] ${r.status.padEnd(4)} ${label} (${secs}s)${note ? ` ${note}` : ""}`);
  return r;
}

function skip(label, why) {
  results.push({ label, status: "skip", secs: "", out: "", note: why });
}

function exec(cmd, args, cwd = root) {
  const started = Date.now();
  return new Promise((done) => {
    // pnpm is a .cmd shim on Windows, which spawn only finds through a shell.
    const child = spawn(cmd, args, { cwd, env, stdio: ["ignore", "pipe", "pipe"], shell: isWindows });
    let out = "";
    child.stdout.on("data", (d) => (out += d));
    child.stderr.on("data", (d) => (out += d));
    child.on("error", (e) => (out += `\n${e.message}`));
    child.on("close", (code) => done({ ok: code === 0, secs: ((Date.now() - started) / 1000).toFixed(1), out }));
  });
}

async function check(label, cmd, args, cwd) {
  const r = await exec(cmd, args, cwd);
  return record(label, r.ok, r.secs, r.out);
}

async function rustChain() {
  if (!wants("rust")) {
    for (const l of ["cargo fmt", "cargo clippy", "clippy baseline", "cargo test"]) skip(l, "no Rust changes");
    return;
  }
  if (fix) await check("cargo fmt (fix)", "cargo", ["fmt", "--all"], tauriDir);
  await check("cargo fmt", "cargo", ["fmt", "--all", "--", "--check"], tauriDir);
  await check("cargo clippy", "cargo", ["clippy", "--workspace", "--all-targets"], tauriDir);
  await check("clippy baseline", "node", [join("scripts", "clippy-baseline.mjs"), "--check"]);
  await check("cargo test", "cargo", ["test", "--workspace", "--no-fail-fast"], tauriDir);
}

function webChecks() {
  if (!wants("web")) {
    for (const l of ["pnpm check", "pnpm test", "pnpm check:i18n"]) skip(l, "no web changes");
    return [];
  }
  return [check("pnpm check", "pnpm", ["-s", "check"]), check("pnpm test", "pnpm", ["-s", "test"]), check("pnpm check:i18n", "pnpm", ["-s", "check:i18n"])];
}

// Suites other scripts define; each runs only when package.json has it.
const optional = [
  ["check:ipc", "ipc", "no src/ or src-tauri/ changes"],
  ["test:extension", "extension", "no browser-extension/ changes"],
  ["test:plugin", "plugin", "no claude-plugin/ changes"],
];

function optionalChecks() {
  const jobs = [];
  for (const [name, area, why] of optional) {
    if (!scripts[name]) continue;
    if (!wants(area)) skip(`pnpm ${name}`, why);
    else jobs.push(check(`pnpm ${name}`, "pnpm", ["-s", name]));
  }
  return jobs;
}

const started = Date.now();
await Promise.all([rustChain(), ...webChecks(), ...optionalChecks()]);

// ---------------------------------------------------------------------------
// Report
// ---------------------------------------------------------------------------

/**
 * The part of a failing check's output worth reading. The tail, plus every
 * test harness "failures:" block: with --no-fail-fast the failing crate's
 * report scrolls far above the tail, which only says "1 target failed".
 */
function digest(out) {
  const lines = out.replace(/\x1b\[[0-9;]*m/g, "").trimEnd().split("\n");
  const blocks = [];
  for (let i = 0; i < lines.length; i++) {
    if (lines[i] !== "failures:" || lines[i + 1] !== "") continue;
    const end = lines.findIndex((l, j) => j > i && l.startsWith("test result: FAILED"));
    const stop = end < 0 ? lines.length : end + 1;
    blocks.push(lines.slice(i, Math.min(stop, i + 200)).join("\n"));
    i = stop - 1;
  }
  const tail = lines.slice(-(blocks.length ? 15 : 80)).join("\n");
  return blocks.length ? `${blocks.join("\n\n")}\n...\n${tail}` : tail;
}

const failed = results.filter((r) => r.status === "FAIL");
for (const r of failed) console.log(`\n── ${r.label} failed ──\n${digest(r.out)}`);

const order = (r) => ({ FAIL: 0, ok: 1, skip: 2 })[r.status];
results.sort((a, b) => order(a) - order(b));
const w = Math.max(5, ...results.map((r) => r.label.length));
console.log(`\n${"check".padEnd(w)}  status  seconds`);
console.log(`${"-".repeat(w)}  ------  -------`);
for (const r of results) {
  console.log(`${r.label.padEnd(w)}  ${r.status.padEnd(6)}  ${String(r.secs).padStart(7)}${r.note ? `  ${r.note}` : ""}`);
}
const ran = results.filter((r) => r.status !== "skip").length;
console.log(
  `\n${failed.length ? `${failed.length} of ${ran} failed` : ran ? `all ${ran} passed` : "nothing to check"}` +
    ` in ${((Date.now() - started) / 1000).toFixed(1)}s${cargoEnvSummary(env)}`,
);
process.exit(failed.length ? 1 : 0);

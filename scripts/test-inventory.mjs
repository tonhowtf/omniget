#!/usr/bin/env node
// Inventory of the Rust test suite: where the tests are, what they cost, and
// which ones need attention.
//
//   node scripts/test-inventory.mjs                  runs cargo nextest in src-tauri, prints Markdown
//   node scripts/test-inventory.mjs --from run.log   reads a saved nextest log instead of running
//   node scripts/test-inventory.mjs --save run.log   runs and also keeps the raw log
//   node scripts/test-inventory.mjs --json           JSON instead of Markdown
//   node scripts/test-inventory.mjs --slow 0.5       threshold for the "slow tests" list (seconds, default 1)
//   node scripts/test-inventory.mjs --top 3          slowest tests shown per area (default 3)
//
// A log for --from needs per-test lines, i.e. a run with `--status-level all`
// (or `pass`), for example:
//   cargo nextest run --workspace --no-fail-fast --status-level all > run.log 2>&1
//
// Per area (crate + first two module segments, e.g. `omniget-core core::llm`)
// it prints the test count, the summed test time and the slowest tests; then
// every test at or above the threshold, every #[ignore] without a reason
// string, and the last git change of each file that holds tests.
import { spawn, spawnSync } from "node:child_process";
import { existsSync, readdirSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, join, relative, sep } from "node:path";
import { createInterface } from "node:readline";
import { fileURLToPath } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const tauriDir = join(root, "src-tauri");

function usage(code) {
  const text = readFileSync(fileURLToPath(import.meta.url), "utf8")
    .split("\n")
    .slice(1)
    .filter((l) => l.startsWith("//"))
    .map((l) => l.replace(/^\/\/ ?/, ""))
    .join("\n");
  (code ? console.error : console.log)(text);
  process.exit(code);
}

const argv = process.argv.slice(2);
const opts = { from: null, save: null, json: false, slow: 1, top: 3 };
for (let i = 0; i < argv.length; i++) {
  const a = argv[i];
  const next = () => {
    if (i + 1 >= argv.length) usage(2);
    return argv[++i];
  };
  if (a === "--from") opts.from = next();
  else if (a === "--save") opts.save = next();
  else if (a === "--json") opts.json = true;
  else if (a === "--slow") opts.slow = Number(next());
  else if (a === "--top") opts.top = Number(next());
  else if (a === "-h" || a === "--help") usage(0);
  else {
    console.error(`unknown argument: ${a}`);
    usage(2);
  }
}
if (!Number.isFinite(opts.slow) || !Number.isFinite(opts.top)) usage(2);

// ---- 1. the nextest log -------------------------------------------------

function runNextest() {
  const args = ["nextest", "run", "--workspace", "--no-fail-fast", "--status-level", "all", "--final-status-level", "none", "--color", "never"];
  console.error(`running: cargo ${args.join(" ")} (in src-tauri)`);
  const res = spawnSync("cargo", args, { cwd: tauriDir, encoding: "utf8", maxBuffer: 1 << 30, env: process.env });
  if (res.error) {
    console.error(`could not run cargo nextest: ${res.error.message}`);
    console.error("install it with `cargo install cargo-nextest --locked`, or pass --from <log>");
    process.exit(2);
  }
  const log = `${res.stdout ?? ""}\n${res.stderr ?? ""}`;
  if (opts.save) writeFileSync(opts.save, log);
  return { log, status: res.status };
}

const { log, status: runStatus } = opts.from ? { log: readFileSync(opts.from, "utf8"), status: null } : runNextest();

// `        PASS [   0.018s] (   1/3771) omniget-core core::llm::tests::name`
// Older nextest prints no `(n/total)` counter; retries print `TRY 2 PASS`;
// failures can be FAIL, TIMEOUT, SIGSEGV, SIGABRT, LEAK-FAIL...
const ansi = /\x1b\[[0-9;]*m/g;
const lineRe = /^\s*(?:TRY \d+ )?(PASS|FAIL|FLAKY|TIMEOUT|LEAK|LEAK-FAIL|SIG[A-Z]+|ABORT)\b[^[]*\[\s*([\d.]+)s\]\s+(?:\(\s*\d+\/\d+\)\s+)?(\S+)\s+(\S+)\s*$/;
const tests = new Map();
for (const raw of log.split(/\r?\n/)) {
  const m = raw.replace(ansi, "").match(lineRe);
  if (!m) continue;
  const [, verdict, secs, binary, name] = m;
  // A retried test shows up once per try; the last line is the verdict.
  tests.set(`${binary} ${name}`, { binary, name, secs: Number(secs), passed: verdict === "PASS" || verdict === "FLAKY" || verdict === "LEAK" });
}
const skipped = Number(log.match(/(\d+) (?:tests? )?skipped/)?.[1] ?? 0);
if (tests.size === 0) {
  console.error("no per-test lines found in the nextest output (the log needs `--status-level all`).");
  process.exit(2);
}

// ---- 2. crates, areas and the files that hold the tests ------------------

function cargoPackages() {
  const res = spawnSync("cargo", ["metadata", "--no-deps", "--format-version", "1"], { cwd: tauriDir, encoding: "utf8", maxBuffer: 1 << 28 });
  const map = new Map();
  if (res.status !== 0) return map;
  for (const pkg of JSON.parse(res.stdout).packages) {
    map.set(pkg.name, {
      dir: dirname(pkg.manifest_path),
      targets: pkg.targets.map((t) => ({ name: t.name, kind: t.kind, src: t.src_path })),
    });
  }
  return map;
}
const packages = cargoPackages();

// nextest binary ids: `crate` (lib unit tests), `crate::test_file`
// (integration test), `crate::bin/name` (unit tests inside a binary).
function crateRoot(binary) {
  const [crate, rest] = binary.split("::");
  const pkg = packages.get(crate);
  if (!pkg) return { crate, rootFile: null };
  let target;
  if (!rest) target = pkg.targets.find((t) => t.kind.includes("lib")) ?? pkg.targets.find((t) => t.kind.includes("bin") && t.name === crate);
  else if (rest.startsWith("bin/")) target = pkg.targets.find((t) => t.kind.includes("bin") && t.name === rest.slice(4));
  else target = pkg.targets.find((t) => t.name === rest && !t.kind.includes("lib"));
  return { crate, rootFile: target?.src ?? null };
}

// Walks the module path from the crate root to the deepest file that exists:
// `core::llm::session::tests::x` -> src/core/llm/session/tests.rs, else
// src/core/llm/session.rs, else src/core/llm.rs ...
function fileFor(rootFile, modules) {
  if (!rootFile) return null;
  let dir = dirname(rootFile);
  let file = rootFile;
  for (const seg of modules) {
    const flat = join(dir, `${seg}.rs`);
    const nested = join(dir, seg, "mod.rs");
    if (existsSync(flat)) file = flat;
    else if (existsSync(nested)) file = nested;
    else break;
    dir = join(dir, seg);
  }
  return file;
}

const rel = (p) => relative(root, p).split(sep).join("/");
const records = [];
for (const t of tests.values()) {
  const { crate, rootFile } = crateRoot(t.binary);
  const parts = t.name.split("::");
  const modules = parts.slice(0, -1);
  // Integration tests and binaries keep their target in the area (`omniget-cli::mcp_stdio`).
  const area = [t.binary, modules.slice(0, 2).join("::")].filter(Boolean).join(" ");
  const file = fileFor(rootFile, modules);
  records.push({ ...t, crate, area, file: file ? rel(file) : null });
}

// ---- 3. #[ignore] without a reason ---------------------------------------

function* rustFiles(dir) {
  for (const entry of readdirSync(dir, { withFileTypes: true })) {
    if (entry.name === "target" || entry.name === "node_modules" || entry.name.startsWith(".")) continue;
    const p = join(dir, entry.name);
    if (entry.isDirectory()) yield* rustFiles(p);
    else if (entry.name.endsWith(".rs")) yield p;
  }
}

const bareIgnores = [];
for (const file of rustFiles(tauriDir)) {
  const lines = readFileSync(file, "utf8").split(/\r?\n/);
  lines.forEach((line, i) => {
    // `#[ignore]` or `#[cfg_attr(..., ignore)]` as an attribute (not in a comment).
    const code = line.replace(/\/\/.*$/, "");
    if (!/#\[\s*ignore\s*\]/.test(code) && !/#\[cfg_attr\(.*\bignore\s*\)\]/.test(code)) return;
    let fn = null;
    for (let j = i + 1; j < Math.min(lines.length, i + 12); j++) {
      const f = lines[j].match(/\bfn\s+([A-Za-z0-9_]+)/);
      if (f) {
        fn = f[1];
        break;
      }
    }
    bareIgnores.push({ file: rel(file), line: i + 1, test: fn });
  });
}

// ---- 4. last git change of each test file --------------------------------

// One walk of the history, newest first, stopped as soon as every wanted
// file has its date (one `git log -1` per file cost ~10 s for 400 files).
function lastChanges(wanted) {
  const found = new Map();
  if (wanted.size === 0) return Promise.resolve(found);
  return new Promise((resolve) => {
    const git = spawn("git", ["log", "--format=@%cs", "--name-only", "--", "src-tauri"], { cwd: root });
    git.on("error", () => resolve(found));
    const lines = createInterface({ input: git.stdout });
    let date = null;
    lines.on("line", (line) => {
      if (line.startsWith("@")) date = line.slice(1);
      else if (line && date && wanted.has(line) && !found.has(line)) {
        found.set(line, date);
        if (found.size === wanted.size) git.kill();
      }
    });
    lines.on("close", () => resolve(found));
  });
}

// ---- 5. aggregate --------------------------------------------------------

const round = (n) => Math.round(n * 1000) / 1000;
const bySlowest = (a, b) => b.secs - a.secs;
const areas = new Map();
for (const r of records) {
  if (!areas.has(r.area)) areas.set(r.area, []);
  areas.get(r.area).push(r);
}
const areaRows = [...areas.entries()]
  .map(([area, list]) => ({
    area,
    tests: list.length,
    seconds: round(list.reduce((s, r) => s + r.secs, 0)),
    slowest: [...list].sort(bySlowest).slice(0, opts.top).map((r) => ({ test: r.name, seconds: r.secs })),
  }))
  .sort((a, b) => b.seconds - a.seconds);

const files = new Map();
for (const r of records) {
  if (!r.file) continue;
  if (!files.has(r.file)) files.set(r.file, { file: r.file, area: r.area, tests: 0, seconds: 0 });
  const f = files.get(r.file);
  f.tests += 1;
  f.seconds += r.secs;
}
const dates = await lastChanges(new Set(files.keys()));
const fileRows = [...files.values()]
  .map((f) => ({ ...f, seconds: round(f.seconds), lastChange: dates.get(f.file) ?? "uncommitted" }))
  .sort((a, b) => a.area.localeCompare(b.area) || a.file.localeCompare(b.file));

const report = {
  total: {
    tests: records.length,
    failed: records.filter((r) => !r.passed).length,
    skipped,
    seconds: round(records.reduce((s, r) => s + r.secs, 0)),
    wall: log.match(/Summary \[\s*([\d.]+)s\]/)?.[1] ? Number(log.match(/Summary \[\s*([\d.]+)s\]/)[1]) : null,
  },
  areas: areaRows,
  slow: records
    .filter((r) => r.secs >= opts.slow)
    .sort(bySlowest)
    .map((r) => ({ test: `${r.binary} ${r.name}`, seconds: r.secs, file: r.file })),
  failed: records.filter((r) => !r.passed).map((r) => `${r.binary} ${r.name}`),
  ignoresWithoutReason: bareIgnores,
  files: fileRows,
};

// ---- 6. print ------------------------------------------------------------

if (opts.json) {
  console.log(JSON.stringify(report, null, 2));
} else {
  const out = [];
  const cell = (s) => String(s).replace(/\|/g, "\\|");
  const t = report.total;
  out.push("# Rust test inventory", "");
  out.push(`${t.tests} tests, ${t.failed} failed, ${t.skipped} skipped; ${t.seconds.toFixed(1)} s of test time${t.wall != null ? `, ${t.wall.toFixed(1)} s wall` : ""}.`, "");
  out.push("## Per area", "", "| Area | Tests | Seconds | Slowest |", "|---|---:|---:|---|");
  for (const a of report.areas) {
    const slowest = a.slowest.map((s) => `\`${s.test.split("::").slice(-1)[0]}\` ${s.seconds.toFixed(2)} s`).join("<br>");
    out.push(`| ${cell(a.area)} | ${a.tests} | ${a.seconds.toFixed(2)} | ${slowest} |`);
  }
  out.push("", `## Tests at or over ${opts.slow} s (${report.slow.length})`, "");
  if (report.slow.length === 0) out.push("None.");
  else {
    out.push("| Seconds | Test | File |", "|---:|---|---|");
    for (const s of report.slow) out.push(`| ${s.seconds.toFixed(2)} | \`${cell(s.test)}\` | ${s.file ?? "?"} |`);
  }
  if (report.failed.length) {
    out.push("", `## Failed (${report.failed.length})`, "");
    for (const f of report.failed) out.push(`- \`${f}\``);
  }
  out.push("", `## #[ignore] without a reason (${report.ignoresWithoutReason.length})`, "");
  if (report.ignoresWithoutReason.length === 0) out.push("None.");
  for (const g of report.ignoresWithoutReason) out.push(`- ${g.file}:${g.line} \`${g.test ?? "?"}\``);
  out.push("", "## Test files", "", "| File | Area | Tests | Seconds | Last change |", "|---|---|---:|---:|---|");
  for (const f of report.files) out.push(`| ${f.file} | ${cell(f.area)} | ${f.tests} | ${f.seconds.toFixed(2)} | ${f.lastChange} |`);
  console.log(out.join("\n"));
}

// Keep the exit status meaningful when this ran the suite itself.
if (runStatus != null && runStatus !== 0) process.exitCode = 1;

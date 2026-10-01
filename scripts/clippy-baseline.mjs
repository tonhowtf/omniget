#!/usr/bin/env node

import { spawn } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const baselinePath = path.join(root, "src-tauri", `clippy-baseline.${process.platform}.json`);

const mode = process.argv.includes("--update")
  ? "update"
  : process.argv.includes("--check")
    ? "check"
    : null;

if (!mode) {
  console.error("Usage: node scripts/clippy-baseline.mjs <--check|--update>");
  process.exit(2);
}

function runClippy() {
  return new Promise((resolve, reject) => {
    const child = spawn(
      "cargo",
      ["clippy", "--workspace", "--all-targets", "--message-format=json"],
      { cwd: path.join(root, "src-tauri"), shell: process.platform === "win32" },
    );

    let stdout = "";
    let stderr = "";
    child.stdout.on("data", (d) => (stdout += d));
    child.stderr.on("data", (d) => (stderr += d));
    child.on("error", reject);
    child.on("close", (code) => {
      if (code !== 0) {
        reject(new Error(`cargo clippy failed (exit ${code}):\n${stderr.slice(-4000)}`));
        return;
      }
      resolve(stdout);
    });
  });
}

function tally(stdout) {
  const counts = {};
  const seen = new Set();
  for (const line of stdout.split("\n")) {
    const trimmed = line.trim();
    if (!trimmed.startsWith("{")) continue;
    let msg;
    try {
      msg = JSON.parse(trimmed);
    } catch {
      continue;
    }
    if (msg.reason !== "compiler-message") continue;
    const message = msg.message ?? {};
    if (message.level !== "warning") continue;
    const lint = message.code?.code;
    if (!lint) continue;

    const primary = (message.spans ?? []).find((s) => s.is_primary);
    const origin = primary
      ? `${primary.file_name}:${primary.line_start}:${primary.column_start}`
      : `nospan:${message.message}`;
    const fingerprint = `${origin}|${lint}`;
    if (seen.has(fingerprint)) continue;
    seen.add(fingerprint);

    const crate = msg.target?.name ?? "unknown";
    const key = `${crate}|${lint}`;
    counts[key] = (counts[key] ?? 0) + 1;
  }
  return Object.fromEntries(Object.entries(counts).sort(([a], [b]) => a.localeCompare(b)));
}

const stdout = await runClippy().catch((e) => {
  console.error(e.message);
  process.exit(1);
});
const current = tally(stdout);
const total = Object.values(current).reduce((a, b) => a + b, 0);

if (mode === "update") {
  fs.writeFileSync(
    baselinePath,
    JSON.stringify({ generated_on: process.platform, total, counts: current }, null, 2) + "\n",
  );
  console.log(`clippy baseline written: ${Object.keys(current).length} lints, ${total} warnings`);
  process.exit(0);
}

if (!fs.existsSync(baselinePath)) {
  console.log(
    `no clippy baseline for ${process.platform} yet — not gating.\n` +
      `Commit this as ${path.relative(root, baselinePath)}:\n`,
  );
  console.log(JSON.stringify({ generated_on: process.platform, total, counts: current }, null, 2));
  process.exit(0);
}

const baseline = JSON.parse(fs.readFileSync(baselinePath, "utf8"));
const before = baseline.counts ?? {};

if (baseline.generated_on && baseline.generated_on !== process.platform) {
  console.log(
    `clippy baseline skipped: recorded on ${baseline.generated_on}, running on ${process.platform}`,
  );
  process.exit(0);
}

const worse = [];
const better = [];
for (const key of new Set([...Object.keys(before), ...Object.keys(current)])) {
  const was = before[key] ?? 0;
  const now = current[key] ?? 0;
  if (now > was) worse.push({ key, was, now });
  if (now < was) better.push({ key, was, now });
}

if (worse.length > 0) {
  console.error("clippy regressed against the committed baseline:\n");
  for (const { key, was, now } of worse) {
    const [crate, lint] = key.split("|");
    console.error(`  ${crate}: ${lint}  ${was} -> ${now}  (+${now - was})`);
  }
  console.error(
    "\nFix the new warnings. If they are genuinely acceptable, run" +
      "\n  node scripts/clippy-baseline.mjs --update" +
      "\nand justify the change in the pull request.",
  );
  process.exit(1);
}

if (better.length > 0) {
  console.log("clippy improved — consider refreshing the baseline:\n");
  for (const { key, was, now } of better) {
    const [crate, lint] = key.split("|");
    console.log(`  ${crate}: ${lint}  ${was} -> ${now}`);
  }
  console.log("\n  node scripts/clippy-baseline.mjs --update");
}

console.log(`clippy baseline OK — ${total} warnings, none new`);

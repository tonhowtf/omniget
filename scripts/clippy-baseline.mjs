#!/usr/bin/env node
// Congela os warnings de clippy que já existem e reprova apenas os novos.
//
// Sem isto, `-D warnings` reprovaria toda PR no primeiro dia (o workspace
// carrega ~76 warnings herdados) e o clippy ficaria informativo para sempre.
// Com isto, o portão é real sem exigir que alguém limpe 76 warnings antes de
// poder mexer em qualquer coisa.
//
// Chave de comparação: `crate|lint`. Não inclui arquivo nem linha de propósito
// — mover código não pode reprovar a CI, só introduzir warning novo pode.
//
// Uso:
//   node scripts/clippy-baseline.mjs --check    reprova se piorou
//   node scripts/clippy-baseline.mjs --update   regrava o baseline
//
// Roda em um SO só (ubuntu na CI): clippy enxerga código diferente por
// `#[cfg(target_os)]`, então um baseline único não descreve as três
// plataformas. Um baseline por SO seria possível, e é dívida registrada.

import { spawn } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const baselinePath = path.join(root, "src-tauri", "clippy-baseline.json");

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
      // clippy sai != 0 em erro de compilação. Warning sozinho não muda o code,
      // então um exit != 0 aqui é falha real de build e precisa aparecer.
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
    if (!lint) continue; // warning sem código de lint (ex.: nota solta)
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
  console.error(`missing ${path.relative(root, baselinePath)} — run with --update`);
  process.exit(1);
}

const baseline = JSON.parse(fs.readFileSync(baselinePath, "utf8"));
const before = baseline.counts ?? {};

// Comparar entre SOs daria falso positivo: `#[cfg(target_os)]` faz o clippy
// enxergar codigo diferente em cada plataforma. Melhor recusar do que reprovar
// alguem por um warning que so existe no SO dele.
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

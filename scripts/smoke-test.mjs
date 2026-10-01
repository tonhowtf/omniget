#!/usr/bin/env node

import { spawn } from "node:child_process";
import { mkdtempSync, mkdirSync, writeFileSync, copyFileSync, existsSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, basename, resolve } from "node:path";

const BANNER = /OmniGet .* starting — pid \d+, (standard|portable) mode/;
const WINDOW = "[startup] main window created";
const TIMEOUT_MS = 90_000;
const EXIT_AFTER_MS = 6000;

const args = process.argv.slice(2);
const portable = args.includes("--portable");
const binArg = args.find((a) => !a.startsWith("--"));

if (!binArg) {
  console.error("uso: node scripts/smoke-test.mjs <binario> [--portable]");
  process.exit(2);
}
if (!existsSync(binArg)) {
  console.error(`binario nao existe: ${binArg}`);
  process.exit(2);
}

const workdir = mkdtempSync(join(tmpdir(), "omniget-smoke-"));
const fakeProfile = join(workdir, "profile");
mkdirSync(fakeProfile, { recursive: true });

let bin = resolve(binArg);
let appDir = workdir;

if (portable) {
  appDir = join(workdir, "app");
  mkdirSync(appDir, { recursive: true });
  bin = join(appDir, basename(binArg));
  copyFileSync(resolve(binArg), bin);
  if (process.platform !== "win32") {
    const { chmodSync } = await import("node:fs");
    chmodSync(bin, 0o755);
  }
  writeFileSync(join(appDir, "portable.txt"), "");
}

const env = {
  ...process.env,
  RUST_LOG: "info",
  OMNIGET_SMOKE_EXIT_MS: String(EXIT_AFTER_MS),
  LOCALAPPDATA: join(fakeProfile, "Local"),
  APPDATA: join(fakeProfile, "Roaming"),
  XDG_DATA_HOME: join(fakeProfile, "share"),
  HOME: process.platform === "win32" ? process.env.HOME : fakeProfile,
};
if (!portable) {
  env.OMNIGET_DATA_DIR = join(workdir, "data-padrao");
}

console.log(`[smoke] modo: ${portable ? "portatil" : "padrao"}`);
console.log(`[smoke] binario: ${bin}`);

const child = spawn(bin, [], { env, cwd: appDir, stdio: ["ignore", "pipe", "pipe"] });

let out = "";
const cap = (chunk) => {
  const t = chunk.toString();
  out += t;
  process.stdout.write(t);
};
child.stdout.on("data", cap);
child.stderr.on("data", cap);

const timer = setTimeout(() => {
  console.error(`\n[smoke] FALHA: nao terminou em ${TIMEOUT_MS}ms — matando`);
  child.kill("SIGKILL");
}, TIMEOUT_MS);

const code = await new Promise((resolve) => {
  child.on("exit", (c) => {
    clearTimeout(timer);
    resolve(c);
  });
  child.on("error", (e) => {
    clearTimeout(timer);
    console.error(`[smoke] falha ao executar: ${e.message}`);
    resolve(127);
  });
});

const falhas = [];

if (!BANNER.test(out)) {
  falhas.push("o banner de boot nao apareceu — o processo nem chegou a subir");
}
if (!out.includes(WINDOW)) {
  falhas.push(`a janela nao foi criada — "${WINDOW}" ausente na saida`);
}
if (code !== 0) {
  falhas.push(`saiu com codigo ${code}, esperado 0`);
}

if (portable) {
  const vazouParaOPerfil = [
    join(fakeProfile, "Local", "wtf.tonho.omniget"),
    join(fakeProfile, "Roaming", "wtf.tonho.omniget"),
    join(fakeProfile, "share", "wtf.tonho.omniget"),
    join(fakeProfile, "Library", "Application Support", "wtf.tonho.omniget"),
  ].filter((p) => existsSync(p));

  if (vazouParaOPerfil.length > 0) {
    falhas.push(`#209: modo portatil escreveu no perfil do usuario: ${vazouParaOPerfil.join(", ")}`);
  }
  if (!existsSync(join(appDir, "data"))) {
    falhas.push("modo portatil nao criou <app>/data");
  }
  if (process.platform === "win32" && !existsSync(join(appDir, "data", "webview"))) {
    falhas.push("#209: <app>/data/webview nao foi criado — o WebView2 nao foi redirecionado");
  }
}

try {
  rmSync(workdir, { recursive: true, force: true });
} catch {
}

if (falhas.length > 0) {
  console.error("\n[smoke] REPROVADO:");
  for (const f of falhas) console.error(`  - ${f}`);
  process.exit(1);
}

console.log(`\n[smoke] OK — janela criada e saida limpa (modo ${portable ? "portatil" : "padrao"})`);

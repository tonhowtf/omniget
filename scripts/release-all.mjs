#!/usr/bin/env node
import { spawnSync } from "node:child_process";
import { copyFileSync, existsSync, mkdirSync, readFileSync, readdirSync, writeFileSync } from "node:fs";
import { homedir } from "node:os";
import { join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const USAGE = `usage: node scripts/release-all.mjs [options]

The release pipeline of this Mac: gates, then every platform into dist-release/v<version>/,
then SHA256SUMS and latest.json over all of it. Nothing is published.

  --only <stages>     comma list of stages (default: gates,mac-arm64,mac-x64,linux-x64,windows-x64,finalize)
  --skip <stages>     comma list of stages to leave out
  --out <dir>         output root (default: dist-release)
  --notes <file>      patch notes for latest.json (default: docs/releases/v<version>.md if it exists)

Stages:
  gates         pnpm verify (fmt, clippy baseline, tests, svelte-check, i18n)
  mac-arm64     scripts/release-local.mjs --target aarch64-apple-darwin --cli
  mac-x64       scripts/release-local.mjs --target x86_64-apple-darwin --cli
  linux-x64     ubuntu:22.04 container (OrbStack/Docker, linux/amd64) built from the tag: deb, rpm, AppImage, CLI
  linux-arm64   the same on linux/arm64
  windows-x64   cross build with cargo-xwin and makensis: NSIS setup, portable exe, CLI zip (not code signed)
  finalize      SHA256SUMS and latest.json over every file in the release folder

The updater key is TAURI_SIGNING_PRIVATE_KEY or ~/.tauri/omniget-updater.key; its password is
TAURI_SIGNING_PRIVATE_KEY_PASSWORD or the Keychain item "omniget-updater". Containers get them
through the environment, never on the command line.`;

const ALL = ["gates", "mac-arm64", "mac-x64", "linux-x64", "windows-x64", "finalize"];
const KNOWN = [...ALL, "linux-arm64"];

const root = resolve(fileURLToPath(new URL("..", import.meta.url)));
const args = process.argv.slice(2);
if (args.includes("--help") || args.includes("-h")) {
  console.log(USAGE);
  process.exit(0);
}
const flag = (name) => {
  const i = args.indexOf(name);
  return i >= 0 ? args[i + 1] : undefined;
};
const list = (name) => (flag(name) ?? "").split(",").map((s) => s.trim()).filter(Boolean);

function fail(message) {
  console.error(`release-all: ${message}`);
  process.exit(1);
}

function run(cmd, cmdArgs, opts = {}) {
  console.log(`$ ${cmd} ${cmdArgs.join(" ")}`);
  const r = spawnSync(cmd, cmdArgs, { cwd: root, stdio: "inherit", ...opts });
  if (r.status !== 0) fail(`${cmd} exited with ${r.status}`);
}

function capture(cmd, cmdArgs, opts = {}) {
  const r = spawnSync(cmd, cmdArgs, { cwd: root, encoding: "utf8", maxBuffer: 64 << 20, ...opts });
  return r.status === 0 ? r.stdout.trim() : null;
}

const only = list("--only");
const skip = list("--skip");
for (const s of [...only, ...skip]) if (!KNOWN.includes(s)) fail(`unknown stage ${s}`);
const stages = (only.length ? only : ALL).filter((s) => !skip.includes(s));

const version = JSON.parse(readFileSync(join(root, "package.json"), "utf8")).version;
const tag = `v${version}`;
const outRoot = resolve(root, flag("--out") ?? "dist-release");
const outDir = join(outRoot, tag);
const defaultNotes = join(root, "docs", "releases", `${tag}.md`);
const notes = flag("--notes") ?? (existsSync(defaultNotes) ? defaultNotes : undefined);
const workDir = join(outRoot, ".work");

function signingEnv() {
  const env = {};
  env.TAURI_SIGNING_PRIVATE_KEY = process.env.TAURI_SIGNING_PRIVATE_KEY;
  if (!env.TAURI_SIGNING_PRIVATE_KEY) {
    const file = join(homedir(), ".tauri", "omniget-updater.key");
    if (!existsSync(file)) fail(`no updater key: set TAURI_SIGNING_PRIVATE_KEY or create ${file}`);
    env.TAURI_SIGNING_PRIVATE_KEY = readFileSync(file, "utf8").trim();
  }
  env.TAURI_SIGNING_PRIVATE_KEY_PASSWORD = process.env.TAURI_SIGNING_PRIVATE_KEY_PASSWORD;
  if (env.TAURI_SIGNING_PRIVATE_KEY_PASSWORD === undefined) {
    const pw = process.platform === "darwin" ? capture("security", ["find-generic-password", "-s", "omniget-updater", "-w"]) : null;
    if (pw === null) fail("no key password: set TAURI_SIGNING_PRIVATE_KEY_PASSWORD or add the Keychain item omniget-updater");
    env.TAURI_SIGNING_PRIVATE_KEY_PASSWORD = pw;
  }
  return env;
}

function releaseLocal(target, extra = [], env = process.env) {
  run(process.execPath, [join(root, "scripts", "release-local.mjs"), "--target", target, "--cli", "--out", outRoot, ...extra], { env });
}

function linux(platform, target) {
  const image = `omniget-release-linux-${platform.split("/")[1]}`;
  run("docker", ["build", "--platform", platform, "-t", image, "-f", join(root, "scripts", "release", "linux.Dockerfile"), join(root, "scripts", "release")]);
  const ref = capture("git", ["rev-parse", "-q", "--verify", `refs/tags/${tag}`]) ? tag : "HEAD";
  if (ref === "HEAD") console.warn(`warning: no tag ${tag}; building HEAD`);
  const inDir = join(workDir, image);
  mkdirSync(inDir, { recursive: true });
  run("git", ["archive", "--format=tar", "-o", join(inDir, "source.tar"), ref]);
  copyFileSync(join(root, "scripts", "release", "linux-build.sh"), join(inDir, "linux-build.sh"));
  mkdirSync(outRoot, { recursive: true });
  const volume = (name, path) => ["-v", `${image}-${name}:${path}`];
  run(
    "docker",
    [
      "run", "--rm", "--platform", platform,
      "-e", "TAURI_SIGNING_PRIVATE_KEY", "-e", "TAURI_SIGNING_PRIVATE_KEY_PASSWORD",
      "-v", `${inDir}:/in:ro`, "-v", `${outRoot}:/out`,
      ...volume("target", "/target"),
      ...volume("cargo-registry", "/usr/local/cargo/registry"),
      ...volume("cargo-git", "/usr/local/cargo/git"),
      ...volume("pnpm-store", "/root/.local/share/pnpm/store"),
      ...volume("corepack", "/root/.cache/node/corepack"),
      image, "bash", "/in/linux-build.sh", target,
    ],
    { env: { ...process.env, ...signingEnv() } },
  );
}

function xwinEnv(target) {
  const llvm = ["/opt/homebrew/opt/llvm/bin", "/usr/local/opt/llvm/bin"].find((p) => existsSync(join(p, "clang-cl")));
  if (!llvm) fail("windows-x64 needs LLVM with clang-cl (brew install llvm lld)");
  if (!capture("which", ["makensis"])) fail("windows-x64 needs makensis (brew install makensis)");
  const base = { ...process.env, PATH: `${llvm}:${process.env.PATH}` };
  const out = capture("cargo", ["xwin", "env", "--target", target], { env: base });
  if (out === null) fail("cargo xwin env failed (cargo install cargo-xwin)");
  const env = { ...base };
  for (const line of out.split("\n")) {
    const m = line.match(/^export ([A-Za-z0-9_]+)="(.*)";$/);
    if (m) env[m[1]] = m[2];
  }
  delete env.CARGO_TARGET_X86_64_PC_WINDOWS_MSVC_RUNNER;
  env.CARGO_TARGET_DIR = process.env.OMNIGET_WINDOWS_TARGET_DIR ?? join(homedir(), ".cache", "omniget", "target-win");
  return env;
}

function finalize() {
  if (!existsSync(outDir)) fail(`${outDir} does not exist`);
  const files = readdirSync(outDir).filter((f) => f.includes(version));
  console.log(`${outDir}:\n  ${files.sort().join("\n  ")}`);
  run(process.execPath, [join(root, "scripts", "make-latest-json.mjs"), outDir, ...(notes ? ["--notes", notes] : [])]);
  const sums = capture("shasum", ["-a", "256", ...files.filter((f) => !f.endsWith(".sig")).sort()], { cwd: outDir });
  if (sums === null) fail("shasum failed");
  writeFileSync(join(outDir, "SHA256SUMS"), `${sums}\n`);
}

const STAGES = {
  gates: () => run("pnpm", ["verify"]),
  "mac-arm64": () => releaseLocal("aarch64-apple-darwin"),
  "mac-x64": () => releaseLocal("x86_64-apple-darwin"),
  "linux-x64": () => linux("linux/amd64", "x86_64-unknown-linux-gnu"),
  "linux-arm64": () => linux("linux/arm64", "aarch64-unknown-linux-gnu"),
  "windows-x64": () => releaseLocal("x86_64-pc-windows-msvc", ["--bundles", "nsis"], xwinEnv("x86_64-pc-windows-msvc")),
  finalize,
};

console.log(`release ${tag}: ${stages.join(", ")}`);
for (const s of stages) {
  const t0 = Date.now();
  console.log(`\n== ${s}`);
  STAGES[s]();
  console.log(`== ${s} ok in ${Math.round((Date.now() - t0) / 1000)} s`);
}

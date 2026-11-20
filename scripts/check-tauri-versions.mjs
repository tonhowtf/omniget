#!/usr/bin/env node
import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");

function crateVersions() {
  const lock = readFileSync(join(root, "src-tauri", "Cargo.lock"), "utf8");
  const found = new Map();
  const re = /\[\[package\]\]\r?\nname = "(tauri(?:-plugin-[a-z0-9-]+)?)"\r?\nversion = "([^"]+)"/g;
  for (const m of lock.matchAll(re)) found.set(m[1], m[2]);
  return found;
}

function npmVersions() {
  const lock = readFileSync(join(root, "pnpm-lock.yaml"), "utf8");
  const found = new Map();
  const re = /^ {6}'(@tauri-apps\/[a-z0-9-]+)':\r?\n {8}specifier: .*\r?\n {8}version: ([0-9][^\s(]*)/gm;
  for (const m of lock.matchAll(re)) found.set(m[1], m[2]);
  return found;
}

const minor = (v) => v.split(".").slice(0, 2).join(".");
const crateFor = (pkg) =>
  pkg === "@tauri-apps/api" ? "tauri" : pkg.startsWith("@tauri-apps/plugin-") ? `tauri-${pkg.slice("@tauri-apps/".length)}` : null;

const crates = crateVersions();
const npm = npmVersions();

if (!crates.has("tauri") || !npm.has("@tauri-apps/api")) {
  console.error("check-tauri-versions: could not read the Tauri versions from the lockfiles");
  process.exit(1);
}

const mismatches = [];
let compared = 0;
for (const [pkg, npmVersion] of npm) {
  const crate = crateFor(pkg);
  if (!crate || !crates.has(crate)) continue;
  compared += 1;
  const crateVersion = crates.get(crate);
  if (minor(npmVersion) !== minor(crateVersion)) {
    mismatches.push(`  ${crate} ${crateVersion} (Cargo.lock)  vs  ${pkg} ${npmVersion} (pnpm-lock.yaml)`);
  }
}

if (mismatches.length > 0) {
  console.error("Tauri crates and npm packages are on different major.minor versions:");
  console.error(mismatches.join("\n"));
  console.error("`tauri build` fails on this. Update the side that stayed behind in the same pull request.");
  process.exit(1);
}

console.log(`check-tauri-versions: ${compared} pairs on matching versions`);

#!/usr/bin/env node

import fs from "fs";
import path from "path";
import { fileURLToPath } from "url";
import { execSync } from "child_process";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const arg = process.argv[2];

if (!arg) {
  console.error("Usage: node scripts/bump-version.js <major|minor|patch|X.Y.Z[-suffix]>");
  process.exit(1);
}

function readCurrentVersion() {
  const pkg = JSON.parse(fs.readFileSync(path.join(root, "package.json"), "utf8"));
  return pkg.version;
}

function bump(current, kind) {
  const match = current.match(/^(\d+)\.(\d+)\.(\d+)(?:-([\w.]+))?$/);
  if (!match) {
    console.error(`Cannot parse current version: ${current}`);
    process.exit(1);
  }
  let [, maj, min, patch] = match;
  maj = Number(maj);
  min = Number(min);
  patch = Number(patch);
  if (kind === "major") {
    maj += 1;
    min = 0;
    patch = 0;
  } else if (kind === "minor") {
    min += 1;
    patch = 0;
  } else if (kind === "patch") {
    patch += 1;
  } else {
    console.error(`Unknown bump kind: ${kind}`);
    process.exit(1);
  }
  return `${maj}.${min}.${patch}`;
}

const current = readCurrentVersion();
let version;
if (["major", "minor", "patch"].includes(arg)) {
  version = bump(current, arg);
} else if (/^\d+\.\d+\.\d+(-[\w.]+)?$/.test(arg)) {
  version = arg;
} else {
  console.error(`Invalid version: ${arg}`);
  console.error("Expected: major | minor | patch | X.Y.Z[-suffix]");
  process.exit(1);
}

console.log(`Bumping ${current} → ${version}`);

const changed = [];

function writeJson(filePath, mutator) {
  const raw = fs.readFileSync(filePath, "utf8");
  const data = JSON.parse(raw);
  mutator(data);
  const out = JSON.stringify(data, null, 2) + (raw.endsWith("\n") ? "\n" : "");
  fs.writeFileSync(filePath, out);
  changed.push(path.relative(root, filePath));
}

function writeText(filePath, mutator) {
  const original = fs.readFileSync(filePath, "utf8");
  const updated = mutator(original);
  if (updated !== original) {
    fs.writeFileSync(filePath, updated);
    changed.push(path.relative(root, filePath));
  }
}

writeJson(path.join(root, "package.json"), (p) => {
  p.version = version;
});

const pkgLockPath = path.join(root, "package-lock.json");
if (fs.existsSync(pkgLockPath)) {
  writeJson(pkgLockPath, (p) => {
    p.version = version;
    if (p.packages && p.packages[""]) {
      p.packages[""].version = version;
    }
  });
}

// The CLI ships in the same release as the app, so `omniget --version` follows it.
const cargoTomlPaths = [
  path.join(root, "src-tauri", "Cargo.toml"),
  path.join(root, "src-tauri", "omniget-cli", "Cargo.toml"),
];
for (const cargoTomlPath of cargoTomlPaths) writeText(cargoTomlPath, (content) => {
  const lines = content.split("\n");
  let inPackage = false;
  let replaced = false;
  for (let i = 0; i < lines.length; i++) {
    const line = lines[i];
    const sectionMatch = line.match(/^\[([^\]]+)\]\s*$/);
    if (sectionMatch) {
      inPackage = sectionMatch[1].trim() === "package";
      continue;
    }
    if (inPackage && /^version\s*=\s*"[^"]+"/.test(line)) {
      lines[i] = line.replace(/^version\s*=\s*"[^"]+"/, `version = "${version}"`);
      replaced = true;
      break;
    }
  }
  if (!replaced) {
    console.error(`Failed to update version in ${path.relative(root, cargoTomlPath)}`);
    process.exit(1);
  }
  return lines.join("\n");
});

const cargoLockPath = path.join(root, "src-tauri", "Cargo.lock");
if (fs.existsSync(cargoLockPath)) {
  writeText(cargoLockPath, (content) => {
    return content.replace(
      /(name = "omniget(?:-cli)?"\nversion = ")([^"]+)(")/g,
      (_, a, _b, c) => `${a}${version}${c}`
    );
  });
}

const tauriConfPath = path.join(root, "src-tauri", "tauri.conf.json");
writeJson(tauriConfPath, (conf) => {
  conf.version = version;
});

const changelogStorePath = path.join(root, "src", "lib", "stores", "changelog-store.svelte.ts");
if (fs.existsSync(changelogStorePath)) {
  writeText(changelogStorePath, (content) =>
    content.replace(
      /(currentVersion\s*=\s*")[^"]+(";)/g,
      (_, a, c) => `${a}${version}${c}`
    )
  );
}

const aboutProjectPath = path.join(root, "src", "routes", "about", "project", "+page.svelte");
if (fs.existsSync(aboutProjectPath)) {
  writeText(aboutProjectPath, (content) =>
    content.replace(
      /(const\s+APP_VERSION\s*=\s*")[^"]+(";)/,
      (_, a, c) => `${a}${version}${c}`
    )
  );
}

if (changed.length === 0) {
  console.error("No files changed — aborting.");
  process.exit(1);
}

console.log("Updated files:");
for (const file of changed) {
  console.log(`  - ${file}`);
}

function hasBinary(name) {
  const probe = process.platform === "win32" ? "where" : "which";
  try {
    execSync(`${probe} ${name}`, { stdio: "ignore" });
    return true;
  } catch {
    return false;
  }
}

const metainfoAbs = path.join(root, "flatpak", "wtf.tonho.omniget.metainfo.xml");
if (hasBinary("appstreamcli")) {
  try {
    execSync(`appstreamcli validate "${metainfoAbs}"`, { stdio: "inherit" });
    console.log("appstreamcli validate: OK");
  } catch {
    console.error("appstreamcli validate failed — aborting.");
    process.exit(1);
  }
} else {
  console.warn("appstreamcli not found in PATH — skipping metainfo validation.");
}

console.log(`v${version}`);

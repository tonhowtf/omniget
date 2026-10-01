#!/usr/bin/env node
// Writes the updater manifest (latest.json) from release files built locally.
// scripts/release-local.mjs calls it at the end; run it again by hand after
// gathering the bundles of other machines in the same folder.
//
//   node scripts/make-latest-json.mjs <dir> [--base https://getomniget.com/dl] [--notes notes.md]
//
// <dir> holds the updater bundles and their .sig files (omniget_1.0.0_aarch64.app.tar.gz,
// omniget_1.0.0_aarch64.app.tar.gz.sig, ...). URLs point to <base>/v<version>/<file>.
import { readFileSync, readdirSync, writeFileSync } from "node:fs";
import { join, resolve } from "node:path";

const args = process.argv.slice(2);
const dir = args.find((a) => !a.startsWith("--"));
if (!dir) {
  console.error("usage: make-latest-json.mjs <dir> [--base URL] [--notes FILE]");
  process.exit(1);
}
const flag = (name) => {
  const i = args.indexOf(name);
  return i >= 0 ? args[i + 1] : undefined;
};
const base = (flag("--base") ?? "https://getomniget.com/dl").replace(/\/+$/, "");
const notesFile = flag("--notes");

const root = resolve(new URL("..", import.meta.url).pathname);
const version = JSON.parse(readFileSync(join(root, "package.json"), "utf8")).version;

const PLATFORMS = [
  ["aarch64.app.tar.gz.sig", "darwin-aarch64"],
  ["x64.app.tar.gz.sig", "darwin-x86_64"],
  ["universal.app.tar.gz.sig", "darwin-universal"],
  ["amd64.AppImage.sig", "linux-x86_64"],
  ["aarch64.AppImage.sig", "linux-aarch64"],
  ["x64-setup.exe.sig", "windows-x86_64"],
];

const manifest = {
  version,
  notes: notesFile ? readFileSync(notesFile, "utf8").trim() : "",
  pub_date: new Date().toISOString().replace(/\.\d{3}Z$/, "Z"),
  platforms: {},
};

const files = readdirSync(dir);
for (const [suffix, platform] of PLATFORMS) {
  const sig = files.find((f) => f.endsWith(suffix) && f.includes(version));
  if (!sig) continue;
  const bundle = sig.slice(0, -".sig".length);
  if (!files.includes(bundle)) {
    console.error(`missing ${bundle} next to ${sig}`);
    process.exit(1);
  }
  manifest.platforms[platform] = {
    signature: readFileSync(join(dir, sig), "utf8").trim(),
    url: `${base}/v${version}/${bundle}`,
  };
}

// A universal macOS build serves both architectures.
const universal = manifest.platforms["darwin-universal"];
if (universal) {
  manifest.platforms["darwin-aarch64"] ??= universal;
  manifest.platforms["darwin-x86_64"] ??= universal;
  delete manifest.platforms["darwin-universal"];
}

if (Object.keys(manifest.platforms).length === 0) {
  console.error(`no updater bundles for ${version} in ${dir}`);
  process.exit(1);
}
writeFileSync(join(dir, "latest.json"), JSON.stringify(manifest, null, 2) + "\n");
console.log(`latest.json: ${version}, ${Object.keys(manifest.platforms).join(", ")}`);

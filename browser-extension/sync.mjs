#!/usr/bin/env node

import fs from "fs";
import path from "path";
import { fileURLToPath } from "url";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const CHROME = path.join(__dirname, "chrome");
const FIREFOX = path.join(__dirname, "firefox");

const SHARED = [
  "src/action-click.js",
  "src/action-feedback.js",
  "src/action-title.js",
  "src/background.js",
  "src/blocked-hosts.js",
  "src/bridge-client.js",
  "src/context-menu.js",
  "src/cookie-summary.js",
  "src/cookies.js",
  "src/cookies-domains.json",
  "src/detect.js",
  "src/hls-grouping.js",
  "src/media-sniffer.js",
  "src/open-app-toggle.js",
  "src/send-via-scheme.js",
  "src/sniffer-filters.js",
  "src/sniffer-storage.js",
  "src/sniffer-toggle.js",
  "popup/popup.js",
  "popup/popup.css",
  "popup/popup.html",
  "pages/error.html",
  "pages/error.css",
  "pages/error.js",
  "pages/error-content.js",
  "pages/options.html",
  "pages/options.css",
  "pages/options.js",
];

const SHARED_DIRS = ["_locales", "icons"];

function copyFile(rel) {
  const from = path.join(CHROME, rel);
  const to = path.join(FIREFOX, rel);
  if (!fs.existsSync(from)) {
    console.warn(`[skip] missing in chrome: ${rel}`);
    return { changed: false, same: false, skipped: true };
  }
  fs.mkdirSync(path.dirname(to), { recursive: true });
  const a = fs.readFileSync(from);
  if (fs.existsSync(to)) {
    const b = fs.readFileSync(to);
    if (a.equals(b)) return { changed: false, same: true, skipped: false };
  }
  fs.writeFileSync(to, a);
  return { changed: true, same: false, skipped: false };
}

function copyDirRecursive(relDir) {
  const from = path.join(CHROME, relDir);
  const to = path.join(FIREFOX, relDir);
  if (!fs.existsSync(from)) return { changed: 0, same: 0 };
  let changed = 0;
  let same = 0;
  const walk = (srcDir, dstDir) => {
    fs.mkdirSync(dstDir, { recursive: true });
    for (const entry of fs.readdirSync(srcDir, { withFileTypes: true })) {
      const srcPath = path.join(srcDir, entry.name);
      const dstPath = path.join(dstDir, entry.name);
      if (entry.isDirectory()) {
        walk(srcPath, dstPath);
      } else if (entry.isFile()) {
        const a = fs.readFileSync(srcPath);
        if (fs.existsSync(dstPath)) {
          const b = fs.readFileSync(dstPath);
          if (a.equals(b)) {
            same++;
            continue;
          }
        }
        fs.writeFileSync(dstPath, a);
        changed++;
      }
    }
  };
  walk(from, to);
  return { changed, same };
}

let totalChanged = 0;
let totalSame = 0;

for (const rel of SHARED) {
  const r = copyFile(rel);
  if (r.changed) {
    console.log(`[copy] ${rel}`);
    totalChanged++;
  } else if (r.same) {
    totalSame++;
  }
}

for (const rel of SHARED_DIRS) {
  const r = copyDirRecursive(rel);
  if (r.changed > 0) console.log(`[copy] ${rel}/ (${r.changed} file(s))`);
  totalChanged += r.changed;
  totalSame += r.same;
}

console.log(`\nSynced chrome → firefox: ${totalChanged} changed, ${totalSame} already identical.`);
console.log(`Note: manifest.json is intentionally per-browser and not synced.`);

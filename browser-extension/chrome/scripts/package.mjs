#!/usr/bin/env node

import { cpSync, createWriteStream, mkdtempSync, readdirSync, readFileSync, rmSync, statSync, writeFileSync } from "node:fs";
import { basename, join, relative, resolve } from "node:path";
import { tmpdir } from "node:os";
import zlib from "node:zlib";

const EXTENSION_DIR = resolve(import.meta.dirname, "..");

const DEV_ONLY = ["tests", "scripts", "CHPR.md", "README.md", "package.json"];

function parseArgs() {
  const args = process.argv.slice(2);
  let output = null;
  let version = null;

  const outputIndex = args.indexOf("--output");
  if (outputIndex !== -1 && args[outputIndex + 1]) {
    output = resolve(args[outputIndex + 1]);
  }

  const versionIndex = args.indexOf("--version");
  if (versionIndex !== -1 && args[versionIndex + 1]) {
    version = args[versionIndex + 1];
  }

  if (!output) {
    const manifest = JSON.parse(readFileSync(join(EXTENSION_DIR, "manifest.json"), "utf8"));
    output = resolve(`omniget-chrome-extension-v${version || manifest.version}${args.includes("--store") ? "-store" : ""}.zip`);
  }

  return { output, version, store: args.includes("--store") };
}

const STORE_EXCLUDED_HOSTS = /(^|[./])(youtube\.com|youtube-nocookie\.com|youtu\.be|googlevideo\.com|ytimg\.com)\//;

function applyStorePolicy(dir) {
  const manifestPath = join(dir, "manifest.json");
  const manifest = JSON.parse(readFileSync(manifestPath, "utf8"));
  manifest.host_permissions = manifest.host_permissions.filter((pattern) => !STORE_EXCLUDED_HOSTS.test(pattern));
  // WHY: the code only clears a legacy alarm through optional chaining, and the store rejects unused permissions.
  manifest.permissions = manifest.permissions.filter((name) => name !== "alarms");
  writeFileSync(manifestPath, JSON.stringify(manifest, null, 2) + "\n");

  const flagPath = join(dir, "src", "store-policy.js");
  const flag = readFileSync(flagPath, "utf8");
  const patched = flag.replace("export const STORE_BUILD = false;", "export const STORE_BUILD = true;");
  if (patched === flag) throw new Error("store-policy.js: STORE_BUILD flag not found");
  writeFileSync(flagPath, patched);

  const popupPath = join(dir, "popup", "popup.html");
  const popup = readFileSync(popupPath, "utf8");
  const start = popup.indexOf('<label class="toggle-group" title="Force H.264 on YouTube">');
  const end = start === -1 ? -1 : popup.indexOf("</label>", start);
  if (end === -1) throw new Error("popup.html: H.264 toggle not found");
  writeFileSync(popupPath, popup.slice(0, start).trimEnd() + "\n" + popup.slice(end + "</label>".length).replace(/^\n/, ""));

  for (const name of ["h264ify.js", "h264ify-60.js"]) rmSync(join(dir, "src", name), { force: true });
}

function patchManifest(dir, version) {
  const manifestPath = join(dir, "manifest.json");
  const manifest = JSON.parse(readFileSync(manifestPath, "utf8"));
  delete manifest.key;
  if (version) {
    manifest.version = version;
  }
  writeFileSync(manifestPath, JSON.stringify(manifest, null, 2) + "\n");
}

function removeDevFiles(dir) {
  for (const name of DEV_ONLY) {
    const target = join(dir, name);
    rmSync(target, { recursive: true, force: true });
  }
}

function walkDir(dir, root = dir) {
  const entries = [];
  for (const entry of readdirSync(dir, { withFileTypes: true })) {
    const full = join(dir, entry.name);
    if (entry.isDirectory()) {
      entries.push(...walkDir(full, root));
    } else {
      entries.push(relative(root, full));
    }
  }
  return entries;
}

function createZip(sourceDir, outputPath) {
  const files = walkDir(sourceDir);
  const fd = createWriteStream(outputPath);
  const centralEntries = [];
  let offset = 0;

  function writeBuffer(buf) {
    fd.write(buf);
    offset += buf.length;
  }

  for (const relPath of files) {
    const absPath = join(sourceDir, relPath);
    const raw = readFileSync(absPath);
    const compressed = zlib.deflateRawSync(raw);
    const crc = crc32(raw);
    const nameBytes = Buffer.from(relPath.replace(/\\/g, "/"), "utf8");

    const localHeaderOffset = offset;

    const localHeader = Buffer.alloc(30);
    localHeader.writeUInt32LE(0x04034b50, 0);
    localHeader.writeUInt16LE(20, 4);
    localHeader.writeUInt16LE(0, 6);
    localHeader.writeUInt16LE(8, 8);
    localHeader.writeUInt16LE(0, 10);
    localHeader.writeUInt16LE(0, 12);
    localHeader.writeUInt32LE(crc, 14);
    localHeader.writeUInt32LE(compressed.length, 18);
    localHeader.writeUInt32LE(raw.length, 22);
    localHeader.writeUInt16LE(nameBytes.length, 26);
    localHeader.writeUInt16LE(0, 28);

    writeBuffer(localHeader);
    writeBuffer(nameBytes);
    writeBuffer(compressed);

    centralEntries.push({ nameBytes, crc, compressed, raw, localHeaderOffset });
  }

  const centralStart = offset;

  for (const entry of centralEntries) {
    const cdHeader = Buffer.alloc(46);
    cdHeader.writeUInt32LE(0x02014b50, 0);
    cdHeader.writeUInt16LE(20, 4);
    cdHeader.writeUInt16LE(20, 6);
    cdHeader.writeUInt16LE(0, 8);
    cdHeader.writeUInt16LE(8, 10);
    cdHeader.writeUInt16LE(0, 12);
    cdHeader.writeUInt16LE(0, 14);
    cdHeader.writeUInt32LE(entry.crc, 16);
    cdHeader.writeUInt32LE(entry.compressed.length, 20);
    cdHeader.writeUInt32LE(entry.raw.length, 24);
    cdHeader.writeUInt16LE(entry.nameBytes.length, 28);
    cdHeader.writeUInt16LE(0, 30);
    cdHeader.writeUInt16LE(0, 32);
    cdHeader.writeUInt16LE(0, 34);
    cdHeader.writeUInt16LE(0, 36);
    cdHeader.writeUInt32LE(0, 38);
    cdHeader.writeUInt32LE(entry.localHeaderOffset, 42);

    writeBuffer(cdHeader);
    writeBuffer(entry.nameBytes);
  }

  const centralSize = offset - centralStart;

  const eocd = Buffer.alloc(22);
  eocd.writeUInt32LE(0x06054b50, 0);
  eocd.writeUInt16LE(0, 4);
  eocd.writeUInt16LE(0, 6);
  eocd.writeUInt16LE(centralEntries.length, 8);
  eocd.writeUInt16LE(centralEntries.length, 10);
  eocd.writeUInt32LE(centralSize, 12);
  eocd.writeUInt32LE(centralStart, 16);
  eocd.writeUInt16LE(0, 20);

  writeBuffer(eocd);
  fd.end();
}

function crc32(buf) {
  let crc = 0xffffffff;
  for (let i = 0; i < buf.length; i++) {
    crc ^= buf[i];
    for (let j = 0; j < 8; j++) {
      crc = (crc >>> 1) ^ (crc & 1 ? 0xedb88320 : 0);
    }
  }
  return (crc ^ 0xffffffff) >>> 0;
}

const { output, version, store } = parseArgs();

const tempDir = mkdtempSync(join(tmpdir(), "omniget-chrome-ext-"));
const stageDir = join(tempDir, "chrome");

try {
  console.log("Copying extension files...");
  cpSync(EXTENSION_DIR, stageDir, { recursive: true });

  console.log("Patching manifest...");
  patchManifest(stageDir, version);

  if (store) {
    console.log("Applying Chrome Web Store policy...");
    applyStorePolicy(stageDir);
  }

  console.log("Removing dev-only files...");
  removeDevFiles(stageDir);

  console.log(`Creating ${basename(output)}...`);
  createZip(stageDir, output);

  console.log(`Done: ${output}`);
} finally {
  rmSync(tempDir, { recursive: true, force: true });
}

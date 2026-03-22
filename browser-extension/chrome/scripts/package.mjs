#!/usr/bin/env node

/**
 * Packages the Chrome extension into a .zip ready for Chrome Web Store upload.
 *
 * Usage:
 *   node browser-extension/chrome/scripts/package.mjs [--version X.Y.Z] [--output path/to/output.zip]
 *
 * What it does:
 *   1. Copies browser-extension/chrome/ into a temp directory
 *   2. Strips the "key" field from manifest.json (CWS assigns its own)
 *   3. If --version is given, overwrites manifest.json "version" field
 *   4. Removes dev-only files (tests/, scripts/, CHPR.md, README.md, package.json)
 *   5. Creates a .zip archive
 */

import { cpSync, createWriteStream, mkdtempSync, readdirSync, readFileSync, rmSync, statSync, writeFileSync } from "node:fs";
import { basename, join, relative, resolve } from "node:path";
import { tmpdir } from "node:os";
import { Writable } from "node:stream";
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
    output = resolve(`omniget-chrome-extension-v${version || manifest.version}.zip`);
  }

  return { output, version };
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

/** Collect all files recursively, returning paths relative to root. */
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

/**
 * Creates a ZIP archive using only Node.js built-ins (no external tools).
 * Implements the ZIP format (local file headers + central directory + EOCD)
 * with DEFLATE compression via node:zlib.
 */
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
    // Use forward slashes in zip entries (required by spec)
    const nameBytes = Buffer.from(relPath.replace(/\\/g, "/"), "utf8");

    const localHeaderOffset = offset;

    // Local file header (30 bytes + name + compressed data)
    const localHeader = Buffer.alloc(30);
    localHeader.writeUInt32LE(0x04034b50, 0);  // signature
    localHeader.writeUInt16LE(20, 4);           // version needed
    localHeader.writeUInt16LE(0, 6);            // flags
    localHeader.writeUInt16LE(8, 8);            // compression: deflate
    localHeader.writeUInt16LE(0, 10);           // mod time
    localHeader.writeUInt16LE(0, 12);           // mod date
    localHeader.writeUInt32LE(crc, 14);         // crc-32
    localHeader.writeUInt32LE(compressed.length, 18); // compressed size
    localHeader.writeUInt32LE(raw.length, 22);  // uncompressed size
    localHeader.writeUInt16LE(nameBytes.length, 26); // name length
    localHeader.writeUInt16LE(0, 28);           // extra length

    writeBuffer(localHeader);
    writeBuffer(nameBytes);
    writeBuffer(compressed);

    // Save for central directory
    centralEntries.push({ nameBytes, crc, compressed, raw, localHeaderOffset });
  }

  const centralStart = offset;

  for (const entry of centralEntries) {
    const cdHeader = Buffer.alloc(46);
    cdHeader.writeUInt32LE(0x02014b50, 0);     // signature
    cdHeader.writeUInt16LE(20, 4);              // version made by
    cdHeader.writeUInt16LE(20, 6);              // version needed
    cdHeader.writeUInt16LE(0, 8);               // flags
    cdHeader.writeUInt16LE(8, 10);              // compression: deflate
    cdHeader.writeUInt16LE(0, 12);              // mod time
    cdHeader.writeUInt16LE(0, 14);              // mod date
    cdHeader.writeUInt32LE(entry.crc, 16);      // crc-32
    cdHeader.writeUInt32LE(entry.compressed.length, 20); // compressed size
    cdHeader.writeUInt32LE(entry.raw.length, 24); // uncompressed size
    cdHeader.writeUInt16LE(entry.nameBytes.length, 28); // name length
    cdHeader.writeUInt16LE(0, 30);              // extra length
    cdHeader.writeUInt16LE(0, 32);              // comment length
    cdHeader.writeUInt16LE(0, 34);              // disk start
    cdHeader.writeUInt16LE(0, 36);              // internal attrs
    cdHeader.writeUInt32LE(0, 38);              // external attrs
    cdHeader.writeUInt32LE(entry.localHeaderOffset, 42); // offset

    writeBuffer(cdHeader);
    writeBuffer(entry.nameBytes);
  }

  const centralSize = offset - centralStart;

  // End of central directory record
  const eocd = Buffer.alloc(22);
  eocd.writeUInt32LE(0x06054b50, 0);           // signature
  eocd.writeUInt16LE(0, 4);                     // disk number
  eocd.writeUInt16LE(0, 6);                     // disk with CD
  eocd.writeUInt16LE(centralEntries.length, 8); // entries on disk
  eocd.writeUInt16LE(centralEntries.length, 10); // total entries
  eocd.writeUInt32LE(centralSize, 12);          // CD size
  eocd.writeUInt32LE(centralStart, 16);         // CD offset
  eocd.writeUInt16LE(0, 20);                    // comment length

  writeBuffer(eocd);
  fd.end();
}

/** CRC-32 (ISO 3309) — same algorithm as used by ZIP. */
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

const { output, version } = parseArgs();

const tempDir = mkdtempSync(join(tmpdir(), "omniget-chrome-ext-"));
const stageDir = join(tempDir, "chrome");

try {
  console.log("Copying extension files...");
  cpSync(EXTENSION_DIR, stageDir, { recursive: true });

  console.log("Patching manifest...");
  patchManifest(stageDir, version);

  console.log("Removing dev-only files...");
  removeDevFiles(stageDir);

  console.log(`Creating ${basename(output)}...`);
  createZip(stageDir, output);

  console.log(`Done: ${output}`);
} finally {
  rmSync(tempDir, { recursive: true, force: true });
}

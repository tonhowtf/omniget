#!/usr/bin/env node
import { spawnSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import zlib from "node:zlib";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const firefox = path.join(root, "browser-extension", "firefox");

const CRC_TABLE = Array.from({ length: 256 }, (_, n) => {
  let c = n;
  for (let k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
  return c >>> 0;
});

function crc32(buf) {
  let c = 0xffffffff;
  for (const byte of buf) c = CRC_TABLE[(c ^ byte) & 0xff] ^ (c >>> 8);
  return (c ^ 0xffffffff) >>> 0;
}

export function listFiles(dir, base = dir) {
  const out = [];
  for (const entry of fs.readdirSync(dir, { withFileTypes: true }).sort((a, b) => a.name.localeCompare(b.name))) {
    if (entry.name.startsWith(".")) continue;
    const full = path.join(dir, entry.name);
    if (entry.isDirectory()) out.push(...listFiles(full, base));
    else if (entry.isFile()) out.push(path.relative(base, full).split(path.sep).join("/"));
  }
  return out;
}

export function buildZip(files) {
  const locals = [];
  const centrals = [];
  let offset = 0;
  for (const { name, data } of files) {
    const nameBuf = Buffer.from(name, "utf8");
    const deflated = zlib.deflateRawSync(data, { level: 9 });
    const crc = crc32(data);
    const local = Buffer.alloc(30);
    local.writeUInt32LE(0x04034b50, 0);
    local.writeUInt16LE(20, 4);
    local.writeUInt16LE(0x0800, 6);
    local.writeUInt16LE(8, 8);
    local.writeUInt16LE(0x5821, 12);
    local.writeUInt32LE(crc, 14);
    local.writeUInt32LE(deflated.length, 18);
    local.writeUInt32LE(data.length, 22);
    local.writeUInt16LE(nameBuf.length, 26);
    locals.push(local, nameBuf, deflated);
    const central = Buffer.alloc(46);
    central.writeUInt32LE(0x02014b50, 0);
    central.writeUInt16LE(20, 4);
    central.writeUInt16LE(20, 6);
    central.writeUInt16LE(0x0800, 8);
    central.writeUInt16LE(8, 10);
    central.writeUInt16LE(0x5821, 14);
    central.writeUInt32LE(crc, 16);
    central.writeUInt32LE(deflated.length, 20);
    central.writeUInt32LE(data.length, 24);
    central.writeUInt16LE(nameBuf.length, 28);
    central.writeUInt32LE(offset, 42);
    centrals.push(central, nameBuf);
    offset += local.length + nameBuf.length + deflated.length;
  }
  const centralBuf = Buffer.concat(centrals);
  const end = Buffer.alloc(22);
  end.writeUInt32LE(0x06054b50, 0);
  end.writeUInt16LE(files.length, 8);
  end.writeUInt16LE(files.length, 10);
  end.writeUInt32LE(centralBuf.length, 12);
  end.writeUInt32LE(offset, 16);
  return Buffer.concat([...locals, centralBuf, end]);
}

export function amoProblems(manifest) {
  const problems = [];
  const gecko = manifest.browser_specific_settings?.gecko;
  if (!gecko?.id) problems.push("manifest has no browser_specific_settings.gecko.id");
  if (!gecko?.data_collection_permissions) problems.push("manifest has no gecko.data_collection_permissions");
  if (manifest.key) problems.push("manifest carries the Chrome key");
  if (!/^\d+(\.\d+){0,3}$/.test(String(manifest.version))) problems.push("version is not a dotted number");
  return problems;
}

function main() {
  const sync = spawnSync(process.execPath, [path.join(root, "browser-extension", "sync.mjs")], { stdio: "inherit" });
  if (sync.status !== 0) process.exit(sync.status ?? 1);
  const manifest = JSON.parse(fs.readFileSync(path.join(firefox, "manifest.json"), "utf8"));
  const problems = amoProblems(manifest);
  if (problems.length) {
    console.error(problems.join("\n"));
    process.exit(1);
  }
  const names = listFiles(firefox);
  const zip = buildZip(names.map((name) => ({ name, data: fs.readFileSync(path.join(firefox, name)) })));
  const outDir = path.join(root, "dist-release", "firefox");
  fs.mkdirSync(outDir, { recursive: true });
  const out = path.join(outDir, `omniget-firefox-${manifest.version}.zip`);
  fs.writeFileSync(out, zip);
  console.log(`${out} (${names.length} files, ${zip.length} bytes)`);
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) main();

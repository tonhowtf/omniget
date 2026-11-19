#!/usr/bin/env node
import { spawnSync } from "node:child_process";
import { existsSync, mkdirSync, readFileSync, readdirSync, rmSync, statSync, writeFileSync } from "node:fs";
import { createHash } from "node:crypto";
import { homedir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { deflateRawSync } from "node:zlib";

const USAGE = `Usage: node scripts/connectors-build.mjs <output-dir> [--only id,id] [--source dir] [--base-url url] [--url-map file]

Builds <output-dir>/packages/<id>-<version>.omniconnector, <output-dir>/catalog.json and
<output-dir>/catalog.json.sig from the connector folders in ../connectors. Nothing is uploaded.

--only      connectors to include (default: every folder with a manifest.json)
--source    folder with the connectors (default: ../connectors)
--base-url  where the packages will be served (default: https://getomniget.com/connectors/packages)
--url-map   JSON file {"<package file name>": "https://..."} for hosts that pick the file name

The signing key is TAURI_SIGNING_PRIVATE_KEY or ~/.tauri/omniget-updater.key; its password is
TAURI_SIGNING_PRIVATE_KEY_PASSWORD or the Keychain item "omniget-updater".`;

const OPTED_OUT = ["kiwify.com.br", "kiwify.com", "kiwify.app"];
const PACKAGED = ["manifest.json", "GUIDE.md"];

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const args = process.argv.slice(2);

function fail(message) {
  console.error(`error: ${message}`);
  process.exit(1);
}

function flag(name) {
  const i = args.indexOf(name);
  if (i < 0) return undefined;
  const value = args[i + 1];
  if (!value) fail(`${name} needs a value`);
  args.splice(i, 2);
  return value;
}

if (args.includes("-h") || args.includes("--help")) {
  console.log(USAGE);
  process.exit(0);
}

const only = flag("--only")?.split(",").map((s) => s.trim()).filter(Boolean);
const source = resolve(flag("--source") ?? join(root, "..", "connectors"));
const baseUrl = (flag("--base-url") ?? "https://getomniget.com/connectors/packages").replace(/\/+$/, "");
const urlMapFile = flag("--url-map");
const urlMap = urlMapFile ? JSON.parse(readFileSync(resolve(urlMapFile), "utf8")) : {};
for (const u of Object.values(urlMap)) if (!String(u).startsWith("https://")) fail("--url-map urls must be https");
const out = args[0] ? resolve(args[0]) : fail(`missing the output folder\n\n${USAGE}`);
if (!baseUrl.startsWith("https://")) fail("--base-url must be https");

const CRC_TABLE = Array.from({ length: 256 }, (_, n) => {
  let c = n;
  for (let k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
  return c >>> 0;
});

function crc32(buf) {
  let c = 0xffffffff;
  for (const b of buf) c = CRC_TABLE[(c ^ b) & 0xff] ^ (c >>> 8);
  return (c ^ 0xffffffff) >>> 0;
}

function zip(files) {
  const DOS_TIME = 0;
  const DOS_DATE = 0x5021;
  const locals = [];
  const centrals = [];
  let offset = 0;
  for (const [name, body] of files) {
    const nameBuf = Buffer.from(name, "utf8");
    const packed = deflateRawSync(body, { level: 9 });
    const crc = crc32(body);
    const local = Buffer.alloc(30);
    local.writeUInt32LE(0x04034b50, 0);
    local.writeUInt16LE(20, 4);
    local.writeUInt16LE(0x0800, 6);
    local.writeUInt16LE(8, 8);
    local.writeUInt16LE(DOS_TIME, 10);
    local.writeUInt16LE(DOS_DATE, 12);
    local.writeUInt32LE(crc, 14);
    local.writeUInt32LE(packed.length, 18);
    local.writeUInt32LE(body.length, 22);
    local.writeUInt16LE(nameBuf.length, 26);
    const central = Buffer.alloc(46);
    central.writeUInt32LE(0x02014b50, 0);
    central.writeUInt16LE(20, 4);
    central.writeUInt16LE(20, 6);
    central.writeUInt16LE(0x0800, 8);
    central.writeUInt16LE(8, 10);
    central.writeUInt16LE(DOS_TIME, 12);
    central.writeUInt16LE(DOS_DATE, 14);
    central.writeUInt32LE(crc, 16);
    central.writeUInt32LE(packed.length, 20);
    central.writeUInt32LE(body.length, 24);
    central.writeUInt16LE(nameBuf.length, 28);
    central.writeUInt32LE(offset, 42);
    locals.push(local, nameBuf, packed);
    centrals.push(central, nameBuf);
    offset += local.length + nameBuf.length + packed.length;
  }
  const directory = Buffer.concat(centrals);
  const end = Buffer.alloc(22);
  end.writeUInt32LE(0x06054b50, 0);
  end.writeUInt16LE(files.length, 8);
  end.writeUInt16LE(files.length, 10);
  end.writeUInt32LE(directory.length, 12);
  end.writeUInt32LE(offset, 16);
  return Buffer.concat([...locals, directory, end]);
}

function isOptedOut(host) {
  const h = host.trim().replace(/^\*\./, "").toLowerCase();
  return OPTED_OUT.some((d) => h === d || h.endsWith(`.${d}`));
}

function mentionsOptedOut(pattern) {
  const plain = pattern.toLowerCase().replaceAll("\\.", ".");
  return OPTED_OUT.some((d) => plain.includes(d));
}

function validIdentity(m, dir) {
  if (!/^[a-z0-9]([a-z0-9._-]{0,62}[a-z0-9])?$/.test(m.id ?? "") || m.id.includes("..")) fail(`${dir}: invalid id "${m.id}"`);
  if (!m.name) fail(`${m.id}: no name`);
  if (!/^\d+(\.\d+){0,2}/.test(m.version ?? "")) fail(`${m.id}: no version`);
  if (!Array.isArray(m.domains) || !m.domains.length) fail(`${m.id}: no domains`);
  if (!Array.isArray(m.match) || !m.match.length) fail(`${m.id}: no match patterns`);
  if (m.domains.some(isOptedOut) || m.match.some(mentionsOptedOut)) fail(`${m.id}: this site's owner asked OmniGet not to support it`);
}

function descriptionI18n(m) {
  const map = m.description_i18n ?? {};
  if (typeof map !== "object" || Array.isArray(map)) fail(`${m.id}: description_i18n must be an object of locale: text`);
  for (const [locale, text] of Object.entries(map)) {
    if (!/^[A-Za-z0-9-]{1,16}$/.test(locale)) fail(`${m.id}: description_i18n has an invalid locale "${locale}"`);
    if (typeof text !== "string" || !text.trim() || [...text].length > 1000) fail(`${m.id}: description_i18n.${locale} must be text of at most 1000 characters`);
  }
  if (Object.keys(map).length > 32) fail(`${m.id}: description_i18n has more than 32 locales`);
  return map;
}

function buildOne(dir) {
  const manifest = JSON.parse(readFileSync(join(dir, "manifest.json"), "utf8"));
  validIdentity(manifest, dir);
  const entry = manifest.entry ?? "connector.js";
  if (entry.includes("/") || entry.includes("\\")) fail(`${manifest.id}: entry must be a file next to the manifest`);
  const names = [...new Set([...PACKAGED, entry])].filter((n) => existsSync(join(dir, n)));
  if (!names.includes(entry)) fail(`${manifest.id}: ${entry} is missing`);
  const files = names.sort().map((n) => [n, readFileSync(join(dir, n))]);
  const body = zip(files);
  const file = `${manifest.id}-${manifest.version}.omniconnector`;
  writeFileSync(join(out, "packages", file), body);
  const record = {
    id: manifest.id,
    name: manifest.name,
    description: manifest.description ?? "",
    version: manifest.version,
    author: manifest.author ?? "",
    match: manifest.match,
    domains: manifest.domains,
    min_omniget: manifest.min_omniget ?? "",
    sha256: createHash("sha256").update(body).digest("hex"),
    package: urlMap[file] ?? `${baseUrl}/${file}`,
    category: manifest.category ?? "",
    permissions: manifest.permissions ?? [],
  };
  if (manifest.login?.url) record.login = { url: manifest.login.url };
  const translated = descriptionI18n(manifest);
  if (Object.keys(translated).length) record.description_i18n = translated;
  return record;
}

function capture(cmd, cmdArgs) {
  const r = spawnSync(cmd, cmdArgs, { encoding: "utf8" });
  return r.status === 0 ? r.stdout.trim() : null;
}

function signingKey() {
  if (process.env.TAURI_SIGNING_PRIVATE_KEY) return process.env.TAURI_SIGNING_PRIVATE_KEY;
  const file = join(homedir(), ".tauri", "omniget-updater.key");
  if (!existsSync(file)) fail(`no updater key: set TAURI_SIGNING_PRIVATE_KEY or create ${file}`);
  return readFileSync(file, "utf8").trim();
}

function signingPassword() {
  if (process.env.TAURI_SIGNING_PRIVATE_KEY_PASSWORD !== undefined) return process.env.TAURI_SIGNING_PRIVATE_KEY_PASSWORD;
  if (process.platform === "darwin") {
    const pw = capture("security", ["find-generic-password", "-s", "omniget-updater", "-w"]);
    if (pw !== null) return pw;
  }
  fail("no key password: set TAURI_SIGNING_PRIVATE_KEY_PASSWORD or add the Keychain item omniget-updater");
}

function sign(file) {
  const bin = join(root, "node_modules", ".bin", process.platform === "win32" ? "tauri.cmd" : "tauri");
  const r = spawnSync(bin, ["signer", "sign", file], {
    cwd: root,
    encoding: "utf8",
    env: { ...process.env, TAURI_SIGNING_PRIVATE_KEY: signingKey(), TAURI_SIGNING_PRIVATE_KEY_PASSWORD: signingPassword() },
  });
  if (r.status !== 0 || !existsSync(`${file}.sig`)) fail(`signing failed\n${r.stderr ?? ""}`);
}

if (!existsSync(source)) fail(`no connectors at ${source}`);
const dirs = readdirSync(source)
  .filter((name) => statSync(join(source, name)).isDirectory() && existsSync(join(source, name, "manifest.json")))
  .filter((name) => !only || only.includes(name))
  .sort();
for (const id of only ?? []) if (!dirs.includes(id)) fail(`no connector folder named ${id}`);
if (!dirs.length) fail("no connectors to build");

rmSync(join(out, "packages"), { recursive: true, force: true });
rmSync(join(out, "catalog.json"), { force: true });
rmSync(join(out, "catalog.json.sig"), { force: true });
mkdirSync(join(out, "packages"), { recursive: true });

const connectors = dirs.map((name) => buildOne(join(source, name)));
const catalog = { generated_at: Math.floor(Date.now() / 1000), connectors };
const catalogFile = join(out, "catalog.json");
writeFileSync(catalogFile, `${JSON.stringify(catalog, null, 2)}\n`);
sign(catalogFile);

console.log(`${connectors.length} connectors in ${out}`);
for (const c of connectors) console.log(`  ${c.id} ${c.version}  ${c.sha256.slice(0, 12)}`);
console.log("Nothing was uploaded.");

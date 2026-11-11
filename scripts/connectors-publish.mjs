#!/usr/bin/env node
import { spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import { readFileSync, readdirSync, writeFileSync } from "node:fs";
import { basename, dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const USAGE = `Usage: node scripts/connectors-publish.mjs <output-dir> [--only id,id]

Builds the packages, uploads them and the signed catalog to catbox.moe, and writes
<output-dir>/pointer.json. Serve pointer.json at the catalog address the app reads
(https://getomniget.com/connectors/catalog.json); the app follows it and checks the signature.`;

const HOST = "https://catbox.moe/user/api.php";
const here = dirname(fileURLToPath(import.meta.url));
const args = process.argv.slice(2);
if (!args[0] || args.includes("-h") || args.includes("--help")) {
  console.log(USAGE);
  process.exit(args[0] ? 0 : 1);
}
const out = resolve(args[0]);
const rest = args.slice(1);

function fail(message) {
  console.error(`error: ${message}`);
  process.exit(1);
}

function build(extra) {
  const r = spawnSync(process.execPath, [join(here, "connectors-build.mjs"), out, ...rest, ...extra], { stdio: "inherit" });
  if (r.status !== 0) fail("build failed");
}

async function upload(file, asName) {
  const form = new FormData();
  form.set("reqtype", "fileupload");
  form.set("fileToUpload", new Blob([readFileSync(file)]), asName ?? basename(file));
  const res = await fetch(HOST, { method: "POST", body: form });
  const url = (await res.text()).trim();
  if (!res.ok || !url.startsWith("https://")) fail(`upload of ${basename(file)} failed: ${url.slice(0, 200)}`);
  return url;
}

async function sameBytes(url, file) {
  const res = await fetch(url);
  if (!res.ok) return false;
  const sha = (b) => createHash("sha256").update(b).digest("hex");
  return sha(Buffer.from(await res.arrayBuffer())) === sha(readFileSync(file));
}

build([]);
const map = {};
for (const name of readdirSync(join(out, "packages")).sort()) {
  const file = join(out, "packages", name);
  map[name] = await upload(file, name.replace(/\.omniconnector$/, ".zip"));
  if (!(await sameBytes(map[name], file))) fail(`${name}: the host serves different bytes`);
  console.log(`  ${name} -> ${map[name]}`);
}
const mapFile = join(out, "package-urls.json");
writeFileSync(mapFile, `${JSON.stringify(map, null, 2)}\n`);
build(["--url-map", mapFile]);

const catalog = await upload(join(out, "catalog.json"));
const signature = await upload(join(out, "catalog.json.sig"), "catalog.sig.txt");
for (const [url, file] of [[catalog, "catalog.json"], [signature, "catalog.json.sig"]]) {
  if (!(await sameBytes(url, join(out, file)))) fail(`${file}: the host serves different bytes`);
}
writeFileSync(join(out, "pointer.json"), `${JSON.stringify({ catalog, signature }, null, 2)}\n`);
console.log(`catalog   ${catalog}\nsignature ${signature}\npointer   ${join(out, "pointer.json")}`);

#!/usr/bin/env node
import { spawnSync } from "node:child_process";
import { readdirSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const dir = path.join("browser-extension", "chrome", "tests");
const files = readdirSync(path.join(root, dir))
  .filter((name) => name.endsWith(".test.mjs"))
  .sort()
  .map((name) => path.join(dir, name));

if (files.length === 0) {
  console.error(`test:extension: no *.test.mjs files in ${dir}`);
  process.exit(1);
}

const result = spawnSync(process.execPath, ["--test", ...process.argv.slice(2), ...files], {
  cwd: root,
  stdio: "inherit",
});
process.exit(result.status ?? 1);

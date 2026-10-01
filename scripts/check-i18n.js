#!/usr/bin/env node

import fs from "fs";
import path from "path";
import { fileURLToPath } from "url";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const i18nDir = path.join(__dirname, "..", "src", "lib", "i18n");
const enPath = path.join(i18nDir, "en.json");

const STRICT = process.argv.includes("--strict");

function extractKeys(obj, prefix = "") {
  const keys = [];
  for (const [key, value] of Object.entries(obj)) {
    const fullKey = prefix ? `${prefix}.${key}` : key;
    if (typeof value === "object" && value !== null && !Array.isArray(value)) {
      keys.push(...extractKeys(value, fullKey));
    } else {
      keys.push(fullKey);
    }
  }
  return keys;
}

if (!fs.existsSync(enPath)) {
  console.error("Error: en.json not found at", enPath);
  process.exit(1);
}

const enKeySet = new Set(extractKeys(JSON.parse(fs.readFileSync(enPath, "utf-8"))));
const otherLocales = fs.readdirSync(i18nDir).filter((f) => f.endsWith(".json") && f !== "en.json");

let hadMismatch = false;
for (const file of otherLocales) {
  try {
    const localeKeys = new Set(extractKeys(JSON.parse(fs.readFileSync(path.join(i18nDir, file), "utf-8"))));
    const missing = [...enKeySet].filter((k) => !localeKeys.has(k));
    const extra = [...localeKeys].filter((k) => !enKeySet.has(k));
    if (missing.length > 0 || extra.length > 0) {
      hadMismatch = true;
      console.warn(`[${file}] missing: ${missing.length}, extra: ${extra.length}`);
    }
  } catch (err) {
    hadMismatch = true;
    console.warn(`[${file}] failed to parse: ${err.message}`);
  }
}
if (!hadMismatch) {
  console.log(`${enKeySet.size} keys; all ${otherLocales.length} other locales in sync with en.json`);
} else if (STRICT) {
  console.error("i18n strict mode: locales out of sync with en.json — aborting.");
  process.exit(1);
}

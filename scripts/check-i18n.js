#!/usr/bin/env node

import fs from "fs";
import path from "path";
import { fileURLToPath } from "url";
import { hardcodedStrings, missingKeys } from "./i18n-scan.mjs";

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

const UNTRANSLATED_BASELINE = path.join(__dirname, "i18n-untranslated-baseline.json");
const ACCEPT_UNTRANSLATED = process.argv.includes("--accept-untranslated");

function flatValues(obj, prefix = "", out = {}) {
  for (const [key, value] of Object.entries(obj)) {
    const fullKey = prefix ? `${prefix}.${key}` : key;
    if (typeof value === "object" && value !== null && !Array.isArray(value)) flatValues(value, fullKey, out);
    else out[fullKey] = value;
  }
  return out;
}

function looksTranslatable(text) {
  return typeof text === "string" && /[a-z]{3,}.*\s.*[a-z]{3,}/i.test(text.replace(/\{\{\w+\}\}/g, ""));
}

const enValues = flatValues(JSON.parse(fs.readFileSync(enPath, "utf-8")));
const untranslated = {};
for (const file of otherLocales) {
  const locale = file.replace(/\.json$/, "");
  const values = flatValues(JSON.parse(fs.readFileSync(path.join(i18nDir, file), "utf-8")));
  untranslated[locale] = Object.keys(enValues)
    .filter((k) => values[k] === enValues[k] && looksTranslatable(enValues[k]))
    .sort();
}
const baseline = fs.existsSync(UNTRANSLATED_BASELINE)
  ? JSON.parse(fs.readFileSync(UNTRANSLATED_BASELINE, "utf-8"))
  : {};
let newUntranslated = 0;
for (const [locale, keys] of Object.entries(untranslated)) {
  const known = new Set(baseline[locale] ?? []);
  const fresh = keys.filter((k) => !known.has(k));
  newUntranslated += fresh.length;
  for (const k of fresh.slice(0, 20)) console.warn(`[${locale}.json] not translated: ${k}`);
  if (fresh.length > 20) console.warn(`[${locale}.json] … and ${fresh.length - 20} more not translated`);
}
const shrunk = Object.entries(untranslated).some(([l, keys]) => keys.length < (baseline[l]?.length ?? 0));
if (ACCEPT_UNTRANSLATED || shrunk) {
  const next = {};
  for (const [locale, keys] of Object.entries(untranslated)) {
    const known = new Set(baseline[locale] ?? []);
    next[locale] = ACCEPT_UNTRANSLATED ? keys : keys.filter((k) => known.has(k));
  }
  fs.writeFileSync(UNTRANSLATED_BASELINE, JSON.stringify(next, null, 1) + "\n");
}
const total = Object.values(untranslated).reduce((n, k) => n + k.length, 0);
console.log(`${total} texts still in English across ${otherLocales.length} locales (${newUntranslated} new)`);
if (newUntranslated > 0 && STRICT && !ACCEPT_UNTRANSLATED) {
  console.error(
    "i18n strict mode: new texts are the same as English. Translate them, or accept them on purpose with node scripts/check-i18n.js --accept-untranslated.",
  );
  process.exit(1);
}

function walk(dir, out = []) {
  for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
    const full = path.join(dir, entry.name);
    if (entry.isDirectory()) {
      if (entry.name !== "dev" && entry.name !== "_kitchen-sink") walk(full, out);
    } else if (/\.(svelte|ts)$/.test(entry.name) && !/\.test\.ts$/.test(entry.name)) {
      out.push(full);
    }
  }
  return out;
}

const srcDir = path.join(__dirname, "..", "src");
let codeProblems = 0;
for (const file of walk(srcDir)) {
  const source = fs.readFileSync(file, "utf-8");
  const rel = path.relative(path.join(__dirname, ".."), file);
  for (const key of missingKeys(source, enKeySet)) {
    codeProblems++;
    console.warn(`[${rel}] key not in en.json: ${key}`);
  }
  if (file.endsWith(".svelte")) {
    for (const text of hardcodedStrings(source)) {
      codeProblems++;
      console.warn(`[${rel}] text outside i18n: "${text}"`);
    }
  }
}
if (codeProblems > 0 && STRICT) {
  console.error(`i18n strict mode: ${codeProblems} string problem(s) in the code — aborting.`);
  process.exit(1);
}

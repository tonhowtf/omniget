#!/usr/bin/env node
/**
 * Integrity of the execution evidence: PNG signature, SHA-256 and dimensions of every
 * authoritative screenshot, planned case counts per mode, and locale/direction of each case.
 * File integrity only; it is not a visual approval.
 *   node scripts/settings-rebrand/verify-evidence.mjs [--dir=docs/settings-rebrand/execution/evidence]
 */
import { readFileSync, writeFileSync, readdirSync, statSync } from "node:fs";
import { createHash } from "node:crypto";
import path from "node:path";
import { fileURLToPath } from "node:url";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../..");
const option = (name, fallback) => process.argv.find(arg => arg.startsWith(`--${name}=`))?.slice(name.length + 3) ?? fallback;
const base = path.resolve(root, option("dir", "docs/settings-rebrand/execution/evidence"));
if (base.startsWith(path.join(root, "docs/settings-rebrand/evidence"))) throw new Error("Refusing to write into the audit evidence folder");

const files = [];
const walk = dir => { for (const name of readdirSync(dir)) { const full = path.join(dir, name); if (statSync(full).isDirectory()) walk(full); else if (/^coverage-.*\.json$/.test(name)) files.push(full); } };
walk(base);
const reports = files.map(file => ({ file: path.relative(base, file), ...JSON.parse(readFileSync(file, "utf8")) }));
const attempts = reports.flatMap(report => report.rows.map(row => ({ ...row, mode: row.mode || report.mode })));
const rows = [...new Map(attempts.map(row => [row.id, row])).values()];
const pngSignature = Buffer.from([137, 80, 78, 71, 13, 10, 26, 10]);
const images = rows.filter(row => row.screenshot).map(row => {
  try {
    const bytes = readFileSync(row.screenshot);
    const sha256 = createHash("sha256").update(bytes).digest("hex");
    const width = bytes.readUInt32BE(16), height = bytes.readUInt32BE(20);
    return { id: row.id, path: path.relative(root, row.screenshot), bytes: bytes.length, width, height, sha256, valid: bytes.subarray(0, 8).equals(pngSignature) && sha256 === row.sha256 && width === row.viewport.width && height === row.viewport.height };
  } catch (error) { return { id: row.id, path: row.screenshot, valid: false, error: String(error) }; }
});

// Planned sizes of the default runs (see visual-audit.mjs and the handoff).
const SECTION_COUNT = 6 + 3 + 12 + 3 + 11 + 8;
const STATE_CASES = 6 + 6 + 6 + 1 + 5 + 5 + 5 + 5 + 3 + 1 + 5;
const navigationMap = JSON.parse(readFileSync(path.join(root, "docs/settings-rebrand/navigation-map.json"), "utf8"));
const ALIAS_CASES = navigationMap.tabSectionAliases.length + navigationMap.legacyHashAliases.length + navigationMap.acceptanceCases.filter(item => item.expected).length + navigationMap.settingAliases.length;
const expected = {
  baseline: 6 * 12 * 2 * 3,
  sections: SECTION_COUNT * 2 * 2,
  aliases: ALIAS_CASES,
  states: STATE_CASES * 2 * 2,
  conditional: 8 * 2 * 2 * 2,
  focus: 6 * 3,
  navigation: 16 * 4 * 3 + 16 * 2 * 3 * 2,
};
const terminal = new Set(["captured-needs-review", "not-applicable", "focus-missed", "focus-lost", "url-mismatch", "mutation-on-navigation", "assertion-failed", "section-missing", "fixture-incomplete"]);
const checks = Object.fromEntries(Object.entries(expected).map(([mode, planned]) => {
  const items = rows.filter(row => row.mode === mode);
  return [mode, {
    expected: planned, observed: items.length,
    executionComplete: items.length === planned && items.every(row => terminal.has(row.status)),
    statuses: items.reduce((total, row) => ({ ...total, [row.status]: (total[row.status] || 0) + 1 }), {}),
  }];
}));
const result = {
  generatedAt: new Date().toISOString(), evidenceKind: "frontend-harness-synthetic-ipc", noNativeProof: true,
  note: "executionComplete means every planned case ran to a recorded outcome (product findings such as focus-missed are outcomes, not harness failures). failed = the harness could not finish the case after one retry. Integrity is not visual approval.",
  checks, imageCount: images.length, imageFailures: images.filter(image => !image.valid),
  languageMismatches: rows.filter(row => row.measurements?.lang && (row.measurements.lang !== row.language || (row.language === "fa") !== (row.measurements.dir === "rtl"))).map(row => ({ id: row.id, lang: row.measurements.lang, dir: row.measurements.dir })),
  sources: reports.map(report => report.file),
  images,
};
writeFileSync(path.join(base, "coverage-merged.json"), JSON.stringify({ generatedAt: result.generatedAt, noNativeProof: true, rows }, null, 2));
const columns = ["id", "mode", "category", "section", "state", "variant", "language", "theme", "status", "screenshot", "sha256", "error"];
const csv = values => values.map(value => `"${String(value ?? "").replaceAll('"', '""')}"`).join(",");
writeFileSync(path.join(base, "coverage-merged.csv"), [csv([...columns, "viewport"]), ...rows.map(row => csv([...columns.map(key => row[key]), row.viewport?.name]))].join("\n") + "\n");
writeFileSync(path.join(base, "integrity.json"), JSON.stringify(result, null, 2));
console.log(JSON.stringify({ checks, imageCount: result.imageCount, imageFailures: result.imageFailures.length, languageMismatches: result.languageMismatches.length }, null, 2));
const ran = Object.values(checks).filter(check => check.observed > 0);
if (result.imageFailures.length || result.languageMismatches.length || ran.some(check => !check.executionComplete)) process.exitCode = 1;

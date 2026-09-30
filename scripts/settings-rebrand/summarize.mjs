#!/usr/bin/env node
/**
 * Merges every coverage-*.json under the evidence folder (default: the execution evidence,
 * never the audit's docs/settings-rebrand/evidence) and writes summary.json + summary.md there.
 * The latest attempt of each case id is authoritative; earlier failed attempts are counted as retries.
 *   node scripts/settings-rebrand/summarize.mjs [--dir=docs/settings-rebrand/execution/evidence]
 */
import { readFileSync, writeFileSync, readdirSync, statSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../..");
const option = (name, fallback) => process.argv.find(arg => arg.startsWith(`--${name}=`))?.slice(name.length + 3) ?? fallback;
const base = path.resolve(root, option("dir", "docs/settings-rebrand/execution/evidence"));
if (base === path.join(root, "docs/settings-rebrand/evidence") || base.startsWith(path.join(root, "docs/settings-rebrand/evidence") + path.sep)) throw new Error("Refusing to write into the audit evidence folder");

const files = [];
const walk = dir => { for (const name of readdirSync(dir)) { const full = path.join(dir, name); if (statSync(full).isDirectory()) walk(full); else if (/^coverage-.*\.json$/.test(name)) files.push(full); } };
walk(base);
const reports = files.map(file => ({ file: path.relative(base, file), ...JSON.parse(readFileSync(file, "utf8")) }));
const attempts = reports.flatMap(report => report.rows.map(row => ({ ...row, mode: row.mode || report.mode })));
const rows = [...new Map(attempts.map(row => [row.id, row])).values()];
const unique = values => [...new Set(values)];
const count = (items, predicate) => items.filter(predicate).length;
const byStatus = items => items.reduce((total, row) => ({ ...total, [row.status]: (total[row.status] || 0) + 1 }), {});
const tally = values => Object.entries(values.reduce((total, value) => ({ ...total, [value]: (total[value] || 0) + 1 }), {})).sort((a, b) => b[1] - a[1]);
const vp = row => row.viewport?.name || "";

const modes = unique(rows.map(row => row.mode));
const perMode = Object.fromEntries(modes.map(mode => { const items = rows.filter(row => row.mode === mode); return [mode, { cases: items.length, statuses: byStatus(items), screenshots: count(items, row => row.screenshot) }]; }));

const measured = rows.filter(row => row.measurements);
const overflowRows = measured.filter(row => row.measurements.documentOverflow || row.measurements.overflow?.length);
const clippedRows = measured.filter(row => row.measurements.clipped?.length);
const unnamedRows = measured.filter(row => row.measurements.unnamed?.length);
const controlKey = control => `${control.tag}#${control.id || "-"}[${control.key || control.name || "?"}]`;
const leakRows = rows.filter(row => row.englishLeaks?.length);

const nav = rows.filter(row => row.mode === "navigation");
const navSummary = unique(nav.map(row => `${row.id.split("__").at(-2)}|${row.variant}`)).map(key => {
  const [caseName, variant] = key.split("|");
  const items = nav.filter(row => row.id.split("__").at(-2) === caseName && row.variant === variant);
  return { case: caseName, variant, runs: items.length, expectsFocus: items.some(row => row.expect), focusOk: count(items, row => row.focus?.ok && row.focus?.stable !== false), focusMissed: count(items, row => row.status === "focus-missed"), focusLost: count(items, row => row.status === "focus-lost"), mutationsByNavigation: items.reduce((sum, row) => sum + (row.mutationsByNavigation?.length || 0), 0), failed: count(items, row => row.status === "failed") };
});
const aliases = rows.filter(row => row.mode === "aliases");
const focusRows = rows.filter(row => row.mode === "focus");
const assertionResults = focusRows.flatMap(row => (row.checks || []).map(check => ({ case: row.id, language: row.language, category: row.category, name: check.name, status: check.status, error: check.error })));
const states = rows.filter(row => row.mode === "states");

const overview = {
  generatedAt: new Date().toISOString(), evidenceDir: path.relative(root, base), noNativeProof: true,
  sources: reports.map(report => report.file),
  totals: {
    uniqueCases: rows.length, attempts: attempts.length, retriedCases: count(rows, row => row.attempt > 1),
    failedAttempts: count(attempts, row => row.status === "failed"), statuses: byStatus(rows),
    screenshots: unique(rows.filter(row => row.screenshot).map(row => row.screenshot)).length,
  },
  perMode,
  layout: {
    measuredCases: measured.length,
    withHorizontalOverflow: overflowRows.length, documentOverflow: count(measured, row => row.measurements.documentOverflow),
    overflowByCase: overflowRows.map(row => ({ id: row.id, document: row.measurements.documentOverflow, containers: row.measurements.overflow })).slice(0, 60),
    withClippedControls: clippedRows.length,
    clippedControls: tally(clippedRows.flatMap(row => row.measurements.clipped.map(control => `${row.mode}:${row.category || row.id.split("__").at(-2)}:${vp(row)}:${controlKey(control)}`))).slice(0, 60),
  },
  accessibility: {
    withUnnamedControls: unnamedRows.length,
    unnamedControls: tally(unnamedRows.flatMap(row => row.measurements.unnamed.map(control => `${row.category || row.id.split("__").at(-2)}:${controlKey(control)}`))).slice(0, 60),
  },
  i18n: {
    casesWithEnglishLeaks: leakRows.length,
    leaksByLanguage: Object.fromEntries(unique(leakRows.map(row => row.language)).map(language => [language, tally(leakRows.filter(row => row.language === language).flatMap(row => row.englishLeaks.map(leak => `${leak.kind || "untranslated"} ${leak.key} = "${leak.text}"`))).slice(0, 40)])),
    languageMismatches: measured.filter(row => row.measurements.lang && (row.measurements.lang !== row.language || (row.language === "fa") !== (row.measurements.dir === "rtl"))).map(row => ({ id: row.id, lang: row.measurements.lang, dir: row.measurements.dir })),
  },
  navigation: { cases: nav.length, mutationsByNavigation: nav.reduce((sum, row) => sum + (row.mutationsByNavigation?.length || 0), 0), mutatingCommandsSeen: unique(nav.flatMap(row => row.mutationsByNavigation || [])), perCaseVariant: navSummary },
  aliases: { cases: aliases.length, urlOk: count(aliases, row => row.urlOk), urlMismatch: aliases.filter(row => row.urlOk === false).map(row => ({ input: row.input, expected: row.expect, final: row.finalPath })), focusMissed: aliases.filter(row => row.status === "focus-missed").map(row => row.input), focusLost: aliases.filter(row => row.status === "focus-lost").map(row => row.input), mutations: aliases.reduce((sum, row) => sum + (row.mock?.mutations?.length || 0), 0) },
  focus: { suites: focusRows.length, assertions: assertionResults.length, passed: count(assertionResults, item => item.status === "passed"), failed: assertionResults.filter(item => item.status === "failed") },
  states: states.map(row => ({ id: row.id, status: row.status, detail: row.stateExercise?.detail, titlePresent: row.titlePresent, alerts: row.measurements?.alerts, statuses: row.measurements?.statuses?.slice(0, 4), saveBars: row.measurements?.saveBars, retryCheck: row.retryCheck, mutations: row.mock?.mutations })),
  unexpectedCommands: unique(rows.flatMap(row => row.mock?.unexpected || [])),
  secretLeaks: rows.filter(row => row.measurements?.secretLeak?.length).map(row => row.id),
  pageErrors: tally(rows.flatMap(row => row.pageErrors || []).map(text => text.slice(0, 200))).slice(0, 20),
  blockedRequests: unique(rows.flatMap(row => row.blockedRequests || []).map(item => `${item.method} ${item.origin}${item.path}`)),
  failures: rows.filter(row => row.status === "failed").map(row => ({ id: row.id, error: row.error?.slice(0, 300) })),
};
writeFileSync(path.join(base, "summary.json"), JSON.stringify(overview, null, 2));

const lines = [
  `# Harness de Configurações (v3): resumo`, "",
  `Gerado em ${overview.generatedAt}. Chromium/Playwright com IPC sintético contra o Vite em localhost:1431. **Não é prova do app nativo.** Screenshots têm status captured-needs-review; só contam como revisados os abertos e listados no handoff.`, "",
  `- Casos únicos: ${overview.totals.uniqueCases} (tentativas: ${overview.totals.attempts}; tentativas falhas: ${overview.totals.failedAttempts}). Status: ${JSON.stringify(overview.totals.statuses)}.`,
  `- Screenshots: ${overview.totals.screenshots}.`,
  ...Object.entries(perMode).map(([mode, item]) => `- ${mode}: ${item.cases} casos, ${JSON.stringify(item.statuses)}.`), "",
  `## Layout e acessibilidade`, "",
  `- Casos medidos: ${measured.length}. Com overflow horizontal: ${overflowRows.length} (documento: ${overview.layout.documentOverflow}). Com controle cortado fora da área de conteúdo: ${clippedRows.length}. Com controle sem nome acessível (heurística DOM): ${unnamedRows.length}.`,
  ...overview.layout.clippedControls.slice(0, 20).map(([key, n]) => `  - cortado ${n}×: ${key}`),
  ...overview.accessibility.unnamedControls.slice(0, 20).map(([key, n]) => `  - sem nome ${n}×: ${key}`), "",
  `## Texto em inglês fora do en`, "",
  `- Casos com string exatamente igual ao catálogo en e ausente no catálogo do idioma: ${leakRows.length}.`,
  ...Object.entries(overview.i18n.leaksByLanguage).flatMap(([language, items]) => [`- ${language}:`, ...items.slice(0, 8).map(([text, n]) => `  - ${n}× ${text}`)]), "",
  `## Navegação contextual`, "",
  `- Casos: ${nav.length}. Chamadas mutantes causadas por navegação: ${overview.navigation.mutationsByNavigation} ${JSON.stringify(overview.navigation.mutatingCommandsSeen)}.`,
  ...navSummary.map(item => `- ${item.case} / ${item.variant}: ${item.runs} execuções; foco ok ${item.expectsFocus ? item.focusOk : "n/a"}, perdido ${item.focusLost}, não chegou ${item.focusMissed}; mutações ${item.mutationsByNavigation}; falhas ${item.failed}.`), "",
  `## Aliases`, "",
  `- ${aliases.length} casos; URL canônica correta em ${overview.aliases.urlOk}; foco não chegou em ${overview.aliases.focusMissed.length}; foco perdido em ${overview.aliases.focusLost.length}; mutações ${overview.aliases.mutations}.`,
  ...overview.aliases.urlMismatch.slice(0, 30).map(item => `  - ${item.input} → ${item.final} (esperado ${item.expected})`), "",
  `## Teclado`, "",
  `- ${overview.focus.suites} suítes, ${overview.focus.passed}/${overview.focus.assertions} verificações passaram.`,
  ...overview.focus.failed.map(item => `  - FALHOU ${item.name} (${item.language}/${item.category}): ${String(item.error).slice(0, 200)}`), "",
  `## Integridade do harness`, "",
  `- Comandos desconhecidos: ${JSON.stringify(overview.unexpectedCommands)}. Vazamento de segredo sentinela: ${JSON.stringify(overview.secretLeaks)}. Requisições bloqueadas: ${JSON.stringify(overview.blockedRequests)}.`,
  `- Falhas de execução: ${overview.failures.length}.`, ...overview.failures.slice(0, 20).map(item => `  - ${item.id}: ${item.error}`),
];
writeFileSync(path.join(base, "summary.md"), lines.join("\n") + "\n");
console.log(JSON.stringify({ totals: overview.totals, perMode, layout: { overflow: overflowRows.length, clipped: clippedRows.length }, unnamed: unnamedRows.length, leaks: leakRows.length, navigation: { cases: nav.length, mutations: overview.navigation.mutationsByNavigation }, aliases: { cases: aliases.length, urlOk: overview.aliases.urlOk }, focus: { passed: overview.focus.passed, total: overview.focus.assertions } }, null, 2));

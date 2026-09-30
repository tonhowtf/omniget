#!/usr/bin/env node
/**
 * Settings frontend harness (v3). No native app, MCP, credentials, files, providers or hardware.
 * Requires an independently started Vite server on localhost:1431 (only GET/HEAD are sent to it).
 * Modes:
 *   baseline    6 categories × languages × themes × viewports (default 12 × 2 × 3)
 *   sections    every section id of the six categories, scrolled into view
 *   aliases     navigation-map tab/section aliases, legacy hashes, acceptance cases, setting aliases
 *   states      loading/error/pendingApply/recovered/saving/saved/saveFailed/dirty/disabled/empty/partialError
 *   conditional prerequisite toggles and advanced disclosures (fixture + real clicks)
 *   focus       Tab/Shift+Tab walk, disclosure keyboard, reset dialog Escape + focus return, search
 *   navigation  contextual destinations (roster, MCP, music, Home, summaries, settings indicators):
 *               focus after late load and zero mutating calls per navigation
 * Examples:
 *   node scripts/settings-rebrand/visual-audit.mjs --mode=navigation --languages=en,pt,fa
 *   node scripts/settings-rebrand/visual-audit.mjs --mode=baseline
 *   node scripts/settings-rebrand/visual-audit.mjs --mode=states --languages=en,pt --categories=general
 * Screenshots and measurements are evidence of browser rendering, never native persistence.
 */
import { chromium, webkit } from "playwright";
import { readFileSync, mkdirSync, writeFileSync, existsSync } from "node:fs";
import { createHash } from "node:crypto";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { installSyntheticIPC, installDestinationFixtures } from "./mock-ipc.mjs";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../..");
const option = (name, fallback) => process.argv.find(arg => arg.startsWith(`--${name}=`))?.slice(name.length + 3) ?? fallback;
const MODES = ["baseline", "sections", "aliases", "states", "conditional", "focus", "navigation"];
const MODE = option("mode", "baseline");
const RESUME = option("resume", "");
const BASE = "http://localhost:1431";
const OUT_ROOT = path.resolve(ROOT, option("out", "docs/settings-rebrand/execution/evidence"));
const OUT = path.join(OUT_ROOT, option("batch", MODE));
const CASE_TIMEOUT_MS = 30000;
const LATE_MS = Number(option("late-ms", "1500"));
const ALL_LANGUAGES = "en,pt,es,fr,it,ru,el,fa,lo,ja,zh,zh-TW";
const defaults = JSON.parse(readFileSync(path.join(ROOT, "docs/settings-rebrand/defaults-fixture.json"), "utf8"));
const navigationMap = JSON.parse(readFileSync(path.join(ROOT, "docs/settings-rebrand/navigation-map.json"), "utf8"));
const defaultLanguages = { baseline: ALL_LANGUAGES, sections: "en,pt", aliases: "en", states: "en,pt", conditional: "en,pt", focus: "en,fa", navigation: "en,pt,fa" };
const defaultThemes = { baseline: "dark,light", sections: "dark,light", aliases: "dark", states: "dark,light", conditional: "dark,light", focus: "dark", navigation: "dark" };
const defaultViewports = { baseline: "800x560,1100x720,1440x900", navigation: "800x560,1100x720,1440x900" };
const languages = option("languages", defaultLanguages[MODE] || "en").split(",");
const themes = option("themes", defaultThemes[MODE] || "dark").split(",");
const viewports = option("viewports", defaultViewports[MODE] || "1100x720").split(",").map(name => { const [width, height] = name.split("x").map(Number); if (!width || !height) throw new Error("Invalid viewport"); return { name, width, height }; });
const categoryFilter = option("categories", "").split(",").filter(Boolean);
const stateFilter = option("states", "").split(",").filter(Boolean);
const caseFilter = option("cases", "").split(",").filter(Boolean);
const variantFilter = option("variants", "").split(",").filter(Boolean);

// Six categories (06-spec) and the section ids the shell/panels expose (fe-shell/fe-panels handoffs).
const CATEGORIES = ["general", "appearance", "downloads", "ai", "integrations", "advanced"];
const SECTIONS = {
  general: ["language", "startup", "profile", "guidance", "limits-strip", "usage-tray"],
  appearance: ["theme", "accessibility", "typography"],
  downloads: ["files", "quality", "subtitles", "clipboard", "output", "presets", "hotkeys", "cookies", "channels", "rules", "sites", "templates"],
  ai: ["connections", "managers", "execution"],
  integrations: ["extension", "extension-details", "extension-install", "music", "world", "world-advanced", "pet", "league", "rpc", "rpc-advanced", "mcp-server"],
  advanced: ["network", "torrent", "performance", "dependencies", "storage", "expert", "debug", "reset"],
};

const flatten = (object, prefix = "", out = {}) => { for (const [k, v] of Object.entries(object)) { if (v && typeof v === "object") flatten(v, `${prefix}${k}.`, out); else out[prefix + k] = String(v); } return out; };
const catalogs = {};
const catalog = language => (catalogs[language] ||= flatten(JSON.parse(readFileSync(path.join(ROOT, "src/lib/i18n", `${language}.json`), "utf8"))));

const rows = [], assertions = [];
const assetCache = new Map();
let browser;
const slug = value => String(value).replace(/[^a-zA-Z0-9_-]/g, "-").slice(0, 90);
const csv = values => values.map(value => `"${String(value ?? "").replaceAll('"', '""')}"`).join(",");
const sleep = ms => new Promise(resolve => setTimeout(resolve, ms));
const withTimeout = (promise, ms, label) => Promise.race([promise, new Promise((_, reject) => setTimeout(() => reject(new Error(`CASE_TIMEOUT ${ms}ms: ${label}`)), ms))]);

async function contextFor(spec) {
  const context = await browser.newContext({ viewport: { width: spec.viewport.width, height: spec.viewport.height }, colorScheme: spec.theme, deviceScaleFactor: 1, reducedMotion: "reduce", serviceWorkers: "block", locale: spec.language === "zh-TW" ? "zh-TW" : spec.language });
  const blocked = [], errors = [], consoleErrors = [], failedRequests = [];
  await context.routeWebSocket("**/*", socket => {
    const url = new URL(socket.url());
    if (url.host === "localhost:1431" && socket.protocols().includes("vite-hmr")) { socket.connectToServer(); return; }
    blocked.push({ origin: url.origin, path: url.pathname, method: "WEBSOCKET" });
    return socket.close({ code: 1008, reason: "Synthetic settings harness blocks external sockets" });
  });
  await context.route("**/*", async route => {
    const request = route.request(), url = new URL(request.url());
    // Only Vite document/assets. No API endpoints, external origins, or HTTP mutations.
    if (url.origin === BASE && ["GET", "HEAD"].includes(request.method()) && !/^\/(mcp|v1|api)(\/|$)/.test(url.pathname)) {
      // Cache only immutable dependencies and binary assets; /src modules can change under
      // other agents' edits (Vite HMR), so they always come fresh from the server.
      const cacheable = request.method() === "GET" && !request.isNavigationRequest() && (/\/node_modules\//.test(url.pathname) || /\.(png|svg|webp|jpe?g|gif|woff2?|ttf|otf)$/.test(url.pathname));
      if (cacheable) {
        try {
          if (!assetCache.has(url.href)) assetCache.set(url.href, (async () => {
            const response = await route.fetch();
            const headers = response.headers();
            delete headers["content-encoding"]; delete headers["content-length"];
            return { status: response.status(), headers, body: await response.body() };
          })());
          return await route.fulfill(await assetCache.get(url.href));
        } catch {
          assetCache.delete(url.href);
          return route.continue().catch(() => {});
        }
      }
      return route.continue().catch(() => {});
    }
    if (["data:", "blob:"].includes(url.protocol)) return route.continue().catch(() => {});
    blocked.push({ origin: url.origin, path: url.pathname, method: request.method() });
    await route.abort("blockedbyclient").catch(() => {});
  });
  await context.addInitScript(installSyntheticIPC, { defaults, language: spec.language, theme: spec.theme, family: spec.family || "", state: spec.fixtureState || "baseline", settingsState: spec.settingsState || "ready", delays: spec.delays || {} });
  await context.addInitScript(installDestinationFixtures);
  // Focus trail: shows who took focus and when (e.g. a router focus reset after a deep-link focus).
  await context.addInitScript(() => {
    const trail = window.__FOCUS_TRAIL__ = [];
    document.addEventListener("focusin", event => { const el = event.target; trail.push({ at: Math.round(performance.now()), id: el.id || null, tag: el.tagName }); }, true);
  });
  const page = await context.newPage();
  page.setDefaultTimeout(CASE_TIMEOUT_MS);
  page.on("pageerror", error => errors.push(String(error)));
  page.on("console", message => { if (message.type() === "error") consoleErrors.push(message.text().slice(0, 400)); });
  page.on("requestfailed", request => failedRequests.push({ url: request.url(), failure: request.failure() }));
  // No automatic confirmation of deletion or reset during visual coverage.
  page.on("dialog", dialog => dialog.dismiss());
  return { context, page, blocked, errors, consoleErrors, failedRequests };
}

async function settle(page) {
  await page.evaluate(() => document.fonts.ready);
  await page.evaluate(() => Promise.race([
    Promise.allSettled([...document.images].map(image => image.decode())),
    new Promise(resolve => setTimeout(resolve, 3000)),
  ]));
  await sleep(300);
}

async function openSettings(page, href, { expectLoaded = true } = {}) {
  await page.goto(`${BASE}${href}`, { waitUntil: "domcontentloaded", timeout: 20000 });
  if (!expectLoaded) {
    // A read that never resolves may keep the whole shell from rendering: capture what is there.
    await page.locator("#settings-panel-title").waitFor({ timeout: 6000 }).catch(() => {});
    await sleep(1500);
    return;
  }
  await page.locator("#settings-panel-title").waitFor({ timeout: CASE_TIMEOUT_MS });
  if (expectLoaded) await page.waitForFunction(() => !document.querySelector(".settings-skeleton"), null, { timeout: 15000 });
  await settle(page);
}

/** DOM measurements inside the settings shell (or the main app surface outside Settings). */
async function measurements(page) {
  return page.evaluate(() => {
    const visible = node => { const rect = node.getBoundingClientRect(), style = getComputedStyle(node); return rect.width > 0 && rect.height > 0 && style.visibility !== "hidden" && style.display !== "none" && !node.closest("[hidden]"); };
    const root = document.querySelector(".settings-shell") || document.querySelector("main") || document.body;
    const content = document.querySelector(".settings-shell-main") || root;
    const box = content.getBoundingClientRect();
    const controls = [...root.querySelectorAll("button, a[href], input:not([type=hidden]), select, textarea, [role=switch], [role=radio], [role=tab], [role=button]")].filter(visible);
    const text = el => (el?.textContent || "").replace(/\s+/g, " ").trim();
    const name = node => node.getAttribute("aria-label")?.trim()
      || (node.getAttribute("aria-labelledby") || "").split(/\s+/).filter(Boolean).map(id => text(document.getElementById(id))).join(" ").trim()
      || [...(node.labels || [])].map(text).join(" ").trim()
      || (node.matches("button, a, [role=button], [role=tab], [role=radio], [role=switch]") ? (text(node) || [...node.querySelectorAll("img[alt]")].map(img => img.alt).join(" ").trim()) : "")
      || node.getAttribute("title")?.trim() || "";
    const info = node => { const rect = node.getBoundingClientRect(); return { tag: node.tagName, id: node.id, type: node.getAttribute("type"), key: node.closest("[data-setting-key]")?.getAttribute("data-setting-key") || null, name: name(node).slice(0, 120), left: Math.round(rect.left), right: Math.round(rect.right), width: Math.round(rect.width), disabled: node.matches(":disabled") || node.getAttribute("aria-disabled") === "true" }; };
    const inContent = node => content.contains(node);
    const clipped = controls.filter(inContent).filter(node => { const rect = node.getBoundingClientRect(); return rect.right > box.right + 1 || rect.left < box.left - 1; }).map(info);
    const overflowing = [...root.querySelectorAll(".settings-shell-main, .settings-shell-content, .settings-panel-rebrand, .setting-group, .setting-row, .settings-save-bar")].filter(visible).filter(node => node.scrollWidth > node.clientWidth + 2 && !["auto", "scroll"].includes(getComputedStyle(node).overflowX)).map(node => ({ tag: node.tagName, id: node.id, class: String(node.className).slice(0, 80), width: node.clientWidth, scrollWidth: node.scrollWidth }));
    const texts = [];
    const walker = document.createTreeWalker(root, NodeFilter.SHOW_TEXT);
    for (let n = walker.nextNode(); n && texts.length < 800; n = walker.nextNode()) { const value = n.nodeValue.replace(/\s+/g, " ").trim(); if (value.length > 1 && n.parentElement && visible(n.parentElement)) texts.push(value); }
    for (const node of root.querySelectorAll("[placeholder], [aria-label]")) if (visible(node)) texts.push((node.getAttribute("placeholder") || node.getAttribute("aria-label") || "").trim());
    const values = [...document.querySelectorAll("input, textarea")].map(node => node.value).join("\n");
    const active = document.activeElement;
    return {
      path: location.pathname + location.search + location.hash,
      title: text(document.getElementById("settings-panel-title")) || null,
      lang: document.documentElement.lang, dir: document.documentElement.dir || getComputedStyle(document.documentElement).direction,
      dataTheme: document.documentElement.dataset.theme || null, dataPreset: document.documentElement.dataset.preset || null,
      documentOverflow: document.scrollingElement.scrollWidth > document.scrollingElement.clientWidth + 1,
      overflow: overflowing, clipped,
      controls: controls.length, unnamed: controls.filter(node => !name(node)).map(info),
      alerts: [...document.querySelectorAll('[role="alert"]')].filter(visible).map(text).filter(Boolean),
      statuses: [...document.querySelectorAll('[role="status"]')].filter(visible).map(text).filter(Boolean),
      saveBars: [...document.querySelectorAll(".settings-save-bar")].filter(visible).map(text),
      groups: [...document.querySelectorAll("[data-settings-section]")].filter(visible).map(node => node.getAttribute("data-settings-section")),
      secretLeak: ["SYNTHETIC-SECRET-PROXY", "SYNTHETIC-SECRET-BRIDGE"].filter(sentinel => document.body.innerText.includes(sentinel) || values.includes(sentinel)),
      active: active && active !== document.body ? { tag: active.tagName, id: active.id, key: active.closest("[data-setting-key]")?.getAttribute("data-setting-key") || null } : null,
      focusTrail: (window.__FOCUS_TRAIL__ || []).slice(-8),
      texts: [...new Set(texts)],
    };
  });
}

/**
 * Visible English copy in a non-English case. `untranslated`: the string is an English catalog
 * value whose key is missing or still identical in the locale catalog (copy never translated).
 * `renderedEnglish`: the locale has a different translation, yet the English one is on screen.
 * Short tokens (brand names, "HD", "URL") are ignored.
 */
function englishLeaks(language, texts) {
  if (language === "en") return [];
  const en = catalog("en"), local = catalog(language);
  const byValue = new Map();
  for (const [key, value] of Object.entries(en)) { const text = value.trim(); if (!byValue.has(text)) byValue.set(text, []); byValue.get(text).push(key); }
  const worded = value => /[A-Za-z]{3}/.test(value) && (value.length >= 12 || (value.length >= 8 && /\s/.test(value))) && !/^(https?:|\/)/.test(value) && !/\{/.test(value);
  const leaks = [];
  for (const value of texts) {
    const keys = byValue.get(value);
    if (!keys || !worded(value)) continue;
    const untranslated = keys.filter(key => local[key] === undefined || local[key].trim() === value);
    if (untranslated.length === keys.length) leaks.push({ text: value.slice(0, 120), key: keys[0], kind: "untranslated" });
    else if (!Object.values(local).some(item => item.trim() === value)) leaks.push({ text: value.slice(0, 120), key: keys[0], kind: "renderedEnglish" });
  }
  return leaks;
}

async function mockState(page) {
  return page.evaluate(() => { const { settings, ai, ...rest } = window.__SETTINGS_AUDIT__.snapshot(); return { ...rest, calls: rest.calls.map(call => call.command) }; });
}

function caseId(parts) { return ["harness", ...parts].map(slug).join("__"); }

async function shoot(page, row) {
  row.screenshot = path.join(OUT, row.id + ".png");
  await page.screenshot({ path: row.screenshot });
  row.sha256 = createHash("sha256").update(readFileSync(row.screenshot)).digest("hex");
}

/** Runs one case with a 30 s budget; a failed case is retried once and both attempts are recorded. */
async function runCase(spec, body) {
  for (let attempt = 1; attempt <= 2; attempt++) {
    const run = await contextFor(spec);
    const row = { id: spec.id, attempt, evidenceKind: "frontend-harness-synthetic-ipc", mode: MODE, ...spec, viewport: spec.viewport, status: "pending" };
    try {
      await withTimeout(body(run.page, row), CASE_TIMEOUT_MS, spec.id);
      const mock = await mockState(run.page);
      row.mock = { unexpected: [...new Set(mock.unexpected)], mutations: mock.mutations, callCount: mock.calls.length, calls: [...new Set(mock.calls)] };
      if (row.measurements) row.englishLeaks = englishLeaks(spec.language, row.measurements.texts);
      if (!row.status || row.status === "pending") row.status = row.mock.unexpected.length ? "fixture-incomplete" : "captured-needs-review";
    } catch (error) { row.status = "failed"; row.error = String(error).slice(0, 800); }
    finally {
      row.pageErrors = run.errors; row.consoleErrors = run.consoleErrors.slice(0, 10); row.blockedRequests = run.blocked;
      if (row.measurements) { row.visibleTexts = row.measurements.texts; delete row.measurements.texts; }
      await run.context.close().catch(() => {});
    }
    rows.push(row);
    writeReports();
    process.stdout.write(`${rows.length}: ${row.id} [${attempt}]: ${row.status}${row.focus ? ` focus=${row.focus.ok}${row.focus.stable === false ? "(lost)" : ""}` : ""}${row.mock?.mutations?.length ? ` mutations=${row.mock.mutations.join("+")}` : ""}\n`);
    if (row.status !== "failed") return row;
  }
}

// ---------------------------------------------------------------- baseline / sections
async function baselineCase(spec) {
  const href = `/settings?tab=${spec.category}${spec.section ? `&section=${spec.section}` : ""}`;
  return runCase(spec, async (page, row) => {
    await openSettings(page, href);
    if (spec.section) {
      await page.waitForFunction(id => { const group = document.getElementById(`settings-group-${id}`); return !group || document.activeElement === group || group.contains(document.activeElement); }, spec.section, { timeout: 5000 }).catch(() => {});
      row.sectionPresent = await page.locator(`#settings-group-${spec.section}`).count() > 0;
      if (row.sectionPresent) await page.locator(`#settings-group-${spec.section}`).evaluate(node => node.scrollIntoView({ block: "start" }));
      await sleep(200);
    }
    row.measurements = await measurements(page);
    await shoot(page, row);
    if (spec.section && !row.sectionPresent) row.status = "section-missing";
  });
}

// ---------------------------------------------------------------- aliases
function aliasCases() {
  const cases = [];
  for (const alias of navigationMap.tabSectionAliases) cases.push({ kind: "tab-section", input: `/settings?tab=${alias.tab}${alias.section ? `&section=${alias.section}` : ""}`, expect: alias.targetHref, section: alias.targetSection || null });
  for (const alias of navigationMap.legacyHashAliases) cases.push({ kind: "legacy-hash", input: `/settings?tab=${alias.tab}${alias.hash}`, expect: `/settings?tab=${alias.targetTab}&section=${alias.targetSection}`, preserveHash: alias.hash, section: alias.targetSection });
  for (const item of navigationMap.acceptanceCases) if (item.expected) cases.push({ kind: "acceptance", input: item.input, expect: item.expected, section: new URL(item.expected, BASE).searchParams.get("section"), setting: new URL(item.expected, BASE).searchParams.get("setting") });
  for (const alias of navigationMap.settingAliases) cases.push({ kind: "setting", input: alias.existingHref, expect: alias.targetHref, section: alias.targetSection || null, setting: alias.setting });
  return cases;
}

function sameDestination(actual, expected, preserveHash) {
  const a = new URL(actual, BASE), e = new URL(expected, BASE);
  if (a.pathname !== e.pathname) return false;
  for (const key of ["tab", "section", "setting"]) if ((a.searchParams.get(key) || "") !== (e.searchParams.get(key) || "")) return false;
  return preserveHash ? a.hash === preserveHash : true;
}

async function aliasCase(spec) {
  return runCase(spec, async (page, row) => {
    await page.goto(`${BASE}${spec.input}`, { waitUntil: "domcontentloaded", timeout: 20000 });
    await page.waitForFunction(() => document.getElementById("settings-panel-title") || document.getElementById("summaries-form"), null, { timeout: CASE_TIMEOUT_MS });
    await page.waitForFunction(() => !document.querySelector(".settings-skeleton"), null, { timeout: 15000 }).catch(() => {});
    const target = spec.setting ? `[data-setting-key="${spec.setting}"]` : spec.section ? `#settings-group-${spec.section}` : null;
    row.focus = await awaitFocus(page, target, spec.section ? `#settings-group-${spec.section}` : null, 5000);
    await settle(page);
    row.finalPath = await page.evaluate(() => location.pathname + location.search + location.hash);
    row.urlOk = sameDestination(row.finalPath, spec.expect, spec.preserveHash);
    row.measurements = await measurements(page);
    await shoot(page, row);
    const mock = await mockState(page);
    if (!row.urlOk) row.status = "url-mismatch";
    else if (mock.mutations.length) row.status = "mutation-on-navigation";
    else if (target && !row.focus.ok) row.status = "focus-missed";
    else if (target && row.focus.stable === false) row.status = "focus-lost";
  });
}

// ---------------------------------------------------------------- focus waiting
async function awaitFocus(page, selector, fallback, timeout) {
  if (!selector) return { ok: null, expected: null };
  const started = Date.now();
  const test = ([sel, fb]) => {
    const active = document.activeElement;
    const hit = s => s && [...document.querySelectorAll(s)].some(node => node === active || node.contains(active));
    return active && active !== document.body && (hit(sel) ? "target" : hit(fb) ? "fallback" : false);
  };
  const matched = await page.waitForFunction(test, [selector, fallback], { timeout, polling: 50 }).then(handle => handle.jsonValue()).catch(() => false);
  const ms = Date.now() - started;
  // Stability: a router focus reset or a late re-render must not take focus away afterwards.
  await sleep(1500);
  const still = await page.evaluate(test, [selector, fallback]);
  return { ok: !!matched, matched: matched || null, ms, stable: matched ? !!still : null, expected: selector, fallback };
}

// ---------------------------------------------------------------- navigation (contextual destinations)
const NAV_CASES = [
  { id: "roster-skills", url: "/llm/roster?bot=fixture-bot&section=skills", expect: "#bot-section-skills", late: ["llm_roster_list", "assist_bot_get", "assist_bot_capabilities"] },
  { id: "roster-learning", url: "/llm/roster?bot=fixture-bot&section=learning", expect: "#bot-section-learning", late: ["llm_roster_list", "assist_bot_get", "assist_learning_overview"] },
  { id: "roster-missing-bot", url: "/llm/roster?bot=missing-bot&section=skills", expect: null, late: ["llm_roster_list"] },
  { id: "mcp-server-permissions", url: "/llm/mcp?mode=server&section=permissions", expect: "#mcp-section-permissions", late: ["tool_mcp_status", "tool_mcp_clients"] },
  { id: "mcp-server-media", url: "/llm/mcp?mode=server&section=media", expect: "#mcp-section-media", late: ["tool_mcp_status", "tool_mcp_media_budget"] },
  { id: "mcp-invalid", url: "/llm/mcp?mode=root&section=secrets", expect: null, late: ["tool_mcp_status"] },
  { id: "music-eq", url: "/music?view=settings&panel=eq", expect: "#music-eq-panel", late: ["music_status", "music_playlists", "music_tracks"] },
  { id: "music-settings", url: "/music?view=settings", expect: "#music-settings-view", late: ["music_status", "music_playlists", "music_tracks"] },
  { id: "home-download-advanced", url: "/?panel=download-advanced", expect: "#home-advanced-title", late: ["yt_templates_list"] },
  { id: "summaries-form", url: "/llm/summaries?section=form", expect: "#summaries-form", late: ["ai_get_config", "ai_history_list"] },
  { id: "summaries-history", url: "/llm/summaries?section=history", expect: "#summaries-history", late: ["ai_get_config", "ai_history_list"] },
  { id: "summaries-setting", url: "/llm/summaries?section=form&setting=settings.ai.history", expect: '[data-setting-key="settings.ai.history"]', late: ["ai_history_list"] },
  { id: "settings-limits-strip", url: "/settings?tab=general&section=limits-strip", expect: "#settings-group-limits-strip", late: ["settings_get_v3", "limits_strip_get_prefs"] },
  { id: "settings-usage-tray", url: "/settings?tab=general&section=usage-tray", expect: "#settings-group-usage-tray", late: ["settings_get_v3", "usage_tray_view"] },
  { id: "settings-summaries-alias", url: "/settings?tab=ai&setting=settings.ai.summarize_title", expect: '[data-setting-key="settings.ai.summarize_title"]', late: ["settings_get_v3", "ai_history_list"] },
  { id: "settings-unknown-section", url: "/settings?tab=general&section=do-not-exist", expect: null, late: ["settings_get_v3"] },
];
const VARIANTS = ["cold", "cold-late", "client", "client-late"];

async function navigationCase(spec) {
  const late = spec.variant.endsWith("-late");
  const delays = late ? Object.fromEntries([...spec.late, ...(spec.variant.startsWith("cold") ? ["settings_get_v3"] : [])].map(command => [command, LATE_MS])) : {};
  const specWithDelays = { ...spec, delays: spec.variant.startsWith("cold") ? delays : {} };
  return runCase(specWithDelays, async (page, row) => {
    let before = 0;
    if (spec.variant.startsWith("cold")) {
      await page.goto(`${BASE}${spec.url}`, { waitUntil: "domcontentloaded", timeout: 20000 });
    } else {
      // Client-side navigation from a neutral Settings page, like a link inside the app.
      await openSettings(page, "/settings?tab=downloads");
      before = (await mockState(page)).mutations.length;
      await page.evaluate(({ href, delays }) => {
        Object.assign(window.__SETTINGS_AUDIT__.behavior.delay, delays);
        window.__FOCUS_TRAIL__.length = 0;
        const link = document.createElement("a"); link.href = href; link.id = "harness-nav-link"; link.textContent = "harness";
        document.body.append(link); link.click(); link.remove();
      }, { href: spec.url, delays });
    }
    row.focus = await awaitFocus(page, spec.expect, null, late ? 12000 : 8000);
    if (!spec.expect) await sleep(late ? LATE_MS + 1500 : 1500);
    await settle(page);
    row.measurements = await measurements(page);
    await shoot(page, row);
    const mock = await mockState(page);
    row.mutationsByNavigation = mock.mutations.slice(before);
    row.mutationsBeforeNavigation = mock.mutations.slice(0, before);
    if (row.mutationsByNavigation.length) row.status = "mutation-on-navigation";
    else if (spec.expect && !row.focus.ok) row.status = "focus-missed";
    else if (spec.expect && row.focus.stable === false) row.status = "focus-lost";
  });
}

// ---------------------------------------------------------------- states
const firstToggle = { general: "#settings-group-startup", appearance: "#settings-group-accessibility", downloads: "#settings-group-files", integrations: "#settings-group-music", advanced: "#settings-group-performance" };
const dirtyTarget = { appearance: "typography", downloads: "output", ai: "connections", integrations: "rpc-advanced", advanced: "network" };
const STATE_RECIPES = {
  loading: { categories: CATEGORIES, settingsState: "loading", expectLoaded: false },
  error: { categories: CATEGORIES, settingsState: "get-failed", expectLoaded: false },
  pendingApply: { categories: CATEGORIES, settingsState: "pending-effect" },
  recovered: { categories: ["general"], settingsState: "recovered" },
  saving: { categories: Object.keys(firstToggle), hold: ["settings_patch_v3"], action: "toggle" },
  saved: { categories: Object.keys(firstToggle), action: "toggle" },
  saveFailed: { categories: Object.keys(firstToggle), settingsState: "patch-rejected", action: "toggle" },
  dirty: { categories: Object.keys(dirtyTarget), action: "dirty" },
  disabled: { categories: ["ai", "integrations", "advanced"], family: { ai: "ai", integrations: "world", advanced: "dependencies" }, fixtureState: "disabled", scrollTo: { ai: "connections", integrations: "world-advanced", advanced: "network" } },
  // profile_get never resolves null in Rust (ProfileView), so General has no honest empty fixture.
  empty: { categories: ["downloads"], family: { downloads: "channels" }, fixtureState: "empty", scrollTo: { downloads: "channels" } },
  partialError: { categories: ["general", "downloads", "ai", "integrations", "advanced"], family: { general: "profile", downloads: "cookies", ai: "ai", integrations: "world", advanced: "dependencies" }, fixtureState: "error", scrollTo: { general: "profile", downloads: "cookies", ai: "connections", integrations: "world-advanced", advanced: "dependencies" } },
};

async function openGroup(page, id) {
  const group = page.locator(`#settings-group-${id}`);
  if (!await group.count()) return false;
  const toggle = group.locator(":scope > header .setting-disclosure, :scope .setting-disclosure").first();
  if (await toggle.count() && await toggle.getAttribute("aria-expanded") === "false") await toggle.click();
  await group.evaluate(node => node.scrollIntoView({ block: "start" }));
  return true;
}

async function enterState(page, spec, recipe) {
  if (recipe.hold) await page.evaluate(hold => window.__SETTINGS_AUDIT__.behavior.hold.push(...hold), recipe.hold);
  if (recipe.action === "toggle") {
    const scope = page.locator(firstToggle[spec.category]);
    await scope.evaluate(node => node.scrollIntoView({ block: "start" }));
    const control = scope.locator('[role="switch"]:enabled, input[type="checkbox"]:enabled, [role="radio"]').first();
    if (!await control.count()) return { applicability: "not-applicable", detail: `No enabled toggle in ${firstToggle[spec.category]}` };
    const label = await control.evaluate(node => node.getAttribute("aria-label") || node.id);
    await control.click();
    await sleep(spec.state === "saved" ? 1200 : 500);
    return { applicability: "interacted", detail: `Clicked ${label} in ${firstToggle[spec.category]}` };
  }
  if (recipe.action === "dirty") {
    const id = dirtyTarget[spec.category];
    if (!await openGroup(page, id)) return { applicability: "not-applicable", detail: `Group ${id} absent` };
    if (spec.category === "advanced") { const enabled = page.locator("#settings-proxy-enabled"); if (await enabled.count() && await enabled.getAttribute("aria-checked") === "false") await enabled.click(); }
    const select = page.locator(`#settings-group-${id} select:enabled`).first();
    if (spec.category === "appearance" && await select.count()) {
      const values = await select.evaluate(node => [...node.options].map(option => option.value));
      const current = await select.inputValue();
      const next = values.find(value => value && value !== current);
      await select.selectOption(next);
      await sleep(300);
      await page.locator(`#settings-group-${id}`).evaluate(node => node.scrollIntoView({ block: "start" }));
      return { applicability: "interacted", detail: `Selected ${next} in ${await select.getAttribute("id")} (${id}); not saved` };
    }
    const input = page.locator(`#settings-group-${id} input[type="text"]:enabled, #settings-group-${id} input:not([type]):enabled, #settings-group-${id} input[type="url"]:enabled`).first();
    if (!await input.count()) return { applicability: "not-applicable", detail: `No editable text field in ${id}` };
    await input.fill("fixture-draft-value");
    await sleep(300);
    await page.locator(`#settings-group-${id}`).evaluate(node => node.scrollIntoView({ block: "start" }));
    return { applicability: "interacted", detail: `Typed into ${await input.getAttribute("id")} in ${id}; not saved` };
  }
  const target = recipe.scrollTo?.[spec.category];
  if (target) await openGroup(page, target);
  return { applicability: "fixture-state", detail: recipe.settingsState || `${recipe.family?.[spec.category] || ""}:${recipe.fixtureState || ""}` };
}

async function stateCase(spec, recipe) {
  return runCase(spec, async (page, row) => {
    await openSettings(page, `/settings?tab=${spec.category}`, { expectLoaded: recipe.expectLoaded !== false });
    row.titlePresent = await page.locator("#settings-panel-title").count() > 0;
    row.stateExercise = await enterState(page, spec, recipe);
    row.measurements = await measurements(page);
    await shoot(page, row);
    if (spec.state === "pendingApply") {
      // Explicit retry must call settings_retry_effect_v3 exactly once and clear the banner.
      const retry = page.locator(".settings-banner button").last();
      if (await retry.count()) {
        await retry.click(); await sleep(600);
        const mock = await mockState(page);
        row.retryCheck = { retryCalls: mock.calls.filter(name => name === "settings_retry_effect_v3").length, bannerAfter: await page.locator(".settings-banner[role=status]").count() };
      } else row.retryCheck = { retryCalls: 0, bannerAfter: null, note: "no retry button visible" };
    }
    if (row.stateExercise.applicability === "not-applicable") row.status = "not-applicable";
  });
}

// ---------------------------------------------------------------- conditional toggles
const CONDITIONAL = [
  { category: "downloads", section: "subtitles", toggleKey: "settings.download.download_subtitles" },
  { category: "downloads", section: "hotkeys" },
  { category: "downloads", section: "sites" },
  { category: "downloads", section: "output" },
  { category: "advanced", section: "network", toggleId: "settings-proxy-enabled" },
  { category: "integrations", section: "world-advanced" },
  { category: "integrations", section: "rpc-advanced" },
  { category: "integrations", section: "extension-details" },
];

async function conditionalCase(spec) {
  return runCase(spec, async (page, row) => {
    await openSettings(page, `/settings?tab=${spec.category}`);
    const present = await openGroup(page, spec.section);
    if (!present) { row.status = "section-missing"; row.measurements = await measurements(page); await shoot(page, row); return; }
    const before = await page.locator(`#settings-group-${spec.section} input, #settings-group-${spec.section} select, #settings-group-${spec.section} [role=switch]`).evaluateAll(nodes => nodes.filter(n => n.getBoundingClientRect().width > 0).length);
    if (spec.variant === "click") {
      // Real click on the prerequisite switch; the fixture starts with it off.
      const toggle = spec.toggleId ? page.locator(`#${spec.toggleId}`) : spec.toggleKey ? page.locator(`[data-setting-key="${spec.toggleKey}"] [role=switch], [data-setting-key="${spec.toggleKey}"] input[type=checkbox]`).first() : page.locator(`#settings-group-${spec.section} [role=switch]:enabled`).first();
      if (await toggle.count()) { await toggle.click(); await sleep(700); row.toggled = await toggle.evaluate(node => node.getAttribute("aria-label") || node.id); }
      else row.toggled = null;
    }
    await page.locator(`#settings-group-${spec.section}`).evaluate(node => node.scrollIntoView({ block: "start" }));
    await sleep(200);
    row.controlsBefore = before;
    row.controlsAfter = await page.locator(`#settings-group-${spec.section} input, #settings-group-${spec.section} select, #settings-group-${spec.section} [role=switch]`).evaluateAll(nodes => nodes.filter(n => n.getBoundingClientRect().width > 0).length);
    row.measurements = await measurements(page);
    await shoot(page, row);
  });
}

// ---------------------------------------------------------------- keyboard focus
async function focusSuite(spec) {
  return runCase(spec, async (page, row) => {
    const results = row.checks = [];
    const check = async (name, action) => { try { results.push({ name, status: "passed", detail: await action() }); } catch (error) { results.push({ name, status: "failed", error: String(error).slice(0, 600) }); } };
    const describe = () => page.evaluate(() => { const el = document.activeElement, style = getComputedStyle(el), rect = el.getBoundingClientRect(); return { tag: el.tagName, id: el.id, name: (el.getAttribute("aria-label") || el.textContent || "").replace(/\s+/g, " ").trim().slice(0, 60), focusVisible: el.matches(":focus-visible"), indicator: style.outlineStyle !== "none" && parseFloat(style.outlineWidth) > 0 || style.boxShadow !== "none", inViewport: rect.bottom > 0 && rect.top < innerHeight && rect.right > 0 && rect.left < innerWidth }; });
    await openSettings(page, `/settings?tab=${spec.category}`);
    await check("tab-walk-visible-focus", async () => {
      await page.locator(".settings-nav-link[aria-current=page], #settings-nav-select").first().focus();
      const sequence = [];
      for (let n = 0; n < 24; n++) { await page.keyboard.press("Tab"); sequence.push(await describe()); }
      const invisible = sequence.filter(item => item.tag !== "BODY" && (!item.focusVisible || !item.indicator));
      const offscreen = sequence.filter(item => item.tag !== "BODY" && !item.inViewport);
      row.tabSequence = sequence;
      if (invisible.length) throw new Error("Focus without visible indicator: " + JSON.stringify(invisible.slice(0, 5)));
      if (offscreen.length) throw new Error("Focused element left outside viewport: " + JSON.stringify(offscreen.slice(0, 5)));
      return { steps: sequence.length };
    });
    await check("shift-tab-reverses", async () => {
      const forward = [];
      for (let n = 0; n < 4; n++) { await page.keyboard.press("Tab"); forward.push((await describe()).id + "|" + (await describe()).name); }
      const backward = [];
      for (let n = 0; n < 3; n++) { await page.keyboard.press("Shift+Tab"); backward.push((await describe()).id + "|" + (await describe()).name); }
      const expected = forward.slice(0, 3).reverse();
      if (JSON.stringify(backward) !== JSON.stringify(expected)) throw new Error(`Shift+Tab order ${JSON.stringify(backward)} != ${JSON.stringify(expected)}`);
      return { forward, backward };
    });
    await check("disclosure-keyboard", async () => {
      const button = page.locator("#settings-active-panel .setting-disclosure").first();
      if (!await button.count()) return { applicability: "no advanced group in this category" };
      await button.focus();
      const before = await button.getAttribute("aria-expanded");
      await page.keyboard.press("Enter");
      await sleep(250);
      const after = await button.getAttribute("aria-expanded");
      const regionHidden = await page.evaluate(id => document.getElementById(id)?.hidden, await button.getAttribute("aria-controls"));
      if (before === after) throw new Error("Enter did not toggle aria-expanded");
      if ((after === "true") === regionHidden) throw new Error("aria-expanded and region hidden disagree");
      await page.keyboard.press("Enter"); await sleep(150);
      return { before, after, controls: await button.getAttribute("aria-controls"), focusKept: await button.evaluate(el => el === document.activeElement) };
    });
    await check("reset-dialog-escape-returns-focus", async () => {
      const trigger = page.locator(".section-reset button, #settings-reset-section").first();
      await trigger.scrollIntoViewIfNeeded(); await trigger.focus();
      await page.keyboard.press("Enter");
      const dialog = page.locator('dialog[open], [role="dialog"]').first();
      await dialog.waitFor({ timeout: 5000 });
      const focusInside = await dialog.evaluate(node => node.contains(document.activeElement));
      await page.keyboard.press("Escape");
      await dialog.waitFor({ state: "hidden", timeout: 5000 });
      const returned = await trigger.evaluate(el => el === document.activeElement);
      const mock = await mockState(page);
      if (mock.calls.includes("settings_reset_v3")) throw new Error("Escape performed a reset");
      if (!focusInside) throw new Error("Focus did not move into the dialog");
      if (!returned) throw new Error("Escape did not return focus to the trigger: " + JSON.stringify(await describe()));
      return { focusInside, returned };
    });
    await check("search-shortcut-open-and-back", async () => {
      await page.locator("#settings-panel-title").focus();
      await page.keyboard.press("Control+f");
      const search = page.locator(".settings-search-input");
      if (!await search.evaluate(el => el === document.activeElement)) throw new Error("Ctrl+F did not focus the search");
      await search.fill("proxy");
      const result = page.locator(".settings-result").first();
      await result.waitFor({ timeout: 5000 });
      await result.click();
      await page.waitForFunction(() => new URL(location.href).searchParams.get("setting"), null, { timeout: 5000 });
      await sleep(600);
      const landed = await describe();
      const back = page.locator(".search-destination button");
      await back.click();
      await sleep(300);
      const query = await search.inputValue();
      if (query !== "proxy") throw new Error(`Back to results lost the query: "${query}"`);
      return { url: await page.evaluate(() => location.search), landed, restoredQuery: query, focusOnSearch: await search.evaluate(el => el === document.activeElement) };
    });
    row.measurements = await measurements(page);
    await shoot(page, row);
    const mock = await mockState(page);
    row.mutationsDuringFocus = mock.mutations;
    if (results.some(item => item.status === "failed")) row.status = "assertion-failed";
    for (const item of results) assertions.push({ caseId: row.id, ...item });
  });
}

// ---------------------------------------------------------------- reporting
function writeReports() {
  const suffix = `${MODE}-${slug(languages.join("-"))}-${slug(themes.join("-"))}-${slug(viewports.map(v => v.name).join("-"))}`;
  const file = path.join(OUT, `coverage-${suffix}.json`);
  writeFileSync(file, JSON.stringify({ evidenceKind: "frontend-harness-synthetic-ipc", mode: MODE, generatedAt: new Date().toISOString(), base: BASE, noNativeProof: true, resumedFrom: RESUME || null, caseTimeoutMs: CASE_TIMEOUT_MS, concurrency: 1, lateMs: LATE_MS, fixtureOverrides: ["appearance.language", "appearance.mode/theme", "appearance.preset=omniget-desktop", "appearance.migration_version=1", "onboarding_completed=true", "legal_acknowledged=true", "network.proxy.enabled in network family cases", "download prerequisites in expanded cases"], rows, assertions }, null, 2));
  const keys = ["id", "attempt", "mode", "category", "section", "state", "variant", "language", "theme", "status", "screenshot", "sha256", "error"];
  writeFileSync(file.replace(/\.json$/, ".csv"), [csv(keys), ...rows.map(row => csv(keys.map(key => key === "viewport" ? row.viewport?.name : row[key])))].join("\n") + "\n");
}

async function main() {
  if (!MODES.includes(MODE)) throw new Error("Unknown mode " + MODE);
  mkdirSync(OUT, { recursive: true });
  const response = await fetch(BASE, { signal: AbortSignal.timeout(5000) }).catch(error => ({ ok: false, error }));
  if (!response.ok) throw new Error("Vite harness server unavailable on :1431; no native fallback is permitted");
  const completed = RESUME && existsSync(path.resolve(ROOT, RESUME)) ? new Set(JSON.parse(readFileSync(path.resolve(ROOT, RESUME), "utf8")).rows.filter(row => !["failed", "pending"].includes(row.status)).map(row => row.id)) : new Set();
  browser = await (option("engine", "chromium") === "webkit" ? webkit : chromium).launch();
  const tasks = [];
  const add = (spec, fn) => { if (!completed.has(spec.id)) tasks.push(() => fn(spec)); };
  const cats = CATEGORIES.filter(category => !categoryFilter.length || categoryFilter.includes(category));
  try {
    for (const language of languages) for (const theme of themes) for (const viewport of viewports) {
      const common = { language, theme, viewport };
      if (MODE === "baseline") for (const category of cats) add({ ...common, id: caseId([MODE, language, theme, viewport.name, category]), category, section: "", state: "baseline" }, baselineCase);
      if (MODE === "sections") for (const category of cats) for (const section of SECTIONS[category]) add({ ...common, id: caseId([MODE, language, theme, viewport.name, category, section]), category, section, state: "baseline" }, baselineCase);
      if (MODE === "aliases") aliasCases().forEach((item, index) => add({ ...common, ...item, id: caseId([MODE, language, theme, viewport.name, item.kind, String(index).padStart(3, "0"), item.input.replace(/^\/settings\?/, "")]), state: "alias" }, aliasCase));
      if (MODE === "states") for (const [state, recipe] of Object.entries(STATE_RECIPES)) {
        if (stateFilter.length && !stateFilter.includes(state)) continue;
        for (const category of recipe.categories.filter(item => cats.includes(item))) add({ ...common, id: caseId([MODE, language, theme, viewport.name, category, state]), category, state, family: recipe.family?.[category] || "", fixtureState: recipe.fixtureState || "baseline", settingsState: recipe.settingsState || "ready" }, spec => stateCase(spec, recipe));
      }
      if (MODE === "conditional") for (const item of CONDITIONAL.filter(entry => cats.includes(entry.category))) for (const variant of ["expanded", "click"]) {
        add({ ...common, ...item, variant, id: caseId([MODE, language, theme, viewport.name, item.category, item.section, variant]), state: variant, family: item.section === "network" && variant === "expanded" ? "network" : "", fixtureState: variant === "expanded" ? "expanded" : "baseline" }, conditionalCase);
      }
      if (MODE === "focus") for (const category of cats) add({ ...common, id: caseId([MODE, language, theme, viewport.name, category]), category, state: "focus" }, focusSuite);
      if (MODE === "navigation") for (const item of NAV_CASES.filter(entry => !caseFilter.length || caseFilter.includes(entry.id))) for (const variant of VARIANTS.filter(entry => !variantFilter.length || variantFilter.includes(entry))) {
        add({ ...common, ...item, url: item.url, variant, id: caseId([MODE, language, theme, viewport.name, item.id, variant]), state: "navigation" }, navigationCase);
      }
    }
    const limit = Number(option("limit", "0"));
    if (limit > 0) tasks.splice(limit);
    process.stdout.write(`${tasks.length} cases, concurrency 1, timeout ${CASE_TIMEOUT_MS} ms, out ${path.relative(ROOT, OUT)}\n`);
    for (const task of tasks) await task();
  } finally {
    await browser.close(); writeReports();
    const final = [...new Map(rows.map(row => [row.id, row])).values()];
    if (final.some(row => row.status !== "captured-needs-review" && row.status !== "not-applicable")) process.exitCode = 1;
  }
}
main().catch(error => { console.error(String(error)); process.exitCode = 1; });

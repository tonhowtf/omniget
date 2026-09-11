import test from "node:test";
import assert from "node:assert/strict";

import {
  CHROME_WEB_STORE_UPDATE_URL,
  backupFileName,
  buildBackup,
  buildHtml,
  buildJson,
  buildMarkdown,
  compareExtensions,
  detectManifestVersion,
  detectStoreOrigin,
  escapeHtml,
  largestIcon,
  normalizeExtension,
  probeManifestVersion,
  storeUrlFor,
  summarize,
  withProbedManifestVersions,
} from "../src/ext-backup.js";

const GENERATED_AT = "2026-09-09T12:00:00.000Z";

function fakeExtension(overrides = {}) {
  return {
    id: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
    name: "Fake extension",
    version: "1.0.0",
    description: "A fake extension",
    enabled: true,
    type: "extension",
    installType: "normal",
    updateUrl: CHROME_WEB_STORE_UPDATE_URL,
    permissions: [],
    hostPermissions: [],
    icons: [{ size: 48, url: "chrome://ext/48.png" }],
    ...overrides,
  };
}

const OLD_BLOCKER = fakeExtension({
  id: "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
  name: "Old Blocker",
  version: "2.4.1",
  enabled: false,
  disabledReason: "unknown",
  permissions: ["webRequest", "webRequestBlocking", "storage"],
  hostPermissions: ["<all_urls>"],
});

const MODERN = fakeExtension({
  id: "cccccccccccccccccccccccccccccccc",
  name: "Modern Tool",
  permissions: ["scripting", "storage"],
});

const SIDELOADED = fakeExtension({
  id: "dddddddddddddddddddddddddddddddd",
  name: "Antivirus Helper",
  installType: "sideload",
  updateUrl: "https://vendor.example/update.xml",
  permissions: ["tabs"],
});

const UNPACKED = fakeExtension({
  id: "eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee",
  name: "My Dev Build",
  installType: "development",
  updateUrl: undefined,
  permissions: [],
});

const THEME = fakeExtension({
  id: "ffffffffffffffffffffffffffffffff",
  name: "Dark Theme",
  type: "theme",
  permissions: [],
});

test("storeUrlFor builds a Chrome Web Store detail link", () => {
  assert.equal(
    storeUrlFor("abc"),
    "https://chromewebstore.google.com/detail/abc",
  );
  assert.equal(storeUrlFor(""), null);
  assert.equal(storeUrlFor(null), null);
});

test("largestIcon picks the biggest usable icon", () => {
  assert.equal(
    largestIcon([
      { size: 16, url: "a.png" },
      { size: 128, url: "b.png" },
      { size: 48, url: "c.png" },
    ]),
    "b.png",
  );
  assert.equal(largestIcon([]), null);
  assert.equal(largestIcon(undefined), null);
  assert.equal(largestIcon([{ size: 16 }]), null);
});

test("detectManifestVersion flags MV2 from webRequestBlocking", () => {
  const detected = detectManifestVersion(OLD_BLOCKER);
  assert.equal(detected.version, 2);
  assert.equal(detected.confidence, "likely");
  assert.match(detected.source, /webRequestBlocking/);
});

test("detectManifestVersion flags MV3 from an MV3-only permission", () => {
  for (const permission of ["scripting", "offscreen", "sidePanel", "declarativeNetRequest"]) {
    const detected = detectManifestVersion({ type: "extension", permissions: [permission] });
    assert.equal(detected.version, 3, `${permission} should read as MV3`);
  }
});

test("detectManifestVersion prefers the MV3 signal when both appear", () => {
  const detected = detectManifestVersion({
    type: "extension",
    permissions: ["webRequestBlocking", "scripting"],
  });
  assert.equal(detected.version, 3);
});

test("detectManifestVersion admits it cannot tell", () => {
  const detected = detectManifestVersion({ type: "extension", permissions: ["storage", "tabs"] });
  assert.equal(detected.version, null);
  assert.equal(detected.confidence, "unknown");
});

test("detectManifestVersion says manifest version does not apply to themes", () => {
  const detected = detectManifestVersion(THEME);
  assert.equal(detected.version, null);
  assert.match(detected.source, /theme/);
});

test("detectStoreOrigin separates store installs from sideloads and dev builds", () => {
  assert.equal(detectStoreOrigin(fakeExtension()), true);
  assert.equal(detectStoreOrigin(SIDELOADED), false);
  assert.equal(detectStoreOrigin(UNPACKED), false);
  assert.equal(detectStoreOrigin({}), null);
});

test("normalizeExtension keeps the fields the backup needs and sorts permissions", () => {
  const entry = normalizeExtension(OLD_BLOCKER);
  assert.equal(entry.id, OLD_BLOCKER.id);
  assert.equal(entry.name, "Old Blocker");
  assert.equal(entry.version, "2.4.1");
  assert.equal(entry.enabled, false);
  assert.equal(entry.installType, "normal");
  assert.equal(entry.manifestVersion, 2);
  assert.equal(entry.manifestVersionConfidence, "likely");
  assert.deepEqual(entry.permissions, ["storage", "webRequest", "webRequestBlocking"]);
  assert.deepEqual(entry.hostPermissions, ["<all_urls>"]);
  assert.equal(entry.storeUrl, `https://chromewebstore.google.com/detail/${OLD_BLOCKER.id}`);
  assert.equal(entry.icon, "chrome://ext/48.png");
});

test("normalizeExtension falls back to the id when the name is missing", () => {
  const entry = normalizeExtension({ id: "xyz" });
  assert.equal(entry.name, "xyz");
  assert.equal(entry.version, null);
  assert.deepEqual(entry.permissions, []);
});

test("normalizeExtension trusts a manifestVersion read from the real manifest", () => {
  const entry = normalizeExtension({ ...MODERN, manifestVersion: 2 });
  assert.equal(entry.manifestVersion, 2);
  assert.equal(entry.manifestVersionConfidence, "certain");
  assert.match(entry.manifestVersionSource, /manifest/);
});

test("compareExtensions puts MV2 first, then unknown, then MV3", () => {
  const entries = [MODERN, THEME, OLD_BLOCKER].map(normalizeExtension).sort(compareExtensions);
  assert.deepEqual(
    entries.map((e) => e.name),
    ["Old Blocker", "Dark Theme", "Modern Tool"],
  );
});

test("compareExtensions sorts by name inside a group, ignoring case", () => {
  const entries = [
    normalizeExtension(fakeExtension({ id: "1", name: "zebra" })),
    normalizeExtension(fakeExtension({ id: "2", name: "Alpha" })),
  ].sort(compareExtensions);
  assert.deepEqual(
    entries.map((e) => e.name),
    ["Alpha", "zebra"],
  );
});

test("summarize counts manifest versions, disabled and sideloaded", () => {
  const entries = [OLD_BLOCKER, MODERN, SIDELOADED, UNPACKED, THEME].map(normalizeExtension);
  const summary = summarize(entries);
  assert.equal(summary.total, 5);
  assert.equal(summary.mv2, 1);
  assert.equal(summary.mv3, 1);
  assert.equal(summary.unknownManifest, 3);
  assert.equal(summary.disabled, 1);
  assert.equal(summary.enabled, 4);
  assert.equal(summary.sideloaded, 1);
  assert.equal(summary.development, 1);
  assert.equal(summary.byType.theme, 1);
  assert.equal(summary.byType.extension, 4);
});

test("buildBackup groups ids and records that the store was not checked", () => {
  const backup = buildBackup([MODERN, OLD_BLOCKER, THEME], { generatedAt: GENERATED_AT });
  assert.equal(backup.format, "omniget-extension-inventory");
  assert.equal(backup.generatedAt, GENERATED_AT);
  assert.equal(backup.storeListingChecked, false);
  assert.deepEqual(backup.groups.mv2, [OLD_BLOCKER.id]);
  assert.deepEqual(backup.groups.mv3, [MODERN.id]);
  assert.deepEqual(backup.groups.unknown, [THEME.id]);
  assert.equal(backup.extensions[0].id, OLD_BLOCKER.id);
});

test("buildBackup survives a missing or empty list", () => {
  const backup = buildBackup(undefined, { generatedAt: GENERATED_AT });
  assert.equal(backup.summary.total, 0);
  assert.deepEqual(backup.extensions, []);
  assert.deepEqual(backup.groups.mv2, []);
});

test("buildJson round-trips through JSON.parse", () => {
  const backup = buildBackup([OLD_BLOCKER, MODERN], { generatedAt: GENERATED_AT });
  const text = buildJson(backup);
  assert.ok(text.endsWith("\n"));
  const parsed = JSON.parse(text);
  assert.equal(parsed.extensions.length, 2);
  assert.equal(parsed.extensions[0].name, "Old Blocker");
});

test("backupFileName uses the export day", () => {
  assert.equal(backupFileName("json", GENERATED_AT), "omniget-extensions-2026-09-09.json");
  assert.equal(backupFileName("md", GENERATED_AT), "omniget-extensions-2026-09-09.md");
});

test("buildMarkdown puts the MV2 section first with store link and permissions", () => {
  const backup = buildBackup([MODERN, OLD_BLOCKER, THEME], { generatedAt: GENERATED_AT });
  const md = buildMarkdown(backup);

  const mv2Index = md.indexOf("## Manifest V2");
  const mv3Index = md.indexOf("## Manifest V3");
  assert.ok(mv2Index > 0);
  assert.ok(mv3Index > mv2Index, "MV2 section must come before MV3");

  assert.match(md, /Total: 3/);
  assert.match(md, /Manifest V2 \(at risk\): 1/);
  assert.match(md, /Disabled: 1/);
  assert.ok(md.includes(`https://chromewebstore.google.com/detail/${OLD_BLOCKER.id}`));
  assert.ok(md.includes("webRequestBlocking"));
  assert.ok(md.includes("Modern Tool"));
});

test("buildMarkdown says a group is empty instead of leaving a hole", () => {
  const md = buildMarkdown(buildBackup([MODERN], { generatedAt: GENERATED_AT }));
  assert.ok(md.includes("Nothing found in this group."));
});

test("escapeHtml neutralizes markup from extension names", () => {
  assert.equal(escapeHtml(`<img src=x onerror="alert(1)">`), "&lt;img src=x onerror=&quot;alert(1)&quot;&gt;");
  assert.equal(escapeHtml("a & b"), "a &amp; b");
});

test("buildHtml escapes hostile names and keeps MV2 on top", () => {
  const evil = fakeExtension({
    id: "gggggggggggggggggggggggggggggggg",
    name: `<script>alert(1)</script>`,
    permissions: ["webRequestBlocking"],
  });
  const html = buildHtml(buildBackup([MODERN, evil], { generatedAt: GENERATED_AT }));

  assert.ok(!html.includes("<script>alert(1)</script>"));
  assert.ok(html.includes("&lt;script&gt;alert(1)&lt;/script&gt;"));
  assert.ok(html.startsWith("<!doctype html>"));
  assert.ok(html.indexOf("Manifest V2 — at risk") < html.indexOf("Modern Tool"));
  assert.ok(html.includes(`https://chromewebstore.google.com/detail/${evil.id}`));
});

test("probeManifestVersion reads the real manifest when the fetch works", async () => {
  const fetchImpl = async (url) => {
    assert.equal(url, "chrome-extension://abc/manifest.json");
    return { ok: true, json: async () => ({ manifest_version: 2 }) };
  };
  assert.equal(await probeManifestVersion("abc", { fetchImpl }), 2);
});

test("probeManifestVersion returns null when the fetch is blocked", async () => {
  const rejecting = async () => {
    throw new Error("blocked");
  };
  assert.equal(await probeManifestVersion("abc", { fetchImpl: rejecting }), null);
  assert.equal(await probeManifestVersion("abc", { fetchImpl: async () => ({ ok: false }) }), null);
  assert.equal(await probeManifestVersion("", { fetchImpl: rejecting }), null);
});

test("withProbedManifestVersions only overrides what it could read", async () => {
  const fetchImpl = async (url) =>
    url.includes(MODERN.id)
      ? { ok: true, json: async () => ({ manifest_version: 3 }) }
      : { ok: false };

  const enriched = await withProbedManifestVersions([MODERN, OLD_BLOCKER], { fetchImpl });
  assert.equal(enriched[0].manifestVersion, 3);
  assert.equal(enriched[1].manifestVersion, undefined);

  const backup = buildBackup(enriched, { generatedAt: GENERATED_AT });
  const modern = backup.extensions.find((e) => e.id === MODERN.id);
  assert.equal(modern.manifestVersionConfidence, "certain");
});

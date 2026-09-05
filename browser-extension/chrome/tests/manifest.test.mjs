import test from "node:test";
import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";

const manifestUrl = new URL("../manifest.json", import.meta.url);

async function readManifest() {
  return JSON.parse(await readFile(manifestUrl, "utf8"));
}

test("declares popup as default action", async () => {
  const manifest = await readManifest();

  assert.equal(manifest.action.default_popup, "popup/popup.html");
});

test("declares the 48px toolbar icon for the inactive action state", async () => {
  const manifest = await readManifest();

  assert.equal(manifest.action.default_icon["48"], "icons/inactive-48.png");
});

test("declares cookies permission for cookie forwarding", async () => {
  const manifest = await readManifest();

  assert.ok(manifest.permissions.includes("cookies"));
});

test("declares webRequest permission for media sniffing", async () => {
  const manifest = await readManifest();

  assert.ok(manifest.permissions.includes("webRequest"));
});

test("declares storage permission for sniffer toggle", async () => {
  const manifest = await readManifest();

  assert.ok(manifest.permissions.includes("storage"));
});

test("declares platform-scoped host_permissions instead of a wildcard", async () => {
  const manifest = await readManifest();

  assert.ok(Array.isArray(manifest.host_permissions), "host_permissions must be an array");
  assert.ok(
    !manifest.host_permissions.includes("*://*/*"),
    "host_permissions must not ship a wildcard by default",
  );

  const requiredPatterns = [
    "*://*.hotmart.com/*",
    "*://*.youtube.com/*",
    "*://youtu.be/*",
    "*://*.instagram.com/*",
    "*://*.tiktok.com/*",
    "*://*.twitter.com/*",
    "*://*.x.com/*",
    "*://*.reddit.com/*",
    "*://*.twitch.tv/*",
    "*://*.pinterest.com/*",
    "*://bsky.app/*",
    "*://t.me/*",
    "*://*.vimeo.com/*",
    "*://*.udemy.com/*",
    "*://*.bilibili.com/*",
  ];
  for (const pattern of requiredPatterns) {
    assert.ok(
      manifest.host_permissions.includes(pattern),
      `host_permissions missing required pattern ${pattern}`,
    );
  }
});

test("declares wildcard host access in optional_host_permissions for media sniffer", async () => {
  const manifest = await readManifest();

  assert.ok(
    Array.isArray(manifest.optional_host_permissions),
    "optional_host_permissions must be an array",
  );
  assert.ok(manifest.optional_host_permissions.includes("*://*/*"));
});

test("declares the send-to-omniget command with Alt+O default shortcut", async () => {
  const manifest = await readManifest();

  assert.ok(manifest.commands, "manifest.commands block missing");
  const command = manifest.commands["send-to-omniget"];
  assert.ok(command, "send-to-omniget command missing");
  assert.equal(command.suggested_key?.default, "Alt+O");
  assert.ok(typeof command.description === "string" && command.description.length > 0);
});

test("declares host permission for Rocketseat so its cookies can be captured", async () => {
  const manifest = await readManifest();
  assert.ok(manifest.host_permissions.includes("*://*.rocketseat.com.br/*"));
});

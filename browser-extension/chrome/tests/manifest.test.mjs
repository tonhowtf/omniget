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

test("keeps management as an optional permission, never a required one", async () => {
  const manifest = await readManifest();

  assert.ok(
    !manifest.permissions.includes("management"),
    "management must not be a required permission",
  );
  assert.ok(
    Array.isArray(manifest.optional_permissions),
    "optional_permissions must be an array",
  );
  assert.ok(
    manifest.optional_permissions.includes("management"),
    "optional_permissions must offer management for the extension backup page",
  );
});

test("does not add declarativeNetRequest or a wildcard host permission", async () => {
  const manifest = await readManifest();

  const required = [...manifest.permissions, ...(manifest.optional_permissions ?? [])];
  assert.ok(!required.some((p) => p.startsWith("declarativeNetRequest")));
  assert.ok(!manifest.host_permissions.includes("<all_urls>"));
});

test("declares scripting permission so deep search can register content scripts", async () => {
  const manifest = await readManifest();

  assert.ok(manifest.permissions.includes("scripting"));
});

test("the firefox manifest keeps the permissions deep search depends on", async () => {
  const firefox = JSON.parse(
    await readFile(new URL("../../firefox/manifest.json", import.meta.url), "utf8")
  );

  assert.ok(firefox.permissions.includes("scripting"));
  assert.ok(firefox.optional_host_permissions.includes("*://*/*"));
  // MAIN-world content scripts need Firefox 128, but the toggle hides itself
  // below that, so the floor stays where it is and nobody loses the extension.
  assert.equal(firefox.browser_specific_settings.gecko.strict_min_version, "109.0");
});

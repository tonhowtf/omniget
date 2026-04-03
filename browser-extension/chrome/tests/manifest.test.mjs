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

test("declares wildcard host_permissions for media detection", async () => {
  const manifest = await readManifest();

  assert.ok(manifest.host_permissions.includes("*://*/*"));
});

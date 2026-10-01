import test from "node:test";
import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";

import {
  SYNTHETIC_MANIFEST_HOST,
  isSyntheticManifestUrl,
} from "../src/synthetic-manifest.js";

test("isSyntheticManifestUrl recognises only the reserved host", () => {
  assert.equal(
    isSyntheticManifestUrl(`https://${SYNTHETIC_MANIFEST_HOST}/player.example.com/abc.m3u8`),
    true
  );
  assert.equal(isSyntheticManifestUrl("https://cdn.example.com/master.m3u8"), false);
  assert.equal(isSyntheticManifestUrl("https://deep-search.omniget.invalid.evil.com/x"), false);
  assert.equal(isSyntheticManifestUrl("blob:https://example.com/uuid"), false);
  assert.equal(isSyntheticManifestUrl(""), false);
  assert.equal(isSyntheticManifestUrl(null), false);
});

test("the host never resolves, by construction", () => {
  assert.ok(SYNTHETIC_MANIFEST_HOST.endsWith(".invalid"));
});

test("deep-search.js builds its URLs on the very same host", async () => {
  const source = await readFile(new URL("../src/deep-search.js", import.meta.url), "utf8");
  assert.ok(
    source.includes(`https://${SYNTHETIC_MANIFEST_HOST}`),
    "deep-search.js and synthetic-manifest.js disagree about the synthetic host"
  );
});

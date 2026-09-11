import test from "node:test";
import assert from "node:assert/strict";

import { formatBytes } from "../src/format-size.js";

test("formatBytes picks the unit the number reads best in", () => {
  assert.equal(formatBytes(512), "1 KB");
  assert.equal(formatBytes(100 * 1024), "100 KB");
  assert.equal(formatBytes(1024 * 1024), "1.0 MB");
  assert.equal(formatBytes(1536 * 1024), "1.5 MB");
  assert.equal(formatBytes(3 * 1024 * 1024 * 1024), "3.00 GB");
});

test("formatBytes says nothing when the size is unknown", () => {
  assert.equal(formatBytes(0), "");
  assert.equal(formatBytes(-1), "");
  assert.equal(formatBytes(null), "");
  assert.equal(formatBytes(undefined), "");
});

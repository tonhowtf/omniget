import test from "node:test";
import assert from "node:assert/strict";

import {
  CAPTURE_RULES_KEY,
  defaultCaptureRules,
  getCaptureRules,
  getCompiledRegexRules,
  resolveCaptureRules,
} from "../src/capture-store.js";

test("the store starts on the shipped defaults when there is no storage", () => {
  const rules = getCaptureRules();
  assert.ok(rules.extensions.length > 0);
  assert.ok(rules.contentTypes.length > 0);
  assert.equal(rules.regex.length, 3);
  assert.equal(getCompiledRegexRules().length, 3);
  assert.ok(getCompiledRegexRules().every(rule => rule.regex instanceof RegExp));
});

test("defaultCaptureRules hands out a fresh, mutable copy each time", () => {
  const a = defaultCaptureRules();
  const b = defaultCaptureRules();
  assert.notEqual(a.extensions, b.extensions);
  assert.notEqual(a.extensions[0], b.extensions[0]);
  a.extensions[0].size = "touched";
  assert.notEqual(b.extensions[0].size, "touched");
  assert.deepEqual(defaultCaptureRules().extensions[0], b.extensions[0]);
});

test("resolveCaptureRules falls back to defaults for anything unusable", () => {
  const defaults = defaultCaptureRules();
  assert.deepEqual(resolveCaptureRules(null), defaults);
  assert.deepEqual(resolveCaptureRules("nope"), defaults);
  assert.deepEqual(resolveCaptureRules({}), defaults);
});

test("resolveCaptureRules keeps the defaults for the tables the user did not save", () => {
  const resolved = resolveCaptureRules({ regex: [{ pattern: "example\\.com" }] });
  assert.deepEqual(resolved.extensions, defaultCaptureRules().extensions);
  assert.deepEqual(resolved.contentTypes, defaultCaptureRules().contentTypes);
  assert.equal(resolved.regex.length, 1);
  assert.equal(resolved.regex[0].pattern, "example\\.com");
});

test("resolveCaptureRules honours a table the user emptied on purpose", () => {
  const resolved = resolveCaptureRules({ extensions: [], contentTypes: [], regex: [] });
  assert.deepEqual(resolved, { extensions: [], contentTypes: [], regex: [] });
});

test("resolveCaptureRules normalises what it stores", () => {
  const resolved = resolveCaptureRules({
    extensions: ["MP4", { ext: ".WEBM", size: " >1 MB ", enabled: false }, 42, null],
  });
  assert.deepEqual(resolved.extensions, [
    { ext: ".mp4", size: null, enabled: true },
    { ext: ".webm", size: ">1 MB", enabled: false },
  ]);
});

test("the storage key is the one the options page will write", () => {
  assert.equal(CAPTURE_RULES_KEY, "captureRules");
});

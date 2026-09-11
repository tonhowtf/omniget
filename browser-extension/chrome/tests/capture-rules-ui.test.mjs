import test from "node:test";
import assert from "node:assert/strict";

import {
  SIZE_RULE_HELP,
  buildCaptureRules,
  describeRegexTest,
  describeSizeRule,
  formatRuleBytes,
  normalizeRuleKey,
} from "../pages/options.js";

// --- formatRuleBytes --------------------------------------------------------

test("formatRuleBytes picks the largest unit the value actually reaches", () => {
  assert.equal(formatRuleBytes(0), "0 bytes");
  assert.equal(formatRuleBytes(1), "1 byte");
  assert.equal(formatRuleBytes(512), "512 bytes");
  assert.equal(formatRuleBytes(50 * 1024), "50 KB");
  assert.equal(formatRuleBytes(1536 * 1024), "1.5 MB");
  assert.equal(formatRuleBytes(3 * 1024 * 1024 * 1024), "3 GB");
});

test("formatRuleBytes stays in base 1024 instead of rounding up a unit", () => {
  // 1000 MB is not 1 GB, and a rule that says one must not read as the other.
  assert.equal(formatRuleBytes(1000 * 1024 * 1024), "1000 MB");
});

test("formatRuleBytes says nothing about a size it cannot render", () => {
  assert.equal(formatRuleBytes(-1), "");
  assert.equal(formatRuleBytes(Number.NaN), "");
  assert.equal(formatRuleBytes("50 KB"), "");
  assert.equal(formatRuleBytes(undefined), "");
});

// --- describeSizeRule -------------------------------------------------------

test("describeSizeRule treats an empty field as no restriction", () => {
  assert.deepEqual(describeSizeRule(""), { state: "empty", text: "No size limit" });
  assert.deepEqual(describeSizeRule("   "), { state: "empty", text: "No size limit" });
  assert.deepEqual(describeSizeRule(null), { state: "empty", text: "No size limit" });
  assert.deepEqual(describeSizeRule(undefined), { state: "empty", text: "No size limit" });
});

test("describeSizeRule spells out every operator", () => {
  assert.deepEqual(describeSizeRule(">=50 KB"), { state: "ok", text: "at least 50 KB" });
  assert.deepEqual(describeSizeRule(">1 GB"), { state: "ok", text: "larger than 1 GB" });
  assert.deepEqual(describeSizeRule("<=2 MB"), { state: "ok", text: "at most 2 MB" });
  assert.deepEqual(describeSizeRule("<1024"), { state: "ok", text: "smaller than 1 KB" });
  assert.deepEqual(describeSizeRule("=500 KB"), { state: "ok", text: "exactly 500 KB" });
  assert.deepEqual(describeSizeRule("!=0"), { state: "ok", text: "any size except 0 bytes" });
});

test("describeSizeRule reads a bare number as a floor, like parseSizeRule does", () => {
  assert.deepEqual(describeSizeRule("50KB"), { state: "ok", text: "at least 50 KB" });
  assert.deepEqual(describeSizeRule("1.5 mb"), { state: "ok", text: "at least 1.5 MB" });
});

test("describeSizeRule reads a range as a span, in either order", () => {
  assert.deepEqual(describeSizeRule("500-1000 MB"), {
    state: "ok",
    text: "between 500 MB and 1000 MB",
  });
  assert.deepEqual(describeSizeRule("1000-500 MB"), {
    state: "ok",
    text: "between 500 MB and 1000 MB",
  });
});

test("describeSizeRule flags what parseSizeRule rejects", () => {
  for (const input of ["abc", ">= KB", "5 zb", ">", "-"]) {
    assert.deepEqual(describeSizeRule(input), { state: "invalid", text: SIZE_RULE_HELP });
  }
});

// --- buildCaptureRules ------------------------------------------------------

test("buildCaptureRules normalises the keys the user typed", () => {
  const rules = buildCaptureRules({
    extensions: [{ ext: "MP4", size: ">=50 KB", enabled: true }],
    contentTypes: [{ type: " Video/MP4 ", size: "", enabled: true }],
    regex: [],
  });
  assert.deepEqual(rules.extensions, [{ ext: ".mp4", size: ">=50 KB", enabled: true }]);
  assert.deepEqual(rules.contentTypes, [{ type: "video/mp4", size: null, enabled: true }]);
});

test("buildCaptureRules drops blank rows in all three tables", () => {
  const rules = buildCaptureRules({
    extensions: [
      { ext: ".mp4", size: "", enabled: true },
      { ext: "   ", size: ">=1 MB", enabled: true },
      { ext: "", size: "", enabled: true },
    ],
    contentTypes: [{ type: "", size: "", enabled: true }],
    regex: [
      { pattern: "  ", flags: "ig", action: "block", ext: "", enabled: true },
      { pattern: "cdn\\.example\\.com", flags: "ig", action: "block", ext: "", enabled: true },
    ],
  });
  assert.equal(rules.extensions.length, 1);
  assert.equal(rules.contentTypes.length, 0);
  assert.equal(rules.regex.length, 1);
  assert.equal(rules.regex[0].pattern, "cdn\\.example\\.com");
});

test("buildCaptureRules keeps a disabled row but never an unparseable size", () => {
  const rules = buildCaptureRules({
    extensions: [{ ext: ".flv", size: "nonsense", enabled: false }],
  });
  assert.deepEqual(rules.extensions, [{ ext: ".flv", size: null, enabled: false }]);
});

test("buildCaptureRules fills in the regex defaults and trims the pattern", () => {
  const rules = buildCaptureRules({
    regex: [{ pattern: " (^https://a\\.b/.*)&bytestart=.* ", flags: "", ext: "MP4" }],
  });
  assert.deepEqual(rules.regex, [
    {
      pattern: "(^https://a\\.b/.*)&bytestart=.*",
      flags: "ig",
      action: "accept",
      ext: "mp4",
      enabled: true,
    },
  ]);
});

test("buildCaptureRules answers with three empty tables for junk input", () => {
  const empty = { extensions: [], contentTypes: [], regex: [] };
  assert.deepEqual(buildCaptureRules(null), empty);
  assert.deepEqual(buildCaptureRules(undefined), empty);
  assert.deepEqual(buildCaptureRules("nope"), empty);
  assert.deepEqual(buildCaptureRules({ extensions: "nope", regex: 7 }), empty);
});

// --- normalizeRuleKey -------------------------------------------------------

test("normalizeRuleKey tidies a key and hands back what it cannot parse", () => {
  assert.equal(normalizeRuleKey("ext", " MP4 "), ".mp4");
  assert.equal(normalizeRuleKey("ext", "...m3u8"), ".m3u8");
  assert.equal(normalizeRuleKey("type", "Video/MP4"), "video/mp4");
  assert.equal(normalizeRuleKey("ext", ""), "");
  assert.equal(normalizeRuleKey("ext", "foo bar"), "foo bar");
  assert.equal(normalizeRuleKey("type", "video"), "video");
});

// --- describeRegexTest ------------------------------------------------------

const TEST_RULES = [
  { pattern: ".*\\.bilivideo\\.com.*/live-bvc/.*m4s", flags: "ig", action: "block", ext: "", enabled: true },
  { pattern: "(^https://cdn\\.example\\.com/.*)&bytestart=.*", flags: "ig", action: "accept", ext: "mp4", enabled: true },
];

test("describeRegexTest says nothing until a URL is typed", () => {
  assert.deepEqual(describeRegexTest("", TEST_RULES), { state: "empty", text: "", url: "" });
  assert.deepEqual(describeRegexTest("   ", TEST_RULES), { state: "empty", text: "", url: "" });
  assert.deepEqual(describeRegexTest(null, TEST_RULES), { state: "empty", text: "", url: "" });
});

test("describeRegexTest names the row that blocks a URL", () => {
  const result = describeRegexTest("https://x.bilivideo.com/live-bvc/a.m4s", TEST_RULES);
  assert.equal(result.state, "blocked");
  assert.equal(result.url, "");
  assert.match(result.text, /^Rule 1 blocks this URL/);
});

test("describeRegexTest shows the URL an accept rule rebuilds", () => {
  const result = describeRegexTest(
    "https://cdn.example.com/v/f.mp4?x=1&bytestart=0&byteend=99",
    TEST_RULES,
  );
  assert.equal(result.state, "accepted");
  assert.equal(result.url, "https://cdn.example.com/v/f.mp4?x=1");
  assert.equal(
    result.text,
    "Rule 2 matches — captured as https://cdn.example.com/v/f.mp4?x=1 (saved as .mp4)",
  );
});

test("describeRegexTest says so when a rule matches without rewriting", () => {
  const result = describeRegexTest("https://cdn.example.com/v/f.mp4", [
    { pattern: "cdn\\.example\\.com", flags: "ig", action: "accept", ext: "", enabled: true },
  ]);
  assert.equal(result.state, "accepted");
  assert.equal(result.url, "https://cdn.example.com/v/f.mp4");
  assert.equal(result.text, "Rule 1 matches — captured as is.");
});

test("describeRegexTest falls through when nothing matches", () => {
  const result = describeRegexTest("https://other.example/f.mp4", TEST_RULES);
  assert.equal(result.state, "no-match");
  assert.equal(result.url, "");
  assert.match(result.text, /^No rule matched/);
});

test("describeRegexTest survives a broken pattern and keeps the row numbers", () => {
  const rules = [
    { pattern: "([", flags: "ig", action: "accept", ext: "", enabled: true },
    ...TEST_RULES,
  ];
  const result = describeRegexTest("https://x.bilivideo.com/live-bvc/a.m4s", rules);
  assert.equal(result.state, "blocked");
  assert.match(result.text, /^Rule 2 blocks this URL/);
});

test("describeRegexTest ignores a row the user unticked", () => {
  const rules = TEST_RULES.map(rule => ({ ...rule, enabled: false }));
  assert.equal(describeRegexTest("https://x.bilivideo.com/live-bvc/a.m4s", rules).state, "no-match");
});

test("describeRegexTest tolerates a missing rule list", () => {
  assert.equal(describeRegexTest("https://example.com/a.mp4", null).state, "no-match");
  assert.equal(describeRegexTest("https://example.com/a.mp4", []).state, "no-match");
});

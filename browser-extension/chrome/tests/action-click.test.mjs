import test from "node:test";
import assert from "node:assert/strict";

import { handleSupportedActionClick } from "../src/action-click.js";

test("clears the previous badge and shows success feedback after a successful send", async () => {
  const calls = [];

  const result = await handleSupportedActionClick({
    tabId: 12,
    url: "https://example.com/watch?v=123",
    sendNativeMessage: async (message) => {
      calls.push(["send", message]);
      return { ok: true };
    },
    clearBadge: async (tabId) => {
      calls.push(["clear", tabId]);
    },
    showSuccessBadge: async (tabId) => {
      calls.push(["success", tabId]);
    },
    openErrorPage: async (details) => {
      calls.push(["error", details]);
    },
    mapChromeErrorCode: () => "LAUNCH_FAILED",
  });

  assert.equal(result, true);
  assert.deepEqual(calls, [
    ["clear", 12],
    ["send", { type: "enqueue", url: "https://example.com/watch?v=123" }],
    ["success", 12],
  ]);
});

test("includes cookies in the native message when getCookies returns data", async () => {
  const calls = [];
  const fakeCookies = [{ domain: ".youtube.com", name: "LOGIN_INFO", value: "abc" }];

  await handleSupportedActionClick({
    tabId: 1,
    url: "https://www.youtube.com/watch?v=123",
    platform: "youtube",
    getCookies: async () => fakeCookies,
    sendNativeMessage: async (message) => {
      calls.push(["send", message]);
      return { ok: true };
    },
    clearBadge: async () => {},
    showSuccessBadge: async () => {},
    openErrorPage: async () => {},
    mapChromeErrorCode: () => "LAUNCH_FAILED",
  });

  assert.deepEqual(calls[0][1].cookies, fakeCookies);
});

test("sends message without cookies when getCookies returns null", async () => {
  const calls = [];

  await handleSupportedActionClick({
    tabId: 1,
    url: "https://www.youtube.com/watch?v=123",
    platform: "youtube",
    getCookies: async () => null,
    sendNativeMessage: async (message) => {
      calls.push(["send", message]);
      return { ok: true };
    },
    clearBadge: async () => {},
    showSuccessBadge: async () => {},
    openErrorPage: async () => {},
    mapChromeErrorCode: () => "LAUNCH_FAILED",
  });

  assert.equal(calls[0][1].cookies, undefined);
});

test("sends message without cookies when getCookies throws", async () => {
  const calls = [];

  await handleSupportedActionClick({
    tabId: 1,
    url: "https://www.youtube.com/watch?v=123",
    platform: "youtube",
    getCookies: async () => { throw new Error("permission denied"); },
    sendNativeMessage: async (message) => {
      calls.push(["send", message]);
      return { ok: true };
    },
    clearBadge: async () => {},
    showSuccessBadge: async () => {},
    openErrorPage: async () => {},
    mapChromeErrorCode: () => "LAUNCH_FAILED",
  });

  assert.equal(calls[0][1].cookies, undefined);
});

test("does not show success feedback when the native host returns an error response", async () => {
  const calls = [];

  const result = await handleSupportedActionClick({
    tabId: 7,
    url: "https://example.com/reel/abc",
    sendNativeMessage: async () => ({ ok: false, code: "LAUNCH_FAILED" }),
    clearBadge: async (tabId) => {
      calls.push(["clear", tabId]);
    },
    showSuccessBadge: async (tabId) => {
      calls.push(["success", tabId]);
    },
    openErrorPage: async (details) => {
      calls.push(["error", details]);
    },
    mapChromeErrorCode: () => "LAUNCH_FAILED",
  });

  assert.equal(result, false);
  assert.deepEqual(calls, [
    ["clear", 7],
    ["error", { code: "LAUNCH_FAILED", message: "", url: "https://example.com/reel/abc" }],
  ]);
});

test("does not show success feedback when native messaging throws", async () => {
  const calls = [];

  const result = await handleSupportedActionClick({
    tabId: 3,
    url: "https://example.com/post/xyz",
    sendNativeMessage: async () => {
      throw new Error("host missing");
    },
    clearBadge: async (tabId) => {
      calls.push(["clear", tabId]);
    },
    showSuccessBadge: async (tabId) => {
      calls.push(["success", tabId]);
    },
    openErrorPage: async (details) => {
      calls.push(["error", details]);
    },
    mapChromeErrorCode: () => "HOST_MISSING",
  });

  assert.equal(result, false);
  assert.deepEqual(calls, [
    ["clear", 3],
    ["error", { code: "HOST_MISSING", message: "host missing", url: "https://example.com/post/xyz" }],
  ]);
});

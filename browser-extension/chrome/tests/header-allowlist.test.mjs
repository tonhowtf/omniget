import test from "node:test";
import assert from "node:assert/strict";

import { extractSendableHeaders, isAllowedHeader } from "../src/header-allowlist.js";

test("isAllowedHeader accepts the named auth headers regardless of case", () => {
  assert.equal(isAllowedHeader("Referer"), true);
  assert.equal(isAllowedHeader("COOKIE"), true);
  assert.equal(isAllowedHeader("authorization"), true);
  assert.equal(isAllowedHeader("Access-Token"), true);
  assert.equal(isAllowedHeader("session-id"), true);
});

test("isAllowedHeader accepts x- headers that look like credentials", () => {
  assert.equal(isAllowedHeader("x-auth-token"), true);
  assert.equal(isAllowedHeader("X-Signature"), true);
  assert.equal(isAllowedHeader("x-api-key"), true);
  assert.equal(isAllowedHeader("x-session-ticket"), true);
});

test("isAllowedHeader rejects fingerprinting and transport headers", () => {
  assert.equal(isAllowedHeader("host"), false);
  assert.equal(isAllowedHeader("accept-encoding"), false);
  assert.equal(isAllowedHeader("sec-ch-ua-platform"), false);
  assert.equal(isAllowedHeader("user-agent"), false);
  assert.equal(isAllowedHeader("x-requested-with"), false);
  assert.equal(isAllowedHeader(""), false);
  assert.equal(isAllowedHeader(null), false);
  assert.equal(isAllowedHeader(42), false);
});

test("extractSendableHeaders keeps only what the download needs", () => {
  const headers = [
    { name: "Host", value: "cdn.example.com" },
    { name: "Referer", value: "https://example.com/watch" },
    { name: "Cookie", value: "sid=abc" },
    { name: "sec-ch-ua-mobile", value: "?0" },
    { name: "X-Auth-Token", value: "tok" },
    { name: "Accept-Encoding", value: "gzip" },
  ];
  assert.deepEqual(extractSendableHeaders(headers), {
    Referer: "https://example.com/watch",
    Cookie: "sid=abc",
    "X-Auth-Token": "tok",
  });
});

test("extractSendableHeaders returns null when nothing survives", () => {
  assert.equal(extractSendableHeaders([{ name: "Host", value: "x" }]), null);
  assert.equal(extractSendableHeaders([]), null);
  assert.equal(extractSendableHeaders(null), null);
  assert.equal(extractSendableHeaders("nope"), null);
});

test("extractSendableHeaders skips malformed and empty entries", () => {
  const headers = [
    null,
    { name: "Cookie" },
    { name: "Cookie", value: "" },
    { value: "orphan" },
    { name: "Referer", value: "https://example.com/" },
  ];
  assert.deepEqual(extractSendableHeaders(headers), { Referer: "https://example.com/" });
});

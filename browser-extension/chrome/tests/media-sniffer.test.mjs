import test from "node:test";
import assert from "node:assert/strict";

import {
  MAX_PENDING_REQUESTS,
  mergeStoredMedia,
  trimPendingRequests,
} from "../src/media-sniffer.js";

function pendingOfSize(size) {
  const map = new Map();
  for (let i = 0; i < size; i++) map.set(`req-${i}`, { tabId: 1 });
  return map;
}

test("trimPendingRequests leaves a map under the cap untouched", () => {
  const pending = pendingOfSize(10);
  assert.equal(trimPendingRequests(pending, 16), 0);
  assert.equal(pending.size, 10);
});

test("trimPendingRequests drops the oldest half once the cap is exceeded", () => {
  const pending = pendingOfSize(17);
  assert.equal(trimPendingRequests(pending, 16), 8);
  assert.equal(pending.size, 9);
  assert.equal(pending.has("req-0"), false);
  assert.equal(pending.has("req-7"), false);
  assert.equal(pending.has("req-8"), true);
  assert.equal(pending.has("req-16"), true);
});

test("trimPendingRequests keeps a leaking map bounded across many rounds", () => {
  const pending = new Map();
  for (let i = 0; i < 20_000; i++) {
    pending.set(`req-${i}`, { tabId: 1 });
    trimPendingRequests(pending, 64);
    assert.ok(pending.size <= 64);
  }
  assert.ok(pending.has("req-19999"));
});

test("MAX_PENDING_REQUESTS is the documented default", () => {
  assert.equal(MAX_PENDING_REQUESTS, 4096);
  const pending = pendingOfSize(MAX_PENDING_REQUESTS + 1);
  assert.equal(trimPendingRequests(pending), 2048);
});

test("mergeStoredMedia fills an empty page from storage", () => {
  const target = new Map();
  const added = mergeStoredMedia(target, "https://example.com/watch", [
    ["https://cdn.example.com/a.m3u8", { url: "https://cdn.example.com/a.m3u8" }],
    ["https://cdn.example.com/b.mp4", { url: "https://cdn.example.com/b.mp4" }],
  ]);

  assert.equal(added, 2);
  assert.equal(target.get("https://example.com/watch").size, 2);
});

test("mergeStoredMedia never drops a detection that arrived first", () => {
  // The exact shape of a service-worker wake-up: the request that woke the
  // worker is recorded while the storage read is still in flight.
  const live = { url: "https://cdn.example.com/a.m3u8", detectedAt: 2000, live: true };
  const stored = { url: "https://cdn.example.com/a.m3u8", detectedAt: 1000, live: false };
  const target = new Map([["https://example.com/watch", new Map([[live.url, live]])]]);

  const added = mergeStoredMedia(target, "https://example.com/watch", [
    [stored.url, stored],
    ["https://cdn.example.com/old.mp4", { url: "https://cdn.example.com/old.mp4" }],
  ]);

  const page = target.get("https://example.com/watch");
  assert.equal(added, 1);
  assert.equal(page.size, 2);
  // The live entry is the fresher one and must survive the restore.
  assert.equal(page.get(live.url), live);
  assert.equal(page.get(live.url).live, true);
});

test("mergeStoredMedia leaves other pages untouched", () => {
  const other = new Map([["https://cdn.example.com/x.mp4", { url: "x" }]]);
  const target = new Map([["https://other.com/page", other]]);
  mergeStoredMedia(target, "https://example.com/watch", []);

  assert.equal(target.get("https://other.com/page"), other);
  assert.equal(target.size, 2);
});

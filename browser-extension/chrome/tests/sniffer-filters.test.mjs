import test from "node:test";
import assert from "node:assert/strict";

import {
  MEDIA_CONTENT_TYPES,
  MEDIA_EXTENSIONS,
  extensionMatchesPath,
  isDashManifest,
  isHlsManifest,
  pathnameOf,
} from "../src/sniffer-filters.js";

test("isHlsManifest matches by URL extension and content-type", () => {
  assert.equal(isHlsManifest("https://cdn.example.com/master.m3u8", ""), true);
  assert.equal(isHlsManifest("https://cdn.example.com/MASTER.M3U8?token=x", ""), true);
  assert.equal(isHlsManifest("https://cdn.example.com/play", "application/x-mpegURL"), true);
  assert.equal(isHlsManifest("https://cdn.example.com/play", "application/vnd.apple.mpegurl"), true);
  assert.equal(isHlsManifest("https://cdn.example.com/play.mp4", "video/mp4"), false);
});

test("isDashManifest matches by URL extension and content-type", () => {
  assert.equal(isDashManifest("https://cdn.example.com/manifest.mpd", ""), true);
  assert.equal(isDashManifest("https://cdn.example.com/manifest.MPD?sig=y", ""), true);
  assert.equal(isDashManifest("https://cdn.example.com/play", "application/dash+xml"), true);
  assert.equal(isDashManifest("https://cdn.example.com/play.mp4", "video/mp4"), false);
});

test("extensionMatchesPath requires the extension to sit at a path boundary", () => {
  assert.equal(extensionMatchesPath("/video/x.mp4", ".mp4"), true);
  assert.equal(extensionMatchesPath("/file.mp4/range/0-100", ".mp4"), true);
  // The false positive that made /api/v2/list.mp4.json look like a video.
  assert.equal(extensionMatchesPath("/api/v2/list.mp4.json", ".mp4"), false);
  assert.equal(extensionMatchesPath("/mp4", ".mp4"), false);
  assert.equal(extensionMatchesPath("/x.mp4", ""), false);
  assert.equal(extensionMatchesPath(null, ".mp4"), false);
});

test("pathnameOf lowercases the path and drops the query string", () => {
  assert.equal(pathnameOf("https://cdn.example.com/Live/MASTER.M3U8?t=1"), "/live/master.m3u8");
  assert.equal(pathnameOf("https://cdn.example.com"), "/");
  assert.equal(pathnameOf("not a url"), null);
});

test("the shipped media tables stay well formed", () => {
  assert.ok(MEDIA_EXTENSIONS.every(ext => ext.startsWith(".") && ext === ext.toLowerCase()));
  assert.ok(MEDIA_CONTENT_TYPES.every(type => type.includes("/") && type === type.toLowerCase()));
  assert.equal(new Set(MEDIA_EXTENSIONS).size, MEDIA_EXTENSIONS.length);
  assert.equal(new Set(MEDIA_CONTENT_TYPES).size, MEDIA_CONTENT_TYPES.length);
});

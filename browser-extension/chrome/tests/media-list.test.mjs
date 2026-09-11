import test from "node:test";
import assert from "node:assert/strict";

import {
  DIRECT_MEDIA_MIN_BYTES,
  isListableMedia,
  isManifestEntry,
} from "../src/media-list.js";

test("isManifestEntry recognises both manifest kinds", () => {
  assert.equal(isManifestEntry({ mediaType: "hls" }), true);
  assert.equal(isManifestEntry({ mediaType: "dash" }), true);
  assert.equal(isManifestEntry({ mediaType: "video" }), false);
  assert.equal(isManifestEntry(null), false);
});

test("isListableMedia keeps a manifest whatever its size", () => {
  // Deep search reports a playlist it found in the page with no size at all.
  assert.equal(isListableMedia({ mediaType: "dash", contentLength: 0 }), true);
  assert.equal(isListableMedia({ mediaType: "hls", contentLength: 0 }), true);
  assert.equal(isListableMedia({ mediaType: "hls", contentLength: 312 }), true);
});

test("isListableMedia drops a plain file that is too small to be the video", () => {
  assert.equal(isListableMedia({ mediaType: "video", contentLength: 1024 }), false);
  assert.equal(isListableMedia({ mediaType: "video", contentLength: DIRECT_MEDIA_MIN_BYTES }), false);
  assert.equal(isListableMedia({ mediaType: "video", contentLength: DIRECT_MEDIA_MIN_BYTES + 1 }), true);
  assert.equal(isListableMedia({ mediaType: "audio", contentLength: 5_000_000 }), true);
});

test("isListableMedia tolerates a missing or junk size", () => {
  assert.equal(isListableMedia({ mediaType: "video" }), false);
  assert.equal(isListableMedia({ mediaType: "video", contentLength: null }), false);
  assert.equal(isListableMedia({ mediaType: "video", contentLength: "abc" }), false);
  assert.equal(isListableMedia(null), false);
});

test("the threshold is overridable for callers with their own policy", () => {
  assert.equal(isListableMedia({ mediaType: "video", contentLength: 2048 }, 1024), true);
});

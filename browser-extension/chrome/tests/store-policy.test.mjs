import test from "node:test";
import assert from "node:assert/strict";

import { STORE_BUILD, isStoreExcludedHost, isStoreExcludedPlatform, isStoreExcludedUrl } from "../src/store-policy.js";
import { detectSupportedMediaUrl } from "../src/detect.js";

test("the unpacked extension is not a store build", () => {
  assert.equal(STORE_BUILD, false);
  assert.equal(isStoreExcludedUrl("https://www.youtube.com/watch?v=abc"), false);
  assert.equal(detectSupportedMediaUrl("https://www.youtube.com/watch?v=abc")?.platform, "youtube");
});

test("a store build excludes every YouTube host", () => {
  for (const url of [
    "https://www.youtube.com/watch?v=abc",
    "https://youtube.com/shorts/abc",
    "https://music.youtube.com/watch?v=abc",
    "https://www.youtube-nocookie.com/embed/abc",
    "https://youtu.be/abc",
    "https://rr3---sn-abc.googlevideo.com/videoplayback?x=1",
    "https://i.ytimg.com/vi/abc/hq.jpg",
  ]) {
    assert.equal(isStoreExcludedUrl(url, true), true, url);
  }
  assert.equal(isStoreExcludedHost(".youtube.com", true), true);
});

test("a store build keeps other hosts and lookalikes", () => {
  for (const url of [
    "https://vimeo.com/123",
    "https://notyoutube.com/watch?v=abc",
    "https://youtube.com.example.org/watch",
    "https://example.org/?u=https://youtube.com/watch",
    "not a url",
  ]) {
    assert.equal(isStoreExcludedUrl(url, true), false, url);
  }
});

test("a store build excludes the YouTube platforms only", () => {
  assert.equal(isStoreExcludedPlatform("youtube", true), true);
  assert.equal(isStoreExcludedPlatform("youtube_music", true), true);
  assert.equal(isStoreExcludedPlatform("vimeo", true), false);
  assert.equal(isStoreExcludedPlatform("youtube", false), false);
});

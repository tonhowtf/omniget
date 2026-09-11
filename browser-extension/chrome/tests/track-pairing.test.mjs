import test from "node:test";
import assert from "node:assert/strict";

import { DEFAULT_PAIR_WINDOW_MS, pairTracks } from "../src/track-pairing.js";

const at = (mediaType, detectedAt, url) => ({ mediaType, detectedAt, url });

test("pairTracks pairs a video and an audio track that arrive together", () => {
  const video = at("video", 1000, "https://cdn.example.com/v.mp4");
  const audio = at("audio", 1080, "https://cdn.example.com/a.m4a");
  const { pairs, singles } = pairTracks([video, audio]);

  assert.equal(pairs.length, 1);
  assert.equal(pairs[0].video, video);
  assert.equal(pairs[0].audio, audio);
  assert.equal(pairs[0].detectedAt, 1080);
  assert.deepEqual(singles, []);
});

test("pairTracks leaves tracks further apart than the window alone", () => {
  const video = at("video", 1000, "https://cdn.example.com/v.mp4");
  const audio = at("audio", 1000 + DEFAULT_PAIR_WINDOW_MS + 1, "https://cdn.example.com/a.m4a");
  const { pairs, singles } = pairTracks([video, audio]);

  assert.equal(pairs.length, 0);
  assert.equal(singles.length, 2);
});

test("pairTracks never pairs two tracks of the same kind", () => {
  const { pairs, singles } = pairTracks([
    at("video", 1000, "https://cdn.example.com/a.mp4"),
    at("video", 1010, "https://cdn.example.com/b.mp4"),
  ]);
  assert.equal(pairs.length, 0);
  assert.equal(singles.length, 2);
});

test("pairTracks ignores manifests and leaves them as singles", () => {
  const hls = at("hls", 1000, "https://cdn.example.com/master.m3u8");
  const audio = at("audio", 1010, "https://cdn.example.com/a.m4a");
  const { pairs, singles } = pairTracks([hls, audio]);

  assert.equal(pairs.length, 0);
  assert.deepEqual(singles, [hls, audio]);
});

test("pairTracks pairs two independent streams without crossing them", () => {
  const v1 = at("video", 1000, "https://cdn.example.com/v1.mp4");
  const a1 = at("audio", 1050, "https://cdn.example.com/a1.m4a");
  const v2 = at("video", 5000, "https://cdn.example.com/v2.mp4");
  const a2 = at("audio", 5050, "https://cdn.example.com/a2.m4a");
  const { pairs, singles } = pairTracks([a2, v1, a1, v2]);

  assert.equal(pairs.length, 2);
  assert.deepEqual(pairs.map(p => p.video), [v1, v2]);
  assert.deepEqual(pairs.map(p => p.audio), [a1, a2]);
  assert.deepEqual(singles, []);
});

test("pairTracks uses each entry at most once", () => {
  const v = at("video", 1000, "https://cdn.example.com/v.mp4");
  const a1 = at("audio", 1010, "https://cdn.example.com/a1.m4a");
  const a2 = at("audio", 1020, "https://cdn.example.com/a2.m4a");
  const { pairs, singles } = pairTracks([v, a1, a2]);

  assert.equal(pairs.length, 1);
  assert.equal(pairs[0].audio, a1);
  assert.deepEqual(singles, [a2]);
});

test("pairTracks tolerates junk input", () => {
  assert.deepEqual(pairTracks(null), { pairs: [], singles: [] });
  assert.deepEqual(pairTracks([]), { pairs: [], singles: [] });
  const one = at("video", 1000, "https://cdn.example.com/v.mp4");
  assert.deepEqual(pairTracks([one]), { pairs: [], singles: [one] });
  const noTime = { mediaType: "video", url: "https://cdn.example.com/v.mp4" };
  assert.equal(pairTracks([noTime]).singles.length, 1);
});

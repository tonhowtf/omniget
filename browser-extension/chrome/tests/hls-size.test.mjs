import test from "node:test";
import assert from "node:assert/strict";

import {
  estimateHlsSize,
  estimateTotalBytes,
  isMediaPlaylist,
  parseSegmentUris,
  resolveAgainst,
} from "../src/hls-size.js";

const MEDIA_PLAYLIST = [
  "#EXTM3U",
  "#EXT-X-TARGETDURATION:10",
  "#EXTINF:10.0,",
  "seg-1.ts",
  "#EXTINF:10.0,",
  "seg-2.ts",
  "#EXTINF:10.0,",
  "https://other.example.com/seg-3.ts",
  "#EXT-X-ENDLIST",
].join("\n");

const MASTER_PLAYLIST = [
  "#EXTM3U",
  "#EXT-X-STREAM-INF:BANDWIDTH=800000,RESOLUTION=640x360",
  "360p.m3u8",
].join("\n");

function fakeResponse({ ok = true, body = "", contentLength = null } = {}) {
  return {
    ok,
    text: async () => body,
    headers: { get: (name) => (name.toLowerCase() === "content-length" ? contentLength : null) },
  };
}

test("resolveAgainst turns a relative segment into an absolute URL", () => {
  const base = "https://cdn.example.com/live/master.m3u8";
  assert.equal(resolveAgainst(base, "seg-1.ts"), "https://cdn.example.com/live/seg-1.ts");
  assert.equal(resolveAgainst(base, "/a/seg.ts"), "https://cdn.example.com/a/seg.ts");
  assert.equal(
    resolveAgainst(base, "https://other.example.com/seg.ts"),
    "https://other.example.com/seg.ts"
  );
  assert.equal(resolveAgainst("not a url", "seg.ts"), null);
});

test("parseSegmentUris takes the non-comment lines and resolves them", () => {
  const segments = parseSegmentUris(MEDIA_PLAYLIST, "https://cdn.example.com/live/master.m3u8");
  assert.deepEqual(segments, [
    "https://cdn.example.com/live/seg-1.ts",
    "https://cdn.example.com/live/seg-2.ts",
    "https://other.example.com/seg-3.ts",
  ]);
});

test("parseSegmentUris tolerates CRLF, blank lines and junk", () => {
  assert.deepEqual(parseSegmentUris("#EXTM3U\r\n\r\nseg.ts\r\n", "https://c.example.com/x/i.m3u8"), [
    "https://c.example.com/x/seg.ts",
  ]);
  assert.deepEqual(parseSegmentUris("", "https://c.example.com/i.m3u8"), []);
  assert.deepEqual(parseSegmentUris(null, "https://c.example.com/i.m3u8"), []);
});

test("isMediaPlaylist separates a media playlist from a master", () => {
  assert.equal(isMediaPlaylist(MEDIA_PLAYLIST), true);
  assert.equal(isMediaPlaylist(MASTER_PLAYLIST), false);
  assert.equal(isMediaPlaylist(null), false);
});

test("estimateTotalBytes averages the samples it actually got", () => {
  assert.equal(estimateTotalBytes([100, 200, 300], 10), 2000);
  assert.equal(estimateTotalBytes([100, null, 300], 10), 2000);
  assert.equal(estimateTotalBytes([1024], 1), 1024);
});

test("estimateTotalBytes returns null rather than a wrong number", () => {
  assert.equal(estimateTotalBytes([], 10), null);
  assert.equal(estimateTotalBytes([null, 0, -5], 10), null);
  assert.equal(estimateTotalBytes([100], 0), null);
  assert.equal(estimateTotalBytes(null, 10), null);
});

test("estimateHlsSize multiplies the sampled average by the segment count", async () => {
  const seen = [];
  const fetchImpl = async (url, init) => {
    seen.push({ url, method: init?.method ?? "GET" });
    if (url.endsWith("master.m3u8")) return fakeResponse({ body: MEDIA_PLAYLIST });
    return fakeResponse({ contentLength: "1000000" });
  };

  const total = await estimateHlsSize("https://cdn.example.com/live/master.m3u8", { fetchImpl });
  assert.equal(total, 3_000_000);
  assert.equal(seen.filter(r => r.method === "HEAD").length, 3);
});

test("estimateHlsSize samples at most sampleCount segments", async () => {
  const manifest = ["#EXTM3U", ...Array.from({ length: 50 }, (_, i) => `#EXTINF:10.0,\nseg-${i}.ts`)].join("\n");
  let heads = 0;
  const fetchImpl = async (url, init) => {
    if (init?.method === "HEAD") {
      heads++;
      return fakeResponse({ contentLength: "500" });
    }
    return fakeResponse({ body: manifest });
  };

  const total = await estimateHlsSize("https://cdn.example.com/live/master.m3u8", {
    fetchImpl,
    sampleCount: 4,
  });
  assert.equal(heads, 4);
  assert.equal(total, 25_000);
});

test("estimateHlsSize gives up quietly instead of throwing", async () => {
  const throwing = async () => { throw new Error("network down"); };
  assert.equal(await estimateHlsSize("https://cdn.example.com/x.m3u8", { fetchImpl: throwing }), null);

  const notOk = async () => fakeResponse({ ok: false });
  assert.equal(await estimateHlsSize("https://cdn.example.com/x.m3u8", { fetchImpl: notOk }), null);

  const master = async () => fakeResponse({ body: MASTER_PLAYLIST });
  assert.equal(await estimateHlsSize("https://cdn.example.com/x.m3u8", { fetchImpl: master }), null);

  assert.equal(await estimateHlsSize("", { fetchImpl: master }), null);
  assert.equal(await estimateHlsSize("https://cdn.example.com/x.m3u8", { fetchImpl: null }), null);
});

test("estimateHlsSize survives segments that refuse HEAD", async () => {
  const fetchImpl = async (url, init) => {
    if (init?.method !== "HEAD") return fakeResponse({ body: MEDIA_PLAYLIST });
    if (url.includes("seg-1")) return fakeResponse({ ok: false });
    return fakeResponse({ contentLength: "2000" });
  };
  const total = await estimateHlsSize("https://cdn.example.com/live/master.m3u8", { fetchImpl });
  assert.equal(total, 6000);
});

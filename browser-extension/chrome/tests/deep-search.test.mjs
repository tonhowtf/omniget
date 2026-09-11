import test from "node:test";
import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";

await import("../src/deep-search.js");

const {
  absolutizeManifest,
  classifyManifest,
  shouldReadBody,
  syntheticManifestUrl,
  parseContentLength,
  toHttpUrl,
} = globalThis.__omnigetDeepSearchInternals;

const BASE = "https://cdn.example.com/videos/abc/master.m3u8";

test("internals are exposed only when there is no page global", () => {
  assert.equal(typeof globalThis.window, "undefined");
  assert.equal(typeof absolutizeManifest, "function");
  assert.equal(typeof classifyManifest, "function");
  assert.equal(typeof shouldReadBody, "function");
  assert.equal(typeof syntheticManifestUrl, "function");
});

// ------------------------------------------------------------ classifyManifest

test("classifyManifest recognises a plain HLS playlist", () => {
  assert.equal(classifyManifest("#EXTM3U\n#EXT-X-VERSION:3\n"), "hls");
});

test("classifyManifest tolerates a BOM and leading whitespace", () => {
  assert.equal(classifyManifest("﻿#EXTM3U\n"), "hls");
  assert.equal(classifyManifest("\n\n   #EXTM3U\n"), "hls");
  assert.equal(classifyManifest("﻿\r\n#EXTM3U\n"), "hls");
});

test("classifyManifest is case-insensitive on the HLS marker", () => {
  assert.equal(classifyManifest("#extm3u\n#EXTINF:4,\n"), "hls");
});

test("classifyManifest recognises a DASH manifest", () => {
  const mpd = '<?xml version="1.0"?>\n<MPD xmlns="urn:mpeg:dash:schema:mpd:2011" type="static"></MPD>';
  assert.equal(classifyManifest(mpd), "dash");
});

test("classifyManifest recognises a DASH manifest without an XML declaration", () => {
  assert.equal(classifyManifest('<MPD type="dynamic"></MPD>'), "dash");
});

test("classifyManifest rejects HTML, JSON and empty input", () => {
  assert.equal(classifyManifest("<!doctype html><html><body>hi</body></html>"), null);
  assert.equal(classifyManifest("<html><head><title>MPD</title></head></html>"), null);
  assert.equal(classifyManifest('{"playlist":"https://a/b.m3u8"}'), null);
  assert.equal(classifyManifest("[1,2,3]"), null);
  assert.equal(classifyManifest(""), null);
  assert.equal(classifyManifest("   \n  "), null);
});

test("classifyManifest rejects non-strings", () => {
  assert.equal(classifyManifest(null), null);
  assert.equal(classifyManifest(undefined), null);
  assert.equal(classifyManifest(42), null);
  assert.equal(classifyManifest({ text: "#EXTM3U" }), null);
});

test("classifyManifest gives up on a large blob whose marker is not at the top", () => {
  const junk = "x".repeat(200000) + "\n#EXTM3U\n";
  assert.equal(classifyManifest(junk), null);
});

// ---------------------------------------------------------- absolutizeManifest

test("absolutizeManifest resolves relative segment lines", () => {
  const text = "#EXTM3U\n#EXTINF:4.0,\nseg1.ts\n#EXTINF:4.0,\nseg2.ts\n";
  const out = absolutizeManifest(text, BASE);
  assert.match(out, /^https:\/\/cdn\.example\.com\/videos\/abc\/seg1\.ts$/m);
  assert.match(out, /^https:\/\/cdn\.example\.com\/videos\/abc\/seg2\.ts$/m);
});

test("absolutizeManifest resolves root-relative segment lines against the origin", () => {
  const out = absolutizeManifest("#EXTM3U\n/media/seg1.ts\n", BASE);
  assert.match(out, /^https:\/\/cdn\.example\.com\/media\/seg1\.ts$/m);
});

test("absolutizeManifest leaves already absolute segments untouched", () => {
  const text = "#EXTM3U\nhttps://other.example.net/a/seg1.ts\n";
  assert.equal(absolutizeManifest(text, BASE), text);
});

test("absolutizeManifest resolves a relative EXT-X-KEY URI", () => {
  const text = '#EXTM3U\n#EXT-X-KEY:METHOD=AES-128,URI="key.bin",IV=0x1\nseg1.ts\n';
  const out = absolutizeManifest(text, BASE);
  assert.ok(out.includes('URI="https://cdn.example.com/videos/abc/key.bin"'));
  assert.ok(out.includes("METHOD=AES-128"));
  assert.ok(out.includes("IV=0x1"));
});

test("absolutizeManifest resolves a relative EXT-X-MAP URI", () => {
  const text = '#EXTM3U\n#EXT-X-MAP:URI="init.mp4"\nseg1.m4s\n';
  const out = absolutizeManifest(text, BASE);
  assert.ok(out.includes('URI="https://cdn.example.com/videos/abc/init.mp4"'));
  assert.match(out, /^https:\/\/cdn\.example\.com\/videos\/abc\/seg1\.m4s$/m);
});

test("absolutizeManifest resolves a relative EXT-X-MEDIA URI", () => {
  const text = '#EXTM3U\n#EXT-X-MEDIA:TYPE=AUDIO,GROUP-ID="a",URI="audio/en.m3u8"\n';
  const out = absolutizeManifest(text, BASE);
  assert.ok(out.includes('URI="https://cdn.example.com/videos/abc/audio/en.m3u8"'));
});

test("absolutizeManifest leaves an absolute URI attribute alone", () => {
  const text = '#EXTM3U\n#EXT-X-KEY:METHOD=AES-128,URI="https://keys.example.net/k1"\n';
  assert.equal(absolutizeManifest(text, BASE), text);
});

test("absolutizeManifest preserves CRLF line endings", () => {
  const text = "#EXTM3U\r\n#EXTINF:4.0,\r\nseg1.ts\r\n";
  const out = absolutizeManifest(text, BASE);
  assert.ok(out.includes("\r\n"));
  assert.equal(out.split("\r\n").length, text.split("\r\n").length);
  assert.ok(out.includes("https://cdn.example.com/videos/abc/seg1.ts\r\n"));
  assert.ok(!out.includes("seg1.ts\n\r"));
});

test("absolutizeManifest preserves LF-only playlists byte for byte outside the URLs", () => {
  const text = "#EXTM3U\n\n# a comment line\n#EXT-X-ENDLIST\n";
  assert.equal(absolutizeManifest(text, BASE), text);
});

test("absolutizeManifest leaves tags without a URI and blank lines untouched", () => {
  const text = "#EXTM3U\n#EXT-X-VERSION:3\n#EXT-X-TARGETDURATION:10\n\n#EXTINF:9.0,\nseg0.ts\n#EXT-X-ENDLIST\n";
  const out = absolutizeManifest(text, BASE);
  assert.ok(out.startsWith("#EXTM3U\n#EXT-X-VERSION:3\n#EXT-X-TARGETDURATION:10\n\n#EXTINF:9.0,\n"));
  assert.ok(out.endsWith("#EXT-X-ENDLIST\n"));
});

test("absolutizeManifest returns the text untouched for an invalid base url", () => {
  const text = "#EXTM3U\nseg1.ts\n";
  assert.equal(absolutizeManifest(text, "not-a-url"), text);
  assert.equal(absolutizeManifest(text, ""), text);
  assert.equal(absolutizeManifest(text, undefined), text);
});

test("absolutizeManifest keeps query strings on resolved segments", () => {
  const out = absolutizeManifest("#EXTM3U\nseg1.ts?token=abc123\n", BASE);
  assert.match(out, /^https:\/\/cdn\.example\.com\/videos\/abc\/seg1\.ts\?token=abc123$/m);
});

test("absolutizeManifest handles empty input", () => {
  assert.equal(absolutizeManifest("", BASE), "");
});

// --------------------------------------------------------------- shouldReadBody

test("shouldReadBody accepts playlist and data content types", () => {
  assert.equal(shouldReadBody("application/vnd.apple.mpegurl", "1024"), true);
  assert.equal(shouldReadBody("application/x-mpegURL", "1024"), true);
  assert.equal(shouldReadBody("application/dash+xml", "2048"), true);
  assert.equal(shouldReadBody("application/json; charset=utf-8", "500"), true);
  assert.equal(shouldReadBody("text/plain", "500"), true);
  assert.equal(shouldReadBody("application/octet-stream", "500"), true);
  assert.equal(shouldReadBody("text/xml", "500"), true);
});

test("shouldReadBody refuses media and other unrelated content types", () => {
  assert.equal(shouldReadBody("video/mp4", "1024"), false);
  assert.equal(shouldReadBody("video/mp2t", "1024"), false);
  assert.equal(shouldReadBody("audio/mpeg", "1024"), false);
  assert.equal(shouldReadBody("image/png", "1024"), false);
  assert.equal(shouldReadBody("text/html", "1024"), false);
  assert.equal(shouldReadBody("text/javascript", "1024"), false);
});

test("shouldReadBody refuses a missing or empty content type", () => {
  assert.equal(shouldReadBody("", "1024"), false);
  assert.equal(shouldReadBody(null, "1024"), false);
  assert.equal(shouldReadBody(undefined, null), false);
});

test("shouldReadBody refuses a body over 4 MB", () => {
  const cap = 4 * 1024 * 1024;
  assert.equal(shouldReadBody("application/json", String(cap)), true);
  assert.equal(shouldReadBody("application/json", String(cap + 1)), false);
  assert.equal(shouldReadBody("application/x-mpegURL", String(50 * 1024 * 1024)), false);
});

test("shouldReadBody accepts a missing or unparseable content length", () => {
  assert.equal(shouldReadBody("application/x-mpegURL", null), true);
  assert.equal(shouldReadBody("application/x-mpegURL", undefined), true);
  assert.equal(shouldReadBody("application/x-mpegURL", ""), true);
  assert.equal(shouldReadBody("application/x-mpegURL", "chunked"), true);
});

test("parseContentLength normalises header values", () => {
  assert.equal(parseContentLength("1024"), 1024);
  assert.equal(parseContentLength(1024), 1024);
  assert.equal(parseContentLength("-1"), null);
  assert.equal(parseContentLength("abc"), null);
  assert.equal(parseContentLength(null), null);
});

// ---------------------------------------------------------- syntheticManifestUrl

test("syntheticManifestUrl builds an https url on a reserved host", () => {
  const url = syntheticManifestUrl("https://player.example.com/watch?v=1");
  // Never the page's own host: the app picks its downloader by host, so a
  // synthetic URL on a known platform would be routed to that platform's
  // extractor instead of using the playlist text we captured.
  assert.ok(url.startsWith("https://deep-search.omniget.invalid/"));
  assert.equal(new URL(url).hostname.endsWith(".invalid"), true);
  // The page host stays in the path, so a log line still says where it came from.
  assert.ok(url.includes("player.example.com"));
  assert.ok(url.endsWith(".m3u8"));
});

test("syntheticManifestUrl keeps the port of the page host in the path", () => {
  const url = syntheticManifestUrl("http://localhost:8080/watch");
  assert.ok(url.startsWith("https://deep-search.omniget.invalid/"));
  assert.ok(url.includes("localhost%3A8080"));
});

test("syntheticManifestUrl gives a different id on every call", () => {
  const a = syntheticManifestUrl("https://player.example.com/watch");
  const b = syntheticManifestUrl("https://player.example.com/watch");
  assert.notEqual(a, b);
});

test("syntheticManifestUrl returns null for non http(s) pages", () => {
  assert.equal(syntheticManifestUrl("about:blank"), null);
  assert.equal(syntheticManifestUrl("file:///tmp/page.html"), null);
  assert.equal(syntheticManifestUrl("blob:https://example.com/uuid"), null);
  assert.equal(syntheticManifestUrl(""), null);
  assert.equal(syntheticManifestUrl(undefined), null);
});

// ------------------------------------------------------------------- toHttpUrl

test("toHttpUrl keeps real http(s) urls and drops everything else", () => {
  assert.equal(toHttpUrl("https://a.example.com/x.m3u8"), "https://a.example.com/x.m3u8");
  assert.equal(toHttpUrl("http://a.example.com/x.m3u8"), "http://a.example.com/x.m3u8");
  assert.equal(toHttpUrl("blob:https://a.example.com/uuid"), null);
  assert.equal(toHttpUrl("data:application/x-mpegurl,%23EXTM3U"), null);
  assert.equal(toHttpUrl("seg1.ts"), null);
  assert.equal(toHttpUrl(""), null);
  assert.equal(toHttpUrl(null), null);
});

// --------------------------------------------------------- deep-search-toggle

const toggle = await import("../src/deep-search-toggle.js");

test("deep search is opt-in and stores its own flag", () => {
  assert.equal(toggle.DEEP_SEARCH_STORAGE_KEY, "omniget_deep_search_enabled");
  assert.equal(toggle.isDeepSearchEnabled(), false);
});

test("deep search reports itself unsupported without chrome.scripting", async () => {
  assert.equal(await toggle.isDeepSearchSupported(), false);
  assert.equal(await toggle.registerDeepSearchScripts(), false);
  assert.equal(await toggle.unregisterDeepSearchScripts(), false);
});

test("shouldSkipDeepSearch covers trackers and heavy apps but not media sites", () => {
  assert.equal(toggle.shouldSkipDeepSearch("https://www.google-analytics.com/collect"), true);
  assert.equal(toggle.shouldSkipDeepSearch("https://docs.google.com/document/d/1"), true);
  assert.equal(toggle.shouldSkipDeepSearch("https://web.whatsapp.com/"), true);
  assert.equal(toggle.shouldSkipDeepSearch("https://www.youtube.com/watch?v=1"), false);
  assert.equal(toggle.shouldSkipDeepSearch("https://player.vimeo.com/video/1"), false);
  assert.equal(toggle.shouldSkipDeepSearch("not-a-url"), false);
});

test("exclude matches are valid patterns built only from real hostnames", () => {
  const patterns = toggle.getDeepSearchExcludeMatches();
  assert.ok(patterns.length > 0);
  assert.ok(patterns.every((p) => /^\*:\/\/\*\.[a-z0-9.-]+\/\*$/.test(p)));
  assert.ok(patterns.includes("*://*.sentry.io/*"));
  // "analytics" is a bare substring in the shared blocklist, not a hostname.
  assert.ok(!patterns.some((p) => p === "*://*.analytics/*"));
});

// ------------------------------------------------ capability probe permissions

test("the MAIN-world probe stays inside a host permission the extension always holds", async () => {
  const { DEEP_SEARCH_PROBE_MATCHES } = await import("../src/deep-search-toggle.js");
  const manifests = await Promise.all(
    ["../manifest.json", "../../firefox/manifest.json"].map(async (rel) =>
      JSON.parse(await readFile(new URL(rel, import.meta.url), "utf8"))
    )
  );

  assert.ok(DEEP_SEARCH_PROBE_MATCHES.length > 0);
  for (const manifest of manifests) {
    for (const pattern of DEEP_SEARCH_PROBE_MATCHES) {
      const origin = new URL(pattern.replace("*://", "https://")).origin;
      const covered = manifest.host_permissions.some((granted) => {
        try {
          return new URL(granted.replace("*://", "https://")).origin === origin;
        } catch {
          return false;
        }
      });
      // registerContentScripts refuses a pattern the extension has no host
      // permission for. If the probe needed the optional wildcard, a fresh
      // install would read the refusal as "MAIN world unsupported" and hide the
      // toggle that asks for the wildcard.
      assert.ok(covered, `${pattern} is not covered by a fixed host permission`);
    }
  }
});

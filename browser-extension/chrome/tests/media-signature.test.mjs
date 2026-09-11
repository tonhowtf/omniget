import test from "node:test";
import assert from "node:assert/strict";

import {
  isPlausibleMedia,
  looksLikeHtml,
  sniffMediaFormat,
  stripImageWrapper,
} from "../src/media-signature.js";

function ascii(text) {
  return Array.from(text, (char) => char.charCodeAt(0));
}

function pad(bytes, length) {
  const out = new Uint8Array(length);
  out.set(bytes.slice(0, length), 0);
  return out;
}

// ISO-BMFF: 4-byte box size, then the box type at offset 4.
function isoBmff(boxType) {
  return pad([0x00, 0x00, 0x00, 0x18, ...ascii(boxType), ...ascii("isom")], 32);
}

// MPEG-TS: 0x47 sync byte at the head of every 188-byte packet.
function mpegTs(packets = 3, offset = 0) {
  const out = new Uint8Array(offset + packets * 188);
  for (let i = 0; i < packets; i += 1) {
    out[offset + i * 188] = 0x47;
    out[offset + i * 188 + 1] = 0x40;
  }
  return out;
}

const WEBM = pad([0x1a, 0x45, 0xdf, 0xa3, 0x01, 0x00, 0x00, 0x00, 0x00], 32);
const MP3_ID3 = pad([...ascii("ID3"), 0x03, 0x00, 0x00, 0x00, 0x00, 0x00, 0x21], 32);
// FF FB: 11-bit sync, MPEG-1, layer III.
const MP3_SYNC = pad([0xff, 0xfb, 0x90, 0x64, 0x00, 0x00, 0x00, 0x00], 32);
// FF F1: 12-bit ADTS sync, MPEG-4, layer field 00, no CRC.
const AAC_ADTS = pad([0xff, 0xf1, 0x4c, 0x80, 0x02, 0x1f, 0xfc, 0x00], 32);
const HTML_ERROR = new Uint8Array(
  ascii("<!DOCTYPE html><html><head><title>404 Not Found</title></head><body>nope</body></html>"),
);

test("sniffMediaFormat recognizes a plain mp4 by its ftyp box", () => {
  assert.equal(sniffMediaFormat(isoBmff("ftyp")), "mp4");
});

test("sniffMediaFormat recognizes fragmented mp4 by styp and by moof", () => {
  assert.equal(sniffMediaFormat(isoBmff("styp")), "fmp4");
  assert.equal(sniffMediaFormat(isoBmff("moof")), "fmp4");
});

test("sniffMediaFormat recognizes webm by its EBML header", () => {
  assert.equal(sniffMediaFormat(WEBM), "webm");
});

test("sniffMediaFormat recognizes mp3 through an ID3 tag", () => {
  assert.equal(sniffMediaFormat(MP3_ID3), "mp3");
});

test("sniffMediaFormat recognizes mp3 through a raw frame sync", () => {
  assert.equal(sniffMediaFormat(MP3_SYNC), "mp3");
});

test("sniffMediaFormat recognizes aac through an ADTS header", () => {
  assert.equal(sniffMediaFormat(AAC_ADTS), "aac");
});

test("sniffMediaFormat separates ADTS from an mp3 sync sharing the 0xF0 mask", () => {
  // FF FB satisfies the loose 0xF0 test but its layer field is not 00, so it is
  // mp3; FF F9 (ADTS, MPEG-2, layer 00) must still come back as aac.
  assert.equal(sniffMediaFormat(pad([0xff, 0xfb, 0x90, 0x64, 0x00, 0x00, 0x00, 0x00], 32)), "mp3");
  assert.equal(sniffMediaFormat(pad([0xff, 0xf9, 0x4c, 0x80, 0x02, 0x1f, 0xfc, 0x00], 32)), "aac");
});

test("sniffMediaFormat recognizes mpeg-ts by aligned sync bytes", () => {
  assert.equal(sniffMediaFormat(mpegTs(3)), "ts");
});

test("sniffMediaFormat needs at least two aligned ts sync bytes", () => {
  const single = new Uint8Array(400);
  single[0] = 0x47;
  assert.equal(sniffMediaFormat(single), null);
});

test("sniffMediaFormat accepts ArrayBuffer and plain arrays alike", () => {
  const bytes = isoBmff("ftyp");
  assert.equal(sniffMediaFormat(bytes.buffer), "mp4");
  assert.equal(sniffMediaFormat(Array.from(bytes)), "mp4");
});

test("sniffMediaFormat returns null for an HTML error page", () => {
  assert.equal(sniffMediaFormat(HTML_ERROR), null);
});

test("sniffMediaFormat returns null for empty, null and short buffers", () => {
  assert.equal(sniffMediaFormat(null), null);
  assert.equal(sniffMediaFormat(undefined), null);
  assert.equal(sniffMediaFormat(new Uint8Array(0)), null);
  assert.equal(sniffMediaFormat([]), null);
  assert.equal(sniffMediaFormat(new Uint8Array([0xff, 0xfb, 0x90])), null);
});

test("looksLikeHtml catches doctype, html and xml openings", () => {
  assert.equal(looksLikeHtml(HTML_ERROR), true);
  assert.equal(looksLikeHtml(new Uint8Array(ascii("<html><body>error</body></html>"))), true);
  assert.equal(looksLikeHtml(new Uint8Array(ascii('<?xml version="1.0"?><Error/>'))), true);
});

test("looksLikeHtml ignores a BOM and leading whitespace and is case-insensitive", () => {
  const withBom = new Uint8Array([0xef, 0xbb, 0xbf, ...ascii("\r\n  <!DoCtYpE HTML>")]);
  assert.equal(looksLikeHtml(withBom), true);
});

test("looksLikeHtml is false for media and for empty input", () => {
  assert.equal(looksLikeHtml(isoBmff("ftyp")), false);
  assert.equal(looksLikeHtml(mpegTs(3)), false);
  assert.equal(looksLikeHtml(new Uint8Array(0)), false);
  assert.equal(looksLikeHtml(null), false);
});

test("isPlausibleMedia rejects the CDN that answers 200 with an HTML error", () => {
  assert.equal(isPlausibleMedia(HTML_ERROR), false);
  assert.equal(isPlausibleMedia(new Uint8Array(ascii('<?xml version="1.0"?><Error><Code>AccessDenied</Code></Error>'))), false);
});

test("isPlausibleMedia accepts real payloads and rejects junk", () => {
  assert.equal(isPlausibleMedia(isoBmff("ftyp")), true);
  assert.equal(isPlausibleMedia(mpegTs(3)), true);
  assert.equal(isPlausibleMedia(AAC_ADTS), true);
  assert.equal(isPlausibleMedia(new Uint8Array(64)), false);
  assert.equal(isPlausibleMedia(null), false);
});

test("stripImageWrapper unwraps a ts segment hidden behind a PNG", () => {
  const ts = mpegTs(3);
  const wrapped = new Uint8Array([
    0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a,
    0x00, 0x00, 0x00, 0x0d, ...ascii("IHDR"), 0x01, 0x02, 0x03, 0x04,
    0x00, 0x00, 0x00, 0x00, ...ascii("IEND"), 0xae, 0x42, 0x60, 0x82,
    ...ts,
  ]);
  const stripped = stripImageWrapper(wrapped);
  assert.ok(stripped instanceof Uint8Array);
  assert.deepEqual(Array.from(stripped), Array.from(ts));
  assert.equal(sniffMediaFormat(stripped), "ts");
});

test("stripImageWrapper unwraps an fmp4 segment hidden behind a JPEG", () => {
  const segment = isoBmff("styp");
  const wrapped = new Uint8Array([
    0xff, 0xd8, 0xff, 0xe0, 0x00, 0x10, ...ascii("JFIF"), 0x00,
    0x01, 0x02, 0x03,
    0xff, 0xd9,
    ...segment,
  ]);
  const stripped = stripImageWrapper(wrapped);
  assert.ok(stripped instanceof Uint8Array);
  assert.deepEqual(Array.from(stripped), Array.from(segment));
  assert.equal(sniffMediaFormat(stripped), "fmp4");
});

test("stripImageWrapper returns the very same reference when there is no wrapper", () => {
  const ts = mpegTs(3);
  assert.equal(stripImageWrapper(ts), ts);
  const array = Array.from(isoBmff("ftyp"));
  assert.equal(stripImageWrapper(array), array);
  const buffer = isoBmff("ftyp").buffer;
  assert.equal(stripImageWrapper(buffer), buffer);
  assert.equal(stripImageWrapper(null), null);
  const short = new Uint8Array([0x89, 0x50, 0x4e]);
  assert.equal(stripImageWrapper(short), short);
});

test("stripImageWrapper keeps a PNG whose IEND marker is missing", () => {
  const truncated = new Uint8Array([
    0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a,
    0x00, 0x00, 0x00, 0x0d, ...ascii("IHDR"), 0x01, 0x02, 0x03, 0x04,
  ]);
  assert.equal(stripImageWrapper(truncated), truncated);
});

test("stripImageWrapper keeps a JPEG whose EOI marker is missing", () => {
  const truncated = new Uint8Array([0xff, 0xd8, 0xff, 0xe0, 0x00, 0x10, ...ascii("JFIF"), 0x00]);
  assert.equal(stripImageWrapper(truncated), truncated);
});

test("stripImageWrapper keeps a PNG with nothing after the IEND chunk", () => {
  const emptyTail = new Uint8Array([
    0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a,
    0x00, 0x00, 0x00, 0x00, ...ascii("IEND"), 0xae, 0x42, 0x60, 0x82,
  ]);
  assert.equal(stripImageWrapper(emptyTail), emptyTail);
});

test("looksLikeHtml does not condemn a file just for being XML", () => {
  // An SVG exported by any design tool opens with an XML declaration, and the
  // desktop app accepts .svg as a direct download.
  const svg = ascii('<?xml version="1.0" encoding="UTF-8"?>\n<svg width="10" height="10"></svg>');
  assert.equal(looksLikeHtml(svg), false);

  const mpd = ascii('<?xml version="1.0"?><MPD xmlns="urn:mpeg:dash:schema:mpd:2011"></MPD>');
  assert.equal(looksLikeHtml(mpd), false);

  const rss = ascii('<?xml version="1.0"?><rss version="2.0"><channel></channel></rss>');
  assert.equal(looksLikeHtml(rss), false);
});

test("looksLikeHtml still catches XML that is a document, not a file", () => {
  const xhtml = ascii('<?xml version="1.0"?><!DOCTYPE html><html><body>403</body></html>');
  assert.equal(looksLikeHtml(xhtml), true);

  // The body an S3-style object store returns for a denied or expired link.
  const s3 = ascii('<?xml version="1.0"?><Error><Code>AccessDenied</Code></Error>');
  assert.equal(looksLikeHtml(s3), true);
});

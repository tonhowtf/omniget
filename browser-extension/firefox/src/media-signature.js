// Portions adapted from cat-catch (js/m3u8.js)
// Copyright (c) xifangczy — https://github.com/xifangczy/cat-catch
// Licensed under GPL-3.0, same as this project.

// Binary signature recognition for downloaded media payloads.
// The motivating case: a CDN answers 200 OK with an HTML error page, and the
// download lands on disk as a 3 KB ".mp4" that no player can open. Sniffing the
// first bytes lets the caller reject that response instead of keeping it.

// Shortest payload we are willing to judge. An ISO-BMFF box type only starts at
// offset 4, so anything below 8 bytes cannot be classified with confidence.
const MIN_SNIFF_BYTES = 8;

// MPEG-TS packets are 188 bytes long and each one starts with a 0x47 sync byte.
const TS_PACKET_SIZE = 188;
// Only the head of the payload is scanned for the first sync byte.
const TS_SCAN_LIMIT = 512;
// Two aligned sync bytes (one packet boundary apart) are enough evidence.
const TS_MIN_SYNC_HITS = 2;

const PNG_SIGNATURE = [0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a];
// IEND chunk type, followed by its 4-byte CRC32.
const PNG_IEND = [0x49, 0x45, 0x4e, 0x44];
const PNG_IEND_TRAILER = 8;
const JPEG_EOI_TRAILER = 2;

const UTF8_BOM = [0xef, 0xbb, 0xbf];
// Enough bytes to see the opening tag of an error page past any leading noise.
const HTML_PROBE_BYTES = 128;
const HTML_PREFIXES = ["<!doctype", "<html"];

/**
 * Normalize any accepted input into a Uint8Array view.
 * @param {ArrayBuffer|Uint8Array|ArrayBufferView|number[]|null|undefined} buffer
 * @returns {Uint8Array|null}
 */
function toBytes(buffer) {
  if (buffer === null || buffer === undefined) return null;
  if (buffer instanceof Uint8Array) return buffer;
  if (buffer instanceof ArrayBuffer) return new Uint8Array(buffer);
  if (ArrayBuffer.isView(buffer)) {
    return new Uint8Array(buffer.buffer, buffer.byteOffset, buffer.byteLength);
  }
  if (Array.isArray(buffer)) return Uint8Array.from(buffer);
  return null;
}

/**
 * Check that `bytes` carries `pattern` starting at `offset`.
 * @param {Uint8Array} bytes
 * @param {number[]} pattern
 * @param {number} offset
 * @returns {boolean}
 */
function matchesAt(bytes, pattern, offset) {
  if (offset + pattern.length > bytes.length) return false;
  for (let i = 0; i < pattern.length; i += 1) {
    if (bytes[offset + i] !== pattern[i]) return false;
  }
  return true;
}

/**
 * Find the first index of `pattern` at or after `from`, or -1.
 * @param {Uint8Array} bytes
 * @param {number[]} pattern
 * @param {number} from
 * @returns {number}
 */
function indexOfPattern(bytes, pattern, from) {
  const last = bytes.length - pattern.length;
  for (let i = Math.max(0, from); i <= last; i += 1) {
    if (matchesAt(bytes, pattern, i)) return i;
  }
  return -1;
}

/**
 * Detect MPEG-TS by locating two sync bytes exactly one packet apart.
 * @param {Uint8Array} bytes
 * @returns {boolean}
 */
function isMpegTs(bytes) {
  const scanEnd = Math.min(TS_SCAN_LIMIT, bytes.length);
  for (let start = 0; start < scanEnd; start += 1) {
    if (bytes[start] !== 0x47) continue;
    let hits = 1;
    for (let at = start + TS_PACKET_SIZE; at < bytes.length; at += TS_PACKET_SIZE) {
      if (bytes[at] !== 0x47) break;
      hits += 1;
      if (hits >= TS_MIN_SYNC_HITS) return true;
    }
  }
  return false;
}

/**
 * Report whether the payload is really an HTML/XML document — the shape a CDN
 * error page takes when it is served with a 200 status and a media file name.
 * @param {ArrayBuffer|Uint8Array|number[]|null|undefined} buffer
 * @returns {boolean}
 */
export function looksLikeHtml(buffer) {
  const bytes = toBytes(buffer);
  if (!bytes || bytes.length === 0) return false;

  let start = 0;
  if (matchesAt(bytes, UTF8_BOM, 0)) start = UTF8_BOM.length;
  // Skip leading whitespace: space, tab, LF, CR, FF, VT.
  while (start < bytes.length) {
    const byte = bytes[start];
    const isSpace =
      byte === 0x20 || byte === 0x09 || byte === 0x0a || byte === 0x0d || byte === 0x0c || byte === 0x0b;
    if (!isSpace) break;
    start += 1;
  }
  if (start >= bytes.length) return false;

  const end = Math.min(bytes.length, start + HTML_PROBE_BYTES);
  let head = "";
  for (let i = start; i < end; i += 1) {
    head += String.fromCharCode(bytes[i]);
  }
  head = head.toLowerCase();
  if (HTML_PREFIXES.some((prefix) => head.startsWith(prefix))) return true;

  // An XML declaration on its own proves nothing: an SVG exported by any design
  // tool opens with one, and so does a DASH manifest. What separates a document
  // from a file is the element that follows — `<html` for an XHTML error page,
  // `<error` for the XML body an S3-style object store returns for a denied or
  // expired link. Content roots (`<svg`, `<mpd`, `<rss`) carry neither.
  if (head.startsWith("<?xml")) {
    return head.includes("<html") || head.includes("<error");
  }
  return false;
}

/**
 * Recognize a media container from its leading bytes.
 * @param {ArrayBuffer|Uint8Array|number[]|null|undefined} buffer
 * @returns {"mp4"|"fmp4"|"webm"|"mp3"|"aac"|"ts"|null}
 */
export function sniffMediaFormat(buffer) {
  const bytes = toBytes(buffer);
  if (!bytes || bytes.length < MIN_SNIFF_BYTES) return null;

  // An HTML error page can never be media, and its text could in principle trip
  // one of the looser probes below, so it is ruled out first.
  if (looksLikeHtml(bytes)) return null;

  // MPEG-TS goes first: its own detector is the strictest one here (two sync
  // bytes exactly 188 bytes apart), while the byte-mask probes further down
  // could in principle claim a transport stream by accident.
  if (isMpegTs(bytes)) return "ts";

  // ISO-BMFF: the box type sits at offset 4. "styp" (segment) and "moof"
  // (movie fragment) mean fragmented MP4; plain "ftyp" means a regular MP4.
  if (bytes[5] === 0x74 && bytes[6] === 0x79 && bytes[7] === 0x70) {
    if (bytes[4] === 0x73) return "fmp4";
    if (bytes[4] === 0x66) return "mp4";
  }
  if (bytes[4] === 0x6d && bytes[5] === 0x6f && bytes[6] === 0x6f && bytes[7] === 0x66) {
    return "fmp4";
  }

  // Matroska / WebM EBML header.
  if (bytes[0] === 0x1a && bytes[1] === 0x45 && bytes[2] === 0xdf && bytes[3] === 0xa3) {
    return "webm";
  }

  // MP3 with an ID3v2 tag in front.
  if (bytes[0] === 0x49 && bytes[1] === 0x44 && bytes[2] === 0x33) return "mp3";

  // 0xFF sync collision between ADTS (AAC) and a raw MPEG audio frame (MP3):
  // both start with 0xFF and both keep their sync bits in the top of byte 1.
  // The MP3 test is the looser one (11-bit sync, mask 0xE0) and ADTS the more
  // specific (12-bit sync, mask 0xF0), so ADTS is tried first. Mask 0xF0 alone
  // is not enough though — a common MP3 header such as FF FB also satisfies it.
  // The tie-breaker is the layer field (byte 1, bits 2-1): ADTS always encodes
  // layer 00, while a valid MPEG audio frame never does. Testing 0xF6 checks
  // the 12-bit sync and that layer field in one go (matches F0, F1, F8, F9).
  if (bytes[0] === 0xff) {
    if ((bytes[1] & 0xf6) === 0xf0) return "aac";
    if ((bytes[1] & 0xe0) === 0xe0) return "mp3";
  }

  return null;
}

/**
 * Predicate for "this finished download is actually media".
 * @param {ArrayBuffer|Uint8Array|number[]|null|undefined} buffer
 * @returns {boolean}
 */
export function isPlausibleMedia(buffer) {
  return sniffMediaFormat(buffer) !== null && !looksLikeHtml(buffer);
}

/**
 * Some servers disguise media segments as images. Drop the PNG or JPEG wrapper
 * and return the payload that follows it. Anything else — including an image
 * signature whose end marker is missing — is returned untouched, by reference.
 * @param {ArrayBuffer|Uint8Array|number[]|null|undefined} buffer
 * @returns {Uint8Array|ArrayBuffer|number[]|null|undefined} the trimmed payload
 *   as a Uint8Array, or the original argument when there is nothing to strip.
 */
export function stripImageWrapper(buffer) {
  const bytes = toBytes(buffer);
  if (!bytes || bytes.length < MIN_SNIFF_BYTES) return buffer;

  let payloadStart = -1;

  if (matchesAt(bytes, PNG_SIGNATURE, 0)) {
    const iend = indexOfPattern(bytes, PNG_IEND, PNG_SIGNATURE.length);
    if (iend !== -1) payloadStart = iend + PNG_IEND_TRAILER;
  } else if (bytes[0] === 0xff && bytes[1] === 0xd8 && bytes[2] === 0xff) {
    for (let i = 2; i < bytes.length - 1; i += 1) {
      if (bytes[i] === 0xff && bytes[i + 1] === 0xd9) {
        payloadStart = i + JPEG_EOI_TRAILER;
        break;
      }
    }
  }

  // No image header, no end marker, or nothing left after it: keep the original.
  if (payloadStart === -1 || payloadStart >= bytes.length) return buffer;
  return bytes.slice(payloadStart);
}

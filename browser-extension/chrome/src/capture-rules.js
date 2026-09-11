// Portions adapted from cat-catch (js/background.js, js/init.js)
// Copyright (c) xifangczy — https://github.com/xifangczy/cat-catch
// Licensed under GPL-3.0, same as this project.
//
// User-editable capture rules: three tables (extensions, content types, regex)
// that will eventually replace the fixed detection constants in the sniffer.
// This module is pure logic only — persistence and UI live elsewhere.

import {
  MEDIA_CONTENT_TYPES,
  MEDIA_EXTENSIONS,
  extensionMatchesPath,
  pathnameOf,
} from "./sniffer-filters.js";

const UNIT_BYTES = Object.freeze({
  b: 1,
  byte: 1,
  bytes: 1,
  kb: 1024,
  mb: 1024 * 1024,
  gb: 1024 * 1024 * 1024,
});

// A range ("500-1000 MB") has no operator; anything else may carry one.
const RANGE_PATTERN = /^(\d+(?:\.\d+)?)\s*-\s*(\d+(?:\.\d+)?)\s*([a-z]+)?$/;
const COMPARISON_PATTERN = /^(>=|<=|!=|>|<|=)?\s*(\d+(?:\.\d+)?)\s*([a-z]+)?$/;

// Default size rule for every media extension/type, mirroring MIN_CONTENT_LENGTH
// in sniffer-filters.js. Manifests carry no size floor: they are tiny by nature.
const DEFAULT_SIZE_RULE = ">=50 KB";
// HLS manifests carry no size floor: they are tiny by nature. An MPD keeps the
// 1 KB floor shouldDropBySize already applied, which filters out the stub
// documents some CDNs answer with.
const DEFAULT_DASH_SIZE_RULE = ">=1 KB";
const UNRESTRICTED_EXTENSIONS = Object.freeze([".m3u8"]);
const UNRESTRICTED_CONTENT_TYPES = Object.freeze([
  "application/vnd.apple.mpegurl",
  "application/x-mpegurl",
]);
const DASH_EXTENSIONS = Object.freeze([".mpd"]);
const DASH_CONTENT_TYPES = Object.freeze(["application/dash+xml", "application/f4m+xml"]);

function defaultSizeFor(key, unrestricted, dash) {
  if (unrestricted.includes(key)) return null;
  if (dash.includes(key)) return DEFAULT_DASH_SIZE_RULE;
  return DEFAULT_SIZE_RULE;
}

/**
 * Resolve a unit suffix to a byte multiplier. Unknown units mean "bytes".
 * @param {string|undefined} unit
 * @returns {number|null} multiplier, or null when the unit is not recognised
 */
function unitMultiplier(unit) {
  if (unit === undefined || unit === null || unit === "") return 1;
  const key = String(unit).trim().toLowerCase();
  if (key === "") return 1;
  return Object.prototype.hasOwnProperty.call(UNIT_BYTES, key) ? UNIT_BYTES[key] : null;
}

/**
 * Parse a size rule typed by the user into a comparison descriptor.
 * Accepts ">100 KB", "<1 GB", "=500 KB", ">=2 MB", "<=2 MB", "!=0" and
 * ranges like "500-1000 MB". Units are case-insensitive and base 1024;
 * a bare number is read as bytes. Never throws — invalid input returns null.
 * @param {string|number|null|undefined} input
 * @returns {{operator: string, size: number}|{operator: "~", min: number, max: number}|null}
 */
export function parseSizeRule(input) {
  if (typeof input === "number") {
    if (!Number.isFinite(input) || input < 0) return null;
    return { operator: ">=", size: Math.round(input) };
  }
  if (typeof input !== "string") return null;

  const text = input.trim().toLowerCase();
  if (text === "") return null;

  const range = RANGE_PATTERN.exec(text);
  if (range) {
    const multiplier = unitMultiplier(range[3]);
    if (multiplier === null) return null;
    const a = Math.round(Number(range[1]) * multiplier);
    const b = Math.round(Number(range[2]) * multiplier);
    // The user may type the bounds the wrong way round.
    return { operator: "~", min: Math.min(a, b), max: Math.max(a, b) };
  }

  const comparison = COMPARISON_PATTERN.exec(text);
  if (!comparison) return null;
  const multiplier = unitMultiplier(comparison[3]);
  if (multiplier === null) return null;

  // No explicit operator means "at least this big", the cat-catch default.
  const operator = comparison[1] || ">=";
  return { operator, size: Math.round(Number(comparison[2]) * multiplier) };
}

/**
 * Apply a parsed size rule to a byte count.
 * A null rule means "no restriction". An unknown size (non-numeric or <= 0)
 * always passes: the sniffer must never drop a resource just because the
 * server did not send a Content-Length.
 * @param {number} bytes
 * @param {object|null} rule
 * @returns {boolean}
 */
export function matchesSize(bytes, rule) {
  if (!rule || typeof rule !== "object") return true;
  if (typeof bytes !== "number" || !Number.isFinite(bytes) || bytes <= 0) return true;

  if (rule.operator === "~") {
    const min = typeof rule.min === "number" ? rule.min : null;
    const max = typeof rule.max === "number" ? rule.max : null;
    if (min !== null && bytes < min) return false;
    if (max !== null && bytes > max) return false;
    return true;
  }

  const target = rule.size;
  if (typeof target !== "number" || !Number.isFinite(target)) return true;

  switch (rule.operator) {
    case "=": return bytes === target;
    case "!=": return bytes !== target;
    case "<": return bytes < target;
    case "<=": return bytes <= target;
    case ">": return bytes > target;
    case ">=": return bytes >= target;
    default: return true;
  }
}

/**
 * Pre-compile user regex rules. An invalid pattern disables that single rule
 * and records the error for the UI — it never throws and never affects the
 * other rules.
 * @param {Array<{pattern?: string, flags?: string, action?: string, ext?: string, enabled?: boolean}|string>} rules
 * @returns {Array<object>} compiled rules
 */
export function compileRegexRules(rules) {
  if (!Array.isArray(rules)) return [];

  const compiled = [];
  for (const entry of rules) {
    const raw = typeof entry === "string" ? { pattern: entry } : entry;
    if (!raw || typeof raw !== "object") continue;

    const pattern = typeof raw.pattern === "string" ? raw.pattern : "";
    const flags = typeof raw.flags === "string" ? raw.flags : "ig";
    const action = raw.action === "block" ? "block" : "accept";
    const ext = typeof raw.ext === "string" ? raw.ext.trim().toLowerCase() : "";
    const wanted = raw.enabled !== false;

    const rule = { pattern, flags, action, ext, enabled: wanted, regex: null, error: null };

    if (pattern === "") {
      rule.enabled = false;
      rule.error = "Empty pattern";
      compiled.push(rule);
      continue;
    }

    try {
      rule.regex = new RegExp(pattern, flags);
    } catch (err) {
      // Fall back to the default flags before giving up: a bad flag string is
      // a far more common typo than a bad pattern.
      try {
        rule.regex = new RegExp(pattern, "ig");
        rule.flags = "ig";
      } catch {
        rule.regex = null;
        rule.enabled = false;
        rule.error = err && err.message ? String(err.message) : "Invalid regular expression";
      }
    }

    compiled.push(rule);
  }

  return compiled;
}

/**
 * Decode a capture group, tolerating stray `%` that would break decodeURIComponent.
 * @param {string|undefined} value
 * @returns {string}
 */
function safeDecode(value) {
  if (typeof value !== "string") return "";
  try {
    return decodeURIComponent(value);
  } catch {
    return value;
  }
}

/**
 * Rebuild a URL out of the capture groups of a match. This is the heart of the
 * technique: a byte-range fragment URL such as
 * `...&bytestart=0&byteend=99999` captures only the part before `&bytestart`,
 * so the rebuilt URL points at the whole file instead of one slice.
 * With no capture groups the original URL is returned untouched.
 * @param {string} url
 * @param {RegExpExecArray} match
 * @returns {string}
 */
function rebuildUrl(url, match) {
  if (match.length <= 1) return url;

  const joined = match.slice(1).map(safeDecode).join("");
  if (joined === "") return url;
  if (/^https?:\/\//i.test(joined)) return joined;

  // A protocol-less rebuild (`//host/path` or `host/path`) borrows the
  // protocol of the request it came from.
  let protocol = "https:";
  try {
    protocol = new URL(url).protocol;
  } catch {
    protocol = "https:";
  }
  return joined.startsWith("//") ? `${protocol}${joined}` : `${protocol}//${joined.replace(/^\/+/, "")}`;
}

/**
 * Run a URL through the compiled regex rules. Block rules are evaluated first,
 * then accept rules; the first rule that matches wins.
 * @param {string} url
 * @param {Array<object>} compiled
 * @returns {{action: "block", rule: object}|{action: "accept", url: string, ext: string|undefined, rule: object}|null}
 */
export function applyRegexRules(url, compiled) {
  if (typeof url !== "string" || url === "" || !Array.isArray(compiled)) return null;

  const usable = compiled.filter(rule => rule && rule.enabled && rule.regex instanceof RegExp);

  for (const rule of usable) {
    if (rule.action !== "block") continue;
    rule.regex.lastIndex = 0;
    if (rule.regex.exec(url) !== null) {
      return { action: "block", rule };
    }
  }

  for (const rule of usable) {
    if (rule.action === "block") continue;
    rule.regex.lastIndex = 0;
    const match = rule.regex.exec(url);
    if (match === null) continue;
    return {
      action: "accept",
      url: rebuildUrl(url, match),
      ext: rule.ext ? rule.ext : undefined,
      rule,
    };
  }

  return null;
}

/**
 * Normalise whatever came out of storage into the canonical rule tables.
 * Accepts a grouped object, a flat array, plain strings, or junk; entries that
 * cannot be understood are dropped silently.
 * @param {any} raw
 * @returns {{extensions: Array<object>, contentTypes: Array<object>, regex: Array<object>}}
 */
export function normalizeUserRules(raw) {
  const out = { extensions: [], contentTypes: [], regex: [] };
  if (!raw || typeof raw !== "object") return out;

  if (Array.isArray(raw)) {
    for (const entry of raw) pushNormalized(out, entry);
    return out;
  }

  const buckets = [
    [raw.extensions ?? raw.ext ?? raw.Ext, "ext"],
    [raw.contentTypes ?? raw.types ?? raw.type ?? raw.Type, "type"],
    [raw.regex ?? raw.Regex, "regex"],
  ];
  for (const [list, kind] of buckets) {
    if (!Array.isArray(list)) continue;
    for (const entry of list) pushNormalized(out, entry, kind);
  }
  return out;
}

/**
 * Normalise one entry into the right bucket. `hint` forces a kind when the
 * caller already knows which table the entry came from.
 * @param {{extensions: Array, contentTypes: Array, regex: Array}} out
 * @param {any} entry
 * @param {string} [hint]
 */
function pushNormalized(out, entry, hint) {
  if (typeof entry === "string") {
    const text = entry.trim();
    if (text === "") return;
    const kind = hint || (text.includes("/") ? "type" : "ext");
    if (kind === "regex") {
      out.regex.push(normalizeRegexEntry({ pattern: text }));
      return;
    }
    if (kind === "type") {
      const type = normalizeContentType(text);
      if (type) out.contentTypes.push({ type, size: null, enabled: true });
      return;
    }
    const ext = normalizeExtension(text);
    if (ext) out.extensions.push({ ext, size: null, enabled: true });
    return;
  }

  if (!entry || typeof entry !== "object" || Array.isArray(entry)) return;

  const size = normalizeSizeText(entry.size);
  const enabled = entry.enabled !== false && entry.state !== false;

  // `pattern`/`regex` wins over the other fields: a regex rule may also carry
  // an `ext` used to label whatever it captures.
  const pattern = typeof entry.pattern === "string" ? entry.pattern
    : typeof entry.regex === "string" ? entry.regex
      : "";
  if ((hint === "regex" || !hint) && pattern.trim() !== "") {
    out.regex.push(normalizeRegexEntry({ ...entry, pattern }));
    return;
  }
  if (hint === "regex") return;

  if (hint !== "ext" && typeof entry.type === "string") {
    const type = normalizeContentType(entry.type);
    if (type) out.contentTypes.push({ type, size, enabled });
    return;
  }
  if (hint !== "type" && typeof entry.ext === "string") {
    const ext = normalizeExtension(entry.ext);
    if (ext) out.extensions.push({ ext, size, enabled });
  }
}

/**
 * @param {object} entry
 * @returns {{pattern: string, flags: string, action: string, ext: string, enabled: boolean}}
 */
function normalizeRegexEntry(entry) {
  const flags = typeof entry.flags === "string" && entry.flags !== "" ? entry.flags
    : typeof entry.type === "string" && /^[a-z]*$/.test(entry.type) ? entry.type
      : "ig";
  const action = entry.action === "block" || entry.blackList === true ? "block" : "accept";
  return {
    pattern: String(entry.pattern),
    flags,
    action,
    ext: typeof entry.ext === "string" ? entry.ext.trim().toLowerCase().replace(/^\./, "") : "",
    enabled: entry.enabled !== false && entry.state !== false,
  };
}

/**
 * @param {any} value
 * @returns {string|null} the size rule as text, or null for "no restriction"
 */
function normalizeSizeText(value) {
  if (typeof value === "number" && Number.isFinite(value) && value > 0) return String(Math.round(value));
  if (typeof value !== "string") return null;
  const text = value.trim();
  return text === "" ? null : text;
}

/**
 * @param {string} value
 * @returns {string|null} `.mp4`-shaped extension, or null when unusable
 */
function normalizeExtension(value) {
  const text = String(value).trim().toLowerCase().replace(/^\.+/, "");
  if (text === "" || !/^[a-z0-9][a-z0-9+._-]*$/.test(text)) return null;
  return `.${text}`;
}

/**
 * @param {string} value
 * @returns {string|null} `video/mp4`-shaped content type, or null when unusable
 */
function normalizeContentType(value) {
  const text = String(value).trim().toLowerCase();
  const parts = text.split("/");
  if (parts.length !== 2) return null;
  if (parts[0] === "" || parts[1] === "") return null;
  if (!/^[a-z0-9*][a-z0-9+.*_-]*$/.test(parts[0])) return null;
  if (!/^[a-z0-9*][a-z0-9+.*_-]*$/.test(parts[1])) return null;
  return text;
}

// The three cat-catch regex rules worth shipping by default. The Instagram and
// Facebook ones capture everything before `&bytestart=`, turning a byte-range
// fragment into the URL of the whole file. The bilibili one blocks the endless
// `live-bvc` segments of a live stream.
export const DEFAULT_REGEX_RULES = Object.freeze([
  Object.freeze({
    pattern: "(^https://scontent[a-z0-9-]*\\.cdninstagram\\.com/.*)&bytestart=.*",
    flags: "ig",
    action: "accept",
    ext: "",
    enabled: true,
  }),
  Object.freeze({
    pattern: "(^https://.*\\.fbcdn\\.net/.*)&bytestart=.*",
    flags: "ig",
    action: "accept",
    ext: "",
    enabled: true,
  }),
  Object.freeze({
    pattern: ".*\\.bilivideo\\.(com|cn).*\\/live-bvc\\/.*m4s",
    flags: "ig",
    action: "block",
    ext: "",
    enabled: true,
  }),
]);

export const DEFAULT_MEDIA_EXTENSIONS = Object.freeze(
  MEDIA_EXTENSIONS.map(ext => Object.freeze({
    ext,
    size: defaultSizeFor(ext, UNRESTRICTED_EXTENSIONS, DASH_EXTENSIONS),
    enabled: true,
  })),
);

export const DEFAULT_MEDIA_CONTENT_TYPES = Object.freeze(
  MEDIA_CONTENT_TYPES.map(type => Object.freeze({
    type,
    size: defaultSizeFor(type, UNRESTRICTED_CONTENT_TYPES, DASH_CONTENT_TYPES),
    enabled: true,
  })),
);

/**
 * Find the extension rule whose extension sits at a path boundary of `url`.
 * Longest extension wins, so `.m3u8` beats a hypothetical `.m3u`.
 * @param {string} url
 * @param {Array<{ext: string, size: string|null, enabled?: boolean}>} extensions
 * @returns {object|null}
 */
export function findExtensionRule(url, extensions) {
  if (!Array.isArray(extensions)) return null;
  const path = pathnameOf(url);
  if (path === null) return null;
  let best = null;
  for (const rule of extensions) {
    const ext = typeof rule?.ext === "string" ? rule.ext.toLowerCase() : "";
    if (!extensionMatchesPath(path, ext)) continue;
    if (!best || ext.length > best.ext.length) best = rule;
  }
  return best;
}

/**
 * Find the content-type rule matching a response's Content-Type header.
 * @param {string} contentType
 * @param {Array<{type: string, size: string|null, enabled?: boolean}>} contentTypes
 * @returns {object|null}
 */
export function findContentTypeRule(contentType, contentTypes) {
  if (!Array.isArray(contentTypes) || typeof contentType !== "string" || contentType === "") {
    return null;
  }
  const lower = contentType.toLowerCase();
  let best = null;
  for (const rule of contentTypes) {
    const type = typeof rule?.type === "string" ? rule.type.toLowerCase() : "";
    if (type === "" || !lower.includes(type)) continue;
    if (!best || type.length > best.type.length) best = rule;
  }
  return best;
}

/**
 * Decide whether a response should be captured, using the user's tables.
 * The extension is consulted before the content type: a CDN that answers
 * `application/octet-stream` for an `.mp4` should still be judged as an mp4.
 * @param {{url: string, contentType: string, contentLength: number,
 *          extensions: Array<object>, contentTypes: Array<object>}} input
 * @returns {{capture: boolean, reason: string|null, rule: object|null}}
 */
export function evaluateCapture({ url, contentType, contentLength, extensions, contentTypes }) {
  const rule = findExtensionRule(url, extensions) || findContentTypeRule(contentType, contentTypes);
  if (!rule) return { capture: false, reason: "no-match", rule: null };
  // An unticked row means "never capture this kind", the equivalent of the
  // `break` in cat-catch's CheckExtension.
  if (rule.enabled === false) return { capture: false, reason: "disabled", rule };
  if (!matchesSize(contentLength, parseSizeRule(rule.size))) {
    return { capture: false, reason: "size", rule };
  }
  return { capture: true, reason: null, rule };
}

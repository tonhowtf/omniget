// Portions adapted from cat-catch (js/m3u8.js, estimateFileInfo)
// Copyright (c) xifangczy — https://github.com/xifangczy/cat-catch
// Licensed under GPL-3.0, same as this project.

// An m3u8 has no Content-Length worth showing: the manifest is a few KB while
// the media behind it is hundreds of MB. Today those rows show no size at all.
// Sampling a handful of segments and multiplying by the segment count gets
// close enough to be useful, and costs a few HEAD requests.
export const DEFAULT_SAMPLE_COUNT = 10;

export function resolveAgainst(baseUrl, uri) {
  try {
    return new URL(uri, baseUrl).href;
  } catch {
    return null;
  }
}

// Segment lines are the non-empty lines that don't start with '#'. A master
// playlist's lines point at other playlists rather than at media, so callers
// must resolve the variant first — isMediaPlaylist() says which one this is.
export function parseSegmentUris(manifestText, baseUrl) {
  if (typeof manifestText !== "string" || manifestText === "") return [];
  const out = [];
  for (const rawLine of manifestText.split(/\r?\n/)) {
    const line = rawLine.trim();
    if (line === "" || line.startsWith("#")) continue;
    const resolved = baseUrl ? resolveAgainst(baseUrl, line) : line;
    if (resolved) out.push(resolved);
  }
  return out;
}

export function isMediaPlaylist(manifestText) {
  if (typeof manifestText !== "string") return false;
  return manifestText.includes("#EXTINF");
}

// Averages the samples we actually got rather than assuming every HEAD
// succeeded — a CDN that refuses HEAD on some segments shouldn't skew the
// estimate to zero. Returns null when nothing usable came back, so the caller
// can keep showing no size instead of showing a wrong one.
export function estimateTotalBytes(sampleSizes, segmentCount) {
  const usable = (Array.isArray(sampleSizes) ? sampleSizes : [])
    .map(Number)
    .filter(n => Number.isFinite(n) && n > 0);
  if (usable.length === 0) return null;
  if (!Number.isFinite(segmentCount) || segmentCount <= 0) return null;
  const average = usable.reduce((sum, n) => sum + n, 0) / usable.length;
  return Math.round(average * segmentCount);
}

async function headContentLength(url, fetchImpl, referer) {
  try {
    const headers = referer ? { Referer: referer } : undefined;
    const response = await fetchImpl(url, { method: "HEAD", headers });
    if (!response?.ok) return null;
    const value = response.headers?.get?.("content-length");
    const parsed = parseInt(value, 10);
    return Number.isFinite(parsed) && parsed > 0 ? parsed : null;
  } catch {
    return null;
  }
}

// Fetches the playlist, HEADs the first `sampleCount` segments in parallel and
// multiplies. Never throws and never rejects: an estimate is a nicety, and a
// CDN that blocks us must not break rendering the popup.
export async function estimateHlsSize(manifestUrl, options = {}) {
  const {
    fetchImpl = typeof fetch === "function" ? fetch : null,
    sampleCount = DEFAULT_SAMPLE_COUNT,
    referer = "",
  } = options;
  if (!fetchImpl || !manifestUrl) return null;

  let text;
  try {
    const response = await fetchImpl(manifestUrl, {
      headers: referer ? { Referer: referer } : undefined,
    });
    if (!response?.ok) return null;
    text = await response.text();
  } catch {
    return null;
  }

  if (!isMediaPlaylist(text)) return null;

  const segments = parseSegmentUris(text, manifestUrl);
  if (segments.length === 0) return null;

  const samples = await Promise.all(
    segments.slice(0, sampleCount).map(url => headContentLength(url, fetchImpl, referer))
  );

  return estimateTotalBytes(samples, segments.length);
}

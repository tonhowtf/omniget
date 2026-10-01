// Copyright (c) xifangczy — https://github.com/xifangczy/cat-catch
// Licensed under GPL-3.0, same as this project.

export const DEFAULT_SAMPLE_COUNT = 10;

export function resolveAgainst(baseUrl, uri) {
  try {
    return new URL(uri, baseUrl).href;
  } catch {
    return null;
  }
}

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

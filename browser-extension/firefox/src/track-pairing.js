// Portions adapted from cat-catch (js/background.js)
// Copyright (c) xifangczy — https://github.com/xifangczy/cat-catch
// Licensed under GPL-3.0, same as this project.

// Adaptive players request the video track and its audio track back to back, so
// the two show up in the sniffer milliseconds apart with the same page as the
// referer. Presenting them as two unrelated files is how a user ends up with a
// silent video and no idea why. cat-catch pairs neighbours inside a 120 ms
// window; we use a slightly wider one because our detection happens on
// onHeadersReceived rather than on the request itself.
export const DEFAULT_PAIR_WINDOW_MS = 200;

function isPairableType(mediaType) {
  return mediaType === "video" || mediaType === "audio";
}

function timeOf(entry) {
  const value = Number(entry?.detectedAt);
  return Number.isFinite(value) ? value : 0;
}

// Returns { pairs, singles }. `pairs` holds { video, audio, detectedAt } for
// tracks that arrived close together with complementary types; `singles` holds
// everything else, in the original order. An entry is never in both.
export function pairTracks(media, windowMs = DEFAULT_PAIR_WINDOW_MS) {
  const list = Array.isArray(media) ? media.filter(Boolean) : [];
  if (list.length < 2) return { pairs: [], singles: list.slice() };

  const candidates = list
    .filter(m => isPairableType(m.mediaType))
    .sort((a, b) => timeOf(a) - timeOf(b));

  const paired = new Set();
  const pairs = [];

  for (let i = 0; i < candidates.length - 1; i++) {
    const current = candidates[i];
    if (paired.has(current)) continue;
    for (let j = i + 1; j < candidates.length; j++) {
      const next = candidates[j];
      if (paired.has(next)) continue;
      if (timeOf(next) - timeOf(current) > windowMs) break;
      if (next.mediaType === current.mediaType) continue;
      const video = current.mediaType === "video" ? current : next;
      const audio = current.mediaType === "audio" ? current : next;
      paired.add(current);
      paired.add(next);
      pairs.push({ video, audio, detectedAt: Math.max(timeOf(video), timeOf(audio)) });
      break;
    }
  }

  return { pairs, singles: list.filter(m => !paired.has(m)) };
}

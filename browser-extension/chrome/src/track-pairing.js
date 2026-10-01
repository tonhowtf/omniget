// Copyright (c) xifangczy — https://github.com/xifangczy/cat-catch
// Licensed under GPL-3.0, same as this project.

export const DEFAULT_PAIR_WINDOW_MS = 200;

function isPairableType(mediaType) {
  return mediaType === "video" || mediaType === "audio";
}

function timeOf(entry) {
  const value = Number(entry?.detectedAt);
  return Number.isFinite(value) ? value : 0;
}

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

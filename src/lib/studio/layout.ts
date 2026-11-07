import type { Segment, Word } from "./types";

export type Cue = { start_ms: number; end_ms: number; lines: Word[][] };

export type LayoutOptions = { maxChars: number; maxLines: 1 | 2; uppercase: boolean };

export function lineLength(words: Word[]): number {
  return words.reduce((n, w, i) => n + w.text.length + (i > 0 ? 1 : 0), 0);
}

export function greedyLines(words: Word[], maxChars: number): Word[][] {
  const limit = Math.max(1, Math.floor(maxChars));
  const lines: Word[][] = [];
  let current: Word[] = [];
  for (const word of words) {
    const next = lineLength(current) + (current.length ? 1 : 0) + word.text.length;
    if (current.length && next > limit) {
      lines.push(current);
      current = [];
    }
    current.push(word);
  }
  if (current.length) lines.push(current);
  return lines;
}

export function balance(words: Word[], maxChars: number): Word[][] {
  if (words.length < 2) return [words];
  let best: Word[][] | null = null;
  let bestScore = Infinity;
  for (let i = 1; i < words.length; i++) {
    const a = words.slice(0, i);
    const b = words.slice(i);
    const la = lineLength(a);
    const lb = lineLength(b);
    const over = Math.max(0, la - maxChars) + Math.max(0, lb - maxChars);
    const score = over * 1000 + Math.max(la, lb) + (lb < la ? 0.5 : 0);
    if (score < bestScore) {
      bestScore = score;
      best = [a, b];
    }
  }
  return best ?? [words];
}

function transform(words: Word[], uppercase: boolean): Word[] {
  return words
    .map((w) => ({ ...w, text: w.text.replace(/[\r\n]+/g, " ").trim() }))
    .filter((w) => w.text)
    .map((w) => (uppercase ? { ...w, text: w.text.toLocaleUpperCase() } : w));
}

export function layoutCues(segments: Segment[], opts: LayoutOptions): Cue[] {
  const maxChars = Math.max(4, Math.floor(opts.maxChars));
  const cues: Cue[] = [];
  for (const seg of segments) {
    const words = transform(seg.words, opts.uppercase);
    if (!words.length) continue;
    const lines = greedyLines(words, maxChars);
    const groups: Word[][][] = [];
    for (let i = 0; i < lines.length; i += opts.maxLines) {
      const chunk = lines.slice(i, i + opts.maxLines);
      groups.push(chunk.length > 1 ? balance(chunk.flat(), maxChars) : chunk);
    }
    groups.forEach((group, gi) => {
      const first = group[0][0];
      const start = gi === 0 ? seg.start_ms : first.start_ms;
      const nextGroup = groups[gi + 1];
      const end = nextGroup ? nextGroup[0][0].start_ms : seg.end_ms;
      cues.push({ start_ms: start, end_ms: Math.max(end, start + 1), lines: group });
    });
  }
  return cues.sort((a, b) => a.start_ms - b.start_ms);
}

export function activeCue(cues: Cue[], t: number): Cue | null {
  let found: Cue | null = null;
  for (const cue of cues) {
    if (cue.start_ms > t) break;
    if (t < cue.end_ms) found = cue;
  }
  return found;
}

export function activeSegmentIndex(segments: Segment[], t: number): number {
  return segments.findIndex((s) => s.start_ms <= t && t < s.end_ms);
}

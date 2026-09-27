// LRC writing on the frontend (the parser lives in Rust and comes back with
// every `music_lyrics_*` call). Used by the offset fix and the tap-to-sync
// editor, which produce new timings.

import type { LrcLine } from "./types";

function stamp(ms: number): string {
  const v = Math.max(0, Math.round(ms));
  const m = Math.floor(v / 60000);
  const s = Math.floor((v % 60000) / 1000);
  const cs = Math.floor((v % 1000) / 10);
  return `${String(m).padStart(2, "0")}:${String(s).padStart(2, "0")}.${String(cs).padStart(2, "0")}`;
}

export function toLrc(lines: LrcLine[], meta: { title?: string; artist?: string; album?: string } = {}, shiftMs = 0): string {
  const head = [
    meta.title ? `[ti:${meta.title}]` : "",
    meta.artist ? `[ar:${meta.artist}]` : "",
    meta.album ? `[al:${meta.album}]` : "",
  ].filter(Boolean);
  const body = lines.map((l) => {
    const words = l.words?.length
      ? l.words.map((w) => `<${stamp(w.time_ms + shiftMs)}>${w.text}`).join("")
      : l.text;
    return `[${stamp(l.time_ms + shiftMs)}]${words}`;
  });
  return [...head, ...body].join("\n") + "\n";
}

/** Plain text -> lines to be stamped one by one. */
export function plainLines(text: string): string[] {
  return text
    .split(/\r?\n/)
    .map((l) => l.replace(/^\[[^\]]*\]/g, "").trim())
    .filter((l, i, arr) => l !== "" || (i > 0 && arr[i - 1] !== ""));
}

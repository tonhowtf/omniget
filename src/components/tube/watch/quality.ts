// Quality list built from a StreamSet: one entry per height (the best format
// of each), the same list the player's quality menu shows, plus audio-only.
import type { Format, StreamSet } from "$lib/tube/types";

type VideoOption = {
  key: string;
  label: string;
  height: number;
  fps: number;
  hdr: boolean;
  badge: string;
  itag: number;
  /** Audio to mux with it (null for muxed formats). */
  audioItag: number | null;
  bytes: number | null;
  codec: string;
};

export type AudioOption = {
  key: string;
  itag: number;
  codec: string;
  kbps: number;
  lang: string | null;
  name: string | null;
  original: boolean;
  bytes: number | null;
};

function family(codecs: string): string {
  const c = codecs.toLowerCase();
  if (c.startsWith("avc1")) return "avc1";
  if (c.startsWith("vp9") || c.startsWith("vp09")) return "vp9";
  if (c.startsWith("av01")) return "av01";
  if (c.startsWith("mp4a")) return "mp4a";
  if (c.startsWith("opus")) return "opus";
  return c.split(".")[0] || "?";
}

/** Original (not auto-dubbed) track first, then bitrate. */
export function bestAudio(set: StreamSet): Format | null {
  const audio = set.formats.filter((f) => f.kind === "audio");
  audio.sort((a, b) => {
    const oa = a.audio_default && !a.auto_dubbed ? 1 : 0;
    const ob = b.audio_default && !b.auto_dubbed ? 1 : 0;
    return ob - oa || b.bitrate - a.bitrate;
  });
  return audio[0] ?? null;
}

export function videoLadder(set: StreamSet, prefer: string = "auto"): VideoOption[] {
  const order = prefer !== "auto" ? [prefer, "vp9", "av01", "avc1"] : ["vp9", "av01", "avc1"];
  const rank = (f: Format) => {
    const i = order.indexOf(family(f.codecs));
    return i < 0 ? order.length : i;
  };
  const audio = bestAudio(set);
  const byKey = new Map<string, Format>();
  for (const f of set.formats) {
    if ((f.kind !== "video" && f.kind !== "muxed") || !f.height) continue;
    const fps = f.fps && f.fps > 30 ? f.fps : 30;
    const key = `${f.height}-${fps}-${f.hdr ? 1 : 0}`;
    const cur = byKey.get(key);
    if (
      !cur ||
      (cur.kind === "muxed" && f.kind === "video") ||
      (cur.kind === f.kind && (rank(f) < rank(cur) || (rank(f) === rank(cur) && f.bitrate > cur.bitrate)))
    ) {
      byKey.set(key, f);
    }
  }
  const out: VideoOption[] = [];
  for (const [key, f] of byKey) {
    const h = f.height ?? 0;
    const fps = f.fps && f.fps > 30 ? Math.round(f.fps) : 0;
    const muxAudio = f.kind === "video" ? audio : null;
    const vBytes = f.content_length ?? null;
    const aBytes = muxAudio?.content_length ?? 0;
    out.push({
      key,
      label: `${h}p${fps ? fps : ""}`,
      height: h,
      fps,
      hdr: f.hdr,
      badge: h >= 4320 ? "8K" : h >= 2160 ? "4K" : h >= 720 ? "HD" : "",
      itag: f.itag,
      audioItag: muxAudio?.itag ?? null,
      bytes: vBytes != null ? vBytes + aBytes : null,
      codec: family(f.codecs),
    });
  }
  out.sort((a, b) => b.height - a.height || b.fps - a.fps || Number(b.hdr) - Number(a.hdr));
  return out;
}

export function audioOptions(set: StreamSet): AudioOption[] {
  const best = new Map<string, Format>();
  for (const f of set.formats) {
    if (f.kind !== "audio") continue;
    const key = `${f.audio_lang ?? ""}-${family(f.codecs)}${f.drc ? "-drc" : ""}`;
    const cur = best.get(key);
    if (!cur || f.bitrate > cur.bitrate) best.set(key, f);
  }
  const out = [...best.entries()].map(([key, f]) => ({
    key,
    itag: f.itag,
    codec: family(f.codecs),
    kbps: Math.round(f.bitrate / 1000),
    lang: f.audio_lang ?? null,
    name: f.audio_name ?? null,
    original: f.audio_default && !f.auto_dubbed,
    bytes: f.content_length ?? null,
  }));
  out.sort((a, b) => Number(b.original) - Number(a.original) || (a.name ?? "").localeCompare(b.name ?? "") || b.kbps - a.kbps);
  return out;
}

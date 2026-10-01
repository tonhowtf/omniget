import type { Format, SbCategory, SbMode, SbSegment, Storyboard, StreamSet } from "./types";

type CodecFamily = "avc1" | "vp9" | "av01" | "mp4a" | "opus" | "other";

function codecFamily(f: Pick<Format, "codecs" | "mime">): CodecFamily {
  const c = (f.codecs || "").toLowerCase();
  if (c.startsWith("avc1") || c.startsWith("avc3")) return "avc1";
  if (c.startsWith("vp9") || c.startsWith("vp09")) return "vp9";
  if (c.startsWith("av01")) return "av01";
  if (c.startsWith("mp4a")) return "mp4a";
  if (c.startsWith("opus")) return "opus";
  return "other";
}

export function mimeCodecs(f: Pick<Format, "codecs" | "mime">): string {
  return f.codecs ? `${f.mime}; codecs="${f.codecs}"` : f.mime;
}

export function isWebKit(ua = typeof navigator !== "undefined" ? navigator.userAgent : ""): boolean {
  return /AppleWebKit/.test(ua) && !/Chrome|Chromium|Edg\//.test(ua);
}

type Supports = (f: Format) => boolean;

export const mseSupports: Supports = (f) => {
  try {
    const MS = (globalThis as { MediaSource?: typeof MediaSource }).MediaSource;
    return !!MS && MS.isTypeSupported(mimeCodecs(f));
  } catch {
    return false;
  }
};

export function hasRanges(f: Format): boolean {
  return !!f.init_range && !!f.index_range;
}

type LadderPrefs = {
  codec: "auto" | "avc1" | "vp9" | "av01" | string;
  webkit: boolean;
  needRanges?: boolean;
  supports?: Supports;
};

function familyOrder(p: LadderPrefs): CodecFamily[] {
  const base: CodecFamily[] = p.webkit ? ["avc1", "vp9", "av01"] : ["vp9", "avc1", "av01"];
  if (p.codec === "avc1" || p.codec === "vp9" || p.codec === "av01") {
    return [p.codec, ...base.filter((c) => c !== p.codec)];
  }
  return base;
}

export function videoLadder(formats: Format[], p: LadderPrefs): Format[] {
  const vids = formats.filter((f) => f.kind === "video" && (f.height ?? 0) > 0 && (!p.needRanges || hasRanges(f)));
  for (const fam of familyOrder(p)) {
    const of = vids.filter((f) => codecFamily(f) === fam && (!p.supports || p.supports(f)));
    if (!of.length) continue;
    const byH = new Map<number, Format>();
    for (const f of of) {
      const h = f.height ?? 0;
      const cur = byH.get(h);
      if (!cur || better(f, cur)) byH.set(h, f);
    }
    return [...byH.values()].sort((a, b) => (b.height ?? 0) - (a.height ?? 0));
  }
  return [];
}

function better(a: Format, b: Format): boolean {
  const fa = a.fps ?? 30;
  const fb = b.fps ?? 30;
  if (fa !== fb) return fa > fb;
  if (a.hdr !== b.hdr) return !a.hdr;
  return a.bitrate > b.bitrate;
}

export function qualityLabel(f: Pick<Format, "height" | "fps" | "hdr">): string {
  const h = f.height ?? 0;
  const fps = f.fps && f.fps > 30 ? String(Math.round(f.fps)) : "";
  return `${h}p${fps}${f.hdr ? " HDR" : ""}`;
}

export function qualityBadge(h: number | null | undefined): string {
  const v = h ?? 0;
  if (v >= 4320) return "8K";
  if (v >= 2160) return "4K";
  if (v >= 1440) return "QHD";
  if (v >= 720) return "HD";
  return "";
}

export function chooseQuality(ladder: Format[], pref: string | number, boxHeight: number, dpr = 1): Format | null {
  if (!ladder.length) return null;
  const asc = [...ladder].sort((a, b) => (a.height ?? 0) - (b.height ?? 0));
  const want = pref === "auto" ? null : Number(pref);
  if (want && Number.isFinite(want)) {
    const fit = asc.filter((f) => (f.height ?? 0) <= want);
    return fit.length ? fit[fit.length - 1] : asc[0];
  }
  const target = Math.min(1080, Math.max(360, Math.round(boxHeight * Math.min(dpr, 2) * 0.9)));
  return asc.find((f) => (f.height ?? 0) >= target) ?? asc.filter((f) => (f.height ?? 0) <= 1080).pop() ?? asc[0];
}

export type AudioTrack = { key: string; lang: string | null; name: string | null; original: boolean };

function trackKey(f: Format): string {
  return `${f.audio_lang ?? ""}|${f.auto_dubbed ? 1 : 0}`;
}

export function audioTracks(formats: Format[]): AudioTrack[] {
  const out = new Map<string, AudioTrack>();
  for (const f of formats) {
    if (f.kind !== "audio") continue;
    const k = trackKey(f);
    const t = out.get(k);
    const original = f.audio_default && !f.auto_dubbed;
    if (!t) out.set(k, { key: k, lang: f.audio_lang ?? null, name: f.audio_name ?? null, original });
    else if (original) t.original = true;
  }
  return [...out.values()].sort((a, b) => Number(b.original) - Number(a.original) || (a.name ?? "").localeCompare(b.name ?? ""));
}

type AudioPrefs = {
  quality: "high" | "normal" | "low" | string;
  webkit: boolean;
  track?: string | null;
  needRanges?: boolean;
  supports?: Supports;
};

export function pickAudio(formats: Format[], p: AudioPrefs): Format | null {
  let aud = formats.filter((f) => f.kind === "audio" && (!p.needRanges || hasRanges(f)) && (!p.supports || p.supports(f)));
  if (!aud.length) return null;
  const inTrack = p.track ? aud.filter((f) => trackKey(f) === p.track) : [];
  if (inTrack.length) aud = inTrack;
  else {
    const orig = aud.filter((f) => f.audio_default && !f.auto_dubbed);
    if (orig.length) aud = orig;
    else {
      const def = aud.filter((f) => f.audio_default);
      if (def.length) aud = def;
    }
  }
  const nodrc = aud.filter((f) => !f.drc);
  if (nodrc.length) aud = nodrc;
  const fams: CodecFamily[] = p.webkit ? ["mp4a", "opus"] : ["opus", "mp4a"];
  for (const fam of fams) {
    const of = aud.filter((f) => codecFamily(f) === fam);
    if (of.length) return byBitrate(of, p.quality);
  }
  return byBitrate(aud, p.quality);
}

function byBitrate(list: Format[], q: string): Format {
  const s = [...list].sort((a, b) => a.bitrate - b.bitrate);
  if (q === "low") return s[0];
  if (q === "normal") {
    return s.reduce((best, f) => (Math.abs(f.bitrate - 128_000) < Math.abs(best.bitrate - 128_000) ? f : best), s[0]);
  }
  return s[s.length - 1];
}

export function pickMuxed(formats: Format[]): Format | null {
  const m = formats.filter((f) => f.kind === "muxed").sort((a, b) => (b.height ?? 0) - (a.height ?? 0));
  return m[0] ?? null;
}

export function pickDirectVideo(formats: Format[], p: LadderPrefs, height: number): Format | null {
  const lad = videoLadder(formats, { ...p, needRanges: false, supports: p.supports });
  if (!lad.length) return null;
  const fit = lad.filter((f) => (f.height ?? 0) <= height);
  return fit[0] ?? lad[lad.length - 1];
}

function withKey(url: string, token: string): string {
  if (/[?&]k=/.test(url)) return url;
  return `${url}${url.includes("?") ? "&" : "?"}k=${encodeURIComponent(token)}`;
}

export function absUrl(set: Pick<StreamSet, "base" | "token">, url: string): string {
  const origin = serverOrigin(set.base);
  const abs = url.startsWith("/") ? `${origin}${url}` : url;
  return origin && abs.startsWith(origin) ? withKey(abs, set.token) : abs;
}

function serverOrigin(base: string): string {
  try {
    return new URL(base).origin;
  } catch {
    return base.replace(/\/yt\/.*$/, "").replace(/\/$/, "");
  }
}

export function mediaUrl(set: Pick<StreamSet, "base" | "token" | "session">, itag: number): string {
  return absUrl(set, `/yt/${set.session}/${itag}`);
}

export function manifestUrl(set: Pick<StreamSet, "base" | "token" | "session">, video: number[], audio: number[]): string {
  return absUrl(set, `/yt/${set.session}/manifest.mpd?v=${video.join(",")}&a=${audio.join(",")}`);
}

export function hlsUrl(set: Pick<StreamSet, "base" | "token" | "session" | "hls">): string | null {
  if (!set.hls) return null;
  return set.hls.startsWith("/") || set.hls.startsWith(set.base) ? absUrl(set, set.hls) : absUrl(set, `/yt/${set.session}/hls.m3u8`);
}

// Port of Flow Desktop's externalAudioSync.ts (GPL-3.0): the rules that keep

const AUDIO_DRIFT_TOLERANCE_S = 0.5;
const AUDIO_RESYNC_MIN_INTERVAL_MS = 1200;
const AUDIO_HOLD_RELEASE_DRIFT_S = 0.12;

type SyncAction = "steady" | "hold" | "resume" | "realign" | "wait";

type SyncInput = {
  drift: number;
  held: boolean;
  videoAdvancing: boolean;
  videoStarved?: boolean;
  audioSeeking: boolean;
  audioReadyState: number;
  msSinceLastRealign: number;
};

export function decideAudioSync(i: SyncInput): SyncAction {
  if (i.held) {
    if (i.videoStarved) return "wait";
    return i.drift <= AUDIO_HOLD_RELEASE_DRIFT_S ? "resume" : "wait";
  }
  if (Math.abs(i.drift) <= AUDIO_DRIFT_TOLERANCE_S) return "steady";
  if (i.drift > 0 && (!i.videoAdvancing || i.videoStarved)) return "hold";
  if (i.msSinceLastRealign < AUDIO_RESYNC_MIN_INTERVAL_MS) return "wait";
  if (i.audioSeeking || i.audioReadyState < 2) return "wait";
  return "realign";
}

// WHY: no rate trim on WebKitGTK: each playbackRate write glitches there.
export function trimRate(rate: number, drift: number): number {
  if (Math.abs(drift) <= 0.08) return rate;
  return Math.min(rate + 0.05, Math.max(rate - 0.05, rate - drift * 0.12));
}

type SbFrame = { url: string; x: number; y: number; w: number; h: number; sheetW: number; sheetH: number };

export function storyboardFrame(sb: Storyboard, t: number, duration: number, boxW = 160): SbFrame | null {
  if (!sb.count || !sb.cols || !sb.rows || !sb.width) return null;
  const interval = sb.interval_ms > 0 ? sb.interval_ms : duration > 0 ? (duration * 1000) / sb.count : 0;
  if (interval <= 0) return null;
  const idx = Math.min(sb.count - 1, Math.max(0, Math.floor((t * 1000) / interval)));
  const per = sb.cols * sb.rows;
  const sheet = Math.floor(idx / per);
  const pos = idx % per;
  const scale = boxW / sb.width;
  const w = Math.round(sb.width * scale);
  const h = Math.round(sb.height * scale);
  return {
    url: sb.url.replace("$M", String(sheet)).replace("{n}", String(sheet)),
    x: (pos % sb.cols) * w,
    y: Math.floor(pos / sb.cols) * h,
    w,
    h,
    sheetW: sb.cols * w,
    sheetH: sb.rows * h,
  };
}

type SbStep = {
  skip: SbSegment | null;
  ask: SbSegment | null;
  mute: boolean;
};

function sbInside(s: SbSegment, t: number): boolean {
  return t >= s.start_s && t < s.end_s - 0.05;
}

export function sbStep(
  segs: SbSegment[],
  t: number,
  modeOf: (c: SbCategory) => SbMode,
  auto: boolean,
  dontSkip: ReadonlySet<string>,
  dismissed: ReadonlySet<string>,
): SbStep {
  const out: SbStep = { skip: null, ask: null, mute: false };
  for (const s of segs) {
    if (s.action !== "skip" && s.action !== "mute") continue;
    if (!sbInside(s, t)) continue;
    const mode = modeOf(s.category);
    if (mode === "off" || mode === "show") continue;
    if (s.action === "mute") {
      if (mode === "skip" && auto) out.mute = true;
      continue;
    }
    if (mode === "skip" && auto && !dontSkip.has(s.uuid)) {
      if (!out.skip || s.end_s > out.skip.end_s) out.skip = s;
    } else if (!dismissed.has(s.uuid) && !dontSkip.has(s.uuid)) {
      if (!out.ask || s.end_s > out.ask.end_s) out.ask = s;
    }
  }
  return out;
}

export function sbForget(segs: SbSegment[], t: number, marks: Set<string>): void {
  for (const s of segs) if (marks.has(s.uuid) && !sbInside(s, t)) marks.delete(s.uuid);
}

export type Cue = { start: number; end: number; text: string };

function vttTime(s: string): number {
  const p = s.trim().split(":");
  let sec = 0;
  for (const x of p) sec = sec * 60 + Number(x.replace(",", "."));
  return sec;
}

export function parseVtt(text: string): Cue[] {
  const cues: Cue[] = [];
  const blocks = text.replace(/\r/g, "").split(/\n\n+/);
  for (const b of blocks) {
    const lines = b.split("\n");
    const i = lines.findIndex((l) => l.includes("-->"));
    if (i < 0) continue;
    const [a, rest] = lines[i].split("-->");
    const end = rest.trim().split(/\s+/)[0];
    const body = lines
      .slice(i + 1)
      .join("\n")
      .replace(/<[^>]+>/g, "")
      .replace(/&amp;/g, "&")
      .replace(/&lt;/g, "<")
      .replace(/&gt;/g, ">")
      .replace(/&nbsp;/g, " ")
      .trim();
    if (!body) continue;
    const c = { start: vttTime(a), end: vttTime(end), text: body };
    const prev = cues[cues.length - 1];
    if (prev && c.text.startsWith(prev.text + "\n") && c.start <= prev.end + 0.05) {
      c.text = c.text.slice(prev.text.length + 1);
    }
    cues.push(c);
  }
  return cues;
}

export function cuesAt(cues: Cue[], t: number): Cue[] {
  let lo = 0;
  let hi = cues.length;
  while (lo < hi) {
    const m = (lo + hi) >> 1;
    if (cues[m].start <= t) lo = m + 1;
    else hi = m;
  }
  const out: Cue[] = [];
  for (let i = lo - 1; i >= 0 && i >= lo - 8; i--) if (cues[i].end > t) out.unshift(cues[i]);
  return out;
}

export function loudnessGain(db: number | null | undefined): number {
  if (db == null || !Number.isFinite(db) || db <= 0) return 1;
  return Math.max(0.1, Math.pow(10, -db / 20));
}

export type DashHandle = {
  destroy(): void;
  setVideo(itag: number, height: number | null): void;
  currentHeight(): number | null;
};

export async function startDash(video: HTMLVideoElement, url: string, startAt: number, onFatal: (why: string) => void, onReady?: () => void): Promise<DashHandle> {
  const dashjs = await import("dashjs");
  const player = dashjs.MediaPlayer().create();
  player.updateSettings({
    streaming: {
      capabilities: { useMediaCapabilitiesApi: false },
      abr: { autoSwitchBitrate: { video: false, audio: false } },
      buffer: {
        fastSwitchEnabled: false,
        bufferToKeep: 60,
        bufferTimeDefault: 12,
        bufferTimeAtTopQuality: 30,
        bufferTimeAtTopQualityLongForm: 60,
        longFormContentDurationThreshold: 600,
      },
    },
  });
  let fatal = false;
  const fail = (why: string) => {
    if (fatal) return;
    fatal = true;
    onFatal(why);
  };
  const ev = dashjs.MediaPlayer.events;
  player.on(ev.ERROR, (e: unknown) => {
    const err = (e as { error?: { code?: number; message?: string } })?.error;
    const code = Number(err?.code);
    if ([3, 17, 25, 27, 28, 34].includes(code) || /decode|manifest|segmentbase/i.test(err?.message ?? "")) fail(`dash:${code}`);
  });
  player.on(ev.PLAYBACK_ERROR, () => fail("dash:playback"));
  player.on(ev.ADAPTATION_SET_REMOVED_NO_CAPABILITIES, () => fail("dash:capabilities"));
  if (onReady) player.on(ev.STREAM_INITIALIZED, onReady);
  player.initialize(video, url, false, startAt > 0 ? startAt : 0);
  return {
    destroy() {
      try {
        player.destroy();
      } catch {
      }
    },
    setVideo(itag, height) {
      try {
        const reps = player.getRepresentationsByType("video") ?? [];
        const rep =
          reps.find((r) => r.id === String(itag)) ??
          [...reps].sort((a, b) => Math.abs((a.height || 0) - (height || 0)) - Math.abs((b.height || 0) - (height || 0)))[0];
        if (rep) player.setRepresentationForTypeById("video", rep.id, false);
      } catch {
      }
    },
    currentHeight() {
      try {
        return player.getCurrentRepresentationForType("video")?.height ?? null;
      } catch {
        return null;
      }
    },
  };
}

export type HlsHandle = { destroy(): void };

export async function startHls(video: HTMLVideoElement, url: string, onFatal: (why: string) => void): Promise<HlsHandle> {
  if (video.canPlayType("application/vnd.apple.mpegurl")) {
    video.src = url;
    return { destroy: () => video.removeAttribute("src") };
  }
  const { default: Hls } = await import("hls.js");
  if (!Hls.isSupported()) {
    onFatal("hls:unsupported");
    return { destroy() {} };
  }
  const hls = new Hls({ lowLatencyMode: true });
  let recoveries = 0;
  hls.on(Hls.Events.ERROR, (_e, data) => {
    if (!data.fatal) return;
    if (recoveries++ < 3) {
      if (data.type === Hls.ErrorTypes.MEDIA_ERROR) hls.recoverMediaError();
      else hls.startLoad();
    } else onFatal(`hls:${data.details}`);
  });
  hls.loadSource(url);
  hls.attachMedia(video);
  return { destroy: () => hls.destroy() };
}

// Video playback engine (W5). One <video> element lives for the whole app
// session and is moved between hosts (the watch page player, the mini
// player, an off-screen garage), so leaving the watch page never restarts the
// video. Three ways to play, tried in order:
//   dash   dash.js over the synthetic MPD (one element, quality switch in place)
//   direct video-only <video> + hidden <audio>, kept in sync (Flow's policy)
//   muxed  progressive itag (360p) as the last resort
// Lives use HLS. Everything goes through the local proxy (`StreamSet.base`).
import { goto } from "$app/navigation";
import * as api from "./api";
import {
  audioTracks,
  chooseQuality,
  cuesAt,
  decideAudioSync,
  hasRanges,
  hlsUrl,
  isWebKit,
  manifestUrl,
  mediaUrl,
  mimeCodecs,
  mseSupports,
  parseVtt,
  pickAudio,
  pickDirectVideo,
  pickMuxed,
  sbForget,
  sbStep,
  startDash,
  startHls,
  trimRate,
  videoLadder,
  absUrl,
  type AudioTrack,
  type Cue,
  type DashHandle,
  type HlsHandle,
} from "./dash";
import { thumbUrl, videoThumb } from "./format";
import { claim, register, setMeta, setPlaybackState } from "./media-session";
import { tubePrefs } from "./prefs.svelte";
import { go } from "./ui.svelte";
import type { Caption, ChannelRef, Format, SbSegment, StreamSet, VideoItem, WatchInfo } from "./types";

export type PlayMode = "dash" | "direct" | "muxed" | "hls";

type SbToast = { seg: SbSegment; undone: boolean; left: number; hold: boolean };

export const vplayer = $state({
  /** Why each playback mode failed for the current video, newest last (diagnostics). */
  failures: [] as string[],
  /** Video currently loaded (watch page or mini player). */
  videoId: null as string | null,
  info: null as WatchInfo | null,
  streams: null as StreamSet | null,
  playing: false,
  buffering: false,
  position: 0,
  duration: 0,
  volume: 1,
  muted: false,
  rate: 1,
  /** Selected heights / itags; null = auto. */
  videoItag: null as number | null,
  audioItag: null as number | null,
  captions: null as string | null,
  segments: [] as SbSegment[],
  /** Mini player visible (video kept playing after leaving the watch page). */
  mini: false,
  theater: false,
  error: null as string | null,
  /** Autoplay queue (playlist or related) for "next". */
  upNext: [] as VideoItem[],

  loading: false,
  mode: null as PlayMode | null,
  /** Quality menu (one rung per height, same list the download menu uses). */
  ladder: [] as Format[],
  autoQuality: true,
  /** Height being rendered right now. */
  height: null as number | null,
  tracks: [] as AudioTrack[],
  /** Audio track key (null = original). */
  track: null as string | null,
  buffered: 0,
  ended: false,
  fullscreen: false,
  ambient: true,
  /** Caption text showing now. */
  cue: "",
  /** Auto-translate target for captions. */
  translate: null as string | null,
  /** Last OmniTube engine that played (mini player vs music bar). */
  active: false,
  list: null as string | null,
  index: null as number | null,
  sbToast: null as SbToast | null,
  sbAsk: null as SbSegment | null,
  sbHighlight: null as SbSegment | null,
  sbFull: null as SbSegment | null,
  /** SponsorBlock auto-skip turned off for this video only. */
  sbOffHere: false,
  /** Seconds left before autoplaying the next video (null = no countdown). */
  nextIn: null as number | null,
  /** Center bezel ("play", "+10s"...): bumped `n` retriggers the animation. */
  bezel: { icon: "", text: "", n: 0 },
  /** Short (vertical) mode: loops, no mini player. */
  short: false,
});

// ------------------------------------------------------------ persistence

const LS = "omnitube.video";

type Saved = { volume: number; muted: boolean; theater: boolean; ambient: boolean; cc: boolean; ccLang: string | null; rate: number };

function load(): Partial<Saved> {
  try {
    return JSON.parse(localStorage.getItem(LS) || "{}");
  } catch {
    return {};
  }
}

const saved = typeof localStorage !== "undefined" ? load() : {};
vplayer.volume = saved.volume ?? 1;
vplayer.muted = saved.muted ?? false;
vplayer.theater = saved.theater ?? false;
vplayer.ambient = saved.ambient ?? true;
let ccWanted = saved.cc ?? false;
let ccLang = saved.ccLang ?? null;

function persist() {
  try {
    const s: Saved = {
      volume: vplayer.volume,
      muted: vplayer.muted,
      theater: vplayer.theater,
      ambient: vplayer.ambient,
      cc: ccWanted,
      ccLang,
      rate: vplayer.rate,
    };
    localStorage.setItem(LS, JSON.stringify(s));
  } catch {
    /* storage off */
  }
}

// ------------------------------------------------------------ elements

let video: HTMLVideoElement | null = null;
let companion: HTMLAudioElement | null = null;
let garage: HTMLDivElement | null = null;
let host: HTMLElement | null = null;
let dash: DashHandle | null = null;
let hls: HlsHandle | null = null;
let cues: Cue[] = [];
let sbMuted = false;
const dontSkip = new Set<string>();
const dismissed = new Set<string>();

export function videoElement(): HTMLVideoElement | null {
  return video;
}

function garageEl(): HTMLDivElement {
  if (!garage) {
    garage = document.createElement("div");
    garage.setAttribute("aria-hidden", "true");
    garage.style.cssText = "position:fixed;left:-10000px;top:0;width:16px;height:9px;overflow:hidden;pointer-events:none;";
    document.body.appendChild(garage);
  }
  return garage;
}

function el(): HTMLVideoElement {
  if (video) return video;
  const v = document.createElement("video");
  v.className = "tube-video";
  v.playsInline = true;
  v.preload = "auto";
  v.setAttribute("playsinline", "");
  v.addEventListener("timeupdate", onTime);
  v.addEventListener("durationchange", () => {
    if (Number.isFinite(v.duration) && v.duration > 0) vplayer.duration = v.duration;
  });
  v.addEventListener("play", onPlay);
  v.addEventListener("pause", onPause);
  v.addEventListener("waiting", () => (vplayer.buffering = true));
  v.addEventListener("playing", () => {
    vplayer.buffering = false;
    if (companion && companion.paused && !held) void companion.play().catch(() => {});
  });
  v.addEventListener("canplay", () => (vplayer.buffering = false));
  v.addEventListener("progress", onProgress);
  v.addEventListener("seeking", () => {
    if (companion) companion.currentTime = v.currentTime;
  });
  v.addEventListener("ratechange", () => {
    vplayer.rate = v.playbackRate;
    if (companion) companion.playbackRate = v.playbackRate;
  });
  v.addEventListener("ended", onEnded);
  v.addEventListener("error", () => {
    if (mode === "direct" || mode === "muxed") void fail(`${mode}:media`);
  });
  video = v;
  garageEl().appendChild(v);
  return v;
}

/** The watch player / mini player hands its stage: the <video> moves there. */
export function attachVideo(node: HTMLElement) {
  host = node;
  node.appendChild(el());
}

/** Host going away: park the element so playback keeps going. */
export function detachVideo(node: HTMLElement) {
  if (host !== node || !video) return;
  host = null;
  garageEl().appendChild(video);
}

function sound(): HTMLMediaElement | null {
  return companion ?? video;
}

function applyVolume() {
  const s = sound();
  if (!s) return;
  s.volume = Math.min(1, Math.max(0, vplayer.volume));
  s.muted = vplayer.muted || sbMuted;
  if (companion && video) video.muted = true;
}

// ------------------------------------------------------------ navigation

/** Open a video on the watch page (navigates). */
export function openVideo(id: string, opts: { list?: string | null; index?: number | null; t?: number | null } = {}): void {
  vplayer.mini = false;
  vplayer.short = false;
  go({ name: "watch", id, list: opts.list ?? null, index: opts.index ?? null, t: opts.t ?? null });
  if (typeof location !== "undefined" && !location.pathname.startsWith("/youtube")) void goto("/youtube");
}

/** Back to the watch page of what the mini player is showing. */
export function expandMini() {
  if (!vplayer.videoId) return;
  openVideo(vplayer.videoId, { list: vplayer.list, index: vplayer.index });
}

/** Watch page unmounted: keep playing in the corner (not for Shorts). */
export function leaveWatch() {
  report();
  if (!vplayer.videoId) return;
  if (vplayer.short || vplayer.error) {
    stopVideo();
    return;
  }
  vplayer.mini = vplayer.playing || vplayer.position > 0;
  if (!vplayer.mini) stopVideo();
}

export function closeMini(): void {
  vplayer.mini = false;
  stopVideo();
}

/** Unload everything. */
function stopVideo() {
  report();
  loadSeq++;
  teardown();
  vplayer.videoId = null;
  vplayer.info = null;
  vplayer.streams = null;
  vplayer.playing = false;
  vplayer.position = 0;
  vplayer.duration = 0;
  vplayer.mini = false;
  vplayer.upNext = [];
  vplayer.nextIn = null;
  vplayer.error = null;
  vplayer.loading = false;
  setMeta("video", null, null);
}

// ------------------------------------------------------------ loading

let loadSeq = 0;
let mode: PlayMode | null = null;
let triedModes = new Set<PlayMode>();
let fallbackTried = false;
let startAt = 0;
let autoplayWanted = true;

function teardown() {
  dash?.destroy();
  dash = null;
  hls?.destroy();
  hls = null;
  if (companion) {
    companion.pause();
    companion.removeAttribute("src");
    companion.load();
    companion = null;
  }
  if (video) {
    video.pause();
    video.removeAttribute("src");
    video.load();
    video.muted = false;
  }
  mode = null;
  vplayer.mode = null;
  cues = [];
  vplayer.cue = "";
  stopSync();
  stopSbTimer();
}

function resetFor(id: string) {
  vplayer.videoId = id;
  if (vplayer.info && vplayer.info.id !== id) vplayer.info = null;
  vplayer.streams = null;
  vplayer.error = null;
  vplayer.loading = true;
  vplayer.position = 0;
  vplayer.duration = 0;
  vplayer.buffered = 0;
  vplayer.ended = false;
  vplayer.playing = false;
  vplayer.buffering = true;
  vplayer.segments = [];
  vplayer.ladder = [];
  vplayer.tracks = [];
  vplayer.track = null;
  vplayer.height = null;
  vplayer.videoItag = null;
  vplayer.audioItag = null;
  vplayer.captions = null;
  vplayer.translate = null;
  vplayer.sbToast = null;
  vplayer.sbAsk = null;
  vplayer.sbHighlight = null;
  vplayer.sbFull = null;
  vplayer.sbOffHere = false;
  vplayer.nextIn = null;
  dontSkip.clear();
  dismissed.clear();
  triedModes = new Set();
  fallbackTried = false;
  lastReportPos = -1;
  vplayer.failures = [];
}

/** Load and (by default) start a video. Called by the watch player. */
export async function loadVideo(
  id: string,
  opts: { t?: number | null; list?: string | null; index?: number | null; autoplay?: boolean; short?: boolean } = {},
): Promise<void> {
  const seq = ++loadSeq;
  if (vplayer.videoId && vplayer.videoId !== id) report();
  teardown();
  resetFor(id);
  vplayer.list = opts.list ?? null;
  vplayer.index = opts.index ?? null;
  vplayer.short = !!opts.short;
  const p = tubePrefs();
  autoplayWanted = opts.autoplay ?? p.autoplay_on_open;
  const [setR, segR, resumeR] = await Promise.allSettled([
    api.streams(id, false),
    p.sponsorblock && !opts.short ? api.sponsorblock(id) : Promise.resolve([] as SbSegment[]),
    opts.t == null && !opts.short ? api.resumeFor(id) : Promise.resolve(null),
  ]);
  if (seq !== loadSeq) return;
  vplayer.segments = segR.status === "fulfilled" ? segR.value : [];
  startAt = opts.t ?? (resumeR.status === "fulfilled" ? resumeR.value ?? 0 : 0);
  let set: StreamSet | null = setR.status === "fulfilled" ? setR.value : null;
  if (!set) {
    fallbackTried = true;
    try {
      set = await api.streamsFallback(id, false);
    } catch (e) {
      if (seq !== loadSeq) return;
      vplayer.loading = false;
      vplayer.buffering = false;
      vplayer.error = errText(setR.status === "rejected" ? setR.reason : e);
      return;
    }
    if (seq !== loadSeq) return;
  }
  await begin(set, seq);
}

/** Try again from scratch (error screen button). */
export function retry() {
  const id = vplayer.videoId;
  if (!id) return;
  const at = vplayer.position;
  void loadVideo(id, { t: at > 0 ? at : null, list: vplayer.list, index: vplayer.index, autoplay: true, short: vplayer.short });
}

function errText(e: unknown): string {
  if (typeof e === "string") return e;
  if (e && typeof e === "object" && "message" in e) return String((e as { message: unknown }).message);
  return String(e);
}

function ladderPrefs(needRanges: boolean) {
  const p = tubePrefs();
  return { codec: p.prefer_codec, webkit: isWebKit(), needRanges, supports: needRanges ? mseSupports : canPlay };
}

const canPlay = (f: Format) => {
  const v = el();
  return v.canPlayType(mimeCodecs(f)) !== "";
};

function modesFor(set: StreamSet): PlayMode[] {
  if (set.is_live && set.hls) return ["hls"];
  const out: PlayMode[] = [];
  const hasMse = typeof MediaSource !== "undefined";
  if (hasMse && set.formats.some((f) => f.kind === "video" && hasRanges(f)) && set.formats.some((f) => f.kind === "audio" && hasRanges(f))) out.push("dash");
  if (set.formats.some((f) => f.kind === "video") && set.formats.some((f) => f.kind === "audio")) out.push("direct");
  if (set.formats.some((f) => f.kind === "muxed")) out.push("muxed");
  if (set.hls) out.push("hls");
  return out;
}

async function begin(set: StreamSet, seq: number) {
  vplayer.streams = set;
  vplayer.duration = set.duration_s || 0;
  vplayer.tracks = audioTracks(set.formats);
  vplayer.upNext = vplayer.upNext.filter((v) => v.id !== set.video_id);
  const next = modesFor(set).find((m) => !triedModes.has(m));
  if (!next) {
    await exhausted(seq);
    return;
  }
  triedModes.add(next);
  try {
    await startMode(next, set, startAt);
  } catch (e) {
    if (seq !== loadSeq) return;
    void fail(`${next}:${errText(e)}`);
    return;
  }
  if (seq !== loadSeq) return;
  vplayer.loading = false;
  updateMeta();
  pickInitialCaptions();
  startSbTimer();
  if (autoplayWanted) void play();
}

async function exhausted(seq: number) {
  const id = vplayer.videoId;
  if (!id) return;
  if (!fallbackTried) {
    fallbackTried = true;
    triedModes = new Set();
    try {
      const set = await api.streamsFallback(id, false);
      if (seq !== loadSeq) return;
      await begin(set, seq);
      return;
    } catch (e) {
      if (seq !== loadSeq) return;
      vplayer.error = errText(e);
    }
  }
  vplayer.loading = false;
  vplayer.buffering = false;
  vplayer.error ??= "playback failed";
}

/** A mode broke (decode, 403, unsupported codec): try the next one at the same time. */
async function fail(why: string) {
  const set = vplayer.streams;
  if (!set || !mode) return;
  console.debug("[omnitube] player mode failed", why);
  vplayer.failures = [...vplayer.failures.slice(-5), why];
  const at = video && video.currentTime > 0 ? video.currentTime : startAt;
  const seq = loadSeq;
  teardown();
  startAt = at;
  await begin(set, seq);
}

async function startMode(m: PlayMode, set: StreamSet, at: number) {
  const v = el();
  const p = tubePrefs();
  mode = m;
  vplayer.mode = m;
  const box = host?.getBoundingClientRect().height || 480;
  const dpr = typeof window !== "undefined" ? window.devicePixelRatio || 1 : 1;
  if (m === "dash") {
    const ladder = videoLadder(set.formats, ladderPrefs(true));
    const aud = pickAudio(set.formats, { quality: p.audio_quality, webkit: isWebKit(), track: vplayer.track, needRanges: true, supports: mseSupports });
    if (!ladder.length || !aud) throw new Error("no dash formats");
    vplayer.ladder = ladder;
    const q = pickRung(ladder, p.default_quality, box, dpr);
    vplayer.videoItag = q?.itag ?? null;
    vplayer.audioItag = aud.itag;
    vplayer.height = q?.height ?? null;
    const url = manifestUrl(set, ladder.map((f) => f.itag), [aud.itag]);
    const seq = loadSeq;
    dash = await startDash(
      v,
      url,
      at,
      (why) => {
        if (seq === loadSeq) void fail(why);
      },
      () => {
        if (q) dash?.setVideo(q.itag, q.height ?? null);
      },
    );
    applyVolume();
    v.playbackRate = vplayer.rate;
    return;
  }
  if (m === "direct") {
    const ladder = videoLadder(set.formats, ladderPrefs(false));
    const target = pickRung(ladder, p.default_quality, box, dpr);
    const vf = target ?? pickDirectVideo(set.formats, ladderPrefs(false), 720);
    const aud = pickAudio(set.formats, { quality: p.audio_quality, webkit: isWebKit(), track: vplayer.track, supports: canPlay });
    if (!vf || !aud) throw new Error("no direct formats");
    vplayer.ladder = ladder;
    vplayer.videoItag = vf.itag;
    vplayer.audioItag = aud.itag;
    vplayer.height = vf.height ?? null;
    companion = new Audio();
    companion.preload = "auto";
    companion.src = mediaUrl(set, aud.itag);
    companion.addEventListener("error", () => void fail("direct:audio"));
    v.muted = true;
    v.src = mediaUrl(set, vf.itag);
    if (at > 0) {
      v.currentTime = at;
      companion.currentTime = at;
    }
    v.playbackRate = vplayer.rate;
    companion.playbackRate = vplayer.rate;
    applyVolume();
    return;
  }
  if (m === "muxed") {
    const mf = pickMuxed(set.formats);
    if (!mf) throw new Error("no muxed format");
    vplayer.ladder = [];
    vplayer.videoItag = mf.itag;
    vplayer.height = mf.height ?? null;
    v.src = mediaUrl(set, mf.itag);
    if (at > 0) v.currentTime = at;
    v.playbackRate = vplayer.rate;
    applyVolume();
    return;
  }
  const url = hlsUrl(set);
  if (!url) throw new Error("no hls");
  vplayer.ladder = [];
  const seq = loadSeq;
  hls = await startHls(v, url, (why) => {
    if (seq === loadSeq) void fail(why);
  });
  applyVolume();
}

function pickRung(ladder: Format[], pref: string, box: number, dpr: number): Format | null {
  if (!vplayer.autoQuality && vplayer.height) {
    const h = vplayer.height;
    return [...ladder].sort((a, b) => Math.abs((a.height ?? 0) - h) - Math.abs((b.height ?? 0) - h))[0] ?? null;
  }
  return chooseQuality(ladder, pref, box, dpr);
}

// ------------------------------------------------------------ element events

let reportTimer: ReturnType<typeof setInterval> | null = null;
let lastReportPos = -1;

function onTime() {
  const v = video;
  if (!v) return;
  vplayer.position = v.currentTime;
  if (cues.length) {
    const now = cuesAt(cues, v.currentTime);
    vplayer.cue = now.map((c) => c.text).join("\n");
  }
  const h = dash?.currentHeight();
  if (h) vplayer.height = h;
  setPlaybackState("video", !v.paused, v.currentTime, vplayer.duration, v.playbackRate);
}

function onProgress() {
  const v = video;
  if (!v || !v.buffered.length) return;
  const t = v.currentTime;
  for (let i = 0; i < v.buffered.length; i++) {
    if (v.buffered.start(i) <= t + 0.5 && v.buffered.end(i) >= t) {
      vplayer.buffered = v.buffered.end(i);
      return;
    }
  }
}

function onPlay() {
  vplayer.playing = true;
  vplayer.ended = false;
  vplayer.nextIn = null;
  claim("video");
  updateMeta();
  if (companion) void companion.play().catch(() => {});
  startSync();
  reportTimer ??= setInterval(() => report(), 10_000);
}

function onPause() {
  vplayer.playing = false;
  if (companion && !companion.paused) companion.pause();
  stopSync();
  if (reportTimer) {
    clearInterval(reportTimer);
    reportTimer = null;
  }
  report();
  if (video) setPlaybackState("video", false, video.currentTime, vplayer.duration, video.playbackRate);
}

function onEnded() {
  vplayer.playing = false;
  vplayer.ended = true;
  report();
  if (vplayer.short) {
    void video?.play();
    return;
  }
  if (tubePrefs().autoplay_next && vplayer.upNext.length) {
    vplayer.nextIn = 5;
    tickNext();
  }
}

let nextTimer: ReturnType<typeof setTimeout> | null = null;
function tickNext() {
  if (nextTimer) clearTimeout(nextTimer);
  nextTimer = setTimeout(() => {
    if (vplayer.nextIn == null) return;
    if (vplayer.nextIn <= 1) {
      vplayer.nextIn = null;
      next();
      return;
    }
    vplayer.nextIn -= 1;
    tickNext();
  }, 1000);
}

export function cancelNext() {
  vplayer.nextIn = null;
  if (nextTimer) clearTimeout(nextTimer);
}

// ------------------------------------------------------------ A/V sync (direct mode)

let syncTimer: ReturnType<typeof setInterval> | null = null;
let held = false;
let lastRealign = 0;
let lastSample = { t: 0, at: 0 };

function startSync() {
  if (!companion || syncTimer) return;
  syncTimer = setInterval(syncTick, 250);
}

function stopSync() {
  if (syncTimer) clearInterval(syncTimer);
  syncTimer = null;
  held = false;
}

function syncTick() {
  const v = video;
  const a = companion;
  if (!v || !a) return;
  const now = performance.now();
  const advancing = now - lastSample.at > 0 && v.currentTime - lastSample.t > ((now - lastSample.at) / 1000) * v.playbackRate * 0.5;
  lastSample = { t: v.currentTime, at: now };
  const drift = a.currentTime - v.currentTime;
  const action = decideAudioSync({
    drift,
    held,
    videoAdvancing: advancing,
    videoStarved: v.readyState < 3,
    audioSeeking: a.seeking,
    audioReadyState: a.readyState,
    msSinceLastRealign: now - lastRealign,
  });
  const linux = /Linux/.test(navigator.userAgent) && isWebKit();
  switch (action) {
    case "steady": {
      const r = linux || v.paused ? v.playbackRate : trimRate(v.playbackRate, drift);
      if (Math.abs(a.playbackRate - r) > 0.001) a.playbackRate = r;
      if (a.paused && !v.paused) void a.play().catch(() => {});
      return;
    }
    case "hold":
      held = true;
      a.pause();
      return;
    case "resume":
      held = false;
      if (!v.paused) void a.play().catch(() => {});
      return;
    case "realign":
      lastRealign = now;
      a.currentTime = v.currentTime;
      a.playbackRate = v.playbackRate;
      if (!v.paused) void a.play().catch(() => {});
      return;
    case "wait":
      return;
  }
}

// ------------------------------------------------------------ controls

export async function play() {
  const v = video;
  if (!v) return;
  if (vplayer.ended) v.currentTime = 0;
  try {
    await v.play();
  } catch {
    /* autoplay refused; the play button still works */
  }
}

export function pause() {
  video?.pause();
}

export function toggle() {
  if (!video) return;
  if (video.paused) {
    void play();
    bezel("play");
  } else {
    pause();
    bezel("pause");
  }
}

export function seek(s: number) {
  const v = video;
  if (!v) return;
  const d = vplayer.duration || v.duration || 0;
  const t = Math.max(0, d > 0 ? Math.min(d - 0.1, s) : s);
  v.currentTime = t;
  if (companion) companion.currentTime = t;
  vplayer.position = t;
  vplayer.ended = false;
  sbForget(vplayer.segments, t, dontSkip);
  sbForget(vplayer.segments, t, dismissed);
}

export function seekBy(d: number) {
  seek((video?.currentTime ?? vplayer.position) + d);
  bezel(d < 0 ? "rewind" : "forward", `${d > 0 ? "+" : "−"}${Math.abs(d)}s`);
}

/** 0–9 keys: jump to n×10 %. */
export function seekFrac(f: number) {
  if (vplayer.duration > 0) seek(vplayer.duration * f);
}

export function setVolume(v: number) {
  vplayer.volume = Math.min(1, Math.max(0, v));
  if (vplayer.volume > 0) vplayer.muted = false;
  applyVolume();
  persist();
}

export function nudgeVolume(d: number) {
  setVolume(vplayer.volume + d);
  bezel(vplayer.volume === 0 ? "mute" : "volume", `${Math.round(vplayer.volume * 100)}%`);
}

export function toggleMute() {
  vplayer.muted = !vplayer.muted;
  applyVolume();
  persist();
  bezel(vplayer.muted ? "mute" : "volume");
}

export const RATES = [0.25, 0.5, 0.75, 1, 1.25, 1.5, 1.75, 2, 2.5, 3, 4];

export function setRate(r: number) {
  vplayer.rate = r;
  if (video) video.playbackRate = r;
  if (companion) companion.playbackRate = r;
}

export function stepRate(dir: 1 | -1) {
  const i = RATES.findIndex((r) => Math.abs(r - vplayer.rate) < 0.001);
  const j = Math.min(RATES.length - 1, Math.max(0, (i < 0 ? RATES.indexOf(1) : i) + dir));
  setRate(RATES[j]);
  bezel("speed", `${RATES[j]}x`);
}

/** Quality menu: `null` = auto. */
export function setQuality(itag: number | null) {
  const set = vplayer.streams;
  if (!set) return;
  if (itag == null) {
    vplayer.autoQuality = true;
    const box = host?.getBoundingClientRect().height || 480;
    const q = chooseQuality(vplayer.ladder, tubePrefs().default_quality, box, window.devicePixelRatio || 1);
    if (q) switchTo(q);
    return;
  }
  const f = vplayer.ladder.find((x) => x.itag === itag);
  if (!f) return;
  vplayer.autoQuality = false;
  switchTo(f);
}

function switchTo(f: Format) {
  const set = vplayer.streams;
  const v = video;
  if (!set || !v) return;
  vplayer.videoItag = f.itag;
  vplayer.height = f.height ?? null;
  if (mode === "dash") {
    dash?.setVideo(f.itag, f.height ?? null);
    return;
  }
  if (mode === "direct") {
    const t = v.currentTime;
    const wasPlaying = !v.paused;
    v.src = mediaUrl(set, f.itag);
    v.currentTime = t;
    v.playbackRate = vplayer.rate;
    if (wasPlaying) void v.play().catch(() => {});
  }
}

/** Audio track (dubs). Dash reloads at the same time; direct swaps the <audio>. */
export async function setTrack(key: string | null) {
  const set = vplayer.streams;
  const v = video;
  if (!set || !v) return;
  vplayer.track = key;
  const t = v.currentTime;
  const wasPlaying = !v.paused;
  if (mode === "dash") {
    const seq = loadSeq;
    teardown();
    triedModes.delete("dash");
    startAt = t;
    autoplayWanted = wasPlaying;
    await begin(set, seq);
    return;
  }
  if (mode === "direct" && companion) {
    const aud = pickAudio(set.formats, { quality: tubePrefs().audio_quality, webkit: isWebKit(), track: key, supports: canPlay });
    if (!aud) return;
    vplayer.audioItag = aud.itag;
    companion.src = mediaUrl(set, aud.itag);
    companion.currentTime = t;
    if (wasPlaying) void companion.play().catch(() => {});
  }
}

// ------------------------------------------------------------ captions

export function captionKey(c: Caption): string {
  return `${c.lang}${c.auto ? ".auto" : ""}`;
}

function pickInitialCaptions() {
  const set = vplayer.streams;
  if (!set || !ccWanted || !set.captions.length) return;
  const pick =
    set.captions.find((c) => !c.auto && c.lang === ccLang) ??
    set.captions.find((c) => c.lang === ccLang) ??
    set.captions.find((c) => !c.auto) ??
    set.captions[0];
  void setCaptions(captionKey(pick), false);
}

/** Turn captions on for a track key (`captionKey`), or off with null. */
export async function setCaptions(key: string | null, remember = true, translate: string | null = vplayer.translate) {
  const set = vplayer.streams;
  vplayer.captions = key;
  vplayer.translate = key ? translate : null;
  cues = [];
  vplayer.cue = "";
  if (remember) {
    ccWanted = !!key;
    if (key) ccLang = key.replace(/\.auto$/, "");
    persist();
  }
  if (!key || !set) return;
  const c = set.captions.find((x) => captionKey(x) === key);
  if (!c) return;
  let url = absUrl(set, c.url);
  if (translate) url += `${url.includes("?") ? "&" : "?"}tlang=${encodeURIComponent(translate)}`;
  try {
    const r = await fetch(url);
    if (!r.ok) throw new Error(String(r.status));
    const text = await r.text();
    if (vplayer.captions !== key) return;
    cues = parseVtt(text);
    onTime();
  } catch {
    vplayer.captions = null;
  }
}

export function toggleCaptions() {
  const set = vplayer.streams;
  if (!set?.captions.length) return;
  if (vplayer.captions) {
    void setCaptions(null);
    bezel("captions", "");
    return;
  }
  ccWanted = true;
  persist();
  pickInitialCaptions();
  bezel("captions-fill", "");
}

// ------------------------------------------------------------ SponsorBlock

let sbTimer: ReturnType<typeof setInterval> | null = null;

function startSbTimer() {
  stopSbTimer();
  if (!vplayer.segments.length) return;
  sbTimer = setInterval(sbTick, 150);
  const p = tubePrefs();
  const on = (c: SbSegment) => p.sponsorblock && (p.sb_categories[c.category] ?? "off") !== "off";
  vplayer.sbHighlight = vplayer.segments.find((s) => s.action === "poi" && on(s)) ?? null;
  vplayer.sbFull = vplayer.segments.find((s) => s.action === "full" && on(s)) ?? null;
}

function stopSbTimer() {
  if (sbTimer) clearInterval(sbTimer);
  sbTimer = null;
}

export function channelWhitelisted(): boolean {
  const ch = vplayer.info?.channel?.id;
  return !!ch && tubePrefs().sb_whitelist.includes(ch);
}

function sbTick() {
  const v = video;
  if (!v || !vplayer.segments.length) return;
  const p = tubePrefs();
  const t = v.currentTime;
  sbForget(vplayer.segments, t, dontSkip);
  sbForget(vplayer.segments, t, dismissed);
  const step = sbStep(
    vplayer.segments,
    t,
    (c) => (p.sponsorblock ? p.sb_categories[c] ?? "off" : "off"),
    !channelWhitelisted() && !vplayer.sbOffHere,
    dontSkip,
    dismissed,
  );
  if (step.skip && !v.paused) {
    seek(step.skip.end_s);
    showSbToast(step.skip);
  }
  if (vplayer.sbAsk?.uuid !== step.ask?.uuid) vplayer.sbAsk = step.ask;
  if (sbMuted !== step.mute) {
    sbMuted = step.mute;
    applyVolume();
  }
}

let toastTimer: ReturnType<typeof setInterval> | null = null;
const TOAST_MS = 5000;

function showSbToast(seg: SbSegment) {
  vplayer.sbToast = { seg, undone: false, left: TOAST_MS, hold: false };
  if (toastTimer) clearInterval(toastTimer);
  toastTimer = setInterval(() => {
    const tt = vplayer.sbToast;
    if (!tt) return;
    if (tt.hold || tt.undone) return;
    tt.left -= 100;
    if (tt.left <= 0) closeSbToast();
  }, 100);
}

export function closeSbToast() {
  vplayer.sbToast = null;
  if (toastTimer) clearInterval(toastTimer);
  toastTimer = null;
}

/** "Undo": back to the start of the skipped segment, not re-skipped until playback leaves it. */
export function undoSkip() {
  const tt = vplayer.sbToast;
  if (!tt) return;
  seek(tt.seg.start_s);
  dontSkip.add(tt.seg.uuid);
  tt.undone = true;
}

/** "Skip again" after an undo. */
export function redoSkip() {
  const tt = vplayer.sbToast;
  if (!tt) return;
  dontSkip.delete(tt.seg.uuid);
  seek(tt.seg.end_s);
  showSbToast(tt.seg);
}

/** Accept the "Skip X" prompt (also Enter). */
export function skipAsked() {
  const s = vplayer.sbAsk;
  if (!s) return;
  vplayer.sbAsk = null;
  seek(s.end_s);
  showSbToast(s);
}

export function dismissAsk() {
  const s = vplayer.sbAsk;
  if (!s) return;
  dismissed.add(s.uuid);
  vplayer.sbAsk = null;
}

export function jumpToHighlight() {
  const s = vplayer.sbHighlight;
  if (s) seek(s.start_s);
}

/** Enter key: prompt first, then the toast (undo / redo). */
export function sbEnter(): boolean {
  if (vplayer.sbAsk) {
    skipAsked();
    return true;
  }
  if (vplayer.sbToast) {
    if (vplayer.sbToast.undone) redoSkip();
    else undoSkip();
    return true;
  }
  return false;
}

// ------------------------------------------------------------ view state

export function setWatchInfo(info: WatchInfo | null) {
  vplayer.info = info;
  if (!info) return;
  const up: VideoItem[] = [];
  const pl = info.playlist;
  if (pl) {
    const i = pl.items.findIndex((v) => v.id === info.id);
    up.push(...pl.items.slice(i >= 0 ? i + 1 : pl.index + 1));
    vplayer.index = i >= 0 ? i : pl.index;
    vplayer.list = pl.id;
  }
  if (!up.length) up.push(...info.related.filter((x): x is VideoItem => x.kind === "video"));
  vplayer.upNext = up;
  updateMeta();
}

export function next() {
  cancelNext();
  const n = vplayer.upNext[0];
  if (!n) return;
  const pl = vplayer.info?.playlist;
  const inList = pl?.items.some((x) => x.id === n.id);
  const opts = inList ? { list: pl?.id ?? null, index: (vplayer.index ?? 0) + 1 } : {};
  if (vplayer.mini) {
    // the mini player moves on in place, like YouTube's
    const rest = vplayer.upNext.slice(1);
    void loadVideo(n.id, { ...opts, autoplay: true });
    vplayer.upNext = rest;
    return;
  }
  openVideo(n.id, opts);
}

export function prev() {
  const pl = vplayer.info?.playlist;
  if (vplayer.position > 3 || !pl) {
    seek(0);
    return;
  }
  const i = pl.items.findIndex((x) => x.id === vplayer.videoId);
  const p = i > 0 ? pl.items[i - 1] : null;
  if (p) openVideo(p.id, { list: pl.id, index: i - 1 });
  else seek(0);
}

export function hasPrev(): boolean {
  const pl = vplayer.info?.playlist;
  return !!pl && pl.items.findIndex((x) => x.id === vplayer.videoId) > 0;
}

function setTheater(on: boolean) {
  vplayer.theater = on;
  persist();
}

export function toggleTheater() {
  setTheater(!vplayer.theater);
}

export function setAmbient(on: boolean) {
  vplayer.ambient = on;
  persist();
}

export async function setFullscreen(on: boolean) {
  try {
    const { getCurrentWindow } = await import("@tauri-apps/api/window");
    await getCurrentWindow().setFullscreen(on);
  } catch {
    try {
      if (on) await document.documentElement.requestFullscreen();
      else if (document.fullscreenElement) await document.exitFullscreen();
    } catch {
      return;
    }
  }
  vplayer.fullscreen = on;
}

export function toggleFullscreen() {
  void setFullscreen(!vplayer.fullscreen);
}

export async function togglePip() {
  const v = video as (HTMLVideoElement & { webkitSetPresentationMode?: (m: string) => void; webkitPresentationMode?: string }) | null;
  if (!v) return;
  try {
    if (document.pictureInPictureElement) await document.exitPictureInPicture();
    else if (v.requestPictureInPicture) await v.requestPictureInPicture();
    else if (v.webkitSetPresentationMode) v.webkitSetPresentationMode(v.webkitPresentationMode === "picture-in-picture" ? "inline" : "picture-in-picture");
  } catch {
    /* PiP refused (no metadata yet) */
  }
}

export function bezel(icon: string, text = "") {
  vplayer.bezel = { icon, text, n: vplayer.bezel.n + 1 };
}

/** "Download in this quality" from the settings menu. */
export async function downloadCurrent(audioOnly = false): Promise<string> {
  const id = vplayer.videoId;
  if (!id) throw new Error("nothing loaded");
  return api.download({
    video_id: id,
    title: titleOf(),
    video_itag: audioOnly ? null : vplayer.videoItag,
    audio_itag: vplayer.audioItag,
    audio_only: audioOnly,
    sponsorblock: null,
    music: false,
  });
}

// ------------------------------------------------------------ progress & session

function titleOf(): string {
  return vplayer.info?.title ?? vplayer.streams?.title ?? vplayer.videoId ?? "";
}

function channelOf(): ChannelRef | null {
  const c = vplayer.info?.channel;
  if (c) return { id: c.id, name: c.name, avatar: c.avatar ?? null, verified: c.verified };
  const a = vplayer.streams?.author;
  return a ? { id: "", name: a } : null;
}

function thumbOf(w = 320): string {
  const s = vplayer.streams;
  const t = s ? thumbUrl(s.thumbs, w) : "";
  return t || (vplayer.videoId ? videoThumb(vplayer.videoId) : "");
}

function report() {
  const id = vplayer.videoId;
  const v = video;
  if (!id || !v || vplayer.short) return;
  const pos = v.currentTime || vplayer.position;
  const dur = vplayer.duration;
  if (dur <= 0 || pos <= 0 || Math.abs(pos - lastReportPos) < 1) return;
  lastReportPos = pos;
  void api
    .progress(
      {
        video_id: id,
        title: titleOf(),
        channel: channelOf(),
        thumb: thumbOf(),
        duration_s: Math.round(dur),
        position_s: Math.floor(pos),
        watched_ms: Date.now(),
        kind: "video",
      },
      tubePrefs().send_history,
    )
    .catch(() => {});
}

function updateMeta() {
  if (!vplayer.videoId) return;
  setMeta(
    "video",
    { title: titleOf(), artist: channelOf()?.name ?? null, artwork: thumbOf(512) },
    {
      play: () => void play(),
      pause,
      next: vplayer.upNext.length ? next : null,
      prev: hasPrev() ? prev : null,
      seek,
      seekBy: (d) => seek(vplayer.position + d),
    },
  );
}

register(
  "video",
  () => {
    if (video && !video.paused) video.pause();
  },
  (mine) => (vplayer.active = mine),
);

if (typeof window !== "undefined") {
  window.addEventListener("pagehide", () => report());
}

// The video player engine and its state. One <video> element (owned by
// Player.svelte and attached here) plays every source: direct files, ffmpeg
// remux/transcode streams that start at `offset` (seek reloads the URL with
// `ss=`), and HLS through hls.js or the native WebKit engine.
//
// Progress goes to the backend every 5 s while playing and on pause, seek and
// close; Jellyfin items also report to the server.

import { get } from "svelte/store";
import { t } from "$lib/i18n";
import * as api from "./api";
import { app } from "./app.svelte";
import { cueAt, cueBefore, fmtSpeed, parseVtt } from "./format";
import type { CourseDetail, Cue, MediaItem, Note, PlaybackInfo, PlayMode, PlayPrefs, SourceRef } from "./types";

export type PanelTab = "lessons" | "notes" | "transcript" | "ai";
export type SkipSilence = PlayPrefs["skip_silence"];

const MIN_SPEED = 0.25;
const MAX_SPEED = 4;
const REPORT_EVERY_MS = 5000;
const JF_REPORT_EVERY_MS = 10000;
const NEXT_COUNTDOWN_S = 8;
const LOCAL_KEY = "omniget.video.player.v1";

type LocalPrefs = { ambient: boolean; subPos: number; panel: PanelTab | null };

function loadLocal(): LocalPrefs {
  const d: LocalPrefs = { ambient: false, subPos: 8, panel: null };
  try {
    const raw = localStorage.getItem(LOCAL_KEY);
    return raw ? { ...d, ...JSON.parse(raw) } : d;
  } catch {
    return d;
  }
}

const local = loadLocal();

function saveLocal() {
  try {
    localStorage.setItem(
      LOCAL_KEY,
      JSON.stringify({ ambient: player.ambient, subPos: player.subs.pos, panel: player.panel } satisfies LocalPrefs),
    );
  } catch {
    /* session only */
  }
}

export const player = $state({
  open: false,
  info: null as PlaybackInfo | null,
  loading: false,
  error: null as string | null,
  /** title shown while a source resolves */
  pendingTitle: "",

  playing: false,
  buffering: false,
  ended: false,
  /** seconds, absolute (offset included) */
  position: 0,
  duration: 0,
  volume: 1,
  muted: false,
  speed: 1,
  /** temporary rate (Shift held) */
  hold: false,
  /** modes that already failed for this item, in order */
  tried: [] as PlayMode[],
  fullscreen: false,
  audioIndex: null as number | null,

  subs: {
    primary: null as string | null,
    secondary: null as string | null,
    delayMs: 0,
    size: 100,
    bg: true,
    pos: local.subPos,
  },
  cues: [] as Cue[],
  cues2: [] as Cue[],

  ab: { a: null as number | null, b: null as number | null },
  autoPause: false,
  skipSilence: "off" as SkipSilence,
  silences: [] as [number, number][],
  inSilence: false,
  savedMs: 0,
  ambient: local.ambient,

  panel: local.panel as PanelTab | null,
  showInfo: false,
  showKeys: false,
  osd: null as { text: string; id: number } | null,
  nextIn: null as number | null,

  item: null as MediaItem | null,
  course: null as CourseDetail | null,
  folder: [] as MediaItem[],
  notes: [] as Note[],
  transcript: [] as Cue[],
  transcriptState: "idle" as "idle" | "loading" | "ready" | "working" | "error",
  noteDraft: null as { t_ms: number; t_end_ms: number | null; text: string; capture: boolean } | null,
  cardDraft: null as { front: string; back: string; t_ms: number | null } | null,
});

// ---- engine internals (not reactive) ----
let el: HTMLVideoElement | null = null;
let hls: { destroy(): void } | null = null;
let offset = 0;
let pendingStart = 0;
let raf = 0;
let ticker: ReturnType<typeof setInterval> | null = null;
let lastReport = 0;
let lastJfReport = 0;
let watchedFrom = 0;
let jfStarted = false;
let fallbackBusy = false;
let lastPos = 0;
let lastLine = -1;
let osdId = 0;
let osdTimer: ReturnType<typeof setTimeout> | null = null;
let nextTimer: ReturnType<typeof setInterval> | null = null;
let prefTimer: ReturnType<typeof setTimeout> | null = null;
let prefPending: Partial<PlayPrefs> = {};
let loadToken = 0;
let wasFullscreen = false;

const tr = (key: string, vars?: Record<string, unknown>) => get(t)(key, vars) as string;

export function osd(text: string) {
  player.osd = { text, id: ++osdId };
  if (osdTimer) clearTimeout(osdTimer);
  osdTimer = setTimeout(() => (player.osd = null), 1200);
}

// ---- element wiring ----

export function attach(video: HTMLVideoElement) {
  el = video;
  el.addEventListener("timeupdate", onTime);
  el.addEventListener("play", onPlay);
  el.addEventListener("pause", onPause);
  el.addEventListener("waiting", onWaiting);
  el.addEventListener("playing", onPlaying);
  el.addEventListener("ended", onEnded);
  el.addEventListener("error", onError);
  el.addEventListener("loadedmetadata", onMeta);
  el.addEventListener("durationchange", onDuration);
  if (player.info) loadSource(player.info, pendingStart);
}

export function detach(video: HTMLVideoElement) {
  if (el !== video) return;
  el.removeEventListener("timeupdate", onTime);
  el.removeEventListener("play", onPlay);
  el.removeEventListener("pause", onPause);
  el.removeEventListener("waiting", onWaiting);
  el.removeEventListener("playing", onPlaying);
  el.removeEventListener("ended", onEnded);
  el.removeEventListener("error", onError);
  el.removeEventListener("loadedmetadata", onMeta);
  el.removeEventListener("durationchange", onDuration);
  stopLoops();
  stopHls();
  el = null;
}

function stopHls() {
  if (hls) {
    hls.destroy();
    hls = null;
  }
}

function withSs(url: string, sec: number): string {
  try {
    const u = new URL(url);
    u.searchParams.set("ss", sec.toFixed(3));
    return u.toString();
  } catch {
    return url;
  }
}

function withParam(url: string, key: string, value: string): string {
  try {
    const u = new URL(url);
    u.searchParams.set(key, value);
    return u.toString();
  } catch {
    return url;
  }
}

async function loadSource(info: PlaybackInfo, startS: number, url = info.url) {
  if (!el) {
    pendingStart = startS;
    return;
  }
  const token = ++loadToken;
  stopHls();
  player.ended = false;
  player.buffering = true;
  offset = info.seek_reloads ? info.offset_ms / 1000 : 0;
  pendingStart = 0;
  if (info.seek_reloads && Math.abs(startS - offset) > 1) {
    url = withSs(url, startS);
    offset = startS;
  } else if (!info.seek_reloads && startS > 0) {
    pendingStart = startS;
  }

  const isHls = info.mode === "hls" || /\.m3u8(\?|$)/i.test(url);
  if (isHls && !el.canPlayType("application/vnd.apple.mpegurl")) {
    const { default: Hls } = await import("hls.js");
    if (token !== loadToken || !el) return;
    if (Hls.isSupported()) {
      const h = new Hls({ startPosition: startS > 0 ? startS : -1, enableWorker: true, lowLatencyMode: info.is_live });
      let recovered = false;
      h.on(Hls.Events.ERROR, (_e, data) => {
        if (!data.fatal) return;
        if (data.type === Hls.ErrorTypes.MEDIA_ERROR && !recovered) {
          recovered = true;
          h.recoverMediaError();
          return;
        }
        if (data.type === Hls.ErrorTypes.NETWORK_ERROR && !recovered) {
          recovered = true;
          h.startLoad();
          return;
        }
        player.error = `${tr("video.player.error_stream")} (${data.details})`;
        player.buffering = false;
      });
      h.loadSource(url);
      h.attachMedia(el);
      hls = h;
      pendingStart = 0;
    } else {
      el.src = url;
    }
  } else {
    el.src = url;
  }
  el.playbackRate = effectiveRate();
  el.volume = player.volume;
  el.muted = player.muted;
  void el.play().catch(() => {
    player.playing = false;
  });
}

function effectiveRate(): number {
  if (player.hold) return Math.max(2, player.speed);
  if (player.inSilence && player.skipSilence === "speed") return silenceRate();
  return player.speed;
}

function silenceRate(): number {
  return Math.min(MAX_SPEED, Math.max(2.5, player.speed * 2));
}

function applyRate() {
  if (el) el.playbackRate = effectiveRate();
}

function curPos(): number {
  return el ? offset + (el.currentTime || 0) : player.position;
}

function curDuration(): number {
  const info = player.info;
  if (!info) return 0;
  if (info.is_live) return el && Number.isFinite(el.duration) ? el.duration : 0;
  if (!info.seek_reloads && el && Number.isFinite(el.duration) && el.duration > 0) return el.duration;
  return info.duration_ms / 1000;
}

// ---- events ----

function onMeta() {
  if (!el || !player.info) return;
  player.duration = curDuration();
  if (pendingStart > 0) {
    el.currentTime = pendingStart;
    pendingStart = 0;
  }
  // "direct" that decodes audio but no picture (HEVC on some webviews): next tier
  const i = player.info.info;
  if (player.info.mode === "direct" && el.videoWidth === 0 && i.vcodec && (i.width ?? 0) > 0) {
    void tryFallback();
  }
}

function onDuration() {
  player.duration = curDuration();
}

function onPlay() {
  player.playing = true;
  player.ended = false;
  cancelNext();
  startLoops();
}

function onPause() {
  player.playing = false;
  stopLoops();
  player.position = curPos();
  void report(false);
}

function onWaiting() {
  player.buffering = true;
}

function onPlaying() {
  player.buffering = false;
  player.error = null;
  const info = player.info;
  if (info && info.source.kind === "jellyfin" && !jfStarted) {
    jfStarted = true;
    void api.jfReport(info.source.source, info.source.item, "start", Math.round(curPos() * 1000), false).catch(() => {});
  }
}

function onEnded() {
  player.playing = false;
  player.ended = true;
  stopLoops();
  player.position = curDuration();
  void report(false);
  if (player.ab.a != null && player.ab.b != null) {
    seek(player.ab.a);
    void el?.play();
    return;
  }
  if (player.info?.next) startNextCountdown();
}

function onError() {
  if (!el || !player.info) return;
  const code = el.error?.code ?? 0;
  // 1 = aborted by us (src change)
  if (code === 1) return;
  void tryFallback();
}

export async function tryFallback() {
  const info = player.info;
  if (!info || fallbackBusy) return;
  const failed = info.mode;
  const canFallBack = (info.source.kind === "local" || info.source.kind === "url") && failed !== "transcode";
  if (!canFallBack) {
    player.error = tr("video.player.error_play");
    player.buffering = false;
    return;
  }
  fallbackBusy = true;
  const at = curPos();
  try {
    const next = await api.playFallback(info.source, failed);
    player.tried = [...player.tried, failed];
    player.info = { ...next, prefs: info.prefs };
    audioUrl = null;
    osd(tr("video.player.fallback_osd", { mode: tr(`video.player.mode_${next.mode}`) }));
    await loadSource(player.info, at);
  } catch (e) {
    player.error = String(e);
    player.buffering = false;
  } finally {
    fallbackBusy = false;
  }
}

function onTime() {
  if (!el) return;
  const pos = curPos();
  const dt = pos - lastPos;
  lastPos = pos;
  player.position = pos;
  // a jump that didn't go through seek() (PiP controls, media keys): the
  // skipped stretch must not count as watched
  if (dt > 5 || dt < -1) watchedFrom = Math.round(pos * 1000);

  // A-B loop
  const { a, b } = player.ab;
  if (a != null && b != null && pos >= b) {
    seek(a);
    return;
  }

  // skip silence
  if (player.skipSilence !== "off" && player.silences.length) {
    const s = silenceAt(pos);
    if (player.skipSilence === "skip" && s && s[1] - pos > 0.25) {
      if (!player.info?.seek_reloads || inBuffer(s[1])) {
        player.savedMs += ((s[1] - pos) * 1000) / player.speed;
        seek(s[1], false);
        return;
      }
    }
    const inside = !!s;
    if (player.skipSilence === "speed") {
      if (inside && dt > 0 && dt < 1) player.savedMs += dt * 1000 * (1 / player.speed - 1 / silenceRate());
      if (inside !== player.inSilence) {
        player.inSilence = inside;
        applyRate();
      }
    }
  } else if (player.inSilence) {
    player.inSilence = false;
    applyRate();
  }

  // auto-pause at the end of each line
  if (player.autoPause && player.playing) {
    const cues = lineCues();
    const ms = pos * 1000 - (cues === player.cues ? player.subs.delayMs : 0);
    const i = cueAt(cues, ms);
    if (lastLine >= 0 && i !== lastLine && dt > 0 && dt < 1 && ms >= cues[lastLine]?.end_ms) {
      lastLine = -1;
      el.pause();
      return;
    }
    if (i >= 0) lastLine = i;
  }
}

function silenceAt(pos: number): [number, number] | null {
  for (const s of player.silences) {
    if (s[0] > pos) break;
    if (pos >= s[0] && pos < s[1]) return s;
  }
  return null;
}

function inBuffer(sec: number): boolean {
  if (!el) return false;
  const local = sec - offset;
  for (let i = 0; i < el.buffered.length; i++) {
    if (local >= el.buffered.start(i) && local <= el.buffered.end(i)) return true;
  }
  // HLS tiers: anything ffmpeg already wrote is seekable without a reload
  for (let i = 0; i < el.seekable.length; i++) {
    if (local >= el.seekable.start(i) && local <= el.seekable.end(i) - 1) return true;
  }
  return false;
}

// ~30 fps position for subtitles, the scrubber, A-B and line pauses
let lastFrame = 0;
function frame(now: number) {
  if (!el) return;
  if (now - lastFrame >= 33) {
    lastFrame = now;
    onTime();
  }
  raf = requestAnimationFrame(frame);
}

function startLoops() {
  if (!raf) raf = requestAnimationFrame(frame);
  if (!ticker) {
    ticker = setInterval(() => {
      const now = Date.now();
      if (player.playing && now - lastReport >= REPORT_EVERY_MS) void report(true);
    }, 1000);
  }
}

function stopLoops() {
  if (raf) cancelAnimationFrame(raf);
  raf = 0;
  if (ticker) clearInterval(ticker);
  ticker = null;
}

async function report(playing: boolean, event: "progress" | "stop" = "progress") {
  const info = player.info;
  if (!info || info.is_live) return;
  const posMs = Math.round(curPos() * 1000);
  const durMs = Math.round((curDuration() || info.duration_ms / 1000) * 1000);
  const now = Date.now();
  lastReport = now;
  const from = watchedFrom;
  watchedFrom = posMs;
  if (app.incognito) return;
  if (durMs > 0) {
    void api
      .progress({ key: info.key, position_ms: posMs, duration_ms: durMs, playing, watched_from_ms: from <= posMs ? from : null })
      .catch(() => {});
  }
  if (info.source.kind === "jellyfin" && (event === "stop" || !playing || now - lastJfReport >= JF_REPORT_EVERY_MS)) {
    lastJfReport = now;
    void api.jfReport(info.source.source, info.source.item, event, posMs, !playing).catch(() => {});
  }
}

// ---- opening and closing ----

async function resolve(source: SourceRef, title?: string): Promise<PlaybackInfo> {
  switch (source.kind) {
    case "local":
    case "url":
      return api.play(source);
    case "jellyfin":
      return api.jfPlay(source.source, source.item);
    case "iptv":
      return api.iptvPlay(source.channel);
    case "addon": {
      const streams = await api.addonStreams(source.type, source.id);
      const pick =
        source.stream_index != null
          ? streams.find((s) => s.index === source.stream_index && s.addon === source.addon)
          : streams.find((s) => s.playable);
      if (!pick) throw new Error(tr("video.player.error_no_stream"));
      return api.addonPlay(pick, source.type, source.id, title ?? source.id);
    }
  }
}

/** Opens the player for any source (local file, link, Jellyfin, IPTV, add-on). */
export async function openPlayer(source: SourceRef, opts: { title?: string; startMs?: number } = {}): Promise<void> {
  finishSession();
  el?.pause();
  player.open = true;
  player.loading = true;
  player.error = null;
  player.info = null;
  player.pendingTitle = opts.title ?? "";
  try {
    const info = await resolve(source, opts.title);
    await openResolved(info, opts.startMs);
  } catch (e) {
    player.error = String(e);
    player.loading = false;
  }
}

/** Resolves a web page link with yt-dlp and plays it. */
export async function openLink(url: string): Promise<void> {
  finishSession();
  player.open = true;
  player.loading = true;
  player.error = null;
  player.info = null;
  player.pendingTitle = url;
  try {
    const info = await api.linkResolve(url.trim());
    await openResolved(info);
  } catch (e) {
    player.error = String(e);
    player.loading = false;
  }
}

/** Plays an already resolved PlaybackInfo (loose files, links, source views). */
export async function openResolved(info: PlaybackInfo, startMs?: number): Promise<void> {
  if (player.info && player.info.key !== info.key) finishSession();
  player.open = true;
  player.loading = false;
  player.error = null;
  player.info = info;
  player.tried = [];
  audioUrl = null;
  player.position = (startMs ?? info.start_ms) / 1000;
  player.duration = info.duration_ms / 1000;
  player.ended = false;
  player.ab = { a: null, b: null };
  player.savedMs = 0;
  player.inSilence = false;
  player.silences = [];
  player.cues = [];
  player.cues2 = [];
  player.notes = [];
  player.transcript = [];
  player.transcriptState = "idle";
  player.noteDraft = null;
  player.cardDraft = null;
  player.item = null;
  player.course = null;
  player.folder = [];
  watchedFrom = Math.round(player.position * 1000);
  lastReport = Date.now();
  lastJfReport = 0;
  jfStarted = false;
  lastLine = -1;
  lastPos = player.position;

  const p = info.prefs;
  player.speed = clampSpeed(p.speed || 1);
  player.volume = p.volume ?? player.volume;
  player.subs.delayMs = p.subtitle_delay_ms ?? 0;
  player.subs.size = p.subtitle_size || 100;
  player.subs.bg = p.subtitle_bg ?? true;
  player.autoPause = !!p.auto_pause;
  player.skipSilence = p.skip_silence ?? "off";
  player.subs.primary = null;
  player.subs.secondary = null;

  await loadSource(info, player.position);
  const wantAudio = p.audio_index;
  const defAudio = info.audio_tracks.find((a) => a.default) ?? info.audio_tracks[0];
  player.audioIndex = defAudio?.index ?? null;
  if (wantAudio != null && defAudio && defAudio.index !== wantAudio && info.audio_tracks.some((a) => a.index === wantAudio)) {
    setAudio(wantAudio, true);
  }
  pickInitialSubtitle(info);
  updateMediaSession();
  void loadSideData(info);
  if (player.skipSilence !== "off") void loadSilences();
}

function pickInitialSubtitle(info: PlaybackInfo) {
  const p = info.prefs;
  const subs = info.subtitles;
  if (!subs.length || p.subtitle_id === "off") return;
  const pick =
    subs.find((s) => s.id === p.subtitle_id) ??
    (p.subtitle_lang ? subs.find((s) => s.lang && s.lang.toLowerCase() === p.subtitle_lang!.toLowerCase()) : undefined) ??
    subs.find((s) => s.forced) ??
    subs.find((s) => s.default);
  if (pick) void selectSubtitle(pick.id, 1, false);
}

async function loadSideData(info: PlaybackInfo) {
  const key = info.key;
  api
    .notes(key)
    .then((n) => {
      if (player.info?.key === key) player.notes = n;
    })
    .catch(() => {});
  if (info.source.kind === "local") {
    try {
      const it = await api.item(info.source.id);
      if (player.info?.key !== key) return;
      player.item = it;
      if (info.course_id == null) {
        const rows = await api.items({ folder: it.folder, limit: 500 });
        if (player.info?.key === key) player.folder = rows.sort((x, y) => x.path.localeCompare(y.path, undefined, { numeric: true, sensitivity: "base" }));
      }
    } catch {
      /* the lessons tab just stays empty */
    }
  }
  if (info.course_id != null) {
    api
      .course(info.course_id)
      .then((c) => {
        if (player.info?.key === key) player.course = c;
      })
      .catch(() => {});
  }
}

/** Reports the last position of the current item and resets per-item timers. */
function finishSession() {
  const info = player.info;
  if (!info) return;
  if (el) player.position = curPos();
  void report(false, "stop");
  cancelNext();
}

export function closePlayer() {
  finishSession();
  loadToken++;
  stopLoops();
  stopHls();
  if (el) {
    el.pause();
    el.removeAttribute("src");
    el.load();
  }
  if (wasFullscreen) void setFullscreen(false);
  player.open = false;
  player.info = null;
  player.loading = false;
  player.error = null;
  player.playing = false;
  player.showInfo = false;
  player.showKeys = false;
  player.noteDraft = null;
  player.cardDraft = null;
  clearMediaSession();
}

// ---- transport ----

export function toggle() {
  if (!el || !player.info) return;
  if (player.ended) {
    seek(0);
    void el.play();
    return;
  }
  if (el.paused) void el.play().catch(() => {});
  else el.pause();
}

export function pause() {
  el?.pause();
}

export function seek(sec: number, doReport = true) {
  const info = player.info;
  if (!el || !info) return;
  const dur = curDuration();
  if (dur > 0) sec = Math.min(Math.max(0, sec), dur - 0.05);
  else sec = Math.max(0, sec);
  if (doReport) void report(player.playing);
  cancelNext();
  player.ended = false;
  lastLine = -1;
  if (info.seek_reloads) {
    if (inBuffer(sec)) el.currentTime = sec - offset;
    else {
      // the ffmpeg tiers are HLS: reload through loadSource so hls.js (or
      // WebKit) gets a playlist that starts at `sec`
      const playing = !el.paused;
      void loadSource(info, sec, currentUrl()).then(() => {
        if (!playing) el?.pause();
      });
    }
  } else {
    el.currentTime = sec;
  }
  player.position = sec;
  lastPos = sec;
  watchedFrom = Math.round(sec * 1000);
}

/** The URL of the current tier with any audio override (seek reloads keep it). */
let audioUrl: string | null = null;
function currentUrl(): string {
  return audioUrl ?? player.info?.url ?? "";
}

export function seekBy(delta: number) {
  seek(curPos() + delta);
}

export function seekPercent(frac: number) {
  const d = curDuration();
  if (d > 0) seek(d * frac);
}

export function frameStep(dir: 1 | -1) {
  if (!el) return;
  el.pause();
  seek(curPos() + dir / 30, false);
}

export function setVolume(v: number) {
  player.volume = Math.min(1, Math.max(0, Math.round(v * 100) / 100));
  player.muted = player.volume === 0;
  if (el) {
    el.volume = player.volume;
    el.muted = player.muted;
  }
  osd(tr("video.player.osd_volume", { value: Math.round(player.volume * 100) }));
  savePrefs({ volume: player.volume }, "global");
}

export function toggleMute() {
  player.muted = !player.muted;
  if (el) el.muted = player.muted;
  osd(player.muted ? tr("video.player.osd_muted") : tr("video.player.osd_volume", { value: Math.round(player.volume * 100) }));
}

function clampSpeed(v: number): number {
  return Math.min(MAX_SPEED, Math.max(MIN_SPEED, Math.round(v * 20) / 20));
}

export function setSpeed(v: number) {
  player.speed = clampSpeed(v);
  applyRate();
  osd(tr("video.player.osd_speed", { value: fmtSpeed(player.speed) }));
  savePrefs({ speed: player.speed });
}

export function holdFast(on: boolean) {
  if (player.hold === on) return;
  player.hold = on;
  applyRate();
  if (on) osd(tr("video.player.osd_speed", { value: fmtSpeed(effectiveRate()) }));
}

export function setAudio(index: number, quiet = false) {
  const info = player.info;
  if (!el || !info) return;
  player.audioIndex = index;
  const at = curPos();
  const tracks = (el as HTMLVideoElement & { audioTracks?: { length: number; [i: number]: { enabled: boolean } } }).audioTracks;
  const pos = info.audio_tracks.findIndex((a) => a.index === index);
  if (tracks && tracks.length === info.audio_tracks.length && pos >= 0) {
    for (let i = 0; i < tracks.length; i++) tracks[i].enabled = i === pos;
  } else if (info.source.kind === "local") {
    // the server picks the audio stream with ai=; a direct file becomes a remux
    let url = currentUrl();
    if (/\/f\/\d+/.test(url)) {
      url = url.replace(/\/f\/(\d+)/, "/m/$1");
      player.info = { ...info, mode: "remux", seek_reloads: true, offset_ms: 0 };
    }
    audioUrl = withParam(url, "ai", String(index));
    void loadSource(player.info!, at, audioUrl);
  }
  if (quiet) return;
  osd(tr("video.player.osd_audio", { name: info.audio_tracks[pos]?.label || String(index) }));
  savePrefs({ audio_index: index });
}

// ---- subtitles ----

const vttCache = new Map<string, Cue[]>();

async function fetchCues(url: string): Promise<Cue[]> {
  const hit = vttCache.get(url);
  if (hit) return hit;
  const res = await fetch(url);
  if (!res.ok) throw new Error(`HTTP ${res.status}`);
  const cues = parseVtt(await res.text());
  vttCache.set(url, cues);
  return cues;
}

/** Selects a subtitle track for slot 1 (main) or 2 (dual subtitles); null turns it off. */
export async function selectSubtitle(id: string | null, slot: 1 | 2 = 1, remember = true) {
  const info = player.info;
  if (!info) return;
  const track = id ? info.subtitles.find((s) => s.id === id) : null;
  if (slot === 1) player.subs.primary = track?.id ?? null;
  else player.subs.secondary = track?.id ?? null;
  if (remember && slot === 1) savePrefs({ subtitle_id: track?.id ?? "off", subtitle_lang: track?.lang || null });
  if (!track) {
    if (slot === 1) player.cues = [];
    else player.cues2 = [];
    return;
  }
  try {
    const cues = await fetchCues(track.url);
    if (player.info?.key !== info.key) return;
    if (slot === 1 && player.subs.primary === track.id) player.cues = cues;
    if (slot === 2 && player.subs.secondary === track.id) player.cues2 = cues;
  } catch (e) {
    osd(`${tr("video.player.error_subtitle")}: ${String(e)}`);
  }
}

export function cycleSubtitle() {
  const subs = player.info?.subtitles ?? [];
  if (!subs.length) {
    osd(tr("video.player.no_subtitles"));
    return;
  }
  const i = subs.findIndex((s) => s.id === player.subs.primary);
  const next = i + 1 < subs.length ? subs[i + 1] : null;
  void selectSubtitle(next?.id ?? null);
  osd(next ? tr("video.player.osd_subtitle", { name: next.label || next.lang }) : tr("video.player.osd_subtitle_off"));
}

export function subtitleDelay(deltaMs: number) {
  player.subs.delayMs += deltaMs;
  osd(tr("video.player.osd_sub_delay", { value: (player.subs.delayMs / 1000).toFixed(1) }));
  savePrefs({ subtitle_delay_ms: player.subs.delayMs });
}

/** Shifts the subtitles so the next line starts right now. */
export function nextLineNow() {
  const cues = player.cues;
  if (!cues.length) return;
  const ms = curPos() * 1000;
  const next = cues.find((c) => c.start_ms + player.subs.delayMs > ms);
  if (!next) return;
  player.subs.delayMs = Math.round(ms - next.start_ms);
  osd(tr("video.player.osd_sub_delay", { value: (player.subs.delayMs / 1000).toFixed(1) }));
  savePrefs({ subtitle_delay_ms: player.subs.delayMs });
}

export function setSubStyle(patch: { size?: number; bg?: boolean; pos?: number }) {
  if (patch.size != null) player.subs.size = Math.min(250, Math.max(50, patch.size));
  if (patch.bg != null) player.subs.bg = patch.bg;
  if (patch.pos != null) player.subs.pos = Math.min(40, Math.max(2, patch.pos));
  savePrefs({ subtitle_size: player.subs.size, subtitle_bg: player.subs.bg });
  saveLocal();
}

// ---- study: lines, A-B, silence ----

/** Cues used for line navigation: the main subtitle, else the transcript. */
function lineCues(): Cue[] {
  return player.cues.length ? player.cues : player.transcript;
}

function lineDelay(cues: Cue[]): number {
  return cues === player.cues ? player.subs.delayMs : 0;
}

export function currentLine(): Cue | null {
  const cues = lineCues();
  const i = cueAt(cues, curPos() * 1000 - lineDelay(cues));
  return i >= 0 ? cues[i] : null;
}

export function jumpLine(dir: -1 | 0 | 1) {
  const cues = lineCues();
  if (!cues.length) {
    osd(tr("video.player.no_lines"));
    return;
  }
  const d = lineDelay(cues);
  const ms = curPos() * 1000 - d;
  let i = cueBefore(cues, ms);
  // "previous" while early inside a line goes to the line before it
  if (dir === -1 && i >= 0 && ms - cues[i].start_ms < 800) i -= 1;
  else if (dir === 1) i += 1;
  i = Math.min(Math.max(0, i), cues.length - 1);
  lastLine = -1;
  seek((cues[i].start_ms + d) / 1000);
  if (el?.paused) void el.play().catch(() => {});
}

export function toggleAutoPause() {
  player.autoPause = !player.autoPause;
  lastLine = -1;
  osd(player.autoPause ? tr("video.player.osd_autopause_on") : tr("video.player.osd_autopause_off"));
  savePrefs({ auto_pause: player.autoPause });
}

export function abLoop() {
  const pos = curPos();
  const { a, b } = player.ab;
  if (a == null) {
    player.ab = { a: pos, b: null };
    osd(tr("video.player.osd_ab_a"));
  } else if (b == null && pos > a + 0.3) {
    player.ab = { a, b: pos };
    osd(tr("video.player.osd_ab_b"));
    seek(a);
  } else {
    player.ab = { a: null, b: null };
    osd(tr("video.player.osd_ab_off"));
  }
}

export function clearAb() {
  player.ab = { a: null, b: null };
}

export async function setSkipSilence(mode: SkipSilence) {
  player.skipSilence = mode;
  player.inSilence = false;
  applyRate();
  osd(tr(`video.player.osd_silence_${mode}`));
  savePrefs({ skip_silence: mode });
  if (mode !== "off" && !player.silences.length) await loadSilences();
}

export function cycleSkipSilence() {
  const order: SkipSilence[] = ["off", "skip", "speed"];
  void setSkipSilence(order[(order.indexOf(player.skipSilence) + 1) % order.length]);
}

async function loadSilences() {
  const info = player.info;
  if (!info || info.source.kind !== "local") {
    if (player.skipSilence !== "off") osd(tr("video.player.silence_local_only"));
    return;
  }
  try {
    const key = info.key;
    let s = await api.silences(info.source.id);
    if (player.info?.key !== key) return;
    // the backend reports seconds; tolerate milliseconds
    const dur = info.duration_ms / 1000;
    if (dur > 0 && s.some((x) => x[1] > dur * 1.5)) s = s.map((x) => [x[0] / 1000, x[1] / 1000] as [number, number]);
    player.silences = s.sort((x, y) => x[0] - y[0]);
  } catch (e) {
    osd(String(e));
  }
}

// ---- notes, transcript, cards ----

export function startNote() {
  if (!player.info) return;
  el?.pause();
  const { a, b } = player.ab;
  const ranged = a != null && b != null;
  player.noteDraft = {
    t_ms: Math.round((ranged ? a : Math.max(0, curPos() - 3)) * 1000),
    t_end_ms: ranged ? Math.round(b * 1000) : null,
    text: "",
    capture: player.info.source.kind === "local",
  };
  player.panel = "notes";
  saveLocal();
}

export async function saveNote() {
  const info = player.info;
  const d = player.noteDraft;
  if (!info || !d || !d.text.trim()) return;
  const mediaId = info.source.kind === "local" ? info.source.id : null;
  const note = await api.noteAdd(info.key, d.t_ms, d.text.trim(), { tEndMs: d.t_end_ms, capture: d.capture && mediaId != null, mediaId });
  player.notes = [...player.notes, note].sort((x, y) => x.t_ms - y.t_ms);
  player.noteDraft = null;
}

export async function updateNote(id: number, text: string) {
  await api.noteUpdate(id, text);
  player.notes = player.notes.map((n) => (n.id === id ? { ...n, text, updated_at: Date.now() } : n));
}

export async function deleteNote(id: number) {
  await api.noteDelete(id);
  player.notes = player.notes.filter((n) => n.id !== id);
}

export async function loadTranscript() {
  const info = player.info;
  if (!info || player.transcriptState === "loading" || player.transcriptState === "working") return;
  player.transcriptState = "loading";
  try {
    const mediaId = info.source.kind === "local" ? info.source.id : null;
    let cues = await api.transcript(info.key, mediaId);
    if (!cues.length && player.cues.length) cues = player.cues;
    if (!cues.length && info.subtitles.length) {
      const track = info.subtitles.find((x) => x.id === player.subs.primary) ?? info.subtitles.find((x) => x.default) ?? info.subtitles[0];
      cues = await fetchCues(track.url).catch(() => []);
    }
    if (player.info?.key !== info.key) return;
    player.transcript = cues;
    player.transcriptState = "ready";
  } catch {
    player.transcriptState = "error";
  }
}

export async function transcribe() {
  const info = player.info;
  if (!info) return;
  player.transcriptState = "working";
  try {
    const mediaId = info.source.kind === "local" ? info.source.id : null;
    const cues = await api.transcribe(info.key, mediaId);
    if (player.info?.key !== info.key) return;
    player.transcript = cues;
    player.transcriptState = "ready";
  } catch (e) {
    player.transcriptState = "error";
    throw e;
  }
}

/** Opens the card composer with the line under the playhead. */
export function startCard() {
  if (!player.info) return;
  el?.pause();
  const line = currentLine();
  player.cardDraft = {
    front: line?.text ?? "",
    back: "",
    t_ms: Math.round((line ? line.start_ms + lineDelay(lineCues()) : curPos() * 1000)),
  };
}

export async function saveCard() {
  const info = player.info;
  const d = player.cardDraft;
  if (!info || !d || !d.front.trim()) return;
  await api.cardAdd(info.key, d.front.trim(), d.back.trim(), d.t_ms);
  player.cardDraft = null;
  osd(tr("video.study.card_added"));
}

// ---- next / previous ----

export function playNext() {
  const next = player.info?.next;
  cancelNext();
  if (next) void openPlayer(next);
}

export function playPrev() {
  const prev = player.info?.prev;
  if (prev) void openPlayer(prev);
}

function startNextCountdown() {
  cancelNext();
  player.nextIn = NEXT_COUNTDOWN_S;
  nextTimer = setInterval(() => {
    if (player.nextIn == null) return cancelNext();
    player.nextIn -= 1;
    if (player.nextIn <= 0) playNext();
  }, 1000);
}

export function cancelNext() {
  if (nextTimer) clearInterval(nextTimer);
  nextTimer = null;
  player.nextIn = null;
}

// ---- view ----

export function togglePanel(tab?: PanelTab) {
  if (!tab) player.panel = player.panel ? null : "lessons";
  else player.panel = player.panel === tab ? null : tab;
  saveLocal();
}

export function setPanel(tab: PanelTab | null) {
  player.panel = tab;
  saveLocal();
}

export function setAmbient(on: boolean) {
  player.ambient = on;
  saveLocal();
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
  player.fullscreen = on;
  wasFullscreen = on;
}

export function toggleFullscreen() {
  void setFullscreen(!player.fullscreen);
}

export async function togglePip() {
  if (!el) return;
  const v = el as HTMLVideoElement & { webkitSetPresentationMode?: (m: string) => void; webkitPresentationMode?: string };
  try {
    if (document.pictureInPictureElement) await document.exitPictureInPicture();
    else if (el.requestPictureInPicture) await el.requestPictureInPicture();
    else if (v.webkitSetPresentationMode) v.webkitSetPresentationMode(v.webkitPresentationMode === "picture-in-picture" ? "inline" : "picture-in-picture");
  } catch (e) {
    osd(String(e));
  }
}

export async function openExternal(which: "mpv" | "iina" | "vlc" | null = null) {
  const info = player.info;
  if (!info) return;
  el?.pause();
  try {
    await api.openExternal(info.source, which);
  } catch (e) {
    osd(String(e));
  }
}

// ---- preferences ----

function prefScope(): string | null {
  const info = player.info;
  if (!info) return null;
  if (info.course_id != null) return `course:${info.course_id}`;
  if (player.item?.show) return `show:${player.item.show}`;
  return `media:${info.key}`;
}

let prefScopeFor: string | null = null;
let globalPending: Partial<PlayPrefs> = {};
let globalTimer: ReturnType<typeof setTimeout> | null = null;
function savePrefs(patch: Partial<PlayPrefs>, scope: string | null = prefScope()) {
  if (!scope) return;
  if (scope === "global") {
    globalPending = { ...globalPending, ...patch };
    if (globalTimer) clearTimeout(globalTimer);
    globalTimer = setTimeout(() => {
      void api.prefsSet("global", globalPending).catch(() => {});
      globalPending = {};
    }, 600);
    return;
  }
  if (prefScopeFor !== scope) {
    flushPrefs();
    prefScopeFor = scope;
  }
  prefPending = { ...prefPending, ...patch };
  if (prefTimer) clearTimeout(prefTimer);
  prefTimer = setTimeout(flushPrefs, 600);
}

function flushPrefs() {
  if (prefTimer) clearTimeout(prefTimer);
  prefTimer = null;
  if (prefScopeFor && Object.keys(prefPending).length) void api.prefsSet(prefScopeFor, prefPending).catch(() => {});
  prefPending = {};
}

// ---- media session (hardware keys, Now Playing) ----

function updateMediaSession() {
  const info = player.info;
  if (!info || !("mediaSession" in navigator)) return;
  try {
    navigator.mediaSession.metadata = new MediaMetadata({ title: info.title, artist: info.subtitle });
    navigator.mediaSession.setActionHandler("play", () => toggle());
    navigator.mediaSession.setActionHandler("pause", () => pause());
    navigator.mediaSession.setActionHandler("seekbackward", () => seekBy(-10));
    navigator.mediaSession.setActionHandler("seekforward", () => seekBy(10));
    navigator.mediaSession.setActionHandler("nexttrack", info.next ? () => playNext() : null);
    navigator.mediaSession.setActionHandler("previoustrack", info.prev ? () => playPrev() : null);
  } catch {
    /* not supported by this webview */
  }
}

function clearMediaSession() {
  if (!("mediaSession" in navigator)) return;
  try {
    navigator.mediaSession.metadata = null;
  } catch {
    /* ignore */
  }
}

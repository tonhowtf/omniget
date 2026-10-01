// The music player engine. Lives for the whole app session (the mini player
// keeps playing while you browse), so nothing here is tied to a component.
//
// Graph per deck:  <audio> -> source -> replaygain -> fade ─┐
//                                                          ├─> preamp -> 10 x peaking EQ -> analyser -> volume -> out
// Two decks let the next track preload (near gapless) and crossfade.

import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { EQ_FREQS } from "./eq";
import { shuffled } from "./library";
import {
  coverUrl,
  ensureStatus,
  patchTracksLocal,
  proxyUrl,
  radioUrl,
  trackById,
  trackUrl,
  transcodeUrl,
} from "./library-store.svelte";
import type { Station, Track } from "./types";

type Repeat = "off" | "all" | "one";
export type RgMode = "off" | "track" | "album" | "auto";
export type VisualizerMode = "bars" | "wave" | "circle" | "off";

const PREFS_KEY = "omniget.music.prefs.v1";
const SESSION_KEY = "omniget.music.session.v1";

/** Formats webviews never decode natively; these go straight to ffmpeg. */
const ALWAYS_TRANSCODE = new Set(["APE", "WavPack", "Musepack", "WMA", "Speex", "DSF"]);

function loadJson<T>(key: string, fallback: T): T {
  try {
    const raw = localStorage.getItem(key);
    return raw ? { ...fallback, ...JSON.parse(raw) } : fallback;
  } catch {
    return fallback;
  }
}

function saveJson(key: string, value: unknown) {
  try {
    localStorage.setItem(key, JSON.stringify(value));
  } catch {
    /* private mode or quota: preferences just won't persist */
  }
}

const DEFAULT_PREFS = {
  volume: 0.8,
  muted: false,
  shuffle: false,
  repeat: "off" as Repeat,
  rgMode: "auto" as RgMode,
  rgPreamp: 0,
  preventClip: true,
  crossfade: 0,
  automix: false,
  trimSilence: false,
  eqOn: false,
  eq: [0, 0, 0, 0, 0, 0, 0, 0, 0, 0] as number[],
  eqPreamp: 0,
  eqPreset: "flat",
  speed: 1,
  keepPitch: true,
  visualizer: "bars" as VisualizerMode,
};

export const prefs = $state(loadJson(PREFS_KEY, DEFAULT_PREFS));

export const player = $state({
  queue: [] as number[],
  /** order before shuffling, to restore it */
  base: [] as number[],
  index: -1,
  playing: false,
  buffering: false,
  position: 0,
  duration: 0,
  current: null as Track | null,
  radio: null as Station | null,
  radioMeta: { title: "", artist: "" as string | null, station: "" },
  sleepAt: null as number | null,
  sleepEndOfTrack: false,
  error: "",
  /** where the queue came from ("Album · X"), for the now-playing header */
  context: "",
});

type Deck = {
  el: HTMLAudioElement;
  source: MediaElementAudioSourceNode | null;
  rg: GainNode | null;
  fade: GainNode | null;
  trackId: number | null;
  /** seconds already skipped by ffmpeg `-ss` when transcoding */
  offset: number;
  transcoded: boolean;
  hls: { destroy(): void } | null;
};

let ctx: AudioContext | null = null;
let preamp: GainNode | null = null;
let filters: BiquadFilterNode[] = [];
let analyser: AnalyserNode | null = null;
let master: GainNode | null = null;
let decks: Deck[] = [];
let active = 0;
let fading = false;
let preloadedFor: number | null = null;
let ticker: ReturnType<typeof setInterval> | null = null;
let lastTick = 0;
let listenedMs = 0;
let lastSaved = 0;
let radioSession = "";
let radioUnlisten: UnlistenFn[] = [];
const needsTranscode = new Set<number>();

function makeDeck(): Deck {
  const el = new Audio();
  el.crossOrigin = "anonymous";
  el.preload = "auto";
  const d: Deck = { el, source: null, rg: null, fade: null, trackId: null, offset: 0, transcoded: false, hls: null };
  el.addEventListener("ended", () => onEnded(d));
  // hidden windows throttle timers to ~1 s; timeupdate keeps position fresh
  el.addEventListener("timeupdate", () => {
    if (d === decks[active] && ticker) pump();
  });
  el.addEventListener("error", () => onError(d));
  el.addEventListener("waiting", () => {
    if (d === decks[active]) player.buffering = true;
  });
  el.addEventListener("playing", () => {
    if (d === decks[active]) player.buffering = false;
  });
  el.addEventListener("loadedmetadata", () => {
    if (d === decks[active] && !player.radio && !d.transcoded && Number.isFinite(el.duration) && el.duration > 0) {
      player.duration = el.duration;
    }
  });
  return d;
}

function ensureDecks() {
  if (!decks.length) decks = [makeDeck(), makeDeck()];
}

/** Builds the Web Audio graph on the first user gesture (autoplay policy). */
function ensureGraph() {
  ensureDecks();
  if (ctx) {
    if (ctx.state === "suspended") void ctx.resume();
    return;
  }
  try {
    ctx = new AudioContext({ latencyHint: "playback" });
  } catch {
    ctx = null;
    return;
  }
  preamp = ctx.createGain();
  filters = EQ_FREQS.map((f, i) => {
    const b = ctx!.createBiquadFilter();
    b.type = i === 0 ? "lowshelf" : i === EQ_FREQS.length - 1 ? "highshelf" : "peaking";
    b.frequency.value = f;
    b.Q.value = 1.1;
    return b;
  });
  analyser = ctx.createAnalyser();
  analyser.fftSize = 2048;
  analyser.smoothingTimeConstant = 0.78;
  master = ctx.createGain();
  let node: AudioNode = preamp;
  for (const f of filters) {
    node.connect(f);
    node = f;
  }
  node.connect(analyser);
  analyser.connect(master);
  master.connect(ctx.destination);
  for (const d of decks) {
    d.source = ctx.createMediaElementSource(d.el);
    d.rg = ctx.createGain();
    d.fade = ctx.createGain();
    d.source.connect(d.rg);
    d.rg.connect(d.fade);
    d.fade.connect(preamp);
  }
  applyEq();
  applyVolume();
}

export function getAnalyser(): AnalyserNode | null {
  return analyser;
}

function dbToLin(db: number) {
  return Math.pow(10, db / 20);
}

function useAlbumGain(t: Track): boolean {
  if (prefs.rgMode === "album") return true;
  if (prefs.rgMode !== "auto") return false;
  // auto: album gain when neighbours in the queue are from the same album
  const i = player.queue.indexOf(t.id);
  const near = [player.queue[i - 1], player.queue[i + 1]].map((id) => (id != null ? trackById(id) : undefined));
  return near.some((n) => n && n.album && n.album === t.album && n.folder === t.folder);
}

/** ReplayGain linear factor for a track, with clipping prevention. */
function replayGainFor(t: Track): number {
  if (prefs.rgMode === "off") return 1;
  const album = useAlbumGain(t);
  const gain = (album ? t.rg_album_gain : null) ?? t.rg_track_gain;
  const peak = (album ? t.rg_album_peak : null) ?? t.rg_track_peak;
  if (gain == null) return 1; // never measured: leave the file as it is
  let db = gain + prefs.rgPreamp;
  if (prefs.preventClip && peak && peak > 0) {
    // keep the true peak under -1 dBTP
    db = Math.min(db, 20 * Math.log10(0.891 / peak));
  }
  return dbToLin(db);
}

function applyRg(d: Deck) {
  const t = d.trackId != null ? trackById(d.trackId) : undefined;
  if (d.rg && t) d.rg.gain.value = replayGainFor(t);
  else if (d.rg) d.rg.gain.value = 1;
}

export function applyEq() {
  if (!preamp) return;
  preamp.gain.value = prefs.eqOn ? dbToLin(prefs.eqPreamp) : 1;
  filters.forEach((f, i) => {
    f.gain.value = prefs.eqOn ? prefs.eq[i] ?? 0 : 0;
  });
  savePrefs();
}

export function applyVolume() {
  const v = prefs.muted ? 0 : prefs.volume * prefs.volume; // perceptual curve
  if (master) master.gain.value = v;
  else for (const d of decks) d.el.volume = Math.min(1, v);
  savePrefs();
}

export function applyReplayGain() {
  for (const d of decks) applyRg(d);
  savePrefs();
}

export function applySpeed() {
  for (const d of decks) {
    d.el.playbackRate = prefs.speed;
    d.el.preservesPitch = prefs.keepPitch;
  }
  savePrefs();
}

export function savePrefs() {
  saveJson(PREFS_KEY, $state.snapshot(prefs));
}

function saveSession() {
  saveJson(SESSION_KEY, {
    queue: player.queue,
    base: player.base,
    index: player.index,
    position: player.position,
    context: player.context,
  });
}

function stopHls(d: Deck) {
  if (d.hls) {
    d.hls.destroy();
    d.hls = null;
  }
}

function loadInto(d: Deck, t: Track, startS = 0) {
  stopHls(d);
  d.trackId = t.id;
  d.transcoded = ALWAYS_TRANSCODE.has(t.codec) || needsTranscode.has(t.id);
  d.offset = d.transcoded ? startS : 0;
  d.el.src = d.transcoded ? transcodeUrl(t.id, startS) : trackUrl(t.id);
  d.el.playbackRate = prefs.speed;
  d.el.preservesPitch = prefs.keepPitch;
  if (!d.transcoded && startS > 0) {
    const seekOnce = () => {
      d.el.currentTime = startS;
      d.el.removeEventListener("loadedmetadata", seekOnce);
    };
    d.el.addEventListener("loadedmetadata", seekOnce);
  }
  applyRg(d);
  if (d.fade) {
    d.fade.gain.cancelScheduledValues(0);
    d.fade.gain.value = 1;
  }
}

function deckPosition(d: Deck): number {
  return d.offset + (d.el.currentTime || 0);
}

function setCurrent(t: Track | null) {
  player.current = t;
  player.duration = t ? t.duration_ms / 1000 : 0;
  player.position = 0;
  listenedMs = 0;
  updateMediaSession();
}

function reportPlay(skipped: boolean, completed: boolean) {
  const t = player.current;
  if (!t || player.radio) return;
  const ms = Math.round(listenedMs);
  listenedMs = 0;
  if (ms < 1000 && !completed) return;
  void invoke("music_record_play", { id: t.id, msPlayed: ms, completed, skipped }).catch(() => {});
  if (completed || ms >= 240_000 || ms * 2 >= t.duration_ms) {
    patchTracksLocal([t.id], { play_count: t.play_count + 1, last_played: Date.now() });
  }
}

// A timer, not requestAnimationFrame: WebKit stops rAF while the window is
// hidden, and music mostly plays with the window hidden. Crossfades, the
// sleep timer and play counting must keep running.
function startTicker() {
  if (ticker) return;
  lastTick = performance.now();
  ticker = setInterval(pump, 100);
}

function pump() {
  const now = performance.now();
  const dt = Math.min(now - lastTick, 2000);
  lastTick = now;
  tick(dt);
}

function stopTicker() {
  if (ticker) clearInterval(ticker);
  ticker = null;
}

function transitionPoint(t: Track, dur: number): { at: number; fade: number } | null {
  const end = prefs.trimSilence && t.end_s ? Math.min(t.end_s, dur) : dur;
  if (prefs.automix && t.outro_s && t.outro_s < end - 1) {
    const fade = Math.min(12, Math.max(prefs.crossfade || 6, 2), end - t.outro_s);
    return { at: Math.max(0, end - fade), fade };
  }
  if (prefs.crossfade > 0) {
    const fade = Math.min(prefs.crossfade, dur / 3);
    return { at: end - fade, fade };
  }
  if (prefs.trimSilence && t.end_s && t.end_s < dur - 0.3) return { at: t.end_s, fade: 0.05 };
  return null;
}

function tick(dt: number) {
  const d = decks[active];
  if (!d) return;
  if (!player.radio) {
    player.position = deckPosition(d);
    if (player.playing && !d.el.paused) listenedMs += dt * prefs.speed;
  }
  if (player.sleepAt && Date.now() >= player.sleepAt) {
    player.sleepAt = null;
    void fadeOutAndPause(4);
  }
  const now = Date.now();
  if (now - lastSaved > 5000) {
    lastSaved = now;
    saveSession();
    updatePositionState();
  }
  const t = player.current;
  // repeat-one loops on "ended"; a crossfade would move on to the next song
  if (!t || player.radio || fading || !player.playing || prefs.repeat === "one") return;
  const dur = player.duration || t.duration_ms / 1000;
  const nextIdx = nextIndex();
  if (nextIdx == null || player.sleepEndOfTrack) return;
  const next = trackById(player.queue[nextIdx]);
  if (!next) return;
  const remaining = dur - player.position;
  // preload the next track for a near-gapless start
  if (remaining < 20 && preloadedFor !== next.id) {
    preloadedFor = next.id;
    const other = decks[1 - active];
    const start = prefs.trimSilence || prefs.automix ? next.intro_s ?? 0 : 0;
    loadInto(other, next, start > 0.05 ? start : 0);
    other.el.load();
  }
  const tp = transitionPoint(t, dur);
  if (tp && player.position >= tp.at) void crossfadeTo(nextIdx, tp.fade);
}

async function crossfadeTo(idx: number, seconds: number) {
  const next = trackById(player.queue[idx]);
  if (!next) return;
  fading = true;
  const from = decks[active];
  const to = decks[1 - active];
  if (to.trackId !== next.id) {
    const start = prefs.trimSilence || prefs.automix ? next.intro_s ?? 0 : 0;
    loadInto(to, next, start > 0.05 ? start : 0);
  }
  reportPlay(false, true);
  const t0 = ctx?.currentTime ?? 0;
  if (to.fade && from.fade && ctx) {
    to.fade.gain.setValueAtTime(0, t0);
    to.fade.gain.linearRampToValueAtTime(1, t0 + seconds);
    from.fade.gain.setValueAtTime(from.fade.gain.value, t0);
    from.fade.gain.linearRampToValueAtTime(0, t0 + seconds);
  }
  try {
    await to.el.play();
  } catch {
    /* handled by the error listener */
  }
  active = 1 - active;
  player.index = idx;
  setCurrent(next);
  preloadedFor = null;
  setTimeout(() => {
    from.el.pause();
    if (from.fade) {
      // a scheduled ramp wins over a plain `.value`, so clear it first
      from.fade.gain.cancelScheduledValues(0);
      from.fade.gain.value = 1;
    }
    fading = false;
  }, Math.max(60, seconds * 1000));
}

function nextIndex(): number | null {
  if (!player.queue.length) return null;
  if (player.index < player.queue.length - 1) return player.index + 1;
  return prefs.repeat === "all" ? 0 : null;
}

function onEnded(d: Deck) {
  if (d !== decks[active] || player.radio) return;
  if (prefs.repeat === "one") {
    reportPlay(false, true);
    void seek(0).then(() => d.el.play());
    return;
  }
  if (player.sleepEndOfTrack) {
    player.sleepEndOfTrack = false;
    reportPlay(false, true);
    player.playing = false;
    updateMediaSession();
    return;
  }
  const n = nextIndex();
  if (n == null) {
    reportPlay(false, true);
    player.playing = false;
    stopTicker();
    updateMediaSession();
    return;
  }
  if (n === 0 && prefs.shuffle) reshuffle(false);
  const other = decks[1 - active];
  const next = trackById(player.queue[n]);
  if (next && other.trackId === next.id) {
    // preloaded: swap decks for the smallest gap
    reportPlay(false, true);
    active = 1 - active;
    player.index = n;
    setCurrent(next);
    preloadedFor = null;
    void other.el.play().catch(() => {});
    return;
  }
  reportPlay(false, true);
  void playIndex(n);
}

function onError(d: Deck) {
  if (d !== decks[active]) return;
  if (player.radio) {
    player.error = `radio_failed:media_${d.el.error?.code ?? 0}`;
    player.playing = false;
    player.buffering = false;
    return;
  }
  const t = d.trackId != null ? trackById(d.trackId) : undefined;
  if (!t) return;
  if (!d.transcoded) {
    // the webview can't decode this file: let ffmpeg do it
    needsTranscode.add(t.id);
    const pos = deckPosition(d);
    loadInto(d, t, pos);
    void d.el.play().catch(() => {});
    return;
  }
  player.error = "track_failed";
  const n = nextIndex();
  if (n != null && n !== player.index) void playIndex(n);
  else {
    player.playing = false;
    player.buffering = false;
  }
}

async function fadeOutAndPause(seconds: number) {
  const d = decks[active];
  if (ctx && master) {
    const t0 = ctx.currentTime;
    const v = master.gain.value;
    master.gain.setValueAtTime(v, t0);
    master.gain.linearRampToValueAtTime(0, t0 + seconds);
    await new Promise((r) => setTimeout(r, seconds * 1000));
    pause();
    master.gain.cancelScheduledValues(ctx.currentTime);
    applyVolume();
  } else {
    d?.el.pause();
    pause();
  }
}

// ---------- public API ----------

export async function playIndex(i: number, startS = 0) {
  const id = player.queue[i];
  const t = id != null ? trackById(id) : undefined;
  if (!t) return;
  await ensureStatus();
  ensureGraph();
  if (player.radio) stopRadio();
  if (player.current && player.index !== i) reportPlay(true, false);
  fading = false;
  preloadedFor = null;
  for (const d of decks) {
    d.el.pause();
    if (d.fade) d.fade.gain.cancelScheduledValues(0);
  }
  const d = decks[active];
  const start = startS || ((prefs.trimSilence && t.intro_s && t.intro_s > 0.05) ? t.intro_s : 0);
  loadInto(d, t, start);
  player.index = i;
  player.error = "";
  setCurrent(t);
  player.position = start;
  try {
    await d.el.play();
    player.playing = true;
  } catch (e) {
    player.playing = false;
    if (String(e).includes("NotAllowed")) return;
  }
  startTicker();
  saveSession();
}

/** Replaces the queue. `startId` is the track clicked (kept first when shuffling). */
export async function playTracks(ids: number[], startId?: number, context = "") {
  if (!ids.length) return;
  player.base = [...ids];
  player.context = context;
  const first = startId ?? ids[0];
  player.queue = prefs.shuffle ? shuffled(ids, first) : [...ids];
  await playIndex(Math.max(0, player.queue.indexOf(first)));
}

export function enqueue(ids: number[], next = false) {
  if (!ids.length) return;
  if (player.index < 0 || !player.queue.length) {
    void playTracks(ids);
    return;
  }
  const at = next ? player.index + 1 : player.queue.length;
  player.queue = [...player.queue.slice(0, at), ...ids, ...player.queue.slice(at)];
  player.base = [...player.base, ...ids];
  saveSession();
}

export function removeFromQueue(i: number) {
  if (i < 0 || i >= player.queue.length) return;
  const id = player.queue[i];
  player.queue = player.queue.filter((_, j) => j !== i);
  const bi = player.base.indexOf(id);
  if (bi >= 0) player.base = player.base.filter((_, j) => j !== bi);
  if (i < player.index) player.index--;
  else if (i === player.index) {
    if (player.queue.length) void playIndex(Math.min(i, player.queue.length - 1));
    else stop();
  }
  saveSession();
}

export function moveInQueue(from: number, to: number) {
  if (from === to || from < 0 || to < 0 || from >= player.queue.length || to >= player.queue.length) return;
  const q = [...player.queue];
  const [x] = q.splice(from, 1);
  q.splice(to, 0, x);
  if (player.index === from) player.index = to;
  else if (from < player.index && to >= player.index) player.index--;
  else if (from > player.index && to <= player.index) player.index++;
  player.queue = q;
  preloadedFor = null;
  saveSession();
}

export function clearQueue(keepCurrent = true) {
  const cur = player.queue[player.index];
  if (keepCurrent && cur != null) {
    player.queue = [cur];
    player.base = [cur];
    player.index = 0;
  } else {
    stop();
    player.queue = [];
    player.base = [];
    player.index = -1;
  }
  preloadedFor = null;
  saveSession();
}

export async function toggle() {
  if (player.radio) {
    const d = decks[active];
    if (d.el.paused) {
      ensureGraph();
      await d.el.play().catch(() => {});
      player.playing = true;
    } else {
      d.el.pause();
      player.playing = false;
    }
    updateMediaSession();
    return;
  }
  if (!player.current) {
    if (player.queue.length) await playIndex(Math.max(0, player.index), player.position);
    return;
  }
  const d = decks[active];
  if (d.trackId !== player.current.id) {
    await playIndex(player.index, player.position);
    return;
  }
  if (d.el.paused) {
    ensureGraph();
    try {
      await d.el.play();
      player.playing = true;
      startTicker();
    } catch {
      /* ignore */
    }
  } else pause();
  updateMediaSession();
}

export function pause() {
  for (const d of decks) d.el.pause();
  player.playing = false;
  saveSession();
  updateMediaSession();
}

export function stop() {
  reportPlay(true, false);
  if (player.radio) stopRadio();
  for (const d of decks) {
    d.el.pause();
    d.el.removeAttribute("src");
    d.el.load();
    d.trackId = null;
  }
  player.playing = false;
  player.current = null;
  player.position = 0;
  stopTicker();
  updateMediaSession();
}

export async function next() {
  if (player.radio) return;
  const n = nextIndex();
  if (n == null) return;
  if (n === 0 && prefs.shuffle) reshuffle(false);
  await playIndex(n);
}

export async function prev() {
  if (player.radio) return;
  if (player.position > 3 || player.index <= 0) {
    await seek(0);
    return;
  }
  await playIndex(player.index - 1);
}

export async function seek(s: number) {
  const d = decks[active];
  const t = player.current;
  if (!d || !t || player.radio) return;
  const target = Math.max(0, Math.min(s, (player.duration || t.duration_ms / 1000) - 0.25));
  if (d.transcoded) {
    const wasPlaying = !d.el.paused;
    loadInto(d, t, target);
    player.position = target;
    if (wasPlaying) await d.el.play().catch(() => {});
  } else {
    d.el.currentTime = target;
    player.position = target;
  }
  // a seek past the crossfade point must not trigger a fade immediately
  fading = false;
  updatePositionState();
}

export function seekBy(delta: number) {
  void seek(player.position + delta);
}

function reshuffle(keepCurrent = true) {
  const cur = player.queue[player.index];
  const pool = player.base.length ? player.base : player.queue;
  player.queue = shuffled(pool, keepCurrent ? cur : undefined);
  player.index = 0;
  preloadedFor = null;
}

export function toggleShuffle() {
  prefs.shuffle = !prefs.shuffle;
  const cur = player.queue[player.index];
  if (prefs.shuffle) reshuffle(true);
  else if (player.base.length) {
    player.queue = [...player.base];
    player.index = Math.max(0, player.queue.indexOf(cur));
  }
  preloadedFor = null;
  savePrefs();
  saveSession();
}

export function cycleRepeat() {
  prefs.repeat = prefs.repeat === "off" ? "all" : prefs.repeat === "all" ? "one" : "off";
  savePrefs();
}

export function setVolume(v: number) {
  prefs.volume = Math.max(0, Math.min(1, v));
  if (prefs.volume > 0) prefs.muted = false;
  applyVolume();
}

export function toggleMute() {
  prefs.muted = !prefs.muted;
  applyVolume();
}

export function setSleepTimer(minutes: number | "track" | null) {
  player.sleepEndOfTrack = minutes === "track";
  player.sleepAt = typeof minutes === "number" ? Date.now() + minutes * 60_000 : null;
}

// ---------- radio ----------

function stopRadio() {
  for (const d of decks) stopHls(d);
  for (const u of radioUnlisten) u();
  radioUnlisten = [];
  player.radio = null;
  player.radioMeta = { title: "", artist: "", station: "" };
}

export async function playStation(st: Station) {
  await ensureStatus();
  ensureGraph();
  if (player.current) reportPlay(true, false);
  for (const d of decks) {
    d.el.pause();
    d.trackId = null;
    if (d.fade) d.fade.gain.value = 1;
    if (d.rg) d.rg.gain.value = 1;
  }
  stopRadio();
  stopTicker();
  player.current = null;
  player.radio = st;
  player.radioMeta = { title: "", artist: "", station: st.name };
  player.error = "";
  player.position = 0;
  player.duration = 0;
  player.buffering = true;
  radioSession = `${Date.now()}-${Math.random().toString(36).slice(2, 8)}`;
  const session = radioSession;
  radioUnlisten.push(
    await listen<{ session: string; title: string; artist: string | null }>("music-radio-meta", (e) => {
      if (e.payload.session !== session) return;
      player.radioMeta = { ...player.radioMeta, title: e.payload.title, artist: e.payload.artist };
      updateMediaSession();
    }),
  );
  const d = decks[active];
  let url = st.url;
  try {
    url = await invoke<string>("music_radio_resolve", { url: st.url });
  } catch {
    /* keep the original */
  }
  const isHls = st.hls || /\.m3u8(\?|$)/i.test(url);
  if (isHls) {
    const { default: Hls } = await import("hls.js");
    if (Hls.isSupported()) {
      const hls = new Hls({
        // every request goes through the local proxy so Web Audio gets CORS
        xhrSetup: (xhr: XMLHttpRequest, u: string) => xhr.open("GET", proxyUrl(u), true),
        lowLatencyMode: false,
      });
      hls.on(Hls.Events.ERROR, (_e, data) => {
        if (!data.fatal || session !== radioSession) return;
        // one recovery attempt for media errors, then give up visibly
        if (data.type === Hls.ErrorTypes.MEDIA_ERROR && !recovered) {
          recovered = true;
          hls.recoverMediaError();
          return;
        }
        player.error = `radio_failed:hls_${data.details}`;
        player.playing = false;
        player.buffering = false;
      });
      let recovered = false;
      hls.loadSource(url);
      hls.attachMedia(d.el);
      d.hls = hls;
      watchId3(d.el, session);
    } else {
      d.el.src = proxyUrl(url);
    }
  } else {
    d.el.src = radioUrl(url, session);
  }
  try {
    await d.el.play();
    player.playing = true;
  } catch (e) {
    player.playing = false;
    player.buffering = false;
    player.error = `radio_failed:${(e as Error)?.name ?? "unknown"}`;
  }
  startTicker();
  updateMediaSession();
  void invoke("music_radio_played", { id: st.id }).catch(() => {});
}

/** HLS radio puts "now playing" in timed ID3 (TIT2 title, TPE1 artist). */
function watchId3(el: HTMLAudioElement, session: string) {
  const attach = (tt: TextTrack) => {
    if (tt.kind !== "metadata") return;
    tt.mode = "hidden";
    tt.addEventListener("cuechange", () => {
      if (session !== radioSession) return;
      const cues = tt.activeCues;
      if (!cues) return;
      let title = "";
      let artist = "";
      for (const c of Array.from(cues)) {
        const v = (c as unknown as { value?: { key?: string; data?: unknown } }).value;
        if (v?.key === "TIT2" && typeof v.data === "string") title = v.data;
        if (v?.key === "TPE1" && typeof v.data === "string") artist = v.data;
      }
      if (title) {
        player.radioMeta = { ...player.radioMeta, title, artist };
        updateMediaSession();
      }
    });
  };
  for (const tt of Array.from(el.textTracks)) attach(tt);
  el.textTracks.addEventListener("addtrack", (e) => {
    if (e.track) attach(e.track);
  });
}

// ---------- OS media controls ----------

function updateMediaSession() {
  if (typeof navigator === "undefined" || !("mediaSession" in navigator)) return;
  const ms = navigator.mediaSession;
  try {
    if (player.radio) {
      ms.metadata = new MediaMetadata({
        title: player.radioMeta.title || player.radio.name,
        artist: player.radioMeta.artist || player.radio.name,
        album: player.radio.name,
        artwork: player.radio.favicon ? [{ src: player.radio.favicon }] : [],
      });
    } else if (player.current) {
      const t = player.current;
      const art = coverUrl(t.cover, false);
      ms.metadata = new MediaMetadata({
        title: t.title,
        artist: t.artist,
        album: t.album,
        artwork: art ? [{ src: art, sizes: "512x512" }] : [],
      });
    } else {
      ms.metadata = null;
    }
    ms.playbackState = player.playing ? "playing" : player.current || player.radio ? "paused" : "none";
  } catch {
    /* older webviews */
  }
}

function updatePositionState() {
  if (typeof navigator === "undefined" || !("mediaSession" in navigator) || !player.current) return;
  try {
    navigator.mediaSession.setPositionState({
      duration: Math.max(player.duration, player.position, 0.1),
      position: Math.min(player.position, player.duration || player.position),
      playbackRate: prefs.speed,
    });
  } catch {
    /* ignore */
  }
}

let initialized = false;

/** Restores the last queue (paused) and wires OS media keys. Safe to call often. */
export function initPlayer() {
  if (initialized || typeof window === "undefined") return;
  initialized = true;
  ensureDecks();
  const s = loadJson(SESSION_KEY, { queue: [] as number[], base: [] as number[], index: -1, position: 0, context: "" });
  player.queue = s.queue ?? [];
  player.base = s.base ?? [];
  player.index = s.index ?? -1;
  player.position = s.position ?? 0;
  player.context = s.context ?? "";
  if ("mediaSession" in navigator) {
    const ms = navigator.mediaSession;
    const set = (a: MediaSessionAction, h: MediaSessionActionHandler) => {
      try {
        ms.setActionHandler(a, h);
      } catch {
        /* unsupported action */
      }
    };
    set("play", () => void toggle());
    set("pause", () => pause());
    set("previoustrack", () => void prev());
    set("nexttrack", () => void next());
    set("stop", () => stop());
    set("seekbackward", () => seekBy(-10));
    set("seekforward", () => seekBy(10));
    set("seekto", (d) => {
      if (d.seekTime != null) void seek(d.seekTime);
    });
  }
}

/** After the library loads, the restored queue can show its current track. */
export function hydrateCurrent() {
  if (player.current || player.radio) return;
  const id = player.queue[player.index];
  const t = id != null ? trackById(id) : undefined;
  if (t) {
    player.current = t;
    player.duration = t.duration_ms / 1000;
  }
}

/** Keeps `player.current` in sync when the row changes (tags edited, liked). */
export function syncCurrent() {
  if (!player.current) return;
  const t = trackById(player.current.id);
  if (t && t !== player.current) {
    player.current = t;
    for (const d of decks) if (d.trackId === t.id) applyRg(d);
  }
}

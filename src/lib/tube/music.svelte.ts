import * as api from "./api";
import { isWebKit, loudnessGain, mediaUrl, pickAudio } from "./dash";
import { errText, thumbUrl } from "./format";
import { claim, register, setMeta, setPlaybackState } from "./media-session";
import { tubePrefs } from "./prefs.svelte";
import type { Lyrics, QueuePage, SongItem, StreamSet } from "./types";

type RepeatMode = "off" | "all" | "one";

export const mplayer = $state({
  queue: [] as SongItem[],
  index: -1,
  playing: false,
  buffering: false,
  position: 0,
  duration: 0,
  volume: 1,
  muted: false,
  shuffle: false,
  repeat: "off" as RepeatMode,
  radio: false,
  context: "" as string,
  continuation: null as string | null,
  error: null as string | null,

  playlistId: null as string | null,
  lyricsId: null as string | null,
  relatedId: null as string | null,
  active: false,
  extending: false,
});

export function currentSong(): SongItem | null {
  return mplayer.index >= 0 ? mplayer.queue[mplayer.index] ?? null : null;
}

const LS = "omnitube.music";
type Saved = {
  queue: SongItem[];
  index: number;
  position: number;
  volume: number;
  muted: boolean;
  shuffle: boolean;
  repeat: RepeatMode;
  radio: boolean;
  context: string;
  continuation: string | null;
  playlistId: string | null;
};

function restore() {
  if (typeof localStorage === "undefined") return;
  try {
    const s = JSON.parse(localStorage.getItem(LS) || "null") as Saved | null;
    if (!s) return;
    mplayer.queue = Array.isArray(s.queue) ? s.queue.slice(0, 500) : [];
    mplayer.index = Math.min(s.index ?? -1, mplayer.queue.length - 1);
    mplayer.position = s.position ?? 0;
    mplayer.volume = s.volume ?? 1;
    mplayer.muted = !!s.muted;
    mplayer.shuffle = !!s.shuffle;
    mplayer.repeat = s.repeat ?? "off";
    mplayer.radio = !!s.radio;
    mplayer.context = s.context ?? "";
    mplayer.continuation = s.continuation ?? null;
    mplayer.playlistId = s.playlistId ?? null;
  } catch {
  }
}
restore();

let saveTimer: ReturnType<typeof setTimeout> | null = null;
function persist() {
  if (saveTimer) return;
  saveTimer = setTimeout(() => {
    saveTimer = null;
    try {
      const s: Saved = {
        queue: mplayer.queue.slice(0, 500),
        index: mplayer.index,
        position: mplayer.position,
        volume: mplayer.volume,
        muted: mplayer.muted,
        shuffle: mplayer.shuffle,
        repeat: mplayer.repeat,
        radio: mplayer.radio,
        context: mplayer.context,
        continuation: mplayer.continuation,
        playlistId: mplayer.playlistId,
      };
      localStorage.setItem(LS, JSON.stringify(s));
    } catch {
    }
  }, 800);
}

let audio: HTMLAudioElement | null = null;
let gain = 1;
let loadedId: string | null = null;
let fallbackFor: string | null = null;

export function attachAudio(el: HTMLAudioElement) {
  if (audio === el) return;
  audio = el;
  el.preload = "auto";
  el.addEventListener("timeupdate", onTime);
  el.addEventListener("durationchange", () => {
    if (Number.isFinite(el.duration) && el.duration > 0) mplayer.duration = el.duration;
  });
  el.addEventListener("play", onPlay);
  el.addEventListener("pause", () => {
    mplayer.playing = false;
    persist();
    report();
    setPlaybackState("music", false, el.currentTime, mplayer.duration);
  });
  el.addEventListener("waiting", () => (mplayer.buffering = true));
  el.addEventListener("playing", () => (mplayer.buffering = false));
  el.addEventListener("canplay", () => (mplayer.buffering = false));
  el.addEventListener("ended", onEnded);
  el.addEventListener("error", onError);
  applyVolume();
}

export function detachAudio(el: HTMLAudioElement) {
  if (audio !== el) return;
  el.pause();
  audio = null;
  loadedId = null;
}

function applyVolume() {
  if (!audio) return;
  audio.volume = Math.min(1, Math.max(0, mplayer.volume * gain));
  audio.muted = mplayer.muted;
}

type Resolved = { set: StreamSet; url: string };
const cache = new Map<string, Resolved>();
const inflight = new Map<string, Promise<Resolved>>();

function fresh(r: Resolved | undefined): r is Resolved {
  return !!r && r.set.expires_ms - Date.now() > 5 * 60_000;
}

async function resolve(id: string, fallback = false): Promise<Resolved> {
  const hit = cache.get(id);
  if (!fallback && fresh(hit)) return hit;
  const running = !fallback ? inflight.get(id) : undefined;
  if (running) return running;
  const p = (async () => {
    const set = fallback ? await api.streamsFallback(id, true) : await api.streams(id, true);
    const f = pickAudio(set.formats, { quality: tubePrefs().audio_quality, webkit: isWebKit() });
    if (!f) throw new Error("no audio format");
    const r = { set, url: mediaUrl(set, f.itag) };
    cache.set(id, r);
    if (cache.size > 30) cache.delete(cache.keys().next().value as string);
    return r;
  })();
  if (!fallback) inflight.set(id, p);
  try {
    return await p;
  } finally {
    inflight.delete(id);
  }
}

let loadSeq = 0;

async function start(i: number, at = 0, autoplay = true): Promise<void> {
  const song = mplayer.queue[i];
  if (!song || !audio) return;
  const seq = ++loadSeq;
  report();
  mplayer.index = i;
  mplayer.position = at;
  mplayer.duration = song.duration_s ?? 0;
  mplayer.error = null;
  mplayer.buffering = true;
  warmed = null;
  updateMeta();
  persist();
  if (autoplay) claim("music");
  let r: Resolved;
  try {
    r = await resolve(song.id, fallbackFor === song.id);
  } catch (e) {
    if (seq !== loadSeq) return;
    if (fallbackFor !== song.id) {
      fallbackFor = song.id;
      void start(i, at, autoplay);
      return;
    }
    mplayer.buffering = false;
    mplayer.error = errText(e);
    skipAfterError();
    return;
  }
  if (seq !== loadSeq || !audio) return;
  gain = tubePrefs().normalize_volume ? loudnessGain(r.set.loudness_db) : 1;
  applyVolume();
  loadedId = song.id;
  audio.src = r.url;
  if (at > 0) audio.currentTime = at;
  if (autoplay) {
    try {
      await audio.play();
    } catch {
      mplayer.buffering = false;
    }
  } else mplayer.buffering = false;
  void refreshNext(song, seq);
  void prefetchNext();
  void extendIfLow();
}

let errorStreak = 0;
function skipAfterError() {
  if (++errorStreak > 2) {
    errorStreak = 0;
    return;
  }
  setTimeout(() => {
    if (mplayer.error && mplayer.index < mplayer.queue.length - 1) void start(mplayer.index + 1);
  }, 1200);
}

function onError() {
  const s = currentSong();
  if (!s || !audio?.src) return;
  if (fallbackFor !== s.id) {
    fallbackFor = s.id;
    cache.delete(s.id);
    void start(mplayer.index, audio.currentTime || mplayer.position);
    return;
  }
  mplayer.error = "playback failed";
  mplayer.buffering = false;
  skipAfterError();
}

let reportTimer: ReturnType<typeof setInterval> | null = null;

function onPlay() {
  mplayer.playing = true;
  errorStreak = 0;
  claim("music");
  updateMeta();
  reportTimer ??= setInterval(report, 10_000);
}

let lastPersist = 0;
function onTime() {
  if (!audio) return;
  mplayer.position = audio.currentTime;
  const d = mplayer.duration;
  if (d > 0 && d - audio.currentTime < 5) warmNext();
  const now = Date.now();
  if (now - lastPersist > 5000) {
    lastPersist = now;
    persist();
  }
  setPlaybackState("music", !audio.paused, audio.currentTime, d);
}

function onEnded() {
  report();
  if (mplayer.repeat === "one") {
    if (audio) {
      audio.currentTime = 0;
      void audio.play();
    }
    return;
  }
  if (mplayer.index < mplayer.queue.length - 1) {
    void start(mplayer.index + 1);
    return;
  }
  if (mplayer.repeat === "all" && mplayer.queue.length) {
    void start(0);
    return;
  }
  void extendIfLow(true).then((added) => {
    if (added) void start(mplayer.index + 1);
    else mplayer.playing = false;
  });
}

let warmed: string | null = null;
let warmer: HTMLAudioElement | null = null;

function nextIndex(): number {
  if (mplayer.repeat === "one") return mplayer.index;
  if (mplayer.index < mplayer.queue.length - 1) return mplayer.index + 1;
  return mplayer.repeat === "all" ? 0 : -1;
}

async function prefetchNext() {
  const i = nextIndex();
  const s = i >= 0 ? mplayer.queue[i] : null;
  if (!s || s.id === currentSong()?.id) return;
  try {
    await resolve(s.id);
  } catch {
  }
}

function warmNext() {
  const i = nextIndex();
  const s = i >= 0 ? mplayer.queue[i] : null;
  if (!s || warmed === s.id) return;
  warmed = s.id;
  const r = cache.get(s.id);
  if (!fresh(r)) {
    void prefetchNext();
    return;
  }
  warmer ??= new Audio();
  warmer.preload = "auto";
  warmer.muted = true;
  warmer.src = r.url;
  warmer.load();
}

function merge(first: SongItem, page: QueuePage): SongItem[] {
  const tracks = page.tracks.filter((t) => t.id !== first.id);
  return [first, ...tracks];
}

let nextInfoFor: string | null = null;

async function refreshNext(song: SongItem, seq: number) {
  if (nextInfoFor === song.id) return;
  nextInfoFor = song.id;
  mplayer.lyricsId = null;
  mplayer.relatedId = null;
  try {
    const page = await api.musicNext(song.id, null, false, null);
    if (seq !== loadSeq || currentSong()?.id !== song.id) return;
    mplayer.lyricsId = page.lyrics_browse_id ?? null;
    mplayer.relatedId = page.related_browse_id ?? null;
  } catch {
    nextInfoFor = null;
  }
}

async function extendIfLow(force = false): Promise<boolean> {
  if (mplayer.extending) return false;
  const left = mplayer.queue.length - 1 - mplayer.index;
  const endless = tubePrefs().endless_radio;
  if (mplayer.radio) {
    if (!force && left > 2) return false;
    if (!endless && !mplayer.continuation) return false;
  } else if (!force || !endless) {
    return false;
  }
  const seed = mplayer.queue[mplayer.queue.length - 1];
  if (!seed) return false;
  const cont = mplayer.radio ? mplayer.continuation : null;
  mplayer.extending = true;
  try {
    const page = cont ? await api.musicNext(seed.id, mplayer.playlistId, true, cont) : await api.musicNext(seed.id, null, true, null);
    const have = new Set(mplayer.queue.map((s) => s.id));
    const add = page.tracks.filter((t) => !have.has(t.id));
    mplayer.continuation = page.continuation ?? null;
    if (page.playlist_id) mplayer.playlistId = page.playlist_id;
    mplayer.radio = true;
    if (add.length) mplayer.queue.push(...add);
    persist();
    return add.length > 0;
  } catch {
    return false;
  } finally {
    mplayer.extending = false;
  }
}

export async function playSongs(songs: SongItem[], startIndex = 0, context = ""): Promise<void> {
  if (!songs.length) return;
  mplayer.queue = [...songs];
  mplayer.context = context;
  mplayer.radio = false;
  mplayer.continuation = null;
  mplayer.playlistId = context.startsWith("playlist:") ? context.slice(9) : null;
  original = null;
  if (mplayer.shuffle) shuffleAhead(startIndex);
  await start(mplayer.shuffle ? 0 : Math.min(startIndex, songs.length - 1));
}

export async function playSong(song: SongItem, opts: { playlistId?: string | null; radio?: boolean } = {}): Promise<void> {
  mplayer.queue = [song];
  mplayer.context = opts.playlistId ? `playlist:${opts.playlistId}` : `radio:${song.id}`;
  mplayer.playlistId = opts.playlistId ?? null;
  mplayer.continuation = null;
  mplayer.radio = opts.radio ?? !opts.playlistId;
  original = null;
  nextInfoFor = song.id;
  mplayer.lyricsId = null;
  mplayer.relatedId = null;
  const started = start(0);
  try {
    const page = await api.musicNext(song.id, opts.playlistId ?? null, mplayer.radio, null);
    if (mplayer.queue[0]?.id !== song.id) return;
    mplayer.queue = merge(song, page);
    mplayer.continuation = page.continuation ?? null;
    if (page.playlist_id) mplayer.playlistId = page.playlist_id;
    mplayer.lyricsId = page.lyrics_browse_id ?? null;
    mplayer.relatedId = page.related_browse_id ?? null;
    nextInfoFor = song.id;
    persist();
    void prefetchNext();
  } catch {
  }
  await started;
}

export async function playRadio(videoId: string, playlistId?: string | null): Promise<void> {
  mplayer.context = `radio:${playlistId ?? videoId}`;
  mplayer.radio = true;
  mplayer.continuation = null;
  mplayer.playlistId = playlistId ?? null;
  original = null;
  try {
    const page = await api.musicNext(videoId, playlistId ?? null, true, null);
    if (!page.tracks.length) throw new Error("empty radio");
    mplayer.queue = page.tracks;
    mplayer.continuation = page.continuation ?? null;
    if (page.playlist_id) mplayer.playlistId = page.playlist_id;
    mplayer.lyricsId = page.lyrics_browse_id ?? null;
    mplayer.relatedId = page.related_browse_id ?? null;
    nextInfoFor = videoId;
    const i = Math.max(0, page.tracks.findIndex((t) => t.id === videoId));
    await start(i);
  } catch (e) {
    mplayer.error = errText(e);
  }
}

export async function playPlaylist(playlistId: string, shuffle = false): Promise<void> {
  try {
    const pl = await api.musicPlaylist(playlistId);
    const songs = pl.items.filter((x): x is SongItem => x.kind === "song");
    if (!songs.length) return;
    if (shuffle) mplayer.shuffle = true;
    await playSongs(songs, shuffle ? Math.floor(Math.random() * songs.length) : 0, `playlist:${playlistId}`);
    mplayer.continuation = pl.continuation ?? null;
  } catch (e) {
    mplayer.error = errText(e);
  }
}

export function enqueue(songs: SongItem[], next = false): void {
  if (!songs.length) return;
  if (mplayer.index < 0) {
    void playSongs(songs, 0, "queue");
    return;
  }
  if (next) mplayer.queue.splice(mplayer.index + 1, 0, ...songs);
  else mplayer.queue.push(...songs);
  persist();
  void prefetchNext();
}

export function toggle(): void {
  if (!audio) return;
  const s = currentSong();
  if (!s) return;
  if (loadedId !== s.id) {
    void start(mplayer.index, mplayer.position);
    return;
  }
  if (audio.paused) void audio.play().catch(() => {});
  else audio.pause();
}

function pause(): void {
  if (audio && !audio.paused) audio.pause();
}

export function next(): void {
  if (mplayer.index < mplayer.queue.length - 1) void start(mplayer.index + 1);
  else if (mplayer.repeat === "all" && mplayer.queue.length) void start(0);
  else
    void extendIfLow(true).then((added) => {
      if (added) void start(mplayer.index + 1);
    });
}

export function prev(): void {
  if (mplayer.position > 3 || mplayer.index <= 0) {
    seek(0);
    return;
  }
  void start(mplayer.index - 1);
}

export function seek(s: number): void {
  const d = mplayer.duration;
  const t = Math.max(0, d > 0 ? Math.min(d - 0.25, s) : s);
  mplayer.position = t;
  const song = currentSong();
  if (!audio || !song) return;
  if (loadedId !== song.id) {
    void start(mplayer.index, t, !audio.paused);
    return;
  }
  audio.currentTime = t;
}

export function seekBy(d: number): void {
  seek(mplayer.position + d);
}

export function setVolume(v: number): void {
  mplayer.volume = Math.min(1, Math.max(0, v));
  if (mplayer.volume > 0) mplayer.muted = false;
  applyVolume();
  persist();
}

export function toggleMute(): void {
  mplayer.muted = !mplayer.muted;
  applyVolume();
  persist();
}

let original: SongItem[] | null = null;

function shuffleAhead(from: number) {
  original = [...mplayer.queue];
  const cur = mplayer.queue[from];
  const rest = mplayer.queue.filter((_, i) => i !== from);
  for (let i = rest.length - 1; i > 0; i--) {
    const j = Math.floor(Math.random() * (i + 1));
    [rest[i], rest[j]] = [rest[j], rest[i]];
  }
  mplayer.queue = cur ? [cur, ...rest] : rest;
  mplayer.index = cur ? 0 : -1;
}

export function toggleShuffle(): void {
  mplayer.shuffle = !mplayer.shuffle;
  const cur = currentSong();
  if (mplayer.shuffle) {
    if (mplayer.index >= 0) {
      const head = mplayer.queue.slice(0, mplayer.index + 1);
      const tail = mplayer.queue.slice(mplayer.index + 1);
      original = [...mplayer.queue];
      for (let i = tail.length - 1; i > 0; i--) {
        const j = Math.floor(Math.random() * (i + 1));
        [tail[i], tail[j]] = [tail[j], tail[i]];
      }
      mplayer.queue = [...head, ...tail];
    }
  } else if (original) {
    const known = new Set(original.map((s) => s.id));
    const extra = mplayer.queue.filter((s) => !known.has(s.id));
    mplayer.queue = [...original, ...extra];
    mplayer.index = cur ? Math.max(0, mplayer.queue.findIndex((s) => s.id === cur.id)) : -1;
    original = null;
  }
  persist();
  void prefetchNext();
}

export function cycleRepeat(): void {
  mplayer.repeat = mplayer.repeat === "off" ? "all" : mplayer.repeat === "all" ? "one" : "off";
  persist();
}

export function removeAt(i: number): void {
  if (i < 0 || i >= mplayer.queue.length) return;
  if (i === mplayer.index) {
    mplayer.queue.splice(i, 1);
    if (!mplayer.queue.length) {
      stopMusic();
      return;
    }
    void start(Math.min(i, mplayer.queue.length - 1), 0, mplayer.playing);
    return;
  }
  mplayer.queue.splice(i, 1);
  if (i < mplayer.index) mplayer.index--;
  persist();
}

export function moveInQueue(from: number, to: number): void {
  const q = mplayer.queue;
  if (from === to || from < 0 || to < 0 || from >= q.length || to >= q.length) return;
  const [s] = q.splice(from, 1);
  q.splice(to, 0, s);
  if (mplayer.index === from) mplayer.index = to;
  else if (from < mplayer.index && to >= mplayer.index) mplayer.index--;
  else if (from > mplayer.index && to <= mplayer.index) mplayer.index++;
  persist();
  void prefetchNext();
}

export function playAt(i: number): void {
  if (i >= 0 && i < mplayer.queue.length) void start(i);
}

export function stopMusic(): void {
  report();
  loadSeq++;
  if (audio) {
    audio.pause();
    audio.removeAttribute("src");
    audio.load();
  }
  loadedId = null;
  mplayer.queue = [];
  mplayer.index = -1;
  mplayer.playing = false;
  mplayer.position = 0;
  mplayer.duration = 0;
  mplayer.radio = false;
  mplayer.context = "";
  mplayer.continuation = null;
  mplayer.error = null;
  persist();
  setMeta("music", null, null);
}

export async function rateSong(song: SongItem, like: boolean | null): Promise<void> {
  const before = song.liked ?? null;
  song.liked = like;
  const cur = mplayer.queue.find((s) => s.id === song.id);
  if (cur && cur !== song) cur.liked = like;
  try {
    await api.action(like === true ? "music_like" : like === false ? "music_dislike" : "music_remove_like", { video_id: song.id });
    persist();
  } catch (e) {
    song.liked = before;
    if (cur && cur !== song) cur.liked = before;
    throw e;
  }
}

const lyricsCache = new Map<string, Lyrics | null>();

export async function lyricsFor(song: SongItem): Promise<Lyrics | null> {
  if (lyricsCache.has(song.id)) return lyricsCache.get(song.id) ?? null;
  const l = await api.musicLyrics(song.id, song.title, song.artists.map((a) => a.name).join(", "), song.duration_s ?? Math.round(mplayer.duration));
  lyricsCache.set(song.id, l);
  if (lyricsCache.size > 50) lyricsCache.delete(lyricsCache.keys().next().value as string);
  return l;
}

function artistsOf(s: SongItem): string {
  return s.artists.map((a) => a.name).join(", ");
}

let lastReported = "";
function report() {
  const s = currentSong();
  if (!s || !audio || mplayer.position < 5) return;
  const key = `${s.id}:${Math.round(mplayer.position / 10)}`;
  if (key === lastReported) return;
  lastReported = key;
  void api
    .progress(
      {
        video_id: s.id,
        title: s.title,
        channel: s.artists[0] ?? null,
        thumb: thumbUrl(s.thumbs, 120) || null,
        duration_s: s.duration_s ?? Math.round(mplayer.duration),
        position_s: Math.floor(mplayer.position),
        watched_ms: Date.now(),
        kind: "song",
      },
      tubePrefs().send_history,
    )
    .catch(() => {});
  if (!mplayer.playing && reportTimer) {
    clearInterval(reportTimer);
    reportTimer = null;
  }
}

function updateMeta() {
  const s = currentSong();
  if (!s) return;
  setMeta(
    "music",
    { title: s.title, artist: artistsOf(s), album: s.album?.name ?? null, artwork: thumbUrl(s.thumbs, 512) || null },
    { play: toggle, pause, next, prev, seek, seekBy },
  );
}

register("music", pause, (mine) => (mplayer.active = mine));

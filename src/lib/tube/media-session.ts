import { pause as pauseLocalMusic, player as localMusic } from "$lib/music/player-store.svelte";

export type Owner = "video" | "music";

export type SessionMeta = {
  title: string;
  artist?: string | null;
  album?: string | null;
  artwork?: string | null;
};

type SessionHandlers = {
  play: () => void;
  pause: () => void;
  next?: (() => void) | null;
  prev?: (() => void) | null;
  seek: (s: number) => void;
  seekBy: (d: number) => void;
};

const session = { owner: null as Owner | null };
const yielders = new Map<Owner, () => void>();
const claimers = new Map<Owner, (mine: boolean) => void>();

export function register(owner: Owner, pause: () => void, active: (mine: boolean) => void) {
  yielders.set(owner, pause);
  claimers.set(owner, active);
}

export function claim(owner: Owner) {
  for (const [o, fn] of yielders) if (o !== owner) fn();
  if (localMusic.playing) pauseLocalMusic();
  session.owner = owner;
  for (const [o, fn] of claimers) fn(o === owner);
}

export function yieldAll() {
  for (const fn of yielders.values()) fn();
}

function ms(): MediaSession | null {
  return typeof navigator !== "undefined" && "mediaSession" in navigator ? navigator.mediaSession : null;
}

export function setMeta(owner: Owner, meta: SessionMeta | null, h: SessionHandlers | null) {
  const m = ms();
  if (!m || (session.owner && session.owner !== owner)) return;
  try {
    m.metadata = meta
      ? new MediaMetadata({
          title: meta.title,
          artist: meta.artist ?? "",
          album: meta.album ?? "",
          artwork: meta.artwork ? [{ src: meta.artwork, sizes: "512x512" }] : [],
        })
      : null;
    const set = (a: MediaSessionAction, fn: MediaSessionActionHandler | null) => {
      try {
        m.setActionHandler(a, fn);
      } catch {
      }
    };
    set("play", h ? () => h.play() : null);
    set("pause", h ? () => h.pause() : null);
    set("nexttrack", h?.next ? () => h.next?.() : null);
    set("previoustrack", h?.prev ? () => h.prev?.() : null);
    set("seekto", h ? (d) => d.seekTime != null && h.seek(d.seekTime) : null);
    set("seekbackward", h ? (d) => h.seekBy(-(d.seekOffset ?? 10)) : null);
    set("seekforward", h ? (d) => h.seekBy(d.seekOffset ?? 10) : null);
  } catch {
  }
}

export function setPlaybackState(owner: Owner, playing: boolean, position: number, duration: number, rate = 1) {
  const m = ms();
  if (!m || session.owner !== owner) return;
  try {
    m.playbackState = playing ? "playing" : "paused";
    if (duration > 0 && Number.isFinite(duration)) {
      m.setPositionState({ duration, position: Math.min(Math.max(0, position), duration), playbackRate: rate || 1 });
    }
  } catch {
  }
}

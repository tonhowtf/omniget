import { get } from "svelte/store";
import { t } from "$lib/i18n";
import { convertFileSrc } from "@tauri-apps/api/core";
import { pluginInvoke } from "$lib/plugin-invoke";
import { dominantColorFromPath, palettePathOrUrl, type RGB } from "./dominant-color";
import { spotifySdk, type SpotifySdkState } from "./spotify-sdk.svelte";
import { attachHls, type HlsHandle } from "./hls-loader.svelte";

export type TrackSource = "local" | "spotify" | "youtube" | "soundcloud";

export type PlayerBroadcastEvent =
  | { type: "track-change"; track: MusicTrack }
  | { type: "play" }
  | { type: "pause"; time: number }
  | { type: "seek"; time: number };

export type MusicTrack = {
  id: number;
  path: string;
  title: string | null;
  artist: string | null;
  album: string | null;
  album_artist?: string | null;
  track_number?: number | null;
  disc_number?: number | null;
  year?: number | null;
  genre?: string | null;
  duration_ms: number | null;
  cover_path: string | null;
  file_size?: number;
  sample_rate?: number | null;
  bitrate?: number | null;
  added_at?: number;
  last_played_at?: number | null;
  play_count?: number;
  favorite?: boolean;
  source?: TrackSource;
  spotify_uri?: string;
  spotify_cover_url?: string;
  youtube_url?: string;
  youtube_video_id?: string;
  youtube_thumbnail?: string;
  isrc?: string;
  soundcloud_id?: number;
  soundcloud_artwork_url?: string;
};

export type RepeatMode = "off" | "all" | "one";

export type PlayerStatus = "idle" | "resolving" | "playing" | "paused" | "error";

export type PlayerErrorKind =
  | "network"
  | "auth_expired"
  | "track_unavailable"
  | "region_blocked"
  | "unsupported_format"
  | "format_unavailable"
  | "unknown";

export type PlayerErrorAction = "retry" | "refresh" | "skip";

export type PlayerError = {
  kind: PlayerErrorKind;
  message: string;
  source: TrackSource;
  trackId: number;
  retriable: boolean;
  primaryAction: PlayerErrorAction;
};

const TERMINAL_KINDS: ReadonlySet<PlayerErrorKind> = new Set([
  "track_unavailable",
  "region_blocked",
  "unsupported_format",
  "format_unavailable",
]);

function classifyPlayerError(
  raw: unknown,
  source: TrackSource,
  trackId: number,
  hint?: { audioCode?: number },
): PlayerError {
  const baseMsg = raw instanceof Error
    ? raw.message
    : typeof raw === "string"
      ? raw
      : raw == null
        ? ""
        : String(raw);
  const lower = baseMsg.toLowerCase();
  let kind: PlayerErrorKind = "unknown";

  if (hint?.audioCode === 4) {
    kind = "unsupported_format";
  } else if (hint?.audioCode === 2) {
    kind = "network";
  } else if (lower.includes("region") || lower.includes("country") || lower.includes("geographic")) {
    kind = "region_blocked";
  } else if (
    lower.includes("removed") ||
    lower.includes("private") ||
    lower.includes("video not found") ||
    lower.includes("track not found")
  ) {
    kind = "track_unavailable";
  } else if (
    lower.includes("requested format is not available") ||
    lower.includes("requested format") ||
    lower.includes("no audio formats") ||
    lower.includes("no formats found") ||
    lower.includes("sign in to confirm") ||
    lower.includes("sign in") ||
    lower.includes("potoken") ||
    lower.includes("unplayable") ||
    lower.includes("playabilitystatus") ||
    lower.includes("video unavailable") ||
    lower.includes("not available") ||
    lower.includes("unavailable")
  ) {
    kind = "format_unavailable";
  } else if (
    lower.includes("401") ||
    lower.includes("403") ||
    lower.includes("unauthorized") ||
    lower.includes("forbidden") ||
    lower.includes("expired") ||
    lower.includes("token") ||
    lower.includes("login")
  ) {
    kind = "auth_expired";
  } else if (
    lower.includes("cipher") ||
    lower.includes("no audio with direct url") ||
    lower.includes("formato") ||
    lower.includes("decod")
  ) {
    kind = "unsupported_format";
  } else if (
    lower.includes("network") ||
    lower.includes("fetch") ||
    lower.includes("timeout") ||
    lower.includes("connection") ||
    lower.includes("connect") ||
    lower.includes("dns") ||
    lower.includes("reset") ||
    lower.includes("rede")
  ) {
    kind = "network";
  } else if (/\b5\d{2}\b/.test(lower)) {
    kind = "network";
  } else if (/\b4\d{2}\b/.test(lower)) {
    kind = "track_unavailable";
  }

  const terminal = TERMINAL_KINDS.has(kind);
  const retriable = !terminal;
  let primaryAction: PlayerErrorAction = "skip";
  if (kind === "auth_expired") primaryAction = "refresh";
  else if (retriable) primaryAction = "retry";

  console.info("[player] classify error", {
    kind,
    source,
    trackId,
    snippet: baseMsg.slice(0, 200),
  });

  return {
    kind,
    message: baseMsg,
    source,
    trackId,
    retriable,
    primaryAction,
  };
}

export type EqPreset =
  | "flat"
  | "bass"
  | "vocal"
  | "treble"
  | "rock"
  | "pop"
  | "classical"
  | "electronic"
  | "custom";

export const EQ_BAND_FREQUENCIES = [60, 250, 1000, 4000, 12000] as const;

export const EQ_PRESETS: Record<EqPreset, number[]> = {
  flat: [0, 0, 0, 0, 0],
  bass: [6, 4, 0, -2, -3],
  vocal: [-2, -1, 4, 3, 0],
  treble: [-3, -2, 0, 4, 6],
  rock: [4, 2, -2, 3, 5],
  pop: [-1, 2, 4, 2, -1],
  classical: [3, 2, 0, 2, 4],
  electronic: [5, 3, -1, 2, 4],
  custom: [0, 0, 0, 0, 0],
};

const PERSIST_KEY = "study.music.player.v1";
const QUEUE_KEY = "study.music.queue.v1";

type YoutubePlayerLike = {
  formats: Array<{
    url: string | null;
    mime_type: string;
    bitrate: number;
    audio_quality: string | null;
    width?: number | null;
    height?: number | null;
  }>;
  adaptive_formats: Array<{
    url: string | null;
    mime_type: string;
    bitrate: number;
    audio_quality: string | null;
    width?: number | null;
    height?: number | null;
  }>;
};

function pickBestYoutubeAudio(res: YoutubePlayerLike): string | null {
  const audioOnly = res.adaptive_formats.filter(
    (f) => f.url && f.mime_type.startsWith("audio/"),
  );
  audioOnly.sort((a, b) => (b.bitrate ?? 0) - (a.bitrate ?? 0));
  if (audioOnly[0]?.url) return audioOnly[0].url;
  const muxed = res.formats.filter((f) => f.url);
  muxed.sort((a, b) => (b.bitrate ?? 0) - (a.bitrate ?? 0));
  return muxed[0]?.url ?? null;
}

export function pickBestYoutubeVideo(res: YoutubePlayerLike): string | null {
  const muxed = res.formats.filter(
    (f) => f.url && f.mime_type.startsWith("video/"),
  );
  muxed.sort((a, b) => {
    const heightDelta = (b.height ?? 0) - (a.height ?? 0);
    if (heightDelta !== 0) return heightDelta;
    return (b.bitrate ?? 0) - (a.bitrate ?? 0);
  });
  const capped = muxed.find((f) => (f.height ?? 0) <= 720 && (f.height ?? 0) > 0);
  return capped?.url ?? muxed[0]?.url ?? null;
}

class MusicPlayerStore {
  private audio: HTMLAudioElement | null = null;
  private playStartedAt = 0;
  private accumulatedPlayedMs = 0;
  private lastReportTrackId: number | null = null;
  private queueSaveTimer: ReturnType<typeof setTimeout> | null = null;
  private eqContext: AudioContext | null = null;
  private eqSource: MediaElementAudioSourceNode | null = null;
  private eqFilters: BiquadFilterNode[] = [];
  private eqGainNode: GainNode | null = null;
  private youtubeRefreshTimer: ReturnType<typeof setTimeout> | null = null;
  private youtubeCurrentVideoId: string | null = null;
  private historyRecorded: Set<string> = new Set();
  private youtubeRefreshFailureCount: number = 0;
  private static YOUTUBE_REFRESH_MAX_FAILURES = 3;
  private static YOUTUBE_REFRESH_RETRY_DELAY_SECS = 30;

  private _spotifyLastKnownPositionMs = 0;
  private _spotifyLastEventAt = 0;
  private _spotifyIsPlaying = false;
  private _spotifyPlaybackRate = 1;
  private _spotifyRafHandle: number | null = null;
  private _spotifyHeartbeatHandle: ReturnType<typeof setInterval> | null = null;
  private _spotifyHeartbeatEnabled = true;
  private _spotifyHeartbeatMs = 30_000;
  private _spotifyVisibilityListener: (() => void) | null = null;
  private _spotifySettingsLoaded = false;
  private static SPOTIFY_SYNC_THRESHOLD_MS = 250;
  private static SPOTIFY_HEARTBEAT_MIN_MS = 5_000;
  private static SPOTIFY_HEARTBEAT_MAX_MS = 300_000;

  private _rpcShowCover = true;
  private _rpcShowButtons = true;
  private _rpcUpdateIntervalMs = 5_000;
  private _rpcTimer: ReturnType<typeof setInterval> | null = null;
  private _rpcSettingsLoaded = false;
  private static RPC_UPDATE_INTERVAL_MIN_MS = 2_000;
  private static RPC_UPDATE_INTERVAL_MAX_MS = 30_000;
  private static RPC_UPDATE_INTERVAL_DEFAULT_MS = 5_000;

  currentTrack = $state<MusicTrack | null>(null);
  queue = $state<MusicTrack[]>([]);
  queueIndex = $state(0);
  paused = $state(true);
  currentTime = $state(0);
  duration = $state(0);
  volume = $state(1);
  muted = $state(false);
  shuffle = $state(false);
  repeat = $state<RepeatMode>("off");
  ready = $state(false);
  loading = $state(false);
  error = $state<string | null>(null);
  status = $state<PlayerStatus>("idle");
  lastError = $state<PlayerError | null>(null);
  autoSkipOnError = $state(false);
  private playerSettingsLoaded = false;
  eqEnabled = $state(false);
  eqGains = $state<number[]>([0, 0, 0, 0, 0]);
  eqPreset = $state<EqPreset>("flat");
  dominantColor = $state<RGB | null>(null);
  palette = $state<RGB[] | null>(null);
  private paletteRefreshToken = 0;
  youtubeVideoUrl = $state<string | null>(null);
  youtubeChapters = $state<import("$lib/study-bridge").YoutubeChapter[]>([]);
  youtubeSponsorBlockSegments = $state<import("$lib/study-bridge").SponsorBlockSegment[]>([]);
  private mediaSessionInstalled = false;

  private broadcastListeners = new Set<(ev: PlayerBroadcastEvent) => void>();

  addBroadcastListener(cb: (ev: PlayerBroadcastEvent) => void): () => void {
    this.broadcastListeners.add(cb);
    return () => {
      this.broadcastListeners.delete(cb);
    };
  }

  private fireBroadcast(ev: PlayerBroadcastEvent) {
    for (const cb of this.broadcastListeners) {
      try {
        cb(ev);
      } catch {
        /* listener errors must not break playback */
      }
    }
  }

  sleepTimerEndsAt = $state<number | null>(null);
  sleepTimerEndOfTrack = $state(false);
  private sleepFadeStarted = false;
  private sleepBaseVolume = 1;
  crossfadeSec = $state(0);
  private crossfadeStarted = false;
  private crossfadeBaseVolume = 1;
  normalizationEnabled = $state(false);
  private normalizationGain: GainNode | null = null;
  skipSilenceEnabled = $state(false);
  private silenceAnalyser: AnalyserNode | null = null;
  private silenceCheckHandle: ReturnType<typeof setInterval> | null = null;
  private silenceStartedAt: number | null = null;

  init() {
    if (this.audio || typeof window === "undefined") return;
    const audio = new Audio();
    audio.preload = "metadata";
    audio.crossOrigin = "anonymous";

    audio.addEventListener("play", () => {
      this.paused = false;
      this.status = "playing";
      this.playStartedAt = Date.now();
      if (typeof navigator !== "undefined" && "mediaSession" in navigator) {
        navigator.mediaSession.playbackState = "playing";
      }
      this.rpcOnPlaybackChange();
    });
    audio.addEventListener("pause", () => {
      this.paused = true;
      if (this.status !== "error") this.status = "paused";
      if (this.playStartedAt > 0) {
        this.accumulatedPlayedMs += Date.now() - this.playStartedAt;
        this.playStartedAt = 0;
      }
      this.scheduleQueueSave();
      if (typeof navigator !== "undefined" && "mediaSession" in navigator) {
        navigator.mediaSession.playbackState = "paused";
      }
      this.rpcOnPlaybackChange();
    });
    audio.addEventListener("timeupdate", () => {
      this.currentTime = audio.currentTime;
      this.scheduleQueueSave();
      this.updateMediaSessionPosition();
      this.tickSleepTimer();
      this.tickCrossfade();
    });
    audio.addEventListener("loadedmetadata", () => {
      this.duration = audio.duration || 0;
      this.ready = true;
      this.loading = false;
      this.updateMediaSessionPosition();
      this.applyNormalizationGain();
    });
    audio.addEventListener("loadstart", () => {
      this.loading = true;
      this.ready = false;
    });
    audio.addEventListener("canplay", () => {
      this.loading = false;
    });
    audio.addEventListener("ended", () => {
      this.handleEnded();
    });
    audio.addEventListener("error", () => {
      const err = audio.error;
      let msg = get(t)("study.music.store.load_failed");
      let code: number | undefined;
      if (err) {
        code = err.code;
        const codeMap: Record<number, string> = {
          1: get(t)("study.music.store.aborted"),
          2: get(t)("study.music.store.network_failed"),
          3: get(t)("study.music.store.decode_failed"),
          4: get(t)("study.music.store.format_unsupported"),
        };
        const reason = codeMap[err.code] ?? `code ${err.code}`;
        msg = `${reason}${err.message ? ` (${err.message})` : ""}`;
        console.error(
          "[player] audio error",
          { code: err.code, src: audio.src.slice(0, 80) },
        );
      }
      const track = this.currentTrack;
      if (track) {
        this.setError(
          classifyPlayerError(msg, track.source ?? "local", track.id, { audioCode: code }),
        );
      } else {
        this.error = msg;
      }
      this.loading = false;
    });
    audio.addEventListener("volumechange", () => {
      this.volume = audio.volume;
      this.muted = audio.muted;
    });

    try {
      const raw = localStorage.getItem(PERSIST_KEY);
      if (raw) {
        const data = JSON.parse(raw) as {
          volume?: number;
          shuffle?: boolean;
          repeat?: RepeatMode;
          muted?: boolean;
          eqEnabled?: boolean;
          eqGains?: number[];
          eqPreset?: EqPreset;
          crossfadeSec?: number;
          sleepEndOfTrack?: boolean;
          normalizationEnabled?: boolean;
          skipSilenceEnabled?: boolean;
        };
        if (typeof data.volume === "number") {
          audio.volume = Math.min(1, Math.max(0, data.volume));
        }
        if (typeof data.shuffle === "boolean") this.shuffle = data.shuffle;
        if (data.repeat === "off" || data.repeat === "all" || data.repeat === "one") {
          this.repeat = data.repeat;
        }
        if (typeof data.muted === "boolean") audio.muted = data.muted;
        if (typeof data.eqEnabled === "boolean") this.eqEnabled = data.eqEnabled;
        if (Array.isArray(data.eqGains) && data.eqGains.length === 5) {
          this.eqGains = data.eqGains.map((g) =>
            Math.max(-12, Math.min(12, Number(g) || 0)),
          );
        }
        if (data.eqPreset && data.eqPreset in EQ_PRESETS) {
          this.eqPreset = data.eqPreset;
        }
        if (typeof data.crossfadeSec === "number") {
          this.crossfadeSec = Math.max(0, Math.min(12, data.crossfadeSec));
        }
        if (typeof data.sleepEndOfTrack === "boolean") {
          this.sleepTimerEndOfTrack = data.sleepEndOfTrack;
        }
        if (typeof data.normalizationEnabled === "boolean") {
          this.normalizationEnabled = data.normalizationEnabled;
        }
        if (typeof data.skipSilenceEnabled === "boolean") {
          this.skipSilenceEnabled = data.skipSilenceEnabled;
        }
      }
    } catch {
      /* ignore */
    }

    this.audio = audio;
    this.volume = audio.volume;
    this.muted = audio.muted;
    this.installMediaSession();
    this.restoreQueue();
    this.installSpotifySync();
    void this.loadPlayerSettings();
    void this.loadSpotifySettings();
    void this.loadRpcSettings();
    if (this.queue.some((t) => t.source === "spotify" && t.spotify_uri)) {
      void spotifySdk.ensureLoaded().catch(() => {
        /* prewarm best-effort */
      });
    }
  }

  private async loadPlayerSettings() {
    if (this.playerSettingsLoaded) return;
    this.playerSettingsLoaded = true;
    try {
      const res = await pluginInvoke<{
        auto_skip_on_error?: boolean;
      }>("study", "study:music:player:settings:get", {});
      this.autoSkipOnError = !!res?.auto_skip_on_error;
    } catch (e) {
      console.warn("[player] failed loading player settings", e);
    }
  }

  async setAutoSkipOnError(value: boolean) {
    this.autoSkipOnError = value;
    try {
      await pluginInvoke("study", "study:music:player:settings:set", {
        auto_skip_on_error: value,
      });
    } catch (e) {
      console.warn("[player] failed saving auto_skip_on_error", e);
    }
  }

  private setError(err: PlayerError) {
    this.lastError = err;
    this.error = err.message || null;
    this.status = "error";
    this.loading = false;
    if (this.autoSkipOnError && !err.retriable) {
      const skipDelay = 300;
      setTimeout(() => {
        if (this.lastError && this.lastError.trackId === err.trackId) {
          this.skipOnError();
        }
      }, skipDelay);
    }
  }

  clearError() {
    this.lastError = null;
    this.error = null;
    if (this.status === "error") {
      this.status = this.paused ? "paused" : "idle";
    }
  }

  retry() {
    const track = this.currentTrack;
    if (!track) return;
    this.clearError();
    void this.play(track, this.queue.length > 0 ? this.queue : undefined);
  }

  async refreshAndRetry() {
    const track = this.currentTrack;
    if (!track) return;
    this.clearError();
    this.status = "resolving";
    this.loading = true;
    if (track.source === "youtube" && track.youtube_video_id) {
      try {
        await pluginInvoke("study", "study:music:track:refresh_url", {
          source: "youtube_music",
          external_id: track.youtube_video_id,
        });
      } catch (e) {
        console.warn("[player] refresh_url failed", e);
      }
    }
    void this.play(track, this.queue.length > 0 ? this.queue : undefined);
  }

  skipOnError() {
    this.clearError();
    this.next();
  }

  private spotifyUnsub: (() => void) | null = null;
  private installSpotifySync() {
    if (this.spotifyUnsub) return;
    this.spotifyUnsub = spotifySdk.onState((state) => this.applySpotifyState(state));
    if (typeof document !== "undefined" && !this._spotifyVisibilityListener) {
      const listener = () => {
        if (document.visibilityState === "visible" && this.isSpotify()) {
          void this.spotifyResyncFromSdk();
        }
      };
      this._spotifyVisibilityListener = listener;
      document.addEventListener("visibilitychange", listener);
    }
  }

  private applySpotifyState(state: SpotifySdkState | null) {
    if (this.currentTrack?.source !== "spotify") return;
    if (!state) return;

    const sdkUri = state.track_window?.current_track?.uri;
    const isTrackChange =
      !!sdkUri &&
      !!this.currentTrack.spotify_uri &&
      sdkUri !== this.currentTrack.spotify_uri;

    const extrapolatedMs = this._spotifyIsPlaying
      ? this._spotifyLastKnownPositionMs +
        (performance.now() - this._spotifyLastEventAt) * this._spotifyPlaybackRate
      : this._spotifyLastKnownPositionMs;
    const drift = Math.abs(state.position - extrapolatedMs);
    const shouldSync =
      isTrackChange ||
      drift > MusicPlayerStore.SPOTIFY_SYNC_THRESHOLD_MS ||
      this._spotifyIsPlaying === state.paused;

    this._spotifyLastKnownPositionMs = state.position;
    this._spotifyLastEventAt = performance.now();
    const stateRate = (state as unknown as { playback_rate?: number }).playback_rate;
    this._spotifyPlaybackRate =
      typeof stateRate === "number" && stateRate > 0 ? stateRate : 1;
    const wasPlaying = this._spotifyIsPlaying;
    this._spotifyIsPlaying = !state.paused;

    this.paused = state.paused;
    if (state.paused) {
      if (this.status !== "error") this.status = "paused";
    } else {
      this.status = "playing";
    }
    if (state.duration) {
      this.duration = state.duration / 1000;
    } else if (this.currentTrack.duration_ms) {
      this.duration = this.currentTrack.duration_ms / 1000;
    }
    this.ready = true;
    this.loading = false;

    if (shouldSync) {
      const clamped =
        this.duration > 0
          ? Math.min(state.position / 1000, this.duration)
          : state.position / 1000;
      this.currentTime = clamped;
      this.updateMediaSessionPosition();
    }

    if (this._spotifyIsPlaying && (!wasPlaying || !this._spotifyRafHandle)) {
      this.ensureSpotifyTick();
    } else if (!this._spotifyIsPlaying && wasPlaying) {
      this.stopSpotifyTick();
    }

    if (isTrackChange) {
      const inQueue = this.queue.find((t) => t.spotify_uri === sdkUri);
      if (inQueue) {
        this.currentTrack = inQueue;
        this.queueIndex = this.queue.indexOf(inQueue);
        this.updateMediaSessionMetadata(inQueue);
      }
    }

    this.rpcOnPlaybackChange();
  }

  private ensureSpotifyTick() {
    if (typeof window === "undefined") return;
    if (!this.isSpotify()) return;
    if (!this._spotifyIsPlaying) return;
    if (this._spotifyRafHandle != null) return;
    const tick = () => {
      if (!this.isSpotify() || !this._spotifyIsPlaying) {
        this._spotifyRafHandle = null;
        return;
      }
      const elapsed = performance.now() - this._spotifyLastEventAt;
      const positionMs =
        this._spotifyLastKnownPositionMs + elapsed * this._spotifyPlaybackRate;
      const seconds = positionMs / 1000;
      if (this.duration > 0 && seconds >= this.duration) {
        this.currentTime = this.duration;
      } else if (seconds >= 0) {
        this.currentTime = seconds;
      }
      this.updateMediaSessionPosition();
      this._spotifyRafHandle = window.requestAnimationFrame(tick);
    };
    this._spotifyRafHandle = window.requestAnimationFrame(tick);
  }

  private stopSpotifyTick() {
    if (this._spotifyRafHandle != null && typeof window !== "undefined") {
      window.cancelAnimationFrame(this._spotifyRafHandle);
    }
    this._spotifyRafHandle = null;
  }

  private startSpotifyHeartbeat() {
    if (this._spotifyHeartbeatHandle) return;
    if (!this._spotifyHeartbeatEnabled) return;
    if (typeof window === "undefined") return;
    const intervalMs = Math.min(
      MusicPlayerStore.SPOTIFY_HEARTBEAT_MAX_MS,
      Math.max(MusicPlayerStore.SPOTIFY_HEARTBEAT_MIN_MS, this._spotifyHeartbeatMs),
    );
    this._spotifyHeartbeatHandle = setInterval(() => {
      if (!this.isSpotify()) return;
      void this.spotifyResyncFromSdk();
    }, intervalMs);
  }

  private stopSpotifyHeartbeat() {
    if (this._spotifyHeartbeatHandle) {
      clearInterval(this._spotifyHeartbeatHandle);
      this._spotifyHeartbeatHandle = null;
    }
  }

  private async spotifyResyncFromSdk() {
    try {
      const state = await spotifySdk.getCurrentState();
      if (state) this.applySpotifyState(state);
    } catch (e) {
      console.warn("[player] spotify heartbeat resync failed", e);
    }
  }

  private async loadSpotifySettings() {
    if (this._spotifySettingsLoaded) return;
    this._spotifySettingsLoaded = true;
    try {
      const res = await pluginInvoke<{
        heartbeat_enabled?: boolean;
        heartbeat_interval_ms?: number;
      }>("study", "study:music:spotify:settings:get", {});
      this._spotifyHeartbeatEnabled = res?.heartbeat_enabled ?? true;
      const ms = Number(res?.heartbeat_interval_ms ?? 30_000);
      this._spotifyHeartbeatMs = Number.isFinite(ms)
        ? Math.min(
            MusicPlayerStore.SPOTIFY_HEARTBEAT_MAX_MS,
            Math.max(MusicPlayerStore.SPOTIFY_HEARTBEAT_MIN_MS, ms),
          )
        : 30_000;
    } catch (e) {
      console.warn("[player] failed loading spotify settings", e);
    }
  }

  async setSpotifyHeartbeatEnabled(value: boolean) {
    this._spotifyHeartbeatEnabled = value;
    try {
      await pluginInvoke("study", "study:music:spotify:settings:set", {
        heartbeat_enabled: value,
      });
    } catch (e) {
      console.warn("[player] failed saving spotify heartbeat_enabled", e);
    }
    if (value && this.isSpotify() && this._spotifyIsPlaying) {
      this.startSpotifyHeartbeat();
    } else if (!value) {
      this.stopSpotifyHeartbeat();
    }
  }

  async setSpotifyHeartbeatIntervalMs(ms: number) {
    const clamped = Math.min(
      MusicPlayerStore.SPOTIFY_HEARTBEAT_MAX_MS,
      Math.max(
        MusicPlayerStore.SPOTIFY_HEARTBEAT_MIN_MS,
        Math.floor(Number(ms) || 30_000),
      ),
    );
    this._spotifyHeartbeatMs = clamped;
    try {
      await pluginInvoke("study", "study:music:spotify:settings:set", {
        heartbeat_interval_ms: clamped,
      });
    } catch (e) {
      console.warn("[player] failed saving spotify heartbeat_interval_ms", e);
    }
    if (this._spotifyHeartbeatHandle) {
      this.stopSpotifyHeartbeat();
      if (this.isSpotify() && this._spotifyIsPlaying) this.startSpotifyHeartbeat();
    }
  }

  getSpotifyHeartbeatEnabled(): boolean {
    return this._spotifyHeartbeatEnabled;
  }

  getSpotifyHeartbeatIntervalMs(): number {
    return this._spotifyHeartbeatMs;
  }

  private static clampRpcInterval(ms: number): number {
    return Math.min(
      MusicPlayerStore.RPC_UPDATE_INTERVAL_MAX_MS,
      Math.max(
        MusicPlayerStore.RPC_UPDATE_INTERVAL_MIN_MS,
        Math.floor(Number(ms) || MusicPlayerStore.RPC_UPDATE_INTERVAL_DEFAULT_MS),
      ),
    );
  }

  private async loadRpcSettings() {
    if (this._rpcSettingsLoaded) return;
    this._rpcSettingsLoaded = true;
    try {
      const res = await pluginInvoke<{
        show_cover?: boolean;
        show_buttons?: boolean;
        update_interval_ms?: number;
      }>("study", "study:music:rpc:music_settings:get", {});
      if (typeof res?.show_cover === "boolean") this._rpcShowCover = res.show_cover;
      if (typeof res?.show_buttons === "boolean") this._rpcShowButtons = res.show_buttons;
      if (typeof res?.update_interval_ms === "number") {
        this._rpcUpdateIntervalMs = MusicPlayerStore.clampRpcInterval(
          res.update_interval_ms,
        );
      }
    } catch (e) {
      console.warn("[player] failed loading rpc music_settings", e);
    }
  }

  getRpcShowCover(): boolean {
    return this._rpcShowCover;
  }

  getRpcShowButtons(): boolean {
    return this._rpcShowButtons;
  }

  getRpcUpdateIntervalMs(): number {
    return this._rpcUpdateIntervalMs;
  }

  async setRpcShowCover(value: boolean) {
    this._rpcShowCover = value;
    try {
      await pluginInvoke("study", "study:music:rpc:music_settings:set", {
        show_cover: value,
      });
    } catch (e) {
      console.warn("[player] failed saving rpc show_cover", e);
    }
    this.rpcOnPlaybackChange();
  }

  async setRpcShowButtons(value: boolean) {
    this._rpcShowButtons = value;
    try {
      await pluginInvoke("study", "study:music:rpc:music_settings:set", {
        show_buttons: value,
      });
    } catch (e) {
      console.warn("[player] failed saving rpc show_buttons", e);
    }
    this.rpcOnPlaybackChange();
  }

  async setRpcUpdateIntervalMs(ms: number) {
    const clamped = MusicPlayerStore.clampRpcInterval(ms);
    this._rpcUpdateIntervalMs = clamped;
    try {
      await pluginInvoke("study", "study:music:rpc:music_settings:set", {
        update_interval_ms: clamped,
      });
    } catch (e) {
      console.warn("[player] failed saving rpc update_interval_ms", e);
    }
    if (this._rpcTimer !== null) {
      this.stopRpcLoop();
      this.startRpcLoop();
    }
  }

  private rpcSnapshot(): {
    title: string;
    artist: string;
    album: string;
    duration: number;
    position: number;
    paused: boolean;
    large_image_key: string | null;
    large_image_text: string | null;
    small_image_key: string | null;
    small_image_text: string | null;
    primary_button_url: string | null;
  } | null {
    const track = this.currentTrack;
    if (!track) return null;
    const source: TrackSource = track.source ?? "local";
    const title = (track.title ?? "").trim() || get(t)("study.music.store.untitled");
    const artist = (track.artist ?? "").trim();
    const album = (track.album ?? "").trim();
    const duration = Math.max(
      0,
      Math.floor((track.duration_ms ?? this.duration * 1000) / 1000),
    );
    const position = Math.max(0, Math.floor(this.currentTime || 0));
    const paused = this.paused;

    let coverUrl: string | null = null;
    if (this._rpcShowCover) {
      if (source === "spotify" && track.spotify_cover_url) {
        coverUrl = track.spotify_cover_url;
      } else if (source === "youtube" && track.youtube_thumbnail) {
        coverUrl = track.youtube_thumbnail;
      } else if (source === "soundcloud" && track.soundcloud_artwork_url) {
        coverUrl = track.soundcloud_artwork_url;
      }
    }

    let primaryButtonUrl: string | null = null;
    if (source === "youtube") {
      if (track.youtube_url) primaryButtonUrl = track.youtube_url;
      else if (track.youtube_video_id) {
        primaryButtonUrl = `https://www.youtube.com/watch?v=${track.youtube_video_id}`;
      }
    } else if (source === "spotify" && track.spotify_uri) {
      const m = /^spotify:track:([A-Za-z0-9]+)/.exec(track.spotify_uri);
      if (m) primaryButtonUrl = `https://open.spotify.com/track/${m[1]}`;
    }

    const smallImageKey =
      source === "youtube"
        ? "yt_logo"
        : source === "spotify"
          ? "spotify_logo"
          : source === "soundcloud"
            ? "soundcloud_logo"
            : null;
    const smallImageText =
      source === "youtube"
        ? "YouTube"
        : source === "spotify"
          ? "Spotify"
          : source === "soundcloud"
            ? "SoundCloud"
            : source === "local"
              ? "Biblioteca local"
              : null;

    return {
      title,
      artist,
      album,
      duration,
      position,
      paused,
      large_image_key: coverUrl,
      large_image_text: artist || "omniget · study",
      small_image_key: smallImageKey,
      small_image_text: smallImageText,
      primary_button_url: primaryButtonUrl,
    };
  }

  private async fireRpcUpdate() {
    const snap = this.rpcSnapshot();
    if (!snap) return;
    try {
      await pluginInvoke("study", "study:music:rpc:set", snap);
    } catch {
      /* ignore — discord may be offline */
    }
  }

  private startRpcLoop() {
    if (typeof window === "undefined") return;
    this.stopRpcLoop();
    this._rpcTimer = setInterval(() => {
      void this.fireRpcUpdate();
    }, this._rpcUpdateIntervalMs);
  }

  private stopRpcLoop() {
    if (this._rpcTimer !== null) {
      clearInterval(this._rpcTimer);
      this._rpcTimer = null;
    }
  }

  private rpcOnPlaybackChange() {
    if (!this.currentTrack) {
      this.stopRpcLoop();
      void pluginInvoke("study", "study:music:rpc:clear", {}).catch(() => {});
      return;
    }
    void this.fireRpcUpdate();
    if (this.paused) {
      this.stopRpcLoop();
    } else {
      this.startRpcLoop();
    }
  }

  private async fireRpcClear() {
    this.stopRpcLoop();
    try {
      await pluginInvoke("study", "study:music:rpc:clear", {});
    } catch {
      /* ignore */
    }
  }

  private isSpotify(): boolean {
    return this.currentTrack?.source === "spotify";
  }

  private installMediaSession() {
    if (
      this.mediaSessionInstalled ||
      typeof navigator === "undefined" ||
      !("mediaSession" in navigator)
    ) {
      return;
    }
    try {
      navigator.mediaSession.setActionHandler("play", () => this.resume());
      navigator.mediaSession.setActionHandler("pause", () => this.pause());
      navigator.mediaSession.setActionHandler("previoustrack", () => this.prev());
      navigator.mediaSession.setActionHandler("nexttrack", () => this.next());
      navigator.mediaSession.setActionHandler("seekbackward", (e) => {
        const off = e.seekOffset ?? 10;
        this.seek(Math.max(0, this.currentTime - off));
      });
      navigator.mediaSession.setActionHandler("seekforward", (e) => {
        const off = e.seekOffset ?? 10;
        this.seek(this.currentTime + off);
      });
      navigator.mediaSession.setActionHandler("seekto", (e) => {
        if (typeof e.seekTime === "number") this.seek(e.seekTime);
      });
      navigator.mediaSession.setActionHandler("stop", () => this.pause());
      this.mediaSessionInstalled = true;
    } catch {
      /* not all UAs support every action */
    }
  }

  private updateMediaSessionMetadata(track: MusicTrack | null) {
    if (typeof navigator === "undefined" || !("mediaSession" in navigator)) return;
    if (!track) {
      navigator.mediaSession.metadata = null;
      return;
    }
    let artwork: MediaImage[] = [];
    if (track.cover_path) {
      try {
        const url = convertFileSrc(track.cover_path);
        artwork = [
          { src: url, sizes: "512x512", type: "image/jpeg" },
        ];
      } catch {
        /* ignore */
      }
    }
    try {
      navigator.mediaSession.metadata = new MediaMetadata({
        title: track.title ?? track.path,
        artist: track.artist ?? "",
        album: track.album ?? "",
        artwork,
      });
    } catch {
      /* ignore */
    }
  }

  private updateMediaSessionPosition() {
    if (
      typeof navigator === "undefined" ||
      !("mediaSession" in navigator) ||
      !("setPositionState" in navigator.mediaSession)
    ) {
      return;
    }
    if (!this.duration || !Number.isFinite(this.duration)) return;
    try {
      navigator.mediaSession.setPositionState({
        duration: this.duration,
        position: Math.min(this.currentTime, this.duration),
        playbackRate: 1,
      });
    } catch {
      /* ignore */
    }
  }

  private async refreshDominantColor(track: MusicTrack | null) {
    if (!track) {
      this.dominantColor = null;
      this.palette = null;
      return;
    }
    const source =
      track.cover_path ??
      (track as { spotify_cover_url?: string | null }).spotify_cover_url ??
      (track as { youtube_thumbnail?: string | null }).youtube_thumbnail ??
      null;
    if (!source) {
      this.dominantColor = null;
      this.palette = null;
      return;
    }
    const token = ++this.paletteRefreshToken;
    const trackId = track.id;
    const [single, palette] = await Promise.all([
      track.cover_path
        ? dominantColorFromPath(track.cover_path)
        : Promise.resolve<RGB | null>(null),
      palettePathOrUrl(source),
    ]);
    if (token !== this.paletteRefreshToken) return;
    if (this.currentTrack?.id !== trackId) return;
    if (palette && palette.length > 0) {
      this.palette = palette;
      this.dominantColor = single ?? palette[0];
    } else if (single) {
      this.palette = [single];
      this.dominantColor = single;
    } else {
      this.palette = null;
      this.dominantColor = null;
    }
  }

  private restoreQueue() {
    if (!this.audio) return;
    try {
      const raw = localStorage.getItem(QUEUE_KEY);
      if (!raw) return;
      const data = JSON.parse(raw) as {
        queue?: MusicTrack[];
        queueIndex?: number;
        currentTime?: number;
      };
      if (!Array.isArray(data.queue) || data.queue.length === 0) return;
      const idx = Math.max(
        0,
        Math.min((data.queueIndex ?? 0) | 0, data.queue.length - 1),
      );
      const track = data.queue[idx];
      if (!track || typeof track.path !== "string") return;
      this.queue = data.queue;
      this.queueIndex = idx;
      this.currentTrack = track;
      this.duration = (track.duration_ms ?? 0) / 1000;
      this.audio.src = convertFileSrc(track.path);
      this.updateMediaSessionMetadata(track);
      void this.refreshDominantColor(track);
      const t = Number(data.currentTime);
      if (Number.isFinite(t) && t > 0) {
        const onLoaded = () => {
          if (this.audio) this.audio.currentTime = t;
          this.audio?.removeEventListener("loadedmetadata", onLoaded);
        };
        this.audio.addEventListener("loadedmetadata", onLoaded);
      }
    } catch {
      /* ignore */
    }
  }

  private persist() {
    try {
      localStorage.setItem(
        PERSIST_KEY,
        JSON.stringify({
          volume: this.volume,
          shuffle: this.shuffle,
          repeat: this.repeat,
          muted: this.muted,
          eqEnabled: this.eqEnabled,
          eqGains: this.eqGains,
          eqPreset: this.eqPreset,
        }),
      );
    } catch {
      /* ignore */
    }
  }

  private scheduleQueueSave() {
    if (this.queueSaveTimer) return;
    this.queueSaveTimer = setTimeout(() => {
      this.queueSaveTimer = null;
      this.saveQueueNow();
    }, 1500);
  }

  private saveQueueNow() {
    if (typeof localStorage === "undefined") return;
    try {
      if (this.queue.length === 0 || !this.currentTrack) {
        localStorage.removeItem(QUEUE_KEY);
        return;
      }
      localStorage.setItem(
        QUEUE_KEY,
        JSON.stringify({
          queue: this.queue,
          queueIndex: this.queueIndex,
          currentTime: this.currentTime,
        }),
      );
    } catch {
      /* ignore */
    }
  }

  private ensureEqGraph() {
    if (!this.audio || this.eqContext) return;
    if (typeof window === "undefined" || !window.AudioContext) return;
    try {
      const ctx = new AudioContext();
      const source = ctx.createMediaElementSource(this.audio);
      const filters = EQ_BAND_FREQUENCIES.map((freq) => {
        const f = ctx.createBiquadFilter();
        f.type = "peaking";
        f.frequency.value = freq;
        f.Q.value = 1;
        f.gain.value = 0;
        return f;
      });
      const out = ctx.createGain();
      out.gain.value = 1;
      const norm = ctx.createGain();
      norm.gain.value = 1;
      const analyser = ctx.createAnalyser();
      analyser.fftSize = 2048;
      analyser.smoothingTimeConstant = 0.85;
      let node: AudioNode = source;
      for (const f of filters) {
        node.connect(f);
        node = f;
      }
      node.connect(out);
      out.connect(norm);
      norm.connect(analyser);
      analyser.connect(ctx.destination);
      this.eqContext = ctx;
      this.eqSource = source;
      this.eqFilters = filters;
      this.eqGainNode = out;
      this.normalizationGain = norm;
      this.silenceAnalyser = analyser;
      this.applyEq();
      this.applyNormalizationGain();
      this.maybeStartSilenceWatcher();
    } catch {
      /* ignore — EQ optional */
    }
  }

  private applyNormalizationGain() {
    if (!this.normalizationGain) return;
    if (!this.normalizationEnabled) {
      this.normalizationGain.gain.value = 1;
      return;
    }
    const ld = (this.currentTrack as unknown as { loudness_db?: number } | null)
      ?.loudness_db;
    if (typeof ld === "number" && Number.isFinite(ld)) {
      const target = -14;
      const factor = Math.pow(10, (target - ld) / 20);
      this.normalizationGain.gain.value = Math.max(0.3, Math.min(3, factor));
    } else {
      this.normalizationGain.gain.value = 1;
    }
  }

  setNormalization(enabled: boolean) {
    this.normalizationEnabled = enabled;
    this.ensureEqGraph();
    this.applyNormalizationGain();
    this.persistPlayerSettings();
  }

  setSkipSilence(enabled: boolean) {
    this.skipSilenceEnabled = enabled;
    this.ensureEqGraph();
    this.maybeStartSilenceWatcher();
    this.persistPlayerSettings();
  }

  private maybeStartSilenceWatcher() {
    if (!this.skipSilenceEnabled || !this.silenceAnalyser) {
      if (this.silenceCheckHandle !== null) {
        clearInterval(this.silenceCheckHandle);
        this.silenceCheckHandle = null;
        this.silenceStartedAt = null;
      }
      return;
    }
    if (this.silenceCheckHandle !== null) return;
    const analyser = this.silenceAnalyser;
    const buf = new Float32Array(analyser.fftSize);
    const SILENCE_RMS_THRESHOLD = 0.01;
    const SILENCE_HOLD_MS = 1500;
    let lastSkipAt = 0;
    this.silenceCheckHandle = setInterval(() => {
      if (!this.audio || this.audio.paused) {
        this.silenceStartedAt = null;
        return;
      }
      analyser.getFloatTimeDomainData(buf);
      let sumSq = 0;
      for (let i = 0; i < buf.length; i++) sumSq += buf[i] * buf[i];
      const rms = Math.sqrt(sumSq / buf.length);
      const now = Date.now();
      if (rms < SILENCE_RMS_THRESHOLD) {
        if (this.silenceStartedAt === null) {
          this.silenceStartedAt = now;
        } else if (
          now - this.silenceStartedAt >= SILENCE_HOLD_MS &&
          now - lastSkipAt >= 10_000 &&
          this.audio.duration > 0 &&
          this.audio.currentTime + 3 < this.audio.duration
        ) {
          try {
            this.audio.currentTime = Math.min(
              this.audio.currentTime + 3,
              this.audio.duration - 1,
            );
            lastSkipAt = now;
            this.silenceStartedAt = null;
          } catch {
            /* ignore */
          }
        }
      } else {
        this.silenceStartedAt = null;
      }
    }, 300);
  }

  private applyEq() {
    if (this.eqFilters.length !== 5) return;
    const gains = this.eqEnabled ? this.eqGains : [0, 0, 0, 0, 0];
    for (let i = 0; i < 5; i++) {
      this.eqFilters[i].gain.value = gains[i] ?? 0;
    }
  }

  setEqEnabled(enabled: boolean) {
    this.eqEnabled = enabled;
    this.ensureEqGraph();
    this.applyEq();
    this.persist();
  }

  setEqGain(band: number, gainDb: number) {
    if (band < 0 || band >= 5) return;
    const next = [...this.eqGains];
    next[band] = Math.max(-12, Math.min(12, gainDb));
    this.eqGains = next;
    this.eqPreset = "custom";
    this.applyEq();
    this.persist();
  }

  setEqPreset(preset: EqPreset) {
    if (!(preset in EQ_PRESETS)) return;
    this.eqPreset = preset;
    if (preset !== "custom") {
      this.eqGains = [...EQ_PRESETS[preset]];
    }
    this.applyEq();
    this.persist();
  }

  async play(track: MusicTrack, queue?: MusicTrack[]) {
    this.init();

    void this.flushPlayReport();

    if (queue && queue.length > 0) {
      this.queue = queue;
      const idx = queue.findIndex(
        (t) =>
          t.id === track.id &&
          (t.source ?? "local") === (track.source ?? "local"),
      );
      this.queueIndex = idx >= 0 ? idx : 0;
    } else {
      const existingIdx = this.queue.findIndex(
        (t) =>
          t.id === track.id &&
          (t.source ?? "local") === (track.source ?? "local"),
      );
      if (existingIdx < 0) {
        this.queue = [track];
        this.queueIndex = 0;
      } else {
        this.queueIndex = existingIdx;
      }
    }

    this.currentTrack = track;
    this.error = null;
    this.lastError = null;
    this.status = "resolving";
    this.loading = true;
    this.fireBroadcast({ type: "track-change", track });
    this.ready = false;
    this.currentTime = 0;
    this.duration = (track.duration_ms ?? 0) / 1000;
    this.accumulatedPlayedMs = 0;
    this.playStartedAt = 0;
    this.lastReportTrackId = track.source === "spotify" ? null : track.id;

    if (track.source !== "youtube") {
      this.cancelYoutubeRefresh();
      this.youtubeCurrentVideoId = null;
      this.youtubeVideoUrl = null;
      this.youtubeChapters = [];
      this.youtubeSponsorBlockSegments = [];
    }

    if (track.source !== "spotify") {
      this.stopSpotifyTick();
      this.stopSpotifyHeartbeat();
      this._spotifyIsPlaying = false;
      this._spotifyLastKnownPositionMs = 0;
      this._spotifyLastEventAt = 0;
      this._spotifyPlaybackRate = 1;
    } else {
      this._spotifyLastKnownPositionMs = 0;
      this._spotifyLastEventAt = performance.now();
      this._spotifyPlaybackRate = 1;
      this._spotifyIsPlaying = false;
    }

    if (track.source === "spotify") {
      await this.playSpotify(track);
    } else if (track.source === "youtube") {
      await this.playYoutube(track);
    } else if (track.source === "soundcloud") {
      await this.playSoundcloud(track);
    } else {
      await this.playLocal(track);
    }

    this.recordHistory(track);
  }

  private recordHistory(track: MusicTrack) {
    const source = track.source ?? "local";
    const externalId =
      source === "youtube"
        ? track.youtube_video_id ?? null
        : source === "spotify"
          ? track.spotify_uri ?? null
          : source === "soundcloud"
            ? track.path
            : null;
    const dedupeKey = `${source}|${externalId ?? track.id}`;
    if (this.historyRecorded.has(dedupeKey)) return;
    this.historyRecorded.add(dedupeKey);

    const coverUrl =
      track.spotify_cover_url ??
      track.youtube_thumbnail ??
      track.soundcloud_artwork_url ??
      undefined;

    void import("$lib/study-bridge")
      .then((mod) =>
        mod.studyMusicHistoryAdd({
          source: source as "local" | "spotify" | "youtube" | "soundcloud",
          title: track.title ?? get(t)("study.music.store.untitled"),
          externalId: externalId ?? undefined,
          trackId: track.id,
          artist: track.artist ?? undefined,
          album: track.album ?? undefined,
          coverUrl,
          durationMs: track.duration_ms ?? undefined,
        }),
      )
      .catch(() => {
        /* fire-and-forget */
      });
  }

  soundcloudResolver:
    | ((track: MusicTrack) => Promise<{ url: string; is_hls: boolean }>)
    | null = null;
  private hlsHandle: HlsHandle | null = null;

  private async stopCurrentPlayback(opts?: { keepSpotify?: boolean }) {
    try {
      this.audio?.pause();
    } catch {
      /* ignore */
    }
    if (!opts?.keepSpotify) {
      try {
        await spotifySdk.pause();
      } catch {
        /* ignore */
      }
    }
    if (this.hlsHandle) {
      try {
        this.hlsHandle.destroy();
      } catch {
        /* ignore */
      }
      this.hlsHandle = null;
    }
    this.cancelYoutubeRefresh();
  }

  private async playSoundcloud(track: MusicTrack) {
    if (!this.audio) return;
    if (!this.soundcloudResolver) {
      this.setError(
        classifyPlayerError(
          get(t)("study.music.store.sc_resolver_unconfigured"),
          "soundcloud",
          track.id,
        ),
      );
      return;
    }
    this.loading = true;
    await this.stopCurrentPlayback();
    try {
      const { url, is_hls } = await this.soundcloudResolver(track);
      if (is_hls || url.includes(".m3u8")) {
        this.hlsHandle = await attachHls(this.audio, url);
      } else {
        this.audio.src = url;
      }
      await this.audio.play();
      this.ensureEqGraph();
      this.saveQueueNow();
      this.updateMediaSessionMetadata(track);
    } catch (e) {
      this.setError(classifyPlayerError(e, "soundcloud", track.id));
      throw e;
    }
  }

  private async playYoutube(track: MusicTrack) {
    if (!this.audio) return;
    const videoId = track.youtube_video_id;
    if (!videoId) {
      this.setError(
        classifyPlayerError("video id ausente — track removed", "youtube", track.id),
      );
      return;
    }
    this.loading = true;
    await this.stopCurrentPlayback();
    this.youtubeCurrentVideoId = videoId;
    this.youtubeVideoUrl = null;
    this.youtubeChapters = [];
    this.youtubeSponsorBlockSegments = [];
    this.youtubeRefreshFailureCount = 0;
    try {
      const { audio: audioUrl, video: videoUrl } =
        await this.resolveYoutubeMedia(videoId);
      if (!audioUrl) throw new Error("empty stream url");
      this.audio.crossOrigin = null;
      this.audio.src = audioUrl;
      this.youtubeVideoUrl = videoUrl;
      void this.refreshYoutubeSponsorBlock(videoId);
      await this.audio.play();
      this.saveQueueNow();
      this.updateMediaSessionMetadata(track);
      void this.refreshDominantColor(track);
      void pluginInvoke("study", "study:music:youtube:track_record_play", {
        video_id: videoId,
      }).catch(() => {});
    } catch (e) {
      this.setError(classifyPlayerError(e, "youtube", track.id));
    }
  }

  private cancelYoutubeRefresh() {
    if (this.youtubeRefreshTimer) {
      clearTimeout(this.youtubeRefreshTimer);
      this.youtubeRefreshTimer = null;
    }
  }

  private async resolveYoutubeMedia(
    videoId: string,
  ): Promise<{ audio: string | null; video: string | null; legacy: boolean }> {
    try {
      const { studyYoutubePlayer } = await import("$lib/study-bridge");
      const res = await studyYoutubePlayer({ videoId });
      if (res.sabr_only) {
        throw new Error(
          "SABR-only stream: no directly playable format (format unavailable in this client yet)",
        );
      }
      const audio = pickBestYoutubeAudio(res);
      const video = pickBestYoutubeVideo(res);
      if (!audio) throw new Error("no audio format returned");
      this.youtubeChapters = res.chapters ?? [];
      this.scheduleYoutubeRefresh(videoId, res.expires_at);
      return { audio, video, legacy: false };
    } catch (err) {
      console.warn(
        "[study-music] study:youtube:player failed, falling back to yt-dlp resolver:",
        err,
      );
      const legacy = await pluginInvoke<{ url: string; via?: string }>(
        "study",
        "study:music:youtube:track_stream_url",
        { video_id: videoId },
      );
      if (!legacy.url) {
        throw err instanceof Error
          ? err
          : new Error(typeof err === "string" ? err : "youtube resolve failed");
      }
      this.youtubeChapters = [];
      return { audio: legacy.url, video: null, legacy: true };
    }
  }

  private scheduleYoutubeRefresh(videoId: string, expiresAt: number) {
    this.cancelYoutubeRefresh();
    const now = Math.floor(Date.now() / 1000);
    const delaySecs = Math.max(30, expiresAt - now - 60);
    this.youtubeRefreshTimer = setTimeout(() => {
      void this.refreshYoutubeStream(videoId);
    }, delaySecs * 1000);
  }

  private async refreshYoutubeStream(videoId: string) {
    if (!this.audio) return;
    if (this.youtubeCurrentVideoId !== videoId) return;
    if (this.currentTrack?.youtube_video_id !== videoId) return;
    try {
      const { studyYoutubePlayer } = await import("$lib/study-bridge");
      console.info("[study-music] yt refresh start", { videoId });
      const res = await studyYoutubePlayer({ videoId });
      const url = pickBestYoutubeAudio(res);
      const videoUrl = pickBestYoutubeVideo(res);
      if (!url) throw new Error("refresh returned no audio format");
      const wasPaused = this.audio.paused;
      const t = this.audio.currentTime;
      this.audio.crossOrigin = null;
      this.audio.src = url;
      this.audio.currentTime = t;
      this.youtubeVideoUrl = videoUrl;
      if (!wasPaused) await this.audio.play();
      this.youtubeChapters = res.chapters ?? this.youtubeChapters;
      this.youtubeRefreshFailureCount = 0;
      this.scheduleYoutubeRefresh(videoId, res.expires_at);
      console.info("[study-music] yt refresh ok", { videoId, expires_at: res.expires_at });
    } catch (err) {
      this.youtubeRefreshFailureCount += 1;
      console.warn(
        "[study-music] yt refresh failed",
        { videoId, attempt: this.youtubeRefreshFailureCount },
        err,
      );
      if (
        this.youtubeRefreshFailureCount >= MusicPlayerStore.YOUTUBE_REFRESH_MAX_FAILURES
      ) {
        try {
          this.audio.pause();
        } catch {
          /* ignore */
        }
        const track = this.currentTrack;
        if (track) {
          this.setError(classifyPlayerError("auth_expired", "youtube", track.id));
        }
        this.youtubeRefreshFailureCount = 0;
        return;
      }
      this.cancelYoutubeRefresh();
      this.youtubeRefreshTimer = setTimeout(() => {
        void this.refreshYoutubeStream(videoId);
      }, MusicPlayerStore.YOUTUBE_REFRESH_RETRY_DELAY_SECS * 1000);
    }
  }

  private async refreshYoutubeSponsorBlock(videoId: string) {
    try {
      const { studyYoutubeSponsorblock } = await import("$lib/study-bridge");
      const res = await studyYoutubeSponsorblock({ videoId });
      if (this.youtubeCurrentVideoId !== videoId) return;
      this.youtubeSponsorBlockSegments = res.segments ?? [];
    } catch (err) {
      console.warn("[study-music] sponsorblock lookup failed:", err);
    }
  }

  private async playLocal(track: MusicTrack) {
    if (!this.audio) return;
    await this.stopCurrentPlayback();
    try {
      const isHttpUrl =
        track.path.startsWith("http://") || track.path.startsWith("https://");
      this.audio.src = isHttpUrl ? track.path : convertFileSrc(track.path);
      await this.audio.play();
      this.ensureEqGraph();
      this.saveQueueNow();
      this.updateMediaSessionMetadata(track);
      void this.refreshDominantColor(track);
    } catch (e) {
      this.setError(classifyPlayerError(e, track.source ?? "local", track.id));
    }
  }

  spotifyFreeFallback: ((track: MusicTrack) => Promise<string>) | null = null;

  private async playSpotify(track: MusicTrack) {
    if (!track.spotify_uri) {
      this.setError(classifyPlayerError("track removed: spotify uri ausente", "spotify", track.id));
      return;
    }
    const t0 = performance.now();
    this.loading = true;
    await this.stopCurrentPlayback({ keepSpotify: true });
    const tStop = performance.now();
    let sdkErr: unknown = null;
    try {
      await spotifySdk.ensureLoaded();
      const tReady = performance.now();
      const queueUris = this.queue
        .filter((t) => t.source === "spotify" && t.spotify_uri)
        .map((t) => t.spotify_uri as string);
      const startUri = track.spotify_uri;
      const orderedUris = (() => {
        const idx = queueUris.indexOf(startUri);
        if (idx < 0) return [startUri];
        return [...queueUris.slice(idx), ...queueUris.slice(0, idx)];
      })();
      await spotifySdk.play({ uris: orderedUris });
      const tPlay = performance.now();
      console.info(
        "[player] spotify timing",
        {
          stop_ms: Math.round(tStop - t0),
          ready_ms: Math.round(tReady - tStop),
          play_ms: Math.round(tPlay - tReady),
          total_ms: Math.round(tPlay - t0),
        },
      );
      this.saveQueueNow();
      this.updateMediaSessionMetadata(track);
      if (track.spotify_cover_url) {
        void this.refreshDominantColorFromUrl(track.spotify_cover_url);
      }
      if (this._spotifyHeartbeatEnabled) this.startSpotifyHeartbeat();
      return;
    } catch (e) {
      sdkErr = e;
      console.warn("[player] Spotify SDK failed, trying YouTube fallback:", e);
    }

    if (!this.spotifyFreeFallback || !this.audio) {
      this.setError(classifyPlayerError(sdkErr ?? "spotify sdk failed", "spotify", track.id));
      throw sdkErr ?? new Error("spotify sdk failed");
    }

    try {
      const url = await this.spotifyFreeFallback(track);
      track.youtube_url = url;
      this.audio.src = url;
      await this.audio.play();
      this.ensureEqGraph();
      this.saveQueueNow();
      this.updateMediaSessionMetadata(track);
    } catch (e) {
      this.setError(classifyPlayerError(e, "spotify", track.id));
      throw e;
    }
  }

  private async refreshDominantColorFromUrl(url: string) {
    if (!url) {
      this.dominantColor = null;
      this.palette = null;
      return;
    }
    const token = ++this.paletteRefreshToken;
    const trackId = this.currentTrack?.id ?? null;
    const palette = await palettePathOrUrl(url);
    if (token !== this.paletteRefreshToken) return;
    if (trackId !== null && this.currentTrack?.id !== trackId) return;
    if (palette && palette.length > 0) {
      this.palette = palette;
      this.dominantColor = palette[0];
    } else {
      this.palette = null;
      this.dominantColor = null;
    }
  }

  togglePlay() {
    this.init();
    if (!this.currentTrack) return;
    if (this.isSpotify()) {
      void spotifySdk.togglePlay();
      return;
    }
    if (!this.audio) return;
    if (this.paused) {
      this.audio.play().catch((e) => {
        const track = this.currentTrack;
        if (track) {
          this.setError(classifyPlayerError(e, track.source ?? "local", track.id));
        }
      });
      this.ensureEqGraph();
    } else {
      this.audio.pause();
    }
  }

  pause() {
    const time = this.currentTime;
    if (this.isSpotify()) {
      void spotifySdk.pause();
      this.fireBroadcast({ type: "pause", time });
      return;
    }
    if (this.audio && !this.paused) this.audio.pause();
    this.fireBroadcast({ type: "pause", time });
  }

  resume() {
    if (this.isSpotify()) {
      void spotifySdk.resume();
      this.fireBroadcast({ type: "play" });
      return;
    }
    if (this.audio && this.paused && this.currentTrack) {
      this.audio.play().catch(() => {});
      this.ensureEqGraph();
      this.fireBroadcast({ type: "play" });
    }
  }

  next() {
    if (this.isSpotify()) {
      void spotifySdk.next();
      return;
    }
    if (this.queue.length === 0) return;
    let nextIdx: number;
    if (this.shuffle && this.queue.length > 1) {
      do {
        nextIdx = Math.floor(Math.random() * this.queue.length);
      } while (nextIdx === this.queueIndex);
    } else {
      nextIdx = this.queueIndex + 1;
      if (nextIdx >= this.queue.length) {
        if (this.repeat === "all") {
          nextIdx = 0;
        } else {
          this.audio?.pause();
          return;
        }
      }
    }
    this.queueIndex = nextIdx;
    void this.play(this.queue[nextIdx], this.queue);
  }

  prev() {
    if (this.isSpotify()) {
      void spotifySdk.prev();
      return;
    }
    if (this.queue.length === 0) return;
    if (this.audio && this.audio.currentTime > 3) {
      this.audio.currentTime = 0;
      return;
    }
    let prevIdx = this.queueIndex - 1;
    if (prevIdx < 0) {
      prevIdx = this.repeat === "all" ? this.queue.length - 1 : 0;
    }
    this.queueIndex = prevIdx;
    void this.play(this.queue[prevIdx], this.queue);
  }

  seek(seconds: number) {
    if (this.isSpotify()) {
      const dur = this.duration;
      const target =
        Number.isFinite(dur) && dur > 0
          ? Math.max(0, Math.min(dur, seconds))
          : Math.max(0, seconds);
      const ms = Math.floor(target * 1000);
      this._spotifyLastKnownPositionMs = ms;
      this._spotifyLastEventAt = performance.now();
      this.currentTime = target;
      this.updateMediaSessionPosition();
      void spotifySdk.seek(ms);
      void this.fireRpcUpdate();
      this.fireBroadcast({ type: "seek", time: target });
      return;
    }
    if (!this.audio) return;
    const dur = this.audio.duration || this.duration;
    if (!Number.isFinite(dur) || dur <= 0) return;
    const target = Math.max(0, Math.min(dur, seconds));
    this.audio.currentTime = target;
    void this.fireRpcUpdate();
    this.fireBroadcast({ type: "seek", time: target });
  }

  seekRatio(ratio: number) {
    const dur = this.duration;
    if (!Number.isFinite(dur) || dur <= 0) return;
    this.seek(dur * Math.max(0, Math.min(1, ratio)));
  }

  setVolume(v: number) {
    const clamped = Math.max(0, Math.min(1, v));
    if (this.isSpotify()) {
      void spotifySdk.setVolume(clamped);
      this.volume = clamped;
      this.persist();
      return;
    }
    if (!this.audio) return;
    this.audio.volume = clamped;
    if (clamped > 0 && this.audio.muted) this.audio.muted = false;
    this.persist();
  }

  toggleMute() {
    if (this.isSpotify()) {
      const next = !this.muted;
      void spotifySdk.setVolume(next ? 0 : this.volume);
      this.muted = next;
      this.persist();
      return;
    }
    if (!this.audio) return;
    this.audio.muted = !this.audio.muted;
    this.persist();
  }

  toggleShuffle() {
    this.shuffle = !this.shuffle;
    this.persist();
  }

  cycleRepeat() {
    const order: RepeatMode[] = ["off", "all", "one"];
    const cur = order.indexOf(this.repeat);
    this.repeat = order[(cur + 1) % order.length];
    this.persist();
  }

  clearQueue() {
    this.audio?.pause();
    this.cancelYoutubeRefresh();
    this.youtubeCurrentVideoId = null;
    this.youtubeVideoUrl = null;
    this.youtubeChapters = [];
    this.youtubeSponsorBlockSegments = [];
    this.stopSpotifyTick();
    this.stopSpotifyHeartbeat();
    this._spotifyIsPlaying = false;
    this._spotifyLastKnownPositionMs = 0;
    this._spotifyLastEventAt = 0;
    this.queue = [];
    this.queueIndex = 0;
    this.currentTrack = null;
    this.currentTime = 0;
    this.duration = 0;
    this.historyRecorded.clear();
    this.saveQueueNow();
    void this.fireRpcClear();
  }

  private handleEnded() {
    void this.flushPlayReport();
    if (this.sleepTimerEndOfTrack) {
      this.sleepTimerEndOfTrack = false;
      this.audio?.pause();
      this.persistPlayerSettings();
      return;
    }
    if (this.repeat === "one" && this.audio) {
      this.audio.currentTime = 0;
      this.audio.play().catch(() => {});
      return;
    }
    this.next();
  }

  setSleepTimerMinutes(minutes: number | null) {
    if (minutes === null || minutes <= 0) {
      this.sleepTimerEndsAt = null;
      this.sleepFadeStarted = false;
      if (this.audio && this.sleepBaseVolume > 0) {
        this.audio.volume = this.sleepBaseVolume;
      }
    } else {
      this.sleepTimerEndsAt = Date.now() + minutes * 60_000;
      this.sleepBaseVolume = this.audio?.volume ?? 1;
      this.sleepFadeStarted = false;
    }
    this.persistPlayerSettings();
  }

  setSleepEndOfTrack(enabled: boolean) {
    this.sleepTimerEndOfTrack = enabled;
    if (enabled) {
      this.sleepTimerEndsAt = null;
    }
    this.persistPlayerSettings();
  }

  cancelSleepTimer() {
    this.sleepTimerEndsAt = null;
    this.sleepTimerEndOfTrack = false;
    this.sleepFadeStarted = false;
    if (this.audio && this.sleepBaseVolume > 0) {
      this.audio.volume = this.sleepBaseVolume;
    }
    this.persistPlayerSettings();
  }

  setCrossfade(seconds: number) {
    this.crossfadeSec = Math.max(0, Math.min(12, seconds));
    this.persistPlayerSettings();
  }

  private tickSleepTimer() {
    if (!this.audio) return;
    if (this.sleepTimerEndsAt === null) return;
    const remaining = this.sleepTimerEndsAt - Date.now();
    const FADE_MS = 10_000;
    if (remaining <= 0) {
      this.audio.pause();
      this.audio.volume = this.sleepBaseVolume;
      this.sleepTimerEndsAt = null;
      this.sleepFadeStarted = false;
      this.persistPlayerSettings();
      return;
    }
    if (remaining <= FADE_MS) {
      if (!this.sleepFadeStarted) {
        this.sleepFadeStarted = true;
        this.sleepBaseVolume = this.audio.volume || 1;
      }
      const factor = Math.max(0, remaining / FADE_MS);
      this.audio.volume = this.sleepBaseVolume * factor;
    }
  }

  private tickCrossfade() {
    if (!this.audio) return;
    if (this.crossfadeSec <= 0 || this.duration <= 0) {
      if (this.crossfadeStarted && this.crossfadeBaseVolume > 0) {
        this.audio.volume = this.crossfadeBaseVolume;
        this.crossfadeStarted = false;
      }
      return;
    }
    const remaining = this.duration - this.currentTime;
    if (remaining <= this.crossfadeSec && remaining > 0) {
      if (!this.crossfadeStarted) {
        this.crossfadeStarted = true;
        this.crossfadeBaseVolume = this.audio.volume || 1;
      }
      const factor = Math.max(0, remaining / this.crossfadeSec);
      this.audio.volume = this.crossfadeBaseVolume * factor;
    } else if (this.crossfadeStarted) {
      this.audio.volume = this.crossfadeBaseVolume;
      this.crossfadeStarted = false;
    }
  }

  private persistPlayerSettings() {
    try {
      const raw = localStorage.getItem(PERSIST_KEY);
      const data = raw ? JSON.parse(raw) : {};
      data.crossfadeSec = this.crossfadeSec;
      data.sleepEndOfTrack = this.sleepTimerEndOfTrack;
      data.normalizationEnabled = this.normalizationEnabled;
      data.skipSilenceEnabled = this.skipSilenceEnabled;
      localStorage.setItem(PERSIST_KEY, JSON.stringify(data));
    } catch {
      /* ignore */
    }
  }

  private async flushPlayReport() {
    const track_id = this.lastReportTrackId;
    if (!track_id) return;
    if (this.playStartedAt > 0) {
      this.accumulatedPlayedMs += Date.now() - this.playStartedAt;
      this.playStartedAt = 0;
    }
    const positionMs = Math.floor((this.currentTime || 0) * 1000);
    const durationPlayedMs = this.accumulatedPlayedMs;
    this.accumulatedPlayedMs = 0;
    this.lastReportTrackId = null;
    if (durationPlayedMs <= 0) return;
    try {
      await pluginInvoke("study", "study:music:play:record", {
        trackId: track_id,
        positionMs,
        durationPlayedMs,
      });
    } catch {
      /* ignore */
    }
  }

  async toggleFavorite(trackId: number): Promise<boolean | null> {
    try {
      const res = await pluginInvoke<{ favorite: boolean }>(
        "study",
        "study:music:favorite:toggle",
        { trackId },
      );
      if (this.currentTrack && this.currentTrack.id === trackId) {
        this.currentTrack = { ...this.currentTrack, favorite: res.favorite };
      }
      return res.favorite;
    } catch {
      return null;
    }
  }

  reportNow() {
    void this.flushPlayReport();
    this.saveQueueNow();
  }

  suspendAudioForWatch(): { currentTime: number; wasPlaying: boolean } {
    if (!this.audio) return { currentTime: 0, wasPlaying: false };
    const snap = {
      currentTime: this.audio.currentTime || 0,
      wasPlaying: !this.audio.paused,
    };
    try {
      this.audio.pause();
    } catch {
      /* ignore */
    }
    return snap;
  }

  resumeAudioFromWatch(currentTime: number, shouldPlay: boolean) {
    if (!this.audio) return;
    try {
      this.audio.currentTime = currentTime;
    } catch {
      /* ignore */
    }
    if (shouldPlay) {
      this.audio.play().catch(() => {});
      this.ensureEqGraph();
    }
  }
}

export const musicPlayer = new MusicPlayerStore();

if (typeof window !== "undefined") {
  window.addEventListener("beforeunload", () => {
    musicPlayer.reportNow();
  });
}

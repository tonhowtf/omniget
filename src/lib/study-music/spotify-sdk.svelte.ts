import { get } from "svelte/store";
import { t } from "$lib/i18n";
import { pluginInvoke } from "$lib/plugin-invoke";

declare global {
  interface Window {
    onSpotifyWebPlaybackSDKReady?: () => void;
    Spotify?: {
      Player: new (opts: SpotifyPlayerOptions) => SpotifyPlayer;
    };
  }
}

type SpotifyPlayerOptions = {
  name: string;
  getOAuthToken: (cb: (token: string) => void) => void;
  volume?: number;
};

type SpotifyPlayerEvent =
  | "ready"
  | "not_ready"
  | "initialization_error"
  | "authentication_error"
  | "account_error"
  | "playback_error"
  | "player_state_changed";

type SpotifyPlayer = {
  connect(): Promise<boolean>;
  disconnect(): void;
  addListener(event: SpotifyPlayerEvent, cb: (data: any) => void): void;
  removeListener(event: SpotifyPlayerEvent): void;
  pause(): Promise<void>;
  resume(): Promise<void>;
  togglePlay(): Promise<void>;
  seek(positionMs: number): Promise<void>;
  nextTrack(): Promise<void>;
  previousTrack(): Promise<void>;
  setVolume(volume: number): Promise<void>;
  getVolume(): Promise<number>;
  getCurrentState(): Promise<SpotifySdkState | null>;
  setName(name: string): Promise<void>;
};

export type SpotifySdkState = {
  paused: boolean;
  position: number;
  duration: number;
  shuffle: boolean;
  repeat_mode: number;
  track_window: {
    current_track: {
      id: string;
      uri: string;
      name: string;
      duration_ms: number;
      artists: { name: string; uri: string }[];
      album: { name: string; uri: string; images: { url: string }[] };
    };
  };
};

const SDK_SCRIPT = "https://sdk.scdn.co/spotify-player.js";
const DEVICE_NAME = "OmniGet";

class SpotifySdkController {
  ready = $state(false);
  deviceId = $state<string | null>(null);
  unavailableReason = $state<string | null>(null);
  loading = $state(false);
  state = $state<SpotifySdkState | null>(null);

  private player: SpotifyPlayer | null = null;
  private loadingPromise: Promise<void> | null = null;
  private stateListeners = new Set<(state: SpotifySdkState | null) => void>();

  onState(cb: (state: SpotifySdkState | null) => void): () => void {
    this.stateListeners.add(cb);
    return () => this.stateListeners.delete(cb);
  }

  private emitState(state: SpotifySdkState | null) {
    this.state = state;
    for (const cb of this.stateListeners) {
      try {
        cb(state);
      } catch {
        /* ignore listener errors */
      }
    }
  }

  async checkWidevine(): Promise<boolean> {
    if (typeof navigator === "undefined" || !navigator.requestMediaKeySystemAccess) {
      return false;
    }
    try {
      const config: MediaKeySystemConfiguration[] = [
        {
          initDataTypes: ["cenc"],
          audioCapabilities: [
            { contentType: 'audio/mp4;codecs="mp4a.40.2"' },
          ],
        },
      ];
      await navigator.requestMediaKeySystemAccess("com.widevine.alpha", config);
      return true;
    } catch {
      return false;
    }
  }

  async ensureLoaded(): Promise<void> {
    if (this.player && this.ready) return;
    if (this.loadingPromise) return this.loadingPromise;

    this.loading = true;
    this.unavailableReason = null;

    this.loadingPromise = new Promise<void>((resolve, reject) => {
      void (async () => {
        const widevine = await this.checkWidevine();
        if (!widevine) {
          this.unavailableReason =
            get(t)("study.music.sdk.widevine_unavailable");
          this.loading = false;
          reject(new Error(this.unavailableReason));
          return;
        }

        const onSdkReady = () => {
          if (!window.Spotify) {
            this.unavailableReason = get(t)("study.music.sdk.sdk_no_global");
            this.loading = false;
            reject(new Error(this.unavailableReason));
            return;
          }

          const player = new window.Spotify.Player({
            name: DEVICE_NAME,
            getOAuthToken: async (cb) => {
              try {
                const res = await pluginInvoke<{ access_token: string }>(
                  "study",
                  "study:spotify:auth:access_token",
                );
                cb(res.access_token);
              } catch (e) {
                console.error("[spotify-sdk] failed to get access_token:", e);
              }
            },
            volume: 1.0,
          });

          player.addListener("ready", ({ device_id }: { device_id: string }) => {
            this.deviceId = device_id;
            this.ready = true;
            this.loading = false;
            resolve();
          });
          player.addListener("not_ready", () => {
            this.ready = false;
          });
          player.addListener(
            "initialization_error",
            ({ message }: { message: string }) => {
              this.unavailableReason = `init: ${message}`;
              this.loading = false;
              reject(new Error(message));
            },
          );
          player.addListener(
            "authentication_error",
            ({ message }: { message: string }) => {
              this.unavailableReason = `auth: ${message}`;
            },
          );
          player.addListener(
            "account_error",
            ({ message }: { message: string }) => {
              this.unavailableReason = `Spotify Premium é obrigatório (${message})`;
            },
          );
          player.addListener(
            "playback_error",
            ({ message }: { message: string }) => {
              console.warn("[spotify-sdk] playback error:", message);
            },
          );
          player.addListener(
            "player_state_changed",
            (state: SpotifySdkState | null) => {
              this.emitState(state);
            },
          );

          this.player = player;
          void player.connect().catch((e) => {
            this.unavailableReason = `connect: ${String(e)}`;
            this.loading = false;
            reject(e);
          });
        };

        if (window.Spotify) {
          onSdkReady();
          return;
        }
        const existing = document.querySelector(
          `script[src="${SDK_SCRIPT}"]`,
        ) as HTMLScriptElement | null;
        window.onSpotifyWebPlaybackSDKReady = onSdkReady;
        if (!existing) {
          const script = document.createElement("script");
          script.src = SDK_SCRIPT;
          script.async = true;
          script.onerror = () => {
            this.unavailableReason = "Falha ao carregar SDK do Spotify (offline?)";
            this.loading = false;
            reject(new Error(this.unavailableReason));
          };
          document.head.appendChild(script);
        }
      })();
    }).catch((e) => {
      this.loadingPromise = null;
      throw e;
    });

    return this.loadingPromise;
  }

  async play(opts: {
    uris?: string[];
    contextUri?: string;
    positionMs?: number;
  }): Promise<void> {
    await this.ensureLoaded();
    if (!this.deviceId) throw new Error("Spotify SDK device not ready");
    await pluginInvoke("study", "study:spotify:playback:play", {
      deviceId: this.deviceId,
      uris: opts.uris ?? [],
      contextUri: opts.contextUri,
      positionMs: opts.positionMs,
    });
  }

  async pause(): Promise<void> {
    if (this.player) await this.player.pause();
  }

  async resume(): Promise<void> {
    if (this.player) await this.player.resume();
  }

  async togglePlay(): Promise<void> {
    if (this.player) await this.player.togglePlay();
  }

  async seek(positionMs: number): Promise<void> {
    if (this.player) await this.player.seek(Math.max(0, Math.floor(positionMs)));
  }

  async getCurrentState(): Promise<SpotifySdkState | null> {
    if (!this.player) return null;
    try {
      return await this.player.getCurrentState();
    } catch {
      return null;
    }
  }

  async next(): Promise<void> {
    if (this.player) await this.player.nextTrack();
  }

  async prev(): Promise<void> {
    if (this.player) await this.player.previousTrack();
  }

  async setVolume(volume: number): Promise<void> {
    if (this.player) await this.player.setVolume(Math.max(0, Math.min(1, volume)));
  }

  disconnect(): void {
    if (this.player) {
      try {
        this.player.disconnect();
      } catch {
        /* ignore */
      }
      this.player = null;
    }
    this.ready = false;
    this.deviceId = null;
    this.state = null;
    this.loadingPromise = null;
  }
}

export const spotifySdk = new SpotifySdkController();

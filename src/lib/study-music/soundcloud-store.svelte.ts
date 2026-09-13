import { get } from "svelte/store";
import { t } from "$lib/i18n";
import { pluginInvoke } from "$lib/plugin-invoke";
import { musicPlayer, type MusicTrack } from "./player-store.svelte";

export type ScUser = {
  id: number;
  username: string;
  permalink: string;
  permalink_url: string;
  avatar_url: string | null;
  followers_count?: number;
  followings_count?: number;
  description?: string | null;
  city?: string | null;
  country_code?: string | null;
};

export type ScTrack = {
  id: number;
  title: string;
  duration: number;
  artwork_url: string | null;
  permalink_url: string;
  streamable: boolean;
  downloadable: boolean;
  user: ScUser;
  display_date?: string;
  genre?: string | null;
  description?: string | null;
  tag_list?: string;
  likes_count?: number;
  playback_count?: number;
};

export type ScPlaylist = {
  id: number;
  title: string;
  permalink_url: string;
  artwork_url: string | null;
  track_count: number;
  duration: number;
  is_album: boolean;
  user: ScUser;
  tracks?: ScTrack[];
  description?: string | null;
};

export type ScQuality = {
  id: string;
  label: string;
  protocol: string;
  quality: string;
  preset?: string;
  mime_type?: string;
  is_premium: boolean;
};

type AuthStatus = {
  has_client_id: boolean;
  has_oauth: boolean;
  user_id: number | null;
};

function bigArtwork(url: string | null | undefined): string | null {
  if (!url) return null;
  return url.replace(/-(large|t300x300|t120x120|t67x67|small|tiny|mini)\./, "-t500x500.");
}

function mapUser(u: any): ScUser | null {
  if (!u || typeof u.id !== "number") return null;
  return {
    id: u.id,
    username: u.username ?? "",
    permalink: u.permalink ?? "",
    permalink_url: u.permalink_url ?? "",
    avatar_url: u.avatar_url ?? null,
    followers_count: u.followers_count,
    followings_count: u.followings_count,
    description: u.description ?? null,
    city: u.city ?? null,
    country_code: u.country_code ?? null,
  };
}

export function mapTrack(t: any): ScTrack | null {
  if (!t || typeof t.id !== "number") return null;
  const user = mapUser(t.user);
  if (!user) return null;
  return {
    id: t.id,
    title: t.title ?? "",
    duration: t.duration ?? 0,
    artwork_url: t.artwork_url ?? null,
    permalink_url: t.permalink_url ?? "",
    streamable: !!t.streamable,
    downloadable: !!t.downloadable,
    user,
    display_date: t.display_date,
    genre: t.genre ?? null,
    description: t.description ?? null,
    tag_list: t.tag_list,
    likes_count: t.likes_count,
    playback_count: t.playback_count,
  };
}

export function mapPlaylist(p: any): ScPlaylist | null {
  if (!p || typeof p.id !== "number") return null;
  const user = mapUser(p.user);
  if (!user) return null;
  const tracks: ScTrack[] = Array.isArray(p.tracks)
    ? (p.tracks as any[]).map(mapTrack).filter((t): t is ScTrack => !!t)
    : [];
  return {
    id: p.id,
    title: p.title ?? "",
    permalink_url: p.permalink_url ?? "",
    artwork_url: p.artwork_url ?? null,
    track_count: p.track_count ?? tracks.length,
    duration: p.duration ?? 0,
    is_album: !!p.is_album,
    user,
    tracks,
    description: p.description ?? null,
  };
}

export function scTrackToMusicTrack(t: ScTrack): MusicTrack {
  return {
    id: -(t.id + 1_000_000_000),
    path: `soundcloud:track:${t.id}`,
    title: t.title,
    artist: t.user.username,
    album: null,
    duration_ms: t.duration,
    cover_path: null,
    source: "soundcloud" as any,
    soundcloud_id: t.id,
    soundcloud_artwork_url: bigArtwork(t.artwork_url) ?? bigArtwork(t.user.avatar_url) ?? undefined,
  } as MusicTrack & { soundcloud_id: number; soundcloud_artwork_url?: string };
}

class SoundCloudStore {
  status = $state<AuthStatus>({
    has_client_id: false,
    has_oauth: false,
    user_id: null,
  });
  profile = $state<ScUser | null>(null);
  likedTracks = $state<ScTrack[]>([]);
  playlists = $state<ScPlaylist[]>([]);
  followings = $state<ScUser[]>([]);
  streamFeed = $state<ScTrack[]>([]);
  loading = $state(false);
  error = $state<string | null>(null);

  pickArtwork = bigArtwork;

  get isLoggedIn(): boolean {
    return this.status.has_oauth && !!this.status.user_id;
  }

  get hasOauth(): boolean {
    return this.status.has_oauth;
  }

  async refreshStatus() {
    try {
      let s = await pluginInvoke<AuthStatus>("study", "study:soundcloud:auth:refresh");
      this.status = s;
      if (!s.has_oauth) {
        const hydrated = await this.tryHydrateFromCookieManager();
        if (hydrated) {
          s = await pluginInvoke<AuthStatus>("study", "study:soundcloud:auth:refresh");
          this.status = s;
        }
      }
      if (s.has_oauth && s.user_id == null) {
        try {
          const me = await pluginInvoke<any>("study", "study:soundcloud:me");
          this.profile = mapUser(me);
          if (this.profile) this.status = { ...this.status, user_id: this.profile.id };
        } catch (e) {
          console.warn("[soundcloud] me failed:", e);
        }
      } else if (s.user_id) {
        try {
          const me = await pluginInvoke<any>("study", "study:soundcloud:me");
          this.profile = mapUser(me);
        } catch {
          /* ignore */
        }
      }
    } catch (e) {
      this.error = e instanceof Error ? e.message : String(e);
    }
  }

  async tryHydrateFromCookieManager(): Promise<boolean> {
    try {
      const { invoke } = await import("@tauri-apps/api/core");
      const accounts = await invoke<{ domain: string; accounts: { slug: string }[] }>(
        "cookies_accounts_for_url",
        { url: "https://soundcloud.com/" }
      );
      if (!accounts.accounts.length) return false;
      const slug = accounts.accounts[0].slug;
      const cookies = await invoke<
        { name: string; value: string; domain: string; path: string }[]
      >("cookies_read_as_json", { request: { domain: accounts.domain, slug } });
      if (!cookies.length) return false;
      await pluginInvoke("study", "study:soundcloud:auth:set_cookies", {
        cookies_json: JSON.stringify(cookies),
      });
      return true;
    } catch (e) {
      console.warn("[soundcloud] hydrate from cookie manager failed:", e);
      return false;
    }
  }

  async logout() {
    try {
      await pluginInvoke("study", "study:soundcloud:auth:logout");
    } finally {
      this.profile = null;
      this.likedTracks = [];
      this.playlists = [];
      this.followings = [];
      this.streamFeed = [];
      this.status = { has_client_id: this.status.has_client_id, has_oauth: false, user_id: null };
      this.error = null;
      await this.refreshStatus();
    }
  }

  async loginWithWebview(): Promise<void> {
    const { invoke } = await import("@tauri-apps/api/core");
    this.error = null;
    const result = await invoke<{
      cookies: { name: string; value: string; domain: string; path: string }[];
      finalUrl: string;
    }>("open_auth_webview", {
      request: {
        url: "https://soundcloud.com/signin",
        title: "Entrar com SoundCloud",
        cookieDomains: [".soundcloud.com", "soundcloud.com"],
        successUrlContains: null,
        waitForCookie: "oauth_token",
        initializationScript: null,
      },
    });
    if (!result?.cookies || result.cookies.length === 0) {
      throw new Error(get(t)("study.music.store.login_not_captured"));
    }
    const cookiesJson = JSON.stringify(result.cookies);
    await pluginInvoke("study", "study:soundcloud:auth:set_cookies", {
      cookies_json: cookiesJson,
    });
    await this.refreshStatus();
    if (this.isLoggedIn) {
      await this.loadAll();
    }
  }

  async loadAll() {
    if (!this.isLoggedIn) return;
    this.loading = true;
    try {
      const [likedRes, playlistsRes, followingsRes, streamRes] = await Promise.allSettled([
        pluginInvoke<{ collection?: any[] }>("study", "study:soundcloud:liked_tracks", {
          limit: 50,
          offset: 0,
        }),
        pluginInvoke<{ collection?: any[] }>("study", "study:soundcloud:user_playlists", {
          limit: 50,
          offset: 0,
        }),
        pluginInvoke<{ collection?: any[] }>("study", "study:soundcloud:followings", {
          limit: 50,
          offset: 0,
        }),
        pluginInvoke<{ collection?: any[] }>("study", "study:soundcloud:stream_feed", {
          limit: 50,
          offset: 0,
        }),
      ]);

      if (likedRes.status === "fulfilled") {
        const items = likedRes.value.collection ?? [];
        this.likedTracks = items
          .map((it: any) => mapTrack(it.track ?? it))
          .filter((t): t is ScTrack => !!t);
      }
      if (playlistsRes.status === "fulfilled") {
        const items = playlistsRes.value.collection ?? [];
        this.playlists = items.map(mapPlaylist).filter((p): p is ScPlaylist => !!p);
      }
      if (followingsRes.status === "fulfilled") {
        const items = followingsRes.value.collection ?? [];
        this.followings = items.map(mapUser).filter((u): u is ScUser => !!u);
      }
      if (streamRes.status === "fulfilled") {
        const items = streamRes.value.collection ?? [];
        const tracks: ScTrack[] = [];
        const seen = new Set<number>();
        for (const it of items) {
          const t = mapTrack(it.track ?? it.origin?.track ?? it.origin ?? it);
          if (t && !seen.has(t.id)) {
            tracks.push(t);
            seen.add(t.id);
          }
        }
        this.streamFeed = tracks;
      }
    } finally {
      this.loading = false;
    }
  }

  async getPlaylist(playlistId: number): Promise<ScPlaylist | null> {
    const v = await pluginInvoke<any>("study", "study:soundcloud:playlist:get", {
      playlistId,
    });
    return mapPlaylist(v);
  }

  async getUser(userId: number): Promise<ScUser | null> {
    const v = await pluginInvoke<any>("study", "study:soundcloud:user:get", { userId });
    return mapUser(v);
  }

  async getUserTracks(userId: number, limit = 50): Promise<ScTrack[]> {
    const res = await pluginInvoke<{ collection?: any[] }>(
      "study",
      "study:soundcloud:user:tracks",
      { userId, limit, offset: 0 },
    );
    return (res.collection ?? []).map(mapTrack).filter((t): t is ScTrack => !!t);
  }

  async getUserPlaylists(userId: number, limit = 50): Promise<ScPlaylist[]> {
    const res = await pluginInvoke<{ collection?: any[] }>(
      "study",
      "study:soundcloud:user:playlists",
      { userId, limit, offset: 0 },
    );
    return (res.collection ?? []).map(mapPlaylist).filter((p): p is ScPlaylist => !!p);
  }

  async search(
    kind: "tracks" | "users" | "albums" | "playlists",
    q: string,
    limit = 20,
  ) {
    const res = await pluginInvoke<{ collection?: any[] }>(
      "study",
      "study:soundcloud:search",
      { kind, q, limit },
    );
    const items = res.collection ?? [];
    if (kind === "users") return items.map(mapUser).filter((u): u is ScUser => !!u);
    if (kind === "playlists" || kind === "albums")
      return items.map(mapPlaylist).filter((p): p is ScPlaylist => !!p);
    return items.map(mapTrack).filter((t): t is ScTrack => !!t);
  }

  async listQualities(trackId: number): Promise<ScQuality[]> {
    const res = await pluginInvoke<{ qualities: ScQuality[] }>(
      "study",
      "study:soundcloud:list_qualities",
      { trackId },
    );
    return res.qualities ?? [];
  }

  async resolveStream(trackId: number, quality = "progressive"): Promise<{
    url: string;
    is_hls: boolean;
    mime_type: string;
    quality: string;
  }> {
    return pluginInvoke("study", "study:soundcloud:stream:resolve", {
      trackId,
      quality,
    });
  }

  async download(opts: {
    trackId: number;
    codec: string;
    outputDir: string;
    quality: string;
  }): Promise<{ path: string; codec: string; quality: string }> {
    return pluginInvoke("study", "study:soundcloud:download", opts);
  }

  async downloadPlaylist(opts: {
    playlistId: number;
    codec: string;
    outputDir: string;
    quality: string;
  }): Promise<{ total: number; success: number; failed: number; files: string[] }> {
    return pluginInvoke("study", "study:soundcloud:download:playlist", opts);
  }

  async resolveAnyUrl(url: string): Promise<{ kind: string; id: number; title: string; data: any }> {
    return pluginInvoke("study", "study:soundcloud:resolve_url", { url });
  }

  async relatedTracks(trackId: number, limit = 12): Promise<ScTrack[]> {
    const res = await pluginInvoke<{ collection?: any[] }>(
      "study",
      "study:soundcloud:track:related",
      { trackId, limit },
    );
    return (res.collection ?? []).map(mapTrack).filter((t): t is ScTrack => !!t);
  }

  async toggleLike(trackId: number, currentlyLiked: boolean): Promise<boolean> {
    if (currentlyLiked) {
      await pluginInvoke("study", "study:soundcloud:like:remove", { trackId });
      this.likedTracks = this.likedTracks.filter((t) => t.id !== trackId);
      return false;
    }
    await pluginInvoke("study", "study:soundcloud:like:add", { trackId });
    return true;
  }

  async playTrack(track: ScTrack, queue?: ScTrack[]): Promise<void> {
    const local = scTrackToMusicTrack(track);
    const localQueue = (queue && queue.length > 0 ? queue : [track]).map(scTrackToMusicTrack);
    await musicPlayer.play(local, localQueue);
  }
}

export const soundcloudStore = new SoundCloudStore();

async function resolveSoundcloudStream(
  track: MusicTrack,
): Promise<{ url: string; is_hls: boolean }> {
  const scId = (track as any).soundcloud_id as number | undefined;
  if (!scId) throw new Error("Track sem soundcloud_id");
  const res = await pluginInvoke<{ url: string; is_hls: boolean }>(
    "study",
    "study:soundcloud:stream:resolve",
    { trackId: scId, quality: "progressive" },
  );
  if (!res.url) throw new Error("SoundCloud nao retornou URL");
  return res;
}

if (typeof window !== "undefined") {
  (musicPlayer as any).soundcloudResolver = resolveSoundcloudStream;
}

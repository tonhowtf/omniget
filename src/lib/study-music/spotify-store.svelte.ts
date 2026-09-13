import { get } from "svelte/store";
import { t } from "$lib/i18n";
import { pluginInvoke } from "$lib/plugin-invoke";
import { musicPlayer, type MusicTrack } from "./player-store.svelte";
import { spotifySdk } from "./spotify-sdk.svelte";

export type SpotifyImage = { url: string; height?: number | null; width?: number | null };

export type SpotifyArtistRef = { id: string; name: string; uri: string };

export type SpotifyAlbumRef = {
  id: string;
  name: string;
  uri: string;
  images: SpotifyImage[];
  release_date?: string;
};

export type SpotifyTrack = {
  id: string;
  name: string;
  uri: string;
  duration_ms: number;
  explicit: boolean;
  is_playable?: boolean;
  preview_url: string | null;
  isrc?: string;
  artists: SpotifyArtistRef[];
  album: SpotifyAlbumRef;
  added_at?: string;
  played_at?: string;
};

export type SpotifyPlaylist = {
  id: string;
  name: string;
  description: string | null;
  uri: string;
  images: SpotifyImage[];
  tracks_total: number;
  owner_name: string | null;
  owner_id: string | null;
  collaborative: boolean;
  public: boolean;
};

export type SpotifyArtist = {
  id: string;
  name: string;
  uri: string;
  images: SpotifyImage[];
  genres: string[];
  followers?: number;
  popularity?: number;
};

export type SpotifySavedAlbum = {
  id: string;
  name: string;
  uri: string;
  images: SpotifyImage[];
  artists: SpotifyArtistRef[];
  release_date: string | null;
  total_tracks: number;
  added_at?: string;
};

export type SpotifyDevice = {
  id: string | null;
  name: string;
  type: string;
  is_active: boolean;
  is_private_session: boolean;
  is_restricted: boolean;
  volume_percent: number | null;
};

export type SpotifyProfile = {
  id: string;
  display_name: string | null;
  email: string | null;
  product: string | null;
  country: string | null;
  images: SpotifyImage[];
};

type AuthStatus = {
  logged_in: boolean;
  expires_in_secs: number;
  scope: string;
  has_client_id: boolean;
  redirect_uri: string;
};

function pickImage(images: SpotifyImage[] | undefined | null, target = 300): string | null {
  if (!images || images.length === 0) return null;
  const sorted = [...images].sort((a, b) => (a.width ?? 0) - (b.width ?? 0));
  const found = sorted.find((i) => (i.width ?? 0) >= target);
  return (found ?? sorted[sorted.length - 1])?.url ?? null;
}

function hashStringToInt(s: string): number {
  let h = 0;
  for (let i = 0; i < s.length; i++) {
    h = (h << 5) - h + s.charCodeAt(i);
    h |= 0;
  }
  return -Math.abs(h || 1);
}

export function spotifyTrackToMusicTrack(track: SpotifyTrack): MusicTrack {
  const cover = pickImage(track.album.images, 300);
  return {
    id: hashStringToInt(track.uri),
    path: track.uri,
    title: track.name,
    artist: track.artists.map((a) => a.name).join(", "),
    album: track.album.name,
    duration_ms: track.duration_ms,
    cover_path: null,
    source: "spotify",
    spotify_uri: track.uri,
    spotify_cover_url: cover ?? undefined,
    isrc: track.isrc,
  };
}

async function resolveSpotifyToYoutube(track: MusicTrack): Promise<string> {
  if (!track.title || !track.artist) {
    throw new Error("Track sem metadata pra resolver no YouTube");
  }
  console.log("[spotify-yt] resolving:", {
    title: track.title,
    artist: track.artist,
    isrc: track.isrc,
    duration_ms: track.duration_ms,
  });
  const res = await pluginInvoke<{
    youtube_url: string;
    video_id: string;
    video_title: string;
    channel: string;
    score: number;
  }>("study", "study:spotify:resolve_youtube", {
    title: track.title,
    artist: track.artist,
    durationMs: track.duration_ms ?? 0,
    isrc: track.isrc,
  });
  console.log("[spotify-yt] resolved:", {
    video_id: res.video_id,
    video_title: res.video_title,
    channel: res.channel,
    score: res.score,
    url_preview: res.youtube_url?.slice(0, 100),
  });
  if (!res.youtube_url) throw new Error(get(t)("study.music.store.yt_no_url"));
  return res.youtube_url;
}

if (typeof window !== "undefined") {
  musicPlayer.spotifyFreeFallback = resolveSpotifyToYoutube;
}

function mapTrack(item: any, addedAt?: string, playedAt?: string): SpotifyTrack | null {
  const t = item?.track ?? item;
  if (!t || !t.id) return null;
  return {
    id: t.id,
    name: t.name ?? "",
    uri: t.uri ?? `spotify:track:${t.id}`,
    duration_ms: t.duration_ms ?? 0,
    explicit: !!t.explicit,
    is_playable: t.is_playable,
    preview_url: t.preview_url ?? null,
    isrc: t.external_ids?.isrc,
    artists: (t.artists ?? []).map((a: any) => ({
      id: a.id,
      name: a.name,
      uri: a.uri ?? `spotify:artist:${a.id}`,
    })),
    album: {
      id: t.album?.id ?? "",
      name: t.album?.name ?? "",
      uri: t.album?.uri ?? "",
      images: t.album?.images ?? [],
      release_date: t.album?.release_date,
    },
    added_at: addedAt ?? item?.added_at,
    played_at: playedAt ?? item?.played_at,
  };
}

function mapPlaylist(p: any, _owner?: any): SpotifyPlaylist | null {
  if (!p || !p.id) return null;
  return {
    id: p.id,
    name: p.name ?? "",
    description: p.description ?? null,
    uri: p.uri ?? `spotify:playlist:${p.id}`,
    images: p.images ?? [],
    tracks_total: p.tracks?.total ?? 0,
    owner_name: p.owner?.display_name ?? p.owner?.id ?? null,
    owner_id: p.owner?.id ?? null,
    collaborative: !!p.collaborative,
    public: !!p.public,
  };
}

function mapArtist(a: any): SpotifyArtist | null {
  if (!a || !a.id) return null;
  return {
    id: a.id,
    name: a.name ?? "",
    uri: a.uri ?? `spotify:artist:${a.id}`,
    images: a.images ?? [],
    genres: a.genres ?? [],
    followers: a.followers?.total,
    popularity: a.popularity,
  };
}

class SpotifyStore {
  status = $state<AuthStatus>({
    logged_in: false,
    expires_in_secs: 0,
    scope: "",
    has_client_id: false,
    redirect_uri: "http://127.0.0.1:8888/callback",
  });
  profile = $state<SpotifyProfile | null>(null);
  savedTracks = $state<SpotifyTrack[]>([]);
  playlists = $state<SpotifyPlaylist[]>([]);
  recentlyPlayed = $state<SpotifyTrack[]>([]);
  topArtists = $state<SpotifyArtist[]>([]);
  topTracks = $state<SpotifyTrack[]>([]);
  savedAlbums = $state<SpotifySavedAlbum[]>([]);
  followedArtists = $state<SpotifyArtist[]>([]);
  devices = $state<SpotifyDevice[]>([]);
  loading = $state(false);
  loadingLibrary = $state(false);
  error = $state<string | null>(null);
  authInProgress = $state(false);

  widevineSupported = $state<boolean | null>(null);
  sdkPrewarmed = $state(false);
  localMatches = $state<Map<string, number>>(new Map());

  pickImage = pickImage;

  get isPremium(): boolean {
    return this.profile?.product === "premium";
  }

  get playbackMode(): "sdk" | "connect" | "unknown" {
    if (this.isPremium && this.widevineSupported && !spotifySdk.unavailableReason) {
      return "sdk";
    }
    if (this.profile) {
      return "connect";
    }
    return "unknown";
  }

  async detectCapabilities() {
    this.widevineSupported = await spotifySdk.checkWidevine();
  }

  async prewarmSdk() {
    if (this.sdkPrewarmed) return;
    if (!this.isPremium || !this.widevineSupported) return;
    try {
      await spotifySdk.ensureLoaded();
      this.sdkPrewarmed = true;
    } catch {
      /* ignore — fallback to Connect */
    }
  }

  get allArtists(): SpotifyArtist[] {
    const map = new Map<string, SpotifyArtist>();
    for (const a of this.topArtists) map.set(a.id, a);
    for (const a of this.followedArtists) {
      if (!map.has(a.id)) map.set(a.id, a);
    }
    const seen = new Set<string>(map.keys());
    const inferFromTracks = (tracks: SpotifyTrack[]) => {
      for (const t of tracks) {
        for (const a of t.artists) {
          if (a.id && !seen.has(a.id)) {
            seen.add(a.id);
            map.set(a.id, {
              id: a.id,
              name: a.name,
              uri: a.uri,
              images: [],
              genres: [],
            });
          }
        }
      }
    };
    inferFromTracks(this.savedTracks);
    inferFromTracks(this.recentlyPlayed);
    inferFromTracks(this.topTracks);
    for (const al of this.savedAlbums) {
      for (const a of al.artists) {
        if (a.id && !seen.has(a.id)) {
          seen.add(a.id);
          map.set(a.id, {
            id: a.id,
            name: a.name,
            uri: a.uri,
            images: [],
            genres: [],
          });
        }
      }
    }
    return [...map.values()].sort((a, b) => a.name.localeCompare(b.name));
  }

  isPlaylistOwned(p: SpotifyPlaylist): boolean {
    return !!this.profile?.id && p.owner_id === this.profile.id;
  }

  searchLocal(query: string): {
    tracks: SpotifyTrack[];
    artists: SpotifyArtist[];
    playlists: SpotifyPlaylist[];
  } {
    const q = query.trim().toLowerCase();
    if (!q) return { tracks: [], artists: [], playlists: [] };

    const allTracks = new Map<string, SpotifyTrack>();
    for (const t of this.savedTracks) allTracks.set(t.id, t);
    for (const t of this.recentlyPlayed) if (!allTracks.has(t.id)) allTracks.set(t.id, t);
    for (const t of this.topTracks) if (!allTracks.has(t.id)) allTracks.set(t.id, t);

    const matchesText = (s: string | null | undefined) =>
      !!s && s.toLowerCase().includes(q);

    const tracks = [...allTracks.values()].filter(
      (t) =>
        matchesText(t.name) ||
        t.artists.some((a) => matchesText(a.name)) ||
        matchesText(t.album.name),
    );

    const artists = this.allArtists.filter((a) => matchesText(a.name));

    const playlists = this.playlists.filter(
      (p) => matchesText(p.name) || matchesText(p.description),
    );

    return { tracks, artists, playlists };
  }

  async refreshLocalMatches() {
    if (this.savedTracks.length === 0) return;
    const items = this.savedTracks.map((t) => ({
      title: t.name,
      artist: t.artists[0]?.name ?? "",
      duration_ms: t.duration_ms,
    }));
    try {
      const res = await pluginInvoke<{ matches: { index: number; local_id: number }[] }>(
        "study",
        "study:spotify:match:local",
        { items },
      );
      const next = new Map<string, number>();
      for (const m of res.matches) {
        const t = this.savedTracks[m.index];
        if (t) next.set(t.id, m.local_id);
      }
      this.localMatches = next;
    } catch {
      /* ignore */
    }
  }

  async refreshStatus() {
    try {
      const res = await pluginInvoke<AuthStatus>("study", "study:spotify:auth:status");
      this.status = res;
      if (res.logged_in && !this.profile) {
        await this.loadProfile().catch(() => {});
      }
      if (!res.logged_in) {
        this.profile = null;
      }
    } catch (e) {
      this.error = e instanceof Error ? e.message : String(e);
    }
  }

  async setClientId(id: string): Promise<void> {
    await pluginInvoke("study", "study:spotify:config:set_client_id", { clientId: id });
    await this.refreshStatus();
  }

  async login(): Promise<{ auth_url: string }> {
    this.error = null;
    this.authInProgress = true;
    try {
      const res = await pluginInvoke<{ auth_url: string }>(
        "study",
        "study:spotify:auth:start",
      );
      const { open } = await import("@tauri-apps/plugin-shell");
      await open(res.auth_url).catch(() => {
        try {
          window.open(res.auth_url, "_blank");
        } catch {
          /* ignore */
        }
      });
      return res;
    } catch (e) {
      this.authInProgress = false;
      this.error = e instanceof Error ? e.message : String(e);
      throw e;
    }
  }

  async cancelAuth(): Promise<void> {
    try {
      await pluginInvoke("study", "study:spotify:auth:cancel");
    } catch {
      /* ignore */
    }
    this.authInProgress = false;
  }

  async logout(): Promise<void> {
    try {
      await pluginInvoke("study", "study:spotify:auth:logout");
    } finally {
      this.profile = null;
      this.savedTracks = [];
      this.playlists = [];
      this.recentlyPlayed = [];
      this.topArtists = [];
      this.topTracks = [];
      this.devices = [];
      await this.refreshStatus();
    }
  }

  async loadProfile() {
    const me = await pluginInvoke<SpotifyProfile & { images?: SpotifyImage[] }>(
      "study",
      "study:spotify:me",
    );
    this.profile = {
      id: me.id,
      display_name: me.display_name ?? null,
      email: me.email ?? null,
      product: me.product ?? null,
      country: me.country ?? null,
      images: me.images ?? [],
    };
  }

  async loadAll() {
    if (!this.status.logged_in) return;
    this.loadingLibrary = true;
    try {
      const [savedRes, playlistsRes, recentRes, topArtistsRes, topTracksRes, savedAlbumsRes] =
        await Promise.allSettled([
          pluginInvoke<{ items: any[] }>("study", "study:spotify:library:saved_tracks", {
            limit: 50,
            offset: 0,
          }),
          pluginInvoke<{ items: any[] }>("study", "study:spotify:library:playlists", {
            limit: 50,
            offset: 0,
          }),
          pluginInvoke<{ items: any[] }>(
            "study",
            "study:spotify:library:recently_played",
            { limit: 50 },
          ),
          pluginInvoke<{ items: any[] }>("study", "study:spotify:library:top_artists", {
            timeRange: "medium_term",
            limit: 20,
          }),
          pluginInvoke<{ items: any[] }>("study", "study:spotify:library:top_tracks", {
            timeRange: "medium_term",
            limit: 20,
          }),
          pluginInvoke<{ items: any[] }>("study", "study:spotify:library:saved_albums", {
            limit: 50,
            offset: 0,
          }),
        ]);

      if (savedRes.status === "fulfilled") {
        this.savedTracks = (savedRes.value.items ?? [])
          .map((it) => mapTrack(it.track, it.added_at))
          .filter((t): t is SpotifyTrack => !!t);
        void this.refreshLocalMatches();
      }
      if (playlistsRes.status === "fulfilled") {
        this.playlists = (playlistsRes.value.items ?? [])
          .map((p) => mapPlaylist(p, p?.owner))
          .filter((p): p is SpotifyPlaylist => !!p)
          .filter((p) => p.owner_id !== "spotify");
      }
      if (recentRes.status === "fulfilled") {
        const seen = new Set<string>();
        this.recentlyPlayed = (recentRes.value.items ?? [])
          .map((it) => mapTrack(it.track, undefined, it.played_at))
          .filter((t): t is SpotifyTrack => {
            if (!t || seen.has(t.id)) return false;
            seen.add(t.id);
            return true;
          });
      }
      if (topArtistsRes.status === "fulfilled") {
        this.topArtists = (topArtistsRes.value.items ?? [])
          .map(mapArtist)
          .filter((a): a is SpotifyArtist => !!a);
      }
      if (topTracksRes.status === "fulfilled") {
        this.topTracks = (topTracksRes.value.items ?? [])
          .map((t) => mapTrack(t))
          .filter((t): t is SpotifyTrack => !!t);
      }
      void this.getFollowedArtists()
        .then((arr) => {
          this.followedArtists = arr;
        })
        .catch(() => {});

      if (savedAlbumsRes.status === "fulfilled") {
        const out: SpotifySavedAlbum[] = [];
        for (const it of savedAlbumsRes.value.items ?? []) {
          const a = it?.album;
          if (!a || !a.id) continue;
          out.push({
            id: a.id,
            name: a.name ?? "",
            uri: a.uri ?? `spotify:album:${a.id}`,
            images: a.images ?? [],
            artists: (a.artists ?? []).map((ar: any) => ({
              id: ar.id,
              name: ar.name,
              uri: ar.uri ?? `spotify:artist:${ar.id}`,
            })),
            release_date: a.release_date ?? null,
            total_tracks: a.total_tracks ?? 0,
            added_at: it?.added_at,
          });
        }
        this.savedAlbums = out;
      }
    } finally {
      this.loadingLibrary = false;
    }
  }

  async loadPlaylistTracks(playlistId: string): Promise<SpotifyTrack[]> {
    const out: SpotifyTrack[] = [];
    let offset = 0;
    const pageSize = 100;
    for (let page = 0; page < 20; page++) {
      const res = await pluginInvoke<{ items: any[]; next: string | null }>(
        "study",
        "study:spotify:library:playlist_tracks",
        { playlistId, limit: pageSize, offset },
      );
      const items = res.items ?? [];
      for (const it of items) {
        const t = mapTrack(it.track, it.added_at);
        if (t) out.push(t);
      }
      if (items.length < pageSize) break;
      offset += pageSize;
    }
    return out;
  }

  async loadDevices() {
    const res = await pluginInvoke<{ devices: SpotifyDevice[] }>(
      "study",
      "study:spotify:playback:devices",
    );
    this.devices = res.devices ?? [];
    return this.devices;
  }

  async transferPlayback(deviceId: string, play = true) {
    await pluginInvoke("study", "study:spotify:playback:transfer", { deviceId, play });
  }

  async playUris(opts: {
    deviceId?: string;
    uris?: string[];
    contextUri?: string;
    positionMs?: number;
  }) {
    await pluginInvoke("study", "study:spotify:playback:play", {
      deviceId: opts.deviceId,
      uris: opts.uris ?? [],
      contextUri: opts.contextUri,
      positionMs: opts.positionMs,
    });
  }

  async toggleSaveTrack(trackId: string): Promise<boolean> {
    const contains = await pluginInvoke<boolean[]>(
      "study",
      "study:spotify:tracks:contains",
      { ids: [trackId] },
    );
    const isSaved = contains?.[0] === true;
    if (isSaved) {
      await pluginInvoke("study", "study:spotify:tracks:unsave", {
        ids: [trackId],
      });
      this.savedTracks = this.savedTracks.filter((t) => t.id !== trackId);
      return false;
    }
    await pluginInvoke("study", "study:spotify:tracks:save", {
      ids: [trackId],
    });
    return true;
  }

  async toggleSaveAlbum(albumId: string): Promise<boolean> {
    const contains = await pluginInvoke<boolean[]>(
      "study",
      "study:spotify:albums:contains",
      { ids: [albumId] },
    );
    const isSaved = contains?.[0] === true;
    if (isSaved) {
      await pluginInvoke("study", "study:spotify:albums:unsave", {
        ids: [albumId],
      });
      this.savedAlbums = this.savedAlbums.filter((a) => a.id !== albumId);
      return false;
    }
    await pluginInvoke("study", "study:spotify:albums:save", {
      ids: [albumId],
    });
    return true;
  }

  async addToQueue(trackUri: string, deviceId?: string): Promise<void> {
    await pluginInvoke("study", "study:spotify:playback:add_to_queue", {
      uri: trackUri,
      deviceId,
    });
  }

  async getAlbum(albumId: string): Promise<any> {
    return pluginInvoke("study", "study:spotify:albums:get", { albumId });
  }

  async getAlbumTracks(albumId: string): Promise<SpotifyTrack[]> {
    const out: SpotifyTrack[] = [];
    let offset = 0;
    for (let page = 0; page < 10; page++) {
      const res = await pluginInvoke<{ items: any[]; next: string | null }>(
        "study",
        "study:spotify:albums:tracks",
        { albumId, limit: 50, offset },
      );
      const items = res.items ?? [];
      for (const it of items) {
        const t = mapTrack(it);
        if (t) out.push(t);
      }
      if (items.length < 50) break;
      offset += 50;
    }
    return out;
  }

  async getArtist(artistId: string): Promise<any> {
    return pluginInvoke("study", "study:spotify:artists:get", { artistId });
  }

  async getArtistAlbums(artistId: string): Promise<SpotifySavedAlbum[]> {
    const res = await pluginInvoke<{ items: any[] }>(
      "study",
      "study:spotify:artists:albums",
      { artistId, limit: 50 },
    );
    const out: SpotifySavedAlbum[] = [];
    for (const a of res.items ?? []) {
      if (!a?.id) continue;
      out.push({
        id: a.id,
        name: a.name ?? "",
        uri: a.uri ?? `spotify:album:${a.id}`,
        images: a.images ?? [],
        artists: (a.artists ?? []).map((ar: any) => ({
          id: ar.id,
          name: ar.name,
          uri: ar.uri ?? `spotify:artist:${ar.id}`,
        })),
        release_date: a.release_date ?? null,
        total_tracks: a.total_tracks ?? 0,
      });
    }
    return out;
  }

  async getArtistTopTracks(artistId: string, market = "US"): Promise<SpotifyTrack[]> {
    const res = await pluginInvoke<{ tracks: any[] }>(
      "study",
      "study:spotify:artists:top_tracks",
      { artistId, market },
    );
    const out: SpotifyTrack[] = [];
    for (const t of res.tracks ?? []) {
      const mapped = mapTrack(t);
      if (mapped) out.push(mapped);
    }
    return out;
  }

  async getFollowedArtists(): Promise<SpotifyArtist[]> {
    const res = await pluginInvoke<{ artists: { items: any[] } }>(
      "study",
      "study:spotify:artists:followed",
      { limit: 50 },
    );
    const out: SpotifyArtist[] = [];
    for (const a of res.artists?.items ?? []) {
      const m = mapArtist(a);
      if (m) out.push(m);
    }
    return out;
  }

  async checkFollowArtists(ids: string[]): Promise<boolean[]> {
    if (ids.length === 0) return [];
    return pluginInvoke<boolean[]>("study", "study:spotify:artists:check_follow", {
      ids,
    });
  }

  async toggleFollowArtist(artistId: string): Promise<boolean> {
    const res = await this.checkFollowArtists([artistId]);
    const isFollowing = res[0] === true;
    if (isFollowing) {
      await pluginInvoke("study", "study:spotify:artists:unfollow", {
        ids: [artistId],
      });
      return false;
    }
    await pluginInvoke("study", "study:spotify:artists:follow", {
      ids: [artistId],
    });
    return true;
  }

  async playTrack(track: SpotifyTrack, queue?: SpotifyTrack[]): Promise<"sdk" | "youtube"> {
    const local = spotifyTrackToMusicTrack(track);
    const localQueue = (queue && queue.length > 0 ? queue : [track]).map(
      spotifyTrackToMusicTrack,
    );
    await musicPlayer.play(local, localQueue);
    return this.isPremium && spotifySdk.ready ? "sdk" : "youtube";
  }

  async playPlaylistContext(
    _playlistUri: string,
    firstTrack: SpotifyTrack,
    tracks: SpotifyTrack[],
  ): Promise<"sdk" | "youtube"> {
    return this.playTrack(firstTrack, tracks);
  }
}

export const spotifyStore = new SpotifyStore();

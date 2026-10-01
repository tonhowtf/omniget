// Helpers shared by the OmniTube Music screens and cards: turning any Item
// into something playable, opening it, and the song/collection actions of
// the ⋮ menus. Views stay thin; the playback engine is `$lib/tube/music`.
import { get } from "svelte/store";
import { t } from "$lib/i18n";
import * as api from "$lib/tube/api";
import { currentSong, enqueue, mplayer, playPlaylist, playRadio, playSong, playSongs } from "$lib/tube/music.svelte";
import { go } from "$lib/tube/ui.svelte";
import { showToast } from "$lib/stores/toast-store.svelte";
import type { AlbumPage, Item, SongItem } from "$lib/tube/types";

const tr = (key: string, vars?: Record<string, unknown>) => get(t)(key, vars) as string;

/** Account state for the Music face (sign-in banner, library invite). Loaded once. */
export const macct = $state({ loaded: false, logged: false, name: null as string | null, avatar: null as string | null });
let acctLoading: Promise<void> | null = null;

export function loadAccount(force = false): Promise<void> {
  if (acctLoading && !force) return acctLoading;
  acctLoading = api
    .account()
    .then((a) => {
      macct.logged = !!a?.logged_in;
      macct.name = a?.name ?? null;
      macct.avatar = a?.avatar ?? null;
    })
    .catch(() => {
      macct.logged = false;
    })
    .finally(() => {
      macct.loaded = true;
    });
  return acctLoading;
}

export async function signIn(): Promise<boolean> {
  try {
    const a = await api.login();
    macct.logged = !!a?.logged_in;
    macct.name = a?.name ?? null;
    macct.avatar = a?.avatar ?? null;
    macct.loaded = true;
    return macct.logged;
  } catch (e) {
    showToast("error", String(e));
    return false;
  }
}

/** Songs and music videos both play in the music player. */
export function songFromItem(it: Item): SongItem | null {
  if (it.kind === "song") return it;
  if (it.kind === "video") {
    return {
      kind: "song",
      id: it.id,
      title: it.title,
      artists: it.channel ? [it.channel] : [],
      album: null,
      thumbs: it.thumbs,
      duration_s: it.duration_s ?? null,
      explicit: it.badges.includes("explicit"),
      is_video: true,
      plays_text: it.views_text ?? null,
      set_video_id: it.set_video_id ?? null,
    };
  }
  return null;
}

export function songsOf(items: Item[]): SongItem[] {
  const out: SongItem[] = [];
  for (const it of items) {
    const s = songFromItem(it);
    if (s) out.push(s);
  }
  return out;
}

/** Stable key for keyed #each over items of any kind. */
export function itemKey(it: Item, i: number): string {
  return `${it.kind}:${"id" in it ? it.id : it.browse_id + (it.params ?? "")}:${i}`;
}

/** Queue source string for a collection, matched against `mplayer.context`. */
export function ctxOf(it: Item): string | null {
  if (it.kind === "album") return `album:${it.id}`;
  if (it.kind === "playlist") return `playlist:${it.id}`;
  if (it.kind === "artist") return `artist:${it.id}`;
  return null;
}

export function isCurrent(id: string): boolean {
  return currentSong()?.id === id;
}

export function isPlayingCtx(ctx: string | null): boolean {
  return !!ctx && mplayer.context === ctx && mplayer.playing;
}

/** Card/row click on the body: navigate (collections) or play (songs). */
export function openItem(it: Item): void {
  switch (it.kind) {
    case "album":
      go({ name: "m_album", id: it.id });
      break;
    case "artist":
      go({ name: "m_artist", id: it.id });
      break;
    case "channel":
      go({ name: "m_artist", id: it.id });
      break;
    case "playlist":
      go({ name: "m_playlist", id: it.id });
      break;
    case "mood":
      go({ name: "m_browse", id: it.browse_id, params: it.params ?? null, title: it.title });
      break;
    case "short":
      go({ name: "watch", id: it.id });
      break;
    default: {
      const s = songFromItem(it);
      if (s) void playSong(s);
    }
  }
}

/** Plays an album page's tracks, keeping the album as the queue context. */
export function playAlbumPage(a: AlbumPage, start = 0, shuffle = false): void {
  if (!a.tracks.length) return;
  if (shuffle) {
    const list = shuffled(a.tracks);
    void playSongs(list, 0, `album:${a.id}`);
    mplayer.shuffle = true;
    return;
  }
  void playSongs(a.tracks, start, `album:${a.id}`);
}

/** The floating play button of a card. */
export async function playItem(it: Item, shuffle = false): Promise<void> {
  try {
    switch (it.kind) {
      case "album": {
        if (it.playlist_id) await playPlaylist(it.playlist_id, shuffle);
        else playAlbumPage(await api.musicAlbum(it.id), 0, shuffle);
        break;
      }
      case "playlist":
        if (it.is_mix && it.first_video_id) await playRadio(it.first_video_id, it.id);
        else await playPlaylist(it.id, shuffle);
        break;
      case "artist":
      case "channel": {
        const a = await api.musicArtist(it.id);
        if (a.shuffle_playlist_id) await playPlaylist(a.shuffle_playlist_id, true);
        else if (a.radio_playlist_id) await playRadio("", a.radio_playlist_id);
        break;
      }
      default: {
        const s = songFromItem(it);
        if (s) await playSong(s);
        else openItem(it);
      }
    }
  } catch (e) {
    showToast("error", tr("tube.music.play_failed", { reason: String(e) }));
  }
}

export function shuffled<T>(list: T[]): T[] {
  const a = list.slice();
  for (let i = a.length - 1; i > 0; i--) {
    const j = Math.floor(Math.random() * (i + 1));
    [a[i], a[j]] = [a[j], a[i]];
  }
  return a;
}

export function playNext(s: SongItem): void {
  enqueue([s], true);
  showToast("info", tr("tube.music.toast_play_next", { title: s.title }));
}

export function addToQueue(songs: SongItem[]): void {
  if (!songs.length) return;
  enqueue(songs);
  showToast("info", songs.length === 1 ? tr("tube.music.toast_queued", { title: songs[0].title }) : tr("tube.music.toast_queued_many", { count: songs.length }));
}

export async function likeSong(s: SongItem, liked: boolean): Promise<boolean> {
  try {
    await api.action(liked ? "music_like" : "music_remove_like", { video_id: s.id });
    s.liked = liked;
    return true;
  } catch (e) {
    showToast("error", String(e));
    return false;
  }
}

export async function downloadSongs(songs: SongItem[]): Promise<void> {
  let ok = 0;
  for (const s of songs) {
    try {
      await api.download({ video_id: s.id, title: s.title, audio_only: true, music: true });
      ok++;
    } catch (e) {
      showToast("error", String(e));
    }
  }
  if (ok) showToast("success", ok === 1 ? tr("tube.music.toast_download_one") : tr("tube.music.toast_download_many", { count: ok }));
}

export async function saveToLibrary(playlistId: string, add = true): Promise<boolean> {
  try {
    await api.action(add ? "library_add" : "library_remove", { playlist_id: playlistId });
    showToast("success", add ? tr("tube.music.toast_saved") : tr("tube.music.toast_removed"));
    return true;
  } catch (e) {
    showToast("error", String(e));
    return false;
  }
}

export function linkFor(it: Item): string {
  switch (it.kind) {
    case "song":
    case "video":
      return `https://music.youtube.com/watch?v=${it.id}`;
    case "album":
      return `https://music.youtube.com/browse/${it.id}`;
    case "artist":
    case "channel":
      return `https://music.youtube.com/channel/${it.id}`;
    case "playlist":
      return `https://music.youtube.com/playlist?list=${it.id}`;
    default:
      return "https://music.youtube.com/";
  }
}

export async function copyLink(url: string): Promise<void> {
  try {
    await navigator.clipboard.writeText(url);
    showToast("success", tr("tube.music.toast_link_copied"));
  } catch {
    showToast("error", tr("tube.music.toast_copy_failed"));
  }
}

/** "Artist A, Artist B" */
function artistNames(s: { artists: { name: string }[] }): string {
  return s.artists.map((a) => a.name).join(", ");
}

/** Second line under a card title, by kind. */
export function subtitleOf(it: Item): string {
  switch (it.kind) {
    case "song":
      return [it.is_video ? tr("tube.music.kind_video") : tr("tube.music.kind_song"), artistNames(it), it.plays_text].filter(Boolean).join(" • ");
    case "video":
      return [it.channel?.name, it.views_text].filter(Boolean).join(" • ");
    case "album":
      return [it.album_type || tr("tube.music.kind_album"), artistNames(it), it.year].filter(Boolean).join(" • ");
    case "artist":
      return [tr("tube.music.kind_artist"), it.subscribers_text].filter(Boolean).join(" • ");
    case "channel":
      return [tr("tube.music.kind_artist"), it.subscribers_text].filter(Boolean).join(" • ");
    case "playlist":
      return [it.is_mix ? tr("tube.music.kind_mix") : tr("tube.music.kind_playlist"), it.channel?.name, it.count_text].filter(Boolean).join(" • ");
    case "short":
      return it.views_text ?? "";
    default:
      return "";
  }
}

export function titleOf(it: Item): string {
  return it.kind === "artist" || it.kind === "channel" ? it.name : it.title;
}

/** Most items carry `thumbs`; channels carry a single avatar URL. */
export function thumbsOf(it: Item) {
  if (it.kind === "channel") return it.avatar ? [{ url: it.avatar, width: 176, height: 176 }] : [];
  if (it.kind === "mood") return [];
  return it.thumbs;
}

/** Infinite scroll: calls `load` when `node` gets near the viewport. */
export function nearEnd(node: HTMLElement, load: () => void) {
  let cb = load;
  const io = new IntersectionObserver(
    (entries) => {
      if (entries.some((e) => e.isIntersecting)) cb();
    },
    { rootMargin: "800px 0px" },
  );
  io.observe(node);
  return {
    update(next: () => void) {
      cb = next;
    },
    destroy() {
      io.disconnect();
    },
  };
}

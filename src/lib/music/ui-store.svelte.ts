import { invoke } from "@tauri-apps/api/core";
import { get } from "svelte/store";
import { t } from "$lib/i18n";
import { showToast } from "$lib/stores/toast-store.svelte";
import type { MenuItem } from "$components/music/ContextMenu.svelte";
import { albumKey } from "./library";
import { patchTracksLocal, removeTracksLocal, trackById } from "./library-store.svelte";
import { enqueue, playTracks } from "./player-store.svelte";
import type { Playlist, Track } from "./types";

export type View =
  | { kind: "home" }
  | { kind: "songs" }
  | { kind: "liked" }
  | { kind: "recent" }
  | { kind: "albums" }
  | { kind: "album"; key: string }
  | { kind: "artists" }
  | { kind: "artist"; name: string }
  | { kind: "genres" }
  | { kind: "genre"; name: string }
  | { kind: "folders"; path?: string }
  | { kind: "playlists" }
  | { kind: "playlist"; id: number }
  | { kind: "radio" }
  | { kind: "download"; q?: string }
  | { kind: "tools"; tab?: string; ids?: number[] }
  | { kind: "stats" }
  | { kind: "nowplaying" }
  | { kind: "settings" }
  | { kind: "search"; q: string };

export const ui = $state({
  view: { kind: "home" } as View,
  stack: [] as View[],
  menu: null as { x: number; y: number; items: MenuItem[] } | null,
  tagEditor: null as number[] | null,
  playlistPicker: null as number[] | null,
  confirm: null as { title: string; body: string; danger?: boolean; ok: string; run: () => void } | null,
  side: null as "queue" | "lyrics" | "eq" | null,
  playlists: [] as Playlist[],
  search: "",
});

const tr = (k: string, vars?: Record<string, unknown>) => get(t)(k, vars) as string;

export function go(v: View) {
  const cur = ui.view;
  if (JSON.stringify(cur) === JSON.stringify(v)) return;
  ui.stack = [...ui.stack.slice(-30), cur];
  ui.view = v;
}

export function back() {
  const prev = ui.stack[ui.stack.length - 1];
  if (!prev) return;
  ui.stack = ui.stack.slice(0, -1);
  ui.view = prev;
}

export function openMenu(e: MouseEvent, items: MenuItem[]) {
  e.preventDefault();
  ui.menu = { x: e.clientX, y: e.clientY, items };
}

export async function loadPlaylists() {
  try {
    ui.playlists = await invoke<Playlist[]>("music_playlists");
  } catch {
  }
}

async function addToPlaylist(playlistId: number, ids: number[]) {
  await invoke("music_playlist_add", { id: playlistId, trackIds: ids });
  const pl = ui.playlists.find((p) => p.id === playlistId);
  showToast("success", tr("music.toast.added_to_playlist", { name: pl?.name ?? "" }));
  void loadPlaylists();
}

export async function createPlaylistWith(ids: number[], name?: string) {
  const n = (name ?? "").trim() || tr("music.playlist.untitled");
  const id = await invoke<number>("music_playlist_create", { name: n, smart: null, trackIds: ids });
  await loadPlaylists();
  showToast("success", tr("music.toast.playlist_created", { name: n }));
  return id;
}

async function setLiked(ids: number[], liked: boolean) {
  patchTracksLocal(ids, { liked });
  await invoke("music_set_liked", { ids, liked }).catch(() => {});
}

async function setRating(ids: number[], rating: number) {
  patchTracksLocal(ids, { rating });
  await invoke("music_set_rating", { ids, rating }).catch(() => {});
}

export function analyze(ids: number[]) {
  void invoke("music_analyze", { ids });
  showToast("info", tr("music.toast.analysis_started", { count: ids.length }));
}

export function removeTracks(ids: number[], deleteFiles: boolean) {
  const body = deleteFiles ? tr("music.confirm.delete_body", { count: ids.length }) : tr("music.confirm.remove_body", { count: ids.length });
  ui.confirm = {
    title: deleteFiles ? tr("music.confirm.delete_title") : tr("music.confirm.remove_title"),
    body,
    danger: deleteFiles,
    ok: deleteFiles ? tr("music.menu.delete_file") : tr("music.menu.remove"),
    run: async () => {
      try {
        await invoke("music_remove_tracks", { ids, deleteFiles });
        removeTracksLocal(ids);
      } catch (e) {
        showToast("error", String(e));
      }
    },
  };
}

function trackMenu(ids: number[], extra: MenuItem[] = []): MenuItem[] {
  const tracks = ids.map((id) => trackById(id)).filter(Boolean) as Track[];
  if (!tracks.length) return [];
  const first = tracks[0];
  const allLiked = tracks.every((x) => x.liked);
  const one = tracks.length === 1;
  const plSub: MenuItem[] = [
    { label: tr("music.menu.new_playlist"), icon: "plus", action: () => (ui.playlistPicker = ids) },
    ...(ui.playlists.filter((p) => !p.smart).length ? [{ divider: true } as MenuItem] : []),
    ...ui.playlists
      .filter((p) => !p.smart)
      .map((p) => ({ label: p.name, icon: "playlist", action: () => void addToPlaylist(p.id, ids) })),
  ];
  return [
    { label: tr("music.menu.play"), icon: "play", action: () => void playTracks(ids, ids[0]) },
    { label: tr("music.menu.play_next"), icon: "queue", action: () => enqueue(ids, true) },
    { label: tr("music.menu.add_queue"), icon: "plus", action: () => enqueue(ids, false) },
    { label: tr("music.menu.add_playlist"), icon: "playlist", sub: plSub },
    { divider: true },
    { label: tr(allLiked ? "music.unlike" : "music.like"), icon: allLiked ? "heart-fill" : "heart", action: () => void setLiked(ids, !allLiked) },
    {
      label: tr("music.menu.rating"),
      icon: "star",
      sub: [0, 1, 2, 3, 4, 5].map((r) => ({
        label: r === 0 ? tr("music.menu.no_rating") : "★".repeat(r),
        checked: one && first.rating === r,
        action: () => void setRating(ids, r),
      })),
    },
    { divider: true },
    ...(one && first.album ? [{ label: tr("music.menu.go_album"), icon: "disc", action: () => go({ kind: "album", key: albumKey(first) }) }] : []),
    ...(one && first.artist ? [{ label: tr("music.menu.go_artist"), icon: "artist", action: () => go({ kind: "artist", name: first.artist }) }] : []),
    { label: tr("music.menu.edit_tags"), icon: "edit", hint: one ? "" : String(tracks.length), action: () => (ui.tagEditor = ids) },
    { label: tr("music.menu.analyze"), icon: "activity", action: () => analyze(ids) },
    { label: tr("music.menu.convert"), icon: "convert", action: () => go({ kind: "tools", tab: "convert", ids }) },
    ...(one ? [{ label: tr("music.menu.quality"), icon: "wave", action: () => go({ kind: "tools", tab: "quality", ids }) }] : []),
    ...(one ? [{ label: tr("music.menu.reveal"), icon: "folder", action: () => void invoke("reveal_file", { path: first.path }) }] : []),
    ...extra,
    { divider: true },
    { label: tr("music.menu.remove"), icon: "close", action: () => removeTracks(ids, false) },
    { label: tr("music.menu.delete_file"), icon: "trash", danger: true, action: () => removeTracks(ids, true) },
  ];
}

export function showTrackMenu(e: MouseEvent, ids: number[], extra: MenuItem[] = []) {
  openMenu(e, trackMenu(ids, extra));
}


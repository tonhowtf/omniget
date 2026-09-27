// Music downloads started from /music: they go through the normal download
// queue in audio mode (so the user's music format and tagging apply), and
// when one finishes its file is added to the library.

import { invoke } from "@tauri-apps/api/core";
import type { DownloadItem } from "$lib/stores/download-store.svelte";
import { ensureStatus, upsertTracks } from "./library-store.svelte";
import type { Track } from "./types";

const KEY = "omniget.music.downloads.v1";

export type MusicDownload = {
  id: number;
  url: string;
  title: string;
  thumb?: string | null;
  state: "running" | "done" | "failed";
  trackIds?: number[];
  error?: string;
  at: number;
};

function load(): MusicDownload[] {
  try {
    return JSON.parse(localStorage.getItem(KEY) ?? "[]");
  } catch {
    return [];
  }
}

export const musicDownloads = $state({ items: load() });

function persist() {
  try {
    localStorage.setItem(KEY, JSON.stringify(musicDownloads.items.slice(0, 100)));
  } catch {
    /* ignore */
  }
}

export async function startMusicDownload(url: string, title = "", thumb: string | null = null): Promise<MusicDownload> {
  const status = await ensureStatus();
  const r = await invoke<{ id: number; title: string }>("download_from_url", {
    url,
    outputDir: status.download_dir,
    downloadMode: "audio",
  });
  const item: MusicDownload = { id: r.id, url, title: title || r.title || url, thumb, state: "running", at: Date.now() };
  musicDownloads.items = [item, ...musicDownloads.items.filter((x) => x.id !== r.id)];
  persist();
  return item;
}

const importing = new Set<number>();

/** Called whenever the download queue changes. */
export function syncMusicDownloads(map: Map<number, DownloadItem>) {
  for (const md of musicDownloads.items) {
    if (md.state !== "running" || importing.has(md.id)) continue;
    const d = map.get(md.id);
    if (!d) continue;
    if (d.status === "error") {
      md.state = "failed";
      md.error = d.error;
      persist();
    } else if (d.status === "complete" && d.filePath) {
      importing.add(md.id);
      void invoke<Track[]>("music_import_paths", { paths: [d.filePath], sourceUrl: md.url })
        .then((rows) => {
          upsertTracks(rows);
          md.state = "done";
          md.trackIds = rows.map((r) => r.id);
          if (rows[0] && (!md.title || md.title === md.url)) md.title = rows[0].title;
        })
        .catch((e) => {
          md.state = "failed";
          md.error = String(e);
        })
        .finally(() => {
          importing.delete(md.id);
          musicDownloads.items = [...musicDownloads.items];
          persist();
        });
    }
  }
}

export function clearFinishedMusicDownloads() {
  musicDownloads.items = musicDownloads.items.filter((x) => x.state === "running");
  persist();
}

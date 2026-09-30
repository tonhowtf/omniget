// Book downloads started from /read go through the normal download queue
// (same engine, retries, proxy and cookies as everything else); when one
// finishes its file is added to the reading library.

import { invoke } from "@tauri-apps/api/core";
import type { DownloadItem } from "$lib/stores/download-store.svelte";
import { ensureStatus, upsertBooks } from "./library-store.svelte";
import type { Book } from "./types";

const KEY = "omniget.read.downloads.v1";

export type ReadDownload = {
  id: number;
  url: string;
  title: string;
  source: string;
  cover?: string | null;
  state: "running" | "done" | "failed";
  bookIds?: number[];
  error?: string;
  at: number;
};

function load(): ReadDownload[] {
  try {
    return JSON.parse(localStorage.getItem(KEY) ?? "[]");
  } catch {
    return [];
  }
}

export const readDownloads = $state({ items: load() });

function persist() {
  try {
    localStorage.setItem(KEY, JSON.stringify(readDownloads.items.slice(0, 200)));
  } catch {
    /* ignore */
  }
}

export async function startBookDownload(url: string, title: string, source: string, cover: string | null = null): Promise<ReadDownload> {
  const status = await ensureStatus();
  // OPDS catalogs with a login get their credentials inlined for the engine.
  const finalUrl = source.startsWith("opds:") ? await invoke<string>("read_opds_download_url", { source, url }) : url;
  const r = await invoke<{ id: number; title: string }>("download_from_url", {
    url: finalUrl,
    outputDir: status.download_dir,
  });
  const item: ReadDownload = { id: r.id, url, title: title || r.title || url, source, cover, state: "running", at: Date.now() };
  readDownloads.items = [item, ...readDownloads.items.filter((x) => x.id !== r.id)];
  persist();
  return item;
}

const importing = new Set<number>();

/** Called whenever the download queue changes. */
export function syncReadDownloads(map: Map<number, DownloadItem>) {
  for (const d of readDownloads.items) {
    if (d.state !== "running" || importing.has(d.id)) continue;
    const q = map.get(d.id);
    if (!q) continue;
    if (q.status === "error") {
      d.state = "failed";
      d.error = q.error;
      persist();
    } else if (q.status === "complete" && q.filePath) {
      importing.add(d.id);
      void invoke<Book[]>("read_import_paths", { paths: [q.filePath], source: d.source, sourceUrl: d.url })
        .then((rows) => {
          upsertBooks(rows);
          d.state = "done";
          d.bookIds = rows.map((b) => b.id);
          if (!rows.length) {
            d.state = "failed";
            d.error = "not a book file";
          }
        })
        .catch((e) => {
          d.state = "failed";
          d.error = String(e);
        })
        .finally(() => {
          importing.delete(d.id);
          readDownloads.items = [...readDownloads.items];
          persist();
        });
    }
  }
}

export function clearFinishedReadDownloads() {
  readDownloads.items = readDownloads.items.filter((x) => x.state === "running");
  persist();
}

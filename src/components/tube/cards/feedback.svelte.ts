import * as api from "$lib/tube/api";
import { patchTubePrefs, tubePrefs } from "$lib/tube/prefs.svelte";
import type { Item } from "$lib/tube/types";

type UndoEntry = {
  id: number;
  text: string;
  undo: () => void | Promise<void>;
  commit: () => void | Promise<void>;
};

export const feedback = $state({
  videos: {} as Record<string, boolean>,
  channels: {} as Record<string, boolean>,
  undo: null as UndoEntry | null,
});

let seq = 0;

function push(entry: Omit<UndoEntry, "id">) {
  const prev = feedback.undo;
  feedback.undo = { ...entry, id: ++seq };
  if (prev) void prev.commit();
}

export function channelOf(item: Item): { id: string; name: string } | null {
  if (item.kind === "video" || item.kind === "short" || item.kind === "playlist") {
    return item.channel ? { id: item.channel.id, name: item.channel.name } : null;
  }
  if (item.kind === "channel") return { id: item.id, name: item.name };
  return null;
}

export function isVisible(item: Item): boolean {
  if ("id" in item && feedback.videos[item.id]) return false;
  const ch = channelOf(item);
  if (!ch) return true;
  if (feedback.channels[ch.id]) return false;
  return !tubePrefs().blocked_channels.some((b) => b.id === ch.id);
}

export function notInterested(item: Item, text: string) {
  if (!("id" in item)) return;
  const id = item.id;
  feedback.videos[id] = true;
  const token = item.kind === "video" ? item.feedback_token : null;
  push({
    text,
    undo: () => {
      delete feedback.videos[id];
    },
    commit: async () => {
      if (token) await api.action("not_interested", { video_id: id, feedback_token: token }).catch(() => {});
    },
  });
}

export function dontRecommendChannel(item: Item, text: string) {
  const ch = channelOf(item);
  if (!ch) return;
  feedback.channels[ch.id] = true;
  const token = item.kind === "video" ? item.feedback_token : null;
  push({
    text,
    undo: () => {
      delete feedback.channels[ch.id];
    },
    commit: async () => {
      await api.action("dont_recommend_channel", { channel_id: ch.id, feedback_token: token ?? null }).catch(() => {});
    },
  });
}

export async function blockChannel(ch: { id: string; name: string }, text: string) {
  const list = tubePrefs().blocked_channels;
  if (!list.some((b) => b.id === ch.id)) await patchTubePrefs({ blocked_channels: [...list, ch] });
  push({
    text,
    undo: async () => {
      await patchTubePrefs({ blocked_channels: tubePrefs().blocked_channels.filter((b) => b.id !== ch.id) });
    },
    commit: () => {},
  });
}

export function takeUndo(): UndoEntry | null {
  const u = feedback.undo;
  feedback.undo = null;
  return u;
}

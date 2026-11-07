import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { trackById, trackUrl, upsertTracks } from "./library-store.svelte";
import { activeMedia, player, playIndex, prefs } from "./player-store.svelte";
import { companionVolume, findSet, PROGRESS_EVENT, stemsApi, swapInQueue, syncPlan, type StemJob, type StemSet } from "./stems";
import type { Track } from "./types";

const MIX_KEY = "omniget.music.karaoke.vocal.v1";

function loadMix(): number {
  try {
    const v = Number(localStorage.getItem(MIX_KEY));
    return Number.isFinite(v) ? Math.min(1, Math.max(0, v)) : 0;
  } catch {
    return 0;
  }
}

export const karaoke = $state({
  sets: [] as StemSet[],
  loaded: false,
  vocal: loadMix(),
});

let companion: HTMLAudioElement | null = null;
let companionFor: number | null = null;
let timer: ReturnType<typeof setInterval> | null = null;
let unlisten: Promise<UnlistenFn> | null = null;

export async function loadStemSets() {
  try {
    karaoke.sets = await stemsApi.sets();
    karaoke.loaded = true;
    const ids = karaoke.sets
      .flatMap((s) => [s.vocals_id, s.instrumental_id])
      .filter((id): id is number => id != null && !trackById(id));
    if (ids.length) upsertTracks(await invoke<Track[]>("music_tracks_by_ids", { ids }));
  } catch {
    karaoke.loaded = true;
  }
}

export function stemsFor(trackId: number | null | undefined) {
  return findSet(karaoke.sets, trackId);
}

export function setVocalMix(v: number) {
  karaoke.vocal = Math.min(1, Math.max(0, v));
  try {
    localStorage.setItem(MIX_KEY, String(karaoke.vocal));
  } catch {
  }
  tick();
}

async function swapTo(id: number) {
  if (!trackById(id)) {
    const rows = await invoke<Track[]>("music_tracks_by_ids", { ids: [id] }).catch(() => [] as Track[]);
    upsertTracks(rows);
    if (!trackById(id)) return;
  }
  const i = player.index;
  if (i < 0) return;
  const pos = player.position;
  const old = player.queue[i];
  player.queue = swapInQueue(player.queue, i, id);
  player.base = player.base.map((q) => (q === old ? id : q));
  await playIndex(i, pos);
}

export async function singAlong(set: StemSet) {
  if (set.instrumental_id != null) await swapTo(set.instrumental_id);
}

export async function backToOriginal(set: StemSet) {
  if (set.source_id != null) await swapTo(set.source_id);
}

function stopCompanion() {
  if (!companion || companionFor == null) return;
  companion.pause();
  companion.removeAttribute("src");
  companion.load();
  companionFor = null;
}

function tick() {
  const media = activeMedia();
  const hit = stemsFor(player.current?.id);
  const vocalsId = hit?.role === "instrumental" ? hit.set.vocals_id : null;
  if (!media || vocalsId == null || media.trackId !== player.current?.id || karaoke.vocal <= 0) {
    stopCompanion();
    return;
  }
  if (!companion) {
    companion = new Audio();
    companion.preload = "auto";
  }
  if (companionFor !== vocalsId) {
    companion.src = trackUrl(vocalsId);
    companionFor = vocalsId;
  }
  companion.volume = companionVolume(karaoke.vocal, prefs.volume, prefs.muted);
  if (companion.readyState < 1) return;
  const plan = syncPlan(
    { time: media.offset + media.el.currentTime, paused: media.el.paused, rate: media.el.playbackRate },
    { time: companion.currentTime, paused: companion.paused, rate: companion.playbackRate },
  );
  if (plan.rate != null) companion.playbackRate = plan.rate;
  if (plan.seek != null) companion.currentTime = plan.seek;
  if (plan.play === true) void companion.play().catch(() => {});
  if (plan.play === false) companion.pause();
}

export function startKaraokeSync() {
  if (timer) return;
  timer = setInterval(tick, 200);
  unlisten = listen<StemJob>(PROGRESS_EVENT, (e) => {
    if (e.payload.state === "done") void loadStemSets();
  }).catch(() => () => {});
}

export function stopKaraokeSync() {
  if (timer) clearInterval(timer);
  timer = null;
  void unlisten?.then((u) => u());
  unlisten = null;
  stopCompanion();
}
